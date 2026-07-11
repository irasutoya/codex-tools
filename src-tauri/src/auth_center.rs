use crate::models::{
    AppError, AuthAccount, AuthService, OpenAiDeviceAuthorization, OpenAiDevicePoll,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio::sync::Mutex;

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const DEVICE_START_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";
const DEVICE_POLL_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

#[derive(Clone)]
struct PendingDeviceAuth {
    device_auth_id: String,
    user_code: String,
    expires_at: i64,
}

#[derive(Default)]
pub struct AuthCenter {
    pending: Mutex<HashMap<String, PendingDeviceAuth>>,
    refresh_locks: Mutex<HashMap<String, std::sync::Arc<Mutex<()>>>>,
}

#[derive(Deserialize)]
struct DeviceStartResponse {
    device_auth_id: String,
    user_code: String,
    #[serde(default)]
    interval: Option<Value>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct DevicePollResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

impl AuthCenter {
    pub async fn start_openai(&self) -> Result<OpenAiDeviceAuthorization, AppError> {
        let response = reqwest::Client::new()
            .post(DEVICE_START_URL)
            .header("User-Agent", "codex-tools")
            .json(&json!({"client_id": CODEX_CLIENT_ID}))
            .send()
            .await
            .map_err(safe_network_error)?;
        if !response.status().is_success() {
            return Err(AppError::InvalidConfig(format!(
                "OpenAI 设备登录启动失败（HTTP {}）",
                response.status().as_u16()
            )));
        }
        let payload: DeviceStartResponse = response.json().await.map_err(safe_network_error)?;
        let expires_in = payload.expires_in.unwrap_or(900).max(30);
        let expires_at = chrono::Utc::now().timestamp() + expires_in;
        let interval_secs = parse_interval(payload.interval.as_ref()).saturating_add(3);
        let operation_id = uuid::Uuid::new_v4().to_string();
        let mut pending = self.pending.lock().await;
        let now = chrono::Utc::now().timestamp();
        pending.retain(|_, value| value.expires_at > now);
        pending.insert(
            operation_id.clone(),
            PendingDeviceAuth {
                device_auth_id: payload.device_auth_id,
                user_code: payload.user_code.clone(),
                expires_at,
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

    pub async fn poll_openai(&self, operation_id: &str) -> Result<OpenAiDevicePoll, AppError> {
        let pending = self
            .pending
            .lock()
            .await
            .get(operation_id)
            .cloned()
            .ok_or(AppError::StaleOperation)?;
        if pending.expires_at <= chrono::Utc::now().timestamp() {
            self.pending.lock().await.remove(operation_id);
            return Ok(OpenAiDevicePoll::Expired);
        }
        let response = reqwest::Client::new()
            .post(DEVICE_POLL_URL)
            .header("User-Agent", "codex-tools")
            .json(&json!({
                "device_auth_id": pending.device_auth_id,
                "user_code": pending.user_code,
            }))
            .send()
            .await
            .map_err(safe_network_error)?;
        match response.status().as_u16() {
            403 | 404 => return Ok(OpenAiDevicePoll::Pending),
            410 => {
                self.pending.lock().await.remove(operation_id);
                return Ok(OpenAiDevicePoll::Expired);
            }
            status if !(200..300).contains(&status) => {
                return Err(AppError::InvalidConfig(format!(
                    "OpenAI 设备登录轮询失败（HTTP {status}）"
                )));
            }
            _ => {}
        }
        let code: DevicePollResponse = response.json().await.map_err(safe_network_error)?;
        let tokens = exchange_code(&code.authorization_code, &code.code_verifier).await?;
        let account = account_from_tokens(tokens, None)?;
        self.pending.lock().await.remove(operation_id);
        Ok(OpenAiDevicePoll::Complete {
            account: Box::new(account),
        })
    }

    pub async fn refresh_account(&self, account: &AuthAccount) -> Result<AuthAccount, AppError> {
        let refresh_token = account
            .credential
            .as_ref()
            .and_then(|value| value.pointer("/tokens/refresh_token"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AppError::InvalidConfig("官方账号缺少 refresh_token，请重新登录".into())
            })?
            .to_string();
        let lock = {
            let mut locks = self.refresh_locks.lock().await;
            locks
                .entry(account.id.clone())
                .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;
        let response = reqwest::Client::new()
            .post(TOKEN_URL)
            .header("User-Agent", "codex-tools")
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("client_id", CODEX_CLIENT_ID),
                ("scope", "openid profile email"),
            ])
            .send()
            .await
            .map_err(safe_network_error)?;
        if !response.status().is_success() {
            return Err(AppError::InvalidConfig(format!(
                "OpenAI 登录已失效，请重新登录（HTTP {}）",
                response.status().as_u16()
            )));
        }
        let tokens: TokenResponse = response.json().await.map_err(safe_network_error)?;
        account_from_tokens(tokens, Some((account, refresh_token)))
    }
}

async fn exchange_code(code: &str, verifier: &str) -> Result<TokenResponse, AppError> {
    let response = reqwest::Client::new()
        .post(TOKEN_URL)
        .header("User-Agent", "codex-tools")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("client_id", CODEX_CLIENT_ID),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(safe_network_error)?;
    if !response.status().is_success() {
        return Err(AppError::InvalidConfig(format!(
            "OpenAI 登录令牌交换失败（HTTP {}）",
            response.status().as_u16()
        )));
    }
    response.json().await.map_err(safe_network_error)
}

fn account_from_tokens(
    tokens: TokenResponse,
    previous: Option<(&AuthAccount, String)>,
) -> Result<AuthAccount, AppError> {
    let (account_id, email) = extract_identity(&tokens)
        .ok_or_else(|| AppError::InvalidConfig("OpenAI 登录响应缺少账号标识".into()))?;
    let now = chrono::Utc::now();
    let refresh_token = tokens
        .refresh_token
        .or_else(|| previous.as_ref().map(|(_, token)| token.clone()))
        .ok_or_else(|| AppError::InvalidConfig("OpenAI 登录响应缺少 refresh_token".into()))?;
    let previous_account = previous.as_ref().map(|(account, _)| *account);
    let credential = json!({
        "auth_mode": "chatgpt",
        "tokens": {
            "id_token": tokens.id_token,
            "access_token": tokens.access_token,
            "refresh_token": refresh_token,
            "account_id": account_id,
        },
        "last_refresh": now.to_rfc3339(),
    });
    Ok(AuthAccount {
        id: previous_account
            .map(|account| account.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        service: AuthService::OpenAi,
        name: email.clone().unwrap_or_else(|| "OpenAI 官方账号".into()),
        login: Some(account_id),
        email,
        credential: Some(credential),
        config_snapshot: previous_account.and_then(|account| account.config_snapshot.clone()),
        scopes: vec!["openid".into(), "profile".into(), "email".into()],
        expires_at: Some(now.timestamp() + tokens.expires_in.unwrap_or(3600)),
        active: previous_account.is_some_and(|account| account.active),
        created_at: previous_account.map_or(now.timestamp(), |account| account.created_at),
        updated_at: now.timestamp(),
    })
}

fn extract_identity(tokens: &TokenResponse) -> Option<(String, Option<String>)> {
    tokens
        .id_token
        .as_deref()
        .and_then(parse_claims)
        .or_else(|| parse_claims(&tokens.access_token))
        .and_then(|claims| {
            let account_id = claims
                .get("chatgpt_account_id")
                .and_then(Value::as_str)
                .or_else(|| {
                    claims
                        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
                        .and_then(Value::as_str)
                })
                .or_else(|| {
                    claims
                        .pointer("/organizations/0/id")
                        .and_then(Value::as_str)
                })?;
            let email = claims
                .get("email")
                .and_then(Value::as_str)
                .map(str::to_owned);
            Some((account_id.to_owned(), email))
        })
}

fn parse_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn parse_interval(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => number.as_u64().unwrap_or(5),
        Some(Value::String(value)) => value.parse().unwrap_or(5),
        _ => 5,
    }
    .max(1)
}

fn safe_network_error(error: reqwest::Error) -> AppError {
    AppError::Internal(format!(
        "OpenAI OAuth 网络请求失败：{}",
        error.without_url()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_accepts_number_and_string() {
        assert_eq!(parse_interval(Some(&json!(2))), 2);
        assert_eq!(parse_interval(Some(&json!("7"))), 7);
        assert_eq!(parse_interval(None), 5);
    }

    #[test]
    fn extracts_nested_openai_identity() {
        let claims = URL_SAFE_NO_PAD.encode(
            br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"acct-1"},"email":"a@example.com"}"#,
        );
        let token = format!("e30.{claims}.sig");
        let tokens = TokenResponse {
            access_token: token,
            refresh_token: Some("refresh".into()),
            id_token: None,
            expires_in: Some(3600),
        };
        assert_eq!(
            extract_identity(&tokens),
            Some(("acct-1".into(), Some("a@example.com".into())))
        );
    }
}
