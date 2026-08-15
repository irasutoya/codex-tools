use crate::models::{
    AppError, CodexAuthCredential, CodexAuthTokens, MAX_ACCOUNT_ID_CHARS, MAX_CREDENTIAL_CHARS,
    MAX_DISPLAY_NAME_CHARS, OfficialAccountSource, OpenAiDeviceAuthorization, ProviderAccountQuota,
    StoredOfficialAccount, ensure_char_limit, token_identity, token_local_identity,
};
use crate::proxy_import::ImportedProxyCredential;
use crate::storage::Store;
use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_START_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_POLL_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const CODEX_ORIGINATOR: &str = "codex_cli_rs";
const DEFAULT_DEVICE_LIFETIME_SECS: u64 = 15 * 60;
const MAX_DEVICE_LIFETIME_SECS: u64 = 60 * 60;
const MAX_OAUTH_RESPONSE_BYTES: usize = 64 * 1024;
const ACCESS_TOKEN_REFRESH_WINDOW_SECS: i64 = 5 * 60;

struct PendingDeviceAuth {
    device_auth_id: String,
    user_code: String,
    deadline: Instant,
    expires_at: i64,
    interval: Duration,
    next_poll_at: Instant,
}

struct PendingPoll {
    device_auth_id: String,
    user_code: String,
}

enum LocalPollState {
    Ready(PendingPoll),
    Pending,
    Expired,
}

/// Internal result of device polling. It intentionally does not implement
/// `Serialize`: credentials must be persisted before a redacted view reaches
/// a Tauri command response.
#[derive(Debug)]
pub(crate) enum DevicePollResult {
    Pending,
    Expired,
    Complete(Box<StoredOfficialAccount>),
}

pub struct AuthCenter {
    client: crate::network::ClientCache,
    pending: Mutex<HashMap<String, PendingDeviceAuth>>,
    refresh_lock: Mutex<()>,
}

#[derive(Deserialize)]
struct DeviceStartResponse {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: String,
    #[serde(default)]
    interval: Option<Value>,
    #[serde(default)]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct DevicePollResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

struct CompleteTokens {
    id_token: String,
    access_token: String,
    refresh_token: String,
    expires_in: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountRefreshDecision {
    KeepCurrent,
    Refresh,
}

impl Default for AuthCenter {
    fn default() -> Self {
        Self {
            client: crate::network::ClientCache::default(),
            pending: Mutex::new(HashMap::new()),
            refresh_lock: Mutex::new(()),
        }
    }
}

impl AuthCenter {
    pub async fn start_openai(&self) -> Result<OpenAiDeviceAuthorization, AppError> {
        let http = self.client()?;
        let response = http
            .post(DEVICE_START_URL)
            .json(&json!({ "client_id": CODEX_CLIENT_ID }))
            .send()
            .await
            .map_err(safe_network_error)?;
        let response = require_success(response, "无法向 OpenAI 申请授权码")?;
        let payload: DeviceStartResponse = read_json_bounded(response, "授权码").await?;

        if payload.device_auth_id.trim().is_empty() || payload.user_code.trim().is_empty() {
            return Err(AppError::InvalidConfig(
                "OpenAI 未返回有效的授权码，请重新获取。".into(),
            ));
        }

        let interval_secs = parse_interval(payload.interval.as_ref());
        let lifetime_secs = payload
            .expires_in
            .unwrap_or(DEFAULT_DEVICE_LIFETIME_SECS)
            .clamp(30, MAX_DEVICE_LIFETIME_SECS);
        let now = chrono::Utc::now().timestamp();
        let expires_at = now.saturating_add(lifetime_secs as i64);
        let operation_id = uuid::Uuid::new_v4().to_string();
        let instant_now = Instant::now();

        let mut pending = self.pending.lock().await;
        // The desktop UI supports one login at a time. Replacing an abandoned
        // operation keeps device credentials short-lived when the WebView is closed.
        pending.clear();
        pending.insert(
            operation_id.clone(),
            PendingDeviceAuth {
                device_auth_id: payload.device_auth_id,
                user_code: payload.user_code.clone(),
                deadline: instant_now + Duration::from_secs(lifetime_secs),
                expires_at,
                interval: Duration::from_secs(interval_secs),
                next_poll_at: instant_now,
            },
        );

        Ok(OpenAiDeviceAuthorization {
            operation_id,
            user_code: payload.user_code,
            verification_uri: VERIFICATION_URL.into(),
            expires_at,
            interval_secs,
        })
    }

    pub async fn poll_openai(&self, operation_id: &str) -> Result<DevicePollResult, AppError> {
        let poll = match self.poll_snapshot(operation_id).await? {
            LocalPollState::Ready(poll) => poll,
            LocalPollState::Pending => return Ok(DevicePollResult::Pending),
            LocalPollState::Expired => return Ok(DevicePollResult::Expired),
        };

        let http = self.client()?;
        let response = http
            .post(DEVICE_POLL_URL)
            .json(&json!({
                "device_auth_id": poll.device_auth_id,
                "user_code": poll.user_code,
            }))
            .send()
            .await
            .map_err(safe_network_error)?;

        match classify_poll_status(response.status()) {
            PollStatus::Pending => Ok(DevicePollResult::Pending),
            PollStatus::Expired => {
                self.pending.lock().await.remove(operation_id);
                Ok(DevicePollResult::Expired)
            }
            PollStatus::Failed(status) => Err(AppError::InvalidConfig(format!(
                "无法查询 OpenAI 授权状态（HTTP {status}），请稍后重试。"
            ))),
            PollStatus::Complete => {
                let code: DevicePollResponse = read_json_bounded(response, "授权结果").await?;
                if code.authorization_code.trim().is_empty() || code.code_verifier.trim().is_empty()
                {
                    return Err(AppError::InvalidConfig(
                        "OpenAI 返回的授权结果不完整，请重新获取授权码。".into(),
                    ));
                }
                let tokens = self.exchange_code(&code).await?;
                let account = account_from_tokens(tokens, None)?;
                self.pending.lock().await.remove(operation_id);
                Ok(DevicePollResult::Complete(Box::new(account)))
            }
        }
    }

    pub async fn refresh_account(
        &self,
        store: &Store,
        account_id: &str,
    ) -> Result<StoredOfficialAccount, AppError> {
        self.refresh_account_with_policy(store, account_id, false)
            .await
    }

    /// Explicit login refresh requested by the user. Unlike activation/quota
    /// refreshes, an available refresh token is always exchanged, even while
    /// the current access token is still far from expiry.
    pub async fn refresh_login(
        &self,
        store: &Store,
        account_id: &str,
    ) -> Result<StoredOfficialAccount, AppError> {
        self.refresh_account_with_policy(store, account_id, true)
            .await
    }

    async fn refresh_account_with_policy(
        &self,
        store: &Store,
        account_id: &str,
        force: bool,
    ) -> Result<StoredOfficialAccount, AppError> {
        // Refresh tokens can rotate and are single-use. Keep the store reload,
        // exchange, and durable save in one shared critical section so a waiter
        // never consumes a stale snapshot after another refresh has completed.
        let _guard = self.refresh_lock.lock().await;
        let account = store.official_account(account_id)?;
        if refresh_decision(&account, force, chrono::Utc::now().timestamp())?
            == AccountRefreshDecision::KeepCurrent
        {
            return Ok(account);
        }
        let http = self.client()?;
        let response = http
            .post(TOKEN_URL)
            .json(&json!({
                "client_id": CODEX_CLIENT_ID,
                "grant_type": "refresh_token",
                "refresh_token": account.credential.tokens.refresh_token,
            }))
            .send()
            .await
            .map_err(safe_network_error)?;

        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(AppError::InvalidConfig(
                "OpenAI 授权已过期，请重新登录。".into(),
            ));
        }
        let response = require_success(response, "无法更新 OpenAI 登录凭据")?;
        let refreshed: TokenResponse = read_json_bounded(response, "登录凭据更新结果").await?;
        let tokens = merge_refreshed_tokens(refreshed, &account)?;
        let refreshed = account_from_tokens(tokens, Some(&account))?;
        store.save_official_account(&refreshed)
    }

    pub async fn connections_import_cookie(
        &self,
        imported: ImportedProxyCredential,
        requested_name: Option<String>,
    ) -> Result<StoredOfficialAccount, AppError> {
        if let Some(name) = requested_name.as_deref() {
            ensure_char_limit(
                name.trim(),
                MAX_DISPLAY_NAME_CHARS,
                "账号名称不能超过 100 个字符。",
            )?;
        }
        if let Some(account_id) = imported.account_id.as_deref() {
            ensure_char_limit(
                account_id.trim(),
                MAX_ACCOUNT_ID_CHARS,
                "OpenAI 账号标识不能超过 512 个字符。",
            )?;
        }
        for token in [
            imported.access_token.as_deref(),
            imported.id_token.as_deref(),
            imported.refresh_token.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            ensure_char_limit(
                token,
                MAX_CREDENTIAL_CHARS,
                "Cookie 登录数据中的单个 Token 过长，请检查导入内容。",
            )?;
        }
        if imported.access_token.is_none() {
            // A refresh-token-only import consumes the same rotating credential
            // as normal login refreshes. Serialize the exchange so no other
            // auth path can consume that token at the same time.
            let _guard = self.refresh_lock.lock().await;
            let refresh_token = imported.refresh_token.clone().ok_or_else(|| {
                AppError::InvalidConfig(
                    "Cookie 登录数据缺少 Access Token 或 Refresh Token。".into(),
                )
            })?;
            let http = self.client()?;
            let response = http
                .post(TOKEN_URL)
                .json(&json!({
                    "client_id": CODEX_CLIENT_ID,
                    "grant_type": "refresh_token",
                    "refresh_token": refresh_token,
                }))
                .send()
                .await
                .map_err(safe_network_error)?;
            let response =
                require_success(response, "无法使用 Cookie 中的 Refresh Token 获取登录凭据")?;
            let refreshed: TokenResponse =
                read_json_bounded(response, "Cookie 登录数据交换结果").await?;
            let expires_at = refreshed
                .expires_in
                .filter(|seconds| *seconds > 0)
                .map(|seconds| chrono::Utc::now().timestamp().saturating_add(seconds))
                .or(imported.expires_at);
            let refreshed_import = ImportedProxyCredential {
                access_token: Some(required_token(refreshed.access_token, "access_token")?),
                id_token: non_empty(refreshed.id_token),
                refresh_token: non_empty(refreshed.refresh_token)
                    .or_else(|| imported.refresh_token.clone()),
                account_id: imported.account_id,
                email: imported.email,
                suggested_name: imported.suggested_name,
                expires_at,
                source_format: imported.source_format,
                is_personal_access_token: imported.is_personal_access_token,
            };
            return account_from_imported_tokens(refreshed_import, requested_name);
        }

        account_from_imported_tokens(imported, requested_name)
    }

    fn client(&self) -> Result<reqwest::Client, AppError> {
        self.client.current(build_oauth_client).map_err(|error| {
            AppError::Internal(format!(
                "无法初始化 OpenAI 网络客户端：{}",
                error.without_url()
            ))
        })
    }

    async fn poll_snapshot(&self, operation_id: &str) -> Result<LocalPollState, AppError> {
        let now = Instant::now();
        let unix_now = chrono::Utc::now().timestamp();
        let mut pending = self.pending.lock().await;
        let Some(state) = pending.get_mut(operation_id) else {
            return Ok(LocalPollState::Expired);
        };
        if state.is_expired(now, unix_now) {
            pending.remove(operation_id);
            return Ok(LocalPollState::Expired);
        }
        if now < state.next_poll_at {
            return Ok(LocalPollState::Pending);
        }
        state.next_poll_at = now + state.interval;
        Ok(LocalPollState::Ready(PendingPoll {
            device_auth_id: state.device_auth_id.clone(),
            user_code: state.user_code.clone(),
        }))
    }

    async fn exchange_code(&self, code: &DevicePollResponse) -> Result<CompleteTokens, AppError> {
        let http = self.client()?;
        let response = http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code.authorization_code.as_str()),
                ("redirect_uri", DEVICE_REDIRECT_URI),
                ("client_id", CODEX_CLIENT_ID),
                ("code_verifier", code.code_verifier.as_str()),
            ])
            .send()
            .await
            .map_err(safe_network_error)?;
        let response = require_success(response, "OpenAI 未能完成授权")?;
        let response: TokenResponse = read_json_bounded(response, "授权登录结果").await?;
        complete_login_tokens(response)
    }
}

impl PendingDeviceAuth {
    fn is_expired(&self, now: Instant, unix_now: i64) -> bool {
        now >= self.deadline || unix_now >= self.expires_at
    }
}

fn build_oauth_client(builder: reqwest::ClientBuilder) -> Result<reqwest::Client, reqwest::Error> {
    let mut headers = HeaderMap::new();
    headers.insert("originator", HeaderValue::from_static(CODEX_ORIGINATOR));
    builder
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .https_only(true)
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(60))
        .pool_max_idle_per_host(4)
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Duration::from_secs(60))
        .build()
}

fn complete_login_tokens(response: TokenResponse) -> Result<CompleteTokens, AppError> {
    Ok(CompleteTokens {
        id_token: required_token(response.id_token, "id_token")?,
        access_token: required_token(response.access_token, "access_token")?,
        refresh_token: required_token(response.refresh_token, "refresh_token")?,
        expires_in: response.expires_in,
    })
}

fn merge_refreshed_tokens(
    response: TokenResponse,
    previous: &StoredOfficialAccount,
) -> Result<CompleteTokens, AppError> {
    let id_token =
        non_empty(response.id_token).unwrap_or_else(|| previous.credential.tokens.id_token.clone());
    let access_token = non_empty(response.access_token)
        .unwrap_or_else(|| previous.credential.tokens.access_token.clone());
    let refresh_token = non_empty(response.refresh_token)
        .unwrap_or_else(|| previous.credential.tokens.refresh_token.clone());
    if id_token.is_empty() || access_token.is_empty() || refresh_token.is_empty() {
        return Err(AppError::InvalidConfig(
            "OpenAI 返回的登录凭据不完整，请重新进行官方授权。".into(),
        ));
    }
    Ok(CompleteTokens {
        id_token,
        access_token,
        refresh_token,
        expires_in: response.expires_in,
    })
}

fn required_token(value: Option<String>, name: &str) -> Result<String, AppError> {
    non_empty(value).ok_or_else(|| {
        AppError::InvalidConfig(format!(
            "OpenAI 返回的登录凭据缺少 {name}，请重新进行官方授权。"
        ))
    })
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

fn refresh_decision(
    account: &StoredOfficialAccount,
    force: bool,
    now: i64,
) -> Result<AccountRefreshDecision, AppError> {
    if account.credential.tokens.refresh_token.trim().is_empty() {
        if account
            .expires_at
            .is_some_and(|expires_at| expires_at <= now)
        {
            let message = match account.source {
                OfficialAccountSource::ProxyImport => {
                    "Cookie 登录的 Access Token 已过期，请重新导入 Cookie 数据。"
                }
                OfficialAccountSource::OpenAiOauth => {
                    "OpenAI 登录凭据已过期且无法自动更新，请重新进行官方授权。"
                }
            };
            return Err(AppError::InvalidConfig(message.into()));
        }
        return Ok(AccountRefreshDecision::KeepCurrent);
    }
    if force
        || account.expires_at.is_none_or(|expires_at| {
            expires_at <= now.saturating_add(ACCESS_TOKEN_REFRESH_WINDOW_SECS)
        })
    {
        Ok(AccountRefreshDecision::Refresh)
    } else {
        Ok(AccountRefreshDecision::KeepCurrent)
    }
}

fn account_from_tokens(
    tokens: CompleteTokens,
    previous: Option<&StoredOfficialAccount>,
) -> Result<StoredOfficialAccount, AppError> {
    let id_claims = token_identity(&tokens.id_token).unwrap_or_default();
    let access_claims = token_identity(&tokens.access_token).unwrap_or_default();
    let account_id = id_claims
        .account_id
        .or(access_claims.account_id)
        .or_else(|| previous.map(|account| account.account_id.clone()))
        .ok_or_else(|| {
            AppError::InvalidConfig(
                "OpenAI 返回的登录凭据缺少账号标识，请重新进行官方授权。".into(),
            )
        })?;

    if previous.is_some_and(|account| account.account_id != account_id) {
        return Err(AppError::InvalidConfig(
            "OpenAI 返回的凭据属于其他账号，请重新进行官方授权。".into(),
        ));
    }

    let email = id_claims
        .email
        .or(access_claims.email)
        .or_else(|| previous.map(|account| account.email.clone()))
        .unwrap_or_default();
    let now = chrono::Utc::now();
    let now_timestamp = now.timestamp();
    let expires_at = tokens
        .expires_in
        .filter(|seconds| *seconds > 0)
        .map(|seconds| now_timestamp.saturating_add(seconds))
        .or(access_claims.expires_at)
        .or(id_claims.expires_at)
        .or_else(|| previous.and_then(|account| account.expires_at));
    let credential = CodexAuthCredential {
        auth_mode: "chatgpt".into(),
        openai_api_key: None,
        tokens: CodexAuthTokens {
            id_token: tokens.id_token,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            account_id: account_id.clone(),
        },
        last_refresh: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    };

    Ok(StoredOfficialAccount {
        id: previous.map_or_else(String::new, |account| account.id.clone()),
        name: if email.is_empty() {
            previous.map_or_else(|| "OpenAI 账号".into(), |account| account.name.clone())
        } else {
            email.clone()
        },
        remark: previous.map_or_else(String::new, |account| account.remark.clone()),
        account_id,
        email,
        credential,
        source: previous.map_or(OfficialAccountSource::OpenAiOauth, |account| account.source),
        expires_at,
        quota: previous.map_or_else(ProviderAccountQuota::default, |account| {
            account.quota.clone()
        }),
        created_at: previous.map_or(now_timestamp, |account| account.created_at),
        updated_at: now_timestamp,
    })
}

fn account_from_imported_tokens(
    imported: ImportedProxyCredential,
    requested_name: Option<String>,
) -> Result<StoredOfficialAccount, AppError> {
    let access_token = imported
        .access_token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::InvalidConfig("Cookie 登录数据缺少 Access Token。".into()))?
        .to_owned();
    let id_token = imported.id_token.clone().unwrap_or_default();
    let id_claims = token_identity(&id_token).unwrap_or_default();
    let access_claims = token_identity(&access_token).unwrap_or_default();
    let account_id = imported
        .account_id
        .clone()
        .or(id_claims.account_id)
        .or(access_claims.account_id)
        .unwrap_or_else(|| token_local_identity(&access_token));
    let email = imported
        .email
        .clone()
        .or(id_claims.email)
        .or(access_claims.email)
        .unwrap_or_default();
    let now = chrono::Utc::now();
    let now_timestamp = now.timestamp();
    let expires_at = access_claims
        .expires_at
        .or(id_claims.expires_at)
        .or(imported.expires_at);
    let mut account = StoredOfficialAccount {
        id: String::new(),
        name: String::new(),
        remark: String::new(),
        account_id: account_id.clone(),
        email,
        credential: CodexAuthCredential {
            auth_mode: if imported.is_personal_access_token {
                "personal_access_token".into()
            } else {
                "chatgpt".into()
            },
            openai_api_key: None,
            tokens: CodexAuthTokens {
                id_token,
                access_token,
                refresh_token: imported.refresh_token.clone().unwrap_or_default(),
                account_id,
            },
            last_refresh: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        },
        source: OfficialAccountSource::ProxyImport,
        expires_at,
        quota: ProviderAccountQuota::default(),
        created_at: now_timestamp,
        updated_at: now_timestamp,
    };
    apply_imported_profile(&mut account, &imported, requested_name);
    Ok(account)
}

fn apply_imported_profile(
    account: &mut StoredOfficialAccount,
    imported: &ImportedProxyCredential,
    requested_name: Option<String>,
) {
    if account.email.is_empty() {
        account.email = imported.email.clone().unwrap_or_default();
    }
    account.name = requested_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| imported.suggested_name.clone())
        .or_else(|| (!account.email.is_empty()).then(|| account.email.clone()))
        .unwrap_or_else(|| "Cookie 账号".into());
}

fn parse_interval(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(value)) => value.as_u64().unwrap_or(5),
        Some(Value::String(value)) => value.trim().parse().unwrap_or(5),
        _ => 5,
    }
    .clamp(1, 60)
}

#[derive(Debug, PartialEq, Eq)]
enum PollStatus {
    Pending,
    Expired,
    Complete,
    Failed(u16),
}

fn classify_poll_status(status: StatusCode) -> PollStatus {
    match status.as_u16() {
        403 | 404 => PollStatus::Pending,
        410 => PollStatus::Expired,
        200..=299 => PollStatus::Complete,
        value => PollStatus::Failed(value),
    }
}

fn require_success(
    response: reqwest::Response,
    context: &str,
) -> Result<reqwest::Response, AppError> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(AppError::InvalidConfig(format!(
            "{context}（HTTP {}）",
            response.status().as_u16()
        )))
    }
}

async fn read_json_bounded<T: DeserializeOwned>(
    response: reqwest::Response,
    context: &str,
) -> Result<T, AppError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_OAUTH_RESPONSE_BYTES as u64)
    {
        return Err(AppError::InvalidConfig(format!(
            "OpenAI 返回的{context}数据过大，请稍后重试。"
        )));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(safe_network_error)?;
        if body.len().saturating_add(chunk.len()) > MAX_OAUTH_RESPONSE_BYTES {
            return Err(AppError::InvalidConfig(format!(
                "OpenAI 返回的{context}数据过大，请稍后重试。"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| {
        AppError::InvalidConfig(format!("OpenAI 返回的{context}格式无法识别，请稍后重试。"))
    })
}

fn safe_network_error(error: reqwest::Error) -> AppError {
    let kind = if error.is_timeout() {
        "OpenAI 授权请求超时，请检查网络连接后重试。"
    } else if error.is_connect() {
        "无法连接 OpenAI 授权服务，请检查网络或系统代理。"
    } else if error.is_body() || error.is_decode() {
        "无法读取 OpenAI 返回的授权结果，请重试。"
    } else {
        "OpenAI 授权请求未完成，请检查网络连接后重试。"
    };
    AppError::Internal(kind.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::QuotaStatus;
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    fn jwt(claims: Value) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        format!("e30.{payload}.signature")
    }

    fn refresh_test_account(
        source: OfficialAccountSource,
        refresh_token: &str,
        expires_at: Option<i64>,
    ) -> StoredOfficialAccount {
        StoredOfficialAccount {
            id: "local-id".into(),
            name: "person@example.test".into(),
            remark: "主要账号".into(),
            account_id: "acct-1".into(),
            email: "person@example.test".into(),
            credential: CodexAuthCredential {
                auth_mode: "chatgpt".into(),
                openai_api_key: None,
                tokens: CodexAuthTokens {
                    id_token: jwt(json!({"chatgpt_account_id": "acct-1"})),
                    access_token: jwt(json!({"chatgpt_account_id": "acct-1"})),
                    refresh_token: refresh_token.into(),
                    account_id: "acct-1".into(),
                },
                last_refresh: "2026-01-01T00:00:00Z".into(),
            },
            source,
            expires_at,
            quota: ProviderAccountQuota::default(),
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn extracts_namespaced_codex_claims() {
        let token = jwt(json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct-1"
            },
            "https://api.openai.com/profile": {
                "email": "sakura@example.com"
            },
            "exp": 1_800_000_000_i64
        }));
        let identity = token_identity(&token).unwrap();
        assert_eq!(identity.account_id.as_deref(), Some("acct-1"));
        assert_eq!(identity.email.as_deref(), Some("sakura@example.com"));
        assert_eq!(identity.expires_at, Some(1_800_000_000));
    }

    #[test]
    fn rejects_malformed_claims_without_echoing_tokens() {
        assert!(token_identity("not-a-jwt").is_none());
        assert!(token_identity("a.invalid-base64.c").is_none());
    }

    #[test]
    fn classifies_device_poll_states() {
        assert_eq!(
            classify_poll_status(StatusCode::FORBIDDEN),
            PollStatus::Pending
        );
        assert_eq!(
            classify_poll_status(StatusCode::NOT_FOUND),
            PollStatus::Pending
        );
        assert_eq!(classify_poll_status(StatusCode::GONE), PollStatus::Expired);
        assert_eq!(classify_poll_status(StatusCode::OK), PollStatus::Complete);
        assert_eq!(
            classify_poll_status(StatusCode::TOO_MANY_REQUESTS),
            PollStatus::Failed(429)
        );
    }

    #[tokio::test]
    async fn missing_device_operation_is_terminally_expired() {
        let center = AuthCenter::default();
        assert!(matches!(
            center.poll_snapshot("missing").await.unwrap(),
            LocalPollState::Expired
        ));
    }

    #[test]
    fn interval_accepts_string_and_number_with_safe_bounds() {
        assert_eq!(parse_interval(Some(&json!(2))), 2);
        assert_eq!(parse_interval(Some(&json!("7"))), 7);
        assert_eq!(parse_interval(Some(&json!(0))), 1);
        assert_eq!(parse_interval(Some(&json!(600))), 60);
        assert_eq!(parse_interval(None), 5);
    }

    #[test]
    fn explicit_login_refresh_forces_an_oauth_token_exchange() {
        let now: i64 = 1_000_000;
        let account = refresh_test_account(
            OfficialAccountSource::OpenAiOauth,
            "refresh-token",
            Some(now + 86_400),
        );

        assert_eq!(
            refresh_decision(&account, false, now).unwrap(),
            AccountRefreshDecision::KeepCurrent
        );
        assert_eq!(
            refresh_decision(&account, true, now).unwrap(),
            AccountRefreshDecision::Refresh
        );
    }

    #[tokio::test]
    async fn explicit_login_refresh_accepts_a_valid_cookie_without_refresh_token() {
        let now = chrono::Utc::now().timestamp();
        let account = refresh_test_account(
            OfficialAccountSource::ProxyImport,
            "",
            Some(now.saturating_add(3600)),
        );
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let saved = store.save_official_account(&account).unwrap();

        assert_eq!(
            refresh_decision(&account, true, now).unwrap(),
            AccountRefreshDecision::KeepCurrent
        );
        assert_eq!(
            AuthCenter::default()
                .refresh_login(&store, &saved.id)
                .await
                .unwrap(),
            saved
        );
    }

    #[test]
    fn explicit_login_refresh_rejects_an_expired_cookie_without_refresh_token() {
        let now: i64 = 1_000_000;
        let account = refresh_test_account(
            OfficialAccountSource::ProxyImport,
            "",
            Some(now.saturating_sub(1)),
        );

        let error = refresh_decision(&account, true, now).unwrap_err();
        assert!(error.to_string().contains("Access Token 已过期"));
    }

    #[test]
    fn refreshed_tokens_keep_account_remark_and_quota() {
        let mut previous = refresh_test_account(
            OfficialAccountSource::OpenAiOauth,
            "refresh-old",
            Some(1_800_000_000),
        );
        previous.quota.status = QuotaStatus::Success;
        previous.quota.fetched_at = Some(42);
        let tokens = CompleteTokens {
            id_token: jwt(json!({"chatgpt_account_id": "acct-1"})),
            access_token: jwt(json!({"chatgpt_account_id": "acct-1"})),
            refresh_token: "refresh-new".into(),
            expires_in: Some(3600),
        };

        let refreshed = account_from_tokens(tokens, Some(&previous)).unwrap();

        assert_eq!(refreshed.remark, "主要账号");
        assert_eq!(refreshed.quota.status, QuotaStatus::Success);
        assert_eq!(refreshed.quota.fetched_at, Some(42));
    }

    #[test]
    fn refresh_cannot_change_account_identity() {
        let previous = StoredOfficialAccount {
            id: "local-id".into(),
            name: "old@example.com".into(),
            remark: "主要账号".into(),
            account_id: "acct-old".into(),
            email: "old@example.com".into(),
            credential: CodexAuthCredential {
                auth_mode: "chatgpt".into(),
                openai_api_key: None,
                tokens: CodexAuthTokens {
                    id_token: jwt(json!({"chatgpt_account_id": "acct-old"})),
                    access_token: jwt(json!({"chatgpt_account_id": "acct-old"})),
                    refresh_token: "refresh-old".into(),
                    account_id: "acct-old".into(),
                },
                last_refresh: "2026-01-01T00:00:00Z".into(),
            },
            source: OfficialAccountSource::OpenAiOauth,
            expires_at: None,
            quota: ProviderAccountQuota::default(),
            created_at: 1,
            updated_at: 1,
        };
        let tokens = CompleteTokens {
            id_token: jwt(json!({"chatgpt_account_id": "acct-other"})),
            access_token: jwt(json!({"chatgpt_account_id": "acct-other"})),
            refresh_token: "refresh-new".into(),
            expires_in: Some(3600),
        };
        assert!(account_from_tokens(tokens, Some(&previous)).is_err());
    }

    #[test]
    fn imported_personal_access_token_is_marked_as_proxy_account() {
        let account = account_from_imported_tokens(
            ImportedProxyCredential {
                access_token: Some("at-proxy-secret".into()),
                id_token: None,
                refresh_token: None,
                account_id: None,
                email: Some("proxy@example.test".into()),
                suggested_name: Some("日常 Cookie 账号".into()),
                expires_at: None,
                source_format: crate::proxy_import::ProxyCredentialFormat::RawAccessToken,
                is_personal_access_token: true,
            },
            None,
        )
        .unwrap();

        assert_eq!(account.source, OfficialAccountSource::ProxyImport);
        assert_eq!(account.name, "日常 Cookie 账号");
        assert_eq!(account.email, "proxy@example.test");
        assert!(account.account_id.starts_with("proxy-"));
        assert_eq!(account.credential.tokens.access_token, "at-proxy-secret");
        assert!(account.credential.tokens.refresh_token.is_empty());
    }
}
