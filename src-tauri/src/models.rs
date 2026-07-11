use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub headers: serde_json::Value,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub auto_compact_threshold: Option<u64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub account_count: u64,
}

fn default_timeout() -> u64 {
    30
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    Responses,
    ChatCompletions,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AccountAuthKind {
    ApiKey,
    OfficialOauth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccount {
    pub id: String,
    pub provider_id: Option<String>,
    pub name: String,
    pub auth_kind: AccountAuthKind,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub auth_json: Option<serde_json::Value>,
    #[serde(default)]
    pub headers: serde_json::Value,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub ok: bool,
    pub status: u16,
    pub endpoint: String,
    pub message: String,
    pub suggest_v1: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    pub running: bool,
    pub base_url: Option<String>,
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub provider_count: u64,
    pub active_provider: Option<String>,
    pub codex_home: String,
    pub database_count: usize,
    pub session_count: usize,
    pub database_health: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub identity: String,
    pub id: String,
    pub title: String,
    pub provider: String,
    pub cwd: String,
    pub archived: bool,
    pub updated_at: i64,
    pub source_db: String,
    pub source_rollout: Option<String>,
    pub original_provider: String,
    pub has_user_event: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthService {
    OpenAi,
    GitHub,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthAccount {
    pub id: String,
    pub service: AuthService,
    pub name: String,
    pub login: Option<String>,
    pub email: Option<String>,
    #[serde(default)]
    pub credential: Option<serde_json::Value>,
    #[serde(default)]
    pub config_snapshot: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubDeviceAuthorization {
    pub operation_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: i64,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteLogEntry {
    pub id: u64,
    pub timestamp: i64,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub latency_ms: u64,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RouteConsoleSnapshot {
    pub running: bool,
    pub base_url: Option<String>,
    pub upstream_url: Option<String>,
    pub provider_name: Option<String>,
    pub account_name: Option<String>,
    pub model: Option<String>,
    pub started_at: Option<i64>,
    pub request_count: u64,
    pub success_count: u64,
    pub error_count: u64,
    pub active_requests: u64,
    pub last_latency_ms: Option<u64>,
    pub logs: Vec<RouteLogEntry>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairScan {
    pub operation_id: String,
    pub databases: Vec<DatabaseScan>,
    pub rollout_files: usize,
    pub can_repair: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseScan {
    pub path: String,
    pub health: String,
    pub known_schema: bool,
    pub thread_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairResult {
    pub backup_path: String,
    pub databases_repaired: usize,
    pub rows_updated: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticReport {
    pub app_db: String,
    pub codex_home: String,
    pub config_exists: bool,
    pub auth_exists: bool,
    pub databases: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("配置无效：{0}")]
    InvalidConfig(String),
    #[error("Codex 正在运行，请先退出后再修复数据库")]
    #[allow(dead_code)]
    CodexRunning,
    #[error("未知数据库结构：{0}")]
    UnknownSchema(String),
    #[error("备份失败：{0}")]
    Backup(String),
    #[error("代理失败：{0}")]
    Proxy(String),
    #[error("官方登录信息不存在")]
    OfficialAuthMissing,
    #[error("操作已过期，请重新预览")]
    StaleOperation,
    #[error("内部错误：{0}")]
    Internal(String),
}

impl serde::Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(error: anyhow::Error) -> Self {
        Self::Internal(error.to_string())
    }
}
