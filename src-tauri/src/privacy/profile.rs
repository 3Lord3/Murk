//! What may be revealed, and under what conditions.

use serde::{Deserialize, Serialize};

/// How far a user is allowed to deliberately lift the veil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeekMode {
    /// Nothing can be revealed. The peek commands return an error.
    Disabled,
    /// Only yes/no answers to "will this finish within N minutes?".
    Coarse,
    /// Exact figures, but only after the user confirms a modal.
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// camelCase to match `PlaybackView` and the rest of the IPC surface, so the
// frontend never has to remember which struct uses which convention.
#[serde(rename_all = "camelCase")]
pub struct HidingProfile {
    /// Identifier of the preset this came from, for the settings UI.
    pub id: String,

    pub hide_title: bool,
    pub hide_episode_number: bool,
    pub hide_season_number: bool,
    pub hide_episode_count: bool,
    pub hide_progress_bar: bool,
    pub hide_position: bool,
    pub hide_duration: bool,
    pub hide_remaining: bool,
    pub hide_chapters: bool,
    pub hide_artwork: bool,
    pub hide_next_up: bool,

    pub peek: PeekMode,
}

impl HidingProfile {
    /// Everything hidden, and no way to ask.
    pub fn full_darkness() -> Self {
        Self {
            id: "full_darkness".into(),
            hide_title: true,
            hide_episode_number: true,
            hide_season_number: true,
            hide_episode_count: true,
            hide_progress_bar: true,
            hide_position: true,
            hide_duration: true,
            hide_remaining: true,
            hide_chapters: true,
            hide_artwork: true,
            hide_next_up: true,
            peek: PeekMode::Disabled,
        }
    }

    /// The default: everything hidden, but the user can deliberately ask.
    pub fn standard() -> Self {
        Self {
            id: "standard".into(),
            peek: PeekMode::Confirmed,
            ..Self::full_darkness()
        }
    }

    /// For rewatches: the bar is back, the numbers are not.
    pub fn soft() -> Self {
        Self {
            id: "soft".into(),
            hide_title: false,
            hide_episode_number: false,
            hide_season_number: false,
            hide_episode_count: true,
            hide_progress_bar: false,
            hide_position: true,
            hide_duration: true,
            hide_remaining: true,
            hide_chapters: true,
            hide_artwork: false,
            hide_next_up: true,
            peek: PeekMode::Confirmed,
        }
    }

    pub fn preset(id: &str) -> Option<Self> {
        match id {
            "full_darkness" => Some(Self::full_darkness()),
            "standard" => Some(Self::standard()),
            "soft" => Some(Self::soft()),
            _ => None,
        }
    }

    pub fn presets() -> Vec<Self> {
        vec![Self::full_darkness(), Self::standard(), Self::soft()]
    }
}

impl Default for HidingProfile {
    fn default() -> Self {
        Self::standard()
    }
}
