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
use tauri::State;

const QUOTA_REFRESH_CONCURRENCY: usize = 4;

#[tauri::command]
pub(crate) async fn connections_import_cookie(
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
    let account = center.connections_import_cookie(imported, name).await?;
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
    account_id: String,
) -> Result<ProviderAccountQuota, AppError> {
    refresh_official_quota(&store, &center, &client, &account_id).await
}

#[tauri::command]
pub(crate) async fn connections_refresh_all_quota(
    store: State<'_, Store>,
    center: State<'_, AuthCenter>,
    client: State<'_, ApiClient>,
) -> Result<Vec<QuotaRefreshResult>, AppError> {
    connections_refresh_all_quota_in_store(&store, &center, &client).await
}

async fn connections_refresh_all_quota_in_store(
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
    let requests = account_ids.into_iter().map(|account_id| async move {
        Ok::<_, AppError>(QuotaRefreshResult {
            quota: refresh_official_quota(store, center, client, &account_id).await?,
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
            let _guard = activation.0.lock().await;
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
    let _guard = activation.0.lock().await;
    sync_active_openai_credential(&store, &codex::home(&store.codex_home_setting()?))?;
    let refreshed = center
        .refresh_account(&store.official_account(&id)?)
        .await?;
    let saved = store.save_official_account(&refreshed)?;
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
    activate_openai_record(&store, &manager, &ledger, &proxy, &saved).await
}

#[tauri::command]
pub(crate) fn connections_delete_account(store: State<Store>, id: String) -> Result<(), AppError> {
    store.delete_official_account(&id)
}

#[tauri::command]
pub(crate) fn connections_open_login_page() -> Result<(), AppError> {
    platform_open()
}

fn platform_open() -> Result<(), AppError> {
    crate::platform::open_url("https://auth.openai.com/codex/device").map_err(|error| {
        AppError::Internal(format!(
            "无法打开 OpenAI 登录页面，请手动前往 https://auth.openai.com/codex/device：{error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connections_refresh_all_quota_continues_after_expired_proxy_accounts() {
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

        let results = connections_refresh_all_quota_in_store(
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
}
