use crate::{
    model_unlock,
    models::{AppError, ModelUnlockResult, ModelUnlockStatus},
    storage::Store,
};
use tauri::State;

#[tauri::command]
pub(crate) async fn get_model_unlock_status(
    store: State<'_, Store>,
) -> Result<ModelUnlockStatus, AppError> {
    model_unlock::status(&store).await
}

#[tauri::command]
pub(crate) async fn unlock_codex_models(
    store: State<'_, Store>,
) -> Result<ModelUnlockResult, AppError> {
    model_unlock::unlock(&store).await
}

/// 退出正在运行的 Codex，以调试模式重新启动，并注入解锁脚本。
/// 会关闭当前 Codex 会话窗口，前端调用前应让用户确认。
#[tauri::command]
pub(crate) async fn launch_codex_with_debug(
    store: State<'_, Store>,
) -> Result<ModelUnlockResult, AppError> {
    model_unlock::launch_with_debug(&store).await
}
