use crate::{
    codex::{self, ConfigManager},
    commands::sessions::repair_home,
    models::{ActiveKind, AppError, RepairResult, StoredOfficialAccount},
    provider_sync,
    storage::Store,
};

pub(crate) fn sync_active_openai_credential(
    store: &Store,
    home: &std::path::Path,
) -> Result<(), AppError> {
    let record_id = store.read(|state| {
        state
            .active
            .account_id
            .clone()
            .filter(|_| matches!(state.active.kind, ActiveKind::Official))
    })?;
    let Some(record_id) = record_id else {
        return Ok(());
    };
    let mut credential = match codex::read_official_account(home) {
        Ok(Some(credential)) => credential,
        Ok(None) | Err(AppError::InvalidConfig(_)) => return Ok(()),
        Err(error) => return Err(error),
    };
    let saved = store.official_account(&record_id)?;
    if credential.tokens.account_id.trim().is_empty()
        && saved.source == crate::models::OfficialAccountSource::ProxyImport
    {
        credential.tokens.account_id = saved.account_id.clone();
    }
    if credential.last_refresh.trim().is_empty() {
        credential.last_refresh = if saved.credential.last_refresh.trim().is_empty() {
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        } else {
            saved.credential.last_refresh.clone()
        };
    }
    if saved.account_id == credential.tokens.account_id && saved.credential != credential {
        store.sync_official_credential(&record_id, &credential, saved.expires_at)?;
    }
    Ok(())
}

pub(crate) async fn sync_active_codex_configuration(
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

pub(crate) async fn activate_openai_record(
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

pub(crate) async fn compensate_activation_failure(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AccountAuthKind, CodexAuthCredential, CodexAuthTokens, OfficialAccountSource,
        ProviderAccount, ProviderAccountQuota, ProviderProfile, token_local_identity,
    };
    use std::fs;

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

    #[tokio::test]
    async fn proxy_personal_token_syncs_external_credential_into_store() {
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
        let account_id = token_local_identity("at-proxy-secret");
        let credential = CodexAuthCredential {
            auth_mode: "chatgpt".into(),
            openai_api_key: None,
            tokens: CodexAuthTokens {
                id_token: String::new(),
                access_token: "stale-access".into(),
                refresh_token: String::new(),
                account_id: account_id.clone(),
            },
            last_refresh: "2026-07-31T00:00:00Z".into(),
        };
        let saved = store
            .save_official_account(&StoredOfficialAccount {
                id: String::new(),
                name: "Cookie".into(),
                account_id: account_id.clone(),
                email: String::new(),
                credential: credential.clone(),
                source: OfficialAccountSource::ProxyImport,
                expires_at: None,
                quota: ProviderAccountQuota::default(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store.activate_official_account(&saved.id).unwrap();
        fs::write(
            home.join("auth.json"),
            r#"{"personal_access_token":"at-proxy-secret"}"#,
        )
        .unwrap();

        sync_active_codex_configuration(&store, &ConfigManager::default())
            .await
            .unwrap();

        let synced = store.official_account(&saved.id).unwrap();
        assert_eq!(synced.credential.tokens.access_token, "at-proxy-secret");
        assert_eq!(synced.credential.tokens.account_id, account_id);
        assert_eq!(synced.credential.last_refresh, credential.last_refresh);
        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth["personal_access_token"], "at-proxy-secret");
    }
}
