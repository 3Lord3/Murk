//! The library database.
//!
//! This file knows the season, the episode number, the running time and the
//! path, everything the product exists to withhold. That is fine: it lives in
//! Rust, and the only route from here to a screen is
//! [`crate::privacy::PlaybackView::project`].

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::library::sidecar::{self, Entry, Sidecar};

/// Fraction of the running time after which an episode counts as watched.
pub const WATCHED_FRACTION: f64 = 0.92;

#[derive(Debug, Clone)]
pub struct SeriesRow {
    pub id: i64,
    pub root_path: PathBuf,
    pub display_name: String,
    pub poster_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct EpisodeRow {
    pub id: i64,
    pub series_id: i64,
    pub path: PathBuf,
    pub season: Option<u32>,
    pub number: Option<u32>,
    pub order_key: String,
    pub duration_ms: Option<i64>,
}

/// An episode as the scanner produced it, before it has an id.
#[derive(Debug, Clone)]
pub struct ScannedEpisode {
    pub path: PathBuf,
    pub season: Option<u32>,
    pub number: Option<u32>,
    pub order_key: String,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Millisecond timestamps: a quick switch between two just-saved episodes
/// must not be a coin toss.
fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct Library {
    conn: parking_lot::Mutex<Connection>,
    /// Cached sidecar per series; every save writes through to the file.
    sidecars: parking_lot::Mutex<HashMap<i64, Sidecar>>,
    /// Cached sidecar key per episode; cleared when a series' episodes change.
    keys: parking_lot::Mutex<HashMap<i64, HashMap<i64, String>>>,
}

impl Library {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening library at {}", path.display()))?;
        Self::from_connection(conn)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS series (
              id INTEGER PRIMARY KEY,
              root_path TEXT NOT NULL UNIQUE,
              display_name TEXT NOT NULL,
              poster_path TEXT,
              added_at INTEGER NOT NULL);

            CREATE TABLE IF NOT EXISTS episode (
              id INTEGER PRIMARY KEY,
              series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
              path TEXT NOT NULL UNIQUE,
              season INTEGER,
              number INTEGER,
              order_key TEXT NOT NULL,
              duration_ms INTEGER,
              added_at INTEGER NOT NULL);

            CREATE INDEX IF NOT EXISTS episode_by_order
              ON episode(series_id, order_key);

            CREATE TABLE IF NOT EXISTS setting (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL);
            "#,
        )?;
        let library = Self {
            conn: parking_lot::Mutex::new(conn),
            sidecars: parking_lot::Mutex::new(HashMap::new()),
            keys: parking_lot::Mutex::new(HashMap::new()),
        };
        // A migration failure must never make the app unlaunchable; try later.
        if let Err(e) = library.migrate_legacy_progress() {
            tracing::warn!("legacy progress migration did not finish: {e}");
        }
        Ok(library)
    }

    /// One-time lift of old `progress`/`setting` rows into each series' sidecar.
    /// The `progress` table is dropped only once empty; a series whose folder is
    /// unavailable keeps its rows for a later start.
    fn migrate_legacy_progress(&self) -> Result<()> {
        let has_progress = self.conn.lock().query_row(
            "SELECT EXISTS (
               SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'progress'
             )",
            [],
            |r| r.get::<_, i64>(0),
        )? != 0;
        if !has_progress {
            return Ok(());
        }

        for series in self.list_series()? {
            // No folder, no sidecar; its rows stay for a later start.
            if !series.root_path.is_dir() {
                continue;
            }

            let rows = {
                let conn = self.conn.lock();
                let mut stmt = conn.prepare(
                    "SELECT e.path, e.season, e.number, p.position_ms, p.watched, p.updated_at
                     FROM episode e JOIN progress p ON p.episode_id = e.id
                     WHERE e.series_id = ?1",
                )?;
                let rows = stmt
                    .query_map(params![series.id], |r| {
                        Ok((
                            PathBuf::from(r.get::<_, String>(0)?),
                            r.get::<_, Option<u32>>(1)?,
                            r.get::<_, Option<u32>>(2)?,
                            r.get::<_, i64>(3)?,
                            r.get::<_, i64>(4)? != 0,
                            r.get::<_, i64>(5)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };

            let mut sidecar = Sidecar::load(&series.root_path);
            // An existing sidecar language wins over the legacy setting.
            if sidecar.subtitle_lang.is_none() {
                if let Ok(Some(lang)) = self.get_setting(&format!("subtitle_lang:{}", series.id)) {
                    sidecar.subtitle_lang = Some(lang);
                }
            }

            let keys = unique_keys(
                &series.root_path,
                rows.iter()
                    .map(|(path, season, number, ..)| (*season, *number, path.as_path())),
            );
            for (key, (_, _, _, position_ms, watched, updated_at)) in keys.iter().zip(rows.iter()) {
                sidecar.set_progress(
                    key,
                    Entry {
                        position_ms: *position_ms,
                        watched: *watched,
                        // The legacy value was in whole seconds.
                        updated_at: *updated_at * 1000,
                    },
                );
            }
            if sidecar.has_data() {
                sidecar.save(&series.root_path)?;
            }

            // Drop this series' rows only after the sidecar was saved.
            self.conn.lock().execute(
                "DELETE FROM progress WHERE episode_id IN
                   (SELECT id FROM episode WHERE series_id = ?1)",
                params![series.id],
            )?;
            self.conn.lock().execute(
                "DELETE FROM setting WHERE key = ?1",
                params![format!("subtitle_lang:{}", series.id)],
            )?;
        }

        // Drop the table only once every series was migrated.
        let remaining: i64 =
            self.conn
                .lock()
                .query_row("SELECT COUNT(*) FROM progress", [], |r| r.get(0))?;
        if remaining == 0 {
            self.conn
                .lock()
                .execute_batch("DROP TABLE IF EXISTS progress;")?;
        }
        Ok(())
    }

    // --- series ------------------------------------------------------------

    pub fn add_series(&self, root: &Path, display_name: &str) -> Result<i64> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO series (root_path, display_name, added_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(root_path) DO UPDATE SET display_name = excluded.display_name",
            params![root.to_string_lossy(), display_name, now_secs()],
        )?;
        Ok(conn.query_row(
            "SELECT id FROM series WHERE root_path = ?1",
            params![root.to_string_lossy()],
            |r| r.get(0),
        )?)
    }

    pub fn list_series(&self) -> Result<Vec<SeriesRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, root_path, display_name, poster_path
             FROM series ORDER BY display_name COLLATE NOCASE",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(SeriesRow {
                    id: r.get(0)?,
                    root_path: PathBuf::from(r.get::<_, String>(1)?),
                    display_name: r.get(2)?,
                    poster_path: r.get::<_, Option<String>>(3)?.map(PathBuf::from),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn series(&self, series_id: i64) -> Result<Option<SeriesRow>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT id, root_path, display_name, poster_path
                 FROM series WHERE id = ?1",
                params![series_id],
                |r| {
                    Ok(SeriesRow {
                        id: r.get(0)?,
                        root_path: PathBuf::from(r.get::<_, String>(1)?),
                        display_name: r.get(2)?,
                        poster_path: r.get::<_, Option<String>>(3)?.map(PathBuf::from),
                    })
                },
            )
            .optional()?)
    }

    pub fn set_poster(&self, series_id: i64, poster: Option<&Path>) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE series SET poster_path = ?2 WHERE id = ?1",
            params![series_id, poster.map(|p| p.to_string_lossy().into_owned())],
        )?;
        Ok(())
    }

    pub fn remove_series(&self, series_id: i64) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM series WHERE id = ?1", params![series_id])?;
        drop(conn);
        // The sidecar stays with the folder, so re-adding brings progress back.
        self.sidecars.lock().remove(&series_id);
        self.keys.lock().remove(&series_id);
        Ok(())
    }

    /// Delete series whose folders are unreachable; their progress lives in the
    /// folder's sidecar and returns on a re-add. Series still holding legacy
    /// progress are kept, since dropping them would throw that data away.
    pub fn prune_missing_series(&self) -> Result<u32> {
        let mut pruned = 0u32;
        for series in self.list_series()? {
            if series.root_path.is_dir() {
                continue;
            }
            if self.has_legacy_progress(series.id)? {
                continue;
            }
            self.remove_series(series.id)?;
            pruned += 1;
        }
        Ok(pruned)
    }

    /// Whether the series still has legacy `progress` rows that must not be
    /// deleted (migration was blocked by an unavailable folder).
    fn has_legacy_progress(&self, series_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        let has_table: bool = conn.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'progress'
             )",
            [],
            |r| r.get::<_, i64>(0),
        )? != 0;
        if !has_table {
            return Ok(false);
        }
        Ok(conn.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM progress p JOIN episode e ON e.id = p.episode_id
               WHERE e.series_id = ?1
             )",
            params![series_id],
            |r| r.get::<_, i64>(0),
        )? != 0)
    }

    // --- episodes ----------------------------------------------------------

    /// Insert what the scanner found; drop episodes that disappeared.
    pub fn sync_episodes(&self, series_id: i64, found: &[ScannedEpisode]) -> Result<()> {
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            {
                let mut insert = tx.prepare(
                    "INSERT INTO episode (series_id, path, season, number, order_key, added_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(path) DO UPDATE SET
                       season = excluded.season,
                       number = excluded.number,
                       order_key = excluded.order_key",
                )?;
                for e in found {
                    insert.execute(params![
                        series_id,
                        e.path.to_string_lossy(),
                        e.season,
                        e.number,
                        e.order_key,
                        now_secs()
                    ])?;
                }

                let keep: Vec<String> = found
                    .iter()
                    .map(|e| e.path.to_string_lossy().into_owned())
                    .collect();
                let mut stmt = tx.prepare("SELECT id, path FROM episode WHERE series_id = ?1")?;
                let stale: Vec<i64> = stmt
                    .query_map(params![series_id], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                    .into_iter()
                    .filter(|(_, p)| !keep.contains(p))
                    .map(|(id, _)| id)
                    .collect();
                drop(stmt);
                for id in stale {
                    tx.execute("DELETE FROM episode WHERE id = ?1", params![id])?;
                }
            }
            tx.commit()?;
        }
        // Cached episode keys are stale now.
        self.keys.lock().remove(&series_id);

        // Prune sidecar entries for files that are gone, after the transaction
        // so the sidecar lock is never taken while holding the database lock.
        if let Some(root) = self.series_root(series_id)? {
            if let Err(e) = self.prune_sidecar(series_id, found, &root) {
                tracing::warn!("could not prune sidecar for series {series_id}: {e}");
            }
        }
        Ok(())
    }

    /// Drop sidecar entries that no longer match an episode on disk.
    fn prune_sidecar(&self, series_id: i64, found: &[ScannedEpisode], root: &Path) -> Result<()> {
        let keep: HashSet<String> = unique_keys(
            root,
            found.iter().map(|e| (e.season, e.number, e.path.as_path())),
        )
        .into_iter()
        .collect();
        self.update_sidecar(series_id, |sidecar| {
            sidecar.progress.retain(|key, _| keep.contains(key));
        })
    }

    pub fn episode(&self, episode_id: i64) -> Result<Option<EpisodeRow>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT id, series_id, path, season, number, order_key, duration_ms
                 FROM episode WHERE id = ?1",
                params![episode_id],
                episode_from_row,
            )
            .optional()?)
    }

    pub fn episode_count(&self, series_id: i64) -> Result<u32> {
        let conn = self.conn.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM episode WHERE series_id = ?1",
            params![series_id],
            |r| r.get::<_, i64>(0),
        )? as u32)
    }

    /// The first episode in playback order. Its embedded cover art, if any, is
    /// the closest thing the folder has to a poster.
    pub fn first_episode(&self, series_id: i64) -> Result<Option<EpisodeRow>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT id, series_id, path, season, number, order_key, duration_ms
                 FROM episode WHERE series_id = ?1 ORDER BY order_key LIMIT 1",
                params![series_id],
                episode_from_row,
            )
            .optional()?)
    }

    /// Every episode of a series, in playback order. Never leaves this module.
    fn episodes(&self, series_id: i64) -> Result<Vec<EpisodeRow>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, series_id, path, season, number, order_key, duration_ms
             FROM episode WHERE series_id = ?1 ORDER BY order_key",
        )?;
        let rows = stmt
            .query_map(params![series_id], episode_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Where "Continue" should take the user: the most recently left-unfinished
    /// episode, else the first untouched one. Facts come from the sidecar.
    pub fn resume_target(&self, series_id: i64) -> Result<Option<EpisodeRow>> {
        let root = self
            .series_root(series_id)?
            .ok_or_else(|| anyhow!("no such series: {series_id}"))?;
        let episodes = self.episodes(series_id)?;
        let sidecar = self.cached_sidecar(series_id)?;
        let keys = episode_keys(&episodes, &root);

        // 1. the episode that was left unfinished most recently
        let mut unfinished: Option<(i64, EpisodeRow)> = None;
        for episode in &episodes {
            let Some(key) = keys.get(&episode.id) else {
                continue;
            };
            if let Some(entry) = sidecar.progress_for(key) {
                if !entry.watched
                    && unfinished
                        .as_ref()
                        .is_none_or(|(updated, _)| entry.updated_at > *updated)
                {
                    unfinished = Some((entry.updated_at, episode.clone()));
                }
            }
        }
        if let Some((_, episode)) = unfinished {
            return Ok(Some(episode));
        }

        // 2. otherwise the first episode with no progress at all.
        for episode in episodes {
            let Some(key) = keys.get(&episode.id) else {
                continue;
            };
            if sidecar.progress_for(key).is_none() {
                return Ok(Some(episode));
            }
        }
        Ok(None)
    }

    /// The next episode in playback order, for auto-advance.
    pub fn following(&self, episode: &EpisodeRow) -> Result<Option<EpisodeRow>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT id, series_id, path, season, number, order_key, duration_ms
                 FROM episode WHERE series_id = ?1 AND order_key > ?2
                 ORDER BY order_key LIMIT 1",
                params![episode.series_id, episode.order_key],
                episode_from_row,
            )
            .optional()?)
    }

    // --- progress (the sidecar file) ----------------------------------------

    /// The root path of a series, needed to map episodes onto sidecar keys.
    fn series_root(&self, series_id: i64) -> Result<Option<PathBuf>> {
        Ok(self.series(series_id)?.map(|s| s.root_path))
    }

    /// The sidecar for a series, loaded once and cached.
    fn cached_sidecar(&self, series_id: i64) -> Result<Sidecar> {
        if let Some(sidecar) = self.sidecars.lock().get(&series_id) {
            return Ok(sidecar.clone());
        }
        let root = self
            .series_root(series_id)?
            .ok_or_else(|| anyhow!("no such series: {series_id}"))?;
        let sidecar = Sidecar::load(&root);
        self.sidecars.lock().insert(series_id, sidecar.clone());
        Ok(sidecar)
    }

    /// Load, change and save a series' sidecar under one lock. An empty result
    /// removes the file.
    fn update_sidecar<F>(&self, series_id: i64, change: F) -> Result<()>
    where
        F: FnOnce(&mut Sidecar),
    {
        let root = self
            .series_root(series_id)?
            .ok_or_else(|| anyhow!("no such series: {series_id}"))?;
        let mut sidecars = self.sidecars.lock();
        let mut sidecar = match sidecars.get(&series_id) {
            Some(existing) => existing.clone(),
            None => Sidecar::load(&root),
        };
        change(&mut sidecar);
        if sidecar.has_data() {
            sidecar
                .save(&root)
                .with_context(|| format!("writing sidecar for series {series_id}"))?;
            sidecars.insert(series_id, sidecar);
        } else {
            remove_sidecar_file(&root)?;
            sidecars.remove(&series_id);
        }
        Ok(())
    }

    /// The sidecar key of one episode, cached; invalidated when episodes change.
    fn key_for(&self, series_id: i64, root: &Path, episode: &EpisodeRow) -> Result<String> {
        {
            let cache = self.keys.lock();
            if let Some(keys) = cache.get(&series_id) {
                if let Some(key) = keys.get(&episode.id) {
                    return Ok(key.clone());
                }
            }
        }
        // Cache miss: build the series' key map once and keep it.
        let episodes = self.episodes(series_id)?;
        let keys = episode_keys(&episodes, root);
        let key = keys.get(&episode.id).cloned().unwrap_or_else(|| {
            sidecar::episode_key(episode.season, episode.number, root, &episode.path)
        });
        self.keys.lock().insert(series_id, keys);
        Ok(key)
    }

    pub fn save_progress(
        &self,
        series_id: i64,
        episode: &EpisodeRow,
        position_ms: i64,
        watched: bool,
    ) -> Result<()> {
        let root = self
            .series_root(series_id)?
            .ok_or_else(|| anyhow!("no such series: {series_id}"))?;
        let key = self.key_for(series_id, &root, episode)?;
        self.update_sidecar(series_id, move |sidecar| {
            // Once watched, always watched.
            let watched = match sidecar.progress_for(&key) {
                Some(previous) => previous.watched || watched,
                None => watched,
            };
            sidecar.set_progress(
                &key,
                Entry {
                    position_ms,
                    watched,
                    updated_at: now_millis(),
                },
            );
        })
    }

    pub fn resume_position_ms(&self, series_id: i64, episode: &EpisodeRow) -> Result<i64> {
        let root = self
            .series_root(series_id)?
            .ok_or_else(|| anyhow!("no such series: {series_id}"))?;
        let sidecar = self.cached_sidecar(series_id)?;
        let key = self.key_for(series_id, &root, episode)?;
        // A finished episode restarts from the beginning.
        Ok(match sidecar.progress_for(&key) {
            Some(entry) if !entry.watched => entry.position_ms,
            _ => 0,
        })
    }

    /// Whether any episode has a stored position or watched flag.
    pub fn has_progress(&self, series_id: i64) -> Result<bool> {
        Ok(!self.cached_sidecar(series_id)?.progress.is_empty())
    }

    /// How far the whole folder has been watched, 0 to 1, weighted by running
    /// time. Unknown durations use the series average; unknown to all, the
    /// episode count. Returns `None` for a series with no episodes.
    pub fn series_progress(&self, series_id: i64) -> Result<Option<f64>> {
        let root = match self.series_root(series_id)? {
            Some(root) => root,
            None => return Ok(None),
        };
        let episodes = self.episodes(series_id)?;
        if episodes.is_empty() {
            return Ok(None);
        }
        let sidecar = self.cached_sidecar(series_id)?;
        let keys = episode_keys(&episodes, &root);

        let rows: Vec<(Option<i64>, i64, bool)> = episodes
            .iter()
            .map(|e| {
                let entry = keys.get(&e.id).and_then(|key| sidecar.progress_for(key));
                (
                    e.duration_ms,
                    entry.map(|p| p.position_ms).unwrap_or(0),
                    entry.map(|p| p.watched).unwrap_or(false),
                )
            })
            .collect();

        let known: Vec<i64> = rows
            .iter()
            .filter_map(|(d, _, _)| *d)
            .filter(|d| *d > 0)
            .collect();
        let Some(average) =
            (!known.is_empty()).then(|| known.iter().sum::<i64>() / known.len() as i64)
        else {
            let watched = rows.iter().filter(|(_, _, w)| *w).count();
            return Ok(Some(watched as f64 / rows.len() as f64));
        };

        let mut total = 0f64;
        let mut done = 0f64;
        for (duration, position, watched) in rows {
            let duration = duration.filter(|d| *d > 0).unwrap_or(average) as f64;
            total += duration;
            // A watched episode counts in full.
            done += if watched {
                duration
            } else {
                (position as f64).clamp(0.0, duration)
            };
        }

        Ok(Some(if total > 0.0 {
            (done / total).clamp(0.0, 1.0)
        } else {
            0.0
        }))
    }

    /// Forget every saved position and watched flag in a series.
    pub fn reset_progress(&self, series_id: i64) -> Result<()> {
        self.update_sidecar(series_id, |sidecar| sidecar.progress.clear())
    }

    pub fn record_duration(&self, episode_id: i64, duration_ms: i64) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE episode SET duration_ms = ?2 WHERE id = ?1",
            params![episode_id, duration_ms],
        )?;
        Ok(())
    }

    // --- settings ----------------------------------------------------------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .lock()
            .query_row(
                "SELECT value FROM setting WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO setting (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // --- per-series subtitle preference (the sidecar file) -----------------

    /// The subtitle language last chosen for this series.
    pub fn preferred_subtitle_lang(&self, series_id: i64) -> Result<Option<String>> {
        Ok(self.cached_sidecar(series_id)?.subtitle_lang.clone())
    }

    /// Remember the chosen subtitle language for a series. `None` clears it.
    pub fn set_subtitle_lang(&self, series_id: i64, lang: Option<&str>) -> Result<()> {
        self.update_sidecar(series_id, |sidecar| {
            sidecar.subtitle_lang = lang.map(str::to_string);
        })
    }
}

/// One sidecar key per `(season, number, path)`; episodes that share a season
/// and number fall back to the path relative to the folder, so they don't
/// clobber each other.
fn unique_keys<'a>(
    root: &Path,
    items: impl Iterator<Item = (Option<u32>, Option<u32>, &'a Path)>,
) -> Vec<String> {
    let items: Vec<(Option<u32>, Option<u32>, &Path)> = items.collect();
    let mut counts: HashMap<(u32, u32), usize> = HashMap::new();
    for (season, number, _) in &items {
        if let (Some(s), Some(n)) = (season, number) {
            *counts.entry((*s, *n)).or_default() += 1;
        }
    }
    items
        .iter()
        .map(|(season, number, path)| {
            let duplicated = match (season, number) {
                (Some(s), Some(n)) => counts.get(&(*s, *n)).is_some_and(|c| *c > 1),
                _ => false,
            };
            if duplicated {
                sidecar::relative_key(root, path)
            } else {
                sidecar::episode_key(*season, *number, root, path)
            }
        })
        .collect()
}

/// Sidecar keys for a series' episodes, by episode id.
fn episode_keys(episodes: &[EpisodeRow], root: &Path) -> HashMap<i64, String> {
    unique_keys(
        root,
        episodes
            .iter()
            .map(|e| (e.season, e.number, e.path.as_path())),
    )
    .into_iter()
    .zip(episodes.iter().map(|e| e.id))
    .map(|(key, id)| (id, key))
    .collect()
}

/// Delete a series' sidecar, if it is there.
fn remove_sidecar_file(root: &Path) -> Result<()> {
    let path = Sidecar::path(root);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

fn episode_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<EpisodeRow> {
    Ok(EpisodeRow {
        id: r.get(0)?,
        series_id: r.get(1)?,
        path: PathBuf::from(r.get::<_, String>(2)?),
        season: r.get::<_, Option<u32>>(3)?,
        number: r.get::<_, Option<u32>>(4)?,
        order_key: r.get(5)?,
        duration_ms: r.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded() -> (Library, i64) {
        let lib = Library::in_memory().unwrap();
        let sid = lib.add_series(Path::new("/series/show"), "Show").unwrap();
        let eps: Vec<ScannedEpisode> = (1..=4)
            .map(|n| ScannedEpisode {
                path: PathBuf::from(format!("/series/show/S01E{n:02}.mkv")),
                season: Some(1),
                number: Some(n),
                order_key: format!("0/0001/{n:04}"),
            })
            .collect();
        lib.sync_episodes(sid, &eps).unwrap();
        (lib, sid)
    }

    fn seeded_episodes(lib: &Library, sid: i64) -> Vec<EpisodeRow> {
        lib.episodes(sid).unwrap()
    }

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murk-db-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn series_progress_measures_the_whole_folder_in_time() {
        let (lib, sid) = seeded();
        let eps = seeded_episodes(&lib, sid);
        // Three half-hour episodes and one twice as long.
        for (i, e) in eps.iter().enumerate() {
            let ms = if i == 3 { 3_600_000 } else { 1_800_000 };
            lib.record_duration(e.id, ms).unwrap();
        }
        assert_eq!(lib.series_progress(sid).unwrap(), Some(0.0));

        lib.save_progress(sid, &eps[0], 1_800_000, true).unwrap();
        assert!((lib.series_progress(sid).unwrap().unwrap() - 0.2).abs() < 1e-6);

        // A position inside an unfinished episode counts for its own length.
        lib.save_progress(sid, &eps[1], 900_000, false).unwrap();
        assert!((lib.series_progress(sid).unwrap().unwrap() - 0.3).abs() < 1e-6);
    }

    #[test]
    fn series_progress_falls_back_to_counting_episodes() {
        let (lib, sid) = seeded();
        let eps = seeded_episodes(&lib, sid);
        lib.save_progress(sid, &eps[0], 0, true).unwrap();
        assert_eq!(lib.series_progress(sid).unwrap(), Some(0.25));
    }

    #[test]
    fn resume_starts_at_the_first_episode() {
        let (lib, sid) = seeded();
        let e = lib.resume_target(sid).unwrap().unwrap();
        assert_eq!(e.number, Some(1));
    }

    #[test]
    fn resume_returns_the_unfinished_episode() {
        let (lib, sid) = seeded();
        let first = lib.resume_target(sid).unwrap().unwrap();
        lib.save_progress(sid, &first, 600_000, false).unwrap();
        let again = lib.resume_target(sid).unwrap().unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(lib.resume_position_ms(sid, &first).unwrap(), 600_000);
    }

    #[test]
    fn a_watched_episode_hands_over_to_the_next_one() {
        let (lib, sid) = seeded();
        let first = lib.resume_target(sid).unwrap().unwrap();
        lib.save_progress(sid, &first, 2_600_000, true).unwrap();
        let next = lib.resume_target(sid).unwrap().unwrap();
        assert_eq!(next.number, Some(2));
    }

    #[test]
    fn watched_is_sticky_and_restarts_from_zero() {
        let (lib, sid) = seeded();
        let first = lib.resume_target(sid).unwrap().unwrap();
        lib.save_progress(sid, &first, 2_600_000, true).unwrap();
        // rewinding to the start must not un-watch it
        lib.save_progress(sid, &first, 0, false).unwrap();
        assert_eq!(lib.resume_target(sid).unwrap().unwrap().number, Some(2));
        assert_eq!(lib.resume_position_ms(sid, &first).unwrap(), 0);
    }

    #[test]
    fn reset_progress_forgets_every_position_and_watched_flag() {
        let (lib, sid) = seeded();
        let first = lib.resume_target(sid).unwrap().unwrap();
        lib.save_progress(sid, &first, 600_000, false).unwrap();
        lib.reset_progress(sid).unwrap();
        assert_eq!(lib.resume_position_ms(sid, &first).unwrap(), 0);
        assert_eq!(lib.resume_target(sid).unwrap().unwrap().number, Some(1));
    }

    #[test]
    fn a_finished_series_has_no_resume_target_but_still_has_a_first_episode() {
        let (lib, sid) = seeded();
        while let Some(episode) = lib.resume_target(sid).unwrap() {
            lib.save_progress(sid, &episode, 2_600_000, true).unwrap();
        }
        assert!(lib.resume_target(sid).unwrap().is_none());
        // What `continue_series` falls back on, so the button starts a rewatch
        // instead of failing with `no_video_files`.
        assert_eq!(lib.first_episode(sid).unwrap().unwrap().number, Some(1));
        assert!(lib.has_progress(sid).unwrap());
    }

    #[test]
    fn following_walks_the_order_key() {
        let (lib, sid) = seeded();
        let first = lib.resume_target(sid).unwrap().unwrap();
        let second = lib.following(&first).unwrap().unwrap();
        assert_eq!(second.number, Some(2));

        let third = lib.following(&second).unwrap().unwrap();
        let fourth = lib.following(&third).unwrap().unwrap();
        assert_eq!(fourth.number, Some(4));
        assert!(
            lib.following(&fourth).unwrap().is_none(),
            "no episode after the last"
        );
    }

    #[test]
    fn rescanning_keeps_progress_and_drops_vanished_files() {
        let (lib, sid) = seeded();
        let first = lib.resume_target(sid).unwrap().unwrap();
        lib.save_progress(sid, &first, 42_000, false).unwrap();

        // the fourth file was deleted from disk
        let eps: Vec<ScannedEpisode> = (1..=3)
            .map(|n| ScannedEpisode {
                path: PathBuf::from(format!("/series/show/S01E{n:02}.mkv")),
                season: Some(1),
                number: Some(n),
                order_key: format!("0/0001/{n:04}"),
            })
            .collect();
        lib.sync_episodes(sid, &eps).unwrap();

        assert_eq!(lib.episode_count(sid).unwrap(), 3);
        assert_eq!(lib.resume_position_ms(sid, &first).unwrap(), 42_000);
    }

    #[test]
    fn progress_and_settings_survive_a_folder_being_re_added() {
        let dir = tempdir();
        let lib = Library::in_memory().unwrap();
        let eps: Vec<ScannedEpisode> = (1..=2)
            .map(|n| ScannedEpisode {
                path: dir.join(format!("S01E{n:02}.mkv")),
                season: Some(1),
                number: Some(n),
                order_key: format!("0/0001/{n:04}"),
            })
            .collect();

        let sid = lib.add_series(&dir, "Show").unwrap();
        lib.sync_episodes(sid, &eps).unwrap();
        lib.set_subtitle_lang(sid, Some("rus")).unwrap();
        let first = lib.episodes(sid).unwrap().remove(0);
        lib.save_progress(sid, &first, 42_000, false).unwrap();

        // The sidecar file now exists in the folder.
        assert!(dir.join(sidecar::SIDECAR_FILE).is_file());

        // Removing the series leaves the file; re-adding the folder restores it.
        lib.remove_series(sid).unwrap();
        let again = lib.add_series(&dir, "Show").unwrap();
        lib.sync_episodes(again, &eps).unwrap();

        assert_eq!(
            lib.preferred_subtitle_lang(again).unwrap().as_deref(),
            Some("rus")
        );
        assert!(lib.has_progress(again).unwrap());
        let restored = lib.episodes(again).unwrap().remove(0);
        assert_eq!(lib.resume_position_ms(again, &restored).unwrap(), 42_000);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn duplicate_season_and_number_get_distinct_keys() {
        let lib = Library::in_memory().unwrap();
        let sid = lib.add_series(Path::new("/series/dup"), "Dup").unwrap();
        let eps: Vec<ScannedEpisode> = ["a", "b"]
            .iter()
            .map(|name| ScannedEpisode {
                path: PathBuf::from(format!("/series/dup/{name}.mkv")),
                season: Some(1),
                number: Some(1),
                order_key: "0/0001/0001".to_string(),
            })
            .collect();
        lib.sync_episodes(sid, &eps).unwrap();
        let rows = lib.episodes(sid).unwrap();
        assert_eq!(rows.len(), 2);

        // Two files share S01E01; progress must not bleed between them.
        lib.save_progress(sid, &rows[0], 60_000, false).unwrap();
        assert_eq!(lib.resume_position_ms(sid, &rows[0]).unwrap(), 60_000);
        assert_eq!(lib.resume_position_ms(sid, &rows[1]).unwrap(), 0);
    }

    #[test]
    fn resetting_the_last_progress_removes_the_sidecar() {
        let dir = tempdir();
        let lib = Library::in_memory().unwrap();
        let eps = vec![ScannedEpisode {
            path: dir.join("S01E01.mkv"),
            season: Some(1),
            number: Some(1),
            order_key: "0/0001/0001".to_string(),
        }];
        let sid = lib.add_series(&dir, "Show").unwrap();
        lib.sync_episodes(sid, &eps).unwrap();
        let first = lib.episodes(sid).unwrap().remove(0);
        lib.save_progress(sid, &first, 42_000, false).unwrap();
        assert!(dir.join(sidecar::SIDECAR_FILE).is_file());

        lib.reset_progress(sid).unwrap();
        assert!(
            !dir.join(sidecar::SIDECAR_FILE).exists(),
            "a sidecar with nothing left in it is removed, not left as a husk"
        );
        assert!(!lib.has_progress(sid).unwrap());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_folder_is_pruned_and_readd_restores_progress() {
        let dir = tempdir();
        let lib = Library::in_memory().unwrap();
        let eps = vec![ScannedEpisode {
            path: dir.join("S01E01.mkv"),
            season: Some(1),
            number: Some(1),
            order_key: "0/0001/0001".to_string(),
        }];
        let sid = lib.add_series(&dir, "Show").unwrap();
        lib.sync_episodes(sid, &eps).unwrap();
        let first = lib.episodes(sid).unwrap().remove(0);
        lib.save_progress(sid, &first, 42_000, false).unwrap();
        assert!(lib.has_progress(sid).unwrap());
        assert!(dir.join(sidecar::SIDECAR_FILE).is_file());

        // Simulate an unmounted drive by moving the folder away.
        let away = dir.with_file_name(format!(
            "{}_away",
            dir.file_name().unwrap().to_string_lossy()
        ));
        std::fs::rename(&dir, &away).unwrap();
        assert_eq!(lib.prune_missing_series().unwrap(), 1);
        assert!(lib.series(sid).unwrap().is_none());

        // Re-adding the moved-back folder restores the position.
        std::fs::rename(&away, &dir).unwrap();
        let again = lib.add_series(&dir, "Show").unwrap();
        lib.sync_episodes(again, &eps).unwrap();
        assert!(lib.has_progress(again).unwrap());
        let restored = lib.episodes(again).unwrap().remove(0);
        assert_eq!(lib.resume_position_ms(again, &restored).unwrap(), 42_000);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn prune_keeps_a_series_whose_progress_is_still_legacy() {
        // A missing-folder series still on legacy progress is kept.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE series (
              id INTEGER PRIMARY KEY,
              root_path TEXT NOT NULL UNIQUE,
              display_name TEXT NOT NULL,
              poster_path TEXT,
              added_at INTEGER NOT NULL);
            CREATE TABLE episode (
              id INTEGER PRIMARY KEY,
              series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
              path TEXT NOT NULL UNIQUE,
              season INTEGER,
              number INTEGER,
              order_key TEXT NOT NULL,
              duration_ms INTEGER,
              added_at INTEGER NOT NULL);
            CREATE TABLE progress (
              episode_id INTEGER PRIMARY KEY REFERENCES episode(id) ON DELETE CASCADE,
              position_ms INTEGER NOT NULL,
              watched INTEGER NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL);
            "#,
        )
        .unwrap();
        let sid: i64 = conn
            .execute(
                "INSERT INTO series (root_path, display_name, added_at)
                 VALUES ('/mnt/not-mounted/Show', 'Show', 0)",
                [],
            )
            .unwrap() as i64;
        let eid: i64 = conn
            .execute(
                "INSERT INTO episode (series_id, path, season, number, order_key, added_at)
                 VALUES (?1, '/mnt/not-mounted/Show/S01E01.mkv', 1, 1, '0/0001/0001', 0)",
                params![sid],
            )
            .unwrap() as i64;
        conn.execute(
            "INSERT INTO progress (episode_id, position_ms, watched, updated_at)
             VALUES (?1, 90_000, 0, 123)",
            params![eid],
        )
        .unwrap();

        let lib = Library {
            conn: parking_lot::Mutex::new(conn),
            sidecars: parking_lot::Mutex::new(HashMap::new()),
            keys: parking_lot::Mutex::new(HashMap::new()),
        };
        // A missing-folder series with legacy progress is kept.
        assert_eq!(lib.prune_missing_series().unwrap(), 0);
        assert!(lib.series(sid).unwrap().is_some());
    }

    #[test]
    fn migration_leaves_rows_for_a_folder_that_is_not_there() {
        // A pre-sidecar DB whose only series points at a missing folder.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE series (
              id INTEGER PRIMARY KEY,
              root_path TEXT NOT NULL UNIQUE,
              display_name TEXT NOT NULL,
              poster_path TEXT,
              added_at INTEGER NOT NULL);
            CREATE TABLE episode (
              id INTEGER PRIMARY KEY,
              series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
              path TEXT NOT NULL UNIQUE,
              season INTEGER,
              number INTEGER,
              order_key TEXT NOT NULL,
              duration_ms INTEGER,
              added_at INTEGER NOT NULL);
            CREATE TABLE progress (
              episode_id INTEGER PRIMARY KEY REFERENCES episode(id) ON DELETE CASCADE,
              position_ms INTEGER NOT NULL,
              watched INTEGER NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL);
            CREATE TABLE setting (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL);
            "#,
        )
        .unwrap();
        let sid: i64 = conn
            .execute(
                "INSERT INTO series (root_path, display_name, added_at)
                 VALUES ('/mnt/not-mounted/Show', 'Show', 0)",
                [],
            )
            .unwrap() as i64;
        let eid: i64 = conn
            .execute(
                "INSERT INTO episode (series_id, path, season, number, order_key, added_at)
                 VALUES (?1, '/mnt/not-mounted/Show/S01E01.mkv', 1, 1, '0/0001/0001', 0)",
                params![sid],
            )
            .unwrap() as i64;
        conn.execute(
            "INSERT INTO progress (episode_id, position_ms, watched, updated_at)
             VALUES (?1, 90_000, 0, 123)",
            params![eid],
        )
        .unwrap();

        let lib = Library {
            conn: parking_lot::Mutex::new(conn),
            sidecars: parking_lot::Mutex::new(HashMap::new()),
            keys: parking_lot::Mutex::new(HashMap::new()),
        };
        lib.migrate_legacy_progress().unwrap();

        // The row survives, for when the drive returns.
        let remaining: i64 = lib
            .conn
            .lock()
            .query_row("SELECT COUNT(*) FROM progress", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            remaining, 1,
            "progress for a missing folder is not discarded"
        );
    }

    #[test]
    fn subtitle_preference_is_per_series_and_supports_off() {
        let (lib, sid) = seeded();
        let other = lib.add_series(Path::new("/series/other"), "Other").unwrap();

        assert_eq!(lib.preferred_subtitle_lang(sid).unwrap(), None);

        lib.set_subtitle_lang(sid, Some("rus")).unwrap();
        assert_eq!(
            lib.preferred_subtitle_lang(sid).unwrap().as_deref(),
            Some("rus")
        );
        assert_eq!(lib.preferred_subtitle_lang(other).unwrap(), None);

        // "off" is stored verbatim, exactly as the explicit "no subtitles" choice.
        lib.set_subtitle_lang(sid, Some("off")).unwrap();
        assert_eq!(
            lib.preferred_subtitle_lang(sid).unwrap().as_deref(),
            Some("off")
        );
    }

    #[test]
    fn migrate_legacy_progress_lifts_rows_and_settings_into_the_sidecar() {
        let dir = tempdir();
        // Build the schema a pre-sidecar Murk would have left behind.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE series (
              id INTEGER PRIMARY KEY,
              root_path TEXT NOT NULL UNIQUE,
              display_name TEXT NOT NULL,
              poster_path TEXT,
              added_at INTEGER NOT NULL);
            CREATE TABLE episode (
              id INTEGER PRIMARY KEY,
              series_id INTEGER NOT NULL REFERENCES series(id) ON DELETE CASCADE,
              path TEXT NOT NULL UNIQUE,
              season INTEGER,
              number INTEGER,
              order_key TEXT NOT NULL,
              duration_ms INTEGER,
              added_at INTEGER NOT NULL);
            CREATE TABLE progress (
              episode_id INTEGER PRIMARY KEY REFERENCES episode(id) ON DELETE CASCADE,
              position_ms INTEGER NOT NULL,
              watched INTEGER NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL);
            CREATE TABLE setting (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL);
            "#,
        )
        .unwrap();
        let sid: i64 = conn
            .execute(
                "INSERT INTO series (root_path, display_name, added_at) VALUES (?1, 'Show', 0)",
                params![dir.to_string_lossy()],
            )
            .unwrap() as i64;
        let eid: i64 = conn
            .execute(
                "INSERT INTO episode (series_id, path, season, number, order_key, added_at)
                 VALUES (?1, ?2, 1, 1, '0/0001/0001', 0)",
                params![sid, dir.join("S01E01.mkv").to_string_lossy()],
            )
            .unwrap() as i64;
        conn.execute(
            "INSERT INTO progress (episode_id, position_ms, watched, updated_at)
             VALUES (?1, 90_000, 0, 123)",
            params![eid],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO setting (key, value) VALUES ('subtitle_lang:1', 'rus')",
            [],
        )
        .unwrap();

        let lib = Library {
            conn: parking_lot::Mutex::new(conn),
            sidecars: parking_lot::Mutex::new(HashMap::new()),
            keys: parking_lot::Mutex::new(HashMap::new()),
        };
        lib.migrate_legacy_progress().unwrap();

        // The sidecar now carries both the position and the subtitle choice.
        let sidecar = Sidecar::load(&dir);
        assert!(sidecar.progress.contains_key("1/1"));
        assert_eq!(sidecar.subtitle_lang.as_deref(), Some("rus"));
        // And the legacy storage is gone.
        let has_progress: bool = lib
            .conn
            .lock()
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master
                 WHERE type = 'table' AND name = 'progress')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_progress);

        std::fs::remove_dir_all(&dir).ok();
    }
}
