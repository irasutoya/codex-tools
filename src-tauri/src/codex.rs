use crate::{
    model_catalog,
    models::{
        AppError, CodexAuthCredential, ConfigInspection, ConfigPatchPreview, ProviderProfile,
        RouteSettings,
    },
    storage::atomic_write,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};
use toml_edit::{DocumentMut, Item, Table, value};

pub const MANAGED_PROVIDER_ID: &str = "custom";
pub const MODEL_CATALOG_FILENAME: &str = "model_catalog.json";
const BASE_URL: &str = "http://127.0.0.1:16384/v1";

#[derive(Clone)]
struct PendingPatch {
    base_hash: String,
    auth_base_hash: String,
    target: PathBuf,
    rendered: String,
    created_at: Instant,
}

struct PatchDraft<'a> {
    target: PathBuf,
    original: &'a str,
    rendered: String,
    public_preview: String,
    changes: Vec<String>,
    token: &'a str,
}

#[derive(Default)]
pub struct ConfigManager {
    pending: Mutex<HashMap<String, PendingPatch>>,
}

impl ConfigManager {
    pub fn preview_custom(
        &self,
        codex_home: &Path,
        data_root: &Path,
        model: &str,
        route_settings: &RouteSettings,
        regenerate_token: bool,
    ) -> Result<ConfigPatchPreview, AppError> {
        let path = codex_home.join("config.toml");
        let original = read_optional(&path)?;
        let original_document = parse_document(&original).ok();
        let catalog = absolute_path(&data_root.join(MODEL_CATALOG_FILENAME))?;
        let existing_token = original_document
            .as_ref()
            .and_then(|document| document.get("model_providers"))
            .and_then(Item::as_table)
            .and_then(|providers| providers.get(MANAGED_PROVIDER_ID))
            .and_then(Item::as_table)
            .and_then(|provider| provider.get("experimental_bearer_token"))
            .and_then(Item::as_str)
            .filter(|token| token.starts_with("ct_") && token.len() >= 40)
            .map(str::to_owned);
        let token = if regenerate_token {
            compatibility_token()
        } else {
            existing_token.unwrap_or_else(compatibility_token)
        };
        let mut document = DocumentMut::new();
        apply_custom_fields(&mut document, model, &catalog, &token, route_settings)?;
        let rendered = document.to_string();
        let public_preview = managed_custom_preview(&document, &token);
        let changes = describe_changes();
        self.remember(PatchDraft {
            target: path,
            original: &original,
            rendered,
            public_preview,
            changes,
            token: &token,
        })
    }

    pub fn apply(&self, operation_id: &str) -> Result<(), AppError> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| AppError::Internal("配置预览锁已损坏".into()))?
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
        let _: DocumentMut = pending
            .rendered
            .parse()
            .map_err(|error| AppError::InvalidConfig(format!("生成的 TOML 无效：{error}")))?;
        if let Some(parent) = pending.target.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(&pending.target.with_file_name("auth.json"), b"{}\n")?;
        atomic_write(&pending.target, pending.rendered.as_bytes())?;
        Ok(())
    }

    fn remember(&self, draft: PatchDraft<'_>) -> Result<ConfigPatchPreview, AppError> {
        let operation_id = uuid::Uuid::new_v4().to_string();
        let base_hash = digest(draft.original);
        let auth_base_hash = digest_bytes(&read_optional_bytes(
            &draft.target.with_file_name("auth.json"),
        )?);
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| AppError::Internal("配置预览锁已损坏".into()))?;
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
                target: draft.target.clone(),
                rendered: draft.rendered.clone(),
                created_at: Instant::now(),
            },
        );
        Ok(ConfigPatchPreview {
            operation_id,
            target_path: draft.target.display().to_string(),
            base_hash,
            rendered: draft.public_preview,
            changes: draft.changes,
            compatibility_token_masked: mask_token(draft.token),
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

pub fn activate_official_account(
    codex_home: &Path,
    credential: &CodexAuthCredential,
) -> Result<(), AppError> {
    validate_official_credential(credential)?;
    fs::create_dir_all(codex_home)?;
    // Clear the custom route first. A crash can leave Codex signed out, but it
    // can never leave both custom provider configuration and official auth active.
    atomic_write(&codex_home.join("config.toml"), b"")?;
    let bytes = serde_json::to_vec_pretty(credential)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    atomic_write(&codex_home.join("auth.json"), &bytes)?;
    Ok(())
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
    let credential = serde_json::from_slice::<CodexAuthCredential>(&bytes)
        .map_err(|_| AppError::InvalidConfig("auth.json 不是可识别的 Codex 登录凭据".into()))?;
    validate_official_credential(&credential)?;
    Ok(Some(credential))
}

fn validate_official_credential(credential: &CodexAuthCredential) -> Result<(), AppError> {
    if credential.auth_mode != "chatgpt"
        || credential.tokens.id_token.trim().is_empty()
        || credential.tokens.access_token.trim().is_empty()
        || credential.tokens.refresh_token.trim().is_empty()
        || credential.tokens.account_id.trim().is_empty()
    {
        return Err(AppError::InvalidConfig(
            "OpenAI Account 登录凭据不完整".into(),
        ));
    }
    Ok(())
}

pub fn regenerate_model_catalog(
    provider: &ProviderProfile,
    codex_home: &Path,
    data_root: &Path,
) -> Result<String, AppError> {
    let catalog = model_catalog::build(provider, codex_home)?;
    let bytes = serde_json::to_vec_pretty(&catalog)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let path = data_root.join(MODEL_CATALOG_FILENAME);
    atomic_write(&path, &bytes)?;
    Ok(path.display().to_string())
}

pub fn inspect(codex_home: &Path, data_root: &Path) -> ConfigInspection {
    let path = codex_home.join("config.toml");
    let expected_catalog = data_root.join(MODEL_CATALOG_FILENAME);
    let text = fs::read_to_string(&path).unwrap_or_default();
    match text.parse::<DocumentMut>() {
        Ok(document) => {
            let custom = document
                .get("model_providers")
                .and_then(Item::as_table)
                .and_then(|providers| providers.get(MANAGED_PROVIDER_ID))
                .and_then(Item::as_table);
            let mut warnings = vec![];
            if !expected_catalog.exists() {
                warnings.push("model_catalog.json 尚未生成".into());
            }
            ConfigInspection {
                path: path.display().to_string(),
                valid: true,
                active_provider: document
                    .get("model_provider")
                    .and_then(Item::as_str)
                    .map(str::to_owned),
                managed_provider_present: custom.is_some(),
                model_catalog_path: expected_catalog.display().to_string(),
                warnings,
            }
        }
        Err(error) => ConfigInspection {
            path: path.display().to_string(),
            valid: false,
            active_provider: None,
            managed_provider_present: false,
            model_catalog_path: expected_catalog.display().to_string(),
            warnings: vec![format!("config.toml 无效：{error}")],
        },
    }
}

fn apply_custom_fields(
    document: &mut DocumentMut,
    model: &str,
    catalog: &str,
    token: &str,
    route_settings: &RouteSettings,
) -> Result<(), AppError> {
    if model.trim().is_empty() {
        return Err(AppError::InvalidConfig("请先选择模型".into()));
    }
    document["model"] = value(model.trim());
    document["model_provider"] = value(MANAGED_PROVIDER_ID);
    document["model_catalog_json"] = value(catalog);
    if document.get("model_providers").is_none() {
        document["model_providers"] = Item::Table(Table::new());
    }
    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or_else(|| AppError::InvalidConfig("model_providers 必须是 TOML table".into()))?;
    if providers.get(MANAGED_PROVIDER_ID).is_none() {
        providers.insert(MANAGED_PROVIDER_ID, Item::Table(Table::new()));
    }
    let custom = providers
        .get_mut(MANAGED_PROVIDER_ID)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            AppError::InvalidConfig("model_providers.custom 必须是 TOML table".into())
        })?;
    custom["name"] = value("Custom");
    custom["base_url"] = value(BASE_URL);
    custom["wire_api"] = value("responses");
    custom["experimental_bearer_token"] = value(token);
    custom["request_max_retries"] = value(i64::from(route_settings.request_max_retries));
    custom["stream_max_retries"] = value(i64::from(route_settings.stream_max_retries));
    custom["stream_idle_timeout_ms"] = value(300_000);
    Ok(())
}

fn compatibility_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("ct_{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn parse_document(text: &str) -> Result<DocumentMut, AppError> {
    if text.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        text.parse::<DocumentMut>()
            .map_err(|error| AppError::InvalidConfig(format!("config.toml 无效：{error}")))
    }
}

fn read_optional(path: &Path) -> Result<String, AppError> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

fn read_optional_bytes(path: &Path) -> Result<Vec<u8>, AppError> {
    match fs::read(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error.into()),
    }
}

fn digest(value: &str) -> String {
    digest_bytes(value.as_bytes())
}

fn digest_bytes(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

fn absolute_path(path: &Path) -> Result<String, AppError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(path.display().to_string())
}

fn describe_changes() -> Vec<String> {
    vec![
        "清空 auth.json 中的官方账号".into(),
        "替换整个 config.toml（MCP、Skills、Hooks、沙箱和未知字段将删除）".into(),
        "model_provider → custom".into(),
        "更新 model_providers.custom".into(),
        "更新 model_catalog_json".into(),
    ]
}

fn mask_token(token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    format!(
        "{}…{}",
        &token[..token.len().min(7)],
        &token[token.len().saturating_sub(4)..]
    )
}

fn managed_custom_preview(document: &DocumentMut, token: &str) -> String {
    let mut preview = DocumentMut::new();
    for key in ["model", "model_provider", "model_catalog_json"] {
        if let Some(item) = document.get(key) {
            preview[key] = item.clone();
        }
    }
    if let Some(custom) = document
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(MANAGED_PROVIDER_ID))
    {
        let mut providers = Table::new();
        providers.insert(MANAGED_PROVIDER_ID, custom.clone());
        preview["model_providers"] = Item::Table(providers);
    }
    preview.to_string().replace(token, &mask_token(token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_config_writes_only_required_values() {
        let mut document = DocumentMut::new();
        let settings = RouteSettings {
            request_max_retries: 7,
            stream_max_retries: 6,
            ..RouteSettings::default()
        };
        apply_custom_fields(
            &mut document,
            "gpt-test",
            "C:/data/model_catalog.json",
            "ct_test_token_value_abcdefghijklmnopqrstuvwxyz",
            &settings,
        )
        .unwrap();
        let text = document.to_string();
        let parsed: DocumentMut = text.parse().unwrap();
        let custom = parsed["model_providers"]["custom"].as_table().unwrap();
        assert_eq!(
            parsed
                .as_table()
                .iter()
                .map(|(key, _)| key)
                .collect::<Vec<_>>(),
            vec![
                "model",
                "model_provider",
                "model_catalog_json",
                "model_providers"
            ]
        );
        assert_eq!(
            custom.iter().map(|(key, _)| key).collect::<Vec<_>>(),
            vec![
                "name",
                "base_url",
                "wire_api",
                "experimental_bearer_token",
                "request_max_retries",
                "stream_max_retries",
                "stream_idle_timeout_ms"
            ]
        );
        assert_eq!(parsed["model"].as_str(), Some("gpt-test"));
        assert_eq!(parsed["model_provider"].as_str(), Some("custom"));
        assert_eq!(custom["name"].as_str(), Some("Custom"));
        assert_eq!(custom["base_url"].as_str(), Some(BASE_URL));
        assert_eq!(custom["wire_api"].as_str(), Some("responses"));
        assert_eq!(custom["request_max_retries"].as_integer(), Some(7));
        assert_eq!(custom["stream_max_retries"].as_integer(), Some(6));
        assert_eq!(custom["stream_idle_timeout_ms"].as_integer(), Some(300_000));
        assert!(custom.get("supports_websockets").is_none());
    }

    #[test]
    fn token_is_32_random_bytes_in_url_safe_form() {
        let token = compatibility_token();
        assert!(token.starts_with("ct_"));
        assert_eq!(URL_SAFE_NO_PAD.decode(&token[3..]).unwrap().len(), 32);
    }

    #[test]
    fn preview_reuses_token_and_rejects_concurrent_change() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let data = temp.path().join("data");
        fs::create_dir_all(&home).unwrap();
        let manager = ConfigManager::default();
        fs::write(
            home.join("auth.json"),
            br#"{"auth_mode":"chatgpt","tokens":{"access_token":"secret"}}"#,
        )
        .unwrap();
        let settings = RouteSettings::default();
        let first = manager
            .preview_custom(&home, &data, "model", &settings, false)
            .unwrap();
        manager.apply(&first.operation_id).unwrap();
        assert_eq!(fs::read(home.join("auth.json")).unwrap(), b"{}\n");
        let second = manager
            .preview_custom(&home, &data, "model", &settings, false)
            .unwrap();
        assert_eq!(
            first.compatibility_token_masked,
            second.compatibility_token_masked
        );
        fs::write(home.join("config.toml"), "model = \"external\"\n").unwrap();
        assert!(matches!(
            manager.apply(&second.operation_id),
            Err(AppError::StaleOperation)
        ));
    }

    #[test]
    fn preview_never_returns_unmanaged_toml_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let data = temp.path().join("data");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join("config.toml"),
            r#"model = "official-model"
[mcp_servers.private]
command = "server"
env = { PRIVATE_TOKEN = "must-not-enter-webview" }
"#,
        )
        .unwrap();

        let manager = ConfigManager::default();
        let preview = manager
            .preview_custom(
                &home,
                &data,
                "custom-model",
                &RouteSettings::default(),
                false,
            )
            .unwrap();

        assert!(!preview.rendered.contains("must-not-enter-webview"));
        assert!(!preview.rendered.contains("mcp_servers"));
        assert!(preview.rendered.contains("[model_providers.custom]"));
        manager.apply(&preview.operation_id).unwrap();
        let written = fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(!written.contains("must-not-enter-webview"));
        assert!(!written.contains("mcp_servers"));
        assert!(written.contains("model_provider = \"custom\""));
    }

    #[test]
    fn custom_activation_rejects_concurrent_auth_change() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("config.toml"), "").unwrap();
        fs::write(home.join("auth.json"), b"{\"before\":true}").unwrap();
        let manager = ConfigManager::default();
        let preview = manager
            .preview_custom(
                &home,
                temp.path(),
                "model",
                &RouteSettings::default(),
                false,
            )
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
    fn official_account_clears_config_and_writes_codex_auth_shape() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("config.toml"),
            "model_provider = \"custom\"\n[mcp_servers.remove]\ncommand = \"server\"\n",
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

        activate_official_account(temp.path(), &credential).unwrap();

        assert!(
            fs::read(temp.path().join("config.toml"))
                .unwrap()
                .is_empty()
        );
        let written: serde_json::Value =
            serde_json::from_slice(&fs::read(temp.path().join("auth.json")).unwrap()).unwrap();
        assert_eq!(written.as_object().unwrap().len(), 4);
        assert_eq!(written["auth_mode"], "chatgpt");
        assert!(written["OPENAI_API_KEY"].is_null());
        assert_eq!(
            read_official_account(temp.path())
                .unwrap()
                .unwrap()
                .tokens
                .account_id,
            "account-id"
        );
    }
}
