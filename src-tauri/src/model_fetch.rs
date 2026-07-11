use crate::models::{AppError, FetchedModel, ProviderAccount, ProviderProfile};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::time::Duration;

const KNOWN_COMPAT_SUFFIXES: &[&str] = &[
    "/api/claudecode",
    "/api/anthropic",
    "/apps/anthropic",
    "/api/coding",
    "/claudecode",
    "/anthropic",
    "/step_plan",
    "/coding",
    "/claude",
];

#[derive(Deserialize)]
struct ModelsResponse {
    data: Option<Vec<FetchedModel>>,
}

pub async fn fetch_models(
    provider: &ProviderProfile,
    account: &ProviderAccount,
) -> Result<Vec<FetchedModel>, AppError> {
    let api_key = account.api_key.as_deref().unwrap_or_default().trim();
    if api_key.is_empty() {
        return Err(AppError::InvalidConfig("获取模型需要 API Key".into()));
    }
    let headers = crate::protocol_proxy::build_upstream_headers(
        api_key,
        &provider.headers,
        &account.headers,
    )?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let mut last_status = None;
    for url in build_models_url_candidates(&provider.base_url)? {
        let response = client
            .get(&url)
            .headers(headers.clone())
            .send()
            .await
            .map_err(|error| AppError::InvalidConfig(format!("获取模型失败：{error}")))?;
        let status = response.status();
        if status.is_success() {
            let mut models = response
                .json::<ModelsResponse>()
                .await
                .map_err(|error| AppError::InvalidConfig(format!("模型响应格式无效：{error}")))?
                .data
                .unwrap_or_default();
            models.retain(|model| !model.id.trim().is_empty());
            models.sort_by(|left, right| left.id.cmp(&right.id));
            models.dedup_by(|left, right| left.id == right.id);
            return Ok(models);
        }
        if matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
        ) {
            last_status = Some(status.as_u16());
            continue;
        }
        return Err(AppError::InvalidConfig(format!(
            "获取模型失败（HTTP {}），响应详情已隐藏",
            status.as_u16()
        )));
    }
    Err(AppError::InvalidConfig(format!(
        "所有模型端点均不可用（最后 HTTP {}）",
        last_status.unwrap_or(0)
    )))
}

pub fn build_models_url_candidates(base_url: &str) -> Result<Vec<String>, AppError> {
    let base = base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(AppError::InvalidConfig("Base URL 不能为空".into()));
    }
    let mut candidates = Vec::new();
    if ends_with_version_segment(base) {
        candidates.push(format!("{base}/models"));
        if !base.ends_with("/v1") {
            candidates.push(format!("{base}/v1/models"));
        }
    } else {
        candidates.push(format!("{base}/v1/models"));
    }
    if let Some(root) = strip_compat_suffix(base) {
        let root = root.trim_end_matches('/');
        candidates.push(format!("{root}/v1/models"));
        candidates.push(format!("{root}/models"));
    }
    let mut unique = Vec::new();
    for url in candidates {
        if !unique.contains(&url) {
            unique.push(url);
        }
    }
    Ok(unique)
}

fn ends_with_version_segment(url: &str) -> bool {
    url.rsplit('/')
        .next()
        .unwrap_or_default()
        .strip_prefix('v')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

fn strip_compat_suffix(base_url: &str) -> Option<&str> {
    KNOWN_COMPAT_SUFFIXES
        .iter()
        .find_map(|suffix| base_url.strip_suffix(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_cc_switch_candidate_order() {
        assert_eq!(
            build_models_url_candidates("https://example.test/v1").unwrap(),
            vec!["https://example.test/v1/models"]
        );
        assert_eq!(
            build_models_url_candidates("https://api.deepseek.com/anthropic").unwrap(),
            vec![
                "https://api.deepseek.com/anthropic/v1/models",
                "https://api.deepseek.com/v1/models",
                "https://api.deepseek.com/models"
            ]
        );
    }
}
