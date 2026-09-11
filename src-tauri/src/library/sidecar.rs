//! The per-series sidecar file: progress and local settings, kept inside the
//! series folder itself so they travel with the folder when it is moved.
//!
//! Before this file existed, progress lived in the app's database keyed by
//! episode id, so moving a series folder (or reinstalling Murk) threw every
//! "Continue" cursor away. Now the source of truth is a small JSON file next to
//! the episodes, keyed by what survives a move: season and episode numbers, or
//! the path relative to the folder when a file has no numbers. The database
//! keeps only the catalogue; the sidecar keeps what belongs to the work.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The file name inside a series folder. A dotfile: conventionally hidden, and
/// never mistaken for a media file.
pub const SIDECAR_FILE: &str = ".murk.json";

/// The format version this build writes and understands. A file with a higher
/// version still loads (known fields are read, unknown ones are kept), but it
/// is logged so a downgrade does not go unnoticed.
pub const CURRENT_VERSION: u32 = 1;

/// One episode's remembered position.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub position_ms: i64,
    pub watched: bool,
    /// Unix seconds; which unfinished episode "Continue" resumes first.
    pub updated_at: i64,
}

/// Everything Murk keeps about a series, in the series' own folder.
///
/// Versioned and extensible: unknown fields survive a round-trip through an
/// older build.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sidecar {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Keyed by [`episode_key`]: `"season/number"`, or the relative path for
    /// files without numbers.
    #[serde(default)]
    pub progress: BTreeMap<String, Entry>,
    /// The subtitle language the user last chose for this series, or "off".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle_lang: Option<String>,
    /// Unknown fields from a newer build, carried across a round-trip.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

impl Sidecar {
    /// The sidecar file for a series folder.
    pub fn path(root: &Path) -> PathBuf {
        root.join(SIDECAR_FILE)
    }

    /// Read the sidecar, or start empty when the file is absent/unreadable.
    pub fn load(root: &Path) -> Self {
        let path = Self::path(root);
        match fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Self>(&text) {
                Ok(sidecar) => {
                    if sidecar.version > CURRENT_VERSION {
                        tracing::warn!(
                            "{}: sidecar version {} is newer than {}; loading anyway and preserving unknown fields",
                            path.display(),
                            sidecar.version,
                            CURRENT_VERSION
                        );
                    }
                    sidecar
                }
                Err(e) => {
                    tracing::warn!("could not parse {}: {e}", path.display());
                    // Set the damaged file aside before the next save overwrites it.
                    let backup = root.join(format!("{SIDECAR_FILE}.corrupt"));
                    if let Err(e) = fs::rename(&path, &backup) {
                        tracing::warn!("could not set aside corrupt {}: {e}", path.display());
                    }
                    Self::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!("could not read {}: {e}", path.display());
                Self::default()
            }
        }
    }

    /// Write the sidecar atomically; a no-op when the folder is gone.
    pub fn save(&self, root: &Path) -> std::io::Result<()> {
        if !root.is_dir() {
            return Ok(());
        }
        let path = Self::path(root);
        let tmp = root.join(format!("{SIDECAR_FILE}.tmp"));
        let bytes = serde_json::to_vec_pretty(self).map_err(serde_to_io)?;
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Whether the file carries any data worth keeping.
    pub fn has_data(&self) -> bool {
        !self.progress.is_empty() || self.subtitle_lang.is_some() || !self.extra.is_empty()
    }

    pub fn progress_for(&self, key: &str) -> Option<Entry> {
        self.progress.get(key).copied()
    }

    pub fn set_progress(&mut self, key: &str, entry: Entry) {
        self.progress.insert(key.to_string(), entry);
    }
}

fn serde_to_io(e: serde_json::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
}

/// The stable identity of an episode inside a sidecar.
///
/// Named files are keyed by season/episode (survives rename and move);
/// unnumbered files by their path relative to the folder (survives a move, not
/// a rename).
pub fn episode_key(season: Option<u32>, number: Option<u32>, root: &Path, path: &Path) -> String {
    match (season, number) {
        (Some(s), Some(n)) => format!("{s}/{n}"),
        _ => relative_key(root, path),
    }
}

/// The path of a file relative to the series folder, forward-slashed.
pub fn relative_key(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murk-sidecar-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trips_progress_and_settings() {
        let dir = tempdir();
        let sidecar = Sidecar {
            subtitle_lang: Some("rus".to_string()),
            progress: [(
                "1/3".to_string(),
                Entry {
                    position_ms: 120_000,
                    watched: false,
                    updated_at: 123,
                },
            )]
            .into_iter()
            .collect(),
            ..Sidecar::default()
        };
        sidecar.save(&dir).unwrap();

        let loaded = Sidecar::load(&dir);
        assert_eq!(loaded.subtitle_lang.as_deref(), Some("rus"));
        let entry = loaded.progress_for("1/3").unwrap();
        assert_eq!(entry.position_ms, 120_000);
        assert!(!entry.watched);
        assert_eq!(entry.updated_at, 123);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_fields_survive_a_round_trip() {
        let dir = tempdir();
        fs::write(
            Sidecar::path(&dir),
            br#"{"version":2,"progress":{},"favorite":true,"customThing":{"a":1}}"#,
        )
        .unwrap();

        let loaded = Sidecar::load(&dir);
        assert_eq!(loaded.version, 2);
        loaded.save(&dir).unwrap();

        let again = Sidecar::load(&dir);
        assert_eq!(again.extra.get("favorite"), Some(&serde_json::json!(true)));
        assert_eq!(
            again.extra.get("customThing"),
            Some(&serde_json::json!({"a": 1}))
        );
        assert!(again.has_data());

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_is_set_aside_before_it_would_be_overwritten() {
        let dir = tempdir();
        fs::write(Sidecar::path(&dir), b"this is not json").unwrap();

        Sidecar::load(&dir).save(&dir).unwrap();

        // The damaged original is never silently destroyed.
        assert!(
            dir.join(format!("{SIDECAR_FILE}.corrupt")).is_file(),
            "the broken file is kept as .corrupt"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_loads_empty() {
        let dir = tempdir();
        assert_eq!(Sidecar::load(&dir).progress.len(), 0);
        assert_eq!(Sidecar::load(&dir).subtitle_lang, None);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn corrupt_file_loads_empty_instead_of_crashing() {
        let dir = tempdir();
        fs::write(Sidecar::path(&dir), b"this is not json").unwrap();
        let sidecar = Sidecar::load(&dir);
        assert!(sidecar.progress.is_empty());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saving_without_a_folder_is_a_no_op() {
        let dir = tempdir();
        let gone = dir.join("gone");
        let mut sidecar = Sidecar::default();
        sidecar.set_progress(
            "1/1",
            Entry {
                position_ms: 1,
                watched: false,
                updated_at: 1,
            },
        );
        sidecar.save(&gone).unwrap();
        assert!(!Sidecar::path(&gone).exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn keys_use_numbers_when_present_and_relative_paths_otherwise() {
        let root = Path::new("/mnt/tv/Show");
        assert_eq!(
            episode_key(
                Some(2),
                Some(10),
                root,
                Path::new("/mnt/tv/Show/S02E10.mkv")
            ),
            "2/10"
        );
        // A renumbering keeps the key: numbers are the identity.
        assert_eq!(
            episode_key(
                Some(2),
                Some(10),
                root,
                Path::new("/mnt/tv/Show/Season 2/ep.mkv")
            ),
            "2/10"
        );
        // No numbers -> the path relative to the folder, forward-slashed.
        assert_eq!(
            episode_key(
                None,
                None,
                root,
                Path::new("/mnt/tv/Show/Season 2/Extra.mkv")
            ),
            "Season 2/Extra.mkv"
        );
        // A move keeps the relative path too.
        let moved = Path::new("/mnt/backup/Show");
        assert_eq!(
            episode_key(
                None,
                None,
                moved,
                Path::new("/mnt/backup/Show/Season 2/Extra.mkv")
            ),
            "Season 2/Extra.mkv"
        );
    }

    #[test]
    fn a_file_outside_the_root_falls_back_to_its_full_path() {
        let root = Path::new("/mnt/tv/Show");
        assert_eq!(
            episode_key(None, None, root, Path::new("/elsewhere/clip.mkv")),
            "/elsewhere/clip.mkv"
        );
    }
}
