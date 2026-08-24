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
    _store: &Store,
    _home: PathBuf,
    target_provider: String,
) -> RepairResult {
    // Codex 近期以 JSONL 字节偏移和 ordinal 映射关联 thread_history_1.sqlite；
    // 自动重序列化 rollout 会破坏该映射。账号/服务切换不得扫描或修复历史，
    // 显式的 sessions_repair 仍保留为用户主动维护入口。
    RepairResult {
        target_provider,
        ..RepairResult::default()
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
    use sha2::{Digest, Sha256};
    use std::fs;

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    #[tokio::test]
    async fn post_activation_switch_preserves_rollout_bytes_and_thread_history_sentinel() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let sessions = home.join("sessions/2026/08/24");
        let archived = home.join("archived_sessions/2026/08/24");
        fs::create_dir_all(&sessions).unwrap();
        fs::create_dir_all(&archived).unwrap();

        let active_rollout = sessions.join("rollout.jsonl");
        let archived_rollout = archived.join("rollout.jsonl");
        let active_bytes = br#"{ "type": "session_meta", "payload": { "id": "active", "model_provider": "openai" } }
{"type":"event_msg","payload":{"type":"user_message","message":"keep active ordinal"}}
{"type":"response_item","payload":{"type":"message","id":"msg_active","role":"assistant","content":[]}}
"#;
        let archived_bytes =
            br#"{"type":"session_meta","payload":{"id":"archived","model_provider":"openai"}}
{"type":"event_msg","payload":{"type":"user_message","message":"keep archived ordinal"}}
"#;
        let thread_history = home.join("thread_history_1.sqlite");
        let thread_history_bytes = b"thread-history-v1-byte-cursor-sentinel";
        fs::write(&active_rollout, active_bytes).unwrap();
        fs::write(&archived_rollout, archived_bytes).unwrap();
        fs::write(&thread_history, thread_history_bytes).unwrap();

        let store = Store::open(temp.path().join("data")).unwrap();
        let repair = repair_home_after_activation(&store, home.clone(), "custom".into()).await;

        assert_eq!(repair.files_scanned, 0);
        assert_eq!(repair.files_modified, 0);
        assert_eq!(fs::read(&active_rollout).unwrap(), active_bytes);
        assert_eq!(
            sha256(&fs::read(&active_rollout).unwrap()),
            sha256(active_bytes)
        );
        assert_eq!(fs::read(&archived_rollout).unwrap(), archived_bytes);
        assert_eq!(
            sha256(&fs::read(&archived_rollout).unwrap()),
            sha256(archived_bytes)
        );
        assert_eq!(fs::read(&thread_history).unwrap(), thread_history_bytes);
        assert_eq!(
            sha256(&fs::read(&thread_history).unwrap()),
            sha256(thread_history_bytes)
        );
    }

    #[tokio::test]
    async fn post_activation_repair_is_a_noop() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();

        let result = repair_home_after_activation(
            &store,
            temp.path().join("codex-home"),
            "unsupported".into(),
        )
        .await;

        assert_eq!(result.target_provider, "unsupported");
        assert_eq!(result.files_scanned, 0);
        assert!(result.warnings.is_empty());
    }
}
