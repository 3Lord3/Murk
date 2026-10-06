//! Skipping openings, endings, recaps and previews.
//!
//! TheIntroDB first (it rescales timestamps to the file's length), with
//! AniSkip as the anime fallback. Identity comes from the filename title and
//! the parsed season and episode, or from a `tmdb-12345` / `tt0123456` marker.
//! Segments that do not fit the file are dropped, and they never reach the
//! frontend: they live on [`PlayerHandle`] with the playhead.
//!
//! [`PlayerHandle`]: crate::player::PlayerHandle

use anyhow::Context;
use regex::Regex;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tauri::{AppHandle, Manager};
use ureq::Agent;

use crate::library::MediaKind;
use crate::AppState;

/// The kind of stretch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    Opening,
    Ending,
    Recap,
    Preview,
}

/// A stretch to skip, in seconds from the start.
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub kind: SegmentKind,
    pub start: f64,
    pub end: f64,
}

/// Segments known for one file, already fitted to its length.
#[derive(Debug, Clone, Default)]
pub struct SkipTimes {
    pub segments: Vec<Segment>,
}

/// mpv lands a seek a little off target, so a position this close to the end
/// still counts as inside.
const SKIP_MARGIN_SEC: f64 = 0.5;

/// Shorter than this is noise, not an opening.
const MIN_SEGMENT_SEC: f64 = 3.0;

/// How far AniSkip's `episodeLength` may drift before it is another cut.
const LENGTH_TOLERANCE_SEC: f64 = 30.0;
const LENGTH_TOLERANCE_FRACTION: f64 = 0.02;

impl SkipTimes {
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Where to jump to, if a playhead at `position` sits inside a segment.
    pub fn target(&self, position: f64) -> Option<f64> {
        self.segments
            .iter()
            .find(|s| position >= s.start && position < s.end - SKIP_MARGIN_SEC)
            .map(|s| s.end)
    }
}

// --- identity ---------------------------------------------------------------

/// Everything a lookup needs, copied out of the library for the worker thread.
pub struct Identity {
    pub episode_id: i64,
    /// Cleaned filename title first, then the folder name.
    pub titles: Vec<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub kind: MediaKind,
    /// The episode file, for a `tmdb-12345` / `tt0123456` marker.
    pub path: std::path::PathBuf,
    /// Used to reject or rescale timestamps.
    pub duration_ms: Option<i64>,
}

impl Identity {
    fn is_episode(&self) -> bool {
        self.kind == MediaKind::Series && self.episode.is_some()
    }
}

/// TheIntroDB takes a TMDB id or an IMDB id, plus a season for TV.
#[derive(Debug, Clone, PartialEq, Eq)]
enum IntroDbRef {
    Tmdb(i64, Option<u32>),
    Imdb(String),
}

impl IntroDbRef {
    fn parameter(&self) -> String {
        match self {
            IntroDbRef::Tmdb(id, _) => format!("tmdb_id={id}"),
            IntroDbRef::Imdb(imdb) => format!("imdb_id={imdb}"),
        }
    }

    fn season(&self, fallback: Option<u32>) -> Option<u32> {
        match self {
            IntroDbRef::Tmdb(_, Some(season)) => Some(*season),
            _ => fallback,
        }
    }
}

// --- the store --------------------------------------------------------------

/// Resolved ids and fetched segments, cached for the run. "Not found" is cached
/// too, so it is not retried every episode.
pub struct Store {
    agent: Agent,
    mal_ids: parking_lot::Mutex<HashMap<String, Option<i64>>>,
    introdb_refs: parking_lot::Mutex<HashMap<String, Option<IntroDbRef>>>,
    times: parking_lot::Mutex<HashMap<i64, Option<Arc<SkipTimes>>>>,
    /// Serialises lookups so an episode is fetched once.
    lookups: parking_lot::Mutex<()>,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    pub fn new() -> Self {
        let agent = ureq::builder()
            .user_agent(concat!("Murk/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build();
        Self {
            agent,
            mal_ids: parking_lot::Mutex::new(HashMap::new()),
            introdb_refs: parking_lot::Mutex::new(HashMap::new()),
            times: parking_lot::Mutex::new(HashMap::new()),
            lookups: parking_lot::Mutex::new(()),
        }
    }

    /// Segments for one file, or `None`. Hits the network, so call it off the
    /// playback path.
    pub fn fetch(&self, identity: &Identity) -> Option<Arc<SkipTimes>> {
        if let Some(cached) = self.times.lock().get(&identity.episode_id) {
            return cached.clone();
        }
        // One lock is enough: lookups run off the playback path, one file at a
        // time.
        let _guard = self.lookups.lock();
        if let Some(cached) = self.times.lock().get(&identity.episode_id) {
            return cached.clone();
        }
        let times = self.look_up(identity);
        self.times.lock().insert(identity.episode_id, times.clone());
        times
    }

    /// The cached value for a file, if it was already fetched.
    pub fn cached(&self, episode_id: i64) -> Option<Arc<SkipTimes>> {
        self.times.lock().get(&episode_id).and_then(Clone::clone)
    }

    fn look_up(&self, identity: &Identity) -> Option<Arc<SkipTimes>> {
        let mal_id = self.mal_id(&identity.titles, identity.season);

        // TheIntroDB first: it covers more than anime.
        let mut segments = self
            .introdb_ref(identity, mal_id)
            .and_then(|reference| self.introdb(&reference, identity))
            .unwrap_or_default();

        // AniSkip fills in an opening or ending TheIntroDB lacks.
        if let (Some(mal_id), Some(episode)) = (mal_id, identity.episode) {
            let aniskip = self
                .aniskip(mal_id, episode, identity.duration_ms)
                .unwrap_or_default();
            for kind in [SegmentKind::Opening, SegmentKind::Ending] {
                if !segments.iter().any(|s| s.kind == kind) {
                    segments.extend(aniskip.iter().filter(|s| s.kind == kind).copied());
                }
            }
        }

        let times = finish(segments, identity);
        match times {
            Some(_) => tracing::debug!(episode_id = identity.episode_id, "skip: segments found"),
            None => tracing::debug!(episode_id = identity.episode_id, "skip: nothing found"),
        }
        times
    }

    /// IntroDB reference: an id in the filename, else the MyAnimeList mapping.
    fn introdb_ref(&self, identity: &Identity, mal_id: Option<i64>) -> Option<IntroDbRef> {
        if let Some(reference) = embedded_id(&identity.path) {
            tracing::debug!("skip: using the id embedded in the filename");
            return Some(reference);
        }
        mal_id.and_then(|mal_id| self.anime_mapping(mal_id))
    }

    /// MyAnimeList id for a title, cached; tries each candidate title.
    fn mal_id(&self, titles: &[String], season: Option<u32>) -> Option<i64> {
        for title in titles {
            let base = keyword(title);
            let mut queries = vec![search_question(title, season)];
            if queries[0] != base {
                queries.push(base);
            }
            for query in queries {
                if query.is_empty() {
                    continue;
                }
                if let Some(cached) = self.mal_ids.lock().get(&query) {
                    if cached.is_some() {
                        return *cached;
                    }
                    continue;
                }
                let found = self.mal_search(&query);
                self.mal_ids.lock().insert(query, found);
                if found.is_some() {
                    return found;
                }
            }
        }
        None
    }

    fn mal_search(&self, query: &str) -> Option<i64> {
        let url = format!("{MAL_SEARCH}?type=anime&keyword={}", encode(query));
        let response = get_json::<MalSearch>(&self.agent, &url)?;
        response
            .categories
            .into_iter()
            .find(|c| c.kind == "anime")
            .and_then(|c| c.items.into_iter().find(|i| i.kind == "anime"))
            .map(|i| i.id)
    }

    /// MyAnimeList id to TMDB id and season, via Anime Relations Mapping.
    fn anime_mapping(&self, mal_id: i64) -> Option<IntroDbRef> {
        let key = format!("mal:{mal_id}");
        if let Some(cached) = self.introdb_refs.lock().get(&key) {
            return cached.clone();
        }
        let url =
            format!("{ARM}?source=myanimelist&id={mal_id}&include=themoviedb,themoviedb-season");
        let reference = get_json::<ArmMapping>(&self.agent, &url).and_then(|m| {
            m.themoviedb
                .map(|id| IntroDbRef::Tmdb(id, m.themoviedb_season))
        });
        self.introdb_refs.lock().insert(key, reference.clone());
        reference
    }

    fn introdb(&self, reference: &IntroDbRef, identity: &Identity) -> Option<Vec<Segment>> {
        let mut url = format!("{INTRODB}?{}", reference.parameter());
        if identity.is_episode() {
            let season = reference.season(identity.season).unwrap_or(1);
            let episode = identity.episode.unwrap();
            url.push_str(&format!("&season={season}&episode={episode}"));
        }
        if let Some(ms) = identity.duration_ms {
            url.push_str(&format!("&duration_ms={ms}"));
        }
        let response = get_json::<IntroDbResponse>(&self.agent, &url)?;
        Some(introdb_segments(response, identity.duration_ms))
    }

    fn aniskip(&self, mal_id: i64, episode: u32, duration_ms: Option<i64>) -> Option<Vec<Segment>> {
        let url = format!("{ANISKIP}/{mal_id}/{episode}?types[]=op&types[]=ed&episodeLength=0");
        let response = get_json::<AniSkipResponse>(&self.agent, &url)?;
        if !response.found {
            return None;
        }

        let segments = aniskip_segments(response.results, duration_ms);
        (!segments.is_empty()).then_some(segments)
    }
}

/// Fit a provider's answer to the file.
fn finish(mut segments: Vec<Segment>, identity: &Identity) -> Option<Arc<SkipTimes>> {
    segments = fit_to_duration(segments, identity.duration_ms);
    if segments.is_empty() {
        return None;
    }
    // Sources are concatenated, so restore playback order.
    segments.sort_by(|a, b| a.start.total_cmp(&b.start));
    Some(Arc::new(SkipTimes { segments }))
}

// --- fitting and mapping ----------------------------------------------------

/// Drop or trim segments that cannot fit the file; with no known length they
/// pass through.
fn fit_to_duration(mut segments: Vec<Segment>, duration_ms: Option<i64>) -> Vec<Segment> {
    if let Some(duration_ms) = duration_ms.filter(|d| *d > 0) {
        let duration = duration_ms as f64 / 1000.0;
        segments.retain_mut(|segment| {
            if segment.start >= duration {
                return false;
            }
            if segment.end > duration {
                segment.end = duration;
            }
            segment.end - segment.start >= MIN_SEGMENT_SEC
        });
    }
    segments
}

/// AniSkip reports its `episodeLength` in seconds.
fn length_matches(reported_sec: Option<f64>, duration_ms: Option<i64>) -> bool {
    let (Some(reported), Some(duration)) = (reported_sec, duration_ms.filter(|d| *d > 0)) else {
        // Nothing to compare against; trust the provider.
        return true;
    };
    let duration = duration as f64 / 1000.0;
    let tolerance = (duration * LENGTH_TOLERANCE_FRACTION).max(LENGTH_TOLERANCE_SEC);
    (reported - duration).abs() <= tolerance
}

// --- providers' JSON --------------------------------------------------------

/// MyAnimeList title search (no key).
const MAL_SEARCH: &str = "https://myanimelist.net/search/prefix.json";
/// Anime Relations Mapping: MAL / AniList / TMDB / TVDB ids.
const ARM: &str = "https://arm.haglund.dev/api/v2/ids";
/// TheIntroDB read API; `duration_ms` rescales markers.
const INTRODB: &str = "https://api.theintrodb.org/v3/media";
/// AniSkip.
const ANISKIP: &str = "https://api.aniskip.com/v2/skip-times";

#[derive(Deserialize)]
struct MalSearch {
    #[serde(default)]
    categories: Vec<MalCategory>,
}

#[derive(Deserialize)]
struct MalCategory {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    items: Vec<MalItem>,
}

#[derive(Deserialize)]
struct MalItem {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct ArmMapping {
    #[serde(default)]
    themoviedb: Option<i64>,
    #[serde(default, rename = "themoviedb-season")]
    themoviedb_season: Option<u32>,
}

#[derive(Deserialize)]
struct AniSkipResponse {
    found: bool,
    #[serde(default)]
    results: Vec<AniSkipResult>,
}

#[derive(Deserialize)]
struct AniSkipResult {
    interval: AniSkipInterval,
    #[serde(rename = "skipType")]
    skip_type: String,
    #[serde(default, rename = "episodeLength")]
    episode_length: Option<f64>,
}

#[derive(Deserialize)]
struct AniSkipInterval {
    #[serde(rename = "startTime")]
    start: f64,
    #[serde(rename = "endTime")]
    end: f64,
}

#[derive(Deserialize, Default)]
struct IntroDbResponse {
    #[serde(default)]
    intro: Vec<Marker>,
    #[serde(default)]
    recap: Vec<Marker>,
    #[serde(default)]
    credits: Vec<Marker>,
    #[serde(default)]
    preview: Vec<Marker>,
}

#[derive(Deserialize)]
struct Marker {
    #[serde(default, rename = "start_ms")]
    start_ms: Option<i64>,
    #[serde(default, rename = "end_ms")]
    end_ms: Option<i64>,
}

/// Keep AniSkip openings and endings whose reported length fits the file; the
/// check is per segment, since one answer can mix cuts (Attack on Titan
/// episode 1 times its opening to a longer release).
fn aniskip_segments(results: Vec<AniSkipResult>, duration_ms: Option<i64>) -> Vec<Segment> {
    results
        .into_iter()
        .filter_map(|r| {
            let kind = match r.skip_type.as_str() {
                "op" => SegmentKind::Opening,
                "ed" => SegmentKind::Ending,
                _ => return None,
            };
            if r.interval.end <= r.interval.start {
                return None;
            }
            if !length_matches(r.episode_length, duration_ms) {
                tracing::debug!(
                    reported = ?r.episode_length,
                    ?duration_ms,
                    "skip: AniSkip segment is for a different cut; ignoring"
                );
                return None;
            }
            Some(Segment {
                kind,
                start: r.interval.start,
                end: r.interval.end,
            })
        })
        .collect()
}

/// TheIntroDB lists, flattened to seconds. A `null` end means end of file.
fn introdb_segments(response: IntroDbResponse, duration_ms: Option<i64>) -> Vec<Segment> {
    let duration = duration_ms.map(|d| d as f64 / 1000.0);
    let mut segments = Vec::new();
    let lists = [
        (SegmentKind::Opening, response.intro),
        (SegmentKind::Recap, response.recap),
        (SegmentKind::Ending, response.credits),
        (SegmentKind::Preview, response.preview),
    ];
    for (kind, markers) in lists {
        for marker in markers {
            let Some(end) = marker.end_ms.map(|e| e as f64 / 1000.0).or(duration) else {
                continue;
            };
            let start = marker.start_ms.unwrap_or(0) as f64 / 1000.0;
            if end > start {
                segments.push(Segment { kind, start, end });
            }
        }
    }
    segments
}

// --- request helpers --------------------------------------------------------

fn get_json<T: DeserializeOwned>(agent: &Agent, url: &str) -> Option<T> {
    let response = match agent.get(url).call() {
        Ok(response) => response,
        // A 4xx is "no data", not a failure worth propagating.
        Err(ureq::Error::Status(code, _)) => {
            tracing::debug!(code, url, "skip: request rejected");
            return None;
        }
        Err(e) => {
            tracing::warn!(%e, "skip: request failed");
            return None;
        }
    };
    let text = match response.into_string() {
        Ok(text) => text,
        Err(e) => {
            tracing::warn!(%e, "skip: could not read response");
            return None;
        }
    };
    match serde_json::from_str(&text).context("parsing a skip-lookup response") {
        Ok(value) => Some(value),
        Err(e) => {
            tracing::warn!(%e, "skip: could not parse response");
            None
        }
    }
}

// --- identity helpers -------------------------------------------------------

static TMDB_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)tmdb[\s_.-]*(\d+)").expect("static pattern"));
static IMDB_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\btt(\d{7,9})\b").expect("static pattern"));

/// A `tmdb-12345` or `tt0123456` marker in the filename, for naming the media
/// outright.
fn embedded_id(path: &Path) -> Option<IntroDbRef> {
    let name = path.file_name()?.to_str()?;
    if let Some(caps) = TMDB_MARKER.captures(name) {
        if let Some(id) = caps.get(1).and_then(|m| m.as_str().parse::<i64>().ok()) {
            return Some(IntroDbRef::Tmdb(id, None));
        }
    }
    if let Some(caps) = IMDB_MARKER.captures(name) {
        if let Some(digits) = caps.get(1) {
            return Some(IntroDbRef::Imdb(format!("tt{}", digits.as_str())));
        }
    }
    None
}

/// Where the title ends in a release filename: season or episode number, a
/// `1x05` pair, an episode word, or a quality/codec tag.
const TITLE_END_MARKERS: &[&str] = &[
    r"(?i)\bs\s*\.?\s*\d{1,3}",
    r"(?i)\be\s*\.?\s*\d{1,4}",
    r"(?i)\bep\s*\.?\s*\d{1,4}",
    r"(?i)\b(?:episode|season|серия|эпизод|сезон)\b",
    r"\b\d{1,2}\s*x\s*\d{1,4}\b",
    r"[-–—]\s*\d{1,3}\s*[-–—]",
    r"[-–—]\s*\d{1,3}\b",
    r"(?i)\b(?:2160p|1440p|1080p|720p|480p|web-?dl|web-?rip|bd-?rip|bluray|blu-?ray|remux|hevc|h\.?26[45]|x26[45]|aac|ac3|flac|opus)\b",
];

static TITLE_END: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    TITLE_END_MARKERS
        .iter()
        .map(|pattern| Regex::new(pattern).expect("static pattern"))
        .collect()
});

/// Release-tag brackets, dropped before the title is read.
static BRACKETED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\[\(\{][^\]\)\}]*[\]\)\}]").expect("static pattern"));

/// The title in a release filename, without the group's tags.
///
/// `[SOFCJ-Raws] Shingeki no Kyojin - S1 - E01 [WEB-DL KP 1080p]` becomes
/// `Shingeki no Kyojin`.
fn filename_title(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    // Bracketed groups are release tags, dots and underscores separate words.
    let unbracketed = BRACKETED.replace_all(stem, " ");
    let normalized: String = unbracketed
        .chars()
        .map(|c| if c == '.' || c == '_' { ' ' } else { c })
        .collect();

    let mut end = normalized.len();
    for pattern in TITLE_END.iter() {
        if let Some(found) = pattern.find(&normalized) {
            end = end.min(found.start());
        }
    }

    let mut title = normalized[..end]
        .trim_matches(|c: char| c == '-' || c == '–' || c == '—' || c.is_whitespace())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    // A trailing year is release metadata.
    if let Some((head, last)) = title.rsplit_once(' ') {
        if is_year(last) {
            title = head.trim_end().to_string();
        }
    }

    title
        .chars()
        .any(char::is_alphanumeric)
        .then(|| title.to_string())
}

fn is_year(token: &str) -> bool {
    token.len() == 4
        && token.chars().all(|c| c.is_ascii_digit())
        && (token.starts_with("19") || token.starts_with("20"))
}

/// A plain keyword for MyAnimeList: punctuation becomes spaces.
fn keyword(name: &str) -> String {
    let mapped: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether the title already names its season.
fn names_a_season(name: &str) -> bool {
    let lower = name.to_lowercase();
    ["season", "сезон", "saison", "staffel", "cour"]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// The MyAnimeList keyword to try first; multi-season anime are separate
/// entries, so a revealed season goes into the search.
fn search_question(display_name: &str, season: Option<u32>) -> String {
    let base = keyword(display_name);
    match season {
        Some(s) if s >= 2 && !names_a_season(display_name) => {
            format!("{base} {} season", ordinal(s))
        }
        _ => base,
    }
}

/// 2 -> 2nd, 11 -> 11th.
fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (1, 11) | (2, 12) | (3, 13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// Percent-encode a query value, keeping the unreserved characters.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// --- the playback hook ------------------------------------------------------

/// Whether automatic skipping is on; absent means on, since the setting only
/// records a deliberate "off".
pub fn enabled(library: &crate::library::Library) -> bool {
    library.get_setting("auto_skip").ok().flatten().as_deref() != Some("off")
}

/// Look up the running file's segments and install them on the player. Called
/// after `FileLoaded`, when the length is known.
pub fn spawn_lookup(app: &AppHandle, duration_ms: Option<i64>) {
    let identity = match identity_for(app, duration_ms) {
        Some(identity) => identity,
        None => return,
    };
    let app = app.clone();
    let spawn = std::thread::Builder::new()
        .name("murk-skip".into())
        .spawn(move || {
            let state: tauri::State<'_, AppState> = app.state();
            let times = state.skip.fetch(&identity);
            // A slow lookup must not skip into a file that has since started.
            if state.player.current().map(|c| c.episode_id) == Some(identity.episode_id) {
                state.player.set_skip_times(times);
            }
        });
    if let Err(e) = spawn {
        tracing::warn!("could not start the skip lookup: {e}");
    }
}

/// The lookup's inputs, or `None` if skipping is off or nothing plays.
fn identity_for(app: &AppHandle, duration_ms: Option<i64>) -> Option<Identity> {
    let state = app.state::<AppState>();
    if !enabled(&state.library) {
        return None;
    }
    let current = state.player.current()?;
    let episode = state.library.episode(current.episode_id).ok().flatten()?;
    let series = state.library.series(current.series_id).ok().flatten()?;
    // Filename title first, folder as fallback.
    let mut titles: Vec<String> = filename_title(&episode.path).into_iter().collect();
    let folder = series.display_name.trim().to_string();
    if !folder.is_empty() && !titles.contains(&folder) {
        titles.push(folder);
    }
    Some(Identity {
        episode_id: episode.id,
        titles,
        season: episode.season,
        episode: episode.number,
        kind: series.kind,
        path: episode.path,
        duration_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(duration_ms: Option<i64>) -> Identity {
        Identity {
            episode_id: 1,
            titles: vec!["Show".into()],
            season: Some(1),
            episode: Some(1),
            kind: MediaKind::Series,
            path: "/x/Show/Episode 1.mkv".into(),
            duration_ms,
        }
    }

    fn times() -> SkipTimes {
        SkipTimes {
            segments: vec![
                Segment {
                    kind: SegmentKind::Opening,
                    start: 10.0,
                    end: 100.0,
                },
                Segment {
                    kind: SegmentKind::Ending,
                    start: 1300.0,
                    end: 1400.0,
                },
            ],
        }
    }

    #[test]
    fn jumps_only_from_inside_a_segment() {
        assert_eq!(times().target(0.0), None);
        assert_eq!(times().target(9.0), None);
        assert_eq!(times().target(10.0), Some(100.0));
        assert_eq!(times().target(50.0), Some(100.0));
        assert_eq!(times().target(1300.0), Some(1400.0));
    }

    #[test]
    fn does_not_retrigger_on_the_landing_of_its_own_seek() {
        // mpv lands exactly on the end; the margin keeps the same segment from
        // skipping a second time.
        assert_eq!(times().target(100.0), None);
        assert_eq!(times().target(99.9), None);
    }

    #[test]
    fn segments_that_do_not_fit_the_file_are_dropped_or_trimmed() {
        let fitted = fit_to_duration(times().segments, Some(1_350_000));
        assert_eq!(fitted.len(), 2);
        // The ending ran past the file, so it is trimmed to the end.
        assert_eq!(fitted[1].end, 1350.0);

        // A file that ends inside the opening keeps only the trimmed part.
        let fitted = fit_to_duration(times().segments, Some(60_000));
        assert_eq!(fitted.len(), 1);
        assert_eq!(fitted[0].start, 10.0);
        assert_eq!(fitted[0].end, 60.0);

        // A file that ends before either segment has none left.
        assert!(fit_to_duration(times().segments, Some(5_000)).is_empty());
    }

    #[test]
    fn a_mismatched_aniskip_length_is_rejected() {
        // Attack on Titan S01E01: AniSkip says 25:40, the TV cut is 24:07.
        assert!(!length_matches(Some(1_540.061), Some(1_447_000)));
        // A few seconds apart is the same cut.
        assert!(length_matches(Some(1_450.032), Some(1_447_000)));
        // Without a known duration there is nothing to judge.
        assert!(length_matches(Some(1_540.061), None));
    }

    #[test]
    fn keyword_strips_punctuation() {
        assert_eq!(
            keyword("Black Clover (170 episodes)"),
            "Black Clover 170 episodes"
        );
        assert_eq!(keyword("Fate/stay.night"), "Fate stay night");
    }

    #[test]
    fn a_revealed_season_is_searched_for_unless_the_title_names_one() {
        assert_eq!(search_question("Show", Some(2)), "Show 2nd season");
        assert_eq!(search_question("Show", Some(1)), "Show");
        assert_eq!(search_question("Show", None), "Show");
        assert_eq!(search_question("Show Season 2", Some(2)), "Show Season 2");
        assert_eq!(search_question("Сериал Сезон 3", Some(3)), "Сериал Сезон 3");
    }

    #[test]
    fn ordinals_cover_the_teens() {
        assert_eq!(ordinal(1), "1st");
        assert_eq!(ordinal(2), "2nd");
        assert_eq!(ordinal(3), "3rd");
        assert_eq!(ordinal(4), "4th");
        assert_eq!(ordinal(11), "11th");
        assert_eq!(ordinal(21), "21st");
    }

    #[test]
    fn embedded_ids_are_read_from_the_filename() {
        assert_eq!(
            embedded_id(Path::new("/x/Some.Movie.2024.tmdb-550.mkv")),
            Some(IntroDbRef::Tmdb(550, None))
        );
        assert_eq!(
            embedded_id(Path::new("/x/Show.S01E02.tmdb1429.mkv")),
            Some(IntroDbRef::Tmdb(1429, None))
        );
        assert_eq!(
            embedded_id(Path::new("/x/Fight.Club.tt0137523.mkv")),
            Some(IntroDbRef::Imdb("tt0137523".into()))
        );
        assert_eq!(embedded_id(Path::new("/x/Show.S01E01.mkv")), None);
    }

    #[test]
    fn the_title_is_cleaned_out_of_a_release_filename() {
        let cases = [
            (
                "/x/[SOFCJ-Raws] Shingeki no Kyojin - S1 - E01 [WEB-DL KP 1080p].mkv",
                Some("Shingeki no Kyojin"),
            ),
            (
                "/x/Show.Name.S01E05.1080p.WEB-DL.x264-GROUP.mkv",
                Some("Show Name"),
            ),
            ("/x/Show.Name.2019.S01E05.mkv", Some("Show Name")),
            ("/x/black_clover_ep10.mp4", Some("black clover")),
            ("/x/[Group] Show - 12 [BDRip][x265].mkv", Some("Show")),
            // Nothing left once the episode marker goes: the folder is used.
            ("/x/Episode 1.mkv", None),
        ];
        for (path, want) in cases {
            assert_eq!(
                filename_title(Path::new(path)).as_deref(),
                want,
                "cleaning {path:?}"
            );
        }
    }

    #[test]
    fn introdb_lists_become_segments_with_null_ends_at_the_duration() {
        let response = IntroDbResponse {
            intro: vec![Marker {
                start_ms: Some(123_666),
                end_ms: Some(215_671),
            }],
            preview: vec![Marker {
                start_ms: Some(1_432_000),
                end_ms: None,
            }],
            ..IntroDbResponse::default()
        };
        let segments = introdb_segments(response, Some(1_447_000));
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].kind, SegmentKind::Opening);
        assert!((segments[1].end - 1447.0).abs() < f64::EPSILON);
    }

    #[test]
    fn aniskip_rejects_only_the_segments_for_another_cut() {
        let raw = r#"{
            "found": true,
            "results": [
                {"interval":{"startTime":47.365,"endTime":137.365},"skipType":"op","episodeLength":1540.061},
                {"interval":{"startTime":1342.795,"endTime":1430.616},"skipType":"ed","episodeLength":1446.9973},
                {"interval":{"startTime":200.0,"endTime":210.0},"skipType":"recap"}
            ]
        }"#;
        let response: AniSkipResponse = serde_json::from_str(raw).unwrap();
        // Against a 24:07 TV cut, the opening (submitted for 25:40) falls away
        // and the ending stays. This is Attack on Titan's first episode.
        let segments = aniskip_segments(response.results, Some(1_447_000));
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].kind, SegmentKind::Ending);
        assert!((segments[0].start - 1342.795).abs() < f64::EPSILON);
    }

    #[test]
    fn an_episode_only_queries_introdb_with_a_season_and_number() {
        assert!(episode(None).is_episode());
        let mut movie = episode(None);
        movie.kind = MediaKind::Movie;
        movie.episode = None;
        assert!(!movie.is_episode());
    }

    #[test]
    fn introdb_reference_season_prefers_the_mapping() {
        assert_eq!(IntroDbRef::Tmdb(1429, Some(2)).season(Some(1)), Some(2));
        assert_eq!(IntroDbRef::Tmdb(1429, None).season(Some(1)), Some(1));
        assert_eq!(
            IntroDbRef::Imdb("tt0137523".into()).season(Some(1)),
            Some(1)
        );
    }
}
