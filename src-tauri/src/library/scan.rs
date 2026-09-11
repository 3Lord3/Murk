//! Walking a series folder.
//!
//! Murk asks the user for a *folder*, never a file: the system file chooser
//! prints filenames, and a filename is usually the loudest spoiler available.
//! Picking `~/Series/Dark` reveals nothing; browsing into it to click
//! `S02E08 - Endings and Beginnings.mkv` reveals the ending.

use crate::library::db::ScannedEpisode;
use crate::library::parse;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
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
///
/// Each marker must be a whole word: `sample.mkv` and `Show.Sample.mkv` are
/// extras, but `sample-20s.mp4` is a clip named with a hyphen and must play.
/// Dots, underscores and whitespace delimit words; a hyphen does not.
fn is_extra(stem: &str) -> bool {
    let lower = stem.to_lowercase();
    let tokens: Vec<&str> = lower
        .split(|c: char| c == '.' || c == '_' || c.is_whitespace())
        .filter(|t| !t.is_empty())
        .collect();

    let singles = ["sample", "trailer", "extras", "featurette", "bonus"];
    if singles.iter().any(|marker| tokens.contains(marker)) {
        return true;
    }
    tokens.windows(3).any(|w| w == ["behind", "the", "scenes"])
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

// --- folders that hold whole series -----------------------------------------

/// How far to descend when deciding whether a folder holds series or is one.
/// A container of whole series is rarely more than a couple of levels deep;
/// beyond this the folder is taken as a series rather than walked forever.
pub const MAX_CONTAINER_DEPTH: usize = 4;

/// The most series a single "add folder" may create. Picking a home directory
/// must not silently index a thousand leaves.
pub const MAX_SERIES_TO_ADD: usize = 500;

/// The immediate subfolders of `root`.
pub fn subfolders(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push(entry.path());
        }
    }
    Ok(dirs)
}

/// Whether `root` directly holds a playable video file, as opposed to holding
/// only subfolders.
pub fn has_direct_videos(root: &Path) -> bool {
    fs::read_dir(root)
        .map(|entries| {
            entries.flatten().any(|e| {
                e.file_type().map(|t| t.is_file()).unwrap_or(false) && is_video(&e.path())
            })
        })
        .unwrap_or(false)
}

/// Whether any of `dirs` is named like a season folder ("Season 2", "S01").
fn any_season_subfolder(dirs: &[PathBuf]) -> bool {
    dirs.iter()
        .filter_map(|p| p.file_name().and_then(|n| n.to_str()))
        .any(|name| parse::season_from_directory(name).is_some())
}

/// The series roots the user means when they add `root`.
///
/// A folder that holds video files directly, or keeps its episodes in `Season
/// N` subfolders, is itself a series and comes back as the single root. Any
/// other folder with subfolders is a container of whole series: each subfolder
/// becomes a series root in its turn, so adding `~/Series` adds every show
/// inside it instead of one giant series named "Series".
pub fn series_roots_to_add(root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    collect_series_roots(root, 0, &mut roots);
    roots
}

fn collect_series_roots(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let dirs = match subfolders(root) {
        Ok(dirs) => dirs,
        // An unreadable folder is not evidence that it is a container: treat it
        // as a single series rather than recursing past ground we cannot see.
        Err(_) => {
            out.push(root.to_path_buf());
            return;
        }
    };
    let is_series_folder = has_direct_videos(root) || any_season_subfolder(&dirs) || dirs.is_empty();
    if is_series_folder || depth >= MAX_CONTAINER_DEPTH {
        out.push(root.to_path_buf());
        return;
    }
    for sub in dirs {
        collect_series_roots(&sub, depth + 1, out);
    }
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
    fn standalone_samples_are_extras_but_hyphenated_clips_are_episodes() {
        let root = tempdir();
        for name in [
            "sample.mkv",
            "Show.Sample.mkv",
            "sample-20s.mp4",
            "Episode 1.mkv",
        ] {
            fs::write(root.join(name), b"").unwrap();
        }

        let found = scan_series_folder(&root);
        let names: Vec<String> = found
            .iter()
            .map(|e| e.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            vec!["Episode 1.mkv".to_string(), "sample-20s.mp4".to_string()],
            "standalone samples are dropped, a hyphenated clip is kept as an episode"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn display_name_comes_from_the_folder() {
        assert_eq!(display_name_for(Path::new("/x/The.Wire")), "The Wire");
        assert_eq!(display_name_for(Path::new("/x/Dark")), "Dark");
    }

    #[test]
    fn a_plain_folder_with_videos_is_one_series() {
        let root = tempdir();
        fs::write(root.join("S01E01.mkv"), b"").unwrap();
        assert_eq!(series_roots_to_add(&root), vec![root.clone()]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_folder_with_season_subfolders_is_one_series() {
        let root = tempdir();
        fs::create_dir_all(root.join("Season 1")).unwrap();
        fs::write(root.join("Season 1/ep.mkv"), b"").unwrap();
        assert_eq!(series_roots_to_add(&root), vec![root.clone()]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn season_plus_extras_stays_one_series() {
        let root = tempdir();
        fs::create_dir_all(root.join("Season 1")).unwrap();
        fs::create_dir_all(root.join("Extras")).unwrap();
        fs::write(root.join("Season 1/ep.mkv"), b"").unwrap();
        fs::write(root.join("Extras/featurette.mkv"), b"").unwrap();
        assert_eq!(series_roots_to_add(&root), vec![root.clone()]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_container_adds_each_series_inside_it() {
        let root = tempdir();
        let dark = root.join("Dark");
        let wire = root.join("The Wire");
        fs::create_dir_all(&dark).unwrap();
        fs::create_dir_all(&wire).unwrap();
        fs::write(dark.join("S01E01.mkv"), b"").unwrap();
        fs::write(wire.join("S01E01.mkv"), b"").unwrap();

        let roots = series_roots_to_add(&root);
        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&dark));
        assert!(roots.contains(&wire));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn nested_containers_flatten_to_the_series_leaves() {
        let root = tempdir();
        let dark = root.join("Drama").join("Dark");
        fs::create_dir_all(&dark).unwrap();
        fs::write(dark.join("S01E01.mkv"), b"").unwrap();
        assert_eq!(series_roots_to_add(&root), vec![dark]);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_empty_folder_is_added_as_itself() {
        let root = tempdir();
        assert_eq!(series_roots_to_add(&root), vec![root.clone()]);
        fs::remove_dir_all(&root).ok();
    }
}
