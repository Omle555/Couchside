//! The web UI, compiled into the executable so Solidfin is a single file.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

const INDEX_HTML: &str = include_str!("web/index.html");
const APP_JS: &str = include_str!("web/app.js");
const APP_CSS: &str = include_str!("web/app.css");
const FAVICON_SVG: &str = include_str!("web/favicon.svg");

fn asset(content_type: &'static str, body: &'static str) -> impl IntoResponse {
    (
        [(CONTENT_TYPE, content_type), (CACHE_CONTROL, "no-cache")],
        body,
    )
}

/// Routes for the static UI files (they need no state, so any router can use them).
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route(
            "/",
            get(|| async { asset("text/html; charset=utf-8", INDEX_HTML) }),
        )
        .route(
            "/app.js",
            get(|| async { asset("text/javascript; charset=utf-8", APP_JS) }),
        )
        .route(
            "/app.css",
            get(|| async { asset("text/css; charset=utf-8", APP_CSS) }),
        )
        .route(
            "/favicon.svg",
            get(|| async { asset("image/svg+xml", FAVICON_SVG) }),
        )
}
