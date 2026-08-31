//! Plugging the leaks that are not the `PlaybackView` projection.
//!
//! The projection in [`super::PlaybackView`] covers everything Murk *sends*.
//! This module covers everything the operating system might say on Murk's
//! behalf: the window title, a media-control applet, a "recently opened" list.

use regex::Regex;
use std::sync::OnceLock;

/// The window title, at all times, on every platform. Never derived from the
/// file being played.
pub const WINDOW_TITLE: &str = "Murk";

/// Patterns that turn an innocuous track title into a spoiler.
///
/// Track titles are usually harmless ("English", "Русский дубляж", "Commentary"),
/// but muxers happily copy the episode name into them: an ASS subtitle track
/// called `S03E09 - The Rains of Castamere` announces the episode in the track
/// menu. Cut the identifying parts out and keep the rest.
fn spoiler_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            // S03E09, s3e9, S03 E09
            r"(?i)\bs\s*\d{1,3}\s*[\-_. ]?\s*e\s*\d{1,4}\b",
            // 3x09
            r"(?i)\b\d{1,3}\s*x\s*\d{1,4}\b",
            // Episode 9 / Ep. 9 / E09 / Серия 9 / Сезон 3
            r"(?i)\b(?:episode|episodio|folge|ep)\s*\.?\s*\d{1,4}\b",
            r"(?i)\b(?:season|сезон|saison|staffel)\s*\.?\s*\d{1,3}\b",
            r"(?i)\b(?:серия|серии|эпизод)\s*\.?\s*\d{1,4}\b",
            r"(?i)\b\d{1,4}\s*(?:серия|эпизод)\b",
            // [09] and #09 used as an episode marker
            r"\[\s*\d{1,4}\s*\]",
            r"#\s*\d{1,4}\b",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    })
}

/// Remove episode/season markers from a track title.
///
/// Returns `None` when nothing meaningful survives; the caller then falls back
/// to a positional label ("Дорожка 2").
pub fn sanitize_track_title(raw: &str) -> Option<String> {
    let mut cleaned = raw.to_string();
    for re in spoiler_patterns() {
        cleaned = re.replace_all(&cleaned, " ").into_owned();
    }

    // Collapse the punctuation left dangling where a marker used to be.
    let cleaned: String = cleaned
        .chars()
        .map(|c| if c == '_' { ' ' } else { c })
        .collect();
    let cleaned = cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| c.is_whitespace() || "-–—:.,|/\\[](){}".contains(c))
        .to_string();

    // A title that was *only* an episode marker is now empty or a bare number.
    if cleaned.is_empty() || cleaned.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(cleaned)
}

/// Vocabulary that a track title may consist of without saying anything about
/// the story: language names, the role of the track, and channel layouts.
fn descriptor_vocabulary() -> &'static std::collections::HashSet<&'static str> {
    static VOCAB: OnceLock<std::collections::HashSet<&'static str>> = OnceLock::new();
    VOCAB.get_or_init(|| {
        [
            // languages, as spelled in track titles in practice
            "english",
            "russian",
            "japanese",
            "french",
            "german",
            "spanish",
            "italian",
            "portuguese",
            "polish",
            "ukrainian",
            "chinese",
            "korean",
            "turkish",
            "arabic",
            "hindi",
            "czech",
            "dutch",
            "swedish",
            "danish",
            "norwegian",
            "finnish",
            "hungarian",
            "greek",
            "hebrew",
            "thai",
            "vietnamese",
            "romanian",
            "bulgarian",
            "serbian",
            "croatian",
            "eng",
            "rus",
            "jpn",
            "fre",
            "fra",
            "ger",
            "deu",
            "spa",
            "ita",
            "ukr",
            "русский",
            "английский",
            "японский",
            "французский",
            "немецкий",
            "испанский",
            "итальянский",
            "украинский",
            "польский",
            // the role of the track
            "forced",
            "sdh",
            "cc",
            "commentary",
            "director",
            "directors",
            "dub",
            "dubbed",
            "dubbing",
            "original",
            "signs",
            "songs",
            "full",
            "partial",
            "sub",
            "subs",
            "subtitle",
            "subtitles",
            "audio",
            "track",
            "descriptive",
            "narration",
            "karaoke",
            "default",
            "alternate",
            "дубляж",
            "дублированный",
            "субтитры",
            "форсированные",
            "полные",
            "комментарии",
            "закадровый",
            "многоголосый",
            "одноголосый",
            "авторский",
            "оригинал",
            "оригинальная",
            "перевод",
            "дорожка",
            // codecs and channel layouts
            "stereo",
            "mono",
            "surround",
            "atmos",
            "aac",
            "ac3",
            "eac3",
            "dts",
            "truehd",
            "flac",
            "opus",
            "mp3",
            "pcm",
            "hd",
            "ma",
            "and",
        ]
        .into_iter()
        .collect()
    })
}

/// Is this track title made of nothing but descriptors?
///
/// A track title carries no episode *marker* and still be the loudest spoiler
/// on the screen: muxers routinely name the subtitle track after the episode,
/// so `The Rains of Castamere` arrives with nothing for
/// [`sanitize_track_title`] to cut. When the profile hides the episode title,
/// free text in a track name has to go too, or the track menu quietly
/// reintroduces exactly what the profile removed.
pub fn is_descriptor_only(title: &str) -> bool {
    let words: Vec<String> = title
        .split(|c: char| !(c.is_alphanumeric() || c == '.' || c == '\''))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();

    if words.is_empty() {
        return false;
    }
    let vocab = descriptor_vocabulary();
    words.iter().all(|w| {
        // numbers and channel layouts ("2", "5.1") say nothing about the plot
        w.chars().all(|c| c.is_ascii_digit() || c == '.') || vocab.contains(w.as_str())
    })
}

// Recording what the user opens is handled where the toolkit lives, in
// `player::surface_gtk::harden_recent_files`: GTK's recent-files manager would
// otherwise put every folder the picker touches into `recently-used.xbel`,
// where the desktop's launcher will display the series name back at the user.
// This module deliberately keeps no toolkit dependency so that the privacy
// rules can be tested on their own.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_episode_markers_from_track_titles() {
        let cases = [
            (
                "S03E09 - The Rains of Castamere",
                Some("The Rains of Castamere"),
            ),
            ("English", Some("English")),
            ("Русский дубляж", Some("Русский дубляж")),
            ("Commentary 5.1", Some("Commentary 5.1")),
            ("3x09", None),
            ("[09]", None),
            ("Episode 9", None),
            ("Серия 12 — Развязка", Some("Развязка")),
            ("Forced Subs (S01E01)", Some("Forced Subs")),
        ];
        for (raw, want) in cases {
            let got = sanitize_track_title(raw);
            assert_eq!(got.as_deref(), want, "sanitising {raw:?}");
        }
    }

    #[test]
    fn free_text_track_titles_are_not_descriptors() {
        for safe in [
            "English",
            "Русский дубляж",
            "Commentary 5.1",
            "Forced Subs",
            "eng",
            "SDH",
            "Original",
            "Dutch (Stereo)",
            "Audio 2",
        ] {
            assert!(
                is_descriptor_only(safe),
                "{safe:?} should count as a descriptor"
            );
        }
        for spoiler in [
            "The Rains of Castamere",
            "The Bear and the Maiden Fair",
            "Ned's Execution",
            "Финал сезона",
            "",
        ] {
            assert!(
                !is_descriptor_only(spoiler),
                "{spoiler:?} must not pass as a descriptor"
            );
        }
    }

    #[test]
    fn window_title_never_mentions_content() {
        assert_eq!(WINDOW_TITLE, "Murk");
    }
}
