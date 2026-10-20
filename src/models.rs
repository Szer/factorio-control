use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Status {
    pub state: ServerState,
    #[schema(required = true)]
    pub active_save: Option<String>,
    #[schema(required = true)]
    pub started_at: Option<u64>,
    pub binary_available: bool,
    #[schema(required = true)]
    pub version: Option<String>,
    #[schema(required = true)]
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct Save {
    pub name: String,
    pub size_bytes: u64,
    pub modified_at: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Visibility {
    pub public: bool,
    pub lan: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ServerSettings {
    #[schema(min_length = 1, max_length = 100)]
    pub name: String,
    #[schema(max_length = 2000)]
    pub description: String,
    #[schema(maximum = 65535)]
    pub max_players: u32,
    pub auto_pause: bool,
    #[schema(maximum = 1440)]
    pub autosave_interval: u32,
    pub visibility: Visibility,
}
impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            name: "My Factorio server".into(),
            description: String::new(),
            max_players: 0,
            auto_pause: true,
            autosave_interval: 10,
            visibility: Visibility {
                public: false,
                lan: true,
            },
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsDocument {
    pub revision: String,
    pub settings: ServerSettings,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminsDocument {
    pub revision: String,
    pub admins: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LogEntry {
    pub id: u64,
    pub timestamp: u64,
    pub level: LogLevel,
    pub message: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub save: String,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ApiError {
    pub message: String,
}
