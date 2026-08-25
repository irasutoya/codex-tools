use crate::{
    models::{
        CodexAuthCredential, CodexAuthTokens, OfficialAccountSource, ProviderAccountQuota,
        ProviderApiType, ProviderProfile, QuotaData, QuotaEstimate, QuotaStatus, QuotaWindow,
        StoredOfficialAccount,
    },
    storage::Store,
};
use std::collections::BTreeMap;

pub(crate) const DEV_DEMO_ENV: &str = "CODEX_TOOLS_DEV_DEMO";

const DEMO_ACCOUNT_ID: &str = "official-demo-account";
const DEMO_PROVIDER_ID: &str = "third-party-demo-provider";
const FIVE_HOURS_SECONDS: i64 = 18_000;
const SEVEN_DAYS_SECONDS: i64 = 604_800;

/// 仅允许 `tauri dev` 的 debug 二进制在收到显式开关时播种。
fn demo_mode_enabled(debug_build: bool, enabled: bool) -> bool {
    debug_build && enabled
}

fn is_enabled() -> bool {
    demo_mode_enabled(
        cfg!(debug_assertions),
        std::env::var_os(DEV_DEMO_ENV).is_some_and(|value| value == "1"),
    )
}

pub(crate) fn seed_if_enabled(store: &Store) -> Result<bool, crate::models::AppError> {
    seed_if_enabled_with(store, is_enabled())
}

fn seed_if_enabled_with(store: &Store, enabled: bool) -> Result<bool, crate::models::AppError> {
    if !enabled {
        return Ok(false);
    }

    let overview = store.provider_overview()?;
    if !overview.providers.is_empty() || !overview.official_accounts.is_empty() {
        return Ok(false);
    }

    let now = chrono::Utc::now().timestamp();
    let account = store.save_official_account(&demo_account())?;
    store.save_official_account_quota(&account.id, demo_quota(now))?;
    store.connections_save_provider_with_previous(demo_provider(), true)?;
    Ok(true)
}

fn demo_account() -> StoredOfficialAccount {
    StoredOfficialAccount {
        id: DEMO_ACCOUNT_ID.into(),
        name: "OpenAI 官方演示账号".into(),
        remark: "仅用于本地演示，不可用于真实服务".into(),
        account_id: "demo-workspace-not-real".into(),
        email: "official-demo@example.invalid".into(),
        credential: CodexAuthCredential {
            auth_mode: "personal_access_token".into(),
            openai_api_key: None,
            tokens: CodexAuthTokens {
                id_token: String::new(),
                access_token: "demo-personal-access-token-not-valid".into(),
                refresh_token: String::new(),
                account_id: "demo-workspace-not-real".into(),
            },
            last_refresh: "1970-01-01T00:00:00Z".into(),
        },
        source: OfficialAccountSource::OpenAiOauth,
        expires_at: None,
        quota: ProviderAccountQuota::default(),
        created_at: 0,
        updated_at: 0,
    }
}

fn demo_provider() -> ProviderProfile {
    ProviderProfile {
        id: DEMO_PROVIDER_ID.into(),
        name: "第三方演示 Provider".into(),
        base_url: "https://provider.demo.invalid/v1".into(),
        headers: BTreeMap::new(),
        timeout_secs: 30,
        enabled: true,
        active: false,
        model: String::new(),
        model_context_windows: BTreeMap::from([("demo-model-not-real".into(), 128_000)]),
        available_models: vec!["demo-model-not-real".into()],
        selected_models: Some(vec!["demo-model-not-real".into()]),
        custom_models: Vec::new(),
        models_dev_meta: BTreeMap::new(),
        api_type: ProviderApiType::Responses,
        api_key: Some("demo-provider-key-not-valid".into()),
        has_api_key: true,
        created_at: 0,
        updated_at: 0,
    }
}

fn demo_quota(now: i64) -> ProviderAccountQuota {
    let primary_reset_at = now.saturating_add(FIVE_HOURS_SECONDS);
    let secondary_reset_at = now.saturating_add(SEVEN_DAYS_SECONDS);
    ProviderAccountQuota {
        status: QuotaStatus::Success,
        data: Some(QuotaData::Windowed {
            primary: Some(QuotaWindow {
                used_percent: 42.5,
                remaining_percent: 57.5,
                window_seconds: Some(FIVE_HOURS_SECONDS),
                reset_at: Some(primary_reset_at),
            }),
            secondary: Some(QuotaWindow {
                used_percent: 68.0,
                remaining_percent: 32.0,
                window_seconds: Some(SEVEN_DAYS_SECONDS),
                reset_at: Some(secondary_reset_at),
            }),
        }),
        plan_type: Some("演示套餐（虚构）".into()),
        fetched_at: Some(now),
        last_attempt_at: Some(now),
        error: None,
        error_code: None,
        estimates: vec![
            QuotaEstimate {
                window_seconds: FIVE_HOURS_SECONDS,
                reset_at: primary_reset_at,
                estimated_total_microusd: 2_400_000,
                estimated_at: now,
            },
            QuotaEstimate {
                window_seconds: SEVEN_DAYS_SECONDS,
                reset_at: secondary_reset_at,
                estimated_total_microusd: 8_600_000,
                estimated_at: now,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_mode_requires_debug_build_and_explicit_flag() {
        assert!(demo_mode_enabled(true, true));
        assert!(!demo_mode_enabled(true, false));
        assert!(!demo_mode_enabled(false, true));
    }

    #[test]
    fn empty_store_is_seeded_once_with_inactive_demo_connections() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();

        assert!(seed_if_enabled_with(&store, true).unwrap());
        let overview = store.provider_overview().unwrap();
        assert_eq!(overview.official_accounts.len(), 1);
        assert_eq!(overview.providers.len(), 1);
        assert!(!overview.official_accounts[0].active);
        assert!(!overview.providers[0].active);
        assert_eq!(
            overview.official_accounts[0].quota.status,
            QuotaStatus::Success
        );
        assert_eq!(overview.official_accounts[0].quota.estimates.len(), 2);

        assert!(!seed_if_enabled_with(&store, true).unwrap());
        let repeated = store.provider_overview().unwrap();
        assert_eq!(repeated.official_accounts.len(), 1);
        assert_eq!(repeated.providers.len(), 1);
    }

    #[test]
    fn nonempty_store_is_not_overwritten_or_completed() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        let mut existing = demo_account();
        existing.id = "user-account".into();
        existing.remark = "用户已有数据".into();
        store.save_official_account(&existing).unwrap();

        assert!(!seed_if_enabled_with(&store, true).unwrap());
        let overview = store.provider_overview().unwrap();
        assert_eq!(overview.official_accounts.len(), 1);
        assert_eq!(overview.official_accounts[0].id, "user-account");
        assert_eq!(overview.official_accounts[0].remark, "用户已有数据");
        assert!(overview.providers.is_empty());
    }
}
