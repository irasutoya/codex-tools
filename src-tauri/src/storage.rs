use crate::{json_store::JsonStore, models::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, RwLock, RwLockReadGuard},
};

const MAX_SAVED_PROVIDERS: usize = 500;
const MAX_SAVED_OPENAI_ACCOUNTS: usize = 500;
const MAX_EMAIL_CHARS: usize = 320;
#[cfg(test)]
const MAX_APP_DATA_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AppFile {
    #[serde(default)]
    codex: CodexPreferences,
    #[serde(default)]
    active: ActiveState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ConnectionsFile {
    #[serde(default)]
    providers: Vec<ProviderProfile>,
    #[serde(default)]
    official_accounts: Vec<StoredOfficialAccount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CredentialsFile {
    #[serde(default)]
    provider_api_keys: BTreeMap<String, String>,
    #[serde(default)]
    provider_headers: BTreeMap<String, BTreeMap<String, String>>,
}

pub struct Store {
    root: PathBuf,
    path: PathBuf,
    connections_path: PathBuf,
    credentials_path: PathBuf,
    state: RwLock<AppConfig>,
    persist_mutex: Mutex<()>,
}

impl Store {
    pub fn new() -> anyhow::Result<Self> {
        let root = data_root();
        Self::open(root)
    }

    pub fn open(root: PathBuf) -> anyhow::Result<Self> {
        fs::create_dir_all(&root)?;
        secure_directory(&root)?;
        let path = root.join("app.json");
        let connections_path = root.join("connections.json");
        let credentials_path = root.join("credentials.json");
        let app_file = JsonStore::read_or_default(&path, AppFile::default)?;
        let connections = JsonStore::read_or_default(&connections_path, ConnectionsFile::default)?;
        let credentials = JsonStore::read_or_default(&credentials_path, CredentialsFile::default)?;
        let mut providers = connections.providers;
        for provider in &mut providers {
            if let Some(key) = credentials.provider_api_keys.get(&provider.id) {
                provider.api_key = Some(key.clone());
                provider.has_api_key = true;
            }
            if let Some(headers) = credentials.provider_headers.get(&provider.id) {
                provider.headers = headers.clone();
            }
        }
        let state = AppConfig {
            codex: app_file.codex,
            active: app_file.active,
            providers,
            official_accounts: connections.official_accounts,
        };
        if !path.exists() || !connections_path.exists() || !credentials_path.exists() {
            persist_files(&path, &connections_path, &credentials_path, &state)?;
        }
        for name in [
            "credentials.json",
            "pricing.json",
            "usage.json",
            "sessions.json",
            "cache.json",
        ] {
            JsonStore::ensure_object(&root.join(name))?;
        }
        Ok(Self {
            root,
            path,
            connections_path,
            credentials_path,
            state: RwLock::new(state),
            persist_mutex: Mutex::new(()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_state(&self) -> Result<RwLockReadGuard<'_, AppConfig>, AppError> {
        self.state
            .read()
            .map_err(|_| AppError::Internal("暂时无法读取应用数据，请重启应用后再试。".into()))
    }

    pub(crate) fn read<T>(&self, project: impl FnOnce(&AppConfig) -> T) -> Result<T, AppError> {
        let state = self.read_state()?;
        Ok(project(&state))
    }

    #[cfg(test)]
    pub fn snapshot(&self) -> Result<AppConfig, AppError> {
        self.read(Clone::clone)
    }

    pub fn codex_home_setting(&self) -> Result<String, AppError> {
        Ok(self.read_state()?.codex.home.clone())
    }

    pub fn codex_app_setting(&self) -> Result<Option<String>, AppError> {
        Ok(self.read_state()?.codex.app_path.clone())
    }

    pub fn settings_save_codex_app_path(&self, path: Option<String>) -> Result<(), AppError> {
        self.update(|state| {
            state.codex.app_path = path
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty());
            Ok(())
        })
    }

    pub fn last_debug_port(&self) -> Result<Option<u16>, AppError> {
        Ok(self.read_state()?.codex.last_debug_port)
    }

    pub fn save_last_debug_port(&self, port: u16) -> Result<(), AppError> {
        self.update(|state| {
            state.codex.last_debug_port = Some(port);
            Ok(())
        })
    }

    pub fn last_managed_model(&self) -> Result<Option<String>, AppError> {
        Ok(self.read_state()?.codex.last_managed_model.clone())
    }

    pub fn save_last_managed_model(&self, model: Option<String>) -> Result<(), AppError> {
        self.update(|state| {
            state.codex.last_managed_model = model;
            Ok(())
        })
    }

    pub fn is_active_provider(&self, id: &str) -> Result<bool, AppError> {
        let state = self.read_state()?;
        Ok(matches!(state.active.kind, ActiveKind::Provider)
            && state.active.provider_id.as_deref() == Some(id))
    }

    pub fn update<T>(
        &self,
        mutate: impl FnOnce(&mut AppConfig) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut guard = self
            .state
            .write()
            .map_err(|_| AppError::Internal("暂时无法保存应用数据，请重启应用后再试。".into()))?;
        let mut draft = guard.clone();
        let result = mutate(&mut draft)?;
        *guard = draft;
        drop(guard);
        // 落盘在状态锁之外进行，读操作不会被 fsync 阻塞。拿到持久化锁后
        // 重新读取最新状态，避免并发更新时较旧快照最后写入、覆盖新数据。
        let _persisted = self
            .persist_mutex
            .lock()
            .map_err(|_| AppError::Internal("暂时无法保存应用数据，请重启应用后再试。".into()))?;
        let persisted = self.read_state()?.clone();
        persist_files(
            &self.path,
            &self.connections_path,
            &self.credentials_path,
            &persisted,
        )
        .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok(result)
    }

    pub fn provider(&self, id: &str) -> Result<ProviderProfile, AppError> {
        let state = self.read_state()?;
        let mut profile = state
            .providers
            .iter()
            .find(|provider| provider.id == id)
            .cloned()
            .ok_or_else(|| AppError::InvalidConfig("第三方 API 服务不存在，请刷新页面。".into()))?;
        mark_active_profile(&state, &mut profile);
        Ok(profile)
    }

    pub fn connections_save_provider(
        &self,
        mut provider: ProviderProfile,
    ) -> Result<ProviderProfile, AppError> {
        if provider.id.trim().is_empty() {
            provider.id = uuid::Uuid::new_v4().to_string();
        }
        let existing = {
            let state = self.read_state()?;
            state
                .providers
                .iter()
                .find(|value| value.id == provider.id)
                .cloned()
        };
        let is_new = existing.is_none();
        if let Some(existing) = existing.as_ref() {
            preserve_redacted_headers(&mut provider.headers, &existing.headers);
            // 前端返回的是脱敏后的 api_key（None），保留已保存的 Key。
            if provider
                .api_key
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                provider.api_key = existing.api_key.clone();
            }
            // 前端可能不回传模型上下文窗口，保留已保存的数据。
            if provider.model_context_windows.is_empty() {
                provider.model_context_windows = existing.model_context_windows.clone();
            }
            // 前端不回传可用模型列表，保留已保存的数据（保存时静默抓取）。
            if provider.available_models.is_empty() {
                provider.available_models = existing.available_models.clone();
            }
            // 前端不回传 models.dev 元数据，保留已保存的数据。
            if provider.models_dev_meta.is_empty() {
                provider.models_dev_meta = existing.models_dev_meta.clone();
            }
        }
        provider.normalize_and_validate()?;
        let now = chrono::Utc::now().timestamp();
        if provider.created_at == 0 {
            provider.created_at = existing.map_or(now, |value| value.created_at);
        }
        provider.updated_at = now;
        provider.active = false;
        provider.has_api_key = provider.api_key.is_some();
        let saved = provider.clone();
        self.update(|state| {
            if matches!(state.active.kind, ActiveKind::Provider)
                && state.active.provider_id.as_deref() == Some(provider.id.as_str())
                && !provider.enabled
            {
                return Err(AppError::InvalidConfig(
                    "正在使用此服务，切换到其他账号或服务后才能停用。".into(),
                ));
            }
            if let Some(existing) = state
                .providers
                .iter_mut()
                .find(|value| value.id == provider.id)
            {
                *existing = provider.clone();
            } else {
                if is_new && state.providers.len() >= MAX_SAVED_PROVIDERS {
                    return Err(AppError::InvalidConfig(
                        "最多可保存 500 个第三方 API 服务，请先删除不再使用的服务。".into(),
                    ));
                }
                state.providers.push(provider.clone());
            }
            Ok(())
        })?;
        Ok(saved)
    }

    pub fn connections_delete_provider(&self, id: &str) -> Result<(), AppError> {
        self.update(|state| {
            if state.active.provider_id.as_deref() == Some(id) {
                return Err(AppError::InvalidConfig(
                    "正在使用此服务，切换后才能删除。".into(),
                ));
            }
            let before = state.providers.len();
            state.providers.retain(|value| value.id != id);
            if before == state.providers.len() {
                return Err(AppError::InvalidConfig(
                    "第三方 API 服务不存在，可能已被删除。".into(),
                ));
            }
            Ok(())
        })
    }

    /// 保存从服务 `/models` 接口读取到的模型上下文窗口，供模型目录使用。
    /// 保存服务 `/models` 接口返回的可用模型、上下文窗口，以及 models.dev
    /// 精确匹配的模型元数据，供模型目录使用。
    pub fn update_provider_models(
        &self,
        id: &str,
        models: Vec<String>,
        windows: BTreeMap<String, u64>,
        meta: BTreeMap<String, ProviderModelsDevMeta>,
    ) -> Result<(), AppError> {
        self.update(|state| {
            let provider = state
                .providers
                .iter_mut()
                .find(|value| value.id == id)
                .ok_or_else(|| {
                    AppError::InvalidConfig("第三方 API 服务不存在，请刷新页面。".into())
                })?;
            provider.available_models = models;
            provider.model_context_windows = windows;
            provider.models_dev_meta = meta;
            Ok(())
        })
    }

    pub fn provider_overview(&self) -> Result<ProviderOverview, AppError> {
        let state = self.read_state()?;
        let providers = state
            .providers
            .iter()
            .cloned()
            .map(|mut profile| {
                mark_active_profile(&state, &mut profile);
                profile.redacted()
            })
            .collect();
        let official_accounts = state
            .official_accounts
            .iter()
            .map(|account| {
                let active = matches!(state.active.kind, ActiveKind::Official)
                    && state.active.account_id.as_deref() == Some(account.id.as_str());
                account.view(active)
            })
            .collect();
        Ok(ProviderOverview {
            providers,
            official_accounts,
        })
    }

    pub fn activate(&self, provider_id: &str) -> Result<(), AppError> {
        self.update(|state| {
            let provider = state
                .providers
                .iter_mut()
                .find(|value| value.id == provider_id)
                .ok_or_else(|| {
                    AppError::InvalidConfig("第三方 API 服务不存在，请刷新页面。".into())
                })?;
            if !provider.enabled {
                return Err(AppError::InvalidConfig(
                    "此服务已停用，请先编辑并启用它。".into(),
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
            state.active = ActiveState {
                kind: ActiveKind::Provider,
                provider_id: Some(provider_id.to_owned()),
                account_id: None,
            };
            Ok(())
        })
    }

    pub fn official_account_view(&self, id: &str) -> Result<OfficialAccountView, AppError> {
        let state = self.read_state()?;
        let account = state
            .official_accounts
            .iter()
            .find(|account| account.id == id)
            .ok_or_else(|| AppError::InvalidConfig("OpenAI 账号不存在，可能已被删除。".into()))?;
        let active = matches!(state.active.kind, ActiveKind::Official)
            && state.active.account_id.as_deref() == Some(account.id.as_str());
        Ok(account.view(active))
    }

    /// Returns the sensitive stored record for backend-only auth operations.
    /// Tauri commands must return `official_account_view` instead.
    pub fn official_account(&self, id: &str) -> Result<StoredOfficialAccount, AppError> {
        self.read_state()?
            .official_accounts
            .iter()
            .find(|account| account.id == id)
            .cloned()
            .ok_or_else(|| AppError::InvalidConfig("OpenAI 账号不存在，可能已被删除。".into()))
    }

    pub fn save_official_account(
        &self,
        account: &StoredOfficialAccount,
    ) -> Result<StoredOfficialAccount, AppError> {
        let mut incoming = account.clone();
        normalize_official_account(&mut incoming)?;
        let now = chrono::Utc::now().timestamp();

        self.update(|state| {
            if let Some(existing_index) = state
                .official_accounts
                .iter()
                .position(|saved| saved.account_id == incoming.account_id)
            {
                let existing = &state.official_accounts[existing_index];
                incoming.id = existing.id.clone();
                incoming.created_at = existing.created_at;
                incoming.updated_at = now;
                state.official_accounts[existing_index] = incoming.clone();
                let mut kept_match = false;
                state.official_accounts.retain(|saved| {
                    if saved.account_id != incoming.account_id {
                        true
                    } else if kept_match {
                        false
                    } else {
                        kept_match = true;
                        true
                    }
                });
                return Ok(incoming);
            }

            if incoming.id.trim().is_empty()
                || state
                    .official_accounts
                    .iter()
                    .any(|saved| saved.id == incoming.id)
            {
                incoming.id = uuid::Uuid::new_v4().to_string();
            }
            if incoming.created_at == 0 {
                incoming.created_at = now;
            }
            if state.official_accounts.len() >= MAX_SAVED_OPENAI_ACCOUNTS {
                return Err(AppError::InvalidConfig(
                    "最多可保存 500 个 OpenAI 账号，请先删除不再使用的账号。".into(),
                ));
            }
            incoming.updated_at = now;
            state.official_accounts.push(incoming.clone());
            Ok(incoming)
        })
    }

    pub fn save_official_account_quota(
        &self,
        id: &str,
        quota: ProviderAccountQuota,
    ) -> Result<ProviderAccountQuota, AppError> {
        self.update(|state| {
            let account = state
                .official_accounts
                .iter_mut()
                .find(|account| account.id == id)
                .ok_or_else(|| {
                    AppError::InvalidConfig("OpenAI 账号不存在，可能已被删除。".into())
                })?;
            account.quota = quota.clone();
            Ok(quota)
        })
    }

    pub fn sync_official_credential(
        &self,
        id: &str,
        credential: &CodexAuthCredential,
        expires_at: Option<i64>,
    ) -> Result<StoredOfficialAccount, AppError> {
        validate_official_credential(credential)?;
        self.update(|state| {
            let account = state
                .official_accounts
                .iter_mut()
                .find(|account| account.id == id)
                .ok_or_else(|| AppError::InvalidConfig("OpenAI 账号不存在，请重新登录。".into()))?;
            if account.account_id != credential.tokens.account_id {
                return Err(AppError::InvalidConfig(
                    "OpenAI 返回了其他账号的登录信息，请重新登录。".into(),
                ));
            }
            if account.credential == *credential && account.expires_at == expires_at {
                return Ok(account.clone());
            }
            account.credential = credential.clone();
            account.expires_at = expires_at;
            account.updated_at = chrono::Utc::now().timestamp();
            Ok(account.clone())
        })
    }

    pub fn connections_activate_official_account(&self, id: &str) -> Result<(), AppError> {
        self.update(|state| {
            if !state
                .official_accounts
                .iter()
                .any(|account| account.id == id)
            {
                return Err(AppError::InvalidConfig(
                    "OpenAI 账号不存在，请重新登录。".into(),
                ));
            }
            state.active = ActiveState {
                kind: ActiveKind::Official,
                provider_id: None,
                account_id: Some(id.to_owned()),
            };
            Ok(())
        })
    }

    pub fn delete_official_account(&self, id: &str) -> Result<(), AppError> {
        self.update(|state| {
            if matches!(state.active.kind, ActiveKind::Official)
                && state.active.account_id.as_deref() == Some(id)
            {
                return Err(AppError::InvalidConfig(
                    "正在使用这个 OpenAI 账号，请先切换后再删除。".into(),
                ));
            }
            let before = state.official_accounts.len();
            state.official_accounts.retain(|account| account.id != id);
            if before == state.official_accounts.len() {
                return Err(AppError::InvalidConfig(
                    "OpenAI 账号不存在，可能已被删除。".into(),
                ));
            }
            Ok(())
        })
    }
}

fn normalize_official_account(account: &mut StoredOfficialAccount) -> Result<(), AppError> {
    account.id = account.id.trim().to_owned();
    account.name = account.name.trim().to_owned();
    account.account_id = account.account_id.trim().to_owned();
    account.email = account.email.trim().to_owned();
    if account.name.is_empty() {
        account.name = if account.email.is_empty() {
            "OpenAI".into()
        } else {
            account.email.clone()
        };
    }
    if account.account_id.is_empty() {
        return Err(AppError::InvalidConfig(
            "OpenAI 登录信息缺少账号标识，请重新登录。".into(),
        ));
    }
    ensure_char_limit(
        &account.name,
        MAX_DISPLAY_NAME_CHARS,
        "账号名称不能超过 100 个字符。",
    )?;
    ensure_char_limit(
        &account.account_id,
        MAX_ACCOUNT_ID_CHARS,
        "OpenAI 账号标识不能超过 512 个字符。",
    )?;
    ensure_char_limit(
        &account.email,
        MAX_EMAIL_CHARS,
        "账号邮箱不能超过 320 个字符。",
    )?;
    validate_official_credential(&account.credential)?;
    if account.account_id != account.credential.tokens.account_id {
        return Err(AppError::InvalidConfig(
            "OpenAI 账号与登录信息不匹配，请重新登录。".into(),
        ));
    }
    Ok(())
}

fn preserve_redacted_headers(
    incoming: &mut std::collections::BTreeMap<String, String>,
    existing: &std::collections::BTreeMap<String, String>,
) {
    for (name, value) in incoming.iter_mut() {
        if value.is_empty()
            && let Some(saved) = existing.get(name)
        {
            *value = saved.clone();
        }
    }
}

fn persist_files(
    app_path: &Path,
    connections_path: &Path,
    credentials_path: &Path,
    state: &AppConfig,
) -> anyhow::Result<()> {
    let app = AppFile {
        codex: state.codex.clone(),
        active: state.active.clone(),
    };
    let mut providers = Vec::with_capacity(state.providers.len());
    let mut credentials = CredentialsFile::default();
    for provider in &state.providers {
        let mut public = provider.clone();
        if let Some(key) = public.api_key.take() {
            credentials
                .provider_api_keys
                .insert(provider.id.clone(), key);
        }
        if !public.headers.is_empty() {
            credentials
                .provider_headers
                .insert(provider.id.clone(), public.headers.clone());
            public.headers = public
                .headers
                .keys()
                .map(|name| (name.clone(), String::new()))
                .collect();
        }
        providers.push(public);
    }
    let connections = ConnectionsFile {
        providers,
        official_accounts: state.official_accounts.clone(),
    };
    JsonStore::write_atomic(app_path, &app)?;
    JsonStore::write_atomic(connections_path, &connections)?;
    JsonStore::write_atomic(credentials_path, &credentials)?;
    Ok(())
}

pub fn data_root() -> PathBuf {
    if let Some(value) = std::env::var_os("CODEX_TOOLS_DATA_DIR") {
        return PathBuf::from(value);
    }
    #[cfg(target_os = "macos")]
    {
        dirs::data_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join("Library/Application Support")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("io.github.irasutoya.codex-tools")
    }
    #[cfg(not(target_os = "macos"))]
    {
        // 优先用户数据目录（Windows 的 %APPDATA% / Linux 的 ~/.local/share），
        // 避免应用安装在 Program Files 等只读位置时无法写入；
        // 旧版本把数据放在可执行文件旁的 data/ 目录，仍存在时继续使用（兼容迁移）。
        let preferred = dirs::data_dir().map(|dir| dir.join("io.github.irasutoya.codex-tools"));
        let legacy = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .map(|dir| dir.join("data"));
        match (preferred, legacy) {
            (Some(preferred), Some(legacy)) if !preferred.exists() && legacy.exists() => legacy,
            (Some(preferred), _) => preferred,
            (None, legacy) => legacy.unwrap_or_else(|| PathBuf::from(".")),
        }
    }
}

fn mark_active_profile(state: &AppConfig, profile: &mut ProviderProfile) {
    profile.active = matches!(state.active.kind, ActiveKind::Provider)
        && state.active.provider_id.as_deref() == Some(profile.id.as_str());
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::read(path).is_ok_and(|current| current == bytes) {
        secure_file(path)?;
        return Ok(());
    }
    let temporary = write_temporary(path, bytes)?;
    replace_temporary(&temporary, path)
}

pub fn atomic_write_if_unchanged(
    path: &Path,
    expected: &[u8],
    bytes: &[u8],
) -> anyhow::Result<bool> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if expected == bytes {
        return match fs::read(path) {
            Ok(current) if current == expected => {
                secure_file(path)?;
                Ok(true)
            }
            Ok(_) | Err(_) => Ok(false),
        };
    }
    let temporary = write_temporary(path, bytes)?;
    let unchanged = fs::read(path).is_ok_and(|current| current == expected);
    if !unchanged {
        let _ = fs::remove_file(&temporary);
        return Ok(false);
    }
    replace_temporary(&temporary, path)?;
    Ok(true)
}

fn write_temporary(path: &Path, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("data");
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    let mut file = create_private_file(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    Ok(temporary)
}

fn replace_temporary(temporary: &Path, path: &Path) -> anyhow::Result<()> {
    if let Err(error) = replace_file(temporary, path) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}

fn create_private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(unix)]
pub(crate) fn secure_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
pub(crate) fn secure_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
pub(crate) fn secure_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
pub(crate) fn secure_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> anyhow::Result<()> {
    fs::rename(source, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_app_data_before_parsing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("app.json");
        let file = fs::File::create(&path).unwrap();
        file.set_len(MAX_APP_DATA_BYTES + 1).unwrap();

        let error = Store::open(temp.path().to_path_buf()).err().unwrap();

        assert!(error.to_string().contains("超过 32 MB"));
    }

    fn official_account(account_id: &str, suffix: &str) -> StoredOfficialAccount {
        StoredOfficialAccount {
            id: String::new(),
            name: format!("OpenAI {suffix}"),
            account_id: account_id.into(),
            email: format!("{suffix}@example.test"),
            credential: CodexAuthCredential {
                auth_mode: "chatgpt".into(),
                openai_api_key: None,
                tokens: CodexAuthTokens {
                    id_token: format!("id-secret-{suffix}"),
                    access_token: format!("access-secret-{suffix}"),
                    refresh_token: format!("refresh-secret-{suffix}"),
                    account_id: account_id.into(),
                },
                last_refresh: "2026-07-14T00:00:00Z".into(),
            },
            source: OfficialAccountSource::OpenAiOauth,
            expires_at: Some(1_800_000_000),
            quota: ProviderAccountQuota::default(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn provider(id: &str) -> ProviderProfile {
        ProviderProfile {
            id: id.into(),
            name: id.into(),
            base_url: "https://example.test/v1".into(),
            headers: Default::default(),
            timeout_secs: 30,
            enabled: true,
            active: false,
            model: String::new(),

            model_context_windows: Default::default(),
            available_models: Default::default(),
            models_dev_meta: Default::default(),
            api_type: ProviderApiType::Responses,
            api_key: Some("secret".into()),
            has_api_key: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn creates_app_json_without_touching_unrelated_files() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("unrelated.yaml"), "invalid: [").unwrap();
        fs::write(temp.path().join("unrelated.txt"), "user data").unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        assert!(store.path().ends_with("app.json"));
        assert_eq!(
            fs::read_to_string(temp.path().join("unrelated.txt")).unwrap(),
            "user data"
        );
    }

    #[cfg(unix)]
    #[test]
    fn app_data_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let store = Store::open(root.clone()).unwrap();
        assert_eq!(
            fs::metadata(root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn json_updates_are_atomic_and_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        store
            .update(|state| {
                state.codex.home = "/tmp/round-trip".into();
                Ok(())
            })
            .unwrap();
        let reopened = Store::open(temp.path().to_path_buf()).unwrap();
        assert_eq!(reopened.snapshot().unwrap().codex.home, "/tmp/round-trip");
        assert!(fs::read_dir(temp.path()).unwrap().count() >= 7);
    }

    #[test]
    fn conditional_atomic_write_rejects_concurrent_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout.jsonl");
        fs::write(&path, b"before").unwrap();
        fs::write(&path, b"concurrent append").unwrap();

        let written = atomic_write_if_unchanged(&path, b"before", b"replacement").unwrap();

        assert!(!written);
        assert_eq!(fs::read(path).unwrap(), b"concurrent append");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn identical_atomic_write_keeps_the_existing_file() {
        use std::os::unix::fs::MetadataExt;

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("data.json");
        fs::write(&path, b"unchanged").unwrap();
        let inode = fs::metadata(&path).unwrap().ino();

        atomic_write(&path, b"unchanged").unwrap();

        assert_eq!(fs::metadata(path).unwrap().ino(), inode);
    }

    #[test]
    fn official_account_save_deduplicates_external_account_id() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        let first = store
            .save_official_account(&official_account("workspace-1", "first"))
            .unwrap();
        store
            .update(|state| {
                let mut duplicate = state.official_accounts[0].clone();
                duplicate.id = "duplicate-record".into();
                state.official_accounts.push(duplicate);
                Ok(())
            })
            .unwrap();
        let second = store
            .save_official_account(&official_account("workspace-1", "second"))
            .unwrap();

        assert_eq!(second.id, first.id);
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.name, "OpenAI second");
        assert_eq!(store.snapshot().unwrap().official_accounts.len(), 1);
        assert!(
            fs::read_to_string(&store.connections_path)
                .unwrap()
                .contains("access-secret-second")
        );
    }

    #[test]
    fn official_views_are_redacted_and_active_uses_local_record_id() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        let saved = store
            .save_official_account(&official_account("workspace-1", "person"))
            .unwrap();
        store
            .connections_activate_official_account(&saved.id)
            .unwrap();

        let state = store.snapshot().unwrap();
        assert!(matches!(state.active.kind, ActiveKind::Official));
        assert_eq!(state.active.provider_id, None);
        assert_eq!(state.active.account_id.as_deref(), Some(saved.id.as_str()));

        let view = store.official_account_view(&saved.id).unwrap();
        assert!(view.active);
        let serialized = serde_json::to_string(&view).unwrap();
        assert!(!serialized.contains("credential"));
        assert!(!serialized.contains("access-secret"));
        assert!(!serialized.contains("refresh-secret"));
        assert!(!serialized.contains("id-secret"));
    }

    #[test]
    fn provider_overview_is_single_snapshot_and_redacts_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        store
            .update(|state| {
                state.providers.push(ProviderProfile {
                    id: "provider-1".into(),
                    name: "Provider".into(),
                    base_url: "https://example.test/v1".into(),
                    headers: [("x-secret".into(), "provider-secret".into())]
                        .into_iter()
                        .collect(),
                    timeout_secs: 30,
                    enabled: true,
                    active: false,
                    model: String::new(),

                    model_context_windows: Default::default(),
                    available_models: Default::default(),
                    models_dev_meta: Default::default(),
                    api_type: ProviderApiType::Responses,
                    api_key: Some("api-secret".into()),
                    has_api_key: false,
                    created_at: 1,
                    updated_at: 1,
                });
                state.active = ActiveState {
                    kind: ActiveKind::Provider,
                    provider_id: Some("provider-1".into()),
                    account_id: None,
                };
                Ok(())
            })
            .unwrap();

        let overview = store.provider_overview().unwrap();
        assert!(overview.providers[0].active);
        assert_eq!(overview.providers[0].headers["x-secret"], "");
        assert!(overview.providers[0].api_key.is_none());
        let serialized = serde_json::to_string(&overview).unwrap();
        assert!(!serialized.contains("provider-secret"));
        assert!(!serialized.contains("api-secret"));
    }

    #[test]
    fn credential_sync_checks_identity_and_active_account_cannot_be_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        let saved = store
            .save_official_account(&official_account("workspace-1", "old"))
            .unwrap();
        let replacement = official_account("workspace-1", "new").credential;
        let updated = store
            .sync_official_credential(&saved.id, &replacement, Some(1_900_000_000))
            .unwrap();
        assert_eq!(updated.credential.tokens.access_token, "access-secret-new");
        assert_eq!(updated.expires_at, Some(1_900_000_000));

        let wrong = official_account("workspace-2", "wrong").credential;
        assert!(
            store
                .sync_official_credential(&saved.id, &wrong, None)
                .is_err()
        );

        store
            .connections_activate_official_account(&saved.id)
            .unwrap();
        assert!(store.delete_official_account(&saved.id).is_err());
    }

    #[test]
    fn provider_requires_an_api_key_before_activation() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        let mut without_key = provider("empty");
        without_key.api_key = None;
        store.connections_save_provider(without_key).unwrap();

        assert!(store.activate("empty").is_err());

        let mut with_key = provider("ready");
        with_key.api_key = Some("secret".into());
        store.connections_save_provider(with_key).unwrap();
        store.activate("ready").unwrap();
        assert!(store.provider("ready").unwrap().active);
    }

    #[test]
    fn active_provider_cannot_be_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        store.connections_save_provider(provider("active")).unwrap();
        store.activate("active").unwrap();

        let mut disabled = store.provider("active").unwrap();
        disabled.enabled = false;
        assert!(store.connections_save_provider(disabled).is_err());
        assert!(store.provider("active").unwrap().enabled);
    }

    #[test]
    fn editing_redacted_provider_preserves_the_saved_api_key() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        store
            .connections_save_provider(provider("editable"))
            .unwrap();

        // 模拟前端保存：overview 返回的是脱敏后的 profile（api_key 为空），
        // 编辑后只改了名称，密钥必须保留。
        let mut redacted = store.provider_overview().unwrap().providers[0].clone();
        assert!(redacted.api_key.is_none());
        assert!(redacted.has_api_key);
        redacted.name = "改名后的服务".into();
        store.connections_save_provider(redacted).unwrap();

        let saved = store.provider("editable").unwrap();
        assert_eq!(saved.name, "改名后的服务");
        assert_eq!(saved.api_key.as_deref(), Some("secret"));
    }

    #[test]
    fn update_persists_mutated_state_to_disk_for_reopen() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let store = Store::open(root.clone()).unwrap();
        store
            .connections_save_provider(provider("persisted"))
            .unwrap();
        store.activate("persisted").unwrap();

        let reopened = Store::open(root).unwrap();
        let state = reopened.snapshot().unwrap();
        assert_eq!(state.providers.len(), 1);
        assert_eq!(state.providers[0].id, "persisted");
        assert_eq!(state.active.provider_id.as_deref(), Some("persisted"));
        assert_eq!(state.active.account_id, None);
    }

    #[test]
    fn failed_persist_keeps_memory_updated_and_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(temp.path().to_path_buf()).unwrap();
        store.connections_save_provider(provider("one")).unwrap();
        fs::write(temp.path().join("blocked"), b"file").unwrap();
        store.path = temp.path().join("blocked").join("app.json");

        let result = store.update(|state| {
            state.codex.home = "/tmp/alternate".into();
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(
            store.read(|state| state.codex.home.clone()).unwrap(),
            "/tmp/alternate"
        );
    }
}
