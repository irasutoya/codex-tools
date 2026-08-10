use crate::{
    activation::{
        compensate_activation_failure, record_written_model, sync_active_codex_configuration,
        sync_active_openai_credential,
    },
    chat_proxy::ChatProxyRegistry,
    codex::{self, ConfigManager},
    commands::sessions::repair_home,
    local_usage::UsageLedger,
    models::*,
    models_dev, provider_http, provider_sync,
    state::{ActivationLock, ApiClient},
    storage::Store,
};
use std::collections::BTreeMap;
use tauri::State;

#[tauri::command]
pub(crate) fn connections_list(store: State<Store>) -> Result<ProviderOverview, AppError> {
    store.provider_overview()
}

#[tauri::command]
pub(crate) async fn connections_save_provider(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
    proxy: State<'_, ChatProxyRegistry>,
    client: State<'_, ApiClient>,
    provider: ProviderProfile,
) -> Result<ProviderProfile, AppError> {
    // 判断地址或 Key 是否变化：变化时才静默抓取 /models 可用模型，避免每次保存都请求。
    // 前端回传的是脱敏后的 api_key（为空），因此只有“用户新填了 Key”才算变化；
    // 否则每次编辑保存都会误判为 Key 变化而触发不必要的模型同步。
    let (is_new, url_changed, key_changed, saved) = {
        let _guard = activation.0.lock().await;
        let (is_new, url_changed, key_changed) = store.read(|state| {
            match state.providers.iter().find(|value| value.id == provider.id) {
                Some(existing) => (
                    false,
                    existing.base_url != provider.base_url,
                    provider
                        .api_key
                        .as_deref()
                        .is_some_and(|key| !key.trim().is_empty())
                        && existing.api_key.as_deref() != provider.api_key.as_deref(),
                ),
                None => (true, true, true),
            }
        })?;
        let saved = store.connections_save_provider(provider)?;
        // 若正在使用此服务，同步配置时（对 Chat 类型）会自动启动/重启本机转换代理。
        if store.is_active_provider(&saved.id)? {
            sync_active_codex_configuration(&store, &manager, &proxy).await?;
        }
        (is_new, url_changed, key_changed, saved)
    };
    // 静默获取服务 /models 接口返回的可用模型，并用 models.dev(catalog.json)
    // 精确匹配补充元数据（用户无感知；失败不打扰，保留已有数据）。
    // 放到激活锁之外执行：HTTP 请求可能耗时数秒，不应阻塞其他激活/保存操作。
    if (is_new || url_changed || key_changed)
        && saved
            .api_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty())
    {
        let _ = sync_provider_models(&client.current()?, &store, &saved).await;
    }
    Ok(saved.redacted())
}

#[tauri::command]
pub(crate) async fn connections_delete_provider(
    store: State<'_, Store>,
    proxy: State<'_, ChatProxyRegistry>,
    id: String,
) -> Result<(), AppError> {
    proxy.stop(&id).await;
    store.connections_delete_provider(&id)
}

#[tauri::command]
pub(crate) async fn connections_test_provider(
    store: State<'_, Store>,
    client: State<'_, ApiClient>,
    id: String,
) -> Result<ProviderTestResult, AppError> {
    let mut provider = store.provider(&id)?;
    provider.normalize_and_validate()?;
    if provider
        .api_key
        .as_deref()
        .is_none_or(|key| key.trim().is_empty())
    {
        return Err(AppError::InvalidConfig(
            "此服务还没有 API Key，请先编辑并填写。".into(),
        ));
    }
    let endpoint = provider_http::models_endpoint(&provider.base_url);
    let mut request = client
        .current()?
        .get(&endpoint)
        .headers(provider_http::custom_headers(&provider)?);
    let key = provider.api_key.as_deref().unwrap_or_default();
    request = request.bearer_auth(key);
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(provider.timeout_secs),
        request.send(),
    )
    .await
    .map_err(|_| AppError::InvalidConfig("连接超时，请检查网络和 API 地址后重试。".into()))?
    .map_err(|error| {
        AppError::InvalidConfig(format!("无法连接到服务，请检查网络和 API 地址：{error}"))
    })?;
    let status_code = response.status();
    let status = status_code.as_u16();
    let ok = provider_http::provider_test_succeeded(status_code);
    Ok(ProviderTestResult {
        ok,
        status,
        endpoint,
        message: if ok {
            if matches!(provider.api_type, ProviderApiType::Chat) {
                "模型列表接口可以访问，本机转换代理可正常接入该服务。".into()
            } else {
                "模型列表接口可以访问，Codex 可直接从此服务读取模型。".into()
            }
        } else {
            format!("连接测试未通过（HTTP {status}），请检查 API 地址、API Key 和服务状态。")
        },
        suggest_v1: status == 404 && !provider.base_url.ends_with("/v1"),
    })
}

#[tauri::command]
pub(crate) async fn connections_list_models(
    store: State<'_, Store>,
    client: State<'_, ApiClient>,
    id: String,
) -> Result<Vec<String>, AppError> {
    let mut provider = store.provider(&id)?;
    provider.normalize_and_validate()?;
    if provider
        .api_key
        .as_deref()
        .is_none_or(|key| key.trim().is_empty())
    {
        return Err(AppError::InvalidConfig(
            "此服务还没有 API Key，请先编辑并填写。".into(),
        ));
    }
    // 抓取 `/models` 可用模型并保存（含 models.dev 精确匹配的元数据）。
    sync_provider_models(&client.current()?, &store, &provider).await
}

/// 手动刷新当前激活服务商的模型列表（第三方 API 更新模型后使用）。
#[tauri::command]
pub(crate) async fn connections_refresh_models(
    store: State<'_, Store>,
    client: State<'_, ApiClient>,
) -> Result<Vec<String>, AppError> {
    let active = store.read(|state| state.active.clone())?;
    let provider_id = active
        .provider_id
        .filter(|_| matches!(active.kind, ActiveKind::Provider))
        .ok_or_else(|| {
            AppError::InvalidConfig("当前不是第三方 API 服务，无需刷新模型列表。".into())
        })?;
    let provider = store.provider(&provider_id)?;
    sync_provider_models(&client.current()?, &store, &provider).await
}

/// 抓取服务 `/models` 接口的可用模型，并用 models.dev（catalog.json）**精确
/// 匹配**（id 完全一致）补充展示名/上下文窗口/简介后保存；返回可用模型列表。
async fn sync_provider_models(
    client: &reqwest::Client,
    store: &Store,
    provider: &ProviderProfile,
) -> Result<Vec<String>, AppError> {
    let details = provider_http::fetch_model_details(client, provider).await?;
    let mut models: Vec<String> = details.iter().map(|detail| detail.id.clone()).collect();
    models.sort();
    models.dedup();
    let windows = details
        .iter()
        .filter_map(|detail| {
            detail
                .context_window
                .map(|window| (detail.id.clone(), window))
        })
        .collect::<BTreeMap<_, _>>();
    // 先并入 /models 返回的简介（服务商自己的数据优先），再补 models.dev 元数据。
    let mut meta = details
        .iter()
        .filter_map(|detail| {
            detail.description.clone().map(|description| {
                (
                    detail.id.clone(),
                    ProviderModelsDevMeta {
                        name: None,
                        context_window: None,
                        description: Some(description),
                    },
                )
            })
        })
        .collect::<BTreeMap<_, _>>();
    if let Ok(dev_meta) =
        models_dev::fetch_provider_meta(client, &provider.base_url, &provider.name).await
    {
        // 按 /models 返回的原始 id 解析 models.dev 元数据（兼容纯 id / 厂商前缀 / 大小写）。
        for id in &models {
            let Some(info) = models_dev::lookup_model(&dev_meta, id) else {
                continue;
            };
            let entry = meta.entry(id.clone()).or_default();
            if entry.name.is_none() {
                entry.name = info.name.clone();
            }
            if entry.context_window.is_none() {
                entry.context_window = info.context_window;
            }
            if entry.description.is_none() {
                entry.description = info.description.clone();
            }
        }
    }
    store.update_provider_models(&provider.id, models.clone(), windows, meta)?;
    Ok(models)
}

#[tauri::command]
pub(crate) async fn settings_preview_activation(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    proxy: State<'_, ChatProxyRegistry>,
    provider_id: Option<String>,
) -> Result<ConfigPatchPreview, AppError> {
    let (home_setting, active_provider_id) =
        store.read(|state| (state.codex.home.clone(), state.active.provider_id.clone()))?;
    let home = codex::home(&home_setting);
    let provider_id = provider_id
        .as_deref()
        .or(active_provider_id.as_deref())
        .ok_or_else(|| AppError::InvalidConfig("请先添加并启用一个第三方 API 服务。".into()))?;
    let provider = store.provider(provider_id)?;
    let effective_base_url = crate::chat_proxy::effective_base_url(&provider, &proxy).await?;
    manager.preview_custom(&home, &provider, &effective_base_url)
}

#[tauri::command]
pub(crate) async fn settings_apply_activation(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    activation: State<'_, ActivationLock>,
    operation_id: String,
) -> Result<(), AppError> {
    let _guard = activation.0.lock().await;
    manager.apply(&operation_id)?;
    // 写入成功后记录 config.toml 当前的默认模型，供切换到 OpenAI 时精确清除。
    let home = codex::home(&store.codex_home_setting()?);
    record_written_model(&store, &home)
}

#[tauri::command]
pub(crate) async fn connections_activate(
    store: State<'_, Store>,
    manager: State<'_, ConfigManager>,
    ledger: State<'_, UsageLedger>,
    activation: State<'_, ActivationLock>,
    proxy: State<'_, ChatProxyRegistry>,
    client: State<'_, ApiClient>,
    id: String,
) -> Result<RepairResult, AppError> {
    let repair = {
        let _guard = activation.0.lock().await;
        let mut provider = store.provider(&id)?;
        provider.normalize_and_validate()?;
        if !provider.enabled {
            return Err(AppError::InvalidConfig(
                "所选第三方 API 服务已停用，请检查后重试。".into(),
            ));
        }
        if provider
            .api_key
            .as_deref()
            .is_none_or(|key| key.trim().is_empty())
        {
            return Err(AppError::InvalidConfig(
                "此服务还没有 API Key，请先编辑并填写。".into(),
            ));
        }
        let effective_base_url = crate::chat_proxy::effective_base_url(&provider, &proxy).await?;
        let home = codex::home(&store.codex_home_setting()?);
        sync_active_openai_credential(&store, &home)?;
        let repair_sessions =
            provider_sync::configured_provider(&home) != codex::MANAGED_PROVIDER_ID;
        let preview = manager.preview_custom(&home, &provider, &effective_base_url)?;
        let pending_id = crate::begin_activation(
            &ledger,
            &crate::activation_for_provider(&provider, chrono::Utc::now().timestamp_millis()),
        )?;
        let result = async {
            manager.apply(&preview.operation_id)?;
            if let Err(error) = store.activate(&id) {
                return Err(compensate_activation_failure(&store, &manager, &proxy, error).await);
            }
            // 转换代理保持运行：Codex 会缓存配置里的地址，端口必须在本机会话内
            // 保持稳定，切回其他服务再切回来时才能继续使用同一端口。
            let repair = if repair_sessions {
                repair_home(home, codex::MANAGED_PROVIDER_ID.into()).await?
            } else {
                RepairResult {
                    target_provider: codex::MANAGED_PROVIDER_ID.into(),
                    ..RepairResult::default()
                }
            };
            Ok::<_, AppError>(repair)
        }
        .await;
        match result {
            Ok(mut repair) => {
                crate::confirm_pending(&ledger, &pending_id, &mut repair);
                Ok(repair)
            }
            Err(error) => {
                crate::cancel_pending(&ledger, &pending_id);
                Err(error)
            }
        }
    }?;
    // 切换后同步一次该服务商模型列表（/models + models.dev 元数据）。
    // 放到激活锁之外执行：HTTP 请求可能耗时数秒，不应阻塞其他激活/保存操作；
    // 失败不影响切换，保留已有数据。
    if let Ok(provider) = store.provider(&id) {
        let _ = sync_provider_models(&client.current()?, &store, &provider).await;
    }
    Ok(repair)
}
