use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub protocol: ProviderProtocol,
    pub base_url: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub model_metadata: Vec<FetchedModel>,
    #[serde(default)]
    pub codex_chat_reasoning: Option<CodexChatReasoningConfig>,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexChatReasoningConfig {
    #[serde(default)]
    pub supports_thinking: Option<bool>,
    #[serde(default)]
    pub supports_effort: Option<bool>,
    #[serde(default)]
    pub thinking_param: Option<String>,
    #[serde(default)]
    pub effort_param: Option<String>,
    #[serde(default)]
    pub effort_value_mode: Option<String>,
    #[serde(default)]
    pub output_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FetchedModel {
    pub id: String,
    #[serde(default)]
    pub owned_by: Option<String>,
    #[serde(flatten)]
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

fn default_timeout() -> u64 {
    30
}

fn default_true() -> bool {
    true
}

impl ProviderProfile {
    pub fn normalize_and_validate(&mut self) -> Result<(), AppError> {
        self.name = self.name.trim().to_owned();
        self.base_url = self.base_url.trim().trim_end_matches('/').to_owned();
        if self.name.is_empty() || self.base_url.is_empty() {
            return Err(AppError::InvalidConfig("名称和 Base URL 不能为空".into()));
        }
        let url = reqwest::Url::parse(&self.base_url)
            .map_err(|_| AppError::InvalidConfig("Base URL 必须是有效的 HTTP(S) URL".into()))?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(AppError::InvalidConfig(
                "Base URL 只支持有效的 HTTP(S) 地址".into(),
            ));
        }
        validate_headers(&self.headers)?;
        self.timeout_secs = self.timeout_secs.clamp(1, 600);
        let mut seen = std::collections::HashSet::new();
        self.models = self
            .models
            .iter()
            .map(|model| model.trim())
            .filter(|model| !model.is_empty() && seen.insert((*model).to_owned()))
            .map(str::to_owned)
            .collect();
        self.model_metadata
            .retain(|metadata| self.models.iter().any(|model| model == metadata.id.trim()));
        Ok(())
    }
}

impl ProviderAccount {
    pub fn normalize_and_validate(&mut self) -> Result<(), AppError> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() {
            return Err(AppError::InvalidConfig("账号名称不能为空".into()));
        }
        validate_headers(&self.headers)?;
        if self.auth_kind == AccountAuthKind::ApiKey {
            let key = self.api_key.as_deref().unwrap_or_default().trim();
            if key.is_empty() {
                return Err(AppError::InvalidConfig("API Key 不能为空".into()));
            }
            self.api_key = Some(key.to_owned());
        }
        Ok(())
    }
}

fn validate_headers(headers: &serde_json::Value) -> Result<(), AppError> {
    let object = headers
        .as_object()
        .ok_or_else(|| AppError::InvalidConfig("Headers 必须是字符串键值对象".into()))?;
    for (name, value) in object {
        reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| AppError::InvalidConfig(format!("无效的 Header 名称：{name}")))?;
        let value = value
            .as_str()
            .ok_or_else(|| AppError::InvalidConfig(format!("Header {name} 的值必须是字符串")))?;
        reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| AppError::InvalidConfig(format!("Header {name} 包含无效字符")))?;
    }
    Ok(())
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageResult<T> {
    pub items: Vec<T>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AuthService {
    OpenAi,
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
    Complete { account: Box<AuthAccount> },
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteSettings {
    pub enabled: bool,
    pub listen_address: String,
    pub port: u16,
}

impl Default for RouteSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_address: "127.0.0.1".into(),
            port: 0,
        }
    }
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

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("配置无效：{0}")]
    InvalidConfig(String),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn provider() -> ProviderProfile {
        ProviderProfile {
            id: "provider".into(),
            name: "  Example  ".into(),
            protocol: ProviderProtocol::Responses,
            base_url: " https://example.test/v1/ ".into(),
            models: vec![" model-a ".into(), "model-a".into(), "".into()],
            model_metadata: vec![
                FetchedModel {
                    id: "model-a".into(),
                    owned_by: None,
                    metadata: Default::default(),
                },
                FetchedModel {
                    id: "removed".into(),
                    owned_by: None,
                    metadata: Default::default(),
                },
            ],
            codex_chat_reasoning: None,
            headers: json!({"x-provider":"value"}),
            timeout_secs: 0,
            context_window: None,
            auto_compact_threshold: None,
            enabled: true,
            active: false,
            active_account_id: None,
            account_count: 0,
        }
    }

    #[test]
    fn provider_validation_normalizes_network_configuration() {
        let mut provider = provider();
        provider.normalize_and_validate().unwrap();
        assert_eq!(provider.name, "Example");
        assert_eq!(provider.base_url, "https://example.test/v1");
        assert_eq!(provider.models, vec!["model-a"]);
        assert_eq!(provider.model_metadata.len(), 1);
        assert_eq!(provider.timeout_secs, 1);
    }

    #[test]
    fn provider_validation_rejects_non_http_urls_and_invalid_headers() {
        let mut invalid_url = provider();
        invalid_url.base_url = "file:///secret".into();
        assert!(matches!(
            invalid_url.normalize_and_validate(),
            Err(AppError::InvalidConfig(_))
        ));

        let mut invalid_headers = provider();
        invalid_headers.headers = json!({"authorization":["not", "a string"]});
        assert!(matches!(
            invalid_headers.normalize_and_validate(),
            Err(AppError::InvalidConfig(_))
        ));
    }
}
