//! The `#[tauri::command]` surface.
//!
//! The list of what the frontend *can* ask for. There is no `seek_absolute`,
//! no `get_position`, no `list_episodes` and no `get_current_path`: they do
//! not exist to be called.

use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

use crate::library::{poster, scan, WATCHED_FRACTION};
use crate::player::{events, inhibit, TrackKind};
use crate::privacy::{HidingProfile, PeekMode, PlaybackView};
use crate::AppState;

/// Commands report failure as a stable code, never as a sentence: an error
/// carrying a path or a position would be a leak, and a sentence would be in
/// one language only. The frontend looks the code up in its catalogues, so
/// renaming a code means renaming it there too.
pub type CommandResult<T> = Result<T, String>;

/// Log the real error (which may name a path; the log is local) and hand the
/// frontend the code alone.
fn fail<E: std::fmt::Display>(code: &'static str) -> impl Fn(E) -> String {
    move |e| {
        tracing::warn!("{code}: {e}");
        code.to_string()
    }
}

// --- playback ---------------------------------------------------------------

#[tauri::command]
pub fn get_playback(state: State<'_, AppState>) -> PlaybackView {
    PlaybackView::project(&state.player.state(), &state.player.profile())
}

#[tauri::command]
pub fn play_pause(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    state
        .player
        .play_pause()
        .map_err(fail("play_pause_failed"))?;
    let paused = state.player.state().paused;
    inhibit::set_inhibited(&app, !paused);
    events::emit_playback(&app);
    Ok(())
}

/// Seek by a delta only. See `PlayerHandle::seek_relative` for why there is no
/// absolute form.
#[tauri::command]
pub fn seek_relative(
    app: AppHandle,
    state: State<'_, AppState>,
    delta_sec: f64,
) -> CommandResult<()> {
    state
        .player
        .seek_relative(delta_sec)
        .map_err(fail("seek_failed"))?;
    events::emit_playback(&app);
    Ok(())
}

#[tauri::command]
pub fn set_volume(app: AppHandle, state: State<'_, AppState>, volume: f64) -> CommandResult<()> {
    state
        .player
        .set_volume(volume)
        .map_err(fail("volume_failed"))?;
    events::emit_playback(&app);
    Ok(())
}

#[tauri::command]
pub fn set_track(
    app: AppHandle,
    state: State<'_, AppState>,
    kind: TrackKind,
    id: Option<i64>,
) -> CommandResult<()> {
    state
        .player
        .set_track(kind, id)
        .map_err(fail("track_switch_failed"))?;

    // mpv's track ids are per-file, so the language is what gets remembered.
    if kind == TrackKind::Subtitle {
        remember_subtitle_preference(&state, id);
    }

    events::emit_playback(&app);
    Ok(())
}

/// Store the chosen subtitle language for the series currently playing, or
/// "off" when the user turned subtitles off, so the choice survives an episode
/// change and an app restart.
fn remember_subtitle_preference(state: &State<'_, AppState>, id: Option<i64>) {
    let Some(current) = state.player.current() else {
        return;
    };
    let key = format!("subtitle_lang:{}", current.series_id);
    // "off" is stored verbatim and is not a real language code. A track whose
    // language mpv did not report is left at the previous value.
    let value = match id {
        Some(id) => match state
            .player
            .state()
            .subtitle_tracks
            .iter()
            .find(|t| t.id == id)
            .and_then(|t| t.lang.clone())
        {
            Some(lang) => lang,
            None => return,
        },
        None => "off".to_string(),
    };
    if let Err(e) = state.library.set_setting(&key, &value) {
        tracing::warn!("could not remember subtitle choice: {e}");
    }
}

#[tauri::command]
pub fn stop(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    events::save_progress(&app, false);
    state.player.stop().map_err(fail("stop_failed"))?;
    inhibit::set_inhibited(&app, false);
    events::emit_playback(&app);
    Ok(())
}

#[tauri::command]
pub fn toggle_fullscreen(app: AppHandle) -> CommandResult<bool> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "no_window".to_string())?;
    let full = window.is_fullscreen().map_err(fail("fullscreen_failed"))?;
    window
        .set_fullscreen(!full)
        .map_err(fail("fullscreen_failed"))?;
    Ok(!full)
}

/// Set fullscreen to an explicit state. Used on leaving the player, where
/// toggling would be wrong: we want to end up windowed no matter what.
#[tauri::command]
pub fn set_fullscreen(app: AppHandle, full: bool) -> CommandResult<()> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "no_window".to_string())?;
    let is_full = window.is_fullscreen().map_err(fail("fullscreen_failed"))?;
    if is_full != full {
        window
            .set_fullscreen(full)
            .map_err(fail("fullscreen_failed"))?;
    }
    Ok(())
}

// --- library ----------------------------------------------------------------

/// A series card. Note what is *not* here: no episode count, no "next episode"
/// title, no progress percentage. A card and a Continue button.
///
/// `poster` is a data URL, `null` when the series has no cover. Covers are
/// deliberately *not* gated on the hiding profile: they are the user's own
/// files, and a cover is not a spoiler.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesCard {
    pub id: i64,
    pub display_name: String,
    /// Whether the series has been watched into and still has something left,
    /// as opposed to being untouched or finished. A boolean, so it cannot say
    /// *where* in the series.
    pub in_progress: bool,
    /// Whether anything is stored for this series at all, including a series
    /// watched to the end. What "reset progress" acts on.
    pub has_progress: bool,
    /// How far the whole work has been watched, 0 to 1: the folder end to
    /// end, not the episode in hand. Absent under every profile that hides the
    /// progress bar, so the card cannot say more than the player does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    pub poster: Option<String>,
}

#[tauri::command]
pub fn list_series(state: State<'_, AppState>) -> CommandResult<Vec<SeriesCard>> {
    let series = state
        .library
        .list_series()
        .map_err(fail("library_read_failed"))?;
    let mut cache = state.poster_cache.lock();
    let show_progress = !state.player.profile().hide_progress_bar;
    Ok(series
        .into_iter()
        .map(|s| {
            let has_progress = state.library.has_progress(s.id).unwrap_or(false);
            // "Continue" is about the series, not a half-watched file:
            // anything remembered plus anything left to play.
            let in_progress =
                has_progress && state.library.resume_target(s.id).ok().flatten().is_some();
            let progress = show_progress
                .then(|| state.library.series_progress(s.id).ok().flatten())
                .flatten();
            let poster = s
                .poster_path
                .as_deref()
                .filter(|p| p.is_file())
                .and_then(|p| cache.get(p));
            SeriesCard {
                id: s.id,
                display_name: s.display_name,
                in_progress,
                has_progress,
                progress,
                poster,
            }
        })
        .collect())
}

/// Pick a cover for a series if it does not already have one: first a file
/// the user placed in the folder, then a picture embedded in the first
/// episode. A poster chosen by hand is never overwritten by this.
fn discover_poster(state: &AppState, series_id: i64) {
    let Ok(Some(series)) = state.library.series(series_id) else {
        return;
    };
    if let Some(poster) = &series.poster_path {
        if poster.is_file() {
            return;
        }
    }

    let poster = poster::local_poster(&series.root_path).or_else(|| {
        state
            .library
            .first_episode(series_id)
            .ok()
            .flatten()
            .and_then(|episode| {
                poster::extract_embedded(&episode.path, &state.covers_dir, series_id)
            })
    });
    if poster.is_some() {
        let _ = state.library.set_poster(series_id, poster.as_deref());
    }
}

/// Add a series by **folder**.
///
/// The file chooser is only ever opened at folder level: its list view prints
/// filenames, and `S02E08 - Endings and Beginnings.mkv` is a spoiler the user
/// cannot unsee. The folder name they navigate to, they have already read.
#[tauri::command]
pub fn add_series(state: State<'_, AppState>, path: PathBuf) -> CommandResult<i64> {
    if !path.is_dir() {
        return Err("not_a_folder".into());
    }
    let name = scan::display_name_for(&path);
    let id = state
        .library
        .add_series(&path, &name)
        .map_err(fail("could_not_add_series"))?;
    let found = scan::scan_series_folder(&path);
    state
        .library
        .sync_episodes(id, &found)
        .map_err(fail("could_not_index_series"))?;
    discover_poster(&state, id);
    Ok(id)
}

#[tauri::command]
pub fn rescan_series(state: State<'_, AppState>, series_id: i64) -> CommandResult<()> {
    let series = state
        .library
        .list_series()
        .map_err(fail("library_read_failed"))?
        .into_iter()
        .find(|s| s.id == series_id)
        .ok_or_else(|| "no_such_series".to_string())?;
    let found = scan::scan_series_folder(&series.root_path);
    state
        .library
        .sync_episodes(series_id, &found)
        .map_err(fail("rescan_failed"))?;
    discover_poster(&state, series_id);
    Ok(())
}

/// Set a series' cover from a file the user picked. The image is copied into
/// the app's own covers directory so the card keeps working even if the
/// original is moved or the folder is replaced.
#[tauri::command]
pub fn set_series_poster(
    state: State<'_, AppState>,
    series_id: i64,
    path: PathBuf,
) -> CommandResult<()> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| poster::ACCEPTED_POSTER_EXTENSIONS.contains(&e.as_str()))
        .ok_or_else(|| "unsupported_image_type".to_string())?;
    let source = std::fs::read(&path).map_err(fail("could_not_read_image"))?;
    if source.is_empty() {
        return Err("image_empty".into());
    }
    if source.len() > poster::MAX_DATA_URL {
        return Err("image_too_large".into());
    }

    // Clear any previously cached cover (same series id, any extension).
    if let Ok(entries) = std::fs::read_dir(&state.covers_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(&format!("{series_id}.")) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    let dest = state.covers_dir.join(format!("{series_id}.{ext}"));
    std::fs::write(&dest, source).map_err(fail("could_not_save_cover"))?;
    state
        .library
        .set_poster(series_id, Some(&dest))
        .map_err(fail("could_not_save_cover"))
}

/// Remove a series' cover. Files the user placed in the series folder are left
/// untouched; only the database entry is cleared.
#[tauri::command]
pub fn clear_series_poster(state: State<'_, AppState>, series_id: i64) -> CommandResult<()> {
    let poster_path = state
        .library
        .series(series_id)
        .map_err(fail("library_read_failed"))?
        .and_then(|s| s.poster_path);
    if let Some(path) = poster_path {
        // Delete only our own cached copies, never a file in the user's folder.
        if path.starts_with(&state.covers_dir) {
            let _ = std::fs::remove_file(&path);
        }
    }
    state
        .library
        .set_poster(series_id, None)
        .map_err(fail("could_not_clear_cover"))
}

#[tauri::command]
pub fn remove_series(state: State<'_, AppState>, series_id: i64) -> CommandResult<()> {
    state
        .library
        .remove_series(series_id)
        .map_err(fail("could_not_remove_series"))
}

/// Forget every saved position and watched flag of a series.
#[tauri::command]
pub fn reset_progress(state: State<'_, AppState>, series_id: i64) -> CommandResult<()> {
    state
        .library
        .reset_progress(series_id)
        .map_err(fail("could_not_reset_progress"))
}

/// Start, or continue, a series.
///
/// Which file this is and where it resumes are decided here, in Rust, from the
/// database. The frontend passes a series id and receives nothing back but
/// success.
#[tauri::command]
pub fn continue_series(
    app: AppHandle,
    state: State<'_, AppState>,
    series_id: i64,
) -> CommandResult<()> {
    // A series watched to the end has no resume target, so it falls back to
    // the first episode: a rewatch, which is why the card says "Start".
    // Same predicate the card's label is built from, so the button the user
    // pressed and the behaviour they get cannot drift apart.
    let continuing = state.library.has_progress(series_id).unwrap_or(false)
        && state
            .library
            .resume_target(series_id)
            .ok()
            .flatten()
            .is_some();

    let episode = match state
        .library
        .resume_target(series_id)
        .map_err(fail("library_read_failed"))?
    {
        Some(episode) => episode,
        None => state
            .library
            .first_episode(series_id)
            .map_err(fail("library_read_failed"))?
            .ok_or_else(|| "no_video_files".to_string())?,
    };

    let start_ms = state.library.resume_position_ms(episode.id).unwrap_or(0);
    // "Continue" waits, paused, on the frame it will go on from, so the
    // viewer does not miss the first minute. "Start" and auto-advance play at
    // once.
    play_episode(&app, &state, &episode, start_ms, continuing)
}

/// Accept the auto-advance the backend queued at end of file.
#[tauri::command]
pub fn play_next(app: AppHandle, state: State<'_, AppState>) -> CommandResult<()> {
    let next_id = state.pending_next.lock().take();
    let episode = next_id
        .and_then(|id| state.library.episode(id).ok().flatten())
        .ok_or_else(|| "nothing_next".to_string())?;
    play_episode(&app, &state, &episode, 0, false)
}

#[tauri::command]
pub fn cancel_next(state: State<'_, AppState>) {
    *state.pending_next.lock() = None;
}

fn play_episode(
    app: &AppHandle,
    state: &State<'_, AppState>,
    episode: &crate::library::EpisodeRow,
    start_ms: i64,
    paused: bool,
) -> CommandResult<()> {
    let count = state.library.episode_count(episode.series_id).unwrap_or(0);

    state
        .player
        .set_current(Some(crate::player::CurrentEpisode {
            episode_id: episode.id,
            series_id: episode.series_id,
        }));

    {
        let mut st = state.player.state_mut();
        st.episode = crate::privacy::EpisodeIdentity {
            // The file stem: stored, projected away by every profile that
            // hides the title, and never used as the window title.
            label: episode
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_string),
            season: episode.season,
            number: episode.number,
            count: Some(count),
        };
        st.path = Some(episode.path.clone());
    }

    // Set before loading: `pause` is global, and setting it after `loadfile`
    // would let a second of the opening play first.
    state.player.set_paused(paused).ok();
    state
        .player
        .load_file(&episode.path, start_ms)
        .map_err(fail("could_not_start_playback"))?;
    inhibit::set_inhibited(app, true);
    events::emit_playback(app);
    Ok(())
}

// --- profiles ---------------------------------------------------------------

#[tauri::command]
pub fn get_profile(state: State<'_, AppState>) -> HidingProfile {
    state.player.profile()
}

#[tauri::command]
pub fn list_profiles() -> Vec<HidingProfile> {
    HidingProfile::presets()
}

#[tauri::command]
pub fn set_profile(
    app: AppHandle,
    state: State<'_, AppState>,
    profile_id: String,
) -> CommandResult<HidingProfile> {
    let profile = HidingProfile::preset(&profile_id).ok_or_else(|| {
        tracing::warn!("unknown profile: {profile_id}");
        "unknown_profile".to_string()
    })?;
    state.player.set_profile(profile.clone());
    state
        .library
        .set_setting("hiding_profile", &profile.id)
        .map_err(fail("could_not_save_profile"))?;
    events::emit_playback(&app);
    Ok(profile)
}

// --- peeking ----------------------------------------------------------------

/// Granularity of [`can_finish_within`], in minutes.
///
/// Repeating the question with different values is a binary search on the
/// running time. Quantising the threshold caps what that search can recover at
/// this many minutes, whatever the caller does.
const COARSE_STEP_MIN: u32 = 5;

/// Exact remaining time, in the `Confirmed` mode, after the user has confirmed.
///
/// The confirmation itself happens in the frontend; this command trusts that
/// only because the profile already decided the answer may be shown at all.
#[tauri::command]
pub fn peek_remaining(state: State<'_, AppState>) -> CommandResult<f64> {
    let profile = state.player.profile();
    if profile.peek != PeekMode::Confirmed {
        return Err("peek_disabled".into());
    }
    state
        .player
        .state()
        .remaining_sec()
        .ok_or_else(|| "nothing_playing".to_string())
}

/// "Will it finish within N minutes?": a boolean, and nothing else.
///
/// This works in `Coarse` as well as `Confirmed`, because it answers the
/// practical question without revealing the position, the running time, or how
/// far through the episode the user is.
#[tauri::command]
pub fn can_finish_within(state: State<'_, AppState>, minutes: u32) -> CommandResult<bool> {
    let profile = state.player.profile();
    if profile.peek == PeekMode::Disabled {
        return Err("peek_disabled".into());
    }
    let remaining = state
        .player
        .state()
        .remaining_sec()
        .ok_or_else(|| "nothing_playing".to_string())?;

    let quantised = minutes.div_ceil(COARSE_STEP_MIN) * COARSE_STEP_MIN;
    Ok(remaining <= quantised as f64 * 60.0)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeIdentityView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<u32>,
}

#[tauri::command]
pub fn peek_episode_identity(state: State<'_, AppState>) -> CommandResult<EpisodeIdentityView> {
    let profile = state.player.profile();
    if profile.peek != PeekMode::Confirmed {
        return Err("peek_disabled".into());
    }
    let st = state.player.state();
    // Without this the command would answer with whatever was playing last.
    if st.idle {
        return Err("nothing_playing".into());
    }
    Ok(EpisodeIdentityView {
        season: st.episode.season,
        number: st.episode.number,
    })
}

/// How close to the end an episode counts as watched. Exposed only so the
/// settings screen can explain the rule, not as a live figure.
#[tauri::command]
pub fn watched_fraction() -> f64 {
    WATCHED_FRACTION
}

// --- interface language ------------------------------------------------------

/// The stored value is `"system"` or a locale code the frontend knows. It is
/// deliberately opaque here: which languages exist is a frontend fact, and the
/// backend has no business rejecting a locale a newer UI understands.
#[tauri::command]
pub fn get_locale(state: State<'_, AppState>) -> CommandResult<String> {
    Ok(state
        .library
        .get_setting("ui_locale")
        .map_err(fail("could_not_read_setting"))?
        .unwrap_or_else(|| "system".to_string()))
}

#[tauri::command]
pub fn set_locale(state: State<'_, AppState>, locale: String) -> CommandResult<()> {
    state
        .library
        .set_setting("ui_locale", &locale)
        .map_err(fail("could_not_save_setting"))
}

/// The system's interface languages, most preferred first, as BCP-47-ish tags
/// (`ru-RU`).
///
/// The frontend cannot ask the webview for this: WebKitGTK answers
/// `navigator.language` with `en-US` no matter what the session is set to,
/// unless the embedder tells it otherwise, so an AppImage started under
/// `LANG=ru_RU.UTF-8` came up in English. The environment is the authority on
/// Linux, and only the backend can read it.
///
/// `LANGUAGE` is a colon-separated preference list and wins when set; the
/// `LC_ALL` / `LC_MESSAGES` / `LANG` chain is the usual POSIX fallback order.
/// `C` and `POSIX` name the absence of a language, so they are dropped rather
/// than offered as a tag.
#[tauri::command]
pub fn system_languages() -> Vec<String> {
    fn env(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    /// `ru_RU.UTF-8@euro` -> `ru-RU`: the encoding and the modifier say nothing
    /// about which language to show.
    fn tag(value: &str) -> Option<String> {
        let name = value.split(['.', '@']).next().unwrap_or_default();
        if name.is_empty() || name == "C" || name == "POSIX" {
            return None;
        }
        Some(name.replace('_', "-"))
    }

    let mut tags: Vec<String> = Vec::new();
    let mut push = |value: &str| {
        if let Some(t) = tag(value) {
            if !tags.contains(&t) {
                tags.push(t);
            }
        }
    };

    if let Some(list) = env("LANGUAGE") {
        for value in list.split(':') {
            push(value);
        }
    }
    for name in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = env(name) {
            push(&value);
        }
    }
    tags
}
