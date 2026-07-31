use crate::models::*;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{RwLock, RwLockReadGuard},
};

pub struct Store {
    root: PathBuf,
    path: PathBuf,
    state: RwLock<AppConfig>,
}

impl Store {
    pub fn new() -> anyhow::Result<Self> {
        let root = data_root();
        Self::open(root)
    }

    pub fn open(root: PathBuf) -> anyhow::Result<Self> {
        fs::create_dir_all(&root)?;
        secure_directory(&root)?;
        let path = root.join("app.yaml");
        let state = if path.exists() {
            let text = fs::read_to_string(&path)?;
            serde_yaml::from_str::<AppConfig>(&text).map_err(|error| {
                anyhow::anyhow!("应用数据文件损坏，无法读取已保存的账号和服务：{error}")
            })?
        } else {
            let state = AppConfig::default();
            atomic_yaml(&path, &state)?;
            state
        };
        secure_file(&path)?;
        Ok(Self {
            root,
            path,
            state: RwLock::new(state),
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

    pub fn is_active_provider(&self, id: &str) -> Result<bool, AppError> {
        let state = self.read_state()?;
        Ok(matches!(state.active.kind, ActiveKind::Provider)
            && state.active.provider_id.as_deref() == Some(id))
    }

    pub fn is_active_account(&self, id: &str) -> Result<bool, AppError> {
        let state = self.read_state()?;
        Ok(state.active.account_id.as_deref() == Some(id))
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
        atomic_yaml(&self.path, &draft).map_err(|error| AppError::Internal(error.to_string()))?;
        *guard = draft;
        Ok(result)
    }

    pub fn provider(&self, id: &str) -> Result<ProviderProfile, AppError> {
        let state = self.read_state()?;
        let stored = state
            .providers
            .iter()
            .find(|provider| provider.profile.id == id)
            .ok_or_else(|| AppError::InvalidConfig("第三方 API 服务不存在，请刷新页面。".into()))?;
        let mut profile = stored.profile.clone();
        profile.active = matches!(state.active.kind, ActiveKind::Provider)
            && state.active.provider_id.as_deref() == Some(profile.id.as_str());
        profile.active_account_id = if profile.active {
            state.active.account_id.clone()
        } else {
            stored.profile.active_account_id.clone()
        };
        profile.account_count = stored.accounts.len() as u64;
        Ok(profile)
    }

    pub fn save_provider(
        &self,
        mut provider: ProviderProfile,
    ) -> Result<ProviderProfile, AppError> {
        if provider.id.trim().is_empty() {
            provider.id = uuid::Uuid::new_v4().to_string();
        }
        provider.normalize_and_validate()?;
        let existing_headers = {
            let state = self.read_state()?;
            state
                .providers
                .iter()
                .find(|value| value.profile.id == provider.id)
                .map(|value| value.profile.headers.clone())
        };
        if let Some(existing_headers) = existing_headers {
            preserve_redacted_headers(&mut provider.headers, &existing_headers);
        }
        provider.active = false;
        provider.account_count = 0;
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
                .find(|value| value.profile.id == provider.id)
            {
                let accounts = std::mem::take(&mut existing.accounts);
                let active_account_id = existing.profile.active_account_id.clone();
                *existing = StoredProvider {
                    profile: provider.clone(),
                    accounts,
                };
                existing.profile.active_account_id = active_account_id;
            } else {
                state.providers.push(StoredProvider {
                    profile: provider.clone(),
                    accounts: vec![],
                });
            }
            Ok(())
        })?;
        Ok(saved)
    }

    pub fn delete_provider(&self, id: &str) -> Result<(), AppError> {
        self.update(|state| {
            if state.active.provider_id.as_deref() == Some(id) {
                return Err(AppError::InvalidConfig(
                    "正在使用此服务，切换后才能删除。".into(),
                ));
            }
            let before = state.providers.len();
            state.providers.retain(|value| value.profile.id != id);
            if before == state.providers.len() {
                return Err(AppError::InvalidConfig(
                    "第三方 API 服务不存在，可能已被删除。".into(),
                ));
            }
            Ok(())
        })
    }

    pub fn account(&self, id: &str) -> Result<ProviderAccount, AppError> {
        let state = self.read_state()?;
        let mut account = state
            .providers
            .iter()
            .flat_map(|provider| &provider.accounts)
            .find(|account| account.id == id)
            .cloned()
            .ok_or_else(|| AppError::InvalidConfig("API Key 不存在，可能已被删除。".into()))?;
        account.active = matches!(state.active.kind, ActiveKind::Provider)
            && state.active.account_id.as_deref() == Some(account.id.as_str());
        Ok(account)
    }

    pub fn provider_overview(&self) -> Result<ProviderOverview, AppError> {
        let state = self.read_state()?;
        let providers = state
            .providers
            .iter()
            .map(|stored| {
                let mut profile = stored.profile.clone();
                profile.active = matches!(state.active.kind, ActiveKind::Provider)
                    && state.active.provider_id.as_deref() == Some(profile.id.as_str());
                profile.active_account_id = if profile.active {
                    state.active.account_id.clone()
                } else {
                    stored.profile.active_account_id.clone()
                };
                profile.account_count = stored.accounts.len() as u64;
                profile.redacted()
            })
            .collect();
        let accounts = state
            .providers
            .iter()
            .flat_map(|provider| provider.accounts.iter())
            .cloned()
            .map(|mut account| {
                account.active = matches!(state.active.kind, ActiveKind::Provider)
                    && state.active.account_id.as_deref() == Some(account.id.as_str());
                account.redacted()
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
            accounts,
            official_accounts,
        })
    }

    pub fn save_account(&self, mut account: ProviderAccount) -> Result<ProviderAccount, AppError> {
        if account.id.trim().is_empty() {
            account.id = uuid::Uuid::new_v4().to_string();
        }
        let existing = {
            let state = self.read_state()?;
            state.providers.iter().find_map(|provider| {
                provider
                    .accounts
                    .iter()
                    .find(|value| value.id == account.id)
                    .cloned()
                    .map(|account| (provider.profile.id.clone(), account))
            })
        };
        if let Some((existing_provider_id, existing)) = existing {
            if account.provider_id.as_deref() != Some(existing_provider_id.as_str()) {
                return Err(AppError::InvalidConfig(
                    "不能把已保存的 API Key 移到其他服务，请新建一个 API Key。".into(),
                ));
            }
            if account
                .api_key
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                account.api_key = existing.api_key;
            }
            preserve_redacted_headers(&mut account.headers, &existing.headers);
        }
        account.normalize_and_validate()?;
        let now = chrono::Utc::now().timestamp();
        if account.created_at == 0 {
            account.created_at = now;
        }
        account.updated_at = now;
        account.active = false;
        let provider_id = account
            .provider_id
            .clone()
            .ok_or_else(|| AppError::InvalidConfig("请选择 API Key 所属的第三方服务。".into()))?;
        let saved = account.clone();
        self.update(|state| {
            if let Some(owner) = state
                .providers
                .iter()
                .find(|provider| provider.accounts.iter().any(|value| value.id == account.id))
                && owner.profile.id != provider_id
            {
                return Err(AppError::InvalidConfig(
                    "不能把已保存的 API Key 移到其他服务，请新建一个 API Key。".into(),
                ));
            }
            let provider = state
                .providers
                .iter_mut()
                .find(|value| value.profile.id == provider_id)
                .ok_or_else(|| {
                    AppError::InvalidConfig("第三方 API 服务不存在，请刷新页面。".into())
                })?;
            if let Some(existing) = provider
                .accounts
                .iter_mut()
                .find(|value| value.id == account.id)
            {
                *existing = account.clone();
            } else {
                provider.accounts.push(account.clone());
            }
            Ok(())
        })?;
        Ok(saved)
    }

    pub fn delete_account(&self, id: &str) -> Result<(), AppError> {
        self.update(|state| {
            if state.active.account_id.as_deref() == Some(id) {
                return Err(AppError::InvalidConfig(
                    "正在使用这个 API Key，切换后才能删除。".into(),
                ));
            }
            let mut found = false;
            for provider in &mut state.providers {
                let before = provider.accounts.len();
                provider.accounts.retain(|value| value.id != id);
                found |= before != provider.accounts.len();
            }
            if !found {
                return Err(AppError::InvalidConfig(
                    "API Key 不存在，可能已被删除。".into(),
                ));
            }
            Ok(())
        })
    }

    pub fn activate(&self, provider_id: &str, account_id: &str) -> Result<(), AppError> {
        self.update(|state| {
            let provider = state
                .providers
                .iter_mut()
                .find(|value| value.profile.id == provider_id)
                .ok_or_else(|| {
                    AppError::InvalidConfig("第三方 API 服务不存在，请刷新页面。".into())
                })?;
            if !provider.profile.enabled {
                return Err(AppError::InvalidConfig(
                    "此服务已停用，请先编辑并启用它。".into(),
                ));
            }
            if !provider
                .accounts
                .iter()
                .any(|account| account.id == account_id)
            {
                return Err(AppError::InvalidConfig(
                    "所选 API Key 不属于这个服务，请刷新页面。".into(),
                ));
            }
            provider.profile.active_account_id = Some(account_id.to_owned());
            state.active = ActiveState {
                kind: ActiveKind::Provider,
                provider_id: Some(provider_id.to_owned()),
                account_id: Some(account_id.to_owned()),
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
                    AppError::InvalidConfig("Codex 账号不存在，可能已被删除。".into())
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

    pub fn activate_official_account(&self, id: &str) -> Result<(), AppError> {
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
    validate_official_credential(&account.credential)?;
    if account.account_id != account.credential.tokens.account_id {
        return Err(AppError::InvalidConfig(
            "OpenAI 账号与登录信息不匹配，请重新登录。".into(),
        ));
    }
    Ok(())
}

fn validate_official_credential(credential: &CodexAuthCredential) -> Result<(), AppError> {
    let tokens = &credential.tokens;
    let personal_access_token =
        tokens.id_token.trim().is_empty() || tokens.refresh_token.trim().is_empty();
    if credential.auth_mode != "chatgpt"
        || credential.last_refresh.trim().is_empty()
        || tokens.access_token.trim().is_empty()
        || tokens.account_id.trim().is_empty()
        || (!personal_access_token
            && (tokens.id_token.trim().is_empty() || tokens.refresh_token.trim().is_empty()))
    {
        return Err(AppError::InvalidConfig(
            "OpenAI 登录信息不完整，请重新登录。".into(),
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
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("data")
    }
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

fn atomic_yaml(path: &Path, value: &AppConfig) -> anyhow::Result<()> {
    let text = serde_yaml::to_string(value)?;
    atomic_write(path, text.as_bytes())
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
fn secure_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) -> std::io::Result<()> {
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
            active_account_id: None,
            account_count: 0,
        }
    }

    fn account(id: &str, provider_id: &str) -> ProviderAccount {
        ProviderAccount {
            id: id.into(),
            provider_id: Some(provider_id.into()),
            name: id.into(),
            auth_kind: AccountAuthKind::ApiKey,
            api_key: Some("secret".into()),
            headers: Default::default(),
            active: false,
            email: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn creates_app_yaml_without_touching_unrelated_files() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("unrelated.yaml"), "invalid: [").unwrap();
        fs::write(temp.path().join("unrelated.txt"), "user data").unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        assert!(store.path().ends_with("app.yaml"));
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
    fn yaml_updates_are_atomic_and_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        store
            .update(|state| {
                state.app.theme = "dark".into();
                Ok(())
            })
            .unwrap();
        let reopened = Store::open(temp.path().to_path_buf()).unwrap();
        assert_eq!(reopened.snapshot().unwrap().app.theme, "dark");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
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
            fs::read_to_string(store.path())
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
        store.activate_official_account(&saved.id).unwrap();

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
                state.providers.push(StoredProvider {
                    profile: ProviderProfile {
                        id: "provider-1".into(),
                        name: "Provider".into(),
                        base_url: "https://example.test/v1".into(),
                        headers: [("x-secret".into(), "provider-secret".into())]
                            .into_iter()
                            .collect(),
                        timeout_secs: 30,
                        enabled: true,
                        active: false,
                        active_account_id: None,
                        account_count: 0,
                    },
                    accounts: vec![ProviderAccount {
                        id: "account-1".into(),
                        provider_id: Some("provider-1".into()),
                        name: "Account".into(),
                        auth_kind: AccountAuthKind::ApiKey,
                        api_key: Some("api-secret".into()),
                        headers: [("x-secret".into(), "account-secret".into())]
                            .into_iter()
                            .collect(),
                        active: false,
                        email: None,
                        created_at: 1,
                        updated_at: 1,
                    }],
                });
                state.active = ActiveState {
                    kind: ActiveKind::Provider,
                    provider_id: Some("provider-1".into()),
                    account_id: Some("account-1".into()),
                };
                Ok(())
            })
            .unwrap();

        let overview = store.provider_overview().unwrap();
        assert!(overview.providers[0].active);
        assert_eq!(overview.providers[0].account_count, 1);
        assert_eq!(overview.providers[0].headers["x-secret"], "");
        assert!(overview.accounts[0].active);
        assert!(overview.accounts[0].api_key.is_none());
        assert_eq!(overview.accounts[0].headers["x-secret"], "");
        let serialized = serde_json::to_string(&overview).unwrap();
        assert!(!serialized.contains("provider-secret"));
        assert!(!serialized.contains("account-secret"));
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

        store.activate_official_account(&saved.id).unwrap();
        assert!(store.delete_official_account(&saved.id).is_err());
    }

    #[test]
    fn account_ids_cannot_move_between_providers() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        store.save_provider(provider("one")).unwrap();
        store.save_provider(provider("two")).unwrap();
        store.save_account(account("shared", "one")).unwrap();

        let error = store.save_account(account("shared", "two")).unwrap_err();
        assert!(error.to_string().contains("移到"));
        let state = store.snapshot().unwrap();
        assert_eq!(state.providers[0].accounts.len(), 1);
        assert!(state.providers[1].accounts.is_empty());
    }

    #[test]
    fn active_provider_cannot_be_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().to_path_buf()).unwrap();
        store.save_provider(provider("active")).unwrap();
        store.save_account(account("account", "active")).unwrap();
        store.activate("active", "account").unwrap();

        let mut disabled = store.provider("active").unwrap();
        disabled.enabled = false;
        assert!(store.save_provider(disabled).is_err());
        assert!(store.provider("active").unwrap().enabled);
    }
}
