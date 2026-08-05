use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

pub(crate) const MAX_DISPLAY_NAME_CHARS: usize = 100;
pub(crate) const MAX_ACCOUNT_ID_CHARS: usize = 512;
pub(crate) const MAX_CREDENTIAL_CHARS: usize = 262_144;
const MAX_API_URL_CHARS: usize = 2_048;
const MAX_API_KEY_CHARS: usize = 65_536;
const MAX_CUSTOM_HEADERS: usize = 64;
const MAX_HEADER_NAME_CHARS: usize = 256;
const MAX_HEADER_VALUE_CHARS: usize = 8_192;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    InvalidConfig(String),
    #[error("Codex 配置在预览后发生了变化，请重新打开预览再试。")]
    StaleOperation,
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

#[derive(Debug, Clone, Default)]
pub(crate) struct TokenIdentity {
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub expires_at: Option<i64>,
}

pub(crate) fn token_local_identity(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    let mut result = String::with_capacity(30);
    result.push_str("proxy-");
    for &byte in &digest[..12] {
        result.push(char::from_digit(u32::from(byte >> 4), 16).expect("半字节必为十六进制"));
        result.push(char::from_digit(u32::from(byte & 0xF), 16).expect("半字节必为十六进制"));
    }
    result
}

pub(crate) fn token_identity(token: &str) -> Option<TokenIdentity> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let auth = claims.get("https://api.openai.com/auth");
    let profile = claims.get("https://api.openai.com/profile");
    let account_id = claims
        .get("chatgpt_account_id")
        .and_then(Value::as_str)
        .or_else(|| {
            auth.and_then(|value| value.get("chatgpt_account_id"))
                .and_then(Value::as_str)
        })
        .or_else(|| {
            claims
                .pointer("/organizations/0/id")
                .and_then(Value::as_str)
        })
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .or_else(|| {
            profile
                .and_then(|value| value.get("email"))
                .and_then(Value::as_str)
        })
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned);
    let expires_at = claims.get("exp").and_then(Value::as_i64);
    Some(TokenIdentity {
        account_id,
        email,
        expires_at,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub base_url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
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
            return Err(AppError::InvalidConfig(
                "请填写服务名称和 API 地址。".into(),
            ));
        }
        ensure_char_limit(
            &self.name,
            MAX_DISPLAY_NAME_CHARS,
            "服务名称不能超过 100 个字符。",
        )?;
        ensure_char_limit(
            &self.base_url,
            MAX_API_URL_CHARS,
            "API 地址不能超过 2,048 个字符。",
        )?;
        let url = reqwest::Url::parse(&self.base_url).map_err(|_| {
            AppError::InvalidConfig(
                "API 地址格式不正确，请填写完整的 http:// 或 https:// 地址。".into(),
            )
        })?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(AppError::InvalidConfig(
                "API 地址必须以 http:// 或 https:// 开头。".into(),
            ));
        }
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::InvalidConfig(
                "API 地址中不能包含用户名、密码、查询参数或 # 片段。".into(),
            ));
        }
        let host = url.host_str().unwrap_or_default();
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if url.scheme() != "https" && !loopback {
            return Err(AppError::InvalidConfig(
                "远程 API 必须使用 HTTPS；只有本机地址可以使用 HTTP。".into(),
            ));
        }
        self.timeout_secs = self.timeout_secs.clamp(1, 600);
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OfficialAccountSource {
    #[default]
    OpenAiOauth,
    ProxyImport,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuotaStatus {
    #[default]
    Never,
    Success,
    Unsupported,
    Unauthorized,
    RateLimited,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountQuota {
    #[serde(default)]
    pub status: QuotaStatus,
    pub data: Option<QuotaData>,
    pub fetched_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaRefreshResult {
    pub account_id: String,
    pub quota: ProviderAccountQuota,
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
            return Err(AppError::InvalidConfig("请填写密钥名称和 API Key。".into()));
        }
        ensure_char_limit(
            &self.name,
            MAX_DISPLAY_NAME_CHARS,
            "密钥名称不能超过 100 个字符。",
        )?;
        ensure_char_limit(key, MAX_API_KEY_CHARS, "API Key 不能超过 65,536 个字符。")?;
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
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
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StoredOfficialAccount {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub account_id: String,
    pub email: String,
    pub credential: CodexAuthCredential,
    #[serde(default)]
    pub source: OfficialAccountSource,
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub quota: ProviderAccountQuota,
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

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OfficialAccountView {
    pub id: String,
    pub name: String,
    pub account_id: String,
    pub email: String,
    pub source: OfficialAccountSource,
    pub expires_at: Option<i64>,
    pub quota: ProviderAccountQuota,
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
            source: self.source,
            expires_at: self.expires_at,
            quota: self.quota.clone(),
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default)]
    pub app: AppPreferences,
    #[serde(default)]
    pub codex: CodexPreferences,
    #[serde(default)]
    pub active: ActiveState,
    #[serde(default)]
    pub providers: Vec<StoredProvider>,
    #[serde(default)]
    pub official_accounts: Vec<StoredOfficialAccount>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub window_seconds: Option<i64>,
    pub reset_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum QuotaData {
    Windowed {
        primary: Option<QuotaWindow>,
        secondary: Option<QuotaWindow>,
    },
}

#[derive(Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub provider_count: usize,
    pub active_provider: Option<String>,
    pub active_kind: ActiveKind,
    pub active_account_id: Option<String>,
    pub active_account: Option<String>,
    pub active_quota: Option<ProviderAccountQuota>,
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
pub struct RepairTarget {
    pub id: String,
    pub sources: Vec<String>,
    pub current: bool,
    pub count: usize,
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
    pub api_key_masked: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInspection {
    pub path: String,
    pub valid: bool,
    pub active_provider: Option<String>,
    pub managed_provider_present: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsOverview {
    pub inspection: ConfigInspection,
    pub diagnostics: serde_json::Value,
    pub can_preview_custom: bool,
}

fn validate_headers(headers: &BTreeMap<String, String>) -> Result<(), AppError> {
    if headers.len() > MAX_CUSTOM_HEADERS {
        return Err(AppError::InvalidConfig(
            "自定义请求头不能超过 64 项。".into(),
        ));
    }
    for (name, value) in headers {
        ensure_char_limit(
            name,
            MAX_HEADER_NAME_CHARS,
            "自定义请求头名称不能超过 256 个字符。",
        )?;
        ensure_char_limit(
            value,
            MAX_HEADER_VALUE_CHARS,
            "单个自定义请求头的值不能超过 8,192 个字符。",
        )?;
        reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| AppError::InvalidConfig(format!("自定义请求头名称无效：{name}。")))?;
        reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| AppError::InvalidConfig(format!("自定义请求头的值无效：{name}。")))?;
    }
    Ok(())
}

pub(crate) fn ensure_char_limit(
    value: &str,
    limit: usize,
    message: &'static str,
) -> Result<(), AppError> {
    if value.chars().count() > limit {
        return Err(AppError::InvalidConfig(message.into()));
    }
    Ok(())
}

pub(crate) fn is_personal_access_token_credential(credential: &CodexAuthCredential) -> bool {
    credential.tokens.id_token.trim().is_empty()
        || credential.tokens.refresh_token.trim().is_empty()
}

pub(crate) fn validate_official_credential(
    credential: &CodexAuthCredential,
) -> Result<(), AppError> {
    let tokens = &credential.tokens;
    let personal_access_token = is_personal_access_token_credential(credential);
    if credential.auth_mode != "chatgpt"
        || tokens.access_token.trim().is_empty()
        || tokens.account_id.trim().is_empty()
        || (!personal_access_token
            && (tokens.id_token.trim().is_empty()
                || tokens.refresh_token.trim().is_empty()
                || credential.last_refresh.trim().is_empty()))
    {
        return Err(AppError::InvalidConfig(
            "OpenAI 登录信息不完整，请重新登录。".into(),
        ));
    }
    for token in [
        tokens.id_token.as_str(),
        tokens.access_token.as_str(),
        tokens.refresh_token.as_str(),
    ] {
        ensure_char_limit(
            token,
            MAX_CREDENTIAL_CHARS,
            "OpenAI 登录凭据过长，请重新登录或导入有效的 Cookie。",
        )?;
    }
    ensure_char_limit(
        &tokens.account_id,
        MAX_ACCOUNT_ID_CHARS,
        "OpenAI 账号标识不能超过 512 个字符。",
    )?;
    Ok(())
}

fn redact_header_values(headers: &mut BTreeMap<String, String>) {
    for value in headers.values_mut() {
        value.clear();
    }
}

fn default_true() -> bool {
    true
}
fn default_timeout_secs() -> u64 {
    30
}
fn default_language() -> String {
    "zh-CN".into()
}
fn default_theme() -> String {
    "system".into()
}
#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn provider_and_api_key_inputs_have_size_limits() {
        let mut provider = ProviderProfile {
            id: String::new(),
            name: "x".repeat(MAX_DISPLAY_NAME_CHARS + 1),
            base_url: "https://example.test/v1".into(),
            headers: BTreeMap::new(),
            timeout_secs: 30,
            enabled: true,
            active: false,
            active_account_id: None,
            account_count: 0,
        };
        assert!(provider.normalize_and_validate().is_err());

        let mut account = ProviderAccount {
            id: String::new(),
            provider_id: Some("provider".into()),
            name: "key".into(),
            auth_kind: AccountAuthKind::ApiKey,
            api_key: Some("x".repeat(MAX_API_KEY_CHARS + 1)),
            headers: BTreeMap::new(),
            active: false,
            email: None,
            created_at: 0,
            updated_at: 0,
        };
        assert!(account.normalize_and_validate().is_err());
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
            source: OfficialAccountSource::OpenAiOauth,
            expires_at: Some(1_800_000_000),
            quota: ProviderAccountQuota::default(),
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

    fn credential() -> CodexAuthCredential {
        CodexAuthCredential {
            auth_mode: "chatgpt".into(),
            openai_api_key: None,
            tokens: CodexAuthTokens {
                id_token: "id-secret".into(),
                access_token: "access-secret".into(),
                refresh_token: "refresh-secret".into(),
                account_id: "workspace-account".into(),
            },
            last_refresh: "2026-07-14T00:00:00Z".into(),
        }
    }

    #[test]
    fn token_local_identity_renders_stable_lowercase_hex_suffix() {
        let first = token_local_identity("secret-token");
        assert!(first.starts_with("proxy-"));
        assert_eq!(first.len(), "proxy-".len() + 24);
        assert!(first[6..].chars().all(|c| c.is_ascii_hexdigit()));
        assert!(first[6..].chars().all(|c| !c.is_ascii_uppercase()));
        assert_eq!(first, token_local_identity("secret-token"));
        assert_ne!(first, token_local_identity("other-token"));
    }

    #[test]
    fn validate_official_credential_accepts_complete_oauth_login() {
        assert!(validate_official_credential(&credential()).is_ok());
    }

    #[test]
    fn validate_official_credential_rejects_incomplete_login() {
        let mut missing_access = credential();
        missing_access.tokens.access_token.clear();
        assert!(validate_official_credential(&missing_access).is_err());

        let mut missing_account = credential();
        missing_account.tokens.account_id.clear();
        assert!(validate_official_credential(&missing_account).is_err());

        let mut missing_refresh = credential();
        missing_refresh.tokens.refresh_token.clear();
        assert!(is_personal_access_token_credential(&missing_refresh));
        assert!(validate_official_credential(&missing_refresh).is_ok());

        let mut missing_last_refresh = credential();
        missing_last_refresh.last_refresh.clear();
        assert!(!is_personal_access_token_credential(&missing_last_refresh));
        assert!(validate_official_credential(&missing_last_refresh).is_err());

        let mut wrong_mode = credential();
        wrong_mode.auth_mode = "azure".into();
        assert!(validate_official_credential(&wrong_mode).is_err());
    }

    #[test]
    fn validate_official_credential_accepts_personal_access_token_without_session_tokens() {
        let credential = CodexAuthCredential {
            tokens: CodexAuthTokens {
                id_token: String::new(),
                access_token: "pat-secret".into(),
                refresh_token: String::new(),
                account_id: "workspace-account".into(),
            },
            last_refresh: String::new(),
            ..credential()
        };
        assert!(is_personal_access_token_credential(&credential));
        assert!(validate_official_credential(&credential).is_ok());
    }

    #[test]
    fn validate_official_credential_rejects_oversized_tokens() {
        let mut oversized = credential();
        oversized.tokens.id_token = "x".repeat(MAX_CREDENTIAL_CHARS + 1);
        let error = validate_official_credential(&oversized).unwrap_err();
        assert!(error.to_string().contains("凭据过长"));

        let mut oversized_account = credential();
        oversized_account.tokens.account_id = "a".repeat(MAX_ACCOUNT_ID_CHARS + 1);
        let error = validate_official_credential(&oversized_account).unwrap_err();
        assert!(error.to_string().contains("不能超过 512 个字符"));
    }
}
