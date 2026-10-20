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
#[serde(untagged)]
pub enum AllowCommands {
    Everyone(bool),
    Restricted(CommandPermission),
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub enum CommandPermission {
    #[serde(rename = "admins-only")]
    AdminsOnly,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ServerSettings {
    #[schema(min_length = 1, max_length = 100)]
    pub name: String,
    #[schema(max_length = 2000)]
    pub description: String,
    pub tags: Vec<String>,
    #[schema(maximum = 65535)]
    pub max_players: u32,
    pub visibility: Visibility,
    pub username: String,
    pub password: String,
    pub token: String,
    pub game_password: String,
    pub require_user_verification: bool,
    pub max_upload_in_kilobytes_per_second: u32,
    pub max_upload_slots: u32,
    pub minimum_latency_in_ticks: u32,
    #[schema(minimum = 6, maximum = 240)]
    pub max_heartbeats_per_second: u32,
    pub ignore_player_limit_for_returning_players: bool,
    pub allow_commands: AllowCommands,
    #[schema(maximum = 1440)]
    pub autosave_interval: u32,
    pub autosave_slots: u32,
    pub afk_autokick_interval: u32,
    pub auto_pause: bool,
    pub auto_pause_when_players_connect: bool,
    pub only_admins_can_pause_the_game: bool,
    pub autosave_only_on_server: bool,
    pub non_blocking_saving: bool,
    pub minimum_segment_size: u32,
    pub minimum_segment_size_peer_count: u32,
    pub maximum_segment_size: u32,
    pub maximum_segment_size_peer_count: u32,
}
impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            name: "My Factorio server".into(),
            description: String::new(),
            tags: Vec::new(),
            max_players: 0,
            visibility: Visibility {
                public: false,
                lan: true,
            },
            username: String::new(),
            password: String::new(),
            token: String::new(),
            game_password: String::new(),
            require_user_verification: true,
            max_upload_in_kilobytes_per_second: 0,
            max_upload_slots: 5,
            minimum_latency_in_ticks: 0,
            max_heartbeats_per_second: 60,
            ignore_player_limit_for_returning_players: false,
            allow_commands: AllowCommands::Restricted(CommandPermission::AdminsOnly),
            autosave_interval: 10,
            autosave_slots: 5,
            afk_autokick_interval: 0,
            auto_pause: true,
            auto_pause_when_players_connect: false,
            only_admins_can_pause_the_game: true,
            autosave_only_on_server: true,
            non_blocking_saving: false,
            minimum_segment_size: 25,
            minimum_segment_size_peer_count: 20,
            maximum_segment_size: 100,
            maximum_segment_size_peer_count: 10,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RawSettingsDocument {
    pub revision: String,
    #[schema(value_type = std::collections::HashMap<String, serde_json::Value>)]
    pub settings: serde_json::Map<String, serde_json::Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagerSettings {
    pub enable_script_telemetry: bool,
    #[schema(minimum = 1, maximum = 60)]
    pub poll_interval_seconds: u32,
    #[schema(minimum = 1, maximum = 1440)]
    pub ups_history_minutes: u32,
}
impl Default for ManagerSettings {
    fn default() -> Self {
        Self {
            enable_script_telemetry: false,
            poll_interval_seconds: 2,
            ups_history_minutes: 10,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagerSettingsDocument {
    pub revision: String,
    pub settings: ManagerSettings,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveSelectionDocument {
    pub revision: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schema(required = true)]
    pub save: Option<String>,
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
