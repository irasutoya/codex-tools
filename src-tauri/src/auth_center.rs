use crate::models::{
    AppError, CodexAuthCredential, CodexAuthTokens, OpenAiDeviceAuthorization,
    StoredOfficialAccount,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::StreamExt;
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_START_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_POLL_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const DEVICE_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const CODEX_ORIGINATOR: &str = "codex_cli_rs";
const CODEX_LOGIN_SUFFIX: &str = "codex_login";
const FALLBACK_CODEX_VERSION: &str = "0.144.1";
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
    client: Result<reqwest::Client, String>,
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

#[derive(Default)]
struct TokenIdentity {
    account_id: Option<String>,
    email: Option<String>,
    expires_at: Option<i64>,
}

impl Default for AuthCenter {
    fn default() -> Self {
        let user_agent = codex_user_agent();
        let client = build_oauth_client(&user_agent).map_err(|error| {
            format!(
                "无法准备 OpenAI 登录，请重启应用后再试：{}",
                error.without_url()
            )
        });
        Self {
            client,
            pending: Mutex::new(HashMap::new()),
            refresh_lock: Mutex::new(()),
        }
    }
}

impl AuthCenter {
    pub async fn start_openai(&self) -> Result<OpenAiDeviceAuthorization, AppError> {
        let response = self
            .client()?
            .post(DEVICE_START_URL)
            .json(&json!({ "client_id": CODEX_CLIENT_ID }))
            .send()
            .await
            .map_err(safe_network_error)?;
        let response = require_success(response, "无法开始 OpenAI 登录")?;
        let payload: DeviceStartResponse = read_json_bounded(response, "登录码").await?;

        if payload.device_auth_id.trim().is_empty() || payload.user_code.trim().is_empty() {
            return Err(AppError::InvalidConfig(
                "OpenAI 没有返回有效的登录码，请重新开始登录。".into(),
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

        let response = self
            .client()?
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
                "无法确认 OpenAI 登录结果（HTTP {status}），请稍后重试。"
            ))),
            PollStatus::Complete => {
                let code: DevicePollResponse = read_json_bounded(response, "登录结果").await?;
                if code.authorization_code.trim().is_empty() || code.code_verifier.trim().is_empty()
                {
                    return Err(AppError::InvalidConfig(
                        "OpenAI 返回的登录结果不完整，请重新登录。".into(),
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
        account: &StoredOfficialAccount,
    ) -> Result<StoredOfficialAccount, AppError> {
        if account.expires_at.is_some_and(|expires_at| {
            expires_at
                > chrono::Utc::now()
                    .timestamp()
                    .saturating_add(ACCESS_TOKEN_REFRESH_WINDOW_SECS)
        }) {
            return Ok(account.clone());
        }
        // Refresh tokens can rotate and are single-use. Serializing refreshes prevents
        // two activations from consuming the same token concurrently.
        let _guard = self.refresh_lock.lock().await;
        let response = self
            .client()?
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
                "OpenAI 登录已过期，请重新登录。".into(),
            ));
        }
        let response = require_success(response, "无法续期 OpenAI 登录")?;
        let refreshed: TokenResponse = read_json_bounded(response, "登录续期结果").await?;
        let tokens = merge_refreshed_tokens(refreshed, account)?;
        account_from_tokens(tokens, Some(account))
    }

    fn client(&self) -> Result<&reqwest::Client, AppError> {
        self.client
            .as_ref()
            .map_err(|message| AppError::Internal(message.clone()))
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
        let response = self
            .client()?
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
        let response = require_success(response, "无法完成 OpenAI 登录")?;
        let response: TokenResponse = read_json_bounded(response, "登录结果").await?;
        complete_login_tokens(response)
    }
}

impl PendingDeviceAuth {
    fn is_expired(&self, now: Instant, unix_now: i64) -> bool {
        now >= self.deadline || unix_now >= self.expires_at
    }
}

fn build_oauth_client(user_agent: &str) -> Result<reqwest::Client, reqwest::Error> {
    let mut headers = HeaderMap::new();
    headers.insert("originator", HeaderValue::from_static(CODEX_ORIGINATOR));
    if let Ok(value) = HeaderValue::from_str(user_agent) {
        headers.insert(USER_AGENT, value);
    }
    reqwest::Client::builder()
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
            "OpenAI 返回的登录续期信息不完整，请重新登录。".into(),
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
        AppError::InvalidConfig(format!("OpenAI 返回的登录信息缺少 {name}，请重新登录。"))
    })
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
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
            AppError::InvalidConfig("OpenAI 返回的登录信息缺少账号标识，请重新登录。".into())
        })?;

    if previous.is_some_and(|account| account.account_id != account_id) {
        return Err(AppError::InvalidConfig(
            "OpenAI 返回了其他账号的登录信息，请重新登录。".into(),
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
            previous.map_or_else(|| "OpenAI 官方账号".into(), |account| account.name.clone())
        } else {
            email.clone()
        },
        account_id,
        email,
        credential,
        expires_at,
        created_at: previous.map_or(now_timestamp, |account| account.created_at),
        updated_at: now_timestamp,
    })
}

fn token_identity(token: &str) -> Option<TokenIdentity> {
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
        "登录请求超时，请检查网络后重试。"
    } else if error.is_connect() {
        "无法连接 OpenAI 登录服务，请检查网络。"
    } else if error.is_body() || error.is_decode() {
        "无法读取 OpenAI 的登录结果，请重试。"
    } else {
        "登录请求失败，请检查网络后重试。"
    };
    AppError::Internal(format!("OpenAI OAuth {kind}"))
}

fn codex_user_agent() -> String {
    static USER_AGENT: OnceLock<String> = OnceLock::new();
    USER_AGENT
        .get_or_init(|| {
            build_codex_user_agent(
                detected_codex_version(),
                crate::platform::os_name(),
                &crate::platform::os_version(),
                std::env::consts::ARCH,
            )
        })
        .clone()
}

fn build_codex_user_agent(version: &str, os_name: &str, os_version: &str, arch: &str) -> String {
    format!(
        "{CODEX_ORIGINATOR}/{version} ({os_name} {os_version}; {arch}) unknown ({CODEX_LOGIN_SUFFIX}; {version})"
    )
}

fn detected_codex_version() -> &'static str {
    static VERSION: OnceLock<String> = OnceLock::new();
    VERSION
        .get_or_init(|| detect_codex_version().unwrap_or_else(|| FALLBACK_CODEX_VERSION.into()))
        .as_str()
}

fn detect_codex_version() -> Option<String> {
    let mut command = crate::platform::codex_command();
    command.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_codex_version(&String::from_utf8_lossy(&output.stdout))
}

fn parse_codex_version(output: &str) -> Option<String> {
    let mut fields = output.split_whitespace();
    let product = fields.next()?;
    if !matches!(product, "codex-cli" | "codex") {
        return None;
    }
    let version = fields.next()?.trim_start_matches('v');
    if version.is_empty()
        || version.len() > 32
        || !version
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'.' | b'-' | b'+'))
    {
        return None;
    }
    Some(version.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: Value) -> String {
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        format!("e30.{payload}.signature")
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
    fn user_agent_matches_codex_cli_shape() {
        assert_eq!(
            build_codex_user_agent("0.144.1", "Windows", "10.0.26100", "x86_64"),
            "codex_cli_rs/0.144.1 (Windows 10.0.26100; x86_64) unknown (codex_login; 0.144.1)"
        );
        assert_eq!(
            build_codex_user_agent("0.144.1", "Mac OS", "15.5", "aarch64"),
            "codex_cli_rs/0.144.1 (Mac OS 15.5; aarch64) unknown (codex_login; 0.144.1)"
        );
        assert_eq!(
            parse_codex_version("codex-cli 0.144.1\r\n"),
            Some("0.144.1".into())
        );
        assert_eq!(parse_codex_version("malicious token value"), None);
    }

    #[test]
    fn refresh_cannot_change_account_identity() {
        let previous = StoredOfficialAccount {
            id: "local-id".into(),
            name: "old@example.com".into(),
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
            expires_at: None,
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
}
