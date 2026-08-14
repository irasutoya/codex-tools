use crate::{
    activation::{activate_openai_record, sync_active_openai_credential},
    auth_center::{AuthCenter, DevicePollResult},
    chat_proxy::ChatProxyRegistry,
    codex::{self, ConfigManager},
    local_usage::UsageLedger,
    models::*,
    official_quota, proxy_import,
    state::{ActivationLock, ApiClient},
    storage::Store,
};
use futures_util::{StreamExt, TryStreamExt, stream};
use serde::Serialize;
use std::collections::HashSet;
use tauri::State;

const QUOTA_REFRESH_CONCURRENCY: usize = 4;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProxyImportResult {
    pub accounts: Vec<OfficialAccountView>,
    pub detected_formats: Vec<String>,
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
    sync_active_openai_credential(&store, &home)?;
    let total = imported.len();
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
    let existing_account_ids = store.read(|state| {
        state
            .official_accounts
            .iter()
            .map(|account| account.account_id.clone())
            .collect::<HashSet<_>>()
    })?;
    let new_account_count = resolved_accounts
        .iter()
        .map(|account| account.account_id.clone())
        .filter(|account_id| !existing_account_ids.contains(account_id))
        .collect::<HashSet<_>>()
        .len();
    if existing_account_ids.len() + new_account_count > crate::storage::MAX_SAVED_OPENAI_ACCOUNTS {
        return Err(AppError::InvalidConfig(
            "导入后将超过 500 个 OpenAI 账号的保存上限，请先删除不再使用的账号。".into(),
        ));
    }
    let saved_accounts = store.save_official_accounts(&resolved_accounts)?;
    let active_credential = store.read(|state| {
        if !matches!(state.active.kind, ActiveKind::Official) {
            return None;
        }
        let active_id = state.active.account_id.as_deref()?;
        state
            .official_accounts
            .iter()
            .find(|account| account.id == active_id)
            .map(|account| account.credential.clone())
    })?;
    if let Some(credential) = active_credential {
        codex::connections_activate_official_account(&home, &credential, None)?;
    }
    for account in saved_accounts {
        accounts.push(store.official_account_view(&account.id)?);
    }
    Ok(ProxyImportResult {
        accounts,
        detected_formats,
    })
}

#[cfg(test)]
async fn save_imported_account_and_sync_active(
    store: &Store,
    activation: &ActivationLock,
    account: &StoredOfficialAccount,
) -> Result<OfficialAccountView, AppError> {
    let _guard = activation.0.lock().await;
    let home = codex::home(&store.codex_home_setting()?);
    sync_active_openai_credential(store, &home)?;
    save_imported_account_and_sync_active_locked(store, &home, account)
}

#[cfg(test)]
fn save_imported_account_and_sync_active_locked(
    store: &Store,
    home: &std::path::Path,
    account: &StoredOfficialAccount,
) -> Result<OfficialAccountView, AppError> {
    let saved = store.save_official_account(account)?;
    let is_active = store.read(|state| {
        matches!(state.active.kind, ActiveKind::Official)
            && state.active.account_id.as_deref() == Some(saved.id.as_str())
    })?;
    if is_active {
        // A repeated Cookie import can replace the active single-use refresh
        // token. Update Codex immediately so a later sync cannot re-import the
        // superseded credential from auth.json.
        codex::connections_activate_official_account(home, &saved.credential, None)?;
    }
    store.official_account_view(&saved.id)
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
    Ok(saved
        .into_iter()
        .map(|account| {
            let active = active_account_id.as_deref() == Some(account.id.as_str());
            account.view(active)
        })
        .collect())
}

#[tauri::command]
pub(crate) async fn connections_refresh_login(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    activation: State<'_, ActivationLock>,
    id: String,
) -> Result<OfficialAccountView, AppError> {
    connections_refresh_login_in_store(&store, &center, &activation, &id).await
}

async fn connections_refresh_login_in_store(
    store: &Store,
    center: &AuthCenter,
    activation: &ActivationLock,
    id: &str,
) -> Result<OfficialAccountView, AppError> {
    let saved = refresh_account_and_sync_active(store, center, activation, id, true).await?;
    store.official_account_view(&saved.id)
}

async fn refresh_account_and_sync_active(
    store: &Store,
    center: &AuthCenter,
    activation: &ActivationLock,
    id: &str,
    force: bool,
) -> Result<StoredOfficialAccount, AppError> {
    let _guard = activation.0.lock().await;
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
    let is_active = store.read(|state| {
        matches!(state.active.kind, ActiveKind::Official)
            && state.active.account_id.as_deref() == Some(id)
    })?;
    if is_active {
        codex::connections_activate_official_account(&home, &saved.credential, None)?;
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
    let stored = store.official_account(account_id)?;
    let now = chrono::Utc::now().timestamp();
    let mut snapshot = stored.quota.clone();
    snapshot.last_attempt_at = Some(now);
    let account =
        match refresh_account_and_sync_active(store, center, activation, account_id, false).await {
            Ok(account) => account,
            Err(error) => {
                snapshot.status = match error {
                    AppError::InvalidConfig(_) => QuotaStatus::Unauthorized,
                    AppError::StaleOperation | AppError::Internal(_) => QuotaStatus::Error,
                };
                snapshot.error = Some(error.to_string());
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
pub(crate) async fn connections_login_poll(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    manager: State<'_, ConfigManager>,
    ledger: State<'_, UsageLedger>,
    activation: State<'_, ActivationLock>,
    proxy: State<'_, ChatProxyRegistry>,
    operation_id: String,
) -> Result<OpenAiDevicePoll, AppError> {
    match center.poll_openai(&operation_id).await? {
        DevicePollResult::Pending => Ok(OpenAiDevicePoll::Pending),
        DevicePollResult::Expired => Ok(OpenAiDevicePoll::Expired),
        DevicePollResult::Complete(account) => {
            let activation_operation = activation.begin_operation();
            let _guard = activation.0.lock().await;
            if !activation.is_current(activation_operation) {
                return Err(AppError::StaleOperation);
            }
            sync_active_openai_credential(&store, &codex::home(&store.codex_home_setting()?))?;
            let saved = store.save_official_account(&account)?;
            let repair = activate_openai_record(&store, &manager, &ledger, &proxy, &saved).await?;
            Ok(OpenAiDevicePoll::Complete {
                account: Box::new(store.official_account_view(&saved.id)?),
                repair,
            })
        }
    }
}

#[tauri::command]
pub(crate) async fn connections_activate_account(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    manager: State<'_, ConfigManager>,
    ledger: State<'_, UsageLedger>,
    activation: State<'_, ActivationLock>,
    proxy: State<'_, ChatProxyRegistry>,
    id: String,
) -> Result<RepairResult, AppError> {
    let activation_operation = activation.begin_operation();
    let _guard = activation.0.lock().await;
    if !activation.is_current(activation_operation) {
        return Err(AppError::StaleOperation);
    }
    sync_active_openai_credential(&store, &codex::home(&store.codex_home_setting()?))?;
    let saved = center.refresh_account(&store, &id).await?;
    activate_openai_record(&store, &manager, &ledger, &proxy, &saved).await
}

#[tauri::command]
pub(crate) async fn connections_activate_official(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    manager: State<'_, ConfigManager>,
    ledger: State<'_, UsageLedger>,
    activation: State<'_, ActivationLock>,
    proxy: State<'_, ChatProxyRegistry>,
) -> Result<RepairResult, AppError> {
    let activation_operation = activation.begin_operation();
    let _guard = activation.0.lock().await;
    if !activation.is_current(activation_operation) {
        return Err(AppError::StaleOperation);
    }
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
    let saved = center.refresh_account(&store, &id).await?;
    activate_openai_record(&store, &manager, &ledger, &proxy, &saved).await
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
    use std::fs;

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
    async fn active_account_refresh_and_reimport_paths_sync_cookie_to_codex() {
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

        let refreshed = refresh_account_and_sync_active(
            &store,
            &AuthCenter::default(),
            &ActivationLock::default(),
            &saved.id,
            false,
        )
        .await
        .unwrap();
        assert_eq!(refreshed.id, saved.id);

        let view = connections_refresh_login_in_store(
            &store,
            &AuthCenter::default(),
            &ActivationLock::default(),
            &saved.id,
        )
        .await
        .unwrap();

        assert!(view.active);
        assert_eq!(view.remark, "备用账号");
        let mut reimported = saved.clone();
        reimported.id.clear();
        reimported.remark.clear();
        reimported.credential.tokens.access_token = "at-cookie-reimported".into();
        reimported.credential.last_refresh = "2026-08-01T00:00:00Z".into();
        let imported_view =
            save_imported_account_and_sync_active(&store, &ActivationLock::default(), &reimported)
                .await
                .unwrap();

        assert!(imported_view.active);
        assert_eq!(imported_view.remark, "备用账号");
        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth["personal_access_token"], "at-cookie-reimported");
    }
}
