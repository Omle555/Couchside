//! Online metadata from The Movie Database (TMDB).
//!
//! Images are downloaded once into a local cache folder; clients are never
//! sent to TMDB directly.

pub mod matching;

use matching::{best_match, query_variants, MatchInput};
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;

const API_BASE: &str = "https://api.themoviedb.org/3";
const IMAGE_BASE: &str = "https://image.tmdb.org/t/p";
const MAX_RETRIES: u32 = 3;

/// Errors from TMDB requests.
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    /// Network or HTTP error.
    #[error("TMDB request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The API key was rejected.
    #[error("TMDB rejected the API key. Check it under Settings → Metadata.")]
    Unauthorized,
    /// TMDB kept rate limiting us.
    #[error("TMDB rate limit exceeded")]
    RateLimited,
    /// Writing an image to the cache failed.
    #[error("could not save image: {0}")]
    Io(#[from] std::io::Error),
}

/// Result alias for metadata operations.
pub type MetadataResult<T> = Result<T, MetadataError>;

enum Auth {
    /// v3 API key, sent as a query parameter.
    ApiKey(String),
    /// v4 read access token, sent as a bearer header.
    Bearer(String),
}

/// A TV show match from TMDB.
#[derive(Debug, Clone)]
pub struct ShowInfo {
    /// TMDB id.
    pub tmdb_id: i64,
    /// Localized title.
    pub title: String,
    /// Release year.
    pub year: Option<i64>,
    /// Plot summary.
    pub overview: Option<String>,
    /// TMDB poster path (e.g. `/abc.jpg`).
    pub poster_path: Option<String>,
    /// TMDB backdrop path.
    pub backdrop_path: Option<String>,
}

/// A movie search result, scored against the query.
#[derive(Debug, Clone)]
pub struct MovieCandidate {
    /// TMDB id.
    pub tmdb_id: i64,
    /// Localized title.
    pub title: String,
    /// Original-language title.
    pub original_title: Option<String>,
    /// Release year.
    pub year: Option<i64>,
    /// Plot summary.
    pub overview: Option<String>,
    /// TMDB poster path (e.g. `/abc.jpg`).
    pub poster_path: Option<String>,
    /// Number of TMDB votes.
    pub vote_count: i64,
    /// Match score against the query (higher is better; >= 0.6 is a confident match).
    pub score: f64,
}

/// Full details of one movie.
#[derive(Debug, Clone, Default)]
pub struct MovieDetails {
    /// TMDB id.
    pub tmdb_id: i64,
    /// Localized title.
    pub title: String,
    /// Original-language title (only when different from `title`).
    pub original_title: Option<String>,
    /// Release year.
    pub year: Option<i64>,
    /// Release date, `YYYY-MM-DD`.
    pub release_date: Option<String>,
    /// Plot summary.
    pub overview: Option<String>,
    /// Tagline.
    pub tagline: Option<String>,
    /// Genre names.
    pub genres: Vec<String>,
    /// Average user rating out of 10, one decimal.
    pub rating: Option<f64>,
    /// Runtime in minutes.
    pub runtime_minutes: Option<i64>,
    /// Age rating for the configured region, e.g. `M`, `PG-13`.
    pub certification: Option<String>,
    /// IMDb id.
    pub imdb_id: Option<String>,
    /// Top-billed cast.
    pub cast: Vec<String>,
    /// Directors.
    pub directors: Vec<String>,
    /// TMDB poster path.
    pub poster_path: Option<String>,
    /// TMDB backdrop path.
    pub backdrop_path: Option<String>,
}

/// Number of cast members kept.
const TOP_CAST: usize = 8;

/// One episode from a TMDB season listing.
#[derive(Debug, Clone)]
pub struct EpisodeInfo {
    /// Episode number within the season.
    pub episode_number: i64,
    /// Episode title.
    pub name: Option<String>,
    /// Plot summary.
    pub overview: Option<String>,
    /// TMDB still image path.
    pub still_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse<T> {
    #[serde(default = "Vec::new")]
    results: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct MovieResult {
    id: i64,
    title: Option<String>,
    original_title: Option<String>,
    release_date: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
    #[serde(default)]
    vote_count: i64,
}

#[derive(Debug, Deserialize)]
struct FindResponse {
    #[serde(default)]
    movie_results: Vec<IdOnly>,
}

#[derive(Debug, Deserialize)]
struct IdOnly {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct MovieDetailsResponse {
    id: i64,
    title: Option<String>,
    original_title: Option<String>,
    release_date: Option<String>,
    overview: Option<String>,
    tagline: Option<String>,
    runtime: Option<i64>,
    #[serde(default)]
    genres: Vec<Named>,
    vote_average: Option<f64>,
    #[serde(default)]
    vote_count: i64,
    imdb_id: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    credits: Option<Credits>,
    release_dates: Option<ReleaseDates>,
}

#[derive(Debug, Deserialize)]
struct Named {
    name: String,
}

#[derive(Debug, Deserialize)]
struct Credits {
    #[serde(default)]
    cast: Vec<CastMember>,
    #[serde(default)]
    crew: Vec<CrewMember>,
}

#[derive(Debug, Deserialize)]
struct CastMember {
    name: String,
    order: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CrewMember {
    name: String,
    job: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReleaseDates {
    #[serde(default)]
    results: Vec<CountryReleases>,
}

#[derive(Debug, Deserialize)]
struct CountryReleases {
    iso_3166_1: String,
    #[serde(default)]
    release_dates: Vec<ReleaseDate>,
}

#[derive(Debug, Deserialize)]
struct ReleaseDate {
    certification: Option<String>,
}

/// Region code for certifications: `en-AU` → `AU`, otherwise `US`.
fn region_of(language: &str) -> String {
    language
        .split_once('-')
        .map(|(_, r)| r.to_ascii_uppercase())
        .filter(|r| r.len() == 2)
        .unwrap_or_else(|| "US".to_string())
}

fn certification_for(dates: &ReleaseDates, region: &str) -> Option<String> {
    let pick = |country: &str| {
        dates
            .results
            .iter()
            .filter(|c| c.iso_3166_1.eq_ignore_ascii_case(country))
            .flat_map(|c| c.release_dates.iter())
            .filter_map(|d| non_empty(d.certification.clone()))
            .next()
    };
    pick(region).or_else(|| pick("US"))
}

impl MovieDetailsResponse {
    fn into_details(self, region: &str) -> MovieDetails {
        let title = non_empty(self.title).unwrap_or_default();
        let original_title = non_empty(self.original_title).filter(|o| *o != title);
        let certification = self
            .release_dates
            .as_ref()
            .and_then(|d| certification_for(d, region));
        let (cast, directors) = match self.credits {
            Some(credits) => {
                let mut cast = credits.cast;
                cast.sort_by_key(|m| m.order.unwrap_or(i64::MAX));
                let cast = cast.into_iter().take(TOP_CAST).map(|m| m.name).collect();
                let mut directors: Vec<String> = credits
                    .crew
                    .into_iter()
                    .filter(|m| m.job.as_deref() == Some("Director"))
                    .map(|m| m.name)
                    .collect();
                directors.dedup();
                (cast, directors)
            }
            None => (Vec::new(), Vec::new()),
        };
        MovieDetails {
            tmdb_id: self.id,
            title,
            original_title,
            year: year_of(self.release_date.as_deref()),
            release_date: non_empty(self.release_date),
            overview: non_empty(self.overview),
            tagline: non_empty(self.tagline),
            genres: self.genres.into_iter().map(|g| g.name).collect(),
            rating: self
                .vote_average
                .filter(|_| self.vote_count > 0)
                .map(|r| (r * 10.0).round() / 10.0),
            runtime_minutes: self.runtime.filter(|r| *r > 0),
            certification,
            imdb_id: non_empty(self.imdb_id),
            cast,
            directors,
            poster_path: self.poster_path,
            backdrop_path: self.backdrop_path,
        }
    }
}

/// True for TMDB image paths like `/abc123.jpg` (used to validate proxy requests).
pub fn is_tmdb_image_path(path: &str) -> bool {
    let Some(name) = path.strip_prefix('/') else {
        return false;
    };
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && stem.len() <= 64
        && stem
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && matches!(ext, "jpg" | "jpeg" | "png" | "webp")
}

#[derive(Debug, Deserialize)]
struct TvResult {
    id: i64,
    name: Option<String>,
    first_air_date: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SeasonResponse {
    #[serde(default)]
    episodes: Vec<EpisodeResult>,
}

#[derive(Debug, Deserialize)]
struct EpisodeResult {
    episode_number: i64,
    name: Option<String>,
    overview: Option<String>,
    still_path: Option<String>,
}

fn year_of(date: Option<&str>) -> Option<i64> {
    date.and_then(|d| d.get(..4)).and_then(|y| y.parse().ok())
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// TMDB client with a local image cache.
pub struct Tmdb {
    http: reqwest::Client,
    auth: Auth,
    language: String,
    image_dir: PathBuf,
}

impl Tmdb {
    /// Creates a client. Keys that look like JWTs (v4 tokens) are sent as bearer tokens.
    pub fn new(key: &str, language: &str, image_dir: PathBuf) -> MetadataResult<Self> {
        let key = key.trim().to_string();
        let auth = if key.starts_with("eyJ") {
            Auth::Bearer(key)
        } else {
            Auth::ApiKey(key)
        };
        let http = reqwest::Client::builder()
            .user_agent(concat!("Solidfin/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            auth,
            language: language.to_string(),
            image_dir,
        })
    }

    /// Folder where downloaded images are stored.
    pub fn image_dir(&self) -> &Path {
        &self.image_dir
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> MetadataResult<Option<T>> {
        let url = format!("{API_BASE}{path}");
        for attempt in 0..=MAX_RETRIES {
            let mut req = self
                .http
                .get(&url)
                .query(&[("language", self.language.as_str())])
                .query(query);
            req = match &self.auth {
                Auth::ApiKey(key) => req.query(&[("api_key", key.as_str())]),
                Auth::Bearer(token) => req.bearer_auth(token),
            };
            let resp = req.send().await?;
            match resp.status().as_u16() {
                200..=299 => return Ok(Some(resp.json::<T>().await?)),
                401 => return Err(MetadataError::Unauthorized),
                404 => return Ok(None),
                429 if attempt < MAX_RETRIES => {
                    let wait = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .unwrap_or(2)
                        .clamp(1, 30);
                    tracing::debug!(wait, "TMDB rate limited; waiting");
                    tokio::time::sleep(Duration::from_secs(wait)).await;
                }
                429 => return Err(MetadataError::RateLimited),
                _ => {
                    resp.error_for_status()?;
                    return Ok(None);
                }
            }
        }
        Err(MetadataError::RateLimited)
    }

    /// Checks that the API key works.
    pub async fn validate(&self) -> MetadataResult<()> {
        self.get_json::<IgnoredAny>("/configuration", &[]).await?;
        Ok(())
    }

    /// Raw search results for a title (and optional year), scored and sorted best first.
    pub async fn search_movies(
        &self,
        title: &str,
        year: Option<i64>,
    ) -> MetadataResult<Vec<MovieCandidate>> {
        let mut query = vec![
            ("query", title.to_string()),
            ("include_adult", "false".to_string()),
        ];
        if let Some(y) = year {
            query.push(("year", y.to_string()));
        }
        let found: Option<SearchResponse<MovieResult>> =
            self.get_json("/search/movie", &query).await?;
        let mut candidates: Vec<MovieCandidate> = found
            .map(|r| r.results)
            .unwrap_or_default()
            .into_iter()
            .map(|m| {
                let title_text = non_empty(m.title).unwrap_or_default();
                let original_title = non_empty(m.original_title).filter(|o| *o != title_text);
                let candidate_year = year_of(m.release_date.as_deref());
                let score = matching::score(
                    title,
                    year,
                    &MatchInput {
                        title: &title_text,
                        original_title: original_title.as_deref(),
                        year: candidate_year,
                        vote_count: m.vote_count,
                    },
                );
                MovieCandidate {
                    tmdb_id: m.id,
                    title: title_text,
                    original_title,
                    year: candidate_year,
                    overview: non_empty(m.overview),
                    poster_path: m.poster_path,
                    vote_count: m.vote_count,
                    score,
                }
            })
            .collect();
        candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
        Ok(candidates)
    }

    /// Finds a confident match for a parsed title and year, or `None`.
    /// Tries the title with the year, then without it (release years differ
    /// between countries), then a shorter form of long titles.
    pub async fn match_movie(
        &self,
        title: &str,
        year: Option<i64>,
    ) -> MetadataResult<Option<MovieCandidate>> {
        for variant in query_variants(title) {
            let mut attempts = vec![year];
            if year.is_some() {
                attempts.push(None);
            }
            for search_year in attempts {
                let candidates = self.search_movies(&variant, search_year).await?;
                let inputs: Vec<MatchInput<'_>> = candidates
                    .iter()
                    .map(|c| MatchInput {
                        title: &c.title,
                        original_title: c.original_title.as_deref(),
                        year: c.year,
                        vote_count: c.vote_count,
                    })
                    .collect();
                // Always score against the parsed year, even when searching without it.
                if let Some((index, score)) = best_match(title, year, &inputs) {
                    let mut chosen = candidates[index].clone();
                    chosen.score = score;
                    return Ok(Some(chosen));
                }
            }
        }
        Ok(None)
    }

    /// Full details (with credits and age ratings) for one movie, or `None` if TMDB doesn't know the id.
    pub async fn movie_details(&self, tmdb_id: i64) -> MetadataResult<Option<MovieDetails>> {
        let found: Option<MovieDetailsResponse> = self
            .get_json(
                &format!("/movie/{tmdb_id}"),
                &[("append_to_response", "credits,release_dates".to_string())],
            )
            .await?;
        let region = region_of(&self.language);
        Ok(found.map(|d| d.into_details(&region)))
    }

    /// Looks up the TMDB id for an IMDb id such as `tt0133093`.
    pub async fn find_movie_by_imdb(&self, imdb_id: &str) -> MetadataResult<Option<i64>> {
        let found: Option<FindResponse> = self
            .get_json(
                &format!("/find/{imdb_id}"),
                &[("external_source", "imdb_id".to_string())],
            )
            .await?;
        Ok(found.and_then(|f| f.movie_results.first().map(|m| m.id)))
    }

    /// Downloads a TMDB image into memory (for the "Identify" picker thumbnails).
    pub async fn fetch_image(&self, tmdb_path: &str, size: &str) -> MetadataResult<Vec<u8>> {
        let url = format!("{IMAGE_BASE}/{size}{tmdb_path}");
        let bytes = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Ok(bytes.to_vec())
    }

    /// Finds the best TV show match for a title and optional first-air year.
    pub async fn search_show(
        &self,
        title: &str,
        year: Option<i64>,
    ) -> MetadataResult<Option<ShowInfo>> {
        let mut query = vec![
            ("query", title.to_string()),
            ("include_adult", "false".to_string()),
        ];
        if let Some(y) = year {
            query.push(("first_air_date_year", y.to_string()));
        }
        let mut found: Option<SearchResponse<TvResult>> =
            self.get_json("/search/tv", &query).await?;
        if year.is_some() && found.as_ref().is_none_or(|r| r.results.is_empty()) {
            query.retain(|(k, _)| *k != "first_air_date_year");
            found = self.get_json("/search/tv", &query).await?;
        }
        Ok(found
            .and_then(|r| r.results.into_iter().next())
            .map(|s| ShowInfo {
                tmdb_id: s.id,
                title: non_empty(s.name).unwrap_or_else(|| title.to_string()),
                year: year_of(s.first_air_date.as_deref()),
                overview: non_empty(s.overview),
                poster_path: s.poster_path,
                backdrop_path: s.backdrop_path,
            }))
    }

    /// All episodes of one season. Empty if the season is unknown to TMDB.
    pub async fn season(&self, show_tmdb_id: i64, season: i64) -> MetadataResult<Vec<EpisodeInfo>> {
        let found: Option<SeasonResponse> = self
            .get_json(&format!("/tv/{show_tmdb_id}/season/{season}"), &[])
            .await?;
        Ok(found
            .map(|s| s.episodes)
            .unwrap_or_default()
            .into_iter()
            .map(|e| EpisodeInfo {
                episode_number: e.episode_number,
                name: non_empty(e.name),
                overview: non_empty(e.overview),
                still_path: e.still_path,
            })
            .collect())
    }

    /// Downloads a TMDB image into the cache as `{file_stem}.{ext}` and returns the file name.
    /// `size` is a TMDB size such as `w500` or `w1280`.
    pub async fn download_image(
        &self,
        tmdb_path: &str,
        size: &str,
        file_stem: &str,
    ) -> MetadataResult<String> {
        let ext = Path::new(tmdb_path)
            .extension()
            .and_then(|e| e.to_str())
            .filter(|e| matches!(*e, "jpg" | "jpeg" | "png" | "webp"))
            .unwrap_or("jpg");
        let file_name = format!("{file_stem}.{ext}");
        let target = self.image_dir.join(&file_name);
        let url = format!("{IMAGE_BASE}/{size}{tmdb_path}");
        let bytes = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        tokio::fs::create_dir_all(&self.image_dir).await?;
        let temp = self.image_dir.join(format!("{file_name}.part"));
        tokio::fs::write(&temp, &bytes).await?;
        tokio::fs::rename(&temp, &target).await?;
        Ok(file_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_movie_details() {
        let json = r#"{
            "id": 603, "title": "The Matrix", "original_title": "The Matrix",
            "release_date": "1999-03-30", "overview": "A hacker...", "tagline": "Welcome to the Real World.",
            "runtime": 136, "genres": [{"id": 28, "name": "Action"}, {"id": 878, "name": "Science Fiction"}],
            "vote_average": 8.217, "vote_count": 26000, "imdb_id": "tt0133093",
            "poster_path": "/p.jpg", "backdrop_path": "/b.jpg",
            "credits": {
                "cast": [{"name": "Laurence Fishburne", "order": 1}, {"name": "Keanu Reeves", "order": 0}],
                "crew": [{"name": "Lana Wachowski", "job": "Director"}, {"name": "Lilly Wachowski", "job": "Director"},
                         {"name": "Joel Silver", "job": "Producer"}]
            },
            "release_dates": {"results": [
                {"iso_3166_1": "US", "release_dates": [{"certification": "R"}]},
                {"iso_3166_1": "AU", "release_dates": [{"certification": ""}, {"certification": "M"}]}
            ]}
        }"#;
        let parsed: MovieDetailsResponse = serde_json::from_str(json).expect("parses");
        let d = parsed.into_details("AU");
        assert_eq!(d.title, "The Matrix");
        assert_eq!(d.original_title, None, "same as title");
        assert_eq!(d.year, Some(1999));
        assert_eq!(d.genres, ["Action", "Science Fiction"]);
        assert_eq!(d.rating, Some(8.2));
        assert_eq!(d.runtime_minutes, Some(136));
        assert_eq!(d.certification.as_deref(), Some("M"));
        assert_eq!(d.cast, ["Keanu Reeves", "Laurence Fishburne"]);
        assert_eq!(d.directors, ["Lana Wachowski", "Lilly Wachowski"]);
        assert_eq!(d.imdb_id.as_deref(), Some("tt0133093"));
    }

    #[test]
    fn certification_falls_back_to_us() {
        let dates = ReleaseDates {
            results: vec![CountryReleases {
                iso_3166_1: "US".into(),
                release_dates: vec![ReleaseDate {
                    certification: Some("PG-13".into()),
                }],
            }],
        };
        assert_eq!(certification_for(&dates, "GB").as_deref(), Some("PG-13"));
        assert_eq!(region_of("en-AU"), "AU");
        assert_eq!(region_of("fr"), "US");
    }

    #[test]
    fn image_paths() {
        assert!(is_tmdb_image_path("/kqjL17yufvn9OVLyXYpvtyrFfak.jpg"));
        assert!(!is_tmdb_image_path("kqjL.jpg"));
        assert!(!is_tmdb_image_path("/../../etc/passwd.jpg"));
        assert!(!is_tmdb_image_path("/a.exe"));
        assert!(!is_tmdb_image_path("/.jpg"));
    }

    #[test]
    fn parses_search_response() {
        let json = r#"{"page":1,"results":[{"id":603,"title":"The Matrix","release_date":"1999-03-30",
            "overview":"A hacker...","poster_path":"/p.jpg","backdrop_path":null}]}"#;
        let parsed: SearchResponse<MovieResult> = serde_json::from_str(json).expect("parses");
        assert_eq!(parsed.results[0].id, 603);
        assert_eq!(
            year_of(parsed.results[0].release_date.as_deref()),
            Some(1999)
        );
    }

    #[test]
    fn year_parsing() {
        assert_eq!(year_of(Some("2010-07-15")), Some(2010));
        assert_eq!(year_of(Some("")), None);
        assert_eq!(year_of(None), None);
    }
}
