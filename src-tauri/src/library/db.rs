//! The library database.
//!
//! This file knows the season, the episode number, the running time and the
//! path, everything the product exists to withhold. That is fine: it lives in
//! Rust, and the only route from here to a screen is
//! [`crate::privacy::PlaybackView::project`].

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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

pub struct Library {
    conn: parking_lot::Mutex<Connection>,
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

            CREATE TABLE IF NOT EXISTS progress (
              episode_id INTEGER PRIMARY KEY REFERENCES episode(id) ON DELETE CASCADE,
              position_ms INTEGER NOT NULL,
              watched INTEGER NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL);

            CREATE TABLE IF NOT EXISTS setting (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL);
            "#,
        )?;
        Ok(Self {
            conn: parking_lot::Mutex::new(conn),
        })
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
        conn.execute(
            "DELETE FROM setting WHERE key = ?1",
            params![format!("subtitle_lang:{series_id}")],
        )?;
        Ok(())
    }

    // --- episodes ----------------------------------------------------------

    /// Insert what the scanner found, leaving existing rows (and their progress)
    /// alone. Files that disappeared are dropped, so a renamed folder does not
    /// leave the "continue" cursor pointing at nothing.
    pub fn sync_episodes(&self, series_id: i64, found: &[ScannedEpisode]) -> Result<()> {
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
        Ok(())
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

    /// Where "Continue" should take the user.
    ///
    /// Deliberately returns one episode and nothing else. There is no query in
    /// this file that hands the frontend a list of episodes with their numbers,
    /// because there is no screen that is allowed to show one.
    pub fn resume_target(&self, series_id: i64) -> Result<Option<EpisodeRow>> {
        let conn = self.conn.lock();

        // 1. the episode that was left unfinished most recently
        let started = conn
            .query_row(
                "SELECT e.id, e.series_id, e.path, e.season, e.number, e.order_key, e.duration_ms
                 FROM episode e JOIN progress p ON p.episode_id = e.id
                 WHERE e.series_id = ?1 AND p.watched = 0
                 ORDER BY p.updated_at DESC LIMIT 1",
                params![series_id],
                episode_from_row,
            )
            .optional()?;
        if started.is_some() {
            return Ok(started);
        }

        // 2. otherwise the first episode with no progress at all
        Ok(conn
            .query_row(
                "SELECT e.id, e.series_id, e.path, e.season, e.number, e.order_key, e.duration_ms
                 FROM episode e LEFT JOIN progress p ON p.episode_id = e.id
                 WHERE e.series_id = ?1 AND (p.watched IS NULL OR p.watched = 0)
                 ORDER BY e.order_key LIMIT 1",
                params![series_id],
                episode_from_row,
            )
            .optional()?)
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

    // --- progress ----------------------------------------------------------

    pub fn save_progress(&self, episode_id: i64, position_ms: i64, watched: bool) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO progress (episode_id, position_ms, watched, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(episode_id) DO UPDATE SET
               position_ms = excluded.position_ms,
               -- once watched, always watched: a stray seek to the start must
               -- not resurrect an episode the user has finished
               watched = MAX(progress.watched, excluded.watched),
               updated_at = excluded.updated_at",
            params![episode_id, position_ms, watched as i64, now_secs()],
        )?;
        Ok(())
    }

    pub fn resume_position_ms(&self, episode_id: i64) -> Result<i64> {
        let conn = self.conn.lock();
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT position_ms, watched FROM progress WHERE episode_id = ?1",
                params![episode_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        // A finished episode restarts from the beginning rather than from its
        // last second, which would otherwise show the credits and no more.
        Ok(match row {
            Some((pos, 0)) => pos,
            _ => 0,
        })
    }

    /// Whether the series has anything to forget: a stored position or a
    /// watched flag on any of its episodes. Unlike a resume position this stays
    /// true for a series watched to the end.
    pub fn has_progress(&self, series_id: i64) -> Result<bool> {
        let conn = self.conn.lock();
        Ok(conn.query_row(
            "SELECT EXISTS (
               SELECT 1 FROM progress p JOIN episode e ON e.id = p.episode_id
               WHERE e.series_id = ?1
             )",
            params![series_id],
            |r| r.get::<_, i64>(0),
        )? != 0)
    }

    /// How far the whole work has been watched, from 0 to 1.
    ///
    /// The unit is the folder, not the file: a series is its episodes end to
    /// end, and a film is its single file. Time is what counts, so a
    /// twenty-minute episode does not weigh the same as an hour-long one.
    ///
    /// Running times are only learned when a file is opened, so unplayed
    /// episodes have none. Pretending they are not there would make the bar
    /// leap backwards as the library fills in, so each unknown one is
    /// estimated at the average of the running times already known in the same
    /// series. With nothing known at all, the episode count is the fallback.
    ///
    /// Returns `None` for a series with no episodes.
    pub fn series_progress(&self, series_id: i64) -> Result<Option<f64>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT e.duration_ms, COALESCE(p.position_ms, 0), COALESCE(p.watched, 0)
               FROM episode e LEFT JOIN progress p ON p.episode_id = e.id
              WHERE e.series_id = ?1",
        )?;
        let rows: Vec<(Option<i64>, i64, bool)> = stmt
            .query_map(params![series_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0))
            })?
            .collect::<rusqlite::Result<_>>()?;

        if rows.is_empty() {
            return Ok(None);
        }

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
            // A watched episode counts whole however far into it the last
            // stored position happened to be.
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

    /// Forget every position and watched flag in a series, so it plays again
    /// from the very beginning.
    pub fn reset_progress(&self, series_id: i64) -> Result<()> {
        self.conn.lock().execute(
            "DELETE FROM progress WHERE episode_id IN
               (SELECT id FROM episode WHERE series_id = ?1)",
            params![series_id],
        )?;
        Ok(())
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

    // --- per-series subtitle preference ------------------------------------

    /// The language code of the subtitle track the user last chose for this
    /// series, so the next episode can re-select it. Track ids in mpv are
    /// per-file, so a language is the stable identity that survives the switch.
    pub fn preferred_subtitle_lang(&self, series_id: i64) -> Result<Option<String>> {
        self.get_setting(&format!("subtitle_lang:{series_id}"))
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

    /// Every episode of a series in playback order.
    fn all_episodes(lib: &Library, series_id: i64) -> Vec<EpisodeRow> {
        let mut out = Vec::new();
        let mut cursor = lib.first_episode(series_id).unwrap();
        while let Some(e) = cursor {
            cursor = lib.following(&e).unwrap();
            out.push(e);
        }
        out
    }

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

    #[test]
    fn series_progress_measures_the_whole_folder_in_time() {
        let (lib, sid) = seeded();
        let eps = all_episodes(&lib, sid);
        // Three half-hour episodes and one twice as long.
        for (i, e) in eps.iter().enumerate() {
            let ms = if i == 3 { 3_600_000 } else { 1_800_000 };
            lib.record_duration(e.id, ms).unwrap();
        }
        assert_eq!(lib.series_progress(sid).unwrap(), Some(0.0));

        lib.save_progress(eps[0].id, 1_800_000, true).unwrap();
        assert!((lib.series_progress(sid).unwrap().unwrap() - 0.2).abs() < 1e-6);

        // A position inside an unfinished episode counts for its own length.
        lib.save_progress(eps[1].id, 900_000, false).unwrap();
        assert!((lib.series_progress(sid).unwrap().unwrap() - 0.3).abs() < 1e-6);
    }

    #[test]
    fn series_progress_falls_back_to_counting_episodes() {
        let (lib, sid) = seeded();
        let eps = all_episodes(&lib, sid);
        lib.save_progress(eps[0].id, 0, true).unwrap();
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
        lib.save_progress(first.id, 600_000, false).unwrap();
        let again = lib.resume_target(sid).unwrap().unwrap();
        assert_eq!(again.id, first.id);
        assert_eq!(lib.resume_position_ms(first.id).unwrap(), 600_000);
    }

    #[test]
    fn a_watched_episode_hands_over_to_the_next_one() {
        let (lib, sid) = seeded();
        let first = lib.resume_target(sid).unwrap().unwrap();
        lib.save_progress(first.id, 2_600_000, true).unwrap();
        let next = lib.resume_target(sid).unwrap().unwrap();
        assert_eq!(next.number, Some(2));
    }

    #[test]
    fn watched_is_sticky_and_restarts_from_zero() {
        let (lib, sid) = seeded();
        let first = lib.resume_target(sid).unwrap().unwrap();
        lib.save_progress(first.id, 2_600_000, true).unwrap();
        // rewinding to the start must not un-watch it
        lib.save_progress(first.id, 0, false).unwrap();
        assert_eq!(lib.resume_target(sid).unwrap().unwrap().number, Some(2));
        assert_eq!(lib.resume_position_ms(first.id).unwrap(), 0);
    }

    #[test]
    fn reset_progress_forgets_every_position_and_watched_flag() {
        let (lib, sid) = seeded();
        let first = lib.resume_target(sid).unwrap().unwrap();
        lib.save_progress(first.id, 600_000, false).unwrap();
        lib.reset_progress(sid).unwrap();
        assert_eq!(lib.resume_position_ms(first.id).unwrap(), 0);
        assert_eq!(lib.resume_target(sid).unwrap().unwrap().number, Some(1));
    }

    #[test]
    fn a_finished_series_has_no_resume_target_but_still_has_a_first_episode() {
        let (lib, sid) = seeded();
        while let Some(episode) = lib.resume_target(sid).unwrap() {
            lib.save_progress(episode.id, 2_600_000, true).unwrap();
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
        lib.save_progress(first.id, 42_000, false).unwrap();

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
        assert_eq!(lib.resume_position_ms(first.id).unwrap(), 42_000);
    }

    #[test]
    fn subtitle_preference_is_per_series_off_supported_and_cleared_on_removal() {
        let (lib, sid) = seeded();
        let other = lib.add_series(Path::new("/series/other"), "Other").unwrap();

        assert_eq!(lib.preferred_subtitle_lang(sid).unwrap(), None);

        lib.set_setting(&format!("subtitle_lang:{sid}"), "rus")
            .unwrap();
        assert_eq!(
            lib.preferred_subtitle_lang(sid).unwrap().as_deref(),
            Some("rus")
        );
        assert_eq!(lib.preferred_subtitle_lang(other).unwrap(), None);

        // "off" is stored verbatim, exactly as the explicit "no subtitles" choice.
        lib.set_setting(&format!("subtitle_lang:{sid}"), "off")
            .unwrap();
        assert_eq!(
            lib.preferred_subtitle_lang(sid).unwrap().as_deref(),
            Some("off")
        );

        lib.set_setting(&format!("subtitle_lang:{sid}"), "rus")
            .unwrap();
        lib.remove_series(sid).unwrap();
        assert_eq!(lib.preferred_subtitle_lang(sid).unwrap(), None);
    }
}
