//! Walking a series folder.
//!
//! Murk asks the user for a *folder*, never a file: the system file chooser
//! prints filenames, and a filename is usually the loudest spoiler available.
//! Picking `~/Series/Dark` reveals nothing; browsing into it to click
//! `S02E08 - Endings and Beginnings.mkv` reveals the ending.

use crate::library::db::ScannedEpisode;
use crate::library::parse;
use std::path::Path;
use walkdir::WalkDir;

/// Container extensions Murk will hand to mpv.
const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "webm", "ts", "m2ts", "mpg", "mpeg", "wmv", "flv", "ogv",
    "vob", "divx", "mts",
];

/// How deep to descend. Enough for `Series/Season 2/episode.mkv` and a little
/// slack, not enough to wander into an entire home directory by accident.
const MAX_DEPTH: usize = 4;

fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Sample files, extras and trailers are not episodes and would corrupt the
/// ordering if treated as such.
fn is_extra(stem: &str) -> bool {
    let s = stem.to_lowercase();
    [
        "sample",
        "trailer",
        "extras",
        "featurette",
        "behind the scenes",
        "bonus",
    ]
    .iter()
    .any(|marker| s.contains(marker))
}

pub fn scan_series_folder(root: &Path) -> Vec<ScannedEpisode> {
    let mut found: Vec<ScannedEpisode> = WalkDir::new(root)
        .max_depth(MAX_DEPTH)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| is_video(entry.path()))
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?;
            if is_extra(stem) {
                return None;
            }
            let parent = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str());
            let parsed = parse::parse_episode(stem, parent);
            Some(ScannedEpisode {
                path: path.to_path_buf(),
                season: parsed.season,
                number: parsed.number,
                order_key: parse::order_key(&parsed, path),
            })
        })
        .collect();

    // Not WalkDir's traversal order, which varies across filesystems.
    found.sort_by(|a, b| a.order_key.cmp(&b.order_key));
    found
}

/// A display name for the series, taken from the folder the user picked.
///
/// The user has just navigated to that folder, so showing its name back to
/// them reveals nothing new.
pub fn display_name_for(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.replace(['.', '_'], " ").trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Без названия".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murk-scan-{}-{}",
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
    fn finds_episodes_in_season_subfolders_and_orders_them() {
        let root = tempdir();
        fs::create_dir_all(root.join("Season 1")).unwrap();
        fs::create_dir_all(root.join("Season 2")).unwrap();
        for (dir, name) in [
            ("Season 1", "Episode 2.mkv"),
            ("Season 1", "Episode 10.mkv"),
            ("Season 1", "Episode 1.mkv"),
            ("Season 2", "Episode 1.mkv"),
            ("Season 1", "sample.mkv"),
            ("Season 1", "notes.txt"),
        ] {
            fs::write(root.join(dir).join(name), b"").unwrap();
        }

        let found = scan_series_folder(&root);
        let order: Vec<(Option<u32>, Option<u32>)> =
            found.iter().map(|e| (e.season, e.number)).collect();
        assert_eq!(
            order,
            vec![
                (Some(1), Some(1)),
                (Some(1), Some(2)),
                (Some(1), Some(10)),
                (Some(2), Some(1)),
            ],
            "sample.mkv and notes.txt must be skipped, episode 10 must sort last"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn display_name_comes_from_the_folder() {
        assert_eq!(display_name_for(Path::new("/x/The.Wire")), "The Wire");
        assert_eq!(display_name_for(Path::new("/x/Dark")), "Dark");
    }
}
