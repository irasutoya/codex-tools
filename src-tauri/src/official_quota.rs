use crate::models::{
    OfficialAccountSource, QuotaData, QuotaStatus, QuotaWindow, StoredOfficialAccount,
};
use futures_util::StreamExt;
use reqwest::header::{
    ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, REFERER, USER_AGENT,
};
use serde_json::Value;
use std::time::Duration;

const OPENAI_USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const MAX_QUOTA_RESPONSE_BYTES: usize = 256 * 1024;
const CHATGPT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36";

#[derive(Debug)]
pub struct QuotaFetchError {
    pub status: QuotaStatus,
    pub message: String,
}

impl QuotaFetchError {
    fn new(status: QuotaStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

pub async fn fetch_quota(
    client: &reqwest::Client,
    account: &StoredOfficialAccount,
) -> Result<QuotaData, QuotaFetchError> {
    fetch_quota_from(client, account, OPENAI_USAGE_URL).await
}

async fn fetch_quota_from(
    client: &reqwest::Client,
    account: &StoredOfficialAccount,
    endpoint: &str,
) -> Result<QuotaData, QuotaFetchError> {
    let access_token = account.credential.tokens.access_token.trim();
    if access_token.is_empty() {
        return Err(QuotaFetchError::new(
            QuotaStatus::Unauthorized,
            "账号缺少 accessToken，请重新登录或导入",
        ));
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {access_token}"))
            .map_err(|_| QuotaFetchError::new(QuotaStatus::Error, "accessToken 包含无效字符"))?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(REFERER, HeaderValue::from_static("https://chatgpt.com/"));
    headers.insert(USER_AGENT, HeaderValue::from_static(CHATGPT_USER_AGENT));
    headers.insert("OpenAI-Beta", HeaderValue::from_static("codex-1"));
    headers.insert("oai-language", HeaderValue::from_static("zh-CN"));
    headers.insert("originator", HeaderValue::from_static("Codex Desktop"));

    if should_send_account_id(account) {
        headers.insert(
            "ChatGPT-Account-Id",
            HeaderValue::from_str(account.account_id.trim())
                .map_err(|_| QuotaFetchError::new(QuotaStatus::Error, "账号标识包含无效字符"))?,
        );
    }

    let response = tokio::time::timeout(
        Duration::from_secs(30),
        client.get(endpoint).headers(headers).send(),
    )
    .await
    .map_err(|_| QuotaFetchError::new(QuotaStatus::Error, "OpenAI 额度查询超时"))?
    .map_err(|_| QuotaFetchError::new(QuotaStatus::Error, "无法连接 OpenAI 额度服务"))?;
    let status = response.status();
    if !status.is_success() {
        let (quota_status, message) = match status {
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => (
                QuotaStatus::Unauthorized,
                "OpenAI 登录已失效或无额度查询权限".to_string(),
            ),
            reqwest::StatusCode::TOO_MANY_REQUESTS => (
                QuotaStatus::RateLimited,
                "OpenAI 额度查询过于频繁，请稍后重试".to_string(),
            ),
            _ => (
                QuotaStatus::Error,
                format!("OpenAI 额度接口返回 HTTP {}", status.as_u16()),
            ),
        };
        return Err(QuotaFetchError::new(quota_status, message));
    }

    let payload = read_bounded_json(response).await?;
    let quota = parse_quota_payload(&payload, chrono::Utc::now().timestamp())
        .map_err(|message| QuotaFetchError::new(QuotaStatus::Error, message))?;
    Ok(quota)
}

fn should_send_account_id(account: &StoredOfficialAccount) -> bool {
    let account_id = account.account_id.trim();
    !account_id.is_empty()
        && !(account.source == OfficialAccountSource::ProxyImport
            && account_id.starts_with("proxy-"))
}

async fn read_bounded_json(response: reqwest::Response) -> Result<Value, QuotaFetchError> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|_| QuotaFetchError::new(QuotaStatus::Error, "无法读取 OpenAI 额度响应"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_QUOTA_RESPONSE_BYTES {
            return Err(QuotaFetchError::new(
                QuotaStatus::Error,
                "OpenAI 额度响应内容过大",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| QuotaFetchError::new(QuotaStatus::Error, "OpenAI 额度接口没有返回有效 JSON"))
}

fn parse_quota_payload(payload: &Value, now: i64) -> Result<QuotaData, String> {
    let rate_limit = payload
        .get("rate_limit")
        .or_else(|| payload.get("data").and_then(|data| data.get("rate_limit")))
        .ok_or_else(|| "OpenAI 官方额度响应中没有 5H/7D 窗口".to_string())?;
    let primary = rate_limit
        .get("primary_window")
        .filter(|window| !window.is_null())
        .map(|window| parse_window(window, now))
        .transpose()?;
    let secondary = rate_limit
        .get("secondary_window")
        .filter(|window| !window.is_null())
        .map(|window| parse_window(window, now))
        .transpose()?;
    if primary.is_none() && secondary.is_none() {
        return Err("OpenAI 官方额度响应中没有可用的 5H/7D 窗口".into());
    }
    Ok(QuotaData::Windowed { primary, secondary })
}

fn parse_window(value: &Value, now: i64) -> Result<QuotaWindow, String> {
    let used = value
        .get("used_percent")
        .and_then(Value::as_i64)
        .ok_or_else(|| "OpenAI 额度窗口缺少 used_percent".to_string())?;
    let used_percent =
        u8::try_from(used).map_err(|_| "OpenAI 额度窗口 used_percent 超出 0 到 100")?;
    if used_percent > 100 {
        return Err("OpenAI 额度窗口 used_percent 超出 0 到 100".into());
    }
    let window_seconds = value
        .get("limit_window_seconds")
        .and_then(Value::as_i64)
        .filter(|seconds| *seconds > 0);
    let reset_at = value.get("reset_at").and_then(Value::as_i64).or_else(|| {
        value
            .get("reset_after_seconds")
            .and_then(Value::as_i64)
            .filter(|seconds| *seconds >= 0)
            .map(|seconds| now.saturating_add(seconds))
    });
    Ok(QuotaWindow {
        used_percent,
        remaining_percent: 100 - used_percent,
        window_seconds,
        reset_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CodexAuthCredential, CodexAuthTokens, ProviderAccountQuota};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn account(source: OfficialAccountSource, account_id: &str) -> StoredOfficialAccount {
        StoredOfficialAccount {
            id: "saved-account".into(),
            name: "Account".into(),
            account_id: account_id.into(),
            email: String::new(),
            credential: CodexAuthCredential {
                auth_mode: "chatgpt".into(),
                openai_api_key: None,
                tokens: CodexAuthTokens {
                    id_token: String::new(),
                    access_token: "at-test-secret".into(),
                    refresh_token: String::new(),
                    account_id: account_id.into(),
                },
                last_refresh: "2026-07-31T00:00:00Z".into(),
            },
            source,
            expires_at: None,
            quota: ProviderAccountQuota::default(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[tokio::test]
    async fn fetches_official_windows_with_required_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
            assert!(request.starts_with("get /usage http/1.1"));
            assert!(request.contains("authorization: bearer at-test-secret"));
            assert!(request.contains("chatgpt-account-id: workspace-1"));
            assert!(request.contains("openai-beta: codex-1"));
            assert!(request.contains("originator: codex desktop"));
            let body = r#"{"rate_limit":{"primary_window":{"used_percent":20,"limit_window_seconds":18000},"secondary_window":{"used_percent":40,"limit_window_seconds":604800}}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let quota = fetch_quota_from(
            &client,
            &account(OfficialAccountSource::OpenAiOauth, "workspace-1"),
            &format!("http://{address}/usage"),
        )
        .await
        .unwrap();

        let QuotaData::Windowed { primary, secondary } = quota;
        assert_eq!(primary.unwrap().remaining_percent, 80);
        assert_eq!(secondary.unwrap().remaining_percent, 60);
        server.join().unwrap();
    }

    #[test]
    fn omits_synthetic_proxy_account_id() {
        assert!(!should_send_account_id(&account(
            OfficialAccountSource::ProxyImport,
            "proxy-0123456789"
        )));
        assert!(should_send_account_id(&account(
            OfficialAccountSource::ProxyImport,
            "workspace-1"
        )));
    }
}
