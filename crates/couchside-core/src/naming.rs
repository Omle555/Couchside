//! File and folder name parsing for movies and TV episodes.
//!
//! These functions are pure (no I/O) so they can be tested exhaustively.
//! Supported layouts follow the common Jellyfin/Plex/Kodi conventions:
//!
//! * Movies: `Movie Title (Year)/Movie Title (Year).ext`, `Movie.Title.Year.1080p.ext`
//! * TV: `Show Name (Year)/Season 01/Show Name - S01E02 - Episode Title.ext`,
//!   `S01E02E03`, `S01E02-E03`, `1x02`

use std::path::Path;

/// File extensions treated as playable video.
pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "wmv", "webm", "ts", "m2ts", "mpg", "mpeg", "flv", "ogv",
];

/// File extensions treated as external subtitles.
pub const SUBTITLE_EXTENSIONS: &[&str] = &["srt", "vtt", "ass", "ssa"];

/// Words that mark the start of release junk in a file name.
const JUNK_TOKENS: &[&str] = &[
    "2160p",
    "1080p",
    "1080i",
    "720p",
    "576p",
    "480p",
    "4k",
    "uhd",
    "hdr",
    "hdr10",
    "dv",
    "bluray",
    "blu-ray",
    "bdrip",
    "brrip",
    "remux",
    "web",
    "web-dl",
    "webdl",
    "webrip",
    "hdtv",
    "dvdrip",
    "dvd",
    "x264",
    "x265",
    "h264",
    "h265",
    "hevc",
    "avc",
    "xvid",
    "aac",
    "ac3",
    "dts",
    "atmos",
    "proper",
    "repack",
    "extended",
    "unrated",
    "remastered",
    "imax",
    "multi",
];

/// Online database ids written into a file or folder name, such as
/// `{tmdb-603}`, `[tmdbid=603]` or `{imdb-tt0133093}` (Jellyfin/Plex style).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderIds {
    /// The Movie Database id.
    pub tmdb: Option<i64>,
    /// IMDb id, e.g. `tt0133093`.
    pub imdb: Option<String>,
}

impl ProviderIds {
    /// Fills missing ids from `other`.
    fn or(self, other: ProviderIds) -> ProviderIds {
        ProviderIds {
            tmdb: self.tmdb.or(other.tmdb),
            imdb: self.imdb.or(other.imdb),
        }
    }
}

/// A movie title and year parsed from a file or folder name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMovie {
    /// Human readable title.
    pub title: String,
    /// Release year, if present in the name.
    pub year: Option<i32>,
    /// Database ids found in the name.
    pub ids: ProviderIds,
}

/// Season/episode numbers parsed from an episode file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEpisode {
    /// Season number (0 = specials).
    pub season: i32,
    /// First episode number in the file.
    pub episode: i32,
    /// Last episode number for multi-episode files (`S01E01E02`).
    pub episode_end: Option<i32>,
    /// Episode title following the `SxxEyy` marker, if any.
    pub title: Option<String>,
}

fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// Returns true if the path has a known video extension.
pub fn is_video_file(path: &Path) -> bool {
    extension_lower(path).is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.as_str()))
}

/// Returns true if the path has a known subtitle extension.
pub fn is_subtitle_file(path: &Path) -> bool {
    extension_lower(path).is_some_and(|e| SUBTITLE_EXTENSIONS.contains(&e.as_str()))
}

/// Returns true for files and folders the scanner should skip entirely:
/// hidden/system folders, samples, trailers and extras.
pub fn is_ignored_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with('.') || lower.starts_with('$') {
        return true;
    }
    if matches!(
        lower.as_str(),
        "system volume information"
            | "@eadir"
            | "extras"
            | "featurettes"
            | "behind the scenes"
            | "deleted scenes"
            | "trailers"
            | "samples"
            | "sample"
    ) {
        return true;
    }
    let stem = lower.rsplit_once('.').map_or(lower.as_str(), |(s, _)| s);
    stem.ends_with("-sample")
        || stem.ends_with(".sample")
        || stem.ends_with(" sample")
        || stem.ends_with("-trailer")
        || stem.ends_with(".trailer")
        || stem.ends_with(" trailer")
}

/// Key used for alphabetical sorting: lowercase without a leading article.
pub fn sort_title(title: &str) -> String {
    let lower = title.trim().to_lowercase();
    for article in ["the ", "a ", "an "] {
        if let Some(rest) = lower.strip_prefix(article) {
            if !rest.is_empty() {
                return rest.trim_start().to_string();
            }
        }
    }
    lower
}

/// Parses a movie from its file stem, falling back to the parent folder name
/// when the file name carries no year (e.g. `Movie (2010)/movie.mkv`).
/// Ids tagged on either the file or its folder are used.
pub fn parse_movie(file_stem: &str, parent_dir: Option<&str>) -> ParsedMovie {
    let from_file = clean_title_and_year(file_stem);
    let Some(from_parent) = parent_dir.map(clean_title_and_year) else {
        return from_file;
    };
    let ids = from_file.ids.clone().or(from_parent.ids.clone());
    let chosen = if from_file.year.is_none() && from_parent.year.is_some() {
        from_parent
    } else {
        from_file
    };
    ParsedMovie { ids, ..chosen }
}

/// Removes provider id tags (`{tmdb-603}`, `[imdbid=tt0133093]`...) from a name
/// and returns the cleaned name together with the ids found. Other bracketed
/// text is left untouched.
pub fn extract_provider_ids(name: &str) -> (String, ProviderIds) {
    let mut ids = ProviderIds::default();
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(open_idx) = rest.find(['[', '{']) {
        let open = if rest[open_idx..].starts_with('[') {
            '['
        } else {
            '{'
        };
        let close = if open == '[' { ']' } else { '}' };
        out.push_str(&rest[..open_idx]);
        let after = &rest[open_idx + 1..];
        match after.find(close) {
            Some(close_idx) => {
                let inner = &after[..close_idx];
                if !parse_id_tag(inner, &mut ids) {
                    out.push(open);
                    out.push_str(inner);
                    out.push(close);
                }
                rest = &after[close_idx + 1..];
            }
            None => {
                out.push(open);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    (out, ids)
}

/// Recognises `tmdb-603`, `tmdbid=603`, `imdb-tt0133093`, `imdbid=tt0133093`.
fn parse_id_tag(inner: &str, ids: &mut ProviderIds) -> bool {
    let lower = inner.trim().to_ascii_lowercase();
    let Some((key, value)) = lower.split_once(['=', '-', ':', ' ']) else {
        return false;
    };
    let value = value.trim();
    match key.trim() {
        "tmdb" | "tmdbid" => match value.parse::<i64>() {
            Ok(id) if id > 0 => {
                ids.tmdb = Some(id);
                true
            }
            _ => false,
        },
        "imdb" | "imdbid" => {
            let valid = value.len() > 2
                && value.starts_with("tt")
                && value[2..].chars().all(|c| c.is_ascii_digit());
            if valid {
                ids.imdb = Some(value.to_string());
            }
            valid
        }
        _ => false,
    }
}

/// Parses a show folder name such as `The Office (US) (2005)`.
pub fn parse_show_folder(name: &str) -> ParsedMovie {
    clean_title_and_year(name)
}

/// Parses a season folder name such as `Season 01`, `S2` or `Specials`.
pub fn parse_season_folder(name: &str) -> Option<i32> {
    let lower = name.trim().to_ascii_lowercase();
    if lower == "specials" || lower == "extras" {
        return Some(0);
    }
    let digits = lower
        .strip_prefix("season")
        .or_else(|| lower.strip_prefix('s'))?
        .trim_start_matches([' ', '_', '.', '-']);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Parses `SxxEyy`, `SxxEyyEzz`, `SxxEyy-Ezz` or `NxNN` markers from an episode file stem.
pub fn parse_episode(file_stem: &str) -> Option<ParsedEpisode> {
    let chars: Vec<char> = file_stem.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_ascii_lowercase()).collect();

    for start in 0..lower.len() {
        if start > 0 && lower[start - 1].is_alphanumeric() {
            continue;
        }
        if let Some((season, episode, end_idx)) =
            match_sxxeyy(&lower, start).or_else(|| match_nxnn(&lower, start))
        {
            let (episode_end, after) = match_episode_end(&lower, end_idx, episode);
            let rest: String = chars[after..].iter().collect();
            return Some(ParsedEpisode {
                season,
                episode,
                episode_end,
                title: clean_episode_title(&rest),
            });
        }
    }
    None
}

/// Reads up to `max` ASCII digits starting at `idx`. Returns (value, next index).
fn read_digits(s: &[char], idx: usize, max: usize) -> Option<(i32, usize)> {
    let mut end = idx;
    while end < s.len() && end - idx < max && s[end].is_ascii_digit() {
        end += 1;
    }
    if end == idx {
        return None;
    }
    let text: String = s[idx..end].iter().collect();
    text.parse().ok().map(|v| (v, end))
}

fn match_sxxeyy(s: &[char], start: usize) -> Option<(i32, i32, usize)> {
    if s.get(start) != Some(&'s') {
        return None;
    }
    let (season, idx) = read_digits(s, start + 1, 2)?;
    // Allow an optional separator: "S01E02", "S01.E02", "S01 E02".
    let idx = if matches!(s.get(idx), Some('.' | ' ' | '_')) && s.get(idx + 1) == Some(&'e') {
        idx + 1
    } else {
        idx
    };
    if s.get(idx) != Some(&'e') {
        return None;
    }
    let (episode, end) = read_digits(s, idx + 1, 3)?;
    if s.get(end).is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    Some((season, episode, end))
}

fn match_nxnn(s: &[char], start: usize) -> Option<(i32, i32, usize)> {
    let (season, idx) = read_digits(s, start, 2)?;
    if s.get(idx) != Some(&'x') {
        return None;
    }
    let (episode, end) = read_digits(s, idx + 1, 3)?;
    if end - (idx + 1) < 2 {
        return None;
    }
    if s.get(end).is_some_and(|c| c.is_alphanumeric()) {
        return None;
    }
    Some((season, episode, end))
}

/// Detects a multi-episode suffix (`E03`, `-E03`, `-03`) after the first episode.
fn match_episode_end(s: &[char], idx: usize, first: i32) -> (Option<i32>, usize) {
    let candidates: [&[char]; 3] = [&['e'], &['-', 'e'], &['-']];
    for prefix in candidates {
        let fits = prefix
            .iter()
            .enumerate()
            .all(|(i, c)| s.get(idx + i) == Some(c));
        if !fits {
            continue;
        }
        let digits_at = idx + prefix.len();
        if let Some((value, end)) = read_digits(s, digits_at, 3) {
            let bounded = !s.get(end).is_some_and(|c| c.is_alphanumeric());
            if bounded && value > first {
                return (Some(value), end);
            }
        }
    }
    (None, idx)
}

fn clean_episode_title(rest: &str) -> Option<String> {
    let normalized = normalize_separators(rest);
    let cut = cut_at_junk(&normalized);
    let title = trim_separators(&cut);
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// Replaces `_` with spaces, and `.` with spaces when the name has no real spaces
/// (scene-style `The.Movie.2010.1080p`).
fn normalize_separators(s: &str) -> String {
    let s = s.replace('_', " ");
    if s.contains(' ') {
        s
    } else {
        s.replace('.', " ")
    }
}

/// Trims separators and dangling brackets, keeping balanced ones like `(US)`.
fn trim_separators(s: &str) -> String {
    let is_sep = |c: char| c.is_whitespace() || matches!(c, '.' | '-' | '_' | ',');
    let mut t = s.trim_matches(is_sep);
    loop {
        let before = t.len();
        t = t.trim_end_matches(|c: char| is_sep(c) || matches!(c, '(' | '['));
        for (open, close) in [('(', ')'), ('[', ']')] {
            if t.ends_with(close) && t.matches(open).count() < t.matches(close).count() {
                t = &t[..t.len() - 1];
            }
        }
        if t.len() == before {
            break;
        }
    }
    let t = t.trim_start_matches(|c: char| is_sep(c) || matches!(c, '(' | '[' | ')' | ']'));
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Strips a leading `[Group]` tag.
fn strip_leading_tag(s: &str) -> &str {
    let t = s.trim_start();
    if let Some(rest) = t.strip_prefix('[') {
        if let Some(close) = rest.find(']') {
            return &rest[close + 1..];
        }
    }
    t
}

/// Cuts the string at the first release-junk word (resolution, codec, source...).
fn cut_at_junk(s: &str) -> String {
    let mut out = Vec::new();
    for word in s.split(' ') {
        let bare = word
            .trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | '.' | ','))
            .to_ascii_lowercase();
        if JUNK_TOKENS.contains(&bare.as_str()) {
            break;
        }
        out.push(word);
    }
    out.join(" ")
}

/// Finds plausible years (1900-2099) bounded by non-alphanumeric characters.
fn find_years(s: &str) -> Vec<(usize, i32)> {
    let bytes = s.as_bytes();
    let mut found = Vec::new();
    if bytes.len() < 4 {
        return found;
    }
    for i in 0..=bytes.len() - 4 {
        let window = &bytes[i..i + 4];
        if !window.iter().all(u8::is_ascii_digit) {
            continue;
        }
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after_ok = i + 4 == bytes.len() || !bytes[i + 4].is_ascii_alphanumeric();
        if !before_ok || !after_ok {
            continue;
        }
        let Ok(text) = std::str::from_utf8(window) else {
            continue;
        };
        if let Ok(year) = text.parse::<i32>() {
            if (1900..=2099).contains(&year) {
                found.push((i, year));
            }
        }
    }
    found
}

/// Splits a raw name into a clean title and optional year.
pub fn clean_title_and_year(raw: &str) -> ParsedMovie {
    let (without_ids, ids) = extract_provider_ids(raw);
    let normalized = normalize_separators(strip_leading_tag(&without_ids));

    // Use the last year that has a non-empty title before it, so that
    // "Blade Runner 2049 (2017)" -> ("Blade Runner 2049", 2017)
    // and "1917" -> ("1917", None).
    for (pos, year) in find_years(&normalized).into_iter().rev() {
        let title = trim_separators(&cut_at_junk(&normalized[..pos]));
        if !title.is_empty() {
            return ParsedMovie {
                title,
                year: Some(year),
                ids,
            };
        }
    }

    let title = trim_separators(&cut_at_junk(&normalized));
    ParsedMovie {
        title: if title.is_empty() {
            trim_separators(&without_ids)
        } else {
            title
        },
        year: None,
        ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(title: &str, year: Option<i32>) -> ParsedMovie {
        ParsedMovie {
            title: title.to_string(),
            year,
            ids: ProviderIds::default(),
        }
    }

    #[test]
    fn provider_ids_in_file_name() {
        let parsed = parse_movie("The Matrix (1999) {tmdb-603}", None);
        assert_eq!(parsed.title, "The Matrix");
        assert_eq!(parsed.year, Some(1999));
        assert_eq!(parsed.ids.tmdb, Some(603));

        let parsed = parse_movie("Heat (1995) [imdbid-tt0113277]", None);
        assert_eq!(parsed.title, "Heat");
        assert_eq!(parsed.ids.imdb.as_deref(), Some("tt0113277"));

        let parsed = parse_movie("Alien [tmdbid=348] (1979)", None);
        assert_eq!((parsed.title.as_str(), parsed.year), ("Alien", Some(1979)));
        assert_eq!(parsed.ids.tmdb, Some(348));
    }

    #[test]
    fn provider_ids_from_folder() {
        let parsed = parse_movie("movie", Some("Inception (2010) [tmdbid-27205]"));
        assert_eq!(parsed.title, "Inception");
        assert_eq!(parsed.ids.tmdb, Some(27205));

        let parsed = parse_movie("Inception (2010)", Some("Inception {imdb-tt1375666}"));
        assert_eq!(parsed.year, Some(2010));
        assert_eq!(parsed.ids.imdb.as_deref(), Some("tt1375666"));
    }

    #[test]
    fn other_brackets_are_not_ids() {
        let (clean, ids) = extract_provider_ids("Shameless [US] {Extended} [tmdb-x] [imdb-nm1]");
        assert_eq!(clean, "Shameless [US] {Extended} [tmdb-x] [imdb-nm1]");
        assert_eq!(ids, ProviderIds::default());
        let (clean, _) = extract_provider_ids("Broken [tmdb-12");
        assert_eq!(clean, "Broken [tmdb-12");
    }

    #[test]
    fn movie_standard_layout() {
        assert_eq!(
            parse_movie("The Matrix (1999)", None),
            movie("The Matrix", Some(1999))
        );
    }

    #[test]
    fn movie_scene_name() {
        assert_eq!(
            parse_movie("The.Matrix.1999.1080p.BluRay.x264-GROUP", None),
            movie("The Matrix", Some(1999))
        );
    }

    #[test]
    fn movie_with_number_in_title() {
        assert_eq!(
            parse_movie("Blade Runner 2049 (2017)", None),
            movie("Blade Runner 2049", Some(2017))
        );
        assert_eq!(
            parse_movie("Blade.Runner.2049.2017.2160p.UHD", None),
            movie("Blade Runner 2049", Some(2017))
        );
        assert_eq!(
            parse_movie("2001 A Space Odyssey (1968)", None),
            movie("2001 A Space Odyssey", Some(1968))
        );
    }

    #[test]
    fn movie_title_is_only_a_year() {
        assert_eq!(parse_movie("1917 (2019)", None), movie("1917", Some(2019)));
        assert_eq!(parse_movie("1917", None), movie("1917", None));
    }

    #[test]
    fn movie_falls_back_to_parent_folder() {
        assert_eq!(
            parse_movie("movie", Some("Inception (2010)")),
            movie("Inception", Some(2010))
        );
    }

    #[test]
    fn movie_without_year_cuts_junk() {
        assert_eq!(
            parse_movie("Some Film 1080p WEB-DL", None),
            movie("Some Film", None)
        );
    }

    #[test]
    fn movie_keeps_dots_in_spaced_titles() {
        assert_eq!(
            parse_movie("Mr. Smith Goes to Washington (1939)", None),
            movie("Mr. Smith Goes to Washington", Some(1939))
        );
    }

    #[test]
    fn movie_group_tag_and_underscores() {
        assert_eq!(
            parse_movie("[GRP] Spirited_Away_(2001)", None),
            movie("Spirited Away", Some(2001))
        );
    }

    #[test]
    fn movie_ignores_year_glued_to_group() {
        assert_eq!(
            parse_movie("Heat.1995.720p.x264-GRP2020", None),
            movie("Heat", Some(1995))
        );
    }

    #[test]
    fn movie_unicode_title() {
        assert_eq!(
            parse_movie("Amélie (2001)", None),
            movie("Amélie", Some(2001))
        );
        assert_eq!(
            parse_movie("千と千尋の神隠し (2001)", None),
            movie("千と千尋の神隠し", Some(2001))
        );
    }

    #[test]
    fn episode_standard() {
        let ep = parse_episode("Breaking Bad - S01E02 - Cat's in the Bag").expect("parses");
        assert_eq!(ep.season, 1);
        assert_eq!(ep.episode, 2);
        assert_eq!(ep.episode_end, None);
        assert_eq!(ep.title.as_deref(), Some("Cat's in the Bag"));
    }

    #[test]
    fn episode_scene_style() {
        let ep = parse_episode("The.Office.US.S05E14.720p.HDTV.x264").expect("parses");
        assert_eq!((ep.season, ep.episode), (5, 14));
        assert_eq!(ep.title, None);
    }

    #[test]
    fn episode_lowercase_and_single_digits() {
        let ep = parse_episode("show s1e3").expect("parses");
        assert_eq!((ep.season, ep.episode), (1, 3));
    }

    #[test]
    fn episode_multi() {
        let ep = parse_episode("Show - S02E01E02 - Two Parter").expect("parses");
        assert_eq!((ep.season, ep.episode, ep.episode_end), (2, 1, Some(2)));
        assert_eq!(ep.title.as_deref(), Some("Two Parter"));

        let ep = parse_episode("Show S02E01-E03").expect("parses");
        assert_eq!(ep.episode_end, Some(3));

        let ep = parse_episode("Show S02E01-02").expect("parses");
        assert_eq!(ep.episode_end, Some(2));
    }

    #[test]
    fn episode_nxnn() {
        let ep = parse_episode("Futurama 3x07 Title").expect("parses");
        assert_eq!((ep.season, ep.episode), (3, 7));
        assert_eq!(ep.title.as_deref(), Some("Title"));
    }

    #[test]
    fn episode_three_digit() {
        let ep = parse_episode("One Piece S01E123").expect("parses");
        assert_eq!((ep.season, ep.episode), (1, 123));
    }

    #[test]
    fn episode_rejects_non_episodes() {
        assert_eq!(parse_episode("The Matrix (1999)"), None);
        assert_eq!(parse_episode("Video 1920x1080"), None);
        assert_eq!(parse_episode("Mass Effect"), None);
        assert_eq!(parse_episode("Classes"), None);
    }

    #[test]
    fn season_folders() {
        assert_eq!(parse_season_folder("Season 01"), Some(1));
        assert_eq!(parse_season_folder("season 12"), Some(12));
        assert_eq!(parse_season_folder("S3"), Some(3));
        assert_eq!(parse_season_folder("Specials"), Some(0));
        assert_eq!(parse_season_folder("Seasonal"), None);
        assert_eq!(parse_season_folder("Extras Disc"), None);
    }

    #[test]
    fn show_folder() {
        assert_eq!(
            parse_show_folder("The Office (US) (2005)"),
            movie("The Office (US)", Some(2005))
        );
        assert_eq!(parse_show_folder("Firefly"), movie("Firefly", None));
        assert_eq!(
            parse_show_folder("Doctor Who (2005)"),
            movie("Doctor Who", Some(2005))
        );
        assert_eq!(
            parse_show_folder("Shameless (US)"),
            movie("Shameless (US)", None)
        );
    }

    #[test]
    fn ignored_names() {
        assert!(is_ignored_name("$RECYCLE.BIN"));
        assert!(is_ignored_name("System Volume Information"));
        assert!(is_ignored_name(".hidden"));
        assert!(is_ignored_name("movie-sample.mkv"));
        assert!(is_ignored_name("Movie-trailer.mp4"));
        assert!(is_ignored_name("Extras"));
        assert!(!is_ignored_name("Sample Movie (2020).mkv"));
        assert!(!is_ignored_name("The Matrix (1999).mkv"));
    }

    #[test]
    fn extensions() {
        assert!(is_video_file(Path::new("a/b.MKV")));
        assert!(!is_video_file(Path::new("a/b.nfo")));
        assert!(is_subtitle_file(Path::new("a/b.en.srt")));
    }

    #[test]
    fn sort_titles() {
        assert_eq!(sort_title("The Matrix"), "matrix");
        assert_eq!(sort_title("A Quiet Place"), "quiet place");
        assert_eq!(sort_title("Annie"), "annie");
        assert_eq!(sort_title("The"), "the");
    }
}
