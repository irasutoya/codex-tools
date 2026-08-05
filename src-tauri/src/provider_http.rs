use crate::models::{AppError, ProviderAccount, ProviderProfile};

pub fn models_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.rsplit('/').next().is_some_and(|part| {
        part.starts_with('v') && part[1..].chars().all(|value| value.is_ascii_digit())
    }) {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

pub fn provider_test_succeeded(status: reqwest::StatusCode) -> bool {
    status.is_success()
}

pub fn custom_headers(
    provider: &ProviderProfile,
    account: &ProviderAccount,
) -> Result<reqwest::header::HeaderMap, AppError> {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in provider.headers.iter().chain(&account.headers) {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| AppError::InvalidConfig(format!("请求头名称无效：{name}")))?;
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|_| AppError::InvalidConfig("请求头内容包含无效字符。".into()))?;
        headers.insert(name, value);
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_test_only_accepts_success_responses() {
        assert!(provider_test_succeeded(reqwest::StatusCode::OK));
        assert!(provider_test_succeeded(reqwest::StatusCode::NO_CONTENT));
        assert!(!provider_test_succeeded(reqwest::StatusCode::FOUND));
        assert!(!provider_test_succeeded(reqwest::StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn model_endpoint_preserves_versioned_api_roots() {
        assert_eq!(
            models_endpoint("https://api.example.test/v1/"),
            "https://api.example.test/v1/models"
        );
        assert_eq!(
            models_endpoint("https://api.example.test/openai/v2"),
            "https://api.example.test/openai/v2/models"
        );
        assert_eq!(
            models_endpoint("https://api.example.test/openai"),
            "https://api.example.test/openai/v1/models"
        );
    }
}
