mod auth_center;
mod codex;
mod model_fetch;
mod models;
mod protocol_proxy;
mod provider_sync;
mod session_index;
mod storage;

use auth_center::AuthCenter;
use models::*;
use protocol_proxy::ProxyManager;
use storage::Store;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, State, WindowEvent};

const TRAY_SHOW_ID: &str = "tray_show";
const TRAY_EXIT_ID: &str = "tray_exit";

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

async fn start_configured_route(app: tauri::AppHandle) {
    let store = app.state::<Store>();
    let proxy = app.state::<ProxyManager>();
    let Ok(settings) = store.route_settings() else {
        return;
    };
    if !settings.enabled {
        return;
    }
    let Ok(providers) = store.providers() else {
        return;
    };
    let Some(provider) = providers.into_iter().find(|provider| {
        provider.active
            && provider.enabled
            && provider.protocol == ProviderProtocol::ChatCompletions
    }) else {
        return;
    };
    let Ok(accounts) = store.accounts(Some(&provider.id)) else {
        return;
    };
    let Some(account) = accounts.into_iter().find(|account| account.active) else {
        return;
    };
    match proxy.prepare(&provider, &account, &settings).await {
        Ok(_) => {}
        Err(_) => return,
    };
    if proxy.commit().await.is_err() {
        proxy.abort().await;
    }
}

#[tauri::command]
fn list_providers(store: State<Store>) -> Result<Vec<ProviderProfile>, AppError> {
    store.providers().map_err(Into::into)
}

#[tauri::command]
fn save_provider(
    store: State<Store>,
    mut provider: ProviderProfile,
) -> Result<ProviderProfile, AppError> {
    if provider.id.is_empty() {
        provider.id = uuid::Uuid::new_v4().to_string();
    }
    if provider.name.trim().is_empty() || provider.base_url.trim().is_empty() {
        return Err(AppError::InvalidConfig("名称和 Base URL 不能为空".into()));
    }
    provider.model_metadata.retain(|metadata| {
        provider
            .models
            .iter()
            .any(|model| model.trim() == metadata.id.trim())
    });
    store.save_provider(&provider)?;
    Ok(provider)
}

#[tauri::command]
fn delete_provider(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_provider(&id)?;
    Ok(())
}

#[tauri::command]
async fn fetch_provider_models(
    store: State<'_, Store>,
    provider: ProviderProfile,
) -> Result<Vec<FetchedModel>, AppError> {
    let accounts = store.accounts(Some(&provider.id))?;
    let account = accounts
        .iter()
        .find(|account| account.active)
        .or_else(|| accounts.first())
        .ok_or_else(|| AppError::InvalidConfig("请先为 Provider 添加 API 账号".into()))?;
    model_fetch::fetch_models(&provider, account).await
}

#[tauri::command]
fn list_provider_accounts(
    store: State<Store>,
    provider_id: Option<String>,
) -> Result<Vec<ProviderAccount>, AppError> {
    store.accounts(provider_id.as_deref()).map_err(Into::into)
}

#[tauri::command]
fn save_provider_account(
    store: State<Store>,
    mut account: ProviderAccount,
) -> Result<ProviderAccount, AppError> {
    if account.id.is_empty() {
        account.id = uuid::Uuid::new_v4().to_string();
    }
    if account.name.trim().is_empty() {
        return Err(AppError::InvalidConfig("账号名称不能为空".into()));
    }
    if account.auth_kind == AccountAuthKind::ApiKey
        && account
            .api_key
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        return Err(AppError::InvalidConfig("API Key 不能为空".into()));
    }
    store.save_account(&account)?;
    Ok(account)
}

#[tauri::command]
fn delete_provider_account(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_account(&id)?;
    Ok(())
}

#[tauri::command]
async fn test_provider(
    store: State<'_, Store>,
    id: String,
    account_id: String,
) -> Result<ProviderTestResult, AppError> {
    let provider = store.provider(&id)?;
    let account = store.account(&account_id)?;
    if account.provider_id.as_deref() != Some(&id) {
        return Err(AppError::InvalidConfig("账号不属于所选 Provider".into()));
    }
    let base = provider.base_url.trim_end_matches('/');
    let suffix = if provider.protocol == ProviderProtocol::Responses {
        "responses"
    } else {
        "chat/completions"
    };
    let endpoint = format!("{base}/{suffix}");
    let model = provider
        .models
        .first()
        .ok_or_else(|| AppError::InvalidConfig("请先获取或填写模型列表，再执行连接测试".into()))?;
    let payload = if provider.protocol == ProviderProtocol::Responses {
        serde_json::json!({"model":model,"input":"hi","max_output_tokens":8})
    } else {
        serde_json::json!({"model":model,"messages":[{"role":"user","content":"hi"}],"max_tokens":8})
    };
    let mut request = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(provider.timeout_secs.max(1)))
        .build()
        .map_err(|error| AppError::Internal(error.to_string()))?
        .post(&endpoint)
        .bearer_auth(account.api_key.as_deref().unwrap_or_default())
        .json(&payload);
    for headers in [&provider.headers, &account.headers] {
        if let Some(headers) = headers.as_object() {
            for (name, value) in headers {
                if let Some(value) = value.as_str() {
                    request = request.header(name, value);
                }
            }
        }
    }
    let response = request
        .send()
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let status = response.status().as_u16();
    // Consume the response body without retaining or exposing potentially sensitive data.
    let _ = response.text().await;
    let message = if status < 400 {
        "连接成功".into()
    } else {
        "上游返回错误，响应详情已隐藏".into()
    };
    Ok(ProviderTestResult {
        ok: status < 400,
        status,
        endpoint,
        message,
        suggest_v1: status == 404 && !base.ends_with("/v1"),
    })
}

#[tauri::command]
async fn activate_provider(
    store: State<'_, Store>,
    proxy: State<'_, ProxyManager>,
    id: String,
    account_id: String,
    force: bool,
) -> Result<RepairResult, AppError> {
    let _guard = proxy.activation_guard().await;
    let provider = store.provider(&id)?;
    let account = store.account(&account_id)?;
    if !provider.enabled {
        return Err(AppError::InvalidConfig("Provider 已禁用".into()));
    }
    if account.provider_id.as_deref() != Some(&id) {
        return Err(AppError::InvalidConfig("账号不属于所选 Provider".into()));
    }
    let _ = force;
    let previous = store.active_state()?;
    let endpoint = if provider.protocol == ProviderProtocol::ChatCompletions {
        let settings = store.route_settings()?;
        if !settings.enabled {
            return Err(AppError::InvalidConfig(
                "Chat Completions 需要先启用本地路由".into(),
            ));
        }
        Some(proxy.prepare(&provider, &account, &settings).await?)
    } else {
        None
    };
    let backup = match codex::apply_provider_with_proxy(&provider, &account, endpoint.as_ref()) {
        Ok(path) => path,
        Err(error) => {
            proxy.abort().await;
            return Err(error);
        }
    };
    if let Err(error) = store.activate(&id, &account_id) {
        let restored = codex::restore_provider_backup(&backup);
        proxy.abort().await;
        return match restored {
            Ok(()) => Err(error.into()),
            Err(rollback) => Err(AppError::Backup(format!(
                "切换状态失败：{error}；回滚失败：{rollback}"
            ))),
        };
    }
    let result = match codex::repair(codex::MANAGED_PROVIDER_ID) {
        Ok(result) => result,
        Err(error) => {
            let config = codex::restore_provider_backup(&backup);
            let active = store.restore_active((
                previous.0.as_deref(),
                previous.1.as_deref(),
                previous.2.as_deref(),
            ));
            proxy.abort().await;
            return match (config, active) {
                (Ok(()), Ok(())) => Err(error),
                (a, b) => Err(AppError::Backup(format!(
                    "切换失败：{error}；配置回滚：{a:?}；状态回滚：{b:?}"
                ))),
            };
        }
    };
    if endpoint.is_some() {
        proxy.commit().await?;
    }
    codex::discard_provider_backup(&backup);
    Ok(result)
}

#[tauri::command]
async fn get_proxy_status(proxy: State<'_, ProxyManager>) -> Result<ProxyStatus, AppError> {
    let endpoint = proxy.endpoint().await;
    Ok(ProxyStatus {
        running: endpoint.is_some(),
        base_url: endpoint.map(|value| value.base_url),
    })
}

#[tauri::command]
fn scan_codex_data() -> RepairScan {
    codex::scan()
}

#[tauri::command]
fn repair_codex_data(operation_id: String) -> Result<RepairResult, AppError> {
    if operation_id.is_empty() {
        return Err(AppError::StaleOperation);
    }
    codex::repair(codex::MANAGED_PROVIDER_ID)
}

#[tauri::command]
fn list_sessions(
    query: Option<String>,
    page: Option<usize>,
    page_size: Option<usize>,
) -> Result<PageResult<SessionSummary>, AppError> {
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size.unwrap_or(25).clamp(1, 100);
    let query = query.unwrap_or_default().to_lowercase();
    let sessions: Vec<_> = session_index::rebuild()?
        .into_iter()
        .filter(|session| {
            query.is_empty()
                || format!(
                    "{} {} {} {}",
                    session.id, session.title, session.provider, session.cwd
                )
                .to_lowercase()
                .contains(&query)
        })
        .collect();
    let total = sessions.len();
    let start = (page - 1).saturating_mul(page_size).min(total);
    let items = sessions.into_iter().skip(start).take(page_size).collect();
    Ok(PageResult {
        items,
        total,
        page,
        page_size,
    })
}

#[tauri::command]
fn list_auth_accounts(store: State<Store>) -> Result<Vec<AuthAccount>, AppError> {
    store.auth_accounts().map_err(Into::into)
}

#[tauri::command]
async fn activate_openai_account(
    store: State<'_, Store>,
    proxy: State<'_, ProxyManager>,
    center: State<'_, AuthCenter>,
    id: String,
) -> Result<RepairResult, AppError> {
    let _guard = proxy.activation_guard().await;
    let account = center.refresh_account(&store.auth_account(&id)?).await?;
    store.save_auth_account(&account)?;
    if account.service != AuthService::OpenAi {
        return Err(AppError::InvalidConfig("所选账号不是 OpenAI 账号".into()));
    }
    let auth = account
        .credential
        .as_ref()
        .ok_or(AppError::OfficialAuthMissing)?;
    let backup = codex::restore_official_snapshot(auth, account.config_snapshot.as_deref())?;
    let mut result = match codex::repair("openai") {
        Ok(result) => result,
        Err(error) => {
            let rollback = codex::restore_provider_backup(&backup);
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback) => Err(AppError::Backup(format!(
                    "恢复官方历史失败：{error}；配置回滚失败：{rollback}"
                ))),
            };
        }
    };
    if let Err(error) = store.activate_auth_account(&id) {
        let rollback = codex::restore_provider_backup(&backup);
        return match rollback {
            Ok(()) => Err(error.into()),
            Err(rollback) => Err(AppError::Backup(format!(
                "官方账号状态更新失败：{error}；配置回滚失败：{rollback}"
            ))),
        };
    }
    result
        .warnings
        .push("已恢复官方认证，并将全部会话历史统一修复为 OpenAI Provider。".into());
    codex::discard_provider_backup(&backup);
    Ok(result)
}

#[tauri::command]
fn delete_auth_account(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_auth_account(&id)?;
    Ok(())
}

#[tauri::command]
async fn start_openai_device_auth(
    center: State<'_, AuthCenter>,
) -> Result<OpenAiDeviceAuthorization, AppError> {
    center.start_openai().await
}

#[tauri::command]
async fn poll_openai_device_auth(
    center: State<'_, AuthCenter>,
    store: State<'_, Store>,
    operation_id: String,
) -> Result<OpenAiDevicePoll, AppError> {
    let result = center.poll_openai(&operation_id).await?;
    if let OpenAiDevicePoll::Complete { account } = &result {
        store.save_auth_account(account)?;
    }
    Ok(result)
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
    app: tauri::AppHandle,
    store: State<'_, Store>,
    proxy: State<'_, ProxyManager>,
    settings: RouteSettings,
) -> Result<RouteConsoleSnapshot, AppError> {
    let address = settings
        .listen_address
        .trim()
        .parse::<std::net::IpAddr>()
        .map_err(|_| AppError::InvalidConfig("监听地址必须是有效的 IP 地址".into()))?;
    if !address.is_loopback() {
        return Err(AppError::InvalidConfig(
            "为避免明文密钥暴露，本地路由仅允许监听回环地址".into(),
        ));
    }
    let settings = RouteSettings {
        listen_address: address.to_string(),
        ..settings
    };
    store.save_route_settings(&settings)?;
    proxy.stop().await;
    if settings.enabled {
        start_configured_route(app.clone()).await;
    }
    let store = app.state::<Store>();
    let proxy = app.state::<ProxyManager>();
    Ok(proxy.console(store.route_settings()?, 1, 25).await)
}

#[tauri::command]
async fn clear_route_logs(proxy: State<'_, ProxyManager>) -> Result<(), AppError> {
    proxy.clear_logs().await;
    Ok(())
}
#[tauri::command]
fn delete_sessions_permanently(ids: Vec<String>) -> Result<usize, AppError> {
    codex::delete_sessions(&ids).map_err(Into::into)
}
#[tauri::command]
fn export_sessions(ids: Vec<String>, target: String) -> Result<String, AppError> {
    codex::export_sessions(&ids, std::path::Path::new(&target)).map_err(Into::into)
}

#[tauri::command]
fn get_dashboard(store: State<Store>) -> Result<Dashboard, AppError> {
    let providers = store.providers()?;
    let scan = codex::scan();
    let session_count = scan
        .databases
        .iter()
        .map(|database| database.thread_count as usize)
        .sum();
    Ok(Dashboard {
        provider_count: providers.len() as u64,
        active_provider: providers.into_iter().find(|p| p.active).map(|p| p.name),
        codex_home: codex::home().display().to_string(),
        database_count: scan.databases.len(),
        session_count,
        database_health: if scan.databases.iter().all(|db| db.health == "ok") {
            "健康".into()
        } else {
            "需要检查".into()
        },
    })
}

#[tauri::command]
fn get_diagnostics(store: State<Store>) -> DiagnosticReport {
    DiagnosticReport {
        app_db: store.path().display().to_string(),
        codex_home: codex::home().display().to_string(),
        config_exists: codex::home().join("config.toml").exists(),
        auth_exists: codex::home().join("auth.json").exists(),
        databases: codex::databases()
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        warnings: codex::scan().warnings,
    }
}

pub fn run() {
    let store = Store::open().expect("无法初始化应用数据库");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(store)
        .manage(ProxyManager::default())
        .manage(AuthCenter::default())
        .setup(|app| {
            let show =
                MenuItem::with_id(app, TRAY_SHOW_ID, "打开 Codex Tools", true, None::<&str>)?;
            let exit = MenuItem::with_id(app, TRAY_EXIT_ID, "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &exit])?;
            let mut tray = TrayIconBuilder::with_id("main")
                .menu(&menu)
                .tooltip("Codex Tools")
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    TRAY_SHOW_ID => show_main_window(app),
                    TRAY_EXIT_ID => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;
            tauri::async_runtime::spawn(start_configured_route(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Resized(_) if window.is_minimized().unwrap_or(false) => {
                let _ = window.hide();
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            get_dashboard,
            list_providers,
            save_provider,
            delete_provider,
            fetch_provider_models,
            list_provider_accounts,
            save_provider_account,
            delete_provider_account,
            test_provider,
            activate_provider,
            get_proxy_status,
            scan_codex_data,
            repair_codex_data,
            list_sessions,
            export_sessions,
            delete_sessions_permanently,
            get_diagnostics,
            list_auth_accounts,
            activate_openai_account,
            delete_auth_account,
            start_openai_device_auth,
            poll_openai_device_auth,
            get_route_console,
            save_route_settings,
            clear_route_logs
        ])
        .run(tauri::generate_context!())
        .expect("运行 Codex Tools 失败");
}
