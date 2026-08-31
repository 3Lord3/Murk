//! The IPC boundary.
//!
//! [`PlaybackState`] is what Rust knows. [`PlaybackView`] is what the frontend
//! is told. [`PlaybackView::project`] is the only bridge between them, and it
//! is a *narrowing* one: a field the profile hides is not zeroed or blanked, it
//! is absent from the serialised JSON entirely. A layout bug, an open devtools
//! window or a stray `console.log` therefore cannot surface what was hidden,
//! because it never arrived.

pub mod leaks;
pub mod profile;

use serde::Serialize;
use std::path::PathBuf;

pub use profile::{HidingProfile, PeekMode};

/// An audio or subtitle track as mpv reports it.
#[derive(Debug, Clone, Default)]
pub struct Track {
    pub id: i64,
    pub title: Option<String>,
    pub lang: Option<String>,
    pub selected: bool,
}

/// What Murk knows about the episode being played. Stays in Rust.
#[derive(Debug, Clone, Default)]
pub struct EpisodeIdentity {
    pub label: Option<String>,
    pub season: Option<u32>,
    pub number: Option<u32>,
    /// How many episodes the series has in the library.
    pub count: Option<u32>,
}

/// Everything the backend knows about the current playback. Never serialised.
#[derive(Debug, Clone, Default)]
pub struct PlaybackState {
    pub paused: bool,
    pub idle: bool,
    pub volume: f64,

    pub position_sec: Option<f64>,
    pub duration_sec: Option<f64>,

    pub audio_tracks: Vec<Track>,
    pub subtitle_tracks: Vec<Track>,

    pub episode: EpisodeIdentity,
    /// The path is here for the library layer. It is not in `PlaybackView`
    /// under any profile: a filename is usually the loudest spoiler of all.
    pub path: Option<PathBuf>,
}

impl PlaybackState {
    pub fn remaining_sec(&self) -> Option<f64> {
        match (self.position_sec, self.duration_sec) {
            (Some(p), Some(d)) if d > 0.0 => Some((d - p).max(0.0)),
            _ => None,
        }
    }

    pub fn progress(&self) -> Option<f64> {
        match (self.position_sec, self.duration_sec) {
            (Some(p), Some(d)) if d > 0.0 => Some((p / d).clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

/// A track, with its title already run through the sanitiser.
#[derive(Debug, Clone, Serialize)]
pub struct TrackView {
    pub id: i64,
    /// `None` means "the frontend should show a positional label". It never
    /// means "the title was empty".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    pub selected: bool,
}

impl TrackView {
    fn project(track: &Track, profile: &HidingProfile) -> Self {
        let title = track
            .title
            .as_deref()
            .and_then(leaks::sanitize_track_title)
            // A track title with no episode marker in it can still *be* the
            // episode title. When the profile hides the title, free text in a
            // track name goes with it; `lang` and the track's position are
            // enough to pick a track by.
            .filter(|t| !profile.hide_title || leaks::is_descriptor_only(t));

        Self {
            id: track.id,
            title,
            lang: track.lang.clone(),
            selected: track.selected,
        }
    }
}

/// The single shape that crosses the IPC boundary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackView {
    pub paused: bool,
    pub idle: bool,
    pub volume: f64,
    pub audio_tracks: Vec<TrackView>,
    pub subtitle_tracks: Vec<TrackView>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub position_sec: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_sec: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_sec: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub season_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode_count: Option<u32>,
}

impl PlaybackView {
    pub fn project(state: &PlaybackState, profile: &HidingProfile) -> Self {
        // Reads the same way every time, so a new field is hard to add the
        // wrong way round.
        fn reveal<T>(hidden: bool, value: Option<T>) -> Option<T> {
            if hidden {
                None
            } else {
                value
            }
        }

        Self {
            paused: state.paused,
            idle: state.idle,
            volume: state.volume,
            audio_tracks: state
                .audio_tracks
                .iter()
                .map(|t| TrackView::project(t, profile))
                .collect(),
            subtitle_tracks: state
                .subtitle_tracks
                .iter()
                .map(|t| TrackView::project(t, profile))
                .collect(),

            position_sec: reveal(profile.hide_position, state.position_sec),
            duration_sec: reveal(profile.hide_duration, state.duration_sec),
            remaining_sec: reveal(profile.hide_remaining, state.remaining_sec()),
            progress: reveal(profile.hide_progress_bar, state.progress()),

            episode_label: reveal(profile.hide_title, state.episode.label.clone()),
            season_number: reveal(profile.hide_season_number, state.episode.season),
            episode_number: reveal(profile.hide_episode_number, state.episode.number),
            episode_count: reveal(profile.hide_episode_count, state.episode.count),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded_state() -> PlaybackState {
        PlaybackState {
            paused: false,
            idle: false,
            volume: 100.0,
            position_sec: Some(1234.0),
            duration_sec: Some(2700.0),
            audio_tracks: vec![Track {
                id: 1,
                title: Some("S03E07 - The Bear and the Maiden Fair".into()),
                lang: Some("eng".into()),
                selected: true,
            }],
            subtitle_tracks: vec![],
            episode: EpisodeIdentity {
                label: Some("The Bear and the Maiden Fair".into()),
                season: Some(3),
                number: Some(7),
                count: Some(24),
            },
            path: Some("/home/user/Series/GoT/S03E07.mkv".into()),
        }
    }

    /// The regression barrier the whole architecture exists for.
    #[test]
    fn full_darkness_profile_leaks_nothing() {
        let json = serde_json::to_string(&PlaybackView::project(
            &loaded_state(),
            &HidingProfile::full_darkness(),
        ))
        .unwrap();

        for key in [
            "position",
            "duration",
            "remaining",
            "progress",
            "episode",
            "season",
            "count",
            "1234",
            "2700",
            "24",
            "Bear",
            "Maiden",
        ] {
            assert!(!json.contains(key), "leaked `{key}` in: {json}");
        }
    }

    #[test]
    fn standard_profile_hides_the_same_fields_as_full_darkness() {
        // The two presets differ only in whether peeking is allowed; the
        // passively emitted view must be identical.
        let state = loaded_state();
        let dark = serde_json::to_string(&PlaybackView::project(
            &state,
            &HidingProfile::full_darkness(),
        ))
        .unwrap();
        let std = serde_json::to_string(&PlaybackView::project(&state, &HidingProfile::standard()))
            .unwrap();
        assert_eq!(dark, std);
    }

    #[test]
    fn soft_profile_reveals_the_bar_but_no_numbers() {
        let view = PlaybackView::project(&loaded_state(), &HidingProfile::soft());
        assert!(view.progress.is_some(), "soft profile should show the bar");
        assert!(view.position_sec.is_none());
        assert!(view.duration_sec.is_none());
        assert!(view.remaining_sec.is_none());
        assert!(view.episode_count.is_none());

        let json = serde_json::to_string(&view).unwrap();
        for key in [
            "positionSec",
            "durationSec",
            "remainingSec",
            "episodeCount",
            "1234",
            "2700",
        ] {
            assert!(!json.contains(key), "leaked `{key}` in: {json}");
        }
    }

    #[test]
    fn no_profile_ever_exposes_the_file_path() {
        for profile in HidingProfile::presets() {
            let json =
                serde_json::to_string(&PlaybackView::project(&loaded_state(), &profile)).unwrap();
            assert!(
                !json.contains("GoT"),
                "path leaked under {}: {json}",
                profile.id
            );
            assert!(
                !json.contains(".mkv"),
                "path leaked under {}: {json}",
                profile.id
            );
            assert!(
                !json.contains("path"),
                "path leaked under {}: {json}",
                profile.id
            );
        }
    }

    #[test]
    fn episode_markers_are_cut_from_track_titles_under_every_profile() {
        for profile in HidingProfile::presets() {
            let view = PlaybackView::project(&loaded_state(), &profile);
            let json = serde_json::to_string(&view).unwrap();
            assert!(
                !json.contains("S03E07"),
                "under profile {}: {json}",
                profile.id
            );
        }
    }

    #[test]
    fn a_track_title_cannot_smuggle_the_episode_title_back_in() {
        for profile in [HidingProfile::full_darkness(), HidingProfile::standard()] {
            let view = PlaybackView::project(&loaded_state(), &profile);
            assert_eq!(view.audio_tracks[0].title, None, "profile {}", profile.id);
            assert_eq!(view.audio_tracks[0].lang.as_deref(), Some("eng"));
        }

        // The soft profile shows titles on purpose, minus the episode marker.
        let soft = PlaybackView::project(&loaded_state(), &HidingProfile::soft());
        assert_eq!(
            soft.audio_tracks[0].title.as_deref(),
            Some("The Bear and the Maiden Fair")
        );
    }

    #[test]
    fn descriptor_track_titles_survive_full_darkness() {
        let mut state = loaded_state();
        state.audio_tracks = vec![
            Track {
                id: 1,
                title: Some("English".into()),
                lang: Some("eng".into()),
                selected: true,
            },
            Track {
                id: 2,
                title: Some("Commentary 5.1".into()),
                lang: Some("eng".into()),
                selected: false,
            },
        ];
        let view = PlaybackView::project(&state, &HidingProfile::full_darkness());
        assert_eq!(view.audio_tracks[0].title.as_deref(), Some("English"));
        assert_eq!(
            view.audio_tracks[1].title.as_deref(),
            Some("Commentary 5.1")
        );
    }

    #[test]
    fn idle_state_emits_no_optional_fields_at_all() {
        let json = serde_json::to_string(&PlaybackView::project(
            &PlaybackState {
                idle: true,
                ..Default::default()
            },
            &HidingProfile::soft(),
        ))
        .unwrap();
        assert!(!json.contains("progress"), "{json}");
    }
}
