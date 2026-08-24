use crate::{
    activation::{sync_active_codex_configuration, sync_active_openai_credential},
    auth_center::AuthCenter,
    chat_proxy::ChatProxyRegistry,
    codex::{self, ConfigManager},
    models::{
        AppError, CredentialMaintenanceOutcome, CredentialMaintenanceResult,
        CredentialRefreshState, CredentialRefreshStatus, StoredOfficialAccount,
    },
    state::ActivationLock,
    storage::Store,
};

pub(crate) const REFRESH_WINDOW_SECS: i64 = 2 * 24 * 60 * 60;
pub(crate) const UNKNOWN_EXPIRY_RETRY_SECS: i64 = 24 * 60 * 60;
pub(crate) const RETRY_DELAYS_SECS: [i64; 4] = [60, 5 * 60, 15 * 60, 60 * 60];

fn codex_is_running(configured: Option<&str>) -> bool {
    #[cfg(test)]
    {
        let _ = configured;
        false
    }
    #[cfg(not(test))]
    {
        crate::platform::codex_app_running(configured)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaintenanceDecision {
    NotRefreshable,
    ReauthenticationRequired,
    WaitingRetry,
    Unchanged,
    Refresh,
}

pub(crate) fn needs_credential_refresh(expires_at: Option<i64>, now: i64) -> bool {
    expires_at.is_some_and(|expires_at| expires_at <= now.saturating_add(REFRESH_WINDOW_SECS))
}

pub(crate) fn maintenance_decision(
    account: &StoredOfficialAccount,
    state: &CredentialRefreshState,
    now: i64,
) -> MaintenanceDecision {
    if account.credential.tokens.refresh_token.trim().is_empty() {
        return MaintenanceDecision::NotRefreshable;
    }
    if state.status == CredentialRefreshStatus::ReauthenticationRequired {
        return MaintenanceDecision::ReauthenticationRequired;
    }
    if state.next_retry_at.is_some_and(|next| next > now) {
        return MaintenanceDecision::WaitingRetry;
    }
    if needs_credential_refresh(account.expires_at, now) {
        return MaintenanceDecision::Refresh;
    }
    if account.expires_at.is_none()
        && state
            .last_attempt_at
            .is_none_or(|last| now.saturating_sub(last) >= UNKNOWN_EXPIRY_RETRY_SECS)
    {
        return MaintenanceDecision::Refresh;
    }
    MaintenanceDecision::Unchanged
}

fn save_refresh_state_if_changed(
    store: &Store,
    id: &str,
    previous: &CredentialRefreshState,
    next: CredentialRefreshState,
) -> Result<(), AppError> {
    if previous != &next {
        store.save_credential_refresh_state(id, next)?;
    }
    Ok(())
}

pub(crate) fn healthy_state(now: i64) -> CredentialRefreshState {
    CredentialRefreshState {
        status: CredentialRefreshStatus::Healthy,
        last_attempt_at: Some(now),
        last_success_at: Some(now),
        next_retry_at: None,
        retry_count: 0,
    }
}

fn retry_state(previous: &CredentialRefreshState, now: i64) -> CredentialRefreshState {
    let retry_count = previous.retry_count.saturating_add(1);
    let index = usize::from(retry_count.saturating_sub(1)).min(RETRY_DELAYS_SECS.len() - 1);
    CredentialRefreshState {
        status: CredentialRefreshStatus::WaitingRetry,
        last_attempt_at: Some(now),
        last_success_at: previous.last_success_at,
        next_retry_at: Some(now.saturating_add(RETRY_DELAYS_SECS[index])),
        retry_count,
    }
}

fn reauthentication_state(previous: &CredentialRefreshState, now: i64) -> CredentialRefreshState {
    CredentialRefreshState {
        status: CredentialRefreshStatus::ReauthenticationRequired,
        last_attempt_at: Some(now),
        last_success_at: previous.last_success_at,
        next_retry_at: None,
        retry_count: previous.retry_count,
    }
}

fn not_refreshable_state(previous: &CredentialRefreshState) -> CredentialRefreshState {
    CredentialRefreshState {
        status: CredentialRefreshStatus::NotRefreshable,
        last_attempt_at: previous.last_attempt_at,
        last_success_at: previous.last_success_at,
        next_retry_at: None,
        retry_count: 0,
    }
}

fn managed_by_codex_state(
    previous: &CredentialRefreshState,
    now: i64,
    synced: bool,
) -> CredentialRefreshState {
    CredentialRefreshState {
        status: CredentialRefreshStatus::ManagedByCodex,
        last_attempt_at: previous.last_attempt_at,
        last_success_at: synced.then_some(now).or(previous.last_success_at),
        next_retry_at: None,
        retry_count: 0,
    }
}

/// 统一后台和“检查/更新登录”入口。它只在 Codex 已停止时尝试 OAuth；
/// 运行中仅读取 auth.json，并让 Codex 独占轮换 Refresh Token。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn maintain_account(
    store: &Store,
    center: &AuthCenter,
    manager: &ConfigManager,
    activation: &ActivationLock,
    proxy: &ChatProxyRegistry,
    id: &str,
) -> Result<CredentialMaintenanceResult, AppError> {
    maintain_account_with_running(
        store,
        center,
        manager,
        activation,
        proxy,
        id,
        codex_is_running,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn maintain_account_with_running(
    store: &Store,
    center: &AuthCenter,
    manager: &ConfigManager,
    activation: &ActivationLock,
    proxy: &ChatProxyRegistry,
    id: &str,
    running: impl FnOnce(Option<&str>) -> bool,
) -> Result<CredentialMaintenanceResult, AppError> {
    let _guard = activation.0.lock().await;
    let now = chrono::Utc::now().timestamp();
    let (account, state, active) = store
        .official_accounts_for_maintenance()?
        .into_iter()
        .find(|(account, _, _)| account.id == id)
        .ok_or_else(|| AppError::InvalidConfig("OpenAI 账号不存在，可能已被删除。".into()))?;
    let home = codex::home(&store.codex_home_setting()?);
    let configured = store.codex_app_setting()?;
    if active && running(configured.as_deref()) {
        let synced = sync_active_openai_credential(store, &home)?;
        let next = managed_by_codex_state(&state, now, synced);
        if synced {
            // 严格更新凭据会清除旧维护状态；即使显示状态未变也必须重新写入。
            store.save_credential_refresh_state(&account.id, next)?;
        } else {
            save_refresh_state_if_changed(store, &account.id, &state, next)?;
        }
        return Ok(CredentialMaintenanceResult {
            account: store.official_account_view(&account.id)?,
            outcome: if synced {
                CredentialMaintenanceOutcome::SyncedFromCodex
            } else {
                CredentialMaintenanceOutcome::ManagedByCodex
            },
        });
    }

    // 停止后先接收 Codex 最后的 auth.json，再决定是否需要轮换凭据。
    let synced = active && sync_active_openai_credential(store, &home)?;
    let account = store.official_account(&account.id)?;
    let decision = maintenance_decision(&account, &state, now);
    match decision {
        MaintenanceDecision::NotRefreshable => {
            save_refresh_state_if_changed(
                store,
                &account.id,
                &state,
                not_refreshable_state(&state),
            )?;
        }
        MaintenanceDecision::ReauthenticationRequired | MaintenanceDecision::WaitingRetry => {}
        MaintenanceDecision::Unchanged => {
            if state.status != CredentialRefreshStatus::Healthy {
                save_refresh_state_if_changed(store, &account.id, &state, healthy_state(now))?;
            }
        }
        MaintenanceDecision::Refresh => {
            let attempted = CredentialRefreshState {
                last_attempt_at: Some(now),
                ..state.clone()
            };
            store.save_credential_refresh_state(&account.id, attempted)?;
            match center
                .refresh_account_for_maintenance(store, &account.id)
                .await
            {
                Ok(result) if result.refreshed => {
                    store.save_credential_refresh_state(&account.id, healthy_state(now))?;
                    if active {
                        sync_active_codex_configuration(store, manager, proxy).await?;
                    }
                    return Ok(CredentialMaintenanceResult {
                        account: store.official_account_view(&account.id)?,
                        outcome: CredentialMaintenanceOutcome::Refreshed,
                    });
                }
                Ok(_) => {}
                Err(AppError::InvalidConfig(_)) => {
                    store.save_credential_refresh_state(
                        &account.id,
                        reauthentication_state(&state, now),
                    )?;
                }
                Err(error) => {
                    store.save_credential_refresh_state(&account.id, retry_state(&state, now))?;
                    return Err(error);
                }
            }
        }
    }
    // Codex 已停止时，未到刷新阈值的活动账号也要把已同步的当前凭据写回
    // auth.json，保证下一次启动读取的是同一个持久快照。
    if active {
        sync_active_codex_configuration(store, manager, proxy).await?;
    }
    Ok(CredentialMaintenanceResult {
        account: store.official_account_view(&account.id)?,
        outcome: if synced {
            CredentialMaintenanceOutcome::SyncedFromCodex
        } else {
            CredentialMaintenanceOutcome::Unchanged
        },
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_once(
    store: &Store,
    center: &AuthCenter,
    manager: &ConfigManager,
    activation: &ActivationLock,
    proxy: &ChatProxyRegistry,
) {
    let ids = match store.official_accounts_for_maintenance() {
        Ok(accounts) => accounts
            .into_iter()
            .map(|(account, _, _)| account.id)
            .collect::<Vec<_>>(),
        Err(_) => return,
    };
    for id in ids {
        // 单账号失败不阻断其他账号，状态已记录为重试或重新登录；错误本身不含响应体。
        let _ = maintain_account(store, center, manager, activation, proxy, &id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CodexAuthCredential, CodexAuthTokens, OfficialAccountSource, ProviderAccountQuota,
    };

    fn account(expires_at: Option<i64>) -> StoredOfficialAccount {
        StoredOfficialAccount {
            id: "account".into(),
            name: "账号".into(),
            remark: String::new(),
            account_id: "workspace".into(),
            email: "account@example.test".into(),
            credential: CodexAuthCredential {
                auth_mode: "chatgpt".into(),
                openai_api_key: None,
                tokens: CodexAuthTokens {
                    id_token: "test-id".into(),
                    access_token: "test-access".into(),
                    refresh_token: "test-refresh".into(),
                    account_id: "workspace".into(),
                },
                last_refresh: "2026-01-01T00:00:00Z".into(),
            },
            source: OfficialAccountSource::OpenAiOauth,
            expires_at,
            quota: ProviderAccountQuota::default(),
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn two_day_threshold_uses_an_inclusive_boundary() {
        let now = 1_800_000_000;
        assert!(!needs_credential_refresh(
            Some(now + REFRESH_WINDOW_SECS + 1),
            now
        ));
        assert!(needs_credential_refresh(
            Some(now + REFRESH_WINDOW_SECS),
            now
        ));
        assert!(needs_credential_refresh(
            Some(now + REFRESH_WINDOW_SECS - 1),
            now
        ));
        assert!(!needs_credential_refresh(Some(now + 5 * 24 * 60 * 60), now));
    }

    #[test]
    fn unknown_expiry_is_checked_at_most_once_per_day() {
        let now = 1_800_000_000;
        let account = account(None);
        let mut state = CredentialRefreshState {
            last_attempt_at: Some(now - UNKNOWN_EXPIRY_RETRY_SECS + 1),
            ..CredentialRefreshState::default()
        };
        assert_eq!(
            maintenance_decision(&account, &state, now),
            MaintenanceDecision::Unchanged
        );
        state.last_attempt_at = Some(now - UNKNOWN_EXPIRY_RETRY_SECS);
        assert_eq!(
            maintenance_decision(&account, &state, now),
            MaintenanceDecision::Refresh
        );
    }

    #[test]
    fn reauthentication_requirement_is_never_reclassified_as_healthy() {
        let now = 1_800_000_000;
        let account = account(Some(now));
        let state = CredentialRefreshState {
            status: CredentialRefreshStatus::ReauthenticationRequired,
            ..CredentialRefreshState::default()
        };
        assert_eq!(
            maintenance_decision(&account, &state, now),
            MaintenanceDecision::ReauthenticationRequired
        );
    }

    #[tokio::test]
    async fn reauthentication_requirement_persists_until_credentials_are_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let saved = store
            .save_official_account(&account(Some(chrono::Utc::now().timestamp())))
            .unwrap();
        let state = CredentialRefreshState {
            status: CredentialRefreshStatus::ReauthenticationRequired,
            last_attempt_at: Some(1),
            ..CredentialRefreshState::default()
        };
        store
            .save_credential_refresh_state(&saved.id, state.clone())
            .unwrap();

        let result = maintain_account_with_running(
            &store,
            &AuthCenter::default(),
            &ConfigManager::default(),
            &ActivationLock::default(),
            &ChatProxyRegistry::default(),
            &saved.id,
            |_| false,
        )
        .await
        .unwrap();

        assert_eq!(result.outcome, CredentialMaintenanceOutcome::Unchanged);
        assert_eq!(result.account.credential_refresh, state);
    }

    #[tokio::test]
    async fn running_codex_manages_an_expiring_active_account_without_oauth_or_auth_write() {
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
            .save_official_account(&account(Some(chrono::Utc::now().timestamp())))
            .unwrap();
        store
            .connections_activate_official_account(&saved.id)
            .unwrap();

        let result = maintain_account_with_running(
            &store,
            &AuthCenter::default(),
            &ConfigManager::default(),
            &ActivationLock::default(),
            &ChatProxyRegistry::default(),
            &saved.id,
            |_| true,
        )
        .await
        .unwrap();

        assert_eq!(result.outcome, CredentialMaintenanceOutcome::ManagedByCodex);
        assert_eq!(
            result.account.credential_refresh.status,
            CredentialRefreshStatus::ManagedByCodex
        );
        assert!(!home.join("auth.json").exists());
    }
}
