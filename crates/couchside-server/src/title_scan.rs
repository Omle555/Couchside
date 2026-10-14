//! "Scan video for title" in the Identify dialog: grabs frames from the opening or the end
//! credits of a movie, reads the text on them and keeps the frames so the user can check them.
//! Nothing is matched automatically; the user picks what to search for.

use crate::ocr::{self, OcrError};
use couchside_core::MediaId;
use couchside_library::{Ffmpeg, Frame, FrameError, FrameRequest};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

const OPENING: FrameRequest = FrameRequest {
    start: Duration::ZERO,
    window: Duration::from_secs(10 * 60),
    min_gap: Duration::from_millis(1500),
    max_frames: 300,
    width: 960,
};
/// The end credits: the last [`ENDING_WINDOW`] of the movie. Credits often open with a title
/// card, and some movies only show their title there.
const ENDING: FrameRequest = FrameRequest {
    start: Duration::ZERO,
    window: ENDING_WINDOW,
    min_gap: Duration::from_secs(2),
    max_frames: 360,
    width: 960,
};
const ENDING_WINDOW: Duration = Duration::from_secs(12 * 60);
/// Most frames with text shown to the user.
const MAX_TEXT_FRAMES: usize = 48;
/// Frames shown when no text was found (or OCR isn't available), spread over the window.
const FALLBACK_FRAMES: usize = 24;
/// Most suggested titles.
const MAX_SUGGESTIONS: usize = 8;
/// Scans whose frames are kept in memory for the dialog.
const KEPT_SCANS: usize = 3;
/// Longest line considered a possible title.
const MAX_TITLE_CHARS: usize = 60;
/// Most words in a possible title.
const MAX_TITLE_WORDS: usize = 8;

/// Credits and logos that appear before most titles. Matched against lower-cased text.
const CREDIT_WORDS: &[&str] = &[
    "presents",
    "a film by",
    "film by",
    "directed by",
    "produced by",
    "written by",
    "screenplay",
    "music by",
    "starring",
    "production",
    "pictures",
    "entertainment",
    "studios",
    "in association with",
    "executive producer",
    "casting",
    "edited by",
    "director of photography",
    "based on",
    "copyright",
    "all rights reserved",
    "www.",
    ".com",
    "subtitle",
];

/// Extra credit words common in end credits. Only used when scanning the ending.
const END_CREDIT_WORDS: &[&str] = &[
    "supervisor",
    "coordinator",
    "assistant",
    "department",
    "visual effects",
    "special effects",
    "stunt",
    "special thanks",
    "thanks to",
    "courtesy of",
    "performed by",
    "composed by",
    "soundtrack",
    "dolby",
    "motion picture",
    "filmed in",
    "filmed at",
    "no animals",
    "iatse",
    "©",
];

/// Which part of the movie to scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPart {
    /// The first minutes, where most titles appear.
    Opening,
    /// The end credits.
    Ending,
}

/// Errors from a title scan.
#[derive(Debug, thiserror::Error)]
pub enum TitleScanError {
    /// ffmpeg wasn't found at startup.
    #[error(
        "ffmpeg wasn't found, so Couchside can't look inside videos (see [ffmpeg] in config.toml)"
    )]
    NoFfmpeg,
    /// Another scan is still running.
    #[error("another title scan is still running; try again when it's done")]
    Busy,
    /// ffmpeg couldn't read the file.
    #[error(transparent)]
    Frames(#[from] FrameError),
    /// The video has no frames in the scanned window.
    #[error("couldn't find any video frames in this file")]
    NoFrames,
    /// The end can't be found because the video's length isn't known yet.
    #[error("Couchside doesn't know how long this video is yet, so it can't find the end credits; try again after the library scan")]
    NoRuntime,
    /// The OCR worker thread failed.
    #[error("text recognition failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// One frame shown in the dialog.
#[derive(Debug, Clone)]
pub struct ScannedFrame {
    /// Position in the video, in seconds.
    pub time_seconds: f64,
    /// Lines of text read on the frame.
    pub lines: Vec<String>,
}

/// What a scan found.
#[derive(Debug, Clone)]
pub struct ScanResult {
    /// Frames to show, in video order. Images come from [`TitleScanner::frame_image`].
    pub frames: Vec<ScannedFrame>,
    /// Text that looks like a title, best first.
    pub suggestions: Vec<String>,
    /// Whether text recognition ran.
    pub ocr_available: bool,
    /// How many frames were read.
    pub frames_read: usize,
}

/// JPEGs of the last few scans, oldest first.
type KeptImages = VecDeque<(MediaId, Vec<Vec<u8>>)>;

/// Runs title scans one at a time and keeps the last few scans' images.
pub struct TitleScanner {
    ffmpeg: Option<Ffmpeg>,
    running: tokio::sync::Semaphore,
    images: Mutex<KeptImages>,
}

impl TitleScanner {
    /// Creates the scanner. Scans fail with [`TitleScanError::NoFfmpeg`] without ffmpeg.
    pub fn new(ffmpeg: Option<Ffmpeg>) -> Self {
        Self {
            ffmpeg,
            running: tokio::sync::Semaphore::new(1),
            images: Mutex::new(VecDeque::new()),
        }
    }

    /// Whether ffmpeg was found.
    pub fn available(&self) -> bool {
        self.ffmpeg.is_some()
    }

    /// Grabs frames from `part` of `file`, reads their text and keeps the frames worth showing.
    /// `runtime_seconds` (the video's length) is needed to find the ending.
    pub async fn scan(
        &self,
        id: MediaId,
        file: &Path,
        part: ScanPart,
        runtime_seconds: Option<f64>,
    ) -> Result<ScanResult, TitleScanError> {
        let ffmpeg = self.ffmpeg.as_ref().ok_or(TitleScanError::NoFfmpeg)?;
        let request = match part {
            ScanPart::Opening => OPENING,
            ScanPart::Ending => {
                let runtime = runtime_seconds
                    .filter(|r| r.is_finite() && *r > 0.0)
                    .ok_or(TitleScanError::NoRuntime)?;
                FrameRequest {
                    start: Duration::from_secs_f64(
                        (runtime - ENDING_WINDOW.as_secs_f64()).max(0.0),
                    ),
                    ..ENDING
                }
            }
        };
        let _permit = self
            .running
            .try_acquire()
            .map_err(|_| TitleScanError::Busy)?;
        let frames = ffmpeg.grab_frames(file, request).await?;
        if frames.is_empty() {
            return Err(TitleScanError::NoFrames);
        }
        let (frames, texts) = tokio::task::spawn_blocking(move || {
            let images: Vec<&[u8]> = frames.iter().map(|f| f.jpeg.as_slice()).collect();
            let texts = ocr::read_lines(&images);
            (frames, texts)
        })
        .await?;
        let ocr_available = match &texts {
            Ok(_) => true,
            Err(OcrError::Unavailable) => false,
        };
        let texts = texts.unwrap_or_else(|_| vec![Vec::new(); frames.len()]);
        let frames_read = frames.len();
        let suggestions = suggest_titles(&texts, part);
        let chosen = choose_frames(&texts);

        let mut images = Vec::with_capacity(chosen.len());
        let mut shown = Vec::with_capacity(chosen.len());
        let mut frames: Vec<Option<Frame>> = frames.into_iter().map(Some).collect();
        for i in chosen {
            if let Some(frame) = frames.get_mut(i).and_then(Option::take) {
                images.push(frame.jpeg);
                shown.push(ScannedFrame {
                    time_seconds: frame.time_seconds,
                    lines: texts.get(i).cloned().unwrap_or_default(),
                });
            }
        }
        self.keep_images(id, images);
        tracing::info!(
            item = id.0,
            frames_read,
            shown = shown.len(),
            ?part,
            ocr_available,
            "title scan finished"
        );
        Ok(ScanResult {
            frames: shown,
            suggestions,
            ocr_available,
            frames_read,
        })
    }

    /// The JPEG for frame `index` of the last scan of item `id`.
    pub fn frame_image(&self, id: MediaId, index: usize) -> Option<Vec<u8>> {
        let images = self.images.lock().ok()?;
        let (_, frames) = images.iter().find(|(item, _)| *item == id)?;
        frames.get(index).cloned()
    }

    fn keep_images(&self, id: MediaId, images: Vec<Vec<u8>>) {
        if let Ok(mut kept) = self.images.lock() {
            kept.retain(|(item, _)| *item != id);
            kept.push_back((id, images));
            while kept.len() > KEPT_SCANS {
                kept.pop_front();
            }
        }
    }
}

/// Indexes of the frames to show, in video order: those with text if any, otherwise frames
/// spread evenly. With more text frames than fit (end credits are full of text), frames with
/// the fewest lines win, since title cards show little else.
fn choose_frames(texts: &[Vec<String>]) -> Vec<usize> {
    let mut with_text: Vec<(usize, usize)> = texts
        .iter()
        .enumerate()
        .map(|(i, lines)| (i, lines.iter().filter(|l| letter_count(l) >= 2).count()))
        .filter(|(_, count)| *count > 0)
        .collect();
    if !with_text.is_empty() {
        with_text.sort_by_key(|(i, count)| (*count, *i));
        let mut chosen: Vec<usize> = with_text
            .into_iter()
            .take(MAX_TEXT_FRAMES)
            .map(|(i, _)| i)
            .collect();
        chosen.sort_unstable();
        return chosen;
    }
    let total = texts.len();
    let step = total.div_ceil(FALLBACK_FRAMES).max(1);
    (0..total).step_by(step).collect()
}

fn letter_count(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}

/// Tidies an OCR line: single spaces, no stray punctuation at the ends.
fn clean_line(line: &str) -> String {
    let joined = line.split_whitespace().collect::<Vec<_>>().join(" ");
    joined
        .trim_matches(|c: char| !c.is_alphanumeric() && !"!?'&".contains(c))
        .to_string()
}

/// Whether a cleaned line could be a movie title rather than a credit or OCR noise.
fn looks_like_title(line: &str, part: ScanPart) -> bool {
    let letters = letter_count(line);
    let visible = line.chars().filter(|c| !c.is_whitespace()).count();
    let lower = line.to_lowercase();
    let end_credit = part == ScanPart::Ending
        && (lower == "the end" || END_CREDIT_WORDS.iter().any(|w| lower.contains(w)));
    letters >= 2
        && line.chars().count() <= MAX_TITLE_CHARS
        && line.split_whitespace().count() <= MAX_TITLE_WORDS
        && letters * 10 >= visible * 6
        && !CREDIT_WORDS.iter().any(|w| lower.contains(w))
        && !end_credit
}

/// Key used to treat OCR variants of the same text as one ("THE MATRIX", "The Matrix.").
fn text_key(line: &str) -> String {
    line.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Ranks text that might be the title: text seen on more frames ranks higher, and so does text
/// alone on screen (titles are usually shown by themselves). Ties keep video order. In the end
/// credits only text alone on screen counts: rolling credits are full of names.
fn suggest_titles(texts: &[Vec<String>], part: ScanPart) -> Vec<String> {
    let mut found: HashMap<String, (String, usize, usize)> = HashMap::new();
    let mut order = Vec::new();
    for lines in texts {
        let alone = lines.iter().filter(|l| letter_count(l) >= 2).count() <= 2;
        if part == ScanPart::Ending && !alone {
            continue;
        }
        for line in lines.iter().map(|l| clean_line(l)) {
            if !looks_like_title(&line, part) {
                continue;
            }
            let key = text_key(&line);
            let entry = found.entry(key.clone()).or_insert_with(|| {
                order.push(key);
                (line, 0, 0)
            });
            entry.1 += 1;
            if alone {
                entry.2 += 1;
            }
        }
    }
    let mut ranked: Vec<(usize, usize, String)> = order
        .into_iter()
        .enumerate()
        .filter_map(|(position, key)| {
            let (text, seen, alone) = found.remove(&key)?;
            Some((seen + alone, position, text))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked
        .into_iter()
        .map(|(_, _, text)| text)
        .take(MAX_SUGGESTIONS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(lines: &[&[&str]]) -> Vec<Vec<String>> {
        lines
            .iter()
            .map(|f| f.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    fn title_shown_alone_beats_credits() {
        let texts = frames(&[
            &["WARNER BROS. PICTURES PRESENTS"],
            &["A FILM BY CHRISTOPHER NOLAN"],
            &[],
            &["INCEPTION"],
            &["INCEPTION"],
            &["Inception."],
        ]);
        assert_eq!(
            suggest_titles(&texts, ScanPart::Opening),
            vec!["INCEPTION".to_string()]
        );
    }

    #[test]
    fn rejects_noise_and_long_lines() {
        let title = |line| looks_like_title(line, ScanPart::Opening);
        assert!(!title("~|; ,"));
        assert!(!title("x"));
        assert!(!title(
            "this sentence is far too long to be the title of any reasonable movie at all"
        ));
        assert!(!title("Directed by Someone"));
        assert!(title("Heat"));
        assert!(title("Amélie"));
        assert!(title("2001: A Space Odyssey"));
    }

    #[test]
    fn end_credits_skip_rolling_credits_and_crew_lines() {
        let texts = frames(&[
            &["THE END"],
            &["HEAT"],
            &["HEAT"],
            &["Jane Doe", "John Roe", "Ann Poe", "Max Moe"],
            &["Stunt Coordinator"],
            &["Special Thanks"],
        ]);
        assert_eq!(suggest_titles(&texts, ScanPart::Ending), vec!["HEAT"]);
        // The same words are fine in an opening scan.
        assert!(looks_like_title("The End", ScanPart::Opening));
    }

    #[test]
    fn crowded_frames_lose_to_title_cards() {
        let mut texts = vec![vec!["a b".to_string(); 6]; MAX_TEXT_FRAMES + 5];
        texts[MAX_TEXT_FRAMES + 2] = vec!["Heat".to_string()];
        let chosen = choose_frames(&texts);
        assert_eq!(chosen.len(), MAX_TEXT_FRAMES);
        assert!(chosen.contains(&(MAX_TEXT_FRAMES + 2)));
        assert!(chosen.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn cleans_ocr_lines() {
        assert_eq!(clean_line("  — THE   MATRIX . "), "THE MATRIX");
        assert_eq!(clean_line("Airplane!"), "Airplane!");
        assert_eq!(text_key("The Matrix."), text_key("THE MATRIX"));
    }

    #[test]
    fn frequent_text_ranks_first_and_ties_keep_order() {
        let texts = frames(&[&["Alpha"], &["Beta"], &["Beta"], &["Gamma"]]);
        assert_eq!(
            suggest_titles(&texts, ScanPart::Opening),
            vec!["Beta", "Alpha", "Gamma"]
        );
    }

    #[test]
    fn shows_text_frames_or_spread_fallback() {
        let texts = frames(&[&[], &["Heat"], &[], &["~"]]);
        assert_eq!(choose_frames(&texts), vec![1]);

        let empty = vec![Vec::new(); 100];
        let chosen = choose_frames(&empty);
        assert!(chosen.len() <= FALLBACK_FRAMES);
        assert_eq!(chosen.first(), Some(&0));
        assert!(chosen.windows(2).all(|w| w[0] < w[1]));
    }
}
