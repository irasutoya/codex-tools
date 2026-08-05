use crate::{
    codex, commands::usage::local_today_range, local_usage::UsageLedger, models::*, platform,
    session_index::SessionIndex, storage::Store,
};
use tauri::State;

const CODEX_APP_URI: &str = "codex://";

#[derive(Debug, Default)]
struct ActiveProjection {
    home: String,
    provider_count: usize,
    active_provider: Option<String>,
    active_kind: ActiveKind,
    active_account_id: Option<String>,
    active_account: Option<String>,
    active_quota: Option<ProviderAccountQuota>,
}

fn project_active(state: &AppConfig) -> ActiveProjection {
    let active_stored_provider = state.active.provider_id.as_deref().and_then(|id| {
        state
            .providers
            .iter()
            .find(|provider| provider.profile.id == id)
    });
    let active_official_account = state
        .active
        .account_id
        .as_deref()
        .filter(|_| matches!(state.active.kind, ActiveKind::Official))
        .and_then(|id| {
            state
                .official_accounts
                .iter()
                .find(|account| account.id == id)
        });
    let active_account = active_stored_provider.and_then(|provider| {
        state
            .active
            .account_id
            .as_deref()
            .and_then(|id| provider.accounts.iter().find(|account| account.id == id))
    });
    let active_provider = active_stored_provider
        .map(|provider| provider.profile.name.clone())
        .or_else(|| active_official_account.map(|account| format!("OpenAI · {}", account.name)))
        .or_else(|| {
            matches!(state.active.kind, ActiveKind::Official).then(|| "OpenAI 账号".into())
        });
    ActiveProjection {
        home: state.codex.home.clone(),
        provider_count: state.providers.len(),
        active_provider,
        active_kind: state.active.kind,
        active_account_id: active_account
            .map(|account| account.id.clone())
            .or_else(|| active_official_account.map(|account| account.id.clone())),
        active_account: active_account
            .map(|account| account.name.clone())
            .or_else(|| active_official_account.map(|account| account.name.clone())),
        active_quota: active_official_account.map(|account| account.quota.clone()),
    }
}

#[tauri::command]
pub(crate) async fn get_dashboard(
    store: State<'_, Store>,
    index: State<'_, SessionIndex>,
    ledger: State<'_, UsageLedger>,
) -> Result<Dashboard, AppError> {
    let projection = store.read(project_active)?;
    let home = codex::home(&projection.home);
    let index = index.inner().clone();
    let scan_home = home.clone();
    let (session_count, database_count, database_health) =
        tokio::task::spawn_blocking(move || match index.load_with_database_count(&scan_home) {
            Ok((sessions, database_count)) => (sessions.len(), database_count, "可以读取".into()),
            Err(_) => (0, 0, "读取失败".into()),
        })
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let today_query = UsageQuery {
        range: local_today_range(),
        group_by: UsageGroupBy::Account,
    };
    let ledger = ledger.inner().clone();
    let today = tokio::task::spawn_blocking(move || ledger.query(today_query))
        .await
        .map_err(|error| AppError::Internal(error.to_string()))??;
    Ok(Dashboard {
        provider_count: projection.provider_count,
        active_provider: projection.active_provider,
        active_kind: projection.active_kind,
        active_account_id: projection.active_account_id,
        active_account: projection.active_account,
        active_quota: projection.active_quota,
        codex_home: home.display().to_string(),
        database_count,
        session_count,
        database_health,
        today_usage: today.totals.tokens,
        today_requests: today.totals.requests,
        today_estimated_cost_microusd: today.totals.estimated_cost_microusd,
        today_subscription_tokens: today.totals.subscription_tokens,
        today_unpriced_tokens: today.totals.unpriced_tokens,
        today_partial_tokens: today.totals.partial_tokens,
        today_unattributed_tokens: today.totals.unattributed_tokens,
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
pub(crate) fn get_settings_overview(store: State<Store>) -> Result<SettingsOverview, AppError> {
    settings_overview(&store)
}

#[tauri::command]
pub(crate) fn launch_codex() -> Result<(), AppError> {
    platform::open_url(CODEX_APP_URI).map_err(|error| {
        AppError::Internal(format!(
            "无法打开 Codex，请确认已安装 Codex 桌面应用：{error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_projection_identifies_active_provider_account() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let provider = store
            .save_provider(ProviderProfile {
                id: String::new(),
                name: "Provider".into(),
                base_url: "https://example.test/v1".into(),
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
                id: String::new(),
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

        let projection = store.read(project_active).unwrap();

        assert!(matches!(projection.active_kind, ActiveKind::Provider));
        assert_eq!(projection.provider_count, 1);
        assert_eq!(projection.active_provider.as_deref(), Some("Provider"));
        assert_eq!(
            projection.active_account_id.as_deref(),
            Some(account.id.as_str())
        );
        assert_eq!(projection.active_account.as_deref(), Some("Account"));
        assert!(projection.active_quota.is_none());
    }

    #[test]
    fn dashboard_projection_labels_active_official_account() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let saved = store
            .save_official_account(&StoredOfficialAccount {
                id: String::new(),
                name: "工作日账号".into(),
                account_id: "workspace".into(),
                email: "person@example.test".into(),
                credential: CodexAuthCredential {
                    auth_mode: "chatgpt".into(),
                    openai_api_key: None,
                    tokens: CodexAuthTokens {
                        id_token: "id-secret".into(),
                        access_token: "access-secret".into(),
                        refresh_token: "refresh-secret".into(),
                        account_id: "workspace".into(),
                    },
                    last_refresh: "2026-07-15T00:00:00Z".into(),
                },
                source: OfficialAccountSource::OpenAiOauth,
                expires_at: None,
                quota: ProviderAccountQuota::default(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store.activate_official_account(&saved.id).unwrap();

        let projection = store.read(project_active).unwrap();

        assert!(matches!(projection.active_kind, ActiveKind::Official));
        assert_eq!(
            projection.active_provider.as_deref(),
            Some("OpenAI · 工作日账号")
        );
        assert_eq!(
            projection.active_account_id.as_deref(),
            Some(saved.id.as_str())
        );
        assert_eq!(projection.active_account.as_deref(), Some("工作日账号"));
        assert!(projection.active_quota.is_some());
    }
}
