mod auth_center;
mod codex;
mod models;
mod network;
mod official_quota;
mod platform;
mod provider_sync;
mod proxy_import;
mod session_index;
mod storage;

use auth_center::{AuthCenter, DevicePollResult};
use codex::ConfigManager;
use models::*;
use session_index::SessionIndex;
use std::{borrow::Cow, path::PathBuf};
use storage::Store;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State, WindowEvent};

const TRAY_SHOW_ID: &str = "tray_show";
const TRAY_EXIT_ID: &str = "tray_exit";
const DEFAULT_WINDOW_WIDTH: f64 = 1180.0;
const DEFAULT_WINDOW_HEIGHT: f64 = 760.0;
const MIN_WINDOW_WIDTH: f64 = 360.0;
const MIN_WINDOW_HEIGHT: f64 = 520.0;
const CODEX_APP_URI: &str = "codex://";

#[derive(Default)]
struct ActivationLock(tokio::sync::Mutex<()>);

struct ApiClient(reqwest::Client);

impl Default for ApiClient {
    fn default() -> Self {
        Self(
            network::client_builder()
                .expect("无法读取系统代理设置")
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(30))
                .timeout(std::time::Duration::from_secs(60))
                .pool_max_idle_per_host(4)
                .pool_idle_timeout(std::time::Duration::from_secs(90))
                .tcp_keepalive(std::time::Duration::from_secs(60))
                .build()
                .expect("无法初始化 HTTP 客户端"),
        )
    }
}

#[tauri::command]
fn get_provider_overview(store: State<Store>) -> Result<ProviderOverview, AppError> {
    store.provider_overview()
}

#[tauri::command]
async fn save_provider(
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
fn delete_provider(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_provider(&id)
}

#[tauri::command]
async fn save_provider_account(
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
fn delete_provider_account(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_account(&id)
}

#[tauri::command]
async fn import_proxy_account(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    name: Option<String>,
    account_id: Option<String>,
    content: String,
) -> Result<OfficialAccountView, AppError> {
    let mut imported =
        proxy_import::parse_proxy_credential(&content).map_err(AppError::InvalidConfig)?;
    if let Some(account_id) = account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        imported.account_id = Some(account_id.to_owned());
    }
    let account = center.import_proxy_account(imported, name).await?;
    let saved = store.save_official_account(&account)?;
    store.official_account_view(&saved.id)
}

async fn refresh_official_quota(
    store: &Store,
    center: &AuthCenter,
    client: &ApiClient,
    account_id: &str,
) -> Result<ProviderAccountQuota, AppError> {
    let stored = store.official_account(account_id)?;
    let now = chrono::Utc::now().timestamp();
    let mut snapshot = stored.quota.clone();
    snapshot.last_attempt_at = Some(now);
    let account = match center.refresh_account(&stored).await {
        Ok(account) => store.save_official_account(&account)?,
        Err(error) => {
            snapshot.status = match error {
                AppError::InvalidConfig(_) => QuotaStatus::Unauthorized,
                AppError::StaleOperation | AppError::Internal(_) => QuotaStatus::Error,
            };
            snapshot.error = Some(error.to_string());
            return store.save_official_account_quota(account_id, snapshot);
        }
    };
    match official_quota::fetch_quota(&client.0, &account).await {
        Ok(data) => {
            snapshot.status = QuotaStatus::Success;
            snapshot.data = Some(data);
            snapshot.fetched_at = Some(now);
            snapshot.error = None;
        }
        Err(error) => {
            snapshot.status = error.status;
            snapshot.error = Some(error.message);
        }
    }
    store.save_official_account_quota(account_id, snapshot)
}

#[tauri::command]
async fn refresh_official_account_quota(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    client: State<'_, ApiClient>,
    account_id: String,
) -> Result<ProviderAccountQuota, AppError> {
    refresh_official_quota(&store, &center, &client, &account_id).await
}

#[tauri::command]
async fn refresh_all_official_quotas(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    client: State<'_, ApiClient>,
) -> Result<Vec<QuotaRefreshResult>, AppError> {
    refresh_all_official_quotas_in_store(&store, &center, &client).await
}

async fn refresh_all_official_quotas_in_store(
    store: &Store,
    center: &AuthCenter,
    client: &ApiClient,
) -> Result<Vec<QuotaRefreshResult>, AppError> {
    let account_ids = store.read(|state| {
        state
            .official_accounts
            .iter()
            .map(|account| account.id.clone())
            .collect::<Vec<_>>()
    })?;
    let mut results = Vec::with_capacity(account_ids.len());
    for account_id in account_ids {
        results.push(QuotaRefreshResult {
            quota: refresh_official_quota(store, center, client, &account_id).await?,
            account_id,
        });
    }
    Ok(results)
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
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
    operation_id: String,
) -> Result<OpenAiDevicePoll, AppError> {
    match center.poll_openai(&operation_id).await? {
        DevicePollResult::Pending => Ok(OpenAiDevicePoll::Pending),
        DevicePollResult::Expired => Ok(OpenAiDevicePoll::Expired),
        DevicePollResult::Complete(account) => {
            let _guard = activation.0.lock().await;
            sync_active_openai_credential(&store, &codex::home(&store.codex_home_setting()?))?;
            let saved = store.save_official_account(&account)?;
            let repair = activate_openai_record(&store, &manager, &saved).await?;
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
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
    id: String,
) -> Result<RepairResult, AppError> {
    let _guard = activation.0.lock().await;
    sync_active_openai_credential(&store, &codex::home(&store.codex_home_setting()?))?;
    let refreshed = center
        .refresh_account(&store.official_account(&id)?)
        .await?;
    let saved = store.save_official_account(&refreshed)?;
    activate_openai_record(&store, &manager, &saved).await
}

#[tauri::command]
fn delete_openai_account(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_official_account(&id)
}

#[tauri::command]
fn open_openai_device_page() -> Result<(), AppError> {
    platform::open_url("https://auth.openai.com/codex/device").map_err(|error| {
        AppError::Internal(format!(
            "无法打开 OpenAI 登录页面，请手动前往 https://auth.openai.com/codex/device：{error}"
        ))
    })
}

#[tauri::command]
async fn test_provider(
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
    let endpoint = models_endpoint(&provider.base_url);
    let mut request = client
        .0
        .get(&endpoint)
        .headers(custom_headers(&provider, &account)?);
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
    let ok = provider_test_succeeded(status_code);
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
fn preview_activation(
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
        .or(provider.active_account_id.clone())
        .ok_or_else(|| AppError::InvalidConfig("请先为这个服务添加一个 API Key。".into()))?;
    let account = store.account(&account_id)?;
    manager.preview_custom(&home, &provider, &account)
}

#[tauri::command]
async fn apply_activation(
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
    operation_id: String,
) -> Result<(), AppError> {
    let _guard = activation.0.lock().await;
    manager.apply(&operation_id)
}

async fn scan_home(home: PathBuf) -> Result<RepairScan, AppError> {
    tokio::task::spawn_blocking(move || provider_sync::scan(&home))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))
}

async fn repair_home(home: PathBuf, target_provider: String) -> Result<RepairResult, AppError> {
    tokio::task::spawn_blocking(move || provider_sync::repair(&home, &target_provider))
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
    let credential = match codex::read_official_account(home) {
        Ok(Some(credential)) => credential,
        Ok(None) | Err(AppError::InvalidConfig(_)) => return Ok(()),
        Err(error) => return Err(error),
    };
    let saved = store.official_account(&record_id)?;
    if saved.account_id == credential.tokens.account_id && saved.credential != credential {
        store.sync_official_credential(&record_id, &credential, saved.expires_at)?;
    }
    Ok(())
}

async fn sync_active_codex_configuration(
    store: &Store,
    manager: &ConfigManager,
) -> Result<(), AppError> {
    let (home_setting, active) =
        store.read(|state| (state.codex.home.clone(), state.active.clone()))?;
    let home = codex::home(&home_setting);
    match active.kind {
        ActiveKind::Official => {
            sync_active_openai_credential(store, &home)?;
            let account_id = active.account_id.as_deref().ok_or_else(|| {
                AppError::InvalidConfig("当前 OpenAI 登录信息不完整，请重新登录。".into())
            })?;
            let account = store.official_account(account_id)?;
            codex::activate_official_account(&home, &account.credential)?;
            return Ok(());
        }
        ActiveKind::None => return Ok(()),
        ActiveKind::Provider => {}
    }

    let provider_id = active.provider_id.as_deref().ok_or_else(|| {
        AppError::InvalidConfig("当前第三方 API 服务信息不完整，请重新选择。".into())
    })?;
    let account_id = active
        .account_id
        .as_deref()
        .ok_or_else(|| AppError::InvalidConfig("当前 API Key 信息不完整，请重新选择。".into()))?;
    let mut provider = store.provider(provider_id)?;
    let mut account = store.account(account_id)?;
    provider.normalize_and_validate()?;
    account.normalize_and_validate()?;
    if !provider.enabled || account.provider_id.as_deref() != Some(provider_id) {
        return Err(AppError::InvalidConfig(
            "当前第三方 API 服务或 API Key 已不可用，请重新选择。".into(),
        ));
    }

    let preview = manager.preview_custom(&home, &provider, &account)?;
    manager.apply(&preview.operation_id)
}

async fn activate_openai_record(
    store: &Store,
    manager: &ConfigManager,
    account: &StoredOfficialAccount,
) -> Result<RepairResult, AppError> {
    let home = codex::home(&store.codex_home_setting()?);
    let repair_sessions = provider_sync::configured_provider(&home) == codex::MANAGED_PROVIDER_ID;
    codex::activate_official_account(&home, &account.credential)?;
    if let Err(error) = store.activate_official_account(&account.id) {
        return Err(compensate_activation_failure(store, manager, error).await);
    }
    let repair = if repair_sessions {
        repair_home(home, "openai".into()).await?
    } else {
        RepairResult {
            target_provider: "openai".into(),
            ..RepairResult::default()
        }
    };
    Ok(repair)
}

#[tauri::command]
async fn activate_provider(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
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
    Ok(repair)
}

#[tauri::command]
async fn activate_official(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
) -> Result<RepairResult, AppError> {
    let _guard = activation.0.lock().await;
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
    let id =
        id.ok_or_else(|| AppError::InvalidConfig("请先在“账号与服务”中登录 OpenAI。".into()))?;
    let refreshed = center
        .refresh_account(&store.official_account(&id)?)
        .await?;
    let saved = store.save_official_account(&refreshed)?;
    activate_openai_record(&store, &manager, &saved).await
}

async fn compensate_activation_failure(
    store: &Store,
    manager: &ConfigManager,
    error: AppError,
) -> AppError {
    match sync_active_codex_configuration(store, manager).await {
        Ok(()) => error,
        Err(rollback) => AppError::Internal(format!(
            "{error}；原来的 Codex 连接也未能恢复，请重新选择账号或服务：{rollback}"
        )),
    }
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
    let result = repair_home(home, target_provider).await;
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
async fn get_dashboard(
    store: State<'_, Store>,
    index: State<'_, SessionIndex>,
) -> Result<Dashboard, AppError> {
    let (
        home_setting,
        provider_count,
        active_provider,
        active_kind,
        active_account_id,
        active_account,
        active_quota,
    ) = store.read(|state| {
        let active_stored_provider = state.active.provider_id.as_deref().and_then(|id| {
            state
                .providers
                .iter()
                .find(|provider| provider.profile.id == id)
        });
        let active_provider = active_stored_provider
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
                        .unwrap_or_else(|| "OpenAI 官方账号".into())
                })
            });
        let active_account = active_stored_provider.and_then(|provider| {
            state
                .active
                .account_id
                .as_deref()
                .and_then(|id| provider.accounts.iter().find(|account| account.id == id))
        });
        let active_official_account = matches!(state.active.kind, ActiveKind::Official)
            .then(|| {
                state.active.account_id.as_deref().and_then(|id| {
                    state
                        .official_accounts
                        .iter()
                        .find(|account| account.id == id)
                })
            })
            .flatten();
        (
            state.codex.home.clone(),
            state.providers.len(),
            active_provider,
            state.active.kind,
            active_account
                .map(|account| account.id.clone())
                .or_else(|| active_official_account.map(|account| account.id.clone())),
            active_account
                .map(|account| account.name.clone())
                .or_else(|| active_official_account.map(|account| account.name.clone())),
            active_official_account.map(|account| account.quota.clone()),
        )
    })?;
    let home = codex::home(&home_setting);
    let index = index.inner().clone();
    let scan_home = home.clone();
    let (session_count, database_count, database_health) = tokio::task::spawn_blocking(move || {
        let database_count = provider_sync::database_paths(&scan_home).len();
        match index.load(&scan_home) {
            Ok(sessions) => (sessions.len(), database_count, "可以读取".into()),
            Err(_) => (0, database_count, "读取失败".into()),
        }
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(Dashboard {
        provider_count,
        active_provider,
        active_kind,
        active_account_id,
        active_account,
        active_quota,
        codex_home: home.display().to_string(),
        database_count,
        session_count,
        database_health,
    })
}

fn settings_overview(store: &Store) -> Result<SettingsOverview, AppError> {
    let (home_setting, active) =
        store.read(|state| (state.codex.home.clone(), state.active.clone()))?;
    let home = codex::home(&home_setting);
    let inspection = codex::inspect(&home);
    let diagnostics = serde_json::json!({
        "dataDirectory": store.root(),
        "configFile": store.path(),
        "codex": &inspection,
        "active": active,
    });
    Ok(SettingsOverview {
        inspection,
        diagnostics,
        can_preview_custom: matches!(active.kind, ActiveKind::Provider),
    })
}

#[tauri::command]
fn get_settings_overview(store: State<Store>) -> Result<SettingsOverview, AppError> {
    settings_overview(&store)
}

#[tauri::command]
fn launch_codex() -> Result<(), AppError> {
    platform::open_url(CODEX_APP_URI).map_err(|error| {
        AppError::Internal(format!(
            "无法打开 Codex，请确认已安装 Codex 桌面应用：{error}"
        ))
    })
}

fn models_endpoint(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if base.rsplit('/').next().is_some_and(|part| {
        part.starts_with('v') && part[1..].chars().all(|value| value.is_ascii_digit())
    }) {
        format!("{base}/models")
    } else {
        format!("{base}/v1/models")
    }
}

fn provider_test_succeeded(status: reqwest::StatusCode) -> bool {
    status.is_success()
}

fn custom_headers(
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

async fn sync_configured_provider(app: tauri::AppHandle) {
    let store = app.state::<Store>();
    let manager = app.state::<ConfigManager>();
    let activation = app.state::<ActivationLock>();
    let _guard = activation.0.lock().await;
    let _ = sync_active_codex_configuration(&store, &manager).await;
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
            .inner_size(DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
            .min_inner_size(MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)
            .build();
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app)
        }))
        .manage(Store::new().expect("无法初始化应用数据"))
        .manage(AuthCenter::default())
        .manage(ConfigManager::default())
        .manage(ActivationLock::default())
        .manage(ApiClient::default())
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
            tauri::async_runtime::spawn(sync_configured_provider(handle));
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
            get_settings_overview,
            get_provider_overview,
            save_provider,
            delete_provider,
            save_provider_account,
            delete_provider_account,
            import_proxy_account,
            start_openai_device_auth,
            poll_openai_device_auth,
            activate_openai_account,
            delete_openai_account,
            open_openai_device_page,
            test_provider,
            refresh_official_account_quota,
            refresh_all_official_quotas,
            preview_activation,
            apply_activation,
            activate_provider,
            activate_official,
            scan_codex_data,
            repair_codex_data,
            list_sessions,
            launch_codex,
        ])
        .build(tauri::generate_context!())
        .expect("Tauri 运行失败");
    app.run(|app, event| match event {
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen {
            has_visible_windows: false,
            ..
        } => show_main_window(app),
        _ => {
            let _ = app;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

    #[tokio::test]
    async fn refresh_all_official_quotas_continues_after_expired_proxy_accounts() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        for account_id in ["first", "second"] {
            store
                .save_official_account(&StoredOfficialAccount {
                    id: String::new(),
                    name: account_id.into(),
                    account_id: account_id.into(),
                    email: String::new(),
                    credential: CodexAuthCredential {
                        auth_mode: "chatgpt".into(),
                        openai_api_key: None,
                        tokens: CodexAuthTokens {
                            id_token: String::new(),
                            access_token: format!("{account_id}-access"),
                            refresh_token: String::new(),
                            account_id: account_id.into(),
                        },
                        last_refresh: "2026-07-31T00:00:00Z".into(),
                    },
                    source: OfficialAccountSource::ProxyImport,
                    expires_at: Some(1),
                    quota: ProviderAccountQuota::default(),
                    created_at: 0,
                    updated_at: 0,
                })
                .unwrap();
        }

        let results = refresh_all_official_quotas_in_store(
            &store,
            &AuthCenter::default(),
            &ApiClient::default(),
        )
        .await
        .unwrap();

        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|result| result.quota.status == QuotaStatus::Unauthorized)
        );
        assert!(
            results
                .iter()
                .all(|result| result.quota.last_attempt_at.is_some())
        );
    }

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
    async fn active_custom_configuration_syncs_codex_credentials_and_provider() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        fs::create_dir_all(&home).unwrap();
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
                base_url: "http://127.0.0.1:9/v1".into(),
                headers: Default::default(),
                timeout_secs: 30,
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

        sync_active_codex_configuration(&store, &ConfigManager::default())
            .await
            .unwrap();

        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        let document = config.parse::<toml_edit::DocumentMut>().unwrap();
        let custom = document["model_providers"]["custom"].as_table().unwrap();
        assert_eq!(custom["base_url"].as_str(), Some("http://127.0.0.1:9/v1"));
        assert_eq!(custom["wire_api"].as_str(), Some("responses"));
        assert_eq!(custom["requires_openai_auth"].as_bool(), Some(true));
        assert!(custom.get("experimental_bearer_token").is_none());
        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth.as_object().unwrap().len(), 1);
        assert_eq!(auth["OPENAI_API_KEY"], "secret");
    }

    #[tokio::test]
    async fn official_sync_repairs_drifted_custom_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        fs::create_dir_all(&home).unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        store
            .update(|state| {
                state.codex.home = home.display().to_string();
                Ok(())
            })
            .unwrap();
        let credential = CodexAuthCredential {
            auth_mode: "chatgpt".into(),
            openai_api_key: None,
            tokens: CodexAuthTokens {
                id_token: "id-secret".into(),
                access_token: "access-secret".into(),
                refresh_token: "refresh-secret".into(),
                account_id: "workspace".into(),
            },
            last_refresh: "2026-07-15T00:00:00Z".into(),
        };
        let saved = store
            .save_official_account(&StoredOfficialAccount {
                id: String::new(),
                name: "OpenAI".into(),
                account_id: "workspace".into(),
                email: "person@example.test".into(),
                credential: credential.clone(),
                source: OfficialAccountSource::OpenAiOauth,
                expires_at: None,
                quota: ProviderAccountQuota::default(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store.activate_official_account(&saved.id).unwrap();
        fs::write(
            home.join("config.toml"),
            "model_provider = \"custom\"\n[model_providers.custom]\nwire_api = \"responses\"\nbase_url = \"https://wrong.example.test/v1\"\n",
        )
        .unwrap();
        fs::write(
            home.join("auth.json"),
            r#"{"OPENAI_API_KEY":"wrong-secret"}"#,
        )
        .unwrap();

        sync_active_codex_configuration(&store, &ConfigManager::default())
            .await
            .unwrap();

        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        let document = config.parse::<toml_edit::DocumentMut>().unwrap();
        assert!(document.get("model_provider").is_none());
        assert!(
            document
                .get("model_providers")
                .and_then(toml_edit::Item::as_table)
                .is_none_or(|providers| providers.get("custom").is_none())
        );
        let repaired: CodexAuthCredential =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(repaired, credential);
    }
}
