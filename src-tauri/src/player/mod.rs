//! The player core: a safe API over libmpv, and the state it maintains.

pub mod events;
pub mod inhibit;
pub mod surface;

// The GL loader is the Linux path's business: it exists to feed libmpv's
// render API through libepoxy. The Windows surface lets mpv own the drawing,
// so nothing there needs a loader.
#[cfg(target_os = "linux")]
pub mod render;

#[cfg(target_os = "linux")]
pub mod surface_gtk;

#[cfg(target_os = "windows")]
pub mod surface_win;

use anyhow::{Context, Result};
use libmpv2::{mpv_error, Error as MpvError, Mpv};
use parking_lot::{Mutex, RwLock};
use std::path::Path;

use crate::privacy::{HidingProfile, PlaybackState};

/// mpv options, set before `mpv_initialize`.
///
/// Half functional, half anti-spoiler configuration; listed together because
/// forgetting either kind has the same symptom: mpv drawing something Murk
/// never asked it to draw.
const MPV_OPTIONS: &[(&str, &str)] = &[
    // -- functional ---------------------------------------------------------
    // Not a hard `vaapi`: Fedora ships mesa without some patented decoders and
    // NVIDIA takes a different path entirely. `auto-safe` quietly drops to
    // software decoding instead of showing a black frame. On Windows the same
    // value picks d3d11va.
    ("hwdec", "auto-safe"),
    // End of file is Murk's business (auto-advance), not mpv's.
    ("keep-open", "yes"),
    ("terminal", "no"),
    // -- anti-spoiler -------------------------------------------------------
    // ~/.config/mpv may enable an OSC, an MPRIS script, a progress bar. None of
    // that is ours to audit, so none of it is loaded.
    ("config", "no"),
    ("load-scripts", "no"),
    // mpv's own on-screen controller draws a seek bar and a clock. The option
    // only exists in builds that include the cplayer frontend, so a
    // libmpv-only build (the Flatpak) rejects it; see OPTIONAL_MPV_OPTIONS.
    ("osc", "no"),
    ("osd-level", "0"),
    ("osd-bar", "no"),
    // Default key bindings print the position to the OSD. All input arrives
    // through the Vue layer instead.
    ("input-default-bindings", "no"),
    ("input-vo-keyboard", "no"),
    // No MPRIS, no media keys announcing the file to the desktop.
    ("input-media-keys", "no"),
];

/// Options that some libmpv builds do not know, and whose absence means the
/// thing they switch off is not there either.
const OPTIONAL_MPV_OPTIONS: &[&str] = &["osc"];

/// How the video reaches the screen, which is the one thing that differs per
/// platform. See [`surface`] for the rest of that story.
///
/// Linux drives the render API by hand: mpv owns no window, hands us frames,
/// and `surface_gtk` composites them under the webview.
#[cfg(target_os = "linux")]
const MPV_VIDEO_OPTIONS: &[(&str, &str)] = &[
    // Required for the render API: mpv owns no window and hands us frames.
    ("vo", "libmpv"),
    ("gpu-api", "opengl"),
    // Without this, `render()` blocks until the frame's presentation time. We
    // call it from the GTK main thread, so that blocks the whole interface.
    ("video-timing-offset", "0"),
];

/// Windows takes the other route mpv offers: it renders into a child window we
/// hand it (`wid`, set in [`PlayerHandle::new`]), on its own thread, with its
/// own D3D11 swap chain. There is no GL context of ours to keep current and no
/// frame callback to service, so `video-timing-offset` stays at mpv's default
/// and mpv presents on its own schedule.
#[cfg(target_os = "windows")]
const MPV_VIDEO_OPTIONS: &[(&str, &str)] = &[
    ("vo", "gpu"),
    ("gpu-api", "d3d11"),
    // The child window is behind the webview and must never take focus or draw
    // a cursor of its own: every pointer and key event belongs to the Vue layer.
    ("input-cursor", "no"),
    ("cursor-autohide", "no"),
];

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
const MPV_VIDEO_OPTIONS: &[(&str, &str)] = &[];

/// Which episode is on screen, so progress can be written without the frontend
/// ever naming a file.
#[derive(Debug, Clone, Copy, Default)]
pub struct CurrentEpisode {
    pub episode_id: i64,
    pub series_id: i64,
}

/// How much of the running time has actually been played, as opposed to
/// seeked past.
///
/// A seek to the last minutes must not finish an episode: the user has not
/// seen the ending, and marking it watched would make Murk open the *next*
/// one the following evening. Only time that went by while the file was
/// playing counts.
#[derive(Debug, Default, Clone, Copy)]
struct WatchCredit {
    /// The last position seen, to measure the step from.
    last_sec: Option<f64>,
    credited_ms: i64,
}

/// The largest step between two position updates that can be ordinary
/// playback. mpv reports `time-pos` several times a second; anything bigger
/// than this is a seek, and a seek earns no credit.
const MAX_PLAYBACK_STEP_SEC: f64 = 2.0;

impl WatchCredit {
    /// Take a new position and credit the part of it that was played.
    fn advance(&mut self, position_sec: f64) {
        if let Some(last) = self.last_sec {
            let step = position_sec - last;
            // Rewinds earn no credit, but no penalty either: rewatching a
            // scene should not undo what was already seen.
            if step > 0.0 && step <= MAX_PLAYBACK_STEP_SEC {
                self.credited_ms += (step * 1000.0) as i64;
            }
        }
        self.last_sec = Some(position_sec);
    }
}

pub struct PlayerHandle {
    /// `RenderContext<'a>` borrows the `Mpv` it was made from, and both live for
    /// the whole run of the program. Leaking once at startup buys a `'static`
    /// borrow and avoids a self-referential struct; there is exactly one of
    /// these and it is freed by the process exiting.
    mpv: &'static Mpv,
    state: RwLock<PlaybackState>,
    profile: RwLock<HidingProfile>,
    current: Mutex<Option<CurrentEpisode>>,
    credit: Mutex<WatchCredit>,
}

/// Force `LC_NUMERIC` back to `C` for the whole process.
///
/// libmpv parses and formats numbers with the C library and refuses to
/// initialise at all if `LC_NUMERIC` is anything else: under a Russian or
/// German locale a decimal comma would make `video-timing-offset=0.0` mean
/// something other than intended. It prints
///
/// ```text
/// Non-C locale detected. This is not supported.
/// Call 'setlocale(LC_NUMERIC, "C");' in your code.
/// ```
///
/// and fails. Rust never calls `setlocale` itself, but **GTK does**:
/// `gtk_init` runs `setlocale(LC_ALL, "")`, which adopts the user's locale.
/// So this has to run *after* GTK is up and *before* the mpv instance is
/// created, which is exactly here: `PlayerHandle::new` is called from Tauri's
/// setup hook.
///
/// Only the numeric category is touched, so dates, sorting and messages keep
/// the user's locale. Murk's own interface is a webview and does not format
/// numbers through the C library.
#[cfg(unix)]
fn force_c_numeric_locale() {
    // SAFETY: setlocale is called once, on the main thread, before any other
    // thread exists.
    unsafe { libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr()) };
}

#[cfg(not(unix))]
fn force_c_numeric_locale() {}

impl PlayerHandle {
    /// `embed_window` is the native handle of the window mpv should draw into,
    /// for platforms where mpv owns the drawing (Windows: the `HWND` of the
    /// child window `surface_win` creates). Platforms that drive the render
    /// API themselves pass `None`.
    ///
    /// It is a constructor parameter and not a later `set_property` because
    /// `wid` is only read when the video output is created, which happens
    /// inside `mpv_initialize`: set afterwards it is silently ignored and the
    /// video appears in a window of its own.
    pub fn new(profile: HidingProfile, embed_window: Option<i64>) -> Result<Self> {
        force_c_numeric_locale();

        tracing::debug!(?embed_window, options = ?MPV_OPTIONS, video_options = ?MPV_VIDEO_OPTIONS, "initialising mpv");
        let mpv = Mpv::with_initializer(|init| {
            if let Some(wid) = embed_window {
                tracing::debug!(wid, "setting mpv wid");
                init.set_option("wid", wid)?;
            }
            for (key, value) in MPV_OPTIONS.iter().chain(MPV_VIDEO_OPTIONS) {
                match init.set_option(key, *value) {
                    Ok(()) => {}
                    // A libmpv built without the cplayer frontend has no OSC to
                    // switch off, and reports the option as unknown. Refusing to
                    // start over an absent spoiler source would be backwards.
                    Err(MpvError::Raw(mpv_error::OptionNotFound))
                        if OPTIONAL_MPV_OPTIONS.contains(key) =>
                    {
                        tracing::debug!(%key, "optional mpv option not supported by this build");
                    }
                    Err(e) => {
                        tracing::error!(%key, %value, "setting mpv option failed: {e}");
                        return Err(e);
                    }
                }
            }
            Ok(())
        })
        .context(
            "creating the mpv instance failed. If mpv reported a non-C locale \
             just above, something re-ran setlocale after force_c_numeric_locale; \
             otherwise check that libmpv.so.2 is installed with \
             `scripts/deps.sh --check`.",
        )?;

        let mpv: &'static Mpv = Box::leak(Box::new(mpv));

        tracing::info!(
            vo = ?mpv.get_property::<String>("vo"),
            hwdec = ?mpv.get_property::<String>("hwdec"),
            gpu_api = ?mpv.get_property::<String>("gpu-api"),
            "mpv initialised"
        );

        let volume = mpv.get_property::<f64>("volume").unwrap_or(100.0);
        let state = PlaybackState {
            idle: true,
            volume,
            ..Default::default()
        };

        Ok(Self {
            mpv,
            state: RwLock::new(state),
            profile: RwLock::new(profile),
            current: Mutex::new(None),
            credit: Mutex::new(WatchCredit::default()),
        })
    }

    pub fn mpv(&self) -> &'static Mpv {
        self.mpv
    }

    pub fn state(&self) -> parking_lot::RwLockReadGuard<'_, PlaybackState> {
        self.state.read()
    }

    pub fn state_mut(&self) -> parking_lot::RwLockWriteGuard<'_, PlaybackState> {
        self.state.write()
    }

    pub fn profile(&self) -> HidingProfile {
        self.profile.read().clone()
    }

    pub fn set_profile(&self, profile: HidingProfile) {
        *self.profile.write() = profile;
    }

    pub fn current(&self) -> Option<CurrentEpisode> {
        *self.current.lock()
    }

    pub fn set_current(&self, current: Option<CurrentEpisode>) {
        *self.current.lock() = current;
    }

    /// Credit playback up to `position_sec` of the current file.
    pub fn credit_position(&self, position_sec: f64) {
        self.credit.lock().advance(position_sec);
    }

    /// Start counting again, for a different file or the same one from the top.
    pub fn reset_credit(&self) {
        *self.credit.lock() = WatchCredit::default();
    }

    /// How much of the current file has been played rather than skipped.
    pub fn credited_ms(&self) -> i64 {
        self.credit.lock().credited_ms
    }

    // --- playback ----------------------------------------------------------

    /// Start a file, optionally resuming at `start_ms`.
    ///
    /// The resume point comes from the library database, never from the
    /// frontend, which is why this is not, and must not become, a public
    /// "seek to N" command. See [`Self::seek_relative`].
    pub fn load_file(&self, path: &Path, start_ms: i64) -> Result<()> {
        let path = path.to_str().context("path is not valid UTF-8")?;

        // The path is logged: this is a local, opt-in debug log (see `fail`
        // above), not something that reaches the frontend or a report.
        tracing::debug!(path, start_ms, "loading file into mpv");

        // `start` is read when the next file begins. Setting it here and
        // clearing it afterwards keeps absolute positioning entirely inside
        // Rust.
        if start_ms > 0 {
            let seconds = start_ms as f64 / 1000.0;
            self.mpv
                .set_property("start", format!("{seconds:.3}").as_str())?;
        } else {
            self.mpv.set_property("start", "none")?;
        }

        // `mpv_command` takes an argument vector, so the path goes through
        // unquoted and unescaped: no string mangling for spaces or quotes.
        match self.mpv.command("loadfile", &[path, "replace"]) {
            Ok(()) => tracing::debug!("loadfile command accepted"),
            Err(e) => {
                tracing::error!("loadfile command failed: {e}");
                return Err(e.into());
            }
        }
        Ok(())
    }

    pub fn play_pause(&self) -> Result<()> {
        let paused: bool = self.mpv.get_property("pause").unwrap_or(false);
        self.mpv.set_property("pause", !paused)?;
        Ok(())
    }

    pub fn set_paused(&self, paused: bool) -> Result<()> {
        self.mpv.set_property("pause", paused)?;
        Ok(())
    }

    /// Seek by a delta. There is no absolute counterpart anywhere in this API.
    ///
    /// This is the architectural point, not a UI choice: with no
    /// `seek_absolute`, "jump to 60%" cannot be expressed, and a frontend
    /// cannot derive the running time by asking to seek somewhere and seeing
    /// whether it worked.
    pub fn seek_relative(&self, delta_sec: f64) -> Result<()> {
        // Beyond this a "relative" seek is really an absolute one: ±3 hours
        // from anywhere lands at one end of any episode.
        let delta = delta_sec.clamp(-600.0, 600.0);
        // Under `keep-open=yes` the file stays parked on its last frame after
        // it ended, and a forward seek from there errors out. That would
        // surface as "could not seek" for tapping →10s once too often.
        // Backward seeks still work.
        if delta >= 0.0
            && self
                .mpv
                .get_property::<bool>("eof-reached")
                .unwrap_or(false)
        {
            return Ok(());
        }
        self.mpv
            .command("seek", &[&format!("{delta:.3}"), "relative"])?;
        Ok(())
    }

    pub fn set_volume(&self, volume: f64) -> Result<()> {
        self.mpv.set_property("volume", volume.clamp(0.0, 130.0))?;
        Ok(())
    }

    pub fn set_track(&self, kind: TrackKind, id: Option<i64>) -> Result<()> {
        let property = match kind {
            TrackKind::Audio => "aid",
            TrackKind::Subtitle => "sid",
        };
        match id {
            Some(id) => self.mpv.set_property(property, id)?,
            None => self.mpv.set_property(property, "no")?,
        }
        Ok(())
    }

    /// Stop playback and forget what was playing.
    ///
    /// Clearing the identity is part of stopping, not housekeeping: anything
    /// left in `state.episode` keeps being projected into `PlaybackView` and
    /// stays readable through `peek_episode_identity` long after the viewer
    /// has left the player.
    pub fn stop(&self) -> Result<()> {
        self.mpv.command("stop", &[])?;
        self.set_current(None);
        let mut st = self.state_mut();
        st.episode = crate::privacy::EpisodeIdentity::default();
        st.path = None;
        Ok(())
    }

    /// Position in milliseconds, for writing progress to the database.
    /// Stays in Rust; the frontend has no command that returns this.
    pub fn position_ms(&self) -> Option<i64> {
        self.state.read().position_sec.map(|p| (p * 1000.0) as i64)
    }

    pub fn duration_ms(&self) -> Option<i64> {
        self.state.read().duration_sec.map(|d| (d * 1000.0) as i64)
    }
}

// Safety: every field is behind a lock, and libmpv's client API is documented
// as thread-safe apart from `mpv_wait_event`, which only the event thread calls.
unsafe impl Send for PlayerHandle {}
unsafe impl Sync for PlayerHandle {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Audio,
    Subtitle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeking_earns_no_credit() {
        let mut credit = WatchCredit::default();
        for i in 0..60 {
            credit.advance(i as f64);
        }
        credit.advance(2600.0);
        credit.advance(2601.0);
        assert!(credit.credited_ms < 62_000);
    }

    #[test]
    fn rewinding_does_not_take_credit_back() {
        let mut credit = WatchCredit::default();
        credit.advance(10.0);
        credit.advance(11.0);
        let before = credit.credited_ms;
        credit.advance(0.0);
        assert_eq!(credit.credited_ms, before);
    }

    /// Reproduces the failure seen on a `ru_RU.UTF-8` desktop: GTK sets
    /// `LC_NUMERIC` from the environment and libmpv then refuses to start.
    ///
    /// The test sets a non-C numeric locale by hand, the way `gtk_init` would,
    /// and checks that a player can still be created.
    #[test]
    #[cfg(unix)]
    fn mpv_initialises_under_a_comma_decimal_locale() {
        // Try a few spellings; a minimal container may have none of them
        // generated, in which case the locale stays C and the test still
        // exercises the happy path.
        let applied = [c"ru_RU.UTF-8", c"de_DE.UTF-8", c"fr_FR.UTF-8"]
            .iter()
            .any(|name| unsafe { !libc::setlocale(libc::LC_NUMERIC, name.as_ptr()).is_null() });

        let player = PlayerHandle::new(HidingProfile::standard(), None);

        // Restore, so test ordering in this process cannot matter.
        unsafe { libc::setlocale(libc::LC_NUMERIC, c"C".as_ptr()) };

        assert!(
            player.is_ok(),
            "mpv failed to initialise (non-C locale applied: {applied}): {:?}",
            player.err()
        );
    }
}
