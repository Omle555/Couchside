//! Standard file and folder names used by "Clean up" when renaming is switched on:
//! `Movie Title (Year)/Movie Title (Year) 1080p.ext`.
//!
//! These functions are pure so they can be tested exhaustively.

/// Longest file stem Solidfin writes, leaving room for the folder and extension.
const MAX_STEM_CHARS: usize = 180;

/// Names Windows reserves for devices; a file may not be called `CON.mkv`.
const RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// Turns a title into text that is valid in a Windows file name:
/// `Mission: Impossible` → `Mission - Impossible`, `AC/DC` → `AC-DC`, `Why?` → `Why`.
pub fn safe_file_name(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            ':' => out.push_str(" - "),
            '/' | '\\' | '|' => out.push('-'),
            '<' | '>' | '"' | '?' | '*' => {}
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let collapsed = collapsed.replace(" - - ", " - ");
    let mut name: String = collapsed.chars().take(MAX_STEM_CHARS).collect();
    while name.ends_with(['.', ' ']) {
        name.pop();
    }
    if RESERVED.contains(&name.to_ascii_lowercase().as_str()) {
        name.push('_');
    }
    name
}

/// `Movie Title (Year)`, or just the title when the year is unknown.
/// Returns `None` if nothing usable is left of the title.
pub fn movie_folder_name(title: &str, year: Option<i64>) -> Option<String> {
    let title = safe_file_name(title);
    if title.is_empty() {
        return None;
    }
    Some(match year {
        Some(year) => format!("{title} ({year})"),
        None => title,
    })
}

/// `Movie Title (Year) 1080p`: the folder name plus the video resolution, when known.
pub fn movie_file_stem(title: &str, year: Option<i64>, resolution: Option<&str>) -> Option<String> {
    let base = movie_folder_name(title, year)?;
    Some(match resolution {
        Some(res) => format!("{base} {res}"),
        None => base,
    })
}

/// Resolution label for a video size: `2160p`, `1440p`, `1080p`, `720p`, `576p`, `480p`.
///
/// Uses the width as well as the height so cropped widescreen films
/// (1920×800) still count as 1080p.
pub fn resolution_label(width: Option<i64>, height: Option<i64>) -> Option<String> {
    let w = width.filter(|w| *w > 0).unwrap_or(0);
    let h = height.filter(|h| *h > 0).unwrap_or(0);
    if w == 0 && h == 0 {
        return None;
    }
    let label = if w >= 3200 || h >= 1800 {
        "2160p"
    } else if w >= 2300 || h >= 1300 {
        "1440p"
    } else if w >= 1700 || h >= 950 {
        "1080p"
    } else if w >= 1100 || h >= 650 {
        "720p"
    } else if h >= 540 {
        "576p"
    } else if h >= 400 || w >= 640 {
        "480p"
    } else {
        return Some(format!("{h}p"));
    };
    Some(label.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::naming::parse_movie;

    #[test]
    fn invalid_characters() {
        assert_eq!(
            safe_file_name("Mission: Impossible"),
            "Mission - Impossible"
        );
        assert_eq!(safe_file_name("AC/DC: Live"), "AC-DC - Live");
        assert_eq!(safe_file_name("What's Up, Doc?"), "What's Up, Doc");
        assert_eq!(
            safe_file_name("Star Wars: Episode IV - A New Hope"),
            "Star Wars - Episode IV - A New Hope"
        );
        assert_eq!(safe_file_name("  Spaced   out.  "), "Spaced out");
        assert_eq!(safe_file_name("<*>"), "");
        assert_eq!(safe_file_name("Con"), "Con_");
        assert_eq!(safe_file_name("Amélie"), "Amélie");
        assert_eq!(
            safe_file_name("x".repeat(300).as_str()).chars().count(),
            MAX_STEM_CHARS
        );
    }

    #[test]
    fn folder_and_file_names() {
        assert_eq!(
            movie_folder_name("Runner", Some(2026)).as_deref(),
            Some("Runner (2026)")
        );
        assert_eq!(movie_folder_name("Runner", None).as_deref(), Some("Runner"));
        assert_eq!(movie_folder_name("???", Some(2020)), None);
        assert_eq!(
            movie_file_stem("Runner", Some(2026), Some("1080p")).as_deref(),
            Some("Runner (2026) 1080p")
        );
        assert_eq!(
            movie_file_stem("Runner", Some(2026), None).as_deref(),
            Some("Runner (2026)")
        );
    }

    #[test]
    fn resolutions() {
        assert_eq!(
            resolution_label(Some(3840), Some(2160)).as_deref(),
            Some("2160p")
        );
        assert_eq!(
            resolution_label(Some(3840), Some(1600)).as_deref(),
            Some("2160p")
        );
        assert_eq!(
            resolution_label(Some(2560), Some(1440)).as_deref(),
            Some("1440p")
        );
        assert_eq!(
            resolution_label(Some(1920), Some(1080)).as_deref(),
            Some("1080p")
        );
        assert_eq!(
            resolution_label(Some(1920), Some(800)).as_deref(),
            Some("1080p")
        );
        assert_eq!(
            resolution_label(Some(1280), Some(720)).as_deref(),
            Some("720p")
        );
        assert_eq!(
            resolution_label(Some(1280), Some(536)).as_deref(),
            Some("720p")
        );
        assert_eq!(
            resolution_label(Some(720), Some(576)).as_deref(),
            Some("576p")
        );
        assert_eq!(
            resolution_label(Some(720), Some(480)).as_deref(),
            Some("480p")
        );
        assert_eq!(
            resolution_label(Some(320), Some(240)).as_deref(),
            Some("240p")
        );
        assert_eq!(resolution_label(None, None), None);
        assert_eq!(resolution_label(Some(0), Some(0)), None);
    }

    #[test]
    fn new_names_parse_back_to_the_same_movie() {
        for (title, year) in [
            ("Runner", Some(2026)),
            ("Blade Runner 2049", Some(2017)),
            ("1917", Some(2019)),
            ("Mission: Impossible", Some(1996)),
        ] {
            let folder = movie_folder_name(title, year).expect("folder");
            let stem = movie_file_stem(title, year, Some("1080p")).expect("stem");
            let parsed = parse_movie(&stem, Some(&folder));
            assert_eq!(parsed.title, safe_file_name(title), "{stem}");
            assert_eq!(parsed.year.map(i64::from), year, "{stem}");
        }
    }
}
