//! Choosing the right TMDB search result for a title parsed from a file name.
//!
//! Pure functions (no I/O) so the rules can be tested exhaustively.
//! Each candidate gets a score from title similarity, release year and a small
//! popularity tie-breaker. Weak matches are rejected rather than guessed, so a
//! movie is left unmatched instead of being given the wrong poster.

/// The parts of a search result that matter for matching.
#[derive(Debug, Clone, Copy)]
pub struct MatchInput<'a> {
    /// Localized title.
    pub title: &'a str,
    /// Original-language title.
    pub original_title: Option<&'a str>,
    /// Release year.
    pub year: Option<i64>,
    /// Number of TMDB votes (popularity proxy).
    pub vote_count: i64,
}

/// Minimum title similarity (0..=1) for a candidate to be considered.
pub const MIN_TITLE_SIMILARITY: f64 = 0.6;
/// Minimum overall score for a candidate to be accepted.
pub const MIN_SCORE: f64 = 0.6;

/// Replaces common accented Latin letters with their plain form.
fn fold_accent(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
        'ç' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
        'š' => 's',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
        'ý' | 'ÿ' => 'y',
        'ž' => 'z',
        _ => c,
    }
}

/// Lowercases, folds accents, turns `&` into `and`, drops punctuation and a
/// leading article: `"The Lord of the Rings: The Two Towers"` →
/// `"lord of the rings the two towers"`.
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase().replace('&', " and ");
    let cleaned: String = lower
        .chars()
        .map(fold_accent)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let words = match words.split_first() {
        Some((first, rest)) if !rest.is_empty() && matches!(*first, "the" | "a" | "an") => rest,
        _ => &words[..],
    };
    words.join(" ")
}

fn dice<T: PartialEq>(a: &[T], b: &[T]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut remaining: Vec<&T> = b.iter().collect();
    let mut common = 0usize;
    for item in a {
        if let Some(pos) = remaining.iter().position(|x| *x == item) {
            remaining.swap_remove(pos);
            common += 1;
        }
    }
    (2 * common) as f64 / (a.len() + b.len()) as f64
}

fn bigrams(s: &str) -> Vec<(char, char)> {
    let chars: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}

/// Similarity of two titles from 0 (unrelated) to 1 (identical after normalising).
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let (a, b) = (normalize_title(a), normalize_title(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    // "Spider Man" vs "Spiderman", "Se7en" spacing differences...
    if a.replace(' ', "") == b.replace(' ', "") {
        return 0.97;
    }
    let words_a: Vec<&str> = a.split(' ').collect();
    let words_b: Vec<&str> = b.split(' ').collect();
    let word_score = dice(&words_a, &words_b);
    let char_score = dice(&bigrams(&a), &bigrams(&b)) * 0.95;
    word_score.max(char_score)
}

/// Overall score of one candidate for a parsed title and optional year.
pub fn score(query_title: &str, query_year: Option<i64>, candidate: &MatchInput<'_>) -> f64 {
    let similarity = title_similarity(query_title, candidate.title).max(
        candidate
            .original_title
            .map_or(0.0, |original| title_similarity(query_title, original)),
    );
    if similarity < MIN_TITLE_SIMILARITY {
        return similarity;
    }
    let year_adjust = match (query_year, candidate.year) {
        (Some(q), Some(c)) => match (q - c).abs() {
            0 => 0.3,
            1 => 0.15,
            _ => -0.5,
        },
        (Some(_), None) => -0.1,
        (None, _) => 0.0,
    };
    // ln(1 + votes) / 200 gives at most ~0.05 for very popular films.
    let popularity = ((candidate.vote_count.max(0) as f64) + 1.0).ln() / 200.0;
    similarity + year_adjust + popularity.min(0.05)
}

/// Index and score of the best acceptable candidate, or `None` if nothing is a
/// confident match.
pub fn best_match(
    query_title: &str,
    query_year: Option<i64>,
    candidates: &[MatchInput<'_>],
) -> Option<(usize, f64)> {
    candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (i, score(query_title, query_year, c)))
        .filter(|(_, s)| *s >= MIN_SCORE)
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// Alternative search queries for a title: the title itself, then the part
/// before a subtitle separator (`"Alien: Covenant"` → `"Alien"` is *not*
/// tried because the full title usually works; only long titles are split).
pub fn query_variants(title: &str) -> Vec<String> {
    let mut variants = vec![title.trim().to_string()];
    for sep in [" - ", ": "] {
        if let Some((head, _)) = title.split_once(sep) {
            let head = head.trim();
            if head.split_whitespace().count() >= 2 && !variants.iter().any(|v| v == head) {
                variants.push(head.to_string());
            }
        }
    }
    variants
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(title: &'static str, year: Option<i64>, votes: i64) -> MatchInput<'static> {
        MatchInput {
            title,
            original_title: None,
            year,
            vote_count: votes,
        }
    }

    #[test]
    fn normalizes() {
        assert_eq!(
            normalize_title("The Lord of the Rings: The Two Towers"),
            "lord of the rings the two towers"
        );
        assert_eq!(normalize_title("Fast & Furious"), "fast and furious");
        assert_eq!(normalize_title("Amélie"), "amelie");
        assert_eq!(normalize_title("The"), "the");
    }

    #[test]
    fn exact_title_and_year_wins() {
        let cands = [
            c("The Matrix Reloaded", Some(2003), 9000),
            c("The Matrix", Some(1999), 25000),
        ];
        assert_eq!(
            best_match("The Matrix", Some(1999), &cands).map(|m| m.0),
            Some(1)
        );
    }

    #[test]
    fn year_separates_remakes() {
        let cands = [c("Dune", Some(1984), 3000), c("Dune", Some(2021), 12000)];
        assert_eq!(best_match("Dune", Some(2021), &cands).map(|m| m.0), Some(1));
        assert_eq!(best_match("Dune", Some(1984), &cands).map(|m| m.0), Some(0));
        let cands = [c("Heat", Some(1986), 100), c("Heat", Some(1995), 7000)];
        assert_eq!(best_match("Heat", Some(1995), &cands).map(|m| m.0), Some(1));
    }

    #[test]
    fn wrong_year_only_is_rejected() {
        let cands = [c("Dune", Some(1984), 3000)];
        assert_eq!(best_match("Dune", Some(2021), &cands), None);
    }

    #[test]
    fn off_by_one_year_is_ok() {
        let cands = [c("Parasite", Some(2019), 18000)];
        assert!(best_match("Parasite", Some(2020), &cands).is_some());
    }

    #[test]
    fn punctuation_and_spacing_differences() {
        let cands = [c("Spider-Man: Into the Spider-Verse", Some(2018), 15000)];
        assert!(best_match("Spiderman Into the Spider Verse", Some(2018), &cands).is_some());
        let cands = [c("Mission: Impossible - Fallout", Some(2018), 8000)];
        assert!(best_match("Mission Impossible Fallout", Some(2018), &cands).is_some());
    }

    #[test]
    fn original_title_counts() {
        let cands = [MatchInput {
            title: "Spirited Away",
            original_title: Some("千と千尋の神隠し"),
            year: Some(2001),
            vote_count: 16000,
        }];
        assert!(best_match("千と千尋の神隠し", Some(2001), &cands).is_some());
        let cands = [MatchInput {
            title: "Amélie",
            original_title: Some("Le Fabuleux Destin d'Amélie Poulain"),
            year: Some(2001),
            vote_count: 11000,
        }];
        assert!(best_match("Amelie", Some(2001), &cands).is_some());
    }

    #[test]
    fn prefers_exact_title_over_longer_one_without_year() {
        let cands = [
            c("Inception: The Cobol Job", Some(2010), 300),
            c("Inception", Some(2010), 35000),
        ];
        assert_eq!(best_match("Inception", None, &cands).map(|m| m.0), Some(1));
        let cands = [c("Aliens", Some(1986), 9000), c("Alien", Some(1979), 14000)];
        assert_eq!(
            best_match("Alien", Some(1979), &cands).map(|m| m.0),
            Some(1)
        );
    }

    #[test]
    fn unrelated_results_are_rejected() {
        let cands = [c("Something Else Entirely", Some(2015), 500)];
        assert_eq!(best_match("Holiday Video 2015", Some(2015), &cands), None);
        assert_eq!(best_match("Anything", None, &[]), None);
    }

    #[test]
    fn variants() {
        assert_eq!(query_variants("Heat"), vec!["Heat"]);
        assert_eq!(
            query_variants("Harry Potter - Philosopher's Stone"),
            vec!["Harry Potter - Philosopher's Stone", "Harry Potter"]
        );
        assert_eq!(query_variants("Alien: Covenant"), vec!["Alien: Covenant"]);
    }
}
