use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("配置无效：{0}")]
    InvalidConfig(String),
    #[error("配置文件已在预览后发生变化，请重新预览")]
    StaleOperation,
    #[error("本地代理错误：{0}")]
    Proxy(String),
    #[error("{0}")]
    Internal(String),
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self::Internal(value.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::Internal(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    #[default]
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexChatReasoningConfig {
    pub supports_thinking: Option<bool>,
    pub supports_effort: Option<bool>,
    pub thinking_param: Option<String>,
    pub effort_param: Option<String>,
    pub effort_value_mode: Option<String>,
    pub output_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FetchedModel {
    pub id: String,
    #[serde(default, alias = "owned_by")]
    pub owned_by: Option<String>,
    #[serde(flatten)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub protocol: ProviderProtocol,
    pub base_url: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model_metadata: Vec<FetchedModel>,
    #[serde(default)]
    pub model_aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub codex_chat_reasoning: Option<CodexChatReasoningConfig>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    pub context_window: Option<u64>,
    pub auto_compact_threshold: Option<u64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub active: bool,
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub account_count: u64,
}

impl ProviderProfile {
    pub fn normalize_and_validate(&mut self) -> Result<(), AppError> {
        self.name = self.name.trim().to_owned();
        self.base_url = self.base_url.trim().trim_end_matches('/').to_owned();
        if self.name.is_empty() || self.base_url.is_empty() {
            return Err(AppError::InvalidConfig("名称和 Base URL 不能为空".into()));
        }
        let url = reqwest::Url::parse(&self.base_url)
            .map_err(|_| AppError::InvalidConfig("Base URL 必须是 HTTP(S) URL".into()))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(AppError::InvalidConfig(
                "Base URL 必须是 HTTP(S) URL".into(),
            ));
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::InvalidConfig(
                "Base URL 不能包含凭据、查询参数或片段".into(),
            ));
        }
        let host = url.host_str().unwrap_or_default();
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if url.scheme() != "https" && !loopback {
            return Err(AppError::InvalidConfig(
                "远程上游必须使用 HTTPS；HTTP 仅允许回环地址".into(),
            ));
        }
        self.timeout_secs = self.timeout_secs.clamp(1, 600);
        self.models = dedupe(self.models.iter().map(String::as_str));
        self.model_metadata
            .retain(|value| self.models.iter().any(|model| model == &value.id));
        validate_headers(&self.headers)?;
        Ok(())
    }

    pub fn redacted(mut self) -> Self {
        redact_header_values(&mut self.headers);
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountAuthKind {
    #[default]
    ApiKey,
    OfficialOauth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccount {
    #[serde(default)]
    pub id: String,
    pub provider_id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub auth_kind: AccountAuthKind,
    pub api_key: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub active: bool,
    pub email: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

impl ProviderAccount {
    pub fn normalize_and_validate(&mut self) -> Result<(), AppError> {
        self.name = self.name.trim().to_owned();
        let key = self.api_key.as_deref().unwrap_or_default().trim();
        if self.name.is_empty() || key.is_empty() {
            return Err(AppError::InvalidConfig(
                "账号名称和 API Key 不能为空".into(),
            ));
        }
        self.api_key = Some(key.to_owned());
        validate_headers(&self.headers)
    }

    pub fn redacted(mut self) -> Self {
        self.api_key = None;
        redact_header_values(&mut self.headers);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredProvider {
    #[serde(flatten)]
    pub profile: ProviderProfile,
    #[serde(default)]
    pub accounts: Vec<ProviderAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActiveKind {
    #[default]
    None,
    Provider,
    Official,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActiveState {
    pub kind: ActiveKind,
    pub provider_id: Option<String>,
    pub account_id: Option<String>,
}

/// Token payload written to Codex's `auth.json` when an official account is
/// activated. This type is deliberately never exposed by a Tauri command.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct CodexAuthTokens {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: String,
}

impl fmt::Debug for CodexAuthTokens {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexAuthTokens")
            .field("id_token", &"[REDACTED]")
            .field("access_token", &"[REDACTED]")
            .field("refresh_token", &"[REDACTED]")
            .field("account_id", &self.account_id)
            .finish()
    }
}

/// Exact credential object expected by Codex in `auth.json`.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct CodexAuthCredential {
    pub auth_mode: String,
    #[serde(default, rename = "OPENAI_API_KEY")]
    pub openai_api_key: Option<String>,
    pub tokens: CodexAuthTokens,
    pub last_refresh: String,
}

impl fmt::Debug for CodexAuthCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CodexAuthCredential")
            .field("auth_mode", &self.auth_mode)
            .field("OPENAI_API_KEY", &"[REDACTED]")
            .field("tokens", &"[REDACTED]")
            .field("last_refresh", &self.last_refresh)
            .finish()
    }
}

/// Sensitive official-account record. It is serialized only as part of
/// `app.yaml`; public commands must return [`OfficialAccountView`] instead.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredOfficialAccount {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub account_id: String,
    pub email: String,
    pub credential: CodexAuthCredential,
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

impl fmt::Debug for StoredOfficialAccount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredOfficialAccount")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("account_id", &self.account_id)
            .field("email", &self.email)
            .field("credential", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OfficialAccountView {
    pub id: String,
    pub name: String,
    pub account_id: String,
    pub email: String,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOverview {
    pub providers: Vec<ProviderProfile>,
    pub accounts: Vec<ProviderAccount>,
    pub official_accounts: Vec<OfficialAccountView>,
}

impl StoredOfficialAccount {
    pub fn view(&self, active: bool) -> OfficialAccountView {
        OfficialAccountView {
            id: self.id.clone(),
            name: self.name.clone(),
            account_id: self.account_id.clone(),
            email: self.email.clone(),
            expires_at: self.expires_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
            active,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiDeviceAuthorization {
    pub operation_id: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_at: i64,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OpenAiDevicePoll {
    Pending,
    Expired,
    Complete {
        account: Box<OfficialAccountView>,
        repair: RepairResult,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "loopback")]
    pub listen_address: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_request_timeout")]
    pub request_timeout_ms: u64,
    #[serde(default = "default_request_max_retries")]
    pub request_max_retries: u32,
    #[serde(default = "default_stream_max_retries")]
    pub stream_max_retries: u32,
    #[serde(default = "default_concurrency")]
    pub max_concurrent_requests: usize,
}

impl Default for RouteSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_address: loopback(),
            port: default_port(),
            request_timeout_ms: default_request_timeout(),
            request_max_retries: default_request_max_retries(),
            stream_max_retries: default_stream_max_retries(),
            max_concurrent_requests: default_concurrency(),
        }
    }
}

impl RouteSettings {
    pub fn normalize(&mut self) {
        self.listen_address = loopback();
        self.port = default_port();
        self.request_timeout_ms = self.request_timeout_ms.clamp(1_000, 600_000);
        self.request_max_retries = self.request_max_retries.min(100);
        self.stream_max_retries = self.stream_max_retries.min(100);
        self.max_concurrent_requests = self.max_concurrent_requests.clamp(1, 256);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPreferences {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_true")]
    pub close_to_tray: bool,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            language: default_language(),
            theme: default_theme(),
            close_to_tray: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CodexPreferences {
    #[serde(default)]
    pub home: String,
    #[serde(default)]
    pub managed_original: ManagedCodexFields,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ManagedCodexFields {
    #[serde(default)]
    pub captured: bool,
    pub model: Option<String>,
    pub model_provider: Option<String>,
    pub model_catalog_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default = "config_version")]
    pub version: u32,
    #[serde(default)]
    pub app: AppPreferences,
    #[serde(default)]
    pub codex: CodexPreferences,
    #[serde(default)]
    pub route: RouteSettings,
    #[serde(default)]
    pub active: ActiveState,
    #[serde(default)]
    pub providers: Vec<StoredProvider>,
    #[serde(default)]
    pub official_accounts: Vec<StoredOfficialAccount>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: config_version(),
            app: AppPreferences::default(),
            codex: CodexPreferences::default(),
            route: RouteSettings::default(),
            active: ActiveState::default(),
            providers: vec![],
            official_accounts: vec![],
        }
    }
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

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub provider_count: usize,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageResult<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
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
    pub settings: RouteSettings,
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
    pub log_total: usize,
    pub log_page: usize,
    pub log_page_size: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairTarget {
    pub id: String,
    pub sources: Vec<String>,
    pub current: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseScan {
    pub path: String,
    pub schema: String,
    pub thread_count: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairScan {
    pub current_provider: String,
    pub targets: Vec<RepairTarget>,
    pub rollout_files: usize,
    pub session_meta_count: usize,
    pub databases: Vec<DatabaseScan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RepairResult {
    pub target_provider: String,
    pub files_scanned: usize,
    pub files_modified: usize,
    pub files_skipped: usize,
    pub files_failed: usize,
    pub session_meta_updated: usize,
    pub rows_updated: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPatchPreview {
    pub operation_id: String,
    pub target_path: String,
    pub base_hash: String,
    pub rendered: String,
    pub changes: Vec<String>,
    pub compatibility_token_masked: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInspection {
    pub path: String,
    pub valid: bool,
    pub active_provider: Option<String>,
    pub managed_provider_present: bool,
    pub model_catalog_path: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsOverview {
    pub inspection: ConfigInspection,
    pub diagnostics: serde_json::Value,
}

fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), AppError> {
    for (name, value) in headers {
        reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| AppError::InvalidConfig(format!("Header 名称无效：{name}")))?;
        reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| AppError::InvalidConfig(format!("Header 值无效：{name}")))?;
    }
    Ok(())
}

fn redact_header_values(headers: &mut BTreeMap<String, String>) {
    for value in headers.values_mut() {
        value.clear();
    }
}

fn dedupe<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .map(str::trim)
        .filter(|value| !value.is_empty() && seen.insert((*value).to_owned()))
        .map(str::to_owned)
        .collect()
}

fn default_true() -> bool {
    true
}
fn default_timeout_secs() -> u64 {
    30
}
fn loopback() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    16_384
}
fn default_request_timeout() -> u64 {
    300_000
}
fn default_request_max_retries() -> u32 {
    4
}
fn default_stream_max_retries() -> u32 {
    3
}
fn default_concurrency() -> usize {
    64
}
fn default_language() -> String {
    "zh-CN".into()
}
fn default_theme() -> String {
    "system".into()
}
fn config_version() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_is_always_loopback_and_fixed_port() {
        let mut route = RouteSettings {
            enabled: true,
            listen_address: "0.0.0.0".into(),
            port: 9999,
            request_timeout_ms: 1,
            request_max_retries: 101,
            stream_max_retries: 102,
            max_concurrent_requests: 1000,
        };
        route.normalize();
        assert_eq!(route.listen_address, "127.0.0.1");
        assert_eq!(route.port, 16_384);
        assert_eq!(route.request_timeout_ms, 1_000);
        assert_eq!(route.request_max_retries, 100);
        assert_eq!(route.stream_max_retries, 100);
        assert_eq!(route.max_concurrent_requests, 256);
    }

    #[test]
    fn legacy_route_settings_get_retry_defaults() {
        let route = serde_json::from_value::<RouteSettings>(serde_json::json!({
            "enabled": true,
            "listenAddress": "127.0.0.1",
            "port": 16384,
            "requestTimeoutMs": 300000,
            "maxConcurrentRequests": 64
        }))
        .unwrap();

        assert_eq!(route.request_max_retries, 4);
        assert_eq!(route.stream_max_retries, 3);
    }

    #[test]
    fn redacted_account_never_serializes_api_key() {
        let account = ProviderAccount {
            id: "account".into(),
            provider_id: Some("provider".into()),
            name: "default".into(),
            auth_kind: AccountAuthKind::ApiKey,
            api_key: Some("upstream-secret".into()),
            headers: BTreeMap::from([("x-private-token".into(), "header-secret".into())]),
            active: false,
            email: None,
            created_at: 0,
            updated_at: 0,
        }
        .redacted();
        let serialized = serde_json::to_string(&account).unwrap();
        assert!(!serialized.contains("upstream-secret"));
        assert!(!serialized.contains("header-secret"));
        assert_eq!(account.api_key, None);
    }

    #[test]
    fn provider_url_requires_https_except_for_loopback() {
        let make = |base_url: &str| {
            serde_json::from_value::<ProviderProfile>(serde_json::json!({
                "name": "provider",
                "baseUrl": base_url,
                "contextWindow": null,
                "autoCompactThreshold": null,
                "activeAccountId": null
            }))
            .unwrap()
        };

        assert!(
            make("http://api.example.test/v1")
                .normalize_and_validate()
                .is_err()
        );
        assert!(
            make("https://api.example.test/v1?token=secret")
                .normalize_and_validate()
                .is_err()
        );
        assert!(
            make("http://127.0.0.1:9000/v1")
                .normalize_and_validate()
                .is_ok()
        );
    }

    fn official_account() -> StoredOfficialAccount {
        StoredOfficialAccount {
            id: "local-account".into(),
            name: "OpenAI".into(),
            account_id: "workspace-account".into(),
            email: "person@example.test".into(),
            credential: CodexAuthCredential {
                auth_mode: "chatgpt".into(),
                openai_api_key: None,
                tokens: CodexAuthTokens {
                    id_token: "secret-id-token".into(),
                    access_token: "secret-access-token".into(),
                    refresh_token: "secret-refresh-token".into(),
                    account_id: "workspace-account".into(),
                },
                last_refresh: "2026-07-14T00:00:00Z".into(),
            },
            expires_at: Some(1_800_000_000),
            created_at: 1,
            updated_at: 2,
        }
    }

    #[test]
    fn official_view_and_device_dtos_never_serialize_credentials() {
        let view = official_account().view(true);
        let repair = RepairResult::default();
        let values = [
            serde_json::to_value(&view).unwrap(),
            serde_json::to_value(OpenAiDeviceAuthorization {
                operation_id: "operation".into(),
                user_code: "ABCD-EFGH".into(),
                verification_uri: "https://auth.openai.com/device".into(),
                expires_at: 1_800_000_000,
                interval_secs: 5,
            })
            .unwrap(),
            serde_json::to_value(OpenAiDevicePoll::Complete {
                account: Box::new(view),
                repair,
            })
            .unwrap(),
        ];

        for value in values {
            let serialized = value.to_string();
            assert!(!serialized.contains("secret-id-token"));
            assert!(!serialized.contains("secret-access-token"));
            assert!(!serialized.contains("secret-refresh-token"));
            assert!(!serialized.contains("credential"));
            assert!(!serialized.contains("idToken"));
            assert!(!serialized.contains("accessToken"));
            assert!(!serialized.contains("refreshToken"));
            assert!(!serialized.contains("id_token"));
            assert!(!serialized.contains("access_token"));
            assert!(!serialized.contains("refresh_token"));
            assert!(!serialized.contains("device_auth_id"));
        }
    }

    #[test]
    fn credential_serialization_matches_codex_auth_json_shape() {
        let credential = official_account().credential;
        let value = serde_json::to_value(credential).unwrap();
        assert_eq!(value["auth_mode"], "chatgpt");
        assert!(value["OPENAI_API_KEY"].is_null());
        assert_eq!(value["tokens"]["account_id"], "workspace-account");
        assert_eq!(value["last_refresh"], "2026-07-14T00:00:00Z");
        assert!(value.get("authMode").is_none());
        assert!(value["tokens"].get("accessToken").is_none());
    }

    #[test]
    fn sensitive_account_debug_output_is_redacted() {
        let rendered = format!("{:?}", official_account());
        assert!(!rendered.contains("secret-id-token"));
        assert!(!rendered.contains("secret-access-token"));
        assert!(!rendered.contains("secret-refresh-token"));
        assert!(rendered.contains("[REDACTED]"));
    }
}
