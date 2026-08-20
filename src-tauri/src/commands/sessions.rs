use crate::{
    activation::ensure_codex_stopped,
    codex,
    models::{AppError, PageResult, RepairResult, RepairScan, SessionSummary},
    platform, provider_sync,
    session_index::{self, SessionIndex},
    storage::Store,
};
use std::path::PathBuf;
use tauri::State;

const MAX_SESSION_QUERY_CHARS: usize = 256;

pub(crate) async fn scan_home(home: PathBuf) -> Result<RepairScan, AppError> {
    tokio::task::spawn_blocking(move || provider_sync::scan(&home))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))
}

pub(crate) async fn repair_home(
    store: &Store,
    home: PathBuf,
    target_provider: String,
) -> Result<RepairResult, AppError> {
    ensure_codex_stopped(store)?;
    let configured_app = store.codex_app_setting()?;
    let expected_configured_provider = provider_sync::configured_provider(&home);
    tokio::task::spawn_blocking(move || {
        provider_sync::repair_with_guard(&home, &target_provider, || {
            if platform::codex_app_running(configured_app.as_deref()) {
                return Ok(false);
            }
            Ok(provider_sync::configured_provider(&home) == expected_configured_provider)
        })
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

pub(crate) async fn repair_home_after_activation(
    store: &Store,
    home: PathBuf,
    target_provider: String,
) -> RepairResult {
    let result = match ensure_codex_stopped(store) {
        Ok(()) => {
            let configured_app = store.codex_app_setting();
            let repair_target = target_provider.clone();
            tokio::task::spawn_blocking(move || {
                let configured_app = configured_app?;
                provider_sync::repair_after_connection_switch_with_guard(
                    &home,
                    &repair_target,
                    || Ok(!platform::codex_app_running(configured_app.as_deref())),
                )
            })
            .await
            .map_err(|error| AppError::Internal(error.to_string()))
            .and_then(|result| result)
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(result) => result,
        Err(error) => RepairResult {
            target_provider,
            warnings: vec![format!("会话归属未自动修复：{error}")],
            ..RepairResult::default()
        },
    }
}

#[tauri::command]
pub(crate) async fn sessions_scan(store: State<'_, Store>) -> Result<RepairScan, AppError> {
    scan_home(codex::home(&store.codex_home_setting()?)).await
}

#[tauri::command]
pub(crate) async fn sessions_repair(
    store: State<'_, Store>,
    index: State<'_, SessionIndex>,
    target_provider: String,
) -> Result<RepairResult, AppError> {
    let home = codex::home(&store.codex_home_setting()?);
    let index = index.inner().clone();
    let result = repair_home(&store, home, target_provider).await;
    index.invalidate();
    result
}

#[tauri::command]
pub(crate) async fn sessions_list(
    store: State<'_, Store>,
    index: State<'_, SessionIndex>,
    query: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
    refresh: Option<bool>,
) -> Result<PageResult<SessionSummary>, AppError> {
    let home = codex::home(&store.codex_home_setting()?);
    let index = index.inner().clone();
    if refresh.unwrap_or(false) {
        index.invalidate();
    }
    let query = query.unwrap_or_default();
    if query.chars().count() > MAX_SESSION_QUERY_CHARS {
        return Err(AppError::InvalidConfig(
            "会话搜索内容不能超过 256 个字符。".into(),
        ));
    }
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(25).clamp(1, 100);
    tokio::task::spawn_blocking(move || {
        session_index::session_page(&index, &home, &query, page, page_size)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn post_activation_repair_failure_is_returned_as_a_warning() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();

        let result = repair_home_after_activation(
            &store,
            temp.path().join("codex-home"),
            "unsupported".into(),
        )
        .await;

        assert_eq!(result.target_provider, "unsupported");
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("未自动修复"));
    }
}
