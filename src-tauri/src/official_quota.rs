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
    retryable: bool,
}

impl QuotaFetchError {
    fn new(status: QuotaStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            retryable: false,
        }
    }

    fn retryable(message: impl Into<String>) -> Self {
        Self {
            status: QuotaStatus::Error,
            message: message.into(),
            retryable: true,
        }
    }

    pub fn is_retryable(&self) -> bool {
        self.retryable
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
            "账号缺少 Access Token，请重新进行官方授权或导入 Cookie 数据",
        ));
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {access_token}"))
            .map_err(|_| QuotaFetchError::new(QuotaStatus::Error, "Access Token 包含无效字符"))?,
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
    .map_err(|_| QuotaFetchError::retryable("OpenAI 额度查询超时，请稍后重试"))?
    .map_err(|error| {
        if error.is_connect() || error.is_timeout() {
            QuotaFetchError::retryable("无法连接 OpenAI 额度服务，请检查网络或系统代理")
        } else {
            QuotaFetchError::new(QuotaStatus::Error, "无法完成 OpenAI 额度请求")
        }
    })?;
    let status = response.status();
    if !status.is_success() {
        let (quota_status, message) = match status {
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => (
                QuotaStatus::Unauthorized,
                "OpenAI 拒绝了额度查询请求，登录凭据可能已失效或当前账号没有查询权限".to_string(),
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
    let quota =
        parse_quota_payload(&payload, chrono::Utc::now().timestamp()).map_err(
            |error| match error {
                QuotaParseError::Unsupported(message) => {
                    QuotaFetchError::new(QuotaStatus::Unsupported, message)
                }
                QuotaParseError::Invalid(message) => {
                    QuotaFetchError::new(QuotaStatus::Error, message)
                }
            },
        )?;
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

#[derive(Debug)]
enum QuotaParseError {
    Unsupported(String),
    Invalid(String),
}

fn parse_quota_payload(payload: &Value, now: i64) -> Result<QuotaData, QuotaParseError> {
    let rate_limit = payload
        .get("rate_limit")
        .or_else(|| payload.get("data").and_then(|data| data.get("rate_limit")));
    let Some(rate_limit) = rate_limit.filter(|value| !value.is_null()) else {
        return Err(QuotaParseError::Unsupported(
            "OpenAI 返回的数据中没有 5 小时或 7 天用量窗口，该账号暂不支持额度查询。".into(),
        ));
    };
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
        return Err(QuotaParseError::Unsupported(
            "OpenAI 返回的 5 小时和 7 天用量窗口均不可用，该账号暂不支持额度查询。".into(),
        ));
    }
    Ok(QuotaData::Windowed { primary, secondary })
}

fn parse_window(value: &Value, now: i64) -> Result<QuotaWindow, QuotaParseError> {
    let used_percent = parse_percent(value.get("used_percent"))?;
    let remaining_percent = (100.0 - used_percent).max(0.0);
    let window_seconds = value
        .get("limit_window_seconds")
        .and_then(parse_number)
        .filter(|seconds| *seconds > 0);
    let reset_at = value.get("reset_at").and_then(parse_timestamp).or_else(|| {
        value
            .get("reset_after_seconds")
            .and_then(parse_number)
            .filter(|seconds| *seconds >= 0)
            .map(|seconds| now.saturating_add(seconds))
    });
    Ok(QuotaWindow {
        used_percent,
        remaining_percent,
        window_seconds,
        reset_at,
    })
}

fn parse_percent(value: Option<&Value>) -> Result<f64, QuotaParseError> {
    let Some(value) = value else {
        return Err(QuotaParseError::Invalid(
            "OpenAI 返回的额度窗口缺少 used_percent 字段".into(),
        ));
    };
    parse_non_negative_f64(value).ok_or_else(|| {
        QuotaParseError::Invalid("OpenAI 返回的额度窗口包含无效的 used_percent".into())
    })
}

fn parse_number(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| parse_non_negative_f64(value).map(|number| number as i64))
}

fn parse_non_negative_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| {
            value
                .as_str()
                .and_then(|text| text.trim().parse::<f64>().ok())
        })
        .filter(|number| number.is_finite() && *number >= 0.0)
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    parse_number(value).or_else(|| {
        value.as_str().and_then(|text| {
            chrono::DateTime::parse_from_rfc3339(text.trim())
                .ok()
                .map(|time| time.timestamp())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CodexAuthCredential, CodexAuthTokens, ProviderAccountQuota};
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve_usage_response(
        body: &'static str,
        assert_request: impl FnOnce(&str) + Send + 'static,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]).to_ascii_lowercase();
            assert_request(&request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        (format!("http://{address}/usage"), server)
    }

    fn account(source: OfficialAccountSource, account_id: &str) -> StoredOfficialAccount {
        StoredOfficialAccount {
            id: "saved-account".into(),
            name: "Account".into(),
            remark: String::new(),
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
        let body = r#"{"rate_limit":{"primary_window":{"used_percent":20,"limit_window_seconds":18000},"secondary_window":{"used_percent":40,"limit_window_seconds":604800}}}"#;
        let (endpoint, server) = serve_usage_response(body, |request| {
            assert!(request.starts_with("get /usage http/1.1"));
            assert!(request.contains("authorization: bearer at-test-secret"));
            assert!(request.contains("chatgpt-account-id: workspace-1"));
            assert!(request.contains("openai-beta: codex-1"));
            assert!(request.contains("originator: codex desktop"));
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let quota = fetch_quota_from(
            &client,
            &account(OfficialAccountSource::OpenAiOauth, "workspace-1"),
            &endpoint,
        )
        .await
        .unwrap();

        let QuotaData::Windowed { primary, secondary } = quota;
        assert_eq!(primary.unwrap().remaining_percent, 80.0);
        assert_eq!(secondary.unwrap().remaining_percent, 60.0);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn marks_unrecognized_payload_as_unsupported() {
        let (endpoint, server) = serve_usage_response(r#"{"usage":{}}"#, |request| {
            assert!(request.starts_with("get /usage http/1.1"));
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();

        let error = fetch_quota_from(
            &client,
            &account(OfficialAccountSource::OpenAiOauth, "workspace-1"),
            &endpoint,
        )
        .await
        .unwrap_err();

        assert_eq!(error.status, QuotaStatus::Unsupported);
        server.join().unwrap();
    }

    #[test]
    fn accepts_percent_variants_and_missing_window_fields() {
        let window = parse_window(
            &json!({
                "used_percent": "125.5",
                "limit_window_seconds": "18000"
            }),
            1_700_000_000,
        )
        .unwrap();
        assert_eq!(window.used_percent, 125.5);
        assert_eq!(window.remaining_percent, 0.0);
        assert_eq!(window.window_seconds, Some(18_000));
        assert_eq!(window.reset_at, None);

        let window = parse_window(&json!({"used_percent": 20}), 1_700_000_000).unwrap();
        assert_eq!(window.used_percent, 20.0);
        assert_eq!(window.remaining_percent, 80.0);
        assert_eq!(window.window_seconds, None);
        assert_eq!(window.reset_at, None);
    }

    #[test]
    fn parses_positive_numbers_and_rejects_invalid_percent_values() {
        assert_eq!(parse_percent(Some(&json!("25.5"))).unwrap(), 25.5);
        assert_eq!(parse_percent(Some(&json!(0.0))).unwrap(), 0.0);
        assert_eq!(parse_number(&json!("12.75")), Some(12));

        assert!(matches!(
            parse_percent(None),
            Err(QuotaParseError::Invalid(_))
        ));
        assert!(matches!(
            parse_percent(Some(&json!(-1.0))),
            Err(QuotaParseError::Invalid(_))
        ));
        assert!(matches!(
            parse_percent(Some(&json!("NaN"))),
            Err(QuotaParseError::Invalid(_))
        ));
        assert!(matches!(
            parse_percent(Some(&json!("inf"))),
            Err(QuotaParseError::Invalid(_))
        ));
    }

    #[test]
    fn computes_reset_at_from_numeric_and_rfc3339_fields() {
        let window = parse_window(
            &json!({
                "used_percent": 10,
                "reset_after_seconds": 3600
            }),
            1000,
        )
        .unwrap();
        assert_eq!(window.reset_at, Some(4600));

        let window = parse_window(
            &json!({
                "used_percent": 10,
                "reset_at": "2026-08-04T12:00:00Z"
            }),
            0,
        )
        .unwrap();
        assert_eq!(
            window.reset_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-08-04T12:00:00Z")
                    .unwrap()
                    .timestamp()
            )
        );
    }

    #[test]
    fn classifies_unrecognized_and_invalid_payloads() {
        let error = parse_quota_payload(&json!({"data": {"usage": []}}), 0).unwrap_err();
        assert!(matches!(error, QuotaParseError::Unsupported(_)));

        let error = parse_quota_payload(
            &json!({"rate_limit": {"primary_window": null, "secondary_window": null}}),
            0,
        )
        .unwrap_err();
        assert!(matches!(error, QuotaParseError::Unsupported(_)));

        let error = parse_quota_payload(
            &json!({"data": {"rate_limit": {"primary_window": {"used_percent": -1}}}}),
            0,
        )
        .unwrap_err();
        assert!(matches!(error, QuotaParseError::Invalid(_)));
    }

    #[test]
    fn accepts_rate_limit_under_data_wrapper() {
        let quota = parse_quota_payload(
            &json!({
                "data": {
                    "rate_limit": {
                        "primary_window": {"used_percent": 30, "limit_window_seconds": 18000}
                    }
                }
            }),
            0,
        )
        .unwrap();
        let QuotaData::Windowed { primary, secondary } = quota;
        assert_eq!(primary.unwrap().remaining_percent, 70.0);
        assert!(secondary.is_none());
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

    #[test]
    fn marks_connection_failures_as_safe_to_retry() {
        assert!(QuotaFetchError::retryable("暂时不可达").is_retryable());
        assert!(!QuotaFetchError::new(QuotaStatus::Error, "响应无效").is_retryable());
    }
}
