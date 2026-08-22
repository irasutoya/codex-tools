use crate::{
    activation::{
        activate_openai_record, ensure_codex_stopped, sync_active_codex_configuration,
        sync_active_openai_credential,
    },
    auth_center::{AuthCenter, DevicePollResult},
    chat_proxy::ChatProxyRegistry,
    codex::{self, ConfigManager},
    json_store::JsonStore,
    local_usage::UsageLedger,
    models::*,
    official_quota, proxy_import,
    session_index::SessionIndex,
    state::{ActivationLock, ApiClient},
    storage::Store,
};
use futures_util::{StreamExt, TryStreamExt, stream};
use serde::Serialize;
use std::path::Path;
use tauri::State;

const QUOTA_REFRESH_CONCURRENCY: usize = 4;

fn ensure_current_activation(activation: &ActivationLock, operation: u64) -> Result<(), AppError> {
    if activation.is_current(operation) {
        Ok(())
    } else {
        Err(AppError::StaleOperation)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyImportResult {
    pub accounts: Vec<OfficialAccountView>,
    pub detected_formats: Vec<String>,
}

#[derive(Serialize)]
struct AccountCredentialExport<'a> {
    format: &'static str,
    version: u8,
    exported_at: String,
    name: &'a str,
    email: &'a str,
    account_id: &'a str,
    expires_at: Option<i64>,
    tokens: AccountCredentialExportTokens<'a>,
}

#[derive(Serialize)]
struct AccountCredentialExportTokens<'a> {
    id_token: &'a str,
    access_token: &'a str,
    refresh_token: &'a str,
}

#[tauri::command]
pub(crate) fn connections_export_account(
    store: State<'_, Store>,
    id: String,
    path: String,
) -> Result<(), AppError> {
    connections_export_account_in_store(&store, &id, &path)
}

fn connections_export_account_in_store(
    store: &Store,
    id: &str,
    path: &str,
) -> Result<(), AppError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(AppError::InvalidConfig("导出路径不能为空。".into()));
    }
    let account = store.official_account(id)?;
    if account.credential.tokens.refresh_token.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "该账号没有可用的 Refresh Token，无法导出登录凭据。".into(),
        ));
    }
    let name = if account.remark.trim().is_empty() {
        account.name.as_str()
    } else {
        account.remark.as_str()
    };
    let export = AccountCredentialExport {
        format: "codex_tools_account",
        version: 1,
        exported_at: chrono::Utc::now().to_rfc3339(),
        name,
        email: &account.email,
        account_id: &account.account_id,
        expires_at: account.expires_at,
        tokens: AccountCredentialExportTokens {
            id_token: &account.credential.tokens.id_token,
            access_token: &account.credential.tokens.access_token,
            refresh_token: &account.credential.tokens.refresh_token,
        },
    };
    JsonStore::write_atomic(Path::new(path), &export).map_err(AppError::from)
}

#[tauri::command]
pub(crate) async fn connections_import_cookie(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    activation: State<'_, ActivationLock>,
    name: Option<String>,
    account_id: Option<String>,
    content: String,
) -> Result<ProxyImportResult, AppError> {
    connections_import_cookie_in_store(&store, &center, &activation, name, account_id, content)
        .await
}

async fn connections_import_cookie_in_store(
    store: &Store,
    center: &AuthCenter,
    activation: &ActivationLock,
    name: Option<String>,
    account_id: Option<String>,
    content: String,
) -> Result<ProxyImportResult, AppError> {
    let mut imported =
        proxy_import::parse_proxy_credentials(&content).map_err(AppError::InvalidConfig)?;
    if let Some(account_id) = account_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if imported.len() != 1 {
            return Err(AppError::InvalidConfig(
                "导入内容包含多个账号时，请不要填写统一的 Account ID；每个账号应在 JSON 中提供自己的标识。"
                    .into(),
            ));
        }
        imported[0].account_id = Some(account_id.to_owned());
    }
    let _guard = activation.0.lock().await;
    let home = codex::home(&store.codex_home_setting()?);
    sync_active_openai_credential(store, &home)?;
    let total = imported.len();
    if imported
        .iter()
        .any(|credential| credential.access_token.is_none())
    {
        let existing_count = store.read(|state| state.official_accounts.len())?;
        if existing_count.saturating_add(total) > crate::storage::MAX_SAVED_OPENAI_ACCOUNTS {
            return Err(AppError::InvalidConfig(
                "导入后可能超过 500 个 OpenAI 账号的保存上限，请先删除不再使用的账号。".into(),
            ));
        }
    }
    let detected_formats = imported
        .iter()
        .map(|item| item.source_format.label().to_string())
        .fold(Vec::<String>::new(), |mut formats, format| {
            if !formats.contains(&format) {
                formats.push(format);
            }
            formats
        });
    let mut accounts = Vec::with_capacity(total);
    let mut resolved_accounts = Vec::with_capacity(total);
    for (index, credential) in imported.into_iter().enumerate() {
        let requested_name = name.as_deref().map(|value| {
            if total <= 1 {
                value.to_owned()
            } else {
                format!("{} #{}", value, index + 1)
            }
        });
        resolved_accounts.push(
            center
                .connections_import_cookie(credential, requested_name)
                .await?,
        );
    }
    // Identity-aware storage treats a reimport as a credential update without
    // changing the active Codex files. Capacity is checked conservatively both
    // before and inside the atomic save transaction.
    store.ensure_official_account_capacity(&resolved_accounts)?;
    let saved_accounts = store.save_official_accounts(&resolved_accounts)?;
    for account in saved_accounts {
        accounts.push(store.official_account_view(&account.id)?);
    }
    Ok(ProxyImportResult {
        accounts,
        detected_formats,
    })
}

#[tauri::command]
pub(crate) fn connections_update_account_remark(
    store: State<'_, Store>,
    id: String,
    remark: String,
) -> Result<OfficialAccountView, AppError> {
    let saved = store.update_official_account_remark(&id, remark)?;
    store.official_account_view(&saved.id)
}

#[tauri::command]
pub(crate) fn connections_update_account_remarks(
    store: State<'_, Store>,
    updates: Vec<AccountRemarkUpdate>,
) -> Result<Vec<OfficialAccountView>, AppError> {
    connections_update_account_remarks_in_store(&store, updates)
}

fn connections_update_account_remarks_in_store(
    store: &Store,
    updates: Vec<AccountRemarkUpdate>,
) -> Result<Vec<OfficialAccountView>, AppError> {
    let saved = store.update_official_account_remarks(updates)?;
    let active_account_id = store.read(|state| {
        matches!(state.active.kind, ActiveKind::Official)
            .then(|| state.active.account_id.clone())
            .flatten()
    })?;
    saved
        .into_iter()
        .map(|account| {
            let active = active_account_id.as_deref() == Some(account.id.as_str());
            Ok(account.view(active))
        })
        .collect()
}

#[tauri::command]
pub(crate) async fn connections_refresh_login(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
    proxy: State<'_, ChatProxyRegistry>,
    id: String,
) -> Result<OfficialAccountView, AppError> {
    connections_refresh_login_in_store(&store, &center, &manager, &activation, &proxy, &id).await
}

async fn connections_refresh_login_in_store(
    store: &Store,
    center: &AuthCenter,
    manager: &ConfigManager,
    activation: &ActivationLock,
    proxy: &ChatProxyRegistry,
    id: &str,
) -> Result<OfficialAccountView, AppError> {
    let saved =
        refresh_account_and_sync_active(store, center, manager, activation, proxy, id, true)
            .await?;
    store.official_account_view(&saved.id)
}

#[allow(clippy::too_many_arguments)] // Shared refresh path needs the managed Tauri services.
async fn refresh_account_and_sync_active(
    store: &Store,
    center: &AuthCenter,
    manager: &ConfigManager,
    activation: &ActivationLock,
    proxy: &ChatProxyRegistry,
    id: &str,
    force: bool,
) -> Result<StoredOfficialAccount, AppError> {
    let _guard = activation.0.lock().await;
    let is_active = store.read(|state| {
        matches!(state.active.kind, ActiveKind::Official)
            && state.active.account_id.as_deref() == Some(id)
    })?;
    if is_active {
        ensure_codex_stopped(store)?;
    }
    let home = codex::home(&store.codex_home_setting()?);
    // Codex may have rotated the active credential independently. Import that
    // copy before exchanging its refresh token, then write the newly refreshed
    // credential back after it has safely reached persistent storage.
    sync_active_openai_credential(store, &home)?;
    let saved = if force {
        center.refresh_login(store, id).await?
    } else {
        center.refresh_account(store, id).await?
    };
    if is_active {
        sync_active_codex_configuration(store, manager, proxy).await?;
    }
    Ok(saved)
}

async fn refresh_official_quota(
    store: &Store,
    center: &AuthCenter,
    client: &ApiClient,
    activation: &ActivationLock,
    account_id: &str,
) -> Result<ProviderAccountQuota, AppError> {
    let _quota_guard = client.1.lock().await;
    let stored = store.official_account(account_id)?;
    let now = chrono::Utc::now().timestamp();
    let mut snapshot = stored.quota.clone();
    snapshot.last_attempt_at = Some(now);
    let account = match account_for_quota(store, center, activation, account_id).await {
        Ok(account) => account,
        Err(error) => {
            snapshot.status = match error {
                AppError::InvalidConfig(_) => QuotaStatus::Unauthorized,
                AppError::StaleOperation | AppError::Internal(_) => QuotaStatus::Error,
            };
            snapshot.error = Some(error.to_string());
            snapshot.error_code = None;
            return store.save_official_account_quota(account_id, snapshot);
        }
    };
    let http = client.current()?;
    let mut quota_result = official_quota::fetch_quota(&http, &account).await;
    if matches!(&quota_result, Err(error) if error.is_retryable()) {
        client.invalidate();
        let retry_http = client.current()?;
        quota_result = official_quota::fetch_quota(&retry_http, &account).await;
    }
    match quota_result {
        Ok(data) => {
            snapshot.status = QuotaStatus::Success;
            snapshot.data = Some(data.data);
            snapshot.plan_type = data.plan_type;
            snapshot.fetched_at = Some(now);
            snapshot.error = None;
            snapshot.error_code = None;
        }
        Err(error) => {
            snapshot.status = error.status;
            snapshot.error = Some(error.message);
            snapshot.error_code = error.code;
        }
    }
    store.save_official_account_quota(account_id, snapshot)
}

async fn account_for_quota(
    store: &Store,
    center: &AuthCenter,
    activation: &ActivationLock,
    account_id: &str,
) -> Result<StoredOfficialAccount, AppError> {
    let _guard = activation.0.lock().await;
    let stored = store.official_account(account_id)?;
    let is_active = store.read(|state| {
        matches!(state.active.kind, ActiveKind::Official)
            && state.active.account_id.as_deref() == Some(account_id)
    })?;
    // “刷新额度”对当前账号保持只读：不得轮换正在被 Codex 使用的凭据。
    if is_active {
        Ok(stored)
    } else {
        center.refresh_account(store, account_id).await
    }
}

#[tauri::command]
pub(crate) async fn connections_refresh_quota(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    client: State<'_, ApiClient>,
    activation: State<'_, ActivationLock>,
    account_id: String,
) -> Result<ProviderAccountQuota, AppError> {
    refresh_official_quota(&store, &center, &client, &activation, &account_id).await
}

#[tauri::command]
pub(crate) async fn connections_refresh_all_quota(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    client: State<'_, ApiClient>,
    activation: State<'_, ActivationLock>,
) -> Result<Vec<QuotaRefreshResult>, AppError> {
    connections_refresh_all_quota_in_store(&store, &center, &client, &activation).await
}

async fn connections_refresh_all_quota_in_store(
    store: &Store,
    center: &AuthCenter,
    client: &ApiClient,
    activation: &ActivationLock,
) -> Result<Vec<QuotaRefreshResult>, AppError> {
    let account_ids = store.read(|state| {
        state
            .official_accounts
            .iter()
            .map(|account| account.id.clone())
            .collect::<Vec<_>>()
    })?;
    let requests = account_ids.into_iter().map(|account_id| async move {
        Ok::<_, AppError>(QuotaRefreshResult {
            quota: refresh_official_quota(store, center, client, activation, &account_id).await?,
            account_id,
        })
    });
    stream::iter(requests)
        .buffered(QUOTA_REFRESH_CONCURRENCY)
        .try_collect::<Vec<_>>()
        .await
}

#[tauri::command]
pub(crate) async fn connections_login_start(
    center: State<'_, AuthCenter>,
) -> Result<OpenAiDeviceAuthorization, AppError> {
    center.start_openai().await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects each managed state separately.
pub(crate) async fn connections_login_poll(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    activation: State<'_, ActivationLock>,
    operation_id: String,
) -> Result<OpenAiDevicePoll, AppError> {
    match center.poll_openai(&operation_id).await? {
        DevicePollResult::Pending => Ok(OpenAiDevicePoll::Pending),
        DevicePollResult::Expired => Ok(OpenAiDevicePoll::Expired),
        DevicePollResult::Complete(account) => {
            let _guard = activation.0.lock().await;
            let saved = save_completed_login(&store, &account)?;
            center.ack_openai(&operation_id).await;
            Ok(OpenAiDevicePoll::Complete {
                account: Box::new(saved),
            })
        }
    }
}

fn save_completed_login(
    store: &Store,
    account: &StoredOfficialAccount,
) -> Result<OfficialAccountView, AppError> {
    sync_active_openai_credential(store, &codex::home(&store.codex_home_setting()?))?;
    let saved = store.save_official_account(account)?;
    store.official_account_view(&saved.id)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects each managed state separately.
pub(crate) async fn connections_activate_account(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    manager: State<'_, ConfigManager>,
    ledger: State<'_, UsageLedger>,
    activation: State<'_, ActivationLock>,
    proxy: State<'_, ChatProxyRegistry>,
    index: State<'_, SessionIndex>,
    id: String,
) -> Result<RepairResult, AppError> {
    let activation_operation = activation.begin_operation();
    let _guard = activation.0.lock().await;
    ensure_current_activation(&activation, activation_operation)?;
    official_quota::ensure_account_usable(&store.official_account(&id)?)?;
    ensure_codex_stopped(&store)?;
    sync_active_openai_credential(&store, &codex::home(&store.codex_home_setting()?))?;
    let saved = center.refresh_account(&store, &id).await?;
    ensure_current_activation(&activation, activation_operation)?;
    let repair = activate_openai_record(
        &store,
        &manager,
        &ledger,
        &proxy,
        &activation,
        activation_operation,
        &saved,
    )
    .await?;
    index.invalidate();
    Ok(repair)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri injects each managed state separately.
pub(crate) async fn connections_activate_official(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    manager: State<'_, ConfigManager>,
    ledger: State<'_, UsageLedger>,
    activation: State<'_, ActivationLock>,
    proxy: State<'_, ChatProxyRegistry>,
    index: State<'_, SessionIndex>,
) -> Result<RepairResult, AppError> {
    let activation_operation = activation.begin_operation();
    let _guard = activation.0.lock().await;
    ensure_current_activation(&activation, activation_operation)?;
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
    let id =
        id.ok_or_else(|| AppError::InvalidConfig("请先在“账号与服务”中登录 OpenAI。".into()))?;
    official_quota::ensure_account_usable(&store.official_account(&id)?)?;
    ensure_codex_stopped(&store)?;
    sync_active_openai_credential(&store, &codex::home(&home_setting))?;
    let saved = center.refresh_account(&store, &id).await?;
    ensure_current_activation(&activation, activation_operation)?;
    let repair = activate_openai_record(
        &store,
        &manager,
        &ledger,
        &proxy,
        &activation,
        activation_operation,
        &saved,
    )
    .await?;
    index.invalidate();
    Ok(repair)
}

#[tauri::command]
pub(crate) async fn connections_delete_account(
    store: State<'_, Store>,
    activation: State<'_, ActivationLock>,
    id: String,
) -> Result<(), AppError> {
    let _guard = activation.0.lock().await;
    store.delete_official_account(&id)
}

#[tauri::command]
pub(crate) async fn connections_delete_accounts(
    store: State<'_, Store>,
    activation: State<'_, ActivationLock>,
    ids: Vec<String>,
) -> Result<(), AppError> {
    connections_delete_accounts_in_store(&store, &activation, ids).await
}

async fn connections_delete_accounts_in_store(
    store: &Store,
    activation: &ActivationLock,
    ids: Vec<String>,
) -> Result<(), AppError> {
    let _guard = activation.0.lock().await;
    store.delete_official_accounts(ids)
}

#[tauri::command]
pub(crate) fn connections_open_login_page() -> Result<(), AppError> {
    platform_open()
}

fn platform_open() -> Result<(), AppError> {
    crate::platform::open_url("https://auth.openai.com/codex/device").map_err(|error| {
        AppError::Internal(format!(
            "无法自动打开 OpenAI 授权页面。请在浏览器中访问 https://auth.openai.com/codex/device：{error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use std::fs;

    #[test]
    fn stale_account_switch_is_rejected_after_an_async_boundary() {
        let activation = ActivationLock::default();
        let older = activation.begin_operation();
        let newer = activation.begin_operation();

        assert!(matches!(
            ensure_current_activation(&activation, older),
            Err(AppError::StaleOperation)
        ));
        assert!(ensure_current_activation(&activation, newer).is_ok());
    }

    fn oauth_account(account_id: &str, subject: &str, email: &str) -> StoredOfficialAccount {
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "sub": subject,
                "chatgpt_account_id": account_id,
                "email": email
            })
            .to_string()
            .as_bytes(),
        );
        let token = format!("header.{payload}.signature");
        let mut account = account(account_id, "");
        account.name = email.into();
        account.email = email.into();
        account.source = OfficialAccountSource::OpenAiOauth;
        account.credential.tokens.id_token = token.clone();
        account.credential.tokens.access_token = token;
        account.credential.tokens.refresh_token = format!("refresh-{subject}");
        account
    }

    fn account(account_id: &str, remark: &str) -> StoredOfficialAccount {
        StoredOfficialAccount {
            id: String::new(),
            name: account_id.into(),
            remark: remark.into(),
            account_id: account_id.into(),
            email: format!("{account_id}@example.test"),
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
            expires_at: None,
            quota: ProviderAccountQuota::default(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn exported_account_round_trips_through_cookie_import_without_mutating_store() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let mut saved_account = oauth_account("export-account", "export-user", "user@example.test");
        saved_account.remark = "迁移账号".into();
        saved_account.expires_at = Some(1_785_000_000);
        let saved = store.save_official_account(&saved_account).unwrap();
        let credentials_before = store.official_account(&saved.id).unwrap().credential;
        let path = temp.path().join("account.json");

        connections_export_account_in_store(&store, &saved.id, path.to_str().unwrap()).unwrap();

        let exported = fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(value["format"], "codex_tools_account");
        assert_eq!(value["version"], 1);
        assert_eq!(value["name"], "迁移账号");
        assert_eq!(value["account_id"], "export-account");
        assert_eq!(value["email"], "user@example.test");
        assert_eq!(value["expires_at"], 1_785_000_000);
        assert!(value["exported_at"].as_str().is_some());
        assert!(value.get("id").is_none());
        assert!(value.get("quota").is_none());

        let mut imported = proxy_import::parse_proxy_credentials(&exported).unwrap();
        assert_eq!(imported.len(), 1);
        let imported = imported.remove(0);
        assert_eq!(
            imported.id_token.as_deref(),
            Some(credentials_before.tokens.id_token.as_str())
        );
        assert_eq!(
            imported.access_token.as_deref(),
            Some(credentials_before.tokens.access_token.as_str())
        );
        assert_eq!(
            imported.refresh_token.as_deref(),
            Some(credentials_before.tokens.refresh_token.as_str())
        );
        assert_eq!(imported.account_id.as_deref(), Some("export-account"));
        assert_eq!(imported.suggested_name.as_deref(), Some("迁移账号"));
        assert_eq!(
            store.official_account(&saved.id).unwrap().credential,
            credentials_before
        );
    }

    #[test]
    fn account_export_rejects_invalid_requests_without_leaving_temporary_files() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let saved = store
            .save_official_account(&account("missing-refresh", ""))
            .unwrap();
        let missing_refresh_path = temp.path().join("missing-refresh.json");

        let error = connections_export_account_in_store(
            &store,
            &saved.id,
            missing_refresh_path.to_str().unwrap(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("Refresh Token"));
        assert!(!missing_refresh_path.exists());
        assert!(
            connections_export_account_in_store(&store, "missing", " ")
                .unwrap_err()
                .to_string()
                .contains("导出路径")
        );
        assert!(
            connections_export_account_in_store(
                &store,
                "missing",
                temp.path().join("missing-account.json").to_str().unwrap(),
            )
            .unwrap_err()
            .to_string()
            .contains("账号不存在")
        );

        let mut exportable = oauth_account("exportable", "subject", "export@example.test");
        exportable.remark.clear();
        let exportable = store.save_official_account(&exportable).unwrap();
        let unwritable_path = temp.path().join("directory-target");
        fs::create_dir(&unwritable_path).unwrap();
        assert!(
            connections_export_account_in_store(
                &store,
                &exportable.id,
                unwritable_path.to_str().unwrap(),
            )
            .is_err()
        );
        assert!(fs::read_dir(temp.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".directory-target")
        }));
    }

    #[test]
    fn completed_login_saves_the_account_without_switching_connections() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();

        let view = save_completed_login(&store, &account("new-account", "")).unwrap();

        assert!(!view.active);
        assert!(matches!(
            store.snapshot().unwrap().active.kind,
            ActiveKind::None
        ));
    }

    #[test]
    fn completed_login_adds_a_different_user_in_the_active_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let store = Store::open(temp.path().join("data")).unwrap();
        store
            .update(|state| {
                state.codex.home = home.display().to_string();
                Ok(())
            })
            .unwrap();
        let active = store
            .save_official_account(&oauth_account(
                "shared-workspace",
                "user-1",
                "first@example.test",
            ))
            .unwrap();
        store
            .connections_activate_official_account(&active.id)
            .unwrap();
        codex::connections_activate_official_account(&home, &active.credential, None).unwrap();
        let config_before = fs::read(home.join("config.toml")).unwrap();
        let auth_before = fs::read(home.join("auth.json")).unwrap();

        let added = save_completed_login(
            &store,
            &oauth_account("shared-workspace", "user-2", "second@example.test"),
        )
        .unwrap();

        assert_ne!(added.id, active.id);
        assert!(!added.active);
        let state = store.snapshot().unwrap();
        assert_eq!(state.official_accounts.len(), 2);
        assert_eq!(state.active.account_id.as_deref(), Some(active.id.as_str()));
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config_before);
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth_before);
    }

    #[test]
    fn completed_login_updates_the_same_user_and_preserves_local_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let store = Store::open(temp.path().join("data")).unwrap();
        store
            .update(|state| {
                state.codex.home = home.display().to_string();
                Ok(())
            })
            .unwrap();
        let mut original = oauth_account("shared-workspace", "user-1", "first@example.test");
        original.remark = "保留的备注".into();
        original.quota.status = QuotaStatus::Success;
        original.quota.fetched_at = Some(42);
        let original = store.save_official_account(&original).unwrap();
        store
            .connections_activate_official_account(&original.id)
            .unwrap();
        codex::connections_activate_official_account(&home, &original.credential, None).unwrap();
        let auth_before = fs::read(home.join("auth.json")).unwrap();
        let mut refreshed = oauth_account("shared-workspace", "user-1", "renamed@example.test");
        refreshed.credential.tokens.refresh_token = "refresh-updated".into();

        let updated = save_completed_login(&store, &refreshed).unwrap();

        assert_eq!(updated.id, original.id);
        assert!(updated.active);
        assert_eq!(updated.remark, "保留的备注");
        assert_eq!(updated.quota.status, QuotaStatus::Success);
        assert_eq!(updated.quota.fetched_at, Some(42));
        assert_eq!(store.snapshot().unwrap().official_accounts.len(), 1);
        assert_eq!(
            store
                .official_account(&original.id)
                .unwrap()
                .credential
                .tokens
                .refresh_token,
            "refresh-updated"
        );
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth_before);
    }

    #[tokio::test]
    async fn active_account_quota_refresh_does_not_rotate_or_write_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("auth.json"), b"external-codex-auth").unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        store
            .update(|state| {
                state.codex.home = home.display().to_string();
                Ok(())
            })
            .unwrap();
        let mut expired = account("active-account", "");
        expired.expires_at = Some(1);
        let saved = store.save_official_account(&expired).unwrap();
        store
            .connections_activate_official_account(&saved.id)
            .unwrap();

        let quota_account = account_for_quota(
            &store,
            &AuthCenter::default(),
            &ActivationLock::default(),
            &saved.id,
        )
        .await
        .unwrap();

        assert_eq!(quota_account.credential, saved.credential);
        assert_eq!(
            fs::read(home.join("auth.json")).unwrap(),
            b"external-codex-auth"
        );
    }

    #[test]
    fn batch_remark_command_returns_redacted_views_in_request_order() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let first = store
            .save_official_account(&account("first", "old-first"))
            .unwrap();
        let second = store
            .save_official_account(&account("second", "old-second"))
            .unwrap();
        store
            .connections_activate_official_account(&second.id)
            .unwrap();

        let views = connections_update_account_remarks_in_store(
            &store,
            vec![
                AccountRemarkUpdate {
                    id: second.id.clone(),
                    remark: "  new-second  ".into(),
                },
                AccountRemarkUpdate {
                    id: first.id.clone(),
                    remark: "new-first".into(),
                },
            ],
        )
        .unwrap();

        assert_eq!(
            views
                .iter()
                .map(|view| (view.id.as_str(), view.remark.as_str(), view.active))
                .collect::<Vec<_>>(),
            vec![
                (second.id.as_str(), "new-second", true),
                (first.id.as_str(), "new-first", false),
            ]
        );
        assert!(!serde_json::to_string(&views).unwrap().contains("access"));
    }

    #[tokio::test]
    async fn batch_delete_command_uses_activation_lock_and_store_batch_delete() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let first = store.save_official_account(&account("first", "")).unwrap();
        let active = store.save_official_account(&account("active", "")).unwrap();
        store
            .connections_activate_official_account(&active.id)
            .unwrap();
        let activation = ActivationLock::default();

        connections_delete_accounts_in_store(
            &store,
            &activation,
            vec![first.id.clone(), first.id.clone()],
        )
        .await
        .unwrap();
        assert!(store.official_account(&first.id).is_err());
        assert!(
            connections_delete_accounts_in_store(&store, &activation, vec![active.id.clone()],)
                .await
                .is_err()
        );
        assert!(store.official_account(&active.id).is_ok());
    }

    #[tokio::test]
    async fn connections_refresh_all_quota_continues_after_expired_proxy_accounts() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        for account_id in ["first", "second"] {
            store
                .save_official_account(&StoredOfficialAccount {
                    id: String::new(),
                    name: account_id.into(),
                    remark: String::new(),
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

        let results = connections_refresh_all_quota_in_store(
            &store,
            &AuthCenter::default(),
            &ApiClient::default(),
            &ActivationLock::default(),
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

    #[tokio::test]
    async fn active_account_refresh_syncs_cookie_to_codex() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let store = Store::open(temp.path().join("data")).unwrap();
        store
            .update(|state| {
                state.codex.home = home.display().to_string();
                Ok(())
            })
            .unwrap();
        let saved = store
            .save_official_account(&StoredOfficialAccount {
                id: String::new(),
                name: "Cookie".into(),
                remark: "备用账号".into(),
                account_id: "cookie-account".into(),
                email: String::new(),
                credential: CodexAuthCredential {
                    auth_mode: "chatgpt".into(),
                    openai_api_key: None,
                    tokens: CodexAuthTokens {
                        id_token: String::new(),
                        access_token: "at-cookie-secret".into(),
                        refresh_token: String::new(),
                        account_id: "cookie-account".into(),
                    },
                    last_refresh: "2026-07-31T00:00:00Z".into(),
                },
                source: OfficialAccountSource::ProxyImport,
                expires_at: Some(chrono::Utc::now().timestamp().saturating_add(3600)),
                quota: ProviderAccountQuota::default(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store
            .connections_activate_official_account(&saved.id)
            .unwrap();

        let manager = ConfigManager::default();
        let proxy = ChatProxyRegistry::default();
        let refreshed = refresh_account_and_sync_active(
            &store,
            &AuthCenter::default(),
            &manager,
            &ActivationLock::default(),
            &proxy,
            &saved.id,
            false,
        )
        .await
        .unwrap();
        assert_eq!(refreshed.id, saved.id);

        let view = connections_refresh_login_in_store(
            &store,
            &AuthCenter::default(),
            &manager,
            &ActivationLock::default(),
            &proxy,
            &saved.id,
        )
        .await
        .unwrap();

        assert!(view.active);
        assert_eq!(view.remark, "备用账号");
        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth["personal_access_token"], "at-cookie-secret");
    }

    #[tokio::test]
    async fn cookie_import_saves_a_new_account_without_changing_active_codex_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let store = Store::open(temp.path().join("data")).unwrap();
        store
            .update(|state| {
                state.codex.home = home.display().to_string();
                Ok(())
            })
            .unwrap();
        let active = store
            .save_official_account(&oauth_account(
                "shared-workspace",
                "active-user",
                "active@example.test",
            ))
            .unwrap();
        store
            .connections_activate_official_account(&active.id)
            .unwrap();
        codex::connections_activate_official_account(&home, &active.credential, None).unwrap();
        let config_before = fs::read(home.join("config.toml")).unwrap();
        let auth_before = fs::read(home.join("auth.json")).unwrap();

        let result = connections_import_cookie_in_store(
            &store,
            &AuthCenter::default(),
            &ActivationLock::default(),
            Some("新账号".into()),
            Some("shared-workspace".into()),
            "at-new-account".into(),
        )
        .await
        .unwrap();

        assert_eq!(result.accounts.len(), 1);
        assert!(!result.accounts[0].active);
        assert_eq!(result.accounts[0].account_id, "shared-workspace");
        assert_eq!(
            store.snapshot().unwrap().active.account_id.as_deref(),
            Some(active.id.as_str())
        );
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config_before);
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth_before);
    }

    #[tokio::test]
    async fn active_cookie_reimport_only_updates_saved_credentials_and_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        let store = Store::open(temp.path().join("data")).unwrap();
        store
            .update(|state| {
                state.codex.home = home.display().to_string();
                Ok(())
            })
            .unwrap();
        let mut saved_account = account("cookie-account", "保留的备注");
        saved_account.email.clear();
        let saved = store.save_official_account(&saved_account).unwrap();
        let quota = ProviderAccountQuota {
            status: QuotaStatus::Success,
            fetched_at: Some(42),
            ..Default::default()
        };
        store.save_official_account_quota(&saved.id, quota).unwrap();
        store
            .connections_activate_official_account(&saved.id)
            .unwrap();
        codex::connections_activate_official_account(&home, &saved.credential, None).unwrap();
        let config_before = fs::read(home.join("config.toml")).unwrap();
        let auth_before = fs::read(home.join("auth.json")).unwrap();

        let result = connections_import_cookie_in_store(
            &store,
            &AuthCenter::default(),
            &ActivationLock::default(),
            Some("更新后的名称".into()),
            Some("cookie-account".into()),
            "at-cookie-reimported".into(),
        )
        .await
        .unwrap();

        let view = &result.accounts[0];
        assert!(view.active);
        assert_eq!(view.id, saved.id);
        assert_eq!(view.remark, "保留的备注");
        assert_eq!(view.quota.status, QuotaStatus::Success);
        assert_eq!(view.quota.fetched_at, Some(42));
        assert_eq!(
            store
                .official_account(&saved.id)
                .unwrap()
                .credential
                .tokens
                .access_token,
            "at-cookie-reimported"
        );
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), config_before);
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), auth_before);
    }
}
