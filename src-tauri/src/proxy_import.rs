use serde_json::Value;

pub struct ImportedProxyCredential {
    pub access_token: Option<String>,
    pub id_token: Option<String>,
    pub refresh_token: Option<String>,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub suggested_name: Option<String>,
}

pub fn parse_proxy_credential(input: &str) -> Result<ImportedProxyCredential, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("请粘贴反代 Token 或账号 JSON".into());
    }

    if input.starts_with('[') {
        return Err("一次只能导入一个反代号，请粘贴单个账号 JSON".into());
    }

    if input.starts_with('{') {
        let value: Value =
            serde_json::from_str(input).map_err(|_| "账号 JSON 格式不正确".to_string())?;
        let access_token = find_string(
            &value,
            &[
                "access_token",
                "accessToken",
                "personal_access_token",
                "personalAccessToken",
            ],
        );
        let refresh_token = find_string(&value, &["refresh_token", "refreshToken"]);
        if access_token.is_none() && refresh_token.is_none() {
            return Err("账号 JSON 中没有找到 accessToken 或 refresh_token".into());
        }
        return Ok(ImportedProxyCredential {
            access_token,
            id_token: find_string(&value, &["id_token", "idToken"]),
            refresh_token,
            account_id: find_string(
                &value,
                &[
                    "chatgpt_account_id",
                    "chatgptAccountId",
                    "account_id",
                    "accountId",
                ],
            ),
            email: find_string(&value, &["email"]),
            suggested_name: find_string(&value, &["name", "account_name", "accountName"]),
        });
    }

    Ok(ImportedProxyCredential {
        access_token: Some(input.to_owned()),
        id_token: None,
        refresh_token: None,
        account_id: None,
        email: None,
        suggested_name: None,
    })
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(value) = object
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Some(value.to_owned());
                }
            }
            object.values().find_map(|child| find_string(child, keys))
        }
        Value::Array(items) => items.iter().find_map(|child| find_string(child, keys)),
        _ => None,
    }
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
                    "name": "日常反代号",
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
        assert_eq!(imported.suggested_name.as_deref(), Some("日常反代号"));
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
    fn rejects_multi_account_arrays_instead_of_mixing_credentials() {
        let error = parse_proxy_credential(
            r#"[
                {"access_token":"first-access","account_id":"first"},
                {"refresh_token":"second-refresh","account_id":"second"}
            ]"#,
        )
        .err()
        .unwrap();

        assert_eq!(error, "一次只能导入一个反代号，请粘贴单个账号 JSON");
    }
}
