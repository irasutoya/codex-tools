mod auth_center;
mod codex;
mod models;
mod protocol_proxy;
mod provider_sync;
mod session_index;
mod storage;

use auth_center::AuthCenter;
use models::*;
use protocol_proxy::ProxyManager;
use storage::Store;
use tauri::{Manager, State};

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
    if provider.name.trim().is_empty()
        || provider.base_url.trim().is_empty()
        || provider.default_model.trim().is_empty()
    {
        return Err(AppError::InvalidConfig(
            "名称、Base URL 和默认模型不能为空".into(),
        ));
    }
    store.save_provider(&provider)?;
    Ok(provider)
}

#[tauri::command]
fn delete_provider(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_provider(&id)?;
    Ok(())
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
fn capture_official_account(
    store: State<Store>,
    name: String,
) -> Result<ProviderAccount, AppError> {
    let auth = codex::capture_official_auth()?;
    let email = auth
        .pointer("/tokens/id_token/email")
        .or_else(|| auth.pointer("/email"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let now = chrono::Utc::now().timestamp();
    let account = ProviderAccount {
        id: uuid::Uuid::new_v4().to_string(),
        provider_id: None,
        name: if name.trim().is_empty() {
            "官方账号".into()
        } else {
            name
        },
        auth_kind: AccountAuthKind::OfficialOauth,
        api_key: None,
        auth_json: Some(auth),
        headers: serde_json::json!({}),
        active: false,
        email,
        created_at: now,
        updated_at: now,
    };
    store.save_account(&account)?;
    Ok(account)
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
    let payload = if provider.protocol == ProviderProtocol::Responses {
        serde_json::json!({"model":provider.default_model,"input":"hi","max_output_tokens":8})
    } else {
        serde_json::json!({"model":provider.default_model,"messages":[{"role":"user","content":"hi"}],"max_tokens":8})
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
    preserve_current_official_login(&store)?;
    if !provider.enabled {
        return Err(AppError::InvalidConfig("Provider 已禁用".into()));
    }
    if account.provider_id.as_deref() != Some(&id) {
        return Err(AppError::InvalidConfig("账号不属于所选 Provider".into()));
    }
    let _ = force;
    let previous = store.active_state()?;
    let official_threads = if codex::is_official_mode() {
        session_index::rebuild(&store)?
            .into_iter()
            .filter(|session| session.provider == "openai" || session.original_provider == "openai")
            .map(|session| session.id)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    if !official_threads.is_empty() {
        store.remember_session_origins(&codex::home(), &official_threads, "openai")?;
    }
    let endpoint = if provider.protocol == ProviderProtocol::ChatCompletions {
        Some(proxy.prepare(&provider, &account).await?)
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
    let account_only_switch = previous.0.as_deref() == Some(id.as_str());
    if account_only_switch {
        if endpoint.is_some() {
            proxy.commit().await?;
        } else {
            proxy.stop().await;
        }
        return Ok(RepairResult {
            backup_path: backup,
            databases_repaired: 0,
            rows_updated: 0,
            warnings: vec![
                "仅切换账号：已更新 Codex 认证与配置，会话 Provider 未变化，因此未执行数据库修复。"
                    .into(),
            ],
        });
    }
    let result = match codex::repair(codex::MANAGED_PROVIDER_ID) {
        Ok(result) => result,
        Err(error) => {
            let config = codex::restore_provider_backup(&backup);
            let active = store.restore_active((previous.0.as_deref(), previous.1.as_deref()));
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
    } else {
        proxy.stop().await;
    }
    Ok(result)
}

#[tauri::command]
async fn activate_official_account(
    store: State<'_, Store>,
    proxy: State<'_, ProxyManager>,
    account_id: String,
) -> Result<RepairResult, AppError> {
    let _guard = proxy.activation_guard().await;
    let account = store.account(&account_id)?;
    if account.auth_kind != AccountAuthKind::OfficialOauth {
        return Err(AppError::InvalidConfig(
            "所选账号不是官方 OAuth 账号".into(),
        ));
    }
    let backup = codex::restore_official_account(&account)?;
    let origins = store.session_origins(&codex::home(), "openai")?;
    let mut result = match codex::restore_sessions_exact("openai", &origins) {
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
    if let Err(error) = store.activate_official(&account_id) {
        let _ = codex::restore_provider_backup(&backup);
        return Err(error.into());
    }
    store.forget_session_origins(&codex::home(), "openai")?;
    proxy.stop().await;
    result.warnings.push(format!(
        "已切换官方账号，并按迁移账本恢复了 {} 个会话的官方归属。",
        origins.len()
    ));
    if result.backup_path.is_empty() {
        result.backup_path = backup;
    }
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
    store: State<Store>,
    query: Option<String>,
) -> Result<Vec<SessionSummary>, AppError> {
    session_index::rebuild(&store)?;
    store.unified_sessions(query.as_deref()).map_err(Into::into)
}

fn preserve_current_official_login(store: &Store) -> Result<(), AppError> {
    if !codex::is_official_mode() {
        return Ok(());
    }
    let Ok(auth) = codex::capture_official_auth() else {
        return Ok(());
    };
    if store.auth_accounts()?.iter().any(|account| {
        account.service == AuthService::OpenAi && account.credential.as_ref() == Some(&auth)
    }) {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    store.save_auth_account(&AuthAccount {
        id: uuid::Uuid::new_v4().to_string(),
        service: AuthService::OpenAi,
        name: "自动保存的 OpenAI 登录".into(),
        login: None,
        email: auth
            .pointer("/tokens/id_token/email")
            .or_else(|| auth.pointer("/email"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        credential: Some(auth),
        config_snapshot: Some(codex::capture_official_config()),
        scopes: Vec::new(),
        expires_at: None,
        active: false,
        created_at: now,
        updated_at: now,
    })?;
    Ok(())
}

#[tauri::command]
fn list_auth_accounts(store: State<Store>) -> Result<Vec<AuthAccount>, AppError> {
    store.auth_accounts().map_err(Into::into)
}

#[tauri::command]
fn capture_openai_account(store: State<Store>, name: String) -> Result<AuthAccount, AppError> {
    let auth = codex::capture_official_auth()?;
    let now = chrono::Utc::now().timestamp();
    let account = AuthAccount {
        id: uuid::Uuid::new_v4().to_string(),
        service: AuthService::OpenAi,
        name: if name.trim().is_empty() {
            "OpenAI 官方账号".into()
        } else {
            name
        },
        login: None,
        email: auth
            .pointer("/tokens/id_token/email")
            .or_else(|| auth.pointer("/email"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        credential: Some(auth),
        config_snapshot: Some(codex::capture_official_config()),
        scopes: Vec::new(),
        expires_at: None,
        active: false,
        created_at: now,
        updated_at: now,
    };
    store.save_auth_account(&account)?;
    Ok(account)
}

#[tauri::command]
async fn activate_openai_account(
    store: State<'_, Store>,
    proxy: State<'_, ProxyManager>,
    id: String,
) -> Result<RepairResult, AppError> {
    let account = store.auth_account(&id)?;
    if account.service != AuthService::OpenAi {
        return Err(AppError::InvalidConfig("所选账号不是 OpenAI 账号".into()));
    }
    let auth = account
        .credential
        .as_ref()
        .ok_or(AppError::OfficialAuthMissing)?;
    let backup = codex::restore_official_snapshot(auth, account.config_snapshot.as_deref())?;
    let origins = store.session_origins(&codex::home(), "openai")?;
    let mut result = match codex::restore_sessions_exact("openai", &origins) {
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
    store.activate_auth_account(&id)?;
    store.forget_session_origins(&codex::home(), "openai")?;
    proxy.stop().await;
    result.warnings.push(format!(
        "已恢复官方认证和配置；仅按迁移账本恢复了 {} 个会话的官方归属。",
        origins.len()
    ));
    if result.backup_path.is_empty() {
        result.backup_path = backup;
    }
    Ok(result)
}

#[tauri::command]
fn delete_auth_account(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_auth_account(&id)?;
    Ok(())
}

#[tauri::command]
async fn start_github_device_auth(
    center: State<'_, AuthCenter>,
    client_id: String,
    scopes: Vec<String>,
) -> Result<GitHubDeviceAuthorization, AppError> {
    center.start_github(&client_id, &scopes).await
}

#[tauri::command]
async fn complete_github_device_auth(
    center: State<'_, AuthCenter>,
    store: State<'_, Store>,
    operation_id: String,
) -> Result<AuthAccount, AppError> {
    let account = center.complete_github(&operation_id).await?;
    store.save_auth_account(&account)?;
    Ok(account)
}

#[tauri::command]
async fn get_route_console(
    proxy: State<'_, ProxyManager>,
) -> Result<RouteConsoleSnapshot, AppError> {
    Ok(proxy.console().await)
}

#[tauri::command]
async fn stop_local_route(proxy: State<'_, ProxyManager>) -> Result<(), AppError> {
    proxy.stop().await;
    Ok(())
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
    let sessions = session_index::rebuild(&store).unwrap_or_default();
    Ok(Dashboard {
        provider_count: providers.len() as u64,
        active_provider: providers.into_iter().find(|p| p.active).map(|p| p.name),
        codex_home: codex::home().display().to_string(),
        database_count: scan.databases.len(),
        session_count: sessions.len(),
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
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(store)
        .manage(ProxyManager::default())
        .manage(AuthCenter::default())
        .invoke_handler(tauri::generate_handler![
            get_dashboard,
            list_providers,
            save_provider,
            delete_provider,
            list_provider_accounts,
            save_provider_account,
            delete_provider_account,
            capture_official_account,
            test_provider,
            activate_provider,
            activate_official_account,
            get_proxy_status,
            scan_codex_data,
            repair_codex_data,
            list_sessions,
            export_sessions,
            delete_sessions_permanently,
            get_diagnostics,
            list_auth_accounts,
            capture_openai_account,
            activate_openai_account,
            delete_auth_account,
            start_github_device_auth,
            complete_github_device_auth,
            get_route_console,
            stop_local_route,
            clear_route_logs
        ])
        .run(tauri::generate_context!())
        .expect("运行 Codex Tools 失败");
}
