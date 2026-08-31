//! mpv's event stream, turned into events the frontend is allowed to see.
//!
//! Everything mpv reports lands in [`PlaybackState`] inside Rust. What leaves
//! this module is a `PlaybackView`, produced by the projection in
//! [`crate::privacy`]. There is no other emitter of playback data anywhere in
//! the program.

use libmpv2::events::{Event, PropertyData};
use libmpv2::{Format, Mpv};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::library::WATCHED_FRACTION;
use crate::player::TrackKind;
use crate::privacy::{PlaybackView, Track};
use crate::AppState;

/// The event name the frontend listens on. There is exactly one.
pub const PLAYBACK_EVENT: &str = "murk://playback";
/// Fired when a file ends and the next one is queued, so the UI can show its
/// unlabelled countdown.
pub const ADVANCE_EVENT: &str = "murk://advance";

/// Property observation ids.
mod prop {
    pub const PAUSE: u64 = 1;
    pub const TIME_POS: u64 = 2;
    pub const DURATION: u64 = 3;
    pub const EOF_REACHED: u64 = 4;
    pub const TRACK_LIST: u64 = 5;
    pub const VOLUME: u64 = 6;
    pub const IDLE_ACTIVE: u64 = 7;
}

/// How often a view is pushed to the frontend while playing.
///
/// `time-pos` fires several times a second. Even though the position is usually
/// projected away, re-serialising and re-emitting at that rate is pure waste.
const EMIT_INTERVAL: Duration = Duration::from_millis(250);
/// How often the resume point is written to the database.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

pub fn observe_properties(mpv: &Mpv) -> anyhow::Result<()> {
    mpv.observe_property("pause", Format::Flag, prop::PAUSE)?;
    mpv.observe_property("time-pos", Format::Double, prop::TIME_POS)?;
    mpv.observe_property("duration", Format::Double, prop::DURATION)?;
    mpv.observe_property("eof-reached", Format::Flag, prop::EOF_REACHED)?;
    mpv.observe_property("volume", Format::Double, prop::VOLUME)?;
    // `idle-active`, not `core-idle`: the latter is also true whenever playback
    // is merely paused or buffering, and this flag means "no file is loaded".
    mpv.observe_property("idle-active", Format::Flag, prop::IDLE_ACTIVE)?;
    // The track list is an mpv node, which this binding exposes only as JSON.
    // Observing the *count* gives a plain integer that changes whenever the
    // track set does; the details are read back one sub-property at a time.
    mpv.observe_property("track-list/count", Format::Int64, prop::TRACK_LIST)?;
    Ok(())
}

/// Read the track list out of mpv one sub-property at a time.
fn read_tracks(mpv: &Mpv) -> (Vec<Track>, Vec<Track>) {
    let mut audio = Vec::new();
    let mut subs = Vec::new();

    let count: i64 = mpv.get_property("track-list/count").unwrap_or(0);
    for i in 0..count {
        let kind: String = match mpv.get_property(&format!("track-list/{i}/type")) {
            Ok(k) => k,
            Err(_) => continue,
        };
        let track = Track {
            id: mpv.get_property(&format!("track-list/{i}/id")).unwrap_or(0),
            title: mpv
                .get_property::<String>(&format!("track-list/{i}/title"))
                .ok(),
            lang: mpv
                .get_property::<String>(&format!("track-list/{i}/lang"))
                .ok(),
            selected: mpv
                .get_property::<bool>(&format!("track-list/{i}/selected"))
                .unwrap_or(false),
        };
        match kind.as_str() {
            "audio" => audio.push(track),
            "sub" => subs.push(track),
            _ => {}
        }
    }
    (audio, subs)
}

/// If the user chose a subtitle language for this series on a previous episode,
/// re-select the matching track now that the file is loaded. Returns whether
/// the selection was changed, so the caller can re-read the track list for
/// truthful `selected` flags.
///
/// mpv assigns fresh track ids per file, so a language is the only stable
/// identity that survives the switch. When there is no match, nothing is
/// forced and mpv keeps its default.
fn apply_preferred_subtitle(state: &tauri::State<'_, AppState>, subs: &[Track]) -> bool {
    let Some(current) = state.player.current() else {
        return false;
    };
    let Ok(Some(pref)) = state.library.preferred_subtitle_lang(current.series_id) else {
        return false;
    };
    if pref == "off" {
        if subs.iter().any(|t| t.selected) {
            if let Err(e) = state.player.set_track(TrackKind::Subtitle, None) {
                tracing::warn!("could not apply remembered subtitle: {e}");
                return false;
            }
            return true;
        }
        return false;
    }
    let Some(target) = subs
        .iter()
        .find(|t| t.lang.as_deref() == Some(pref.as_str()))
    else {
        return false;
    };
    if target.selected {
        return false;
    }
    if let Err(e) = state.player.set_track(TrackKind::Subtitle, Some(target.id)) {
        tracing::warn!("could not apply remembered subtitle: {e}");
        return false;
    }
    true
}

/// Run the mpv event loop until the player shuts down.
///
/// Owns the only `wait_event` call in the process.
pub fn run(app: AppHandle, shutdown: Arc<AtomicBool>) {
    let state: tauri::State<'_, AppState> = app.state();
    let mpv = state.player.mpv();

    if let Err(e) = observe_properties(mpv) {
        tracing::error!("could not observe mpv properties: {e}");
        return;
    }

    let mut last_emit = Instant::now() - EMIT_INTERVAL;
    let mut last_progress = Instant::now();
    let mut advancing = false;

    while !shutdown.load(Ordering::Relaxed) {
        // `wait_event` returns `None` both on timeout and when a property
        // became *unavailable* (format NONE, which the binding swallows).
        // Either way there is nothing to apply, but the periodic emit stands.
        let Some(event) = mpv.wait_event(0.2) else {
            maybe_emit(&app, &mut last_emit);
            continue;
        };

        let event = match event {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("mpv event error: {e}");
                continue;
            }
        };

        let mut force_emit = false;

        match event {
            Event::PropertyChange {
                reply_userdata,
                change,
                ..
            } => {
                let mut st = state.player.state_mut();
                match (reply_userdata, change) {
                    (prop::PAUSE, PropertyData::Flag(v)) => {
                        st.paused = v;
                        force_emit = true;
                    }
                    (prop::IDLE_ACTIVE, PropertyData::Flag(v)) => {
                        st.idle = v;
                        force_emit = true;
                    }
                    (prop::TIME_POS, PropertyData::Double(v)) => {
                        st.position_sec = Some(v);
                        drop(st);
                        // One step at a time, so seeking into the last
                        // minutes of an episode does not finish it.
                        state.player.credit_position(v);
                        continue;
                    }
                    (prop::DURATION, PropertyData::Double(v)) => {
                        st.duration_sec = Some(v);
                        force_emit = true;
                    }
                    (prop::VOLUME, PropertyData::Double(v)) => {
                        st.volume = v;
                        force_emit = true;
                    }
                    (prop::EOF_REACHED, PropertyData::Flag(true)) => {
                        drop(st);
                        if !advancing {
                            advancing = true;
                            handle_end_of_file(&app);
                        }
                        continue;
                    }
                    // The latch is released here and nowhere else. Under
                    // `keep-open=yes` mpv parks on the last frame with
                    // `eof-reached` still true, so a change notification can
                    // arrive after the *next* file has started; clearing the
                    // flag on `StartFile` would let that stale event end an
                    // episode a second into it.
                    (prop::EOF_REACHED, PropertyData::Flag(false)) => advancing = false,
                    (prop::TRACK_LIST, _) => {
                        drop(st);
                        let (audio, subs) = read_tracks(mpv);
                        let mut st = state.player.state_mut();
                        st.audio_tracks = audio;
                        st.subtitle_tracks = subs;
                        force_emit = true;
                    }
                    _ => {}
                }
            }
            Event::StartFile => {
                state.player.reset_credit();
                let mut st = state.player.state_mut();
                st.idle = false;
                st.position_sec = None;
                st.duration_sec = None;
                force_emit = true;
            }
            Event::FileLoaded => {
                // Record the running time so `can_finish_within` has an
                // answer even before playback produces a position.
                if let (Some(current), Ok(duration)) =
                    (state.player.current(), mpv.get_property::<f64>("duration"))
                {
                    let _ = state
                        .library
                        .record_duration(current.episode_id, (duration * 1000.0) as i64);
                }
                // After the file is fully loaded: mpv finalises its default
                // track selection during load and clobbers a `sid` set when
                // the track list first appeared. `read_tracks` runs again only
                // when a change was made, so `selected` stays truthful.
                let (audio, subs) = read_tracks(mpv);
                let (audio, subs) = if apply_preferred_subtitle(&state, &subs) {
                    read_tracks(mpv)
                } else {
                    (audio, subs)
                };
                let mut st = state.player.state_mut();
                st.audio_tracks = audio;
                st.subtitle_tracks = subs;
                force_emit = true;
            }
            Event::EndFile(_) => {
                let mut st = state.player.state_mut();
                st.idle = true;
                force_emit = true;
            }
            Event::Shutdown => break,
            _ => {}
        }

        if last_progress.elapsed() >= PROGRESS_INTERVAL {
            last_progress = Instant::now();
            save_progress(&app, false);
        }

        if force_emit {
            last_emit = Instant::now() - EMIT_INTERVAL;
        }
        maybe_emit(&app, &mut last_emit);
    }

    // A last write, so closing mid-episode resumes where the user was.
    save_progress(&app, false);
}

fn maybe_emit(app: &AppHandle, last_emit: &mut Instant) {
    if last_emit.elapsed() < EMIT_INTERVAL {
        return;
    }
    *last_emit = Instant::now();
    emit_playback(app);
}

/// The single point where playback data crosses the IPC boundary.
pub fn emit_playback(app: &AppHandle) {
    let state: tauri::State<'_, AppState> = app.state();
    let view = PlaybackView::project(&state.player.state(), &state.player.profile());
    if let Err(e) = app.emit(PLAYBACK_EVENT, view) {
        tracing::warn!("could not emit playback view: {e}");
    }
}

/// Write the resume point. `finished` forces the watched flag on.
pub fn save_progress(app: &AppHandle, finished: bool) {
    let state: tauri::State<'_, AppState> = app.state();
    let Some(current) = state.player.current() else {
        return;
    };
    let position_ms = state.player.position_ms();

    // At end of file mpv may already have dropped `time-pos`; the watched flag
    // still has to land, or the episode never counts as finished and the series
    // stops advancing. Only an ordinary mid-episode write needs a position.
    if position_ms.is_none() && !finished {
        return;
    }

    // Not "how far in is the playhead" but "how much was actually played":
    // dragging the bar to the last minutes does not finish an episode.
    let watched = finished
        || match state.player.duration_ms() {
            Some(d) if d > 0 => state.player.credited_ms() as f64 / d as f64 >= WATCHED_FRACTION,
            _ => false,
        };

    // A watched episode resumes from the start anyway, so an unknown position
    // is recorded as zero rather than guessed at.
    let position_ms = position_ms.unwrap_or(0);

    if let Err(e) = state
        .library
        .save_progress(current.episode_id, position_ms, watched)
    {
        tracing::warn!("could not save progress: {e}");
    }
}

/// End of file: mark it watched, look up what follows, tell the frontend.
///
/// The event carries no title, no number and no countdown target, only the
/// fact that something follows. The frontend shows an unlabelled countdown
/// with a cancel button.
fn handle_end_of_file(app: &AppHandle) {
    let state: tauri::State<'_, AppState> = app.state();
    save_progress(app, true);

    // `keep-open=yes` parks mpv on the last frame rather than stopping, so
    // nothing else would ever release the screensaver inhibit. Without this the
    // display stays lit all night after the last episode.
    crate::player::inhibit::set_inhibited(app, false);

    let Some(current) = state.player.current() else {
        return;
    };
    let next = state
        .library
        .episode(current.episode_id)
        .ok()
        .flatten()
        .and_then(|episode| state.library.following(&episode).ok().flatten());

    let has_next = next.is_some();
    *state.pending_next.lock() = next.map(|e| e.id);

    if let Err(e) = app.emit(ADVANCE_EVENT, serde_json::json!({ "hasNext": has_next })) {
        tracing::warn!("could not emit advance event: {e}");
    }
}
