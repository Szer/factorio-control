#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, unused_must_use)]

pub mod api;
pub mod auth;
pub mod mod_settings;
pub mod models;
pub mod mods;
pub mod process;
pub mod save_metadata;
pub mod storage;
pub mod telemetry;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use models::*;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{process::Child, sync::Mutex};

pub type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub struct Error {
    pub status: StatusCode,
    pub message: String,
}
impl Error {
    pub fn bad(message: &str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }
    pub fn conflict(message: &str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        let status = if value.kind() == std::io::ErrorKind::NotFound {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        Self {
            status,
            message: value.to_string(),
        }
    }
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::bad(&format!("Invalid configuration JSON: {value}"))
    }
}
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ApiError {
                message: self.message,
            }),
        )
            .into_response()
    }
}
#[derive(Clone)]
pub struct App {
    pub inner: Arc<Mutex<Inner>>,
    pub sync_progress: Arc<std::sync::Mutex<Option<mods::save_sync::SyncProgress>>>,
    pub directory: PathBuf,
    pub binary: Option<PathBuf>,
    pub save_operations: Arc<Mutex<()>>,
}
pub struct Inner {
    pub status: Status,
    pub logs: VecDeque<LogEntry>,
    pub next_log_id: u64,
    pub child: Option<Child>,
    pub stopping_at: Option<std::time::Instant>,
    pub generation: u64,
    pub telemetry: telemetry::Runtime,
}
impl App {
    pub fn new(directory: PathBuf, binary: Option<PathBuf>) -> Result<Self> {
        std::fs::create_dir_all(&directory)?;
        storage::ensure_directory(&directory)?;
        let directory = directory.canonicalize()?;
        let saves = directory.join("saves");
        std::fs::create_dir_all(&saves)?;
        storage::ensure_directory(&saves)?;
        let binary_available = binary.as_ref().is_some_and(|p| p.is_file());
        Ok(Self {
            directory,
            binary,
            save_operations: Arc::new(Mutex::new(())),
            sync_progress: Arc::new(std::sync::Mutex::new(None)),
            inner: Arc::new(Mutex::new(Inner {
                status: Status {
                    state: ServerState::Stopped,
                    since: now(),
                    active_save: None,
                    started_at: None,
                    binary_available,
                    version: None,
                    error: None,
                },
                logs: VecDeque::new(),
                next_log_id: 1,
                child: None,
                stopping_at: None,
                generation: 0,
                telemetry: telemetry::Runtime::default(),
            })),
        })
    }
    pub fn settings_path(&self) -> PathBuf {
        self.directory.join("server-settings.json")
    }
    pub fn manager_settings_path(&self) -> PathBuf {
        self.directory.join("manager-settings.json")
    }
    pub fn save_selection_path(&self) -> PathBuf {
        self.directory.join("selected-save.json")
    }
    pub fn admins_path(&self) -> PathBuf {
        self.directory.join("server-adminlist.json")
    }
    pub fn saves_path(&self) -> PathBuf {
        self.directory.join("saves")
    }
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |v| v.as_secs())
}
pub fn log(inner: &mut Inner, level: LogLevel, message: String) {
    if inner.logs.len() >= 2000 {
        inner.logs.pop_front();
    }
    inner.logs.push_back(LogEntry {
        id: inner.next_log_id,
        timestamp: now(),
        level,
        message,
    });
    inner.next_log_id = inner.next_log_id.saturating_add(1);
}
