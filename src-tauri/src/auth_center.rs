use crate::models::{AppError, AuthAccount, AuthService, GitHubDeviceAuthorization};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use tokio::sync::Mutex;

#[derive(Clone)]
struct PendingGitHub {
    device_code: String,
    client_id: String,
    expires_at: i64,
}

#[derive(Default)]
pub struct AuthCenter {
    pending_github: Mutex<HashMap<String, PendingGitHub>>,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: i64,
    interval: Option<u64>,
}

impl AuthCenter {
    pub async fn start_github(
        &self,
        client_id: &str,
        scopes: &[String],
    ) -> Result<GitHubDeviceAuthorization, AppError> {
        if client_id.trim().is_empty() {
            return Err(AppError::InvalidConfig(
                "GitHub OAuth App Client ID 不能为空".into(),
            ));
        }
        let response = reqwest::Client::new()
            .post("https://github.com/login/device/code")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", client_id.trim()),
                ("scope", &scopes.join(" ")),
            ])
            .send()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if !response.status().is_success() {
            return Err(AppError::InvalidConfig(format!(
                "GitHub Device Flow 启动失败（HTTP {}）",
                response.status().as_u16()
            )));
        }
        let payload: DeviceCodeResponse = response
            .json()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let operation_id = uuid::Uuid::new_v4().to_string();
        let expires_at = chrono::Utc::now().timestamp() + payload.expires_in;
        self.pending_github.lock().await.insert(
            operation_id.clone(),
            PendingGitHub {
                device_code: payload.device_code,
                client_id: client_id.trim().to_string(),
                expires_at,
            },
        );
        Ok(GitHubDeviceAuthorization {
            operation_id,
            user_code: payload.user_code,
            verification_uri: payload.verification_uri,
            expires_at,
            interval_secs: payload.interval.unwrap_or(5).max(5),
        })
    }

    pub async fn complete_github(&self, operation_id: &str) -> Result<AuthAccount, AppError> {
        let pending = self
            .pending_github
            .lock()
            .await
            .get(operation_id)
            .cloned()
            .ok_or(AppError::StaleOperation)?;
        if pending.expires_at <= chrono::Utc::now().timestamp() {
            return Err(AppError::StaleOperation);
        }
        let response = reqwest::Client::new()
            .post("https://github.com/login/oauth/access_token")
            .header("Accept", "application/json")
            .form(&[
                ("client_id", pending.client_id.as_str()),
                ("device_code", pending.device_code.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let token: Value = response
            .json()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        if let Some(code) = token.get("error").and_then(Value::as_str) {
            return Err(AppError::InvalidConfig(match code {
                "authorization_pending" => "GitHub 尚未完成授权，请授权后重试".into(),
                "access_denied" => "GitHub 授权已被拒绝".into(),
                "expired_token" => "GitHub 设备授权已过期".into(),
                value => format!("GitHub OAuth 返回错误：{value}"),
            }));
        }
        let access_token = token
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::InvalidConfig("GitHub 未返回访问令牌".into()))?;
        let user: Value = reqwest::Client::new()
            .get("https://api.github.com/user")
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "codex-tools")
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?
            .json()
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let login = user.get("login").and_then(Value::as_str).map(str::to_owned);
        let now = chrono::Utc::now().timestamp();
        self.pending_github.lock().await.remove(operation_id);
        Ok(AuthAccount {
            id: uuid::Uuid::new_v4().to_string(),
            service: AuthService::GitHub,
            name: login.clone().unwrap_or_else(|| "GitHub 账号".into()),
            login,
            email: user.get("email").and_then(Value::as_str).map(str::to_owned),
            credential: Some(
                json!({"access_token":access_token,"token_type":token.get("token_type"),"scope":token.get("scope")}),
            ),
            config_snapshot: None,
            scopes: token
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .split(',')
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect(),
            expires_at: None,
            active: false,
            created_at: now,
            updated_at: now,
        })
    }
}
