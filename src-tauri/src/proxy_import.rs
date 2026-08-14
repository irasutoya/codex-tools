use serde_json::Value;

const MAX_COOKIE_CREDENTIAL_LENGTH: usize = 262_144;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyCredentialFormat {
    Cpa,
    Sub2api,
    Cockpit,
    NineRouter,
    RawAccessToken,
    RawRefreshToken,
    GenericJson,
}

impl ProxyCredentialFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpa => "CPA",
            Self::Sub2api => "Sub2API",
            Self::Cockpit => "Cockpit",
            Self::NineRouter => "9router",
            Self::RawAccessToken => "Access Token",
            Self::RawRefreshToken => "纯 RT",
            Self::GenericJson => "通用 Token JSON",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImportedProxyCredential {
    pub access_token: Option<String>,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub suggested_name: Option<String>,
    pub expires_at: Option<i64>,
    pub source_format: ProxyCredentialFormat,
    pub is_personal_access_token: bool,
}

#[cfg(test)]
fn parse_proxy_credential(input: &str) -> Result<ImportedProxyCredential, String> {
    let mut credentials = parse_proxy_credentials(input)?;
    if credentials.len() != 1 {
        return Err("检测到多个反代账号，请使用批量导入入口。".into());
    }
    Ok(credentials.remove(0))
}

pub fn parse_proxy_credentials(input: &str) -> Result<Vec<ImportedProxyCredential>, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("请粘贴 RT、Access Token 或反代账号 JSON。".into());
    }

    if input.chars().count() > MAX_COOKIE_CREDENTIAL_LENGTH {
        return Err("反代账号内容不能超过 262,144 个字符。".into());
    }

    if input.starts_with('{') || input.starts_with('[') {
        let value: Value =
            serde_json::from_str(input).map_err(|_| "反代账号 JSON 格式不正确。".to_string())?;
        let credentials = parse_json_credentials(&value)?;
        if credentials.is_empty() {
            return Err("反代账号 JSON 中没有找到可导入的账号。".into());
        }
        return Ok(credentials);
    }

    let likely_refresh_token = looks_like_refresh_token(input);
    let likely_access_token =
        input.starts_with("at-") || looks_like_jwt(input) || !likely_refresh_token;
    Ok(vec![ImportedProxyCredential {
        access_token: likely_access_token.then(|| input.to_owned()),
        id_token: None,
        refresh_token: (!likely_access_token).then(|| input.to_owned()),
        account_id: None,
        email: None,
        suggested_name: None,
        expires_at: None,
        source_format: if likely_access_token {
            ProxyCredentialFormat::RawAccessToken
        } else {
            ProxyCredentialFormat::RawRefreshToken
        },
        is_personal_access_token: input.starts_with("at-"),
    }])
}

fn looks_like_jwt(value: &str) -> bool {
    let mut parts = value.split('.');
    parts.next().is_some_and(|part| !part.is_empty())
        && parts.next().is_some_and(|part| !part.is_empty())
        && parts.next().is_some_and(|part| !part.is_empty())
        && parts.next().is_none()
}

fn looks_like_refresh_token(value: &str) -> bool {
    value.starts_with("rt-")
        || value.starts_with("rt_")
        || value.starts_with("refresh-")
        || value.starts_with("refresh_")
}

fn parse_json_credentials(value: &Value) -> Result<Vec<ImportedProxyCredential>, String> {
    match value {
        Value::Array(items) => {
            let mut credentials = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let mut parsed = parse_json_credentials(item)
                    .map_err(|error| format!("第 {} 条反代账号：{error}", index + 1))?;
                credentials.append(&mut parsed);
            }
            Ok(credentials)
        }
        Value::Object(object) => {
            if let Some(accounts) = object.get("accounts").and_then(Value::as_array) {
                let mut credentials = Vec::with_capacity(accounts.len());
                for (index, account) in accounts.iter().enumerate() {
                    credentials.push(
                        extract_json_credential(account, ProxyCredentialFormat::Sub2api)
                            .map_err(|error| format!("Sub2API 第 {} 个账号：{error}", index + 1))?,
                    );
                }
                return Ok(credentials);
            }

            let format = detect_json_format(value);
            extract_json_credential(value, format).map(|credential| vec![credential])
        }
        _ => Err("账号记录必须是 JSON 对象。".into()),
    }
}

fn detect_json_format(value: &Value) -> ProxyCredentialFormat {
    let provider = first_string(value, &["/provider"]);
    if provider
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("codex"))
        || value.pointer("/providerSpecificData").is_some()
    {
        return ProxyCredentialFormat::NineRouter;
    }

    if first_string(value, &["/platform"])
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("openai"))
        && value.pointer("/credentials").is_some()
    {
        return ProxyCredentialFormat::Sub2api;
    }

    if first_string(value, &["/type"])
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case("codex"))
    {
        let cpa_markers = [
            "/chatgpt_account_id",
            "/chatgpt_plan_type",
            "/id_token_synthetic",
            "/disabled",
        ];
        if cpa_markers
            .iter()
            .any(|pointer| value.pointer(pointer).is_some())
        {
            return ProxyCredentialFormat::Cpa;
        }
        return ProxyCredentialFormat::Cockpit;
    }

    ProxyCredentialFormat::GenericJson
}

fn extract_json_credential(
    value: &Value,
    source_format: ProxyCredentialFormat,
) -> Result<ImportedProxyCredential, String> {
    let explicit_personal_access_token =
        first_string(value, &["/personal_access_token", "/personalAccessToken"]).is_some();
    let access_token = first_string(
        value,
        &[
            "/access_token",
            "/accessToken",
            "/personal_access_token",
            "/personalAccessToken",
            "/token/access_token",
            "/token/accessToken",
            "/tokens/access_token",
            "/tokens/accessToken",
            "/credentials/access_token",
            "/credentials/accessToken",
            "/credentials/token",
        ],
    );
    let refresh_token = first_string(
        value,
        &[
            "/refresh_token",
            "/refreshToken",
            "/token/refresh_token",
            "/token/refreshToken",
            "/tokens/refresh_token",
            "/tokens/refreshToken",
            "/credentials/refresh_token",
            "/credentials/refreshToken",
        ],
    );
    if access_token.is_none() && refresh_token.is_none() {
        return Err("没有找到 accessToken/access_token 或 refreshToken/refresh_token。".into());
    }

    let mut account_id = first_string(
        value,
        &[
            "/chatgpt_account_id",
            "/chatgptAccountId",
            "/account_id",
            "/accountId",
            "/account/id",
            "/account/accountId",
            "/account/account_id",
            "/account/chatgpt_account_id",
            "/token/account_id",
            "/tokens/account_id",
            "/tokens/accountId",
            "/credentials/chatgpt_account_id",
            "/credentials/chatgptAccountId",
            "/credentials/account_id",
            "/credentials/accountId",
            "/providerSpecificData/chatgptAccountId",
            "/providerSpecificData/chatgpt_account_id",
        ],
    );
    if account_id.is_none() && source_format == ProxyCredentialFormat::NineRouter {
        account_id = first_string(value, &["/id"]);
    }

    Ok(ImportedProxyCredential {
        access_token,
        id_token: first_string(
            value,
            &[
                "/id_token",
                "/idToken",
                "/token/id_token",
                "/token/idToken",
                "/tokens/id_token",
                "/tokens/idToken",
                "/credentials/id_token",
                "/credentials/idToken",
            ],
        ),
        refresh_token,
        account_id,
        email: first_string(
            value,
            &[
                "/email",
                "/user/email",
                "/account/email",
                "/credentials/email",
                "/providerSpecificData/email",
            ],
        ),
        suggested_name: first_string(
            value,
            &[
                "/name",
                "/account_name",
                "/accountName",
                "/account/name",
                "/user/name",
            ],
        ),
        expires_at: first_timestamp(
            value,
            &[
                "/expires_at",
                "/expiresAt",
                "/expired",
                "/expires",
                "/token/expires_at",
                "/token/expiresAt",
                "/tokens/expires_at",
                "/tokens/expiresAt",
                "/credentials/expires_at",
                "/credentials/expiresAt",
            ],
        ),
        source_format,
        is_personal_access_token: explicit_personal_access_token,
    })
}

fn first_string(value: &Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn first_timestamp(value: &Value, pointers: &[&str]) -> Option<i64> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(parse_timestamp))
}

fn parse_timestamp(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return normalize_unix_timestamp(number);
    }
    if let Some(number) = value.as_u64().and_then(|value| i64::try_from(value).ok()) {
        return normalize_unix_timestamp(number);
    }
    let raw = value.as_str()?.trim();
    if let Ok(number) = raw.parse::<i64>() {
        return normalize_unix_timestamp(number);
    }
    chrono::DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|date_time| date_time.timestamp())
}

fn normalize_unix_timestamp(value: i64) -> Option<i64> {
    if value <= 0 {
        return None;
    }
    Some(if value > 1_000_000_000_000 {
        value / 1000
    } else {
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_tokens_and_identity_from_nested_json() {
        let imported = parse_proxy_credential(
            r#"{
                "account": {
                    "email": "person@example.test",
                    "name": "日常 Cookie 账号",
                    "accountId": "workspace-1"
                },
                "tokens": {
                    "id_token": "id-secret",
                    "accessToken": "at-proxy-secret",
                    "refresh_token": "refresh-secret"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(imported.access_token.as_deref(), Some("at-proxy-secret"));
        assert_eq!(imported.id_token.as_deref(), Some("id-secret"));
        assert_eq!(imported.refresh_token.as_deref(), Some("refresh-secret"));
        assert_eq!(imported.account_id.as_deref(), Some("workspace-1"));
        assert_eq!(imported.email.as_deref(), Some("person@example.test"));
        assert_eq!(imported.suggested_name.as_deref(), Some("日常 Cookie 账号"));
    }

    #[test]
    fn accepts_refresh_token_only_json() {
        let imported = parse_proxy_credential(r#"{"refresh_token":"refresh-secret"}"#).unwrap();

        assert!(imported.access_token.is_none());
        assert_eq!(imported.refresh_token.as_deref(), Some("refresh-secret"));
    }

    #[test]
    fn accepts_plain_personal_access_token() {
        let imported = parse_proxy_credential("at-proxy-secret").unwrap();

        assert_eq!(imported.access_token.as_deref(), Some("at-proxy-secret"));
        assert!(imported.refresh_token.is_none());
    }

    #[test]
    fn imports_cpa_account_array_without_mixing_credentials() {
        let imported = parse_proxy_credentials(
            r#"[
                {
                    "type":"codex",
                    "id_token":"first-id",
                    "access_token":"first-access",
                    "refresh_token":"",
                    "account_id":"first",
                    "chatgpt_account_id":"first",
                    "expired":"2030-01-01T00:00:00Z"
                },
                {
                    "type":"codex",
                    "id_token":"second-id",
                    "access_token":"second-access",
                    "refresh_token":"second-refresh",
                    "account_id":"second"
                }
            ]"#,
        )
        .unwrap();

        assert_eq!(imported.len(), 2);
        assert_eq!(imported[0].source_format, ProxyCredentialFormat::Cpa);
        assert_eq!(imported[0].access_token.as_deref(), Some("first-access"));
        assert_eq!(imported[0].account_id.as_deref(), Some("first"));
        assert_eq!(imported[0].expires_at, Some(1_893_456_000));
        assert_eq!(imported[1].access_token.as_deref(), Some("second-access"));
        assert_eq!(imported[1].refresh_token.as_deref(), Some("second-refresh"));
        assert_eq!(imported[1].account_id.as_deref(), Some("second"));
    }

    #[test]
    fn imports_sub2api_accounts_from_credentials_envelope() {
        let imported = parse_proxy_credentials(
            r#"{
                "exported_at":"2030-01-01T00:00:00Z",
                "proxies":[],
                "accounts":[
                    {
                        "name":"first@example.test",
                        "platform":"openai",
                        "type":"oauth",
                        "credentials":{
                            "access_token":"first-access",
                            "chatgpt_account_id":"first",
                            "email":"first@example.test",
                            "expires_at":"2030-01-01T00:00:00Z"
                        }
                    },
                    {
                        "name":"second@example.test",
                        "platform":"openai",
                        "type":"oauth",
                        "credentials":{
                            "access_token":"second-access",
                            "refresh_token":"second-refresh",
                            "chatgpt_account_id":"second"
                        }
                    }
                ]
            }"#,
        )
        .unwrap();

        assert_eq!(imported.len(), 2);
        assert!(
            imported
                .iter()
                .all(|item| item.source_format == ProxyCredentialFormat::Sub2api)
        );
        assert_eq!(imported[0].access_token.as_deref(), Some("first-access"));
        assert_eq!(imported[0].account_id.as_deref(), Some("first"));
        assert_eq!(imported[0].email.as_deref(), Some("first@example.test"));
        assert_eq!(imported[1].access_token.as_deref(), Some("second-access"));
        assert_eq!(imported[1].refresh_token.as_deref(), Some("second-refresh"));
    }

    #[test]
    fn imports_cockpit_account() {
        let imported = parse_proxy_credentials(
            r#"{
                "type":"codex",
                "id_token":"cockpit-id",
                "access_token":"cockpit-access",
                "refresh_token":"",
                "account_id":"cockpit-account",
                "last_refresh":"2030-01-01T00:00:00Z",
                "email":"cockpit@example.test",
                "expired":"2030-01-02T00:00:00Z"
            }"#,
        )
        .unwrap();

        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].source_format, ProxyCredentialFormat::Cockpit);
        assert_eq!(imported[0].id_token.as_deref(), Some("cockpit-id"));
        assert_eq!(imported[0].refresh_token, None);
        assert_eq!(imported[0].expires_at, Some(1_893_542_400));
    }

    #[test]
    fn imports_nine_router_account() {
        let imported = parse_proxy_credentials(
            r#"{
                "accessToken":"router-access",
                "refreshToken":"router-refresh",
                "expiresAt":"2030-01-01T00:00:00Z",
                "providerSpecificData":{
                    "chatgptAccountId":"router-account",
                    "chatgptPlanType":"plus"
                },
                "id":"router-account",
                "provider":"codex",
                "authType":"oauth",
                "name":"Router Account",
                "email":"router@example.test"
            }"#,
        )
        .unwrap();

        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].source_format, ProxyCredentialFormat::NineRouter);
        assert_eq!(imported[0].access_token.as_deref(), Some("router-access"));
        assert_eq!(imported[0].refresh_token.as_deref(), Some("router-refresh"));
        assert_eq!(imported[0].account_id.as_deref(), Some("router-account"));
        assert_eq!(
            imported[0].suggested_name.as_deref(),
            Some("Router Account")
        );
    }

    #[test]
    fn treats_prefixed_plain_token_as_refresh_token() {
        let imported = parse_proxy_credentials("rt-refresh-token-secret").unwrap();

        assert_eq!(imported.len(), 1);
        assert_eq!(
            imported[0].source_format,
            ProxyCredentialFormat::RawRefreshToken
        );
        assert!(imported[0].access_token.is_none());
        assert_eq!(
            imported[0].refresh_token.as_deref(),
            Some("rt-refresh-token-secret")
        );
    }

    #[test]
    fn preserves_explicit_personal_access_token_semantics() {
        let imported = parse_proxy_credentials(
            r#"{"personal_access_token":"pat-secret","account_id":"pat-account"}"#,
        )
        .unwrap();

        assert!(imported[0].is_personal_access_token);
        assert_eq!(imported[0].access_token.as_deref(), Some("pat-secret"));
    }

    #[test]
    fn rejects_oversized_cookie_content_before_parsing() {
        let input = "x".repeat(MAX_COOKIE_CREDENTIAL_LENGTH + 1);
        let error = parse_proxy_credential(&input).err().unwrap();

        assert_eq!(error, "反代账号内容不能超过 262,144 个字符。");
    }
}
