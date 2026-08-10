use crate::{
    model_unlock,
    models::{AppError, ModelUnlockResult, ModelUnlockStatus},
    storage::Store,
};
use tauri::State;

#[tauri::command]
pub(crate) async fn settings_model_unlock_status(
    store: State<'_, Store>,
) -> Result<ModelUnlockStatus, AppError> {
    model_unlock::status(&store).await
}

#[tauri::command]
pub(crate) async fn settings_unlock_models(
    store: State<'_, Store>,
) -> Result<ModelUnlockResult, AppError> {
    model_unlock::unlock(&store).await
}

/// 以调试模式启动 Codex 并注入解锁脚本；不会退出或重启已有实例。
#[tauri::command]
pub(crate) async fn settings_launch_codex_debug(
    store: State<'_, Store>,
) -> Result<ModelUnlockResult, AppError> {
    model_unlock::launch_with_debug(&store).await
}
