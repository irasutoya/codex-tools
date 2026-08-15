use crate::{
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
    home: PathBuf,
    target_provider: String,
) -> Result<RepairResult, AppError> {
    tokio::task::spawn_blocking(move || provider_sync::repair(&home, &target_provider))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
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
    let configured = store.codex_app_setting()?;
    if platform::codex_app_running(configured.as_deref()) {
        return Err(AppError::InvalidConfig(
            "请先退出 Codex，再修复会话归属，以免覆盖正在写入的会话内容。".into(),
        ));
    }
    let home = codex::home(&store.codex_home_setting()?);
    let index = index.inner().clone();
    let result = repair_home(home, target_provider).await;
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
