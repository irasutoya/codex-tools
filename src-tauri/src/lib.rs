mod auth_center;
mod codex;
mod model_catalog;
mod model_fetch;
mod models;
mod protocol_proxy;
mod provider_sync;
mod session_index;
mod storage;

use auth_center::{AuthCenter, DevicePollResult};
use codex::ConfigManager;
use models::*;
use protocol_proxy::ProxyManager;
use session_index::SessionIndex;
use std::{borrow::Cow, path::PathBuf, process::Command};
use storage::Store;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State, WindowEvent};

const TRAY_SHOW_ID: &str = "tray_show";
const TRAY_EXIT_ID: &str = "tray_exit";

#[tauri::command]
fn list_providers(store: State<Store>) -> Result<Vec<ProviderProfile>, AppError> {
    Ok(store
        .providers()?
        .into_iter()
        .map(ProviderProfile::redacted)
        .collect())
}

#[tauri::command]
fn get_provider_overview(store: State<Store>) -> Result<ProviderOverview, AppError> {
    store.provider_overview()
}

#[tauri::command]
async fn save_provider(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    proxy: State<'_, ProxyManager>,
    provider: ProviderProfile,
) -> Result<ProviderProfile, AppError> {
    let _guard = proxy.activation_guard().await;
    let saved = store.save_provider(provider)?;
    if store.is_active_provider(&saved.id)? {
        sync_active_codex_configuration(&store, &manager, &proxy).await?;
    }
    Ok(saved.redacted())
}

#[tauri::command]
fn delete_provider(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_provider(&id)
}

#[tauri::command]
fn list_provider_accounts(
    store: State<Store>,
    provider_id: Option<String>,
) -> Result<Vec<ProviderAccount>, AppError> {
    Ok(store
        .accounts(provider_id.as_deref())?
        .into_iter()
        .map(ProviderAccount::redacted)
        .collect())
}

#[tauri::command]
async fn save_provider_account(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    proxy: State<'_, ProxyManager>,
    account: ProviderAccount,
) -> Result<ProviderAccount, AppError> {
    let _guard = proxy.activation_guard().await;
    let saved = store.save_account(account)?;
    if store.is_active_account(&saved.id)? {
        sync_active_codex_configuration(&store, &manager, &proxy).await?;
    }
    Ok(saved.redacted())
}

#[tauri::command]
fn delete_provider_account(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_account(&id)
}

#[tauri::command]
fn list_openai_accounts(store: State<Store>) -> Result<Vec<OfficialAccountView>, AppError> {
    store.official_accounts()
}

#[tauri::command]
async fn start_openai_device_auth(
    center: State<'_, AuthCenter>,
) -> Result<OpenAiDeviceAuthorization, AppError> {
    center.start_openai().await
}

#[tauri::command]
async fn poll_openai_device_auth(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    proxy: State<'_, ProxyManager>,
    operation_id: String,
) -> Result<OpenAiDevicePoll, AppError> {
    match center.poll_openai(&operation_id).await? {
        DevicePollResult::Pending => Ok(OpenAiDevicePoll::Pending),
        DevicePollResult::Expired => Ok(OpenAiDevicePoll::Expired),
        DevicePollResult::Complete(account) => {
            let _guard = proxy.activation_guard().await;
            sync_active_openai_credential(&store, &codex::home(&store.codex_home_setting()?))?;
            let saved = store.save_official_account(&account)?;
            let repair = activate_openai_record(&store, &proxy, &saved).await?;
            Ok(OpenAiDevicePoll::Complete {
                account: Box::new(store.official_account_view(&saved.id)?),
                repair,
            })
        }
    }
}

#[tauri::command]
async fn activate_openai_account(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    proxy: State<'_, ProxyManager>,
    id: String,
) -> Result<RepairResult, AppError> {
    let _guard = proxy.activation_guard().await;
    sync_active_openai_credential(&store, &codex::home(&store.codex_home_setting()?))?;
    let refreshed = center
        .refresh_account(&store.official_account(&id)?)
        .await?;
    let saved = store.save_official_account(&refreshed)?;
    activate_openai_record(&store, &proxy, &saved).await
}

#[tauri::command]
fn delete_openai_account(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_official_account(&id)
}

#[tauri::command]
fn open_openai_device_page() -> Result<(), AppError> {
    #[cfg(windows)]
    {
        Command::new("rundll32.exe")
            .args([
                "url.dll,FileProtocolHandler",
                "https://auth.openai.com/codex/device",
            ])
            .spawn()
            .map(|_| ())
            .map_err(|error| AppError::Internal(format!("无法打开 OpenAI 登录页面：{error}")))
    }
    #[cfg(not(windows))]
    {
        Err(AppError::Internal(
            "当前版本仅支持在 Windows 打开 OpenAI 登录页面".into(),
        ))
    }
}

#[tauri::command]
async fn fetch_provider_models(
    store: State<'_, Store>,
    proxy: State<'_, ProxyManager>,
    provider_id: String,
    account_id: String,
) -> Result<Vec<FetchedModel>, AppError> {
    let mut provider = store.provider(&provider_id)?;
    let mut account = store.account(&account_id)?;
    provider.normalize_and_validate()?;
    account.normalize_and_validate()?;
    if account.provider_id.as_deref() != Some(provider_id.as_str()) {
        return Err(AppError::InvalidConfig("账号不属于所选 Provider".into()));
    }
    model_fetch::fetch_models(&proxy.client(), &provider, &account).await
}

#[tauri::command]
async fn test_provider(
    store: State<'_, Store>,
    proxy: State<'_, ProxyManager>,
    id: String,
    account_id: String,
) -> Result<ProviderTestResult, AppError> {
    let mut provider = store.provider(&id)?;
    let mut account = store.account(&account_id)?;
    provider.normalize_and_validate()?;
    account.normalize_and_validate()?;
    if account.provider_id.as_deref() != Some(id.as_str()) {
        return Err(AppError::InvalidConfig("账号不属于所选 Provider".into()));
    }
    let suffix = match provider.protocol {
        ProviderProtocol::Responses => "responses",
        ProviderProtocol::ChatCompletions => "chat/completions",
        ProviderProtocol::AnthropicMessages => "messages",
    };
    let endpoint = endpoint_url(&provider.base_url, suffix);
    let model = provider
        .models
        .first()
        .cloned()
        .unwrap_or_else(|| "model".into());
    let client = proxy.client();
    let mut request = client.post(&endpoint);
    for (name, value) in provider.headers.iter().chain(account.headers.iter()) {
        request = request.header(name, value);
    }
    let key = account.api_key.as_deref().unwrap_or_default();
    let payload = match provider.protocol {
        ProviderProtocol::Responses => {
            request = request.bearer_auth(key);
            serde_json::json!({"model":model,"input":"hi","max_output_tokens":8})
        }
        ProviderProtocol::ChatCompletions => {
            request = request.bearer_auth(key);
            serde_json::json!({"model":model,"messages":[{"role":"user","content":"hi"}],"max_tokens":8})
        }
        ProviderProtocol::AnthropicMessages => {
            request = request
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01");
            serde_json::json!({"model":model,"messages":[{"role":"user","content":"hi"}],"max_tokens":8})
        }
    };
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(provider.timeout_secs),
        request.json(&payload).send(),
    )
    .await
    .map_err(|_| AppError::InvalidConfig("连接超时".into()))?
    .map_err(|error| AppError::InvalidConfig(format!("连接失败：{error}")))?;
    let status = response.status().as_u16();
    Ok(ProviderTestResult {
        ok: status < 400,
        status,
        endpoint,
        message: if status < 400 {
            "连接成功".into()
        } else {
            "上游返回错误，响应详情已隐藏".into()
        },
        suggest_v1: status == 404 && !provider.base_url.ends_with("/v1"),
    })
}

#[tauri::command]
fn preview_activation(
    store: State<Store>,
    manager: State<ConfigManager>,
    mode: String,
    provider_id: Option<String>,
    regenerate_token: Option<bool>,
) -> Result<ConfigPatchPreview, AppError> {
    if mode != "custom" {
        return Err(AppError::InvalidConfig(
            "官方模式必须通过已保存的 OpenAI 账号完整切换".into(),
        ));
    }
    let (home_setting, route, active_provider_id) = store.read(|state| {
        (
            state.codex.home.clone(),
            state.route.clone(),
            state.active.provider_id.clone(),
        )
    })?;
    let home = codex::home(&home_setting);
    let provider_id = provider_id
        .as_deref()
        .or(active_provider_id.as_deref())
        .ok_or_else(|| AppError::InvalidConfig("没有可用的 Provider".into()))?;
    let provider = store.provider(provider_id)?;
    let model = provider
        .models
        .first()
        .ok_or_else(|| AppError::InvalidConfig("Provider 没有模型".into()))?;
    manager.preview_custom(
        &home,
        store.root(),
        model,
        &route,
        regenerate_token.unwrap_or(false),
    )
}

#[tauri::command]
fn apply_activation(manager: State<ConfigManager>, operation_id: String) -> Result<(), AppError> {
    manager.apply(&operation_id)
}

async fn rebuild_model_catalog(
    provider: ProviderProfile,
    home: PathBuf,
    data_root: PathBuf,
) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || {
        codex::regenerate_model_catalog(&provider, &home, &data_root)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

async fn scan_home(home: PathBuf) -> Result<RepairScan, AppError> {
    tokio::task::spawn_blocking(move || provider_sync::scan(&home))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))
}

async fn migrate_home(home: PathBuf, target_provider: String) -> Result<RepairResult, AppError> {
    tokio::task::spawn_blocking(move || provider_sync::migrate(&home, &target_provider))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

fn sync_active_openai_credential(store: &Store, home: &std::path::Path) -> Result<(), AppError> {
    let record_id = store.read(|state| {
        matches!(state.active.kind, ActiveKind::Official)
            .then(|| state.active.account_id.clone())
            .flatten()
    })?;
    let Some(record_id) = record_id else {
        return Ok(());
    };
    let Some(credential) = codex::read_official_account(home)? else {
        return Ok(());
    };
    let saved = store.official_account(&record_id)?;
    if saved.account_id == credential.tokens.account_id {
        store.sync_official_credential(&record_id, &credential, saved.expires_at)?;
    }
    Ok(())
}

async fn sync_active_codex_configuration(
    store: &Store,
    manager: &ConfigManager,
    proxy: &ProxyManager,
) -> Result<(), AppError> {
    let (home_setting, active, route) = store.read(|state| {
        (
            state.codex.home.clone(),
            state.active.clone(),
            state.route.clone(),
        )
    })?;
    let home = codex::home(&home_setting);
    match active.kind {
        ActiveKind::Official => {
            sync_active_openai_credential(store, &home)?;
            proxy.stop().await;
            return Ok(());
        }
        ActiveKind::None => {
            proxy.stop().await;
            return Ok(());
        }
        ActiveKind::Provider => {}
    }

    let provider_id = active
        .provider_id
        .as_deref()
        .ok_or_else(|| AppError::InvalidConfig("当前 Provider 状态不完整".into()))?;
    let account_id = active
        .account_id
        .as_deref()
        .ok_or_else(|| AppError::InvalidConfig("当前 Provider 账号状态不完整".into()))?;
    let mut provider = store.provider(provider_id)?;
    let mut account = store.account(account_id)?;
    provider.normalize_and_validate()?;
    account.normalize_and_validate()?;
    if !provider.enabled || account.provider_id.as_deref() != Some(provider_id) {
        return Err(AppError::InvalidConfig("当前 Provider 或账号不可用".into()));
    }

    if route.enabled {
        proxy.prepare(&provider, &account, &route).await?;
    }
    if let Err(error) =
        rebuild_model_catalog(provider.clone(), home.clone(), store.root().to_path_buf()).await
    {
        if route.enabled {
            proxy.abort_pending().await;
        }
        return Err(error);
    }
    let prepared = (|| {
        let model = provider
            .models
            .first()
            .ok_or_else(|| AppError::InvalidConfig("Provider 没有模型".into()))?;
        let preview = manager.preview_custom(&home, store.root(), model, &route, false)?;
        manager.apply(&preview.operation_id)
    })();
    if let Err(error) = prepared {
        if route.enabled {
            proxy.abort_pending().await;
        }
        return Err(error);
    }

    if route.enabled {
        proxy.commit().await
    } else {
        proxy.stop().await;
        Ok(())
    }
}

async fn activate_openai_record(
    store: &Store,
    proxy: &ProxyManager,
    account: &StoredOfficialAccount,
) -> Result<RepairResult, AppError> {
    let home = codex::home(&store.codex_home_setting()?);
    let migrate_sessions =
        scan_home(home.clone()).await?.current_provider == codex::MANAGED_PROVIDER_ID;
    codex::activate_official_account(&home, &account.credential)?;
    store.activate_official_account(&account.id)?;
    store.clear_managed_codex_fields()?;
    let repair = if migrate_sessions {
        migrate_home(home, "openai".into()).await?
    } else {
        RepairResult {
            target_provider: "openai".into(),
            ..RepairResult::default()
        }
    };
    proxy.stop().await;
    Ok(repair)
}

#[tauri::command]
async fn activate_provider(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    proxy: State<'_, ProxyManager>,
    id: String,
    account_id: String,
) -> Result<RepairResult, AppError> {
    let _guard = proxy.activation_guard().await;
    let mut provider = store.provider(&id)?;
    let mut account = store.account(&account_id)?;
    provider.normalize_and_validate()?;
    account.normalize_and_validate()?;
    if !provider.enabled || account.provider_id.as_deref() != Some(id.as_str()) {
        return Err(AppError::InvalidConfig("Provider 或账号不可用".into()));
    }
    let settings = store.route_settings()?;
    if !settings.enabled {
        return Err(AppError::InvalidConfig("请先启用本地代理".into()));
    }
    let home = codex::home(&store.codex_home_setting()?);
    sync_active_openai_credential(&store, &home)?;
    proxy.prepare(&provider, &account, &settings).await?;
    let (scan, _) = match tokio::try_join!(
        scan_home(home.clone()),
        rebuild_model_catalog(provider.clone(), home.clone(), store.root().to_path_buf())
    ) {
        Ok(result) => result,
        Err(error) => {
            proxy.abort_pending().await;
            return Err(error);
        }
    };
    let migrate_sessions = scan.current_provider != codex::MANAGED_PROVIDER_ID;
    let prepared = (|| {
        let model = provider
            .models
            .first()
            .ok_or_else(|| AppError::InvalidConfig("Provider 没有模型".into()))?;
        let preview = manager.preview_custom(&home, store.root(), model, &settings, false)?;
        manager.apply(&preview.operation_id)?;
        store.activate(&id, &account_id)
    })();
    if let Err(error) = prepared {
        proxy.abort_pending().await;
        return Err(error);
    }
    let migration = if migrate_sessions {
        match migrate_home(home, codex::MANAGED_PROVIDER_ID.into()).await {
            Ok(migration) => migration,
            Err(error) => {
                proxy.abort_pending().await;
                return Err(error);
            }
        }
    } else {
        RepairResult {
            target_provider: codex::MANAGED_PROVIDER_ID.into(),
            ..RepairResult::default()
        }
    };
    proxy.commit().await?;
    Ok(migration)
}

#[tauri::command]
async fn activate_upstream(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    proxy: State<'_, ProxyManager>,
    id: String,
    account_id: String,
) -> Result<(), AppError> {
    let _guard = proxy.activation_guard().await;
    let mut provider = store.provider(&id)?;
    let mut account = store.account(&account_id)?;
    provider.normalize_and_validate()?;
    account.normalize_and_validate()?;
    if !provider.enabled || account.provider_id.as_deref() != Some(id.as_str()) {
        return Err(AppError::InvalidConfig(
            "Provider 或账号不可用，或账号归属不匹配".into(),
        ));
    }
    let settings = store.route_settings()?;
    if !settings.enabled {
        return Err(AppError::InvalidConfig("请先启用本地代理".into()));
    }
    let home = codex::home(&store.codex_home_setting()?);
    sync_active_openai_credential(&store, &home)?;
    proxy.prepare(&provider, &account, &settings).await?;
    if let Err(error) =
        rebuild_model_catalog(provider.clone(), home.clone(), store.root().to_path_buf()).await
    {
        proxy.abort_pending().await;
        return Err(error);
    }
    let prepared = (|| {
        let model = provider
            .models
            .first()
            .ok_or_else(|| AppError::InvalidConfig("Provider 没有模型".into()))?;
        let preview = manager.preview_custom(&home, store.root(), model, &settings, false)?;
        manager.apply(&preview.operation_id)?;
        store.activate(&id, &account_id)
    })();
    if let Err(error) = prepared {
        proxy.abort_pending().await;
        return Err(error);
    }
    proxy.commit().await
}

#[tauri::command]
async fn activate_official(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    proxy: State<'_, ProxyManager>,
) -> Result<RepairResult, AppError> {
    let _guard = proxy.activation_guard().await;
    let (home_setting, id) = store.read(|state| {
        let id = state
            .active
            .account_id
            .clone()
            .filter(|_| matches!(state.active.kind, ActiveKind::Official))
            .or_else(|| {
                state
                    .official_accounts
                    .iter()
                    .max_by_key(|account| account.updated_at)
                    .map(|account| account.id.clone())
            });
        (state.codex.home.clone(), id)
    })?;
    sync_active_openai_credential(&store, &codex::home(&home_setting))?;
    let id = id.ok_or_else(|| AppError::InvalidConfig("请先登录 OpenAI Account".into()))?;
    let refreshed = center
        .refresh_account(&store.official_account(&id)?)
        .await?;
    let saved = store.save_official_account(&refreshed)?;
    activate_openai_record(&store, &proxy, &saved).await
}

#[tauri::command]
fn regenerate_compatibility_token(
    store: State<Store>,
    manager: State<ConfigManager>,
) -> Result<ConfigPatchPreview, AppError> {
    let (home_setting, route, provider_id) = store.read(|state| {
        (
            state.codex.home.clone(),
            state.route.clone(),
            state.active.provider_id.clone(),
        )
    })?;
    let provider_id =
        provider_id.ok_or_else(|| AppError::InvalidConfig("当前未启用第三方 Provider".into()))?;
    let provider = store.provider(&provider_id)?;
    let model = provider
        .models
        .first()
        .ok_or_else(|| AppError::InvalidConfig("Provider 没有模型".into()))?;
    manager.preview_custom(
        &codex::home(&home_setting),
        store.root(),
        model,
        &route,
        true,
    )
}

#[tauri::command]
async fn regenerate_model_catalog(
    store: State<'_, Store>,
    provider_id: String,
) -> Result<String, AppError> {
    rebuild_model_catalog(
        store.provider(&provider_id)?,
        codex::home(&store.codex_home_setting()?),
        store.root().to_path_buf(),
    )
    .await
}

#[tauri::command]
fn inspect_codex_config(store: State<Store>) -> Result<ConfigInspection, AppError> {
    Ok(codex::inspect(
        &codex::home(&store.codex_home_setting()?),
        store.root(),
    ))
}

#[tauri::command]
async fn scan_codex_data(store: State<'_, Store>) -> Result<RepairScan, AppError> {
    scan_home(codex::home(&store.codex_home_setting()?)).await
}

#[tauri::command]
async fn repair_codex_data(
    store: State<'_, Store>,
    index: State<'_, SessionIndex>,
    target_provider: String,
) -> Result<RepairResult, AppError> {
    let home = codex::home(&store.codex_home_setting()?);
    let index = index.inner().clone();
    let result = migrate_home(home, target_provider).await;
    index.invalidate();
    result
}

#[tauri::command]
async fn list_sessions(
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
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(25).clamp(1, 100);
    tokio::task::spawn_blocking(move || session_page(&index, &home, &query, page, page_size))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?
}

fn session_page(
    index: &SessionIndex,
    home: &std::path::Path,
    query: &str,
    page: usize,
    page_size: usize,
) -> Result<PageResult<SessionSummary>, AppError> {
    let sessions = index.load_recent(home)?;
    let query = query.trim();
    let normalized_query = if query.is_ascii() {
        Cow::Borrowed(query)
    } else {
        Cow::Owned(query.to_lowercase())
    };
    let query = normalized_query.as_ref();
    let query_is_ascii = query.is_ascii();
    let start = (page - 1).saturating_mul(page_size);
    let mut total = 0;
    let mut items = Vec::with_capacity(page_size);
    for session in sessions.iter() {
        let matches = query.is_empty()
            || [
                session.id.as_str(),
                session.title.as_str(),
                session.provider.as_str(),
                session.cwd.as_str(),
            ]
            .into_iter()
            .any(|value| {
                if query_is_ascii {
                    value
                        .as_bytes()
                        .windows(query.len())
                        .any(|window| window.eq_ignore_ascii_case(query.as_bytes()))
                } else {
                    value.to_lowercase().contains(query)
                }
            });
        if !matches {
            continue;
        }
        if total >= start && items.len() < page_size {
            items.push(session.clone());
        }
        total += 1;
    }
    Ok(PageResult {
        items,
        total,
        page,
        page_size,
    })
}

#[tauri::command]
async fn get_route_console(
    store: State<'_, Store>,
    proxy: State<'_, ProxyManager>,
    page: Option<usize>,
    page_size: Option<usize>,
) -> Result<RouteConsoleSnapshot, AppError> {
    Ok(proxy
        .console(
            store.route_settings()?,
            page.unwrap_or(1),
            page_size.unwrap_or(25),
        )
        .await)
}

#[tauri::command]
async fn save_route_settings(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    proxy: State<'_, ProxyManager>,
    mut settings: RouteSettings,
) -> Result<(), AppError> {
    let _guard = proxy.activation_guard().await;
    settings.normalize();
    store.save_route_settings(settings)?;
    sync_active_codex_configuration(&store, &manager, &proxy).await
}

#[tauri::command]
async fn clear_route_logs(proxy: State<'_, ProxyManager>) -> Result<(), AppError> {
    proxy.clear_logs().await;
    Ok(())
}

#[tauri::command]
async fn get_dashboard(
    store: State<'_, Store>,
    index: State<'_, SessionIndex>,
) -> Result<Dashboard, AppError> {
    let (home_setting, provider_count, active_provider) = store.read(|state| {
        let active_provider = state
            .active
            .provider_id
            .as_deref()
            .and_then(|id| {
                state
                    .providers
                    .iter()
                    .find(|provider| provider.profile.id == id)
            })
            .map(|provider| provider.profile.name.clone())
            .or_else(|| {
                matches!(state.active.kind, ActiveKind::Official).then(|| {
                    state
                        .active
                        .account_id
                        .as_deref()
                        .and_then(|id| {
                            state
                                .official_accounts
                                .iter()
                                .find(|account| account.id == id)
                        })
                        .map(|account| format!("OpenAI · {}", account.name))
                        .unwrap_or_else(|| "OpenAI 官方模式".into())
                })
            });
        (
            state.codex.home.clone(),
            state.providers.len(),
            active_provider,
        )
    })?;
    let home = codex::home(&home_setting);
    let index = index.inner().clone();
    let scan_home = home.clone();
    let (session_count, database_count) = tokio::task::spawn_blocking(move || {
        let session_count = index.load(&scan_home).map_or(0, |sessions| sessions.len());
        let database_count = provider_sync::database_paths(&scan_home).len();
        (session_count, database_count)
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(Dashboard {
        provider_count,
        active_provider,
        codex_home: home.display().to_string(),
        database_count,
        session_count,
        database_health: "正常".into(),
    })
}

fn settings_overview(store: &Store) -> Result<SettingsOverview, AppError> {
    let (home_setting, active, route) = store.read(|state| {
        (
            state.codex.home.clone(),
            state.active.clone(),
            state.route.clone(),
        )
    })?;
    let home = codex::home(&home_setting);
    let inspection = codex::inspect(&home, store.root());
    let diagnostics = serde_json::json!({
        "dataDirectory": store.root(),
        "configFile": store.path(),
        "codex": &inspection,
        "active": active,
        "route": route,
    });
    Ok(SettingsOverview {
        inspection,
        diagnostics,
    })
}

#[tauri::command]
fn get_settings_overview(store: State<Store>) -> Result<SettingsOverview, AppError> {
    settings_overview(&store)
}

#[tauri::command]
fn get_diagnostics(store: State<Store>) -> Result<serde_json::Value, AppError> {
    Ok(settings_overview(&store)?.diagnostics)
}

#[tauri::command]
fn launch_codex() -> Result<(), AppError> {
    Command::new("codex")
        .spawn()
        .map(|_| ())
        .map_err(|error| AppError::Internal(format!("无法启动 Codex：{error}")))
}

fn endpoint_url(base_url: &str, suffix: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.rsplit('/').next().is_some_and(|part| {
        part.starts_with('v') && part[1..].chars().all(|value| value.is_ascii_digit())
    }) {
        format!("{base}/{suffix}")
    } else {
        format!("{base}/v1/{suffix}")
    }
}

async fn start_configured_route(app: tauri::AppHandle) {
    let store = app.state::<Store>();
    let manager = app.state::<ConfigManager>();
    let proxy = app.state::<ProxyManager>();
    let _guard = proxy.activation_guard().await;
    let _ = sync_active_codex_configuration(&store, &manager, &proxy).await;
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        return;
    }
    let _ =
        tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
            .title("Codex Tools")
            .inner_size(1180.0, 760.0)
            .min_inner_size(900.0, 600.0)
            .build();
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app)
        }))
        .manage(Store::new().expect("无法初始化 data/app.yaml"))
        .manage(AuthCenter::default())
        .manage(ConfigManager::default())
        .manage(ProxyManager::default())
        .manage(SessionIndex::default())
        .setup(|app| {
            let show =
                MenuItem::with_id(app, TRAY_SHOW_ID, "显示 Codex Tools", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, TRAY_EXIT_ID, "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &exit])?;
            let app_icon = app
                .default_window_icon()
                .cloned()
                .expect("application icon must be configured");
            TrayIconBuilder::new()
                .icon(app_icon)
                .menu(&menu)
                .tooltip("Codex Tools")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    TRAY_SHOW_ID => show_main_window(app),
                    TRAY_EXIT_ID => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if matches!(
                        event,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        }
                    ) {
                        show_main_window(tray.app_handle());
                    }
                })
                .build(app)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(start_configured_route(handle));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_dashboard,
            get_diagnostics,
            get_settings_overview,
            get_provider_overview,
            list_providers,
            save_provider,
            delete_provider,
            list_provider_accounts,
            save_provider_account,
            delete_provider_account,
            list_openai_accounts,
            start_openai_device_auth,
            poll_openai_device_auth,
            activate_openai_account,
            delete_openai_account,
            open_openai_device_page,
            fetch_provider_models,
            test_provider,
            preview_activation,
            apply_activation,
            activate_provider,
            activate_upstream,
            activate_official,
            regenerate_compatibility_token,
            regenerate_model_catalog,
            inspect_codex_config,
            scan_codex_data,
            repair_codex_data,
            list_sessions,
            get_route_console,
            save_route_settings,
            clear_route_logs,
            launch_codex,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 运行失败");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn session_pages_filter_without_changing_totals_or_page_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        for (id, provider, cwd, title) in [
            ("one", "custom", "C:/alpha", "Alpha task"),
            ("two", "openai", "C:/beta", "中文任务"),
            ("three", "custom", "C:/gamma", "Gamma task"),
        ] {
            let contents = format!(
                "{}\n{}\n",
                serde_json::json!({
                    "type": "session_meta",
                    "payload": {"id": id, "model_provider": provider, "cwd": cwd}
                }),
                serde_json::json!({
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": title}
                })
            );
            fs::write(sessions.join(format!("{id}.jsonl")), contents).unwrap();
        }
        let index = SessionIndex::default();

        let first = session_page(&index, temp.path(), "", 1, 2).unwrap();
        let second = session_page(&index, temp.path(), "", 2, 2).unwrap();
        assert_eq!(first.total, 3);
        assert_eq!(first.items.len(), 2);
        assert_eq!(second.total, 3);
        assert_eq!(second.items.len(), 1);
        let mut ids = first
            .items
            .into_iter()
            .chain(second.items)
            .map(|session| session.id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, ["one", "three", "two"]);

        let ascii = session_page(&index, temp.path(), "ALPHA", 1, 10).unwrap();
        assert_eq!(ascii.total, 1);
        assert_eq!(ascii.items[0].id, "one");
        let unicode = session_page(&index, temp.path(), "中文", 1, 10).unwrap();
        assert_eq!(unicode.total, 1);
        assert_eq!(unicode.items[0].id, "two");
    }

    #[tokio::test]
    async fn active_custom_configuration_syncs_all_codex_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("models_cache.json"),
            r#"{"models":[{"slug":"template","display_name":"Template","base_instructions":"test","model_messages":{},"apply_patch_tool_type":"custom","shell_type":"shell","context_window":272000}]}"#,
        )
        .unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        store
            .update(|state| {
                state.codex.home = home.display().to_string();
                Ok(())
            })
            .unwrap();
        let provider = store
            .save_provider(ProviderProfile {
                id: "provider".into(),
                name: "Provider".into(),
                protocol: ProviderProtocol::Responses,
                base_url: "http://127.0.0.1:9/v1".into(),
                models: vec!["model-v2".into()],
                model_metadata: vec![],
                model_aliases: Default::default(),
                codex_chat_reasoning: None,
                headers: Default::default(),
                timeout_secs: 30,
                context_window: None,
                auto_compact_threshold: None,
                enabled: true,
                active: false,
                active_account_id: None,
                account_count: 0,
            })
            .unwrap();
        let account = store
            .save_account(ProviderAccount {
                id: "account".into(),
                provider_id: Some(provider.id.clone()),
                name: "Account".into(),
                auth_kind: AccountAuthKind::ApiKey,
                api_key: Some("secret".into()),
                headers: Default::default(),
                active: false,
                email: None,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store.activate(&provider.id, &account.id).unwrap();
        store
            .save_route_settings(RouteSettings {
                enabled: false,
                request_max_retries: 9,
                stream_max_retries: 8,
                ..RouteSettings::default()
            })
            .unwrap();

        sync_active_codex_configuration(
            &store,
            &ConfigManager::default(),
            &ProxyManager::default(),
        )
        .await
        .unwrap();

        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        let document = config.parse::<toml_edit::DocumentMut>().unwrap();
        assert_eq!(document["model"].as_str(), Some("model-v2"));
        let custom = document["model_providers"]["custom"].as_table().unwrap();
        assert_eq!(custom["request_max_retries"].as_integer(), Some(9));
        assert_eq!(custom["stream_max_retries"].as_integer(), Some(8));
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), b"{}\n");
        let catalog = fs::read_to_string(temp.path().join("data/model_catalog.json")).unwrap();
        assert!(catalog.contains("model-v2"));
    }
}
