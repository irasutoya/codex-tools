use crate::{
    model_unlock::{self, MODEL_CATALOG_DIR, MODEL_CATALOG_FILE},
    models::{
        AppError, CodexAuthCredential, ConfigInspection, ConfigPatchPreview, ProviderApiType,
        ProviderProfile, is_personal_access_token_credential, token_identity, token_local_identity,
        validate_official_credential,
    },
    storage::atomic_write,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};
use toml_edit::{DocumentMut, Item, Table, Value, value};

pub const MANAGED_PROVIDER_ID: &str = "custom";
const MAX_CODEX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CODEX_AUTH_BYTES: u64 = 4 * 1024 * 1024;

/// 序列化模型目录内容；没有可用模型时返回 `None`（应用时跳过写目录，
/// 避免用空目录覆盖正在使用的模型目录）。
fn catalog_bytes(catalog: &[crate::models::CodexModelInfo]) -> Option<Vec<u8>> {
    if catalog.is_empty() {
        return None;
    }
    let mut bytes =
        serde_json::to_vec_pretty(&serde_json::json!({ "models": catalog })).unwrap_or_default();
    bytes.push(b'\n');
    Some(bytes)
}

#[derive(Clone)]
struct PendingPatch {
    base_hash: String,
    auth_base_hash: String,
    target: PathBuf,
    rendered: String,
    auth_rendered: Vec<u8>,
    /// 模型目录文件内容（`{"models": [...]}`）；为空表示没有可用模型，
    /// 应用时跳过写目录，避免用空目录覆盖正在使用的目录。
    catalog: Option<Vec<u8>>,
    created_at: Instant,
}

struct PatchDraft<'a> {
    target: PathBuf,
    original: &'a str,
    original_auth: &'a [u8],
    rendered: String,
    auth_rendered: Vec<u8>,
    catalog: Option<Vec<u8>>,
    public_preview: String,
    changes: Vec<String>,
    api_key: &'a str,
}

#[derive(Default)]
pub struct ConfigManager {
    pending: Mutex<HashMap<String, PendingPatch>>,
}

impl ConfigManager {
    pub fn preview_custom(
        &self,
        codex_home: &Path,
        provider: &ProviderProfile,
        effective_base_url: &str,
    ) -> Result<ConfigPatchPreview, AppError> {
        let path = codex_home.join("config.toml");
        let original = read_optional(&path)?;
        let original_auth = read_optional_bytes(&path.with_file_name("auth.json"))?;
        let api_key = provider
            .api_key
            .as_deref()
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| AppError::InvalidConfig("API Key 为空，请重新填写。".into()))?;
        // Chat 类型服务经本机转换代理接入：写入 Codex auth.json 的是固定的
        // 占位 Key，真实的服务商 Key 只保存在本应用，由转换代理注入上游请求。
        let auth_key = if matches!(provider.api_type, ProviderApiType::Chat) {
            crate::chat_proxy::PROXY_FIXED_API_KEY
        } else {
            api_key
        };
        let headers = merged_headers(provider);
        // 准备模型目录内容，但不在此写入磁盘：预览是只读操作，目录文件在
        // 真正应用时才落盘，避免未应用的预览覆盖正在使用的模型目录。
        let catalog =
            catalog_bytes(&model_unlock::build_model_catalog_with_windows_for_provider(provider));
        let mut document = parse_config_document(&original)?;
        apply_custom_fields(
            &mut document,
            &provider.name,
            effective_base_url,
            &headers,
            &provider.model,
        )?;
        let rendered = document.to_string();
        let auth_rendered = render_api_key_auth(auth_key)?;
        let public_preview = managed_custom_preview(&document, auth_key, &headers);
        let changes = describe_changes(
            &provider.model,
            matches!(provider.api_type, ProviderApiType::Chat),
        );
        self.remember(PatchDraft {
            target: path,
            original: &original,
            original_auth: &original_auth,
            rendered,
            auth_rendered,
            catalog,
            public_preview,
            changes,
            api_key: auth_key,
        })
    }

    pub fn apply(&self, operation_id: &str) -> Result<(), AppError> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| AppError::Internal("配置预览暂时不可用，请重启应用后再试。".into()))?
            .remove(operation_id)
            .ok_or(AppError::StaleOperation)?;
        let current = read_optional(&pending.target)?;
        if digest(&current) != pending.base_hash {
            return Err(AppError::StaleOperation);
        }
        let auth = read_optional_bytes(&pending.target.with_file_name("auth.json"))?;
        if digest_bytes(&auth) != pending.auth_base_hash {
            return Err(AppError::StaleOperation);
        }
        let _: DocumentMut = pending.rendered.parse().map_err(|error| {
            AppError::InvalidConfig(format!("生成的 Codex 配置格式不正确，请重新预览：{error}"))
        })?;
        let _: serde_json::Map<String, serde_json::Value> =
            parse_auth_object(&pending.auth_rendered)?;
        // 先写模型目录文件，再写 config.toml，保证 model_catalog_json 指向
        // 的内容在配置生效前已就绪；目录为空时不覆盖现有文件。
        if let Some(catalog) = pending.catalog.as_deref()
            && let Some(parent) = pending.target.parent()
        {
            let catalog_path = parent.join(MODEL_CATALOG_DIR).join(MODEL_CATALOG_FILE);
            atomic_write(&catalog_path, catalog).map_err(AppError::from)?;
        }
        commit_codex_files(
            &pending.target,
            pending.rendered.as_bytes(),
            &pending.auth_rendered,
            |path, bytes| atomic_write(path, bytes).map_err(AppError::from),
        )
    }

    fn remember(&self, draft: PatchDraft<'_>) -> Result<ConfigPatchPreview, AppError> {
        let PatchDraft {
            target,
            original,
            original_auth,
            rendered,
            auth_rendered,
            catalog,
            public_preview,
            changes,
            api_key,
        } = draft;
        let operation_id = uuid::Uuid::new_v4().to_string();
        let base_hash = digest(original);
        let auth_base_hash = digest_bytes(original_auth);
        let target_path = target.display().to_string();
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| AppError::Internal("配置预览暂时不可用，请重启应用后再试。".into()))?;
        pending.retain(|_, patch| patch.created_at.elapsed() < Duration::from_secs(600));
        if pending.len() >= 32
            && let Some(oldest) = pending
                .iter()
                .min_by_key(|(_, patch)| patch.created_at)
                .map(|(id, _)| id.clone())
        {
            pending.remove(&oldest);
        }
        pending.insert(
            operation_id.clone(),
            PendingPatch {
                base_hash: base_hash.clone(),
                auth_base_hash,
                target,
                rendered,
                auth_rendered,
                catalog,
                created_at: Instant::now(),
            },
        );
        Ok(ConfigPatchPreview {
            operation_id,
            target_path,
            base_hash,
            rendered: public_preview,
            changes,
            api_key_masked: mask_secret(api_key),
        })
    }
}

pub fn home(configured: &str) -> PathBuf {
    if !configured.trim().is_empty() {
        return PathBuf::from(configured.trim());
    }
    if let Some(value) = std::env::var_os("CODEX_HOME") {
        return PathBuf::from(value);
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}

pub fn connections_activate_official_account(
    codex_home: &Path,
    credential: &CodexAuthCredential,
    managed_model: Option<&str>,
) -> Result<(), AppError> {
    validate_official_credential(credential)?;
    fs::create_dir_all(codex_home)?;
    let config_path = codex_home.join("config.toml");
    let mut document = parse_config_document(&read_optional(&config_path)?)?;
    clear_custom_provider_fields(&mut document, managed_model)?;

    let mut auth_rendered = if is_personal_access_token_credential(credential) {
        serde_json::to_vec_pretty(&serde_json::json!({
            "OPENAI_API_KEY": null,
            "personal_access_token": credential.tokens.access_token,
        }))
    } else {
        serde_json::to_vec_pretty(credential)
    }
    .map_err(|error| AppError::Internal(error.to_string()))?;
    auth_rendered.push(b'\n');

    commit_codex_files(
        &config_path,
        document.to_string().as_bytes(),
        &auth_rendered,
        |path, bytes| atomic_write(path, bytes).map_err(AppError::from),
    )
}

pub fn read_official_account(codex_home: &Path) -> Result<Option<CodexAuthCredential>, AppError> {
    let path = codex_home.join("auth.json");
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if bytes.iter().all(u8::is_ascii_whitespace) || bytes == b"{}" || bytes == b"{}\n" {
        return Ok(None);
    }
    let credential = match serde_json::from_slice::<CodexAuthCredential>(&bytes) {
        Ok(credential) => credential,
        Err(_) => {
            let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
                AppError::InvalidConfig("Codex 的登录文件无法识别，将在切换账号时重新写入。".into())
            })?;
            let personal_access_token = value
                .get("personal_access_token")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::InvalidConfig(
                        "Codex 的登录文件无法识别，将在切换账号时重新写入。".into(),
                    )
                })?;
            let identity = token_identity(personal_access_token);
            let account_id = identity
                .as_ref()
                .and_then(|identity| identity.account_id.clone())
                .unwrap_or_else(|| token_local_identity(personal_access_token));
            CodexAuthCredential {
                auth_mode: "chatgpt".into(),
                openai_api_key: None,
                tokens: crate::models::CodexAuthTokens {
                    id_token: String::new(),
                    access_token: personal_access_token.to_owned(),
                    refresh_token: String::new(),
                    account_id,
                },
                last_refresh: String::new(),
            }
        }
    };
    validate_official_credential(&credential)?;
    Ok(Some(credential))
}

pub fn inspect(codex_home: &Path) -> ConfigInspection {
    let path = codex_home.join("config.toml");
    let text = match read_optional(&path) {
        Ok(text) => text,
        Err(error) => {
            return ConfigInspection {
                path: path.display().to_string(),
                valid: false,
                active_provider: None,
                managed_provider_present: false,
                warnings: vec![error.to_string()],
            };
        }
    };
    match text.parse::<DocumentMut>() {
        Ok(document) => {
            let custom = document
                .get("model_providers")
                .and_then(Item::as_table)
                .and_then(|providers| providers.get(MANAGED_PROVIDER_ID))
                .and_then(Item::as_table);
            ConfigInspection {
                path: path.display().to_string(),
                valid: true,
                active_provider: document
                    .get("model_provider")
                    .and_then(Item::as_str)
                    .map(str::to_owned),
                managed_provider_present: custom.is_some(),
                warnings: vec![],
            }
        }
        Err(error) => ConfigInspection {
            path: path.display().to_string(),
            valid: false,
            active_provider: None,
            managed_provider_present: false,
            warnings: vec![format!("Codex 配置文件格式有误：{error}")],
        },
    }
}

fn apply_custom_fields(
    document: &mut DocumentMut,
    provider_name: &str,
    base_url: &str,
    headers: &BTreeMap<String, String>,
    model: &str,
) -> Result<(), AppError> {
    update_managed_model_provider_fields(document, MANAGED_PROVIDER_ID);
    // 设置默认模型，让 Codex 直接调用第三方模型；留空时保留 Codex 默认模型。
    if !model.trim().is_empty() {
        document["model"] = value(model.trim());
    }
    normalize_inline_model_providers(document);
    if document.get("model_providers").is_none() {
        document["model_providers"] = Item::Table(Table::new());
    }
    let providers = document["model_providers"].as_table_mut().ok_or_else(|| {
        AppError::InvalidConfig(
            "Codex 配置中的 model_providers 格式不正确，需要手动修复后再试。".into(),
        )
    })?;
    let mut custom = Table::new();
    custom["name"] = value(provider_name.trim());
    custom["wire_api"] = value("responses");
    custom["requires_openai_auth"] = value(true);
    custom["base_url"] = value(base_url.trim().trim_end_matches('/'));
    if !headers.is_empty() {
        let mut table = Table::new();
        for (name, header_value) in headers {
            table[name] = value(header_value);
        }
        custom["http_headers"] = Item::Table(table);
    } else {
        custom.remove("http_headers");
    }
    providers.insert(MANAGED_PROVIDER_ID, Item::Table(custom));
    // 指向本应用生成的模型目录，让 Codex CLI/桌面应用的模型选择器
    // 能列出自定义模型；目录文件在预览/应用时写入。
    document["model_catalog_json"] = value(format!("{}/{}", MODEL_CATALOG_DIR, MODEL_CATALOG_FILE));
    Ok(())
}

fn render_api_key_auth(api_key: &str) -> Result<Vec<u8>, AppError> {
    render_auth_object(serde_json::Map::from_iter([(
        "OPENAI_API_KEY".into(),
        serde_json::Value::String(api_key.to_owned()),
    )]))
}

fn commit_codex_files(
    config_path: &Path,
    config: &[u8],
    auth: &[u8],
    mut write: impl FnMut(&Path, &[u8]) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let auth_path = config_path.with_file_name("auth.json");
    if fs::read(config_path).is_ok_and(|current| current == config)
        && fs::read(&auth_path).is_ok_and(|current| current == auth)
    {
        return Ok(());
    }
    // A crash between files may sign Codex out, but can never pair credentials
    // with an API endpoint they were not intended for.
    write(&auth_path, b"{}\n")?;
    write(config_path, config)?;
    write(&auth_path, auth)
}

fn normalize_inline_model_providers(document: &mut DocumentMut) {
    let entries = document
        .get("model_providers")
        .and_then(Item::as_inline_table)
        .map(|inline| {
            inline
                .iter()
                .map(|(key, value)| (key.to_owned(), value.clone()))
                .collect::<Vec<_>>()
        });
    let Some(entries) = entries else { return };
    let mut providers = Table::new();
    for (key, value) in entries {
        providers.insert(&key, Item::Value(value));
    }
    document["model_providers"] = Item::Table(providers);
}

fn update_managed_model_provider_fields(document: &mut DocumentMut, provider: &str) {
    document["model_provider"] = value(provider);
    if let Some(profiles) = document.get_mut("profiles").and_then(Item::as_table_mut) {
        for (_, profile) in profiles.iter_mut() {
            update_profile_model_provider(profile, Some(provider));
        }
    }
}

fn clear_custom_provider_fields(
    document: &mut DocumentMut,
    managed_model: Option<&str>,
) -> Result<(), AppError> {
    let root = document.as_table_mut();
    root.remove("model_provider");
    // 只有当我们之前写入的第三方模型仍然生效时，才把它一并移除，
    // 避免把用户手动设置的模型误删。
    if let Some(managed) = managed_model.filter(|value| !value.trim().is_empty())
        && root
            .get("model")
            .and_then(Item::as_str)
            .is_some_and(|current| current == managed.trim())
    {
        root.remove("model");
    }
    if let Some(profiles) = root.get_mut("profiles").and_then(Item::as_table_mut) {
        for (_, profile) in profiles.iter_mut() {
            update_profile_model_provider(profile, None);
        }
    }

    let remove_provider_table = match root.get_mut("model_providers") {
        Some(Item::Table(providers)) => {
            providers.remove(MANAGED_PROVIDER_ID);
            providers.is_empty()
        }
        Some(Item::Value(Value::InlineTable(providers))) => {
            providers.remove(MANAGED_PROVIDER_ID);
            providers.is_empty()
        }
        Some(_) => {
            return Err(AppError::InvalidConfig(
                "Codex 配置中的 model_providers 格式不正确，需要手动修复后再切换账号。".into(),
            ));
        }
        None => false,
    };
    if remove_provider_table {
        root.remove("model_providers");
    }
    // 只移除本应用写入的模型目录指针，保留用户手动设置的 `model_catalog_json`。
    if root
        .get("model_catalog_json")
        .and_then(Item::as_str)
        .is_some_and(|path| path == format!("{MODEL_CATALOG_DIR}/{MODEL_CATALOG_FILE}"))
    {
        root.remove("model_catalog_json");
    }
    Ok(())
}

fn update_profile_model_provider(profile: &mut Item, provider: Option<&str>) {
    match profile {
        Item::Table(profile) => match provider {
            Some(provider) if profile.contains_key("model_provider") => {
                profile["model_provider"] = value(provider);
            }
            None => {
                profile.remove("model_provider");
            }
            Some(_) => {}
        },
        Item::Value(Value::InlineTable(profile)) => match provider {
            Some(provider) if profile.contains_key("model_provider") => {
                profile.insert("model_provider", Value::from(provider));
            }
            None => {
                profile.remove("model_provider");
            }
            Some(_) => {}
        },
        _ => {}
    }
}

fn merged_headers(provider: &ProviderProfile) -> BTreeMap<String, String> {
    provider.headers.clone()
}

fn parse_config_document(text: &str) -> Result<DocumentMut, AppError> {
    if text.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    text.parse::<DocumentMut>().map_err(|error| {
        AppError::InvalidConfig(format!(
            "Codex 配置文件格式有误，请修复 config.toml 后再试：{error}"
        ))
    })
}

fn parse_auth_object(bytes: &[u8]) -> Result<serde_json::Map<String, serde_json::Value>, AppError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(serde_json::Map::new());
    }
    serde_json::from_slice::<serde_json::Value>(bytes)
        .map_err(|error| AppError::InvalidConfig(format!("Codex 登录文件格式有误：{error}")))?
        .as_object()
        .cloned()
        .ok_or_else(|| {
            AppError::InvalidConfig("Codex 登录文件格式有误：auth.json 顶层必须是对象。".into())
        })
}

fn render_auth_object(
    auth: serde_json::Map<String, serde_json::Value>,
) -> Result<Vec<u8>, AppError> {
    let mut bytes = serde_json::to_vec_pretty(&serde_json::Value::Object(auth))
        .map_err(|error| AppError::Internal(error.to_string()))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn read_optional(path: &Path) -> Result<String, AppError> {
    reject_oversized_file(
        path,
        MAX_CODEX_CONFIG_BYTES,
        "Codex 配置文件超过 2 MB，程序已停止读取以避免占用过多内存。",
    )?;
    match fs::read_to_string(path) {
        Ok(value) if value.len() as u64 <= MAX_CODEX_CONFIG_BYTES => Ok(value),
        Ok(_) => Err(AppError::InvalidConfig(
            "Codex 配置文件超过 2 MB，程序已停止读取以避免占用过多内存。".into(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(AppError::Internal(format!(
            "无法读取 Codex 配置文件，请检查文件权限：{error}"
        ))),
    }
}

fn read_optional_bytes(path: &Path) -> Result<Vec<u8>, AppError> {
    reject_oversized_file(
        path,
        MAX_CODEX_AUTH_BYTES,
        "Codex 登录文件超过 4 MB，程序已停止读取以避免占用过多内存。",
    )?;
    match fs::read(path) {
        Ok(value) if value.len() as u64 <= MAX_CODEX_AUTH_BYTES => Ok(value),
        Ok(_) => Err(AppError::InvalidConfig(
            "Codex 登录文件超过 4 MB，程序已停止读取以避免占用过多内存。".into(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(AppError::Internal(format!(
            "无法读取 Codex 登录文件，请检查文件权限：{error}"
        ))),
    }
}

fn reject_oversized_file(path: &Path, limit: u64, message: &'static str) -> Result<(), AppError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > limit => Err(AppError::InvalidConfig(message.into())),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(AppError::Internal(format!(
            "无法检查 Codex 文件大小，请检查文件权限：{error}"
        ))),
    }
}

fn digest(value: &str) -> String {
    digest_bytes(value.as_bytes())
}

fn digest_bytes(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn describe_changes(model: &str, chat_proxy: bool) -> Vec<String> {
    let mut changes = vec![
        "保留现有的其他 Codex 设置".into(),
        "将 Codex 切换到所选第三方 API 服务".into(),
        if chat_proxy {
            "写入本机转换代理地址（Chat Completions API 自动转为 Responses API）".into()
        } else {
            "写入 Responses API 地址".into()
        },
        "更新 Codex 使用的 API Key".into(),
    ];
    if !model.trim().is_empty() {
        changes.push(format!("默认模型设为 {}", model.trim()));
    }
    changes.push(format!(
        "写入模型目录 {MODEL_CATALOG_DIR}/{MODEL_CATALOG_FILE}，供模型选择器列出官方与自定义模型"
    ));
    changes
}

fn mask_secret(secret: &str) -> String {
    let characters = secret.chars().collect::<Vec<_>>();
    if characters.is_empty() {
        return String::new();
    }
    if characters.len() <= 8 {
        return "*".repeat(characters.len());
    }
    let prefix = characters.iter().take(4).collect::<String>();
    let suffix = characters
        .iter()
        .skip(characters.len().saturating_sub(4))
        .collect::<String>();
    format!("{prefix}…{suffix}")
}

fn managed_custom_preview(
    document: &DocumentMut,
    api_key: &str,
    headers: &BTreeMap<String, String>,
) -> String {
    let mut preview = DocumentMut::new();
    for key in ["model", "model_provider", "model_catalog_json"] {
        if let Some(item) = document.get(key) {
            preview[key] = item.clone();
        }
    }
    if let Some(source) = document
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(MANAGED_PROVIDER_ID))
        .and_then(Item::as_table)
    {
        let mut custom = Table::new();
        for key in [
            "name",
            "wire_api",
            "requires_openai_auth",
            "base_url",
            "http_headers",
        ] {
            if let Some(item) = source.get(key) {
                custom.insert(key, item.clone());
            }
        }
        let mut providers = Table::new();
        providers.insert(MANAGED_PROVIDER_ID, Item::Table(custom));
        preview["model_providers"] = Item::Table(providers);
    }
    let mut rendered = preview.to_string().replace(api_key, &mask_secret(api_key));
    let mut values = headers
        .values()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort_by_key(|value| std::cmp::Reverse(value.len()));
    for secret in values {
        rendered = rendered.replace(secret, "[REDACTED]");
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_rejects_oversized_config_files() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            vec![b'x'; MAX_CODEX_CONFIG_BYTES as usize + 1],
        )
        .unwrap();

        let inspection = inspect(temp.path());

        assert!(!inspection.valid);
        assert!(
            inspection
                .warnings
                .iter()
                .any(|warning| warning.contains("超过 2 MB"))
        );
    }

    fn prepare_home(home: &Path) {
        fs::create_dir_all(home).unwrap();
    }

    fn provider() -> ProviderProfile {
        ProviderProfile {
            id: "provider".into(),
            name: "Direct Responses".into(),
            base_url: "https://responses.example.test/v1".into(),
            headers: BTreeMap::from([("x-provider".into(), "provider-secret".into())]),
            timeout_secs: 30,
            enabled: true,
            active: false,
            model: String::new(),

            model_context_windows: Default::default(),
            available_models: Default::default(),
            models_dev_meta: Default::default(),
            api_type: ProviderApiType::Responses,
            api_key: Some("sk-direct-secret-value".into()),
            has_api_key: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn custom_config_updates_managed_fields_and_preserves_unknown_toml() {
        let mut document = r#"model = "old-model"
model_provider = "old-root"
user_setting = "keep"
approval_policy = "on-request"

[mcp_servers.private]
command = "server"
env = { model_provider = "mcp-sentinel" }

[model_providers.custom]
name = "old"
wire_api = "responses"
requires_openai_auth = false
base_url = "https://old.example.test"
request_max_retries = 7
experimental_bearer_token = "old-secret"

[model_providers.other]
name = "Other"
base_url = "https://other.example.test"
wire_api = "responses"

[profiles.work]
model_provider = "old-profile"
settings = { model_provider = "old-inline", rules = [{ model_provider = "old-array" }] }
"#
        .parse::<DocumentMut>()
        .unwrap();
        apply_custom_fields(
            &mut document,
            "Direct Responses",
            "https://responses.example.test/v1",
            &BTreeMap::from([("x-tenant".into(), "tenant-1".into())]),
            "",
        )
        .unwrap();
        let text = document.to_string();
        let parsed: DocumentMut = text.parse().unwrap();
        let custom = parsed["model_providers"]["custom"].as_table().unwrap();
        assert_eq!(parsed["model"].as_str(), Some("old-model"));
        assert_eq!(parsed["user_setting"].as_str(), Some("keep"));
        assert_eq!(parsed["model_provider"].as_str(), Some("custom"));
        assert_eq!(
            parsed["model_catalog_json"].as_str(),
            Some("model-catalogs/codex-tools.json")
        );
        assert_eq!(
            parsed["profiles"]["work"]["model_provider"].as_str(),
            Some("custom")
        );
        let settings = parsed["profiles"]["work"]["settings"]
            .as_inline_table()
            .unwrap();
        assert_eq!(
            settings.get("model_provider").and_then(Value::as_str),
            Some("old-inline")
        );
        let rule = settings
            .get("rules")
            .and_then(Value::as_array)
            .and_then(|rules| rules.get(0))
            .and_then(Value::as_inline_table)
            .unwrap();
        assert_eq!(
            rule.get("model_provider").and_then(Value::as_str),
            Some("old-array")
        );
        assert_eq!(parsed["approval_policy"].as_str(), Some("on-request"));
        assert_eq!(
            parsed["mcp_servers"]["private"]["command"].as_str(),
            Some("server")
        );
        assert_eq!(
            parsed["mcp_servers"]["private"]["env"]
                .as_inline_table()
                .and_then(|env| env.get("model_provider"))
                .and_then(Value::as_str),
            Some("mcp-sentinel")
        );
        assert_eq!(
            parsed["model_providers"]["other"]["name"].as_str(),
            Some("Other")
        );
        assert_eq!(custom["name"].as_str(), Some("Direct Responses"));
        assert_eq!(
            custom["base_url"].as_str(),
            Some("https://responses.example.test/v1")
        );
        assert_eq!(custom["wire_api"].as_str(), Some("responses"));
        assert_eq!(custom["requires_openai_auth"].as_bool(), Some(true));
        assert!(custom.get("experimental_bearer_token").is_none());
        assert!(custom.get("request_max_retries").is_none());
        assert_eq!(
            custom["http_headers"]["x-tenant"].as_str(),
            Some("tenant-1")
        );
    }

    #[test]
    fn preview_redacts_direct_credentials_and_rejects_concurrent_change() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        prepare_home(&home);
        let manager = ConfigManager::default();
        fs::write(
            home.join("auth.json"),
            br#"{"auth_mode":"chatgpt","tokens":{"access_token":"secret"},"last_refresh":"yesterday","user_setting":{"keep":true}}"#,
        )
        .unwrap();
        let first = manager
            .preview_custom(&home, &provider(), provider().base_url.as_str())
            .unwrap();
        assert!(!first.rendered.contains("sk-direct-secret-value"));
        assert!(!first.rendered.contains("provider-secret"));
        assert!(!first.rendered.contains("account-secret"));
        assert_eq!(first.api_key_masked, "sk-d…alue");
        manager.apply(&first.operation_id).unwrap();
        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth.as_object().unwrap().len(), 1);
        assert_eq!(auth["OPENAI_API_KEY"], "sk-direct-secret-value");
        assert!(auth.get("user_setting").is_none());
        assert!(auth.get("auth_mode").is_none());
        assert!(auth.get("tokens").is_none());
        assert!(auth.get("last_refresh").is_none());
        let written = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(written.contains("https://responses.example.test/v1"));
        assert!(!written.contains("sk-direct-secret-value"));
        assert!(written.contains("requires_openai_auth = true"));
        let second = manager
            .preview_custom(&home, &provider(), provider().base_url.as_str())
            .unwrap();
        fs::write(home.join("config.toml"), "model = \"external\"\n").unwrap();
        assert!(matches!(
            manager.apply(&second.operation_id),
            Err(AppError::StaleOperation)
        ));
    }

    #[test]
    fn unchanged_codex_files_skip_the_commit_sequence() {
        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("config.toml");
        let auth_path = temp.path().join("auth.json");
        let config = b"model_provider = \"custom\"\n";
        let auth = b"{\"OPENAI_API_KEY\":\"secret\"}\n";
        fs::write(&config_path, config).unwrap();
        fs::write(auth_path, auth).unwrap();
        let mut writes = 0;

        commit_codex_files(&config_path, config, auth, |_, _| {
            writes += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(writes, 0);
    }

    #[test]
    fn preview_never_returns_unmanaged_toml_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        prepare_home(&home);
        fs::write(
            home.join("config.toml"),
            r#"model = "official-model"
[mcp_servers.private]
command = "server"
env = { PRIVATE_TOKEN = "must-not-enter-webview" }

[model_providers.custom]
name = "old-custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://old.example.test"
private_setting = "must-also-not-enter-webview"
"#,
        )
        .unwrap();

        let manager = ConfigManager::default();
        let preview = manager
            .preview_custom(&home, &provider(), provider().base_url.as_str())
            .unwrap();

        assert!(!preview.rendered.contains("must-not-enter-webview"));
        assert!(!preview.rendered.contains("must-also-not-enter-webview"));
        assert!(!preview.rendered.contains("mcp_servers"));
        assert!(preview.rendered.contains("[model_providers.custom]"));
        manager.apply(&preview.operation_id).unwrap();
        let written = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(written.contains("must-not-enter-webview"));
        assert!(!written.contains("must-also-not-enter-webview"));
        assert!(written.contains("mcp_servers"));
        assert!(written.contains("model_provider = \"custom\""));
    }

    #[test]
    fn custom_activation_rejects_concurrent_auth_change() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        prepare_home(&home);
        fs::write(home.join("config.toml"), "").unwrap();
        fs::write(home.join("auth.json"), b"{\"before\":true}").unwrap();
        let manager = ConfigManager::default();
        let preview = manager
            .preview_custom(&home, &provider(), provider().base_url.as_str())
            .unwrap();
        fs::write(home.join("auth.json"), b"{\"concurrent\":true}").unwrap();

        assert!(matches!(
            manager.apply(&preview.operation_id),
            Err(AppError::StaleOperation)
        ));
        assert_eq!(
            fs::read(home.join("auth.json")).unwrap(),
            b"{\"concurrent\":true}"
        );
        assert!(fs::read(home.join("config.toml")).unwrap().is_empty());
    }

    #[test]
    fn chat_proxy_activation_writes_fixed_key_and_proxy_url() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        prepare_home(&home);
        let mut provider = ProviderProfile {
            id: "chat".into(),
            name: "DeepSeek".into(),
            base_url: "https://api.deepseek.com/v1".into(),
            headers: BTreeMap::new(),
            timeout_secs: 30,
            enabled: true,
            active: false,
            model: String::new(),

            model_context_windows: Default::default(),
            available_models: Default::default(),
            models_dev_meta: Default::default(),
            api_type: ProviderApiType::Chat,
            api_key: Some("sk-real-provider-key".into()),
            has_api_key: false,
            created_at: 0,
            updated_at: 0,
        };
        let effective = format!("http://127.0.0.1:{}/v1", crate::chat_proxy::PROXY_PORT);
        let manager = ConfigManager::default();
        let preview = manager
            .preview_custom(&home, &provider, &effective)
            .unwrap();
        // 预览绝不泄露真实的服务商 Key。
        assert!(!preview.rendered.contains("sk-real-provider-key"));
        manager.apply(&preview.operation_id).unwrap();
        let config = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(config.contains(&effective));
        assert!(config.contains("wire_api = \"responses\""));
        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(
            auth["OPENAI_API_KEY"],
            crate::chat_proxy::PROXY_FIXED_API_KEY
        );

        // 直连 Responses 类型的服务仍然写入真实 Key。
        provider.api_type = ProviderApiType::Responses;
        let direct = manager
            .preview_custom(&home, &provider, "https://api.deepseek.com/v1")
            .unwrap();
        manager.apply(&direct.operation_id).unwrap();
        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "sk-real-provider-key");
    }

    #[test]
    fn custom_activation_rejects_invalid_config_and_replaces_invalid_auth() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        prepare_home(&home);
        let invalid_config = b"model = [\n";
        fs::write(home.join("config.toml"), invalid_config).unwrap();
        let manager = ConfigManager::default();

        assert!(
            manager
                .preview_custom(&home, &provider(), provider().base_url.as_str())
                .is_err()
        );
        assert_eq!(fs::read(home.join("config.toml")).unwrap(), invalid_config);

        fs::write(home.join("config.toml"), "model = \"valid\"\n").unwrap();
        let invalid_auth = b"{ invalid json";
        fs::write(home.join("auth.json"), invalid_auth).unwrap();
        let preview = manager
            .preview_custom(&home, &provider(), provider().base_url.as_str())
            .unwrap();
        manager.apply(&preview.operation_id).unwrap();
        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(home.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth.as_object().unwrap().len(), 1);
        assert_eq!(auth["OPENAI_API_KEY"], "sk-direct-secret-value");
        assert_eq!(
            fs::read_to_string(home.join("config.toml"))
                .unwrap()
                .parse::<DocumentMut>()
                .unwrap()["model_provider"]
                .as_str(),
            Some("custom")
        );
    }

    #[test]
    fn official_account_removes_custom_fields_and_rewrites_auth() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            r#"model = "custom-model"
model_provider = "custom"

[model_providers.custom]
name = "Custom"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://custom.example.test/v1"

[model_providers.other]
name = "Other"
wire_api = "responses"
base_url = "https://other.example.test/v1"

[mcp_servers.remove]
command = "server"

[profiles.work]
model_provider = "custom"
settings = { model_provider = "custom" }
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("auth.json"),
            r#"{"OPENAI_API_KEY":"old-api-key","user_setting":{"keep":true}}"#,
        )
        .unwrap();
        let credential = CodexAuthCredential {
            auth_mode: "chatgpt".into(),
            openai_api_key: None,
            tokens: crate::models::CodexAuthTokens {
                id_token: "id-token".into(),
                access_token: "access-token".into(),
                refresh_token: "refresh-token".into(),
                account_id: "account-id".into(),
            },
            last_refresh: "2026-07-14T00:00:00Z".into(),
        };

        connections_activate_official_account(temp.path(), &credential, None).unwrap();

        let config = fs::read_to_string(temp.path().join("config.toml")).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert_eq!(parsed["model"].as_str(), Some("custom-model"));
        assert!(parsed.get("model_provider").is_none());
        assert!(parsed["profiles"]["work"].get("model_provider").is_none());
        assert!(
            parsed["profiles"]["work"]["settings"]
                .as_inline_table()
                .and_then(|settings| settings.get("model_provider"))
                .is_some_and(|provider| provider.as_str() == Some("custom"))
        );
        assert!(parsed["model_providers"].get("custom").is_none());
        assert_eq!(
            parsed["model_providers"]["other"]["name"].as_str(),
            Some("Other")
        );
        assert!(config.contains("[mcp_servers.remove]"));
        assert!(config.contains("command = \"server\""));
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(temp.path().join("auth.json")).unwrap()).unwrap();
        assert_eq!(written.as_object().unwrap().len(), 4);
        assert_eq!(written["auth_mode"], "chatgpt");
        assert!(written["OPENAI_API_KEY"].is_null());
        assert!(written.get("user_setting").is_none());
        assert_eq!(
            read_official_account(temp.path())
                .unwrap()
                .unwrap()
                .tokens
                .account_id,
            "account-id"
        );
    }

    #[test]
    fn official_cleanup_preserves_other_inline_providers() {
        let mut document = r#"model_provider = "custom"
model_providers = { custom = { base_url = "https://custom.example.test/v1" }, other = { base_url = "https://other.example.test/v1" } }
"#
        .parse::<DocumentMut>()
        .unwrap();

        clear_custom_provider_fields(&mut document, None).unwrap();

        let providers = document["model_providers"].as_inline_table().unwrap();
        assert!(providers.get("custom").is_none());
        assert_eq!(
            providers
                .get("other")
                .and_then(Value::as_inline_table)
                .and_then(|other| other.get("base_url"))
                .and_then(Value::as_str),
            Some("https://other.example.test/v1")
        );
    }

    #[test]
    fn official_cleanup_preserves_model_and_unknown_settings() {
        let mut document = r#"model = "custom-model"
model_provider = "custom"
custom_note = "keep"

[model_providers.custom]
base_url = "https://custom.example.test/v1"
"#
        .parse::<DocumentMut>()
        .unwrap();

        clear_custom_provider_fields(&mut document, None).unwrap();

        assert_eq!(document["model"].as_str(), Some("custom-model"));
        assert_eq!(document["custom_note"].as_str(), Some("keep"));
        assert!(document.get("model_provider").is_none());
        assert!(document.get("model_providers").is_none());
    }

    #[test]
    fn official_cleanup_rejects_invalid_provider_container() {
        let mut document = r#"model_provider = "custom"
model_providers = "invalid"
"#
        .parse::<DocumentMut>()
        .unwrap();

        let error = clear_custom_provider_fields(&mut document, None).unwrap_err();

        assert!(error.to_string().contains("model_providers"));
        assert_eq!(document["model_providers"].as_str(), Some("invalid"));
    }

    #[test]
    fn custom_activation_accepts_inline_model_providers() {
        let mut document =
            r#"model_providers = { other = { base_url = "https://other.example.test/v1" } }
"#
            .parse::<DocumentMut>()
            .unwrap();

        apply_custom_fields(
            &mut document,
            "Custom",
            "https://custom.example.test/v1",
            &BTreeMap::new(),
            "",
        )
        .unwrap();

        assert_eq!(
            document["model_providers"]["other"]["base_url"].as_str(),
            Some("https://other.example.test/v1")
        );
        assert_eq!(
            document["model_providers"]["custom"]["base_url"].as_str(),
            Some("https://custom.example.test/v1")
        );
    }

    #[test]
    fn custom_activation_writes_and_preserves_the_default_model() {
        let mut document = r#"model = "user-kept"
model_providers = { other = { base_url = "https://other.example.test/v1" } }
"#
        .parse::<DocumentMut>()
        .unwrap();

        apply_custom_fields(
            &mut document,
            "Custom",
            "https://custom.example.test/v1",
            &BTreeMap::new(),
            "gpt-5.6-luna",
        )
        .unwrap();
        assert_eq!(document["model"].as_str(), Some("gpt-5.6-luna"));

        // 留空时不覆盖用户已设置的模型。
        apply_custom_fields(
            &mut document,
            "Custom",
            "https://custom.example.test/v1",
            &BTreeMap::new(),
            "",
        )
        .unwrap();
        assert_eq!(document["model"].as_str(), Some("gpt-5.6-luna"));
    }

    #[test]
    fn official_switch_removes_only_the_managed_model() {
        let mut document = r#"model = "gpt-5.6-luna"
model_provider = "custom"

[model_providers.custom]
base_url = "https://custom.example.test/v1"
"#
        .parse::<DocumentMut>()
        .unwrap();

        clear_custom_provider_fields(&mut document, Some("gpt-5.6-luna")).unwrap();
        assert!(document.get("model").is_none());
        assert!(document.get("model_provider").is_none());

        // 模型与受管模型不一致时保留，避免误删用户设置。
        let mut manual = r#"model = "my-personal-model"
model_provider = "custom"

[model_providers.custom]
base_url = "https://custom.example.test/v1"
"#
        .parse::<DocumentMut>()
        .unwrap();
        clear_custom_provider_fields(&mut manual, Some("gpt-5.6-luna")).unwrap();
        assert_eq!(manual["model"].as_str(), Some("my-personal-model"));
    }

    #[test]
    fn official_activation_clears_the_managed_model_from_disk() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            r#"model = "gpt-5.6-luna"
model_provider = "custom"

[model_providers.custom]
base_url = "https://custom.example.test/v1"
"#,
        )
        .unwrap();
        fs::write(temp.path().join("auth.json"), b"{}").unwrap();
        let credential = CodexAuthCredential {
            auth_mode: "chatgpt".into(),
            openai_api_key: None,
            tokens: crate::models::CodexAuthTokens {
                id_token: "id-token".into(),
                access_token: "access-token".into(),
                refresh_token: "refresh-token".into(),
                account_id: "account-id".into(),
            },
            last_refresh: "2026-07-14T00:00:00Z".into(),
        };

        connections_activate_official_account(temp.path(), &credential, Some("gpt-5.6-luna"))
            .unwrap();

        let parsed = fs::read_to_string(temp.path().join("config.toml"))
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert!(parsed.get("model").is_none());
        assert!(parsed.get("model_provider").is_none());
    }

    #[test]
    fn credential_commit_neutralizes_auth_before_config_and_final_auth() {
        let config_path = PathBuf::from("/codex/config.toml");
        let mut writes = vec![];

        commit_codex_files(
            &config_path,
            b"model_provider = \"custom\"\n",
            br#"{"OPENAI_API_KEY":"secret"}"#,
            |path, bytes| {
                writes.push((path.to_path_buf(), bytes.to_vec()));
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            writes[0],
            (PathBuf::from("/codex/auth.json"), b"{}\n".to_vec())
        );
        assert_eq!(writes[1].0, config_path);
        assert_eq!(writes[2].0, PathBuf::from("/codex/auth.json"));
        assert!(writes[2].1.windows(6).any(|window| window == b"secret"));
    }

    #[test]
    fn failed_config_commit_leaves_auth_neutral() {
        let config_path = PathBuf::from("/codex/config.toml");
        let mut auth = b"official-token".to_vec();
        let result = commit_codex_files(
            &config_path,
            b"model_provider = \"custom\"\n",
            br#"{"OPENAI_API_KEY":"secret"}"#,
            |path, bytes| {
                if path == config_path {
                    return Err(AppError::Internal("simulated config failure".into()));
                }
                auth = bytes.to_vec();
                Ok(())
            },
        );

        assert!(result.is_err());
        assert_eq!(auth, b"{}\n");
    }

    #[test]
    fn personal_access_token_account_uses_codex_supported_auth_shape() {
        let temp = tempfile::tempdir().unwrap();
        prepare_home(temp.path());
        let credential = CodexAuthCredential {
            auth_mode: "chatgpt".into(),
            openai_api_key: None,
            tokens: crate::models::CodexAuthTokens {
                id_token: String::new(),
                access_token: "at-proxy-secret".into(),
                refresh_token: String::new(),
                account_id: "proxy-local-id".into(),
            },
            last_refresh: "2026-07-31T00:00:00Z".into(),
        };

        connections_activate_official_account(temp.path(), &credential, None).unwrap();

        let auth: serde_json::Value =
            serde_json::from_slice(&fs::read(temp.path().join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], serde_json::Value::Null);
        assert_eq!(auth["personal_access_token"], "at-proxy-secret");
        assert!(auth.get("tokens").is_none());
        assert_eq!(
            read_official_account(temp.path())
                .unwrap()
                .unwrap()
                .tokens
                .access_token,
            "at-proxy-secret"
        );
    }

    #[test]
    fn short_secrets_are_fully_masked() {
        assert_eq!(mask_secret("abcde"), "*****");
        assert_eq!(mask_secret("12345678"), "********");
        assert_eq!(mask_secret("123456789"), "1234…6789");
    }
}
