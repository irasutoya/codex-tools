use crate::{
    chat_proxy::ChatProxyRegistry,
    codex::{self, ConfigManager},
    commands::sessions::repair_home_after_activation,
    installation_id_proxy::InstallationIdProxyRegistry,
    models::{ActiveKind, AppError, RepairResult, StoredOfficialAccount},
    provider_sync,
    state::ActivationLock,
    storage::Store,
};
use std::fs;

const CODEX_RUNNING_WARNING: &str =
    "请先退出 Codex，再更改账号、连接或会话归属，以免当前会话丢失或认证状态被覆盖。";

pub(crate) fn ensure_codex_stopped(store: &Store) -> Result<(), AppError> {
    let configured = store.codex_app_setting()?;
    ensure_codex_stopped_with(configured.as_deref(), |configured| {
        #[cfg(test)]
        {
            let _ = configured;
            false
        }
        #[cfg(not(test))]
        {
            crate::platform::codex_app_running(configured)
        }
    })
}

fn ensure_codex_stopped_with(
    configured: Option<&str>,
    running: impl FnOnce(Option<&str>) -> bool,
) -> Result<(), AppError> {
    if running(configured) {
        return Err(AppError::InvalidConfig(CODEX_RUNNING_WARNING.into()));
    }
    Ok(())
}

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
    let untimestamped_personal_access_token = credential.last_refresh.trim().is_empty()
        && saved.source == crate::models::OfficialAccountSource::ProxyImport
        && crate::models::is_personal_access_token_credential(&credential);
    if credential.last_refresh.trim().is_empty() {
        credential.last_refresh = if saved.credential.last_refresh.trim().is_empty() {
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        } else {
            saved.credential.last_refresh.clone()
        };
    }
    if crate::models::official_credential_identity_matches(&saved, &credential)
        && saved.credential != credential
        && (untimestamped_personal_access_token
            || credential_is_strictly_newer(&credential, &saved.credential))
    {
        store.sync_official_credential(&record_id, &credential, saved.expires_at)?;
    }
    Ok(())
}

fn credential_is_strictly_newer(
    candidate: &crate::models::CodexAuthCredential,
    current: &crate::models::CodexAuthCredential,
) -> bool {
    let candidate = chrono::DateTime::parse_from_rfc3339(candidate.last_refresh.trim());
    let current = chrono::DateTime::parse_from_rfc3339(current.last_refresh.trim());
    matches!((candidate, current), (Ok(candidate), Ok(current)) if candidate > current)
}

/// 记录本次写入 config.toml 的服务模型。模型来自服务 `/models`，记录实际
/// 文件值是为了切回 OpenAI 时只清理由本应用管理的值。
pub(crate) fn record_written_model(store: &Store, home: &std::path::Path) -> Result<(), AppError> {
    let model = fs::read_to_string(home.join("config.toml"))
        .ok()
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|doc| {
            doc.get("model")
                .and_then(toml_edit::Item::as_str)
                .map(str::trim)
                .filter(|model| !model.is_empty())
                .map(str::to_owned)
        });
    store.save_last_managed_model(model)
}

/// 计算切换到 OpenAI 时应从 config.toml 清除的受管模型：只有 config 中的
/// 当前 model 与最近一次本应用写入的记录完全一致时才清除，避免误删用户
/// 手动设置的模型。
fn managed_model_to_remove(
    store: &Store,
    home: &std::path::Path,
) -> Result<Option<String>, AppError> {
    let recorded = store.last_managed_model()?;
    let current = fs::read_to_string(home.join("config.toml"))
        .ok()
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|doc| {
            doc.get("model")
                .and_then(toml_edit::Item::as_str)
                .map(str::to_owned)
        });
    Ok(recorded.filter(|record| current.as_deref() == Some(record.as_str())))
}

#[cfg(test)]
pub(crate) async fn sync_active_codex_configuration(
    store: &Store,
    manager: &ConfigManager,
    proxy: &ChatProxyRegistry,
) -> Result<(), AppError> {
    sync_active_codex_configuration_with_installation_proxy(
        store,
        manager,
        proxy,
        &InstallationIdProxyRegistry::default(),
    )
    .await
}

pub(crate) async fn sync_active_codex_configuration_with_installation_proxy(
    store: &Store,
    manager: &ConfigManager,
    proxy: &ChatProxyRegistry,
    installation_proxy: &InstallationIdProxyRegistry,
) -> Result<(), AppError> {
    ensure_codex_stopped(store)?;
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
            let relay = official_relay_base_url(store, installation_proxy, &account).await?;
            // 启动修复路径：active 已指向 OpenAI，但 config.toml 可能仍残留
            // 本应用上次写入的第三方服务模型；只有与最近一次写入记录一致
            // 才清除（用户手动设置的模型不受影响）。
            let managed_model = managed_model_to_remove(store, &home)?;
            codex::connections_activate_official_account_with_relay(
                &home,
                &account.credential,
                managed_model.as_deref(),
                relay.as_deref(),
            )?;
            store.save_last_managed_model(None)?;
            return Ok(());
        }
        ActiveKind::None => {
            installation_proxy.stop_all().await;
            return Ok(());
        }
        ActiveKind::Provider => {
            installation_proxy.stop_all().await;
        }
    }

    let provider_id = active.provider_id.as_deref().ok_or_else(|| {
        AppError::InvalidConfig("当前第三方 API 服务信息不完整，请重新选择。".into())
    })?;
    let mut provider = store.provider(provider_id)?;
    provider.normalize_and_validate()?;
    if !provider.enabled {
        return Err(AppError::InvalidConfig(
            "当前第三方 API 服务已不可用，请重新选择。".into(),
        ));
    }

    let target = crate::chat_proxy::effective_base_url(&provider, proxy).await?;
    let preview = manager.preview_custom(&home, &provider, &target)?;
    let previous_managed_model = store.last_managed_model()?;
    let applied = manager.apply(&preview.operation_id)?;
    if let Err(error) = record_written_model(store, &home) {
        let files = manager.rollback_applied(applied);
        let managed_model = store.save_last_managed_model(previous_managed_model);
        return match (files, managed_model) {
            (Ok(()), Ok(())) => Err(error),
            (files, managed_model) => Err(AppError::Internal(format!(
                "{error}；Codex 配置回滚失败：{}",
                files
                    .err()
                    .or_else(|| managed_model.err())
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "未知错误".into())
            ))),
        };
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)] // Coordinates the stores and relays that form one switch transaction.
pub(crate) async fn activate_openai_record(
    store: &Store,
    manager: &ConfigManager,
    ledger: &crate::local_usage::UsageLedger,
    proxy: &ChatProxyRegistry,
    installation_proxy: &InstallationIdProxyRegistry,
    activation: &ActivationLock,
    activation_operation: u64,
    account: &StoredOfficialAccount,
) -> Result<RepairResult, AppError> {
    ensure_codex_stopped(store)?;
    let home = codex::home(&store.codex_home_setting()?);
    let repair_sessions = provider_sync::configured_provider(&home) == codex::MANAGED_PROVIDER_ID;
    // 切换前计算本应用写入的服务模型，切回 OpenAI 时把它一并清除，
    // 避免 Codex 用第三方模型名去请求官方账号；只有与最近一次写入记录
    // 一致才清除（用户手动设置的模型不受影响）。
    let managed_model = managed_model_to_remove(store, &home)?;
    let pending_id = crate::begin_activation(
        ledger,
        &crate::activation_for_official(account, chrono::Utc::now().timestamp_millis()),
    )?;
    // 转换代理保持运行直到应用退出或服务被删除：端口在本机会话内保持稳定，
    // 切回 OpenAI 再切回第三方服务时，Codex 缓存的地址仍然有效。
    let result = async {
        let relay = official_relay_base_url(store, installation_proxy, account).await?;
        if !activation.is_current(activation_operation) {
            return Err(AppError::StaleOperation);
        }
        if let Err(error) = codex::connections_activate_official_account_with_relay(
            &home,
            &account.credential,
            managed_model.as_deref(),
            relay.as_deref(),
        ) {
            return Err(compensate_activation_failure_with_installation_proxy(
                store,
                manager,
                proxy,
                installation_proxy,
                error,
            )
            .await);
        }
        if let Err(error) = store.connections_activate_official_account(&account.id) {
            return Err(compensate_activation_failure_with_installation_proxy(
                store,
                manager,
                proxy,
                installation_proxy,
                error,
            )
            .await);
        }
        let repair = if repair_sessions {
            repair_home_after_activation(store, home, "openai".into()).await
        } else {
            RepairResult {
                target_provider: "openai".into(),
                ..RepairResult::default()
            }
        };
        Ok::<_, AppError>(repair)
    }
    .await;
    match result {
        Ok(mut repair) => {
            crate::confirm_pending(ledger, &pending_id, &mut repair);
            Ok(repair)
        }
        Err(error) => {
            crate::cancel_pending(ledger, &pending_id);
            Err(error)
        }
    }
}

pub(crate) async fn compensate_activation_failure_with_installation_proxy(
    store: &Store,
    manager: &ConfigManager,
    proxy: &ChatProxyRegistry,
    installation_proxy: &InstallationIdProxyRegistry,
    error: AppError,
) -> AppError {
    match sync_active_codex_configuration_with_installation_proxy(
        store,
        manager,
        proxy,
        installation_proxy,
    )
    .await
    {
        Ok(()) => error,
        Err(rollback) => AppError::Internal(format!(
            "{error}；原来的 Codex 连接也未能恢复，请重新选择账号或服务：{rollback}"
        )),
    }
}

async fn official_relay_base_url(
    store: &Store,
    registry: &InstallationIdProxyRegistry,
    account: &StoredOfficialAccount,
) -> Result<Option<String>, AppError> {
    let setting = store.official_installation_id_setting(&account.id)?;
    if !setting.enabled {
        registry.stop_all().await;
        return Ok(None);
    }
    let installation_id = setting
        .installation_id
        .as_deref()
        .ok_or_else(|| AppError::Internal("设备＋会话收敛已启用，但稳定设备标识缺失。".into()))?;
    let session_id = setting
        .session_id
        .as_deref()
        .ok_or_else(|| AppError::Internal("设备＋会话收敛已启用，但稳定会话标识缺失。".into()))?;
    Ok(Some(
        registry
            .ensure(installation_id, session_id, &account.account_id)
            .await?
            .base_url,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        CodexAuthCredential, CodexAuthTokens, OfficialAccountSource, ProviderAccountQuota,
        ProviderApiType, ProviderProfile, token_local_identity,
    };
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use std::fs;

    fn oauth_token(account_id: &str, subject: &str) -> String {
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({
                "sub": subject,
                "chatgpt_account_id": account_id,
                "email": format!("{subject}@example.test")
            })
            .to_string()
            .as_bytes(),
        );
        format!("header.{payload}.signature")
    }

    #[test]
    fn codex_mutation_gate_rejects_a_running_app() {
        let error = ensure_codex_stopped_with(Some("Codex.exe"), |_| true).unwrap_err();
        assert!(error.to_string().contains("请先退出 Codex"));
    }

    #[test]
    fn codex_mutation_gate_allows_a_stopped_app() {
        ensure_codex_stopped_with(None, |_| false).unwrap();
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
            .connections_save_provider(ProviderProfile {
                id: "provider".into(),
                name: "Provider".into(),
                base_url: "http://127.0.0.1:9/v1".into(),
                headers: Default::default(),
                timeout_secs: 30,
                enabled: true,
                active: false,
                model: String::new(),

                model_context_windows: Default::default(),
                available_models: vec!["api-model".into()],
                selected_models: None,
                custom_models: Default::default(),
                models_dev_meta: Default::default(),
                api_type: ProviderApiType::Responses,
                api_key: Some("secret".into()),
                has_api_key: false,
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store.activate(&provider.id).unwrap();

        sync_active_codex_configuration(
            &store,
            &ConfigManager::default(),
            &ChatProxyRegistry::default(),
        )
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
                remark: String::new(),
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
        // This regression covers the explicit-off/direct path. OAuth accounts
        // without a stored choice default to the local convergence relay.
        store
            .set_official_installation_id_unification(&saved.id, false)
            .unwrap();
        store
            .connections_activate_official_account(&saved.id)
            .unwrap();
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

        sync_active_codex_configuration(
            &store,
            &ConfigManager::default(),
            &ChatProxyRegistry::default(),
        )
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

    #[test]
    fn active_sync_only_accepts_a_strictly_newer_oauth_credential() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        fs::create_dir_all(&home).unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let current = CodexAuthCredential {
            auth_mode: "chatgpt".into(),
            openai_api_key: None,
            tokens: CodexAuthTokens {
                id_token: "id-current".into(),
                access_token: "access-current".into(),
                refresh_token: "refresh-current".into(),
                account_id: "workspace".into(),
            },
            last_refresh: "2026-08-01T00:00:00Z".into(),
        };
        let saved = store
            .save_official_account(&StoredOfficialAccount {
                id: String::new(),
                name: "OpenAI".into(),
                remark: String::new(),
                account_id: "workspace".into(),
                email: "person@example.test".into(),
                credential: current.clone(),
                source: OfficialAccountSource::OpenAiOauth,
                expires_at: None,
                quota: ProviderAccountQuota::default(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store
            .connections_activate_official_account(&saved.id)
            .unwrap();
        let mut candidate = CodexAuthCredential {
            tokens: CodexAuthTokens {
                id_token: "id-candidate".into(),
                access_token: "access-candidate".into(),
                refresh_token: "refresh-candidate".into(),
                account_id: "workspace".into(),
            },
            last_refresh: "2026-07-31T23:59:59Z".into(),
            ..current.clone()
        };
        for last_refresh in ["2026-07-31T23:59:59Z", "2026-08-01T00:00:00Z", "invalid"] {
            candidate.last_refresh = last_refresh.into();
            fs::write(
                home.join("auth.json"),
                serde_json::to_vec_pretty(&candidate).unwrap(),
            )
            .unwrap();

            sync_active_openai_credential(&store, &home).unwrap();

            assert_eq!(
                store.official_account(&saved.id).unwrap().credential,
                current
            );
        }

        candidate.last_refresh = "2026-08-01T00:00:01Z".into();
        fs::write(
            home.join("auth.json"),
            serde_json::to_vec_pretty(&candidate).unwrap(),
        )
        .unwrap();

        sync_active_openai_credential(&store, &home).unwrap();

        assert_eq!(
            store.official_account(&saved.id).unwrap().credential,
            candidate
        );
    }

    #[test]
    fn active_sync_rejects_a_different_user_in_the_same_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex-home");
        fs::create_dir_all(&home).unwrap();
        let store = Store::open(temp.path().join("data")).unwrap();
        let current_token = oauth_token("workspace", "user-1");
        let current = CodexAuthCredential {
            auth_mode: "chatgpt".into(),
            openai_api_key: None,
            tokens: CodexAuthTokens {
                id_token: current_token.clone(),
                access_token: current_token,
                refresh_token: "refresh-user-1".into(),
                account_id: "workspace".into(),
            },
            last_refresh: "2026-08-01T00:00:00Z".into(),
        };
        let saved = store
            .save_official_account(&StoredOfficialAccount {
                id: String::new(),
                name: "User 1".into(),
                remark: String::new(),
                account_id: "workspace".into(),
                email: "user-1@example.test".into(),
                credential: current.clone(),
                source: OfficialAccountSource::OpenAiOauth,
                expires_at: None,
                quota: ProviderAccountQuota::default(),
                created_at: 0,
                updated_at: 0,
            })
            .unwrap();
        store
            .connections_activate_official_account(&saved.id)
            .unwrap();
        let candidate_token = oauth_token("workspace", "user-2");
        let candidate = CodexAuthCredential {
            tokens: CodexAuthTokens {
                id_token: candidate_token.clone(),
                access_token: candidate_token,
                refresh_token: "refresh-user-2".into(),
                account_id: "workspace".into(),
            },
            last_refresh: "2026-08-02T00:00:00Z".into(),
            ..current.clone()
        };
        fs::write(
            home.join("auth.json"),
            serde_json::to_vec_pretty(&candidate).unwrap(),
        )
        .unwrap();

        sync_active_openai_credential(&store, &home).unwrap();

        assert_eq!(
            store.official_account(&saved.id).unwrap().credential,
            current
        );
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
                remark: String::new(),
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
        store
            .connections_activate_official_account(&saved.id)
            .unwrap();
        fs::write(
            home.join("auth.json"),
            r#"{"personal_access_token":"at-proxy-secret"}"#,
        )
        .unwrap();

        sync_active_codex_configuration(
            &store,
            &ConfigManager::default(),
            &ChatProxyRegistry::default(),
        )
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
