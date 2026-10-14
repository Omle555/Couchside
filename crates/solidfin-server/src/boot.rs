//! The start-up page: while Solidfin opens its database and finds its tools, the
//! web UI is already served so the browser can show a splash screen with progress.

use crate::{lan, web};
use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use serde::Serialize;
use std::sync::{Arc, Mutex};

/// Start-up steps, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootStep {
    /// Opening (and if needed upgrading) the database.
    Database,
    /// Looking for ffprobe and ffmpeg.
    Tools,
    /// Reading saved settings and the TMDB key.
    Settings,
    /// Starting the background library worker.
    Library,
}

impl BootStep {
    const ALL: [BootStep; 4] = [
        BootStep::Database,
        BootStep::Tools,
        BootStep::Settings,
        BootStep::Library,
    ];

    fn label(self) -> &'static str {
        match self {
            BootStep::Database => "Opening the database",
            BootStep::Tools => "Looking for FFmpeg",
            BootStep::Settings => "Loading settings",
            BootStep::Library => "Starting the library",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy)]
enum BootState {
    Running(BootStep),
    Failed,
}

/// How far start-up has got, shared with the start-up page.
#[derive(Debug, Clone)]
pub struct BootProgress(Arc<Mutex<BootState>>);

impl Default for BootProgress {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(BootState::Running(BootStep::Database))))
    }
}

/// What every API call answers during start-up (with status 503).
#[derive(Debug, Serialize, PartialEq, Eq)]
struct BootReply {
    error: &'static str,
    message: &'static str,
    step: usize,
    steps: usize,
}

impl BootProgress {
    /// Records the step that is now running.
    pub fn begin(&self, step: BootStep) {
        self.set(BootState::Running(step));
    }

    /// Records that start-up failed, so the page can say so instead of waiting.
    pub fn fail(&self) {
        self.set(BootState::Failed);
    }

    fn set(&self, state: BootState) {
        if let Ok(mut current) = self.0.lock() {
            *current = state;
        }
    }

    fn reply(&self) -> BootReply {
        let state = self
            .0
            .lock()
            .map(|s| *s)
            .unwrap_or(BootState::Running(BootStep::Database));
        let steps = BootStep::ALL.len();
        match state {
            BootState::Running(step) => BootReply {
                error: "starting",
                message: step.label(),
                step: step.index(),
                steps,
            },
            BootState::Failed => BootReply {
                error: "start_failed",
                message: "Solidfin couldn't start. The console window or the log file says why.",
                step: 0,
                steps,
            },
        }
    }
}

async fn starting(State(progress): State<BootProgress>) -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, Json(progress.reply())).into_response()
}

/// Router used until start-up finishes: the web UI files, and a 503 with the
/// current step for everything else.
pub fn router(progress: BootProgress, lan_only: bool) -> Router {
    let router = web::router().fallback(starting).with_state(progress);
    if lan_only {
        router.layer(middleware::from_fn(lan::lan_only))
    } else {
        router
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_current_step_and_failure() {
        let progress = BootProgress::default();
        assert_eq!(
            progress.reply(),
            BootReply {
                error: "starting",
                message: "Opening the database",
                step: 0,
                steps: 4
            }
        );

        progress.begin(BootStep::Settings);
        let reply = progress.reply();
        assert_eq!((reply.error, reply.step), ("starting", 2));

        progress.fail();
        assert_eq!(progress.reply().error, "start_failed");
    }
}
