use crate::{
    activation::{
        compensate_activation_failure, sync_active_codex_configuration,
        sync_active_openai_credential,
    },
    codex::{self, ConfigManager},
    commands::sessions::repair_home,
    local_usage::UsageLedger,
    models::*,
    provider_http, provider_sync,
    state::{ActivationLock, ApiClient},
    storage::Store,
};
use tauri::State;

#[tauri::command]
pub(crate) fn get_provider_overview(store: State<Store>) -> Result<ProviderOverview, AppError> {
    store.provider_overview()
}

#[tauri::command]
pub(crate) async fn save_provider(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
    provider: ProviderProfile,
) -> Result<ProviderProfile, AppError> {
    let _guard = activation.0.lock().await;
    let saved = store.save_provider(provider)?;
    if store.is_active_provider(&saved.id)? {
        sync_active_codex_configuration(&store, &manager).await?;
    }
    Ok(saved.redacted())
}

#[tauri::command]
pub(crate) fn delete_provider(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_provider(&id)
}

#[tauri::command]
pub(crate) async fn save_provider_account(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
    account: ProviderAccount,
) -> Result<ProviderAccount, AppError> {
    let _guard = activation.0.lock().await;
    let saved = store.save_account(account)?;
    if store.is_active_account(&saved.id)? {
        sync_active_codex_configuration(&store, &manager).await?;
    }
    Ok(saved.redacted())
}

#[tauri::command]
pub(crate) fn delete_provider_account(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_account(&id)
}

#[tauri::command]
pub(crate) async fn test_provider(
    store: State<'_, Store>,
    client: State<'_, ApiClient>,
    id: String,
    account_id: String,
) -> Result<ProviderTestResult, AppError> {
    let mut provider = store.provider(&id)?;
    let mut account = store.account(&account_id)?;
    provider.normalize_and_validate()?;
    account.normalize_and_validate()?;
    if account.provider_id.as_deref() != Some(id.as_str()) {
        return Err(AppError::InvalidConfig(
            "所选 API Key 不属于这个服务，请刷新页面后重试。".into(),
        ));
    }
    let endpoint = provider_http::models_endpoint(&provider.base_url);
    let mut request = client
        .current()?
        .get(&endpoint)
        .headers(provider_http::custom_headers(&provider, &account)?);
    let key = account.api_key.as_deref().unwrap_or_default();
    request = request.bearer_auth(key);
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(provider.timeout_secs),
        request.send(),
    )
    .await
    .map_err(|_| AppError::InvalidConfig("连接超时，请检查网络和 API 地址后重试。".into()))?
    .map_err(|error| {
        AppError::InvalidConfig(format!("无法连接到服务，请检查网络和 API 地址：{error}"))
    })?;
    let status_code = response.status();
    let status = status_code.as_u16();
    let ok = provider_http::provider_test_succeeded(status_code);
    Ok(ProviderTestResult {
        ok,
        status,
        endpoint,
        message: if ok {
            "模型列表接口可以访问，Codex 可直接从此服务读取模型。".into()
        } else {
            format!("连接测试未通过（HTTP {status}），请检查 API 地址、API Key 和服务状态。")
        },
        suggest_v1: status == 404 && !provider.base_url.ends_with("/v1"),
    })
}

#[tauri::command]
pub(crate) fn preview_activation(
    store: State<Store>,
    manager: State<ConfigManager>,
    provider_id: Option<String>,
) -> Result<ConfigPatchPreview, AppError> {
    let (home_setting, active_provider_id, active_account_id) = store.read(|state| {
        (
            state.codex.home.clone(),
            state.active.provider_id.clone(),
            state.active.account_id.clone(),
        )
    })?;
    let home = codex::home(&home_setting);
    let provider_id = provider_id
        .as_deref()
        .or(active_provider_id.as_deref())
        .ok_or_else(|| AppError::InvalidConfig("请先添加并启用一个第三方 API 服务。".into()))?;
    let provider = store.provider(provider_id)?;
    let account_id = active_account_id
        .filter(|_| active_provider_id.as_deref() == Some(provider_id))
        .or_else(|| provider.active_account_id.clone())
        .ok_or_else(|| AppError::InvalidConfig("请先为这个服务添加一个 API Key。".into()))?;
    let account = store.account(&account_id)?;
    manager.preview_custom(&home, &provider, &account)
}

#[tauri::command]
pub(crate) async fn apply_activation(
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
    operation_id: String,
) -> Result<(), AppError> {
    let _guard = activation.0.lock().await;
    manager.apply(&operation_id)
}

#[tauri::command]
pub(crate) async fn activate_provider(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    ledger: State<'_, UsageLedger>,
    activation: State<'_, ActivationLock>,
    id: String,
    account_id: String,
) -> Result<RepairResult, AppError> {
    let _guard = activation.0.lock().await;
    let mut provider = store.provider(&id)?;
    let mut account = store.account(&account_id)?;
    provider.normalize_and_validate()?;
    account.normalize_and_validate()?;
    if !provider.enabled || account.provider_id.as_deref() != Some(id.as_str()) {
        return Err(AppError::InvalidConfig(
            "所选第三方 API 服务或 API Key 已不可用，请检查后重试。".into(),
        ));
    }
    let home = codex::home(&store.codex_home_setting()?);
    sync_active_openai_credential(&store, &home)?;
    let repair_sessions = provider_sync::configured_provider(&home) != codex::MANAGED_PROVIDER_ID;
    let preview = manager.preview_custom(&home, &provider, &account)?;
    let pending_id = crate::begin_activation(
        &ledger,
        &crate::activation_for_provider(
            &provider,
            &account,
            chrono::Utc::now().timestamp_millis(),
        ),
    )?;
    let result = async {
        manager.apply(&preview.operation_id)?;
        if let Err(error) = store.activate(&id, &account_id) {
            return Err(compensate_activation_failure(&store, &manager, error).await);
        }
        let repair = if repair_sessions {
            repair_home(home, codex::MANAGED_PROVIDER_ID.into()).await?
        } else {
            RepairResult {
                target_provider: codex::MANAGED_PROVIDER_ID.into(),
                ..RepairResult::default()
            }
        };
        Ok::<_, AppError>(repair)
    }
    .await;
    match result {
        Ok(mut repair) => {
            crate::confirm_pending(&ledger, &pending_id, &mut repair);
            Ok(repair)
        }
        Err(error) => {
            crate::cancel_pending(&ledger, &pending_id);
            Err(error)
        }
    }
}
