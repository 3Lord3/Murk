//! Turning a filename into a sort order.
//!
//! The result of this module is used **only for ordering** episodes. It is not
//! displayed, and it reaches the frontend only through
//! [`crate::privacy::PlaybackView::project`], which will usually drop it. That
//! is why a wrong guess here is a cosmetic bug and not a spoiler: the worst case
//! is that "next" plays the wrong file, not that a number appears on screen.

use regex::Regex;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParsedEpisode {
    pub season: Option<u32>,
    pub number: Option<u32>,
}

impl ParsedEpisode {
    fn nothing() -> Self {
        Self::default()
    }
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern")
}

/// Release-group noise that would otherwise be mistaken for an episode number.
/// `1080p`, `x264` and `(2019)` all contain perfectly good small integers.
fn noise() -> &'static Vec<Regex> {
    static NOISE: OnceLock<Vec<Regex>> = OnceLock::new();
    NOISE.get_or_init(|| {
        vec![
            re(r"(?i)\b\d{3,4}[pi]\b"),   // 1080p, 720i
            re(r"(?i)\b[xh]\.?26[45]\b"), // x264, h.265
            re(r"(?i)\b(?:hevc|avc|av1|vp9|xvid|divx)\b"),
            re(r"(?i)\b(?:aac|ac3|eac3|dts(?:-hd)?|truehd|flac|opus|mp3)\b"),
            re(r"(?i)\b\d{1,2}\.\d\b"),  // 5.1, 7.1 channel layouts
            re(r"(?i)\b\d{1,2}bits?\b"), // 10bit
            re(r"(?i)\b(?:bluray|bdrip|brrip|web-?dl|webrip|hdtv|dvdrip|remux|uhd|hdr\d*|sdr)\b"),
            re(r"(?i)\b(?:proper|repack|extended|uncut|multi|dual|dub|sub|rus|eng|ita)\b"),
            re(r"\b(?:19|20)\d{2}\b"), // a release year
        ]
    })
}

fn strip_noise(input: &str) -> String {
    // Dots and underscores are separators in scene naming; make them spaces so
    // that `\b`-anchored patterns behave.
    // The channel-layout pattern needs the dots intact, so it runs before the
    // separator swap; everything else runs after.
    let mut s = re(r"(?i)\b\d{1,2}\.\d\b")
        .replace_all(input, " ")
        .into_owned();
    s = s
        .chars()
        .map(|c| if c == '.' || c == '_' { ' ' } else { c })
        .collect();
    for r in noise() {
        s = r.replace_all(&s, " ").into_owned();
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cap(re: &Regex, hay: &str, idx: usize) -> Option<u32> {
    re.captures(hay)?.get(idx)?.as_str().parse().ok()
}

/// Parse a file *stem* (no extension). `parent` is consulted only for a
/// `Season 3`-style directory when the filename itself has no season.
pub fn parse_episode(stem: &str, parent_dir: Option<&str>) -> ParsedEpisode {
    let s = strip_noise(stem);

    // The attempts are ordered from least to most ambiguous. The first one that
    // matches wins, so a filename carrying `S01E05` is never reinterpreted by a
    // later, looser rule.
    // No trailing `\b` on the episode digits: a double-episode file is named
    // `S01E05E06`, where the digits are followed by a word character and the
    // anchor would reject the whole match. Greedy `\d{1,4}` will not stop in
    // the middle of a number, which is all the anchor was buying.
    let s01e05 = re(r"(?i)\bs\s*(\d{1,3})\s*[-_. ]?\s*e\s*(\d{1,4})");
    if let Some(c) = s01e05.captures(&s) {
        return ParsedEpisode {
            season: c.get(1).and_then(|m| m.as_str().parse().ok()),
            number: c.get(2).and_then(|m| m.as_str().parse().ok()),
        };
    }

    let x_form = re(r"(?i)\b(\d{1,2})\s*x\s*(\d{1,4})");
    if let Some(c) = x_form.captures(&s) {
        return ParsedEpisode {
            season: c.get(1).and_then(|m| m.as_str().parse().ok()),
            number: c.get(2).and_then(|m| m.as_str().parse().ok()),
        };
    }

    // A season may live in the directory name even when the file has none.
    let season_from_dir = parent_dir.and_then(|d| {
        let d = strip_noise(d);
        cap(
            &re(r"(?i)\b(?:season|сезон|saison|staffel)\s*\.?\s*(\d{1,3})\b"),
            &d,
            1,
        )
        .or_else(|| cap(&re(r"(?i)\bs\s*(\d{1,3})\b"), &d, 1))
    });
    let season_from_name = cap(
        &re(r"(?i)\b(?:season|сезон|saison|staffel)\s*\.?\s*(\d{1,3})\b"),
        &s,
        1,
    );
    let season = season_from_name.or(season_from_dir);

    let attempts = [
        // " - 05 - " used as a delimiter around the number
        re(r"[-–—]\s*(\d{1,3})\s*[-–—]"),
        // Ep 05 / Episode 05 / E05 / Серия 05 / Эпизод 05
        re(r"(?i)\b(?:episode|episodio|folge|ep|e)\s*\.?\s*(\d{1,4})"),
        re(r"(?i)\b(?:серия|эпизод)\s*\.?\s*(\d{1,4})\b"),
        re(r"(?i)\b(\d{1,4})\s*(?:серия|эпизод)\b"),
        // [05]
        re(r"\[\s*(\d{1,4})\s*\]"),
        // a lone number, the last resort
        re(r"\b(\d{1,3})\b"),
    ];
    for r in &attempts {
        if let Some(n) = cap(r, &s, 1) {
            return ParsedEpisode {
                season,
                number: Some(n),
            };
        }
    }

    // "Pilot" / "Пилот" carries no number but is the first episode; the rest of
    // the files are usually named Ep 2, Ep 3, ... so treating it as episode 1
    // makes the show start in the right place.
    if re(r"(?i)\b(?:pilot|pilote|пилот)\b").is_match(&s) {
        return ParsedEpisode {
            season,
            number: Some(1),
        };
    }

    ParsedEpisode {
        season,
        ..ParsedEpisode::nothing()
    }
}

/// A key that sorts episodes correctly under SQLite's plain `ORDER BY` on TEXT.
///
/// Parsed episodes sort ahead of unparsed ones (prefix `0` vs `1`), so a folder
/// where only some names could be read still plays in a sane order instead of
/// interleaving the two groups.
pub fn order_key(parsed: &ParsedEpisode, path: &Path) -> String {
    match parsed.number {
        Some(n) => format!("0/{:04}/{:04}", parsed.season.unwrap_or(1), n),
        None => format!("1/{}", natural_key(path)),
    }
}

/// Lowercased path with every run of digits zero-padded, so `ep2` < `ep10`.
fn natural_key(path: &Path) -> String {
    let s = path.to_string_lossy().to_lowercase();
    let mut out = String::with_capacity(s.len() + 16);
    let mut digits = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            if !digits.is_empty() {
                out.push_str(&format!("{:0>10}", digits));
                digits.clear();
            }
            out.push(c);
        }
    }
    if !digits.is_empty() {
        out.push_str(&format!("{:0>10}", digits));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(stem: &str) -> Expected {
        let e = parse_episode(stem, None);
        (e.season, e.number)
    }

    /// `(season, episode)`, either of which may be unknown.
    type Expected = (Option<u32>, Option<u32>);

    #[test]
    fn parses_a_table_of_real_world_names() {
        let cases: &[(&str, Expected)] = &[
            // explicit season+episode
            (
                "Show.Name.S01E05.1080p.WEB-DL.x264-GROUP",
                (Some(1), Some(5)),
            ),
            ("Show Name - S01E05 - The Title", (Some(1), Some(5))),
            ("show name s1e5", (Some(1), Some(5))),
            ("Show.Name.S01.E05", (Some(1), Some(5))),
            ("Show Name S01E05E06", (Some(1), Some(5))),
            ("Show (2019) S02E10 2160p HDR", (Some(2), Some(10))),
            ("Show.2019.S02E10.HEVC.10bit", (Some(2), Some(10))),
            ("Show.Name.S10E100", (Some(10), Some(100))),
            // 1x05
            ("Show Name - 1x05 - The Title", (Some(1), Some(5))),
            ("Show Name 12x03", (Some(12), Some(3))),
            // delimited number
            ("Show Name - 05 - The Title", (None, Some(5))),
            ("Show Name — 12 — Title", (None, Some(12))),
            // Ep forms
            ("Show Name Ep 05", (None, Some(5))),
            ("Show Name Ep.05", (None, Some(5))),
            ("Show Name Episode 5", (None, Some(5))),
            ("Show Name E05", (None, Some(5))),
            // bracketed
            ("[Group] Show Name [05] [1080p]", (None, Some(5))),
            ("[Group] Show Name [07][BDRip][x265]", (None, Some(7))),
            // Russian
            ("Сериал Серия 07", (None, Some(7))),
            ("Сериал 07 серия", (None, Some(7))),
            ("Сериал Сезон 2 Серия 07", (Some(2), Some(7))),
            // bare
            ("05", (None, Some(5))),
            ("Episode 7", (None, Some(7))),
            // pilot without a number
            ("THE AMAZING DIGITAL CIRCUS: PILOT", (None, Some(1))),
            ("Сериал Пилот", (None, Some(1))),
            // noise must not be mistaken for a number
            ("Show Name 1080p", (None, None)),
            ("Show Name 720p x265 HEVC", (None, None)),
            ("Show Name 2019", (None, None)),
            ("Show Name BluRay REMUX DTS-HD", (None, None)),
            ("Show Name 5.1 AAC", (None, None)),
            ("Show Name 10bit", (None, None)),
            ("Movie Name", (None, None)),
        ];
        for (stem, want) in cases {
            assert_eq!(p(stem), *want, "parsing {stem:?}");
        }
    }

    #[test]
    fn takes_the_season_from_the_directory_when_the_file_omits_it() {
        let e = parse_episode("Episode 7", Some("Season 3"));
        assert_eq!((e.season, e.number), (Some(3), Some(7)));

        let e = parse_episode("07 - Title", Some("Сезон 2"));
        assert_eq!((e.season, e.number), (Some(2), Some(7)));
    }

    #[test]
    fn order_keys_sort_episodes_in_playback_order() {
        let mut keys: Vec<String> = [
            ("S01E02", "/x/b.mkv"),
            ("S01E10", "/x/c.mkv"),
            ("S01E01", "/x/a.mkv"),
            ("S02E01", "/x/d.mkv"),
        ]
        .iter()
        .map(|(stem, path)| order_key(&parse_episode(stem, None), Path::new(path)))
        .collect();
        keys.sort();
        assert_eq!(
            keys,
            vec!["0/0001/0001", "0/0001/0002", "0/0001/0010", "0/0002/0001"]
        );
    }

    #[test]
    fn unparsed_names_sort_after_parsed_ones_and_naturally_among_themselves() {
        let parsed = order_key(&parse_episode("S01E01", None), Path::new("/x/a.mkv"));
        let bare2 = order_key(&ParsedEpisode::nothing(), Path::new("/x/clip2.mkv"));
        let bare10 = order_key(&ParsedEpisode::nothing(), Path::new("/x/clip10.mkv"));

        assert!(parsed < bare2, "{parsed} should precede {bare2}");
        assert!(bare2 < bare10, "natural order: {bare2} before {bare10}");
    }

    #[test]
    fn pilot_sorts_before_the_rest_of_the_episodes() {
        let pilot = order_key(
            &parse_episode("THE AMAZING DIGITAL CIRCUS: PILOT", None),
            Path::new("/x/pilot.webm"),
        );
        let ep2 = order_key(
            &parse_episode("THE AMAZING DIGITAL CIRCUS - Ep 2: Title", None),
            Path::new("/x/ep2.webm"),
        );

        assert_eq!(pilot, "0/0001/0001");
        assert_eq!(ep2, "0/0001/0002");
        assert!(pilot < ep2, "pilot should play before Ep 2");
    }
}
