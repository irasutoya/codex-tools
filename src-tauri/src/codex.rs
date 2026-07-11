use crate::models::*;
use crate::protocol_proxy::ProxyEndpoint;
use rusqlite::{Connection, params};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use toml_edit::{DocumentMut, Item, Table, value};
use walkdir::WalkDir;

pub const MANAGED_PROVIDER_ID: &str = "custom";
pub const MODEL_CATALOG_FILENAME: &str = "codex-tools-model-catalog.json";
const MODEL_CATALOG_TEMPLATE_SLUG: &str = "gpt-5.5";

#[derive(Clone, Copy, PartialEq, Eq)]
enum CatalogProfile {
    ProxyChat,
    NativeResponses,
}

pub fn home() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex")
}
pub fn databases() -> Vec<PathBuf> {
    let h = home();
    let mut out = Vec::new();
    let legacy = h.join("state_5.sqlite");
    if legacy.exists() {
        out.push(legacy)
    }
    let dir = h.join("sqlite");
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "db" || x == "sqlite") {
                out.push(p)
            }
        }
    }
    out
}
fn has_threads(db: &Connection) -> bool {
    db.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='threads'",
        [],
        |r| r.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}
fn columns(db: &Connection) -> Vec<String> {
    table_columns(db, "threads")
}
fn table_columns(db: &Connection, table: &str) -> Vec<String> {
    if !matches!(table, "threads" | "local_thread_catalog") {
        return vec![];
    }
    let Ok(mut s) = db.prepare(&format!("PRAGMA table_info({table})")) else {
        return vec![];
    };
    s.query_map([], |r| r.get(1))
        .map(|x| x.flatten().collect())
        .unwrap_or_default()
}
pub fn scan() -> RepairScan {
    let mut warnings = vec![];
    let mut scans = vec![];
    for p in databases() {
        match Connection::open(&p) {
            Ok(db) => {
                let health = db
                    .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                    .unwrap_or_else(|_| "unreadable".into());
                let legacy_columns = columns(&db);
                let catalog_columns = table_columns(&db, "local_thread_catalog");
                let legacy = has_threads(&db)
                    && legacy_columns.iter().any(|column| column == "id")
                    && legacy_columns
                        .iter()
                        .any(|column| column == "model_provider");
                let catalog = catalog_columns.iter().any(|column| column == "thread_id")
                    && catalog_columns
                        .iter()
                        .any(|column| column == "model_provider");
                let has_legacy_table = has_threads(&db);
                let has_catalog_table = !catalog_columns.is_empty();
                let known = legacy || catalog;
                let count = if known {
                    let table = if legacy {
                        "threads"
                    } else {
                        "local_thread_catalog"
                    };
                    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                        r.get::<_, u64>(0)
                    })
                    .unwrap_or(0)
                } else {
                    0
                };
                if !known && (has_legacy_table || has_catalog_table) {
                    warnings.push(format!("未知会话目录 schema：{}", p.display()));
                    scans.push(DatabaseScan {
                        path: p.display().to_string(),
                        health,
                        known_schema: false,
                        thread_count: 0,
                    });
                    continue;
                }
                if !known {
                    warnings.push(format!("已跳过不包含会话目录的辅助数据库：{}", p.display()));
                    continue;
                }
                scans.push(DatabaseScan {
                    path: p.display().to_string(),
                    health,
                    known_schema: known,
                    thread_count: count,
                })
            }
            Err(e) => warnings.push(format!("无法打开 {}：{e}", p.display())),
        }
    }
    let rollouts = rollout_files();
    RepairScan {
        operation_id: uuid::Uuid::new_v4().to_string(),
        can_repair: scans.iter().all(|x| x.known_schema && x.health == "ok"),
        databases: scans,
        rollout_files: rollouts.len(),
        warnings,
    }
}
pub fn rollout_files() -> Vec<PathBuf> {
    let h = home();
    [h.join("sessions"), h.join("archived_sessions")]
        .into_iter()
        .flat_map(|d| {
            WalkDir::new(d)
                .into_iter()
                .filter_map(Result::ok)
                .map(|e| e.into_path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        })
        .collect()
}
pub fn list_sessions(query: Option<String>) -> anyhow::Result<Vec<SessionSummary>> {
    let q = query.unwrap_or_default().to_lowercase();
    let mut out = vec![];
    for path in databases() {
        let db = Connection::open(&path)?;
        let legacy_columns = columns(&db);
        let catalog_columns = table_columns(&db, "local_thread_catalog");
        let (table, id, cols) = if legacy_columns.contains(&"id".into()) {
            ("threads", "id", legacy_columns)
        } else if catalog_columns.contains(&"thread_id".into()) {
            ("local_thread_catalog", "thread_id", catalog_columns)
        } else {
            continue;
        };
        let sql = session_list_query(table, id, &cols);
        let mut st = db.prepare(&sql)?;
        let rows = st.query_map([], |r| {
            Ok(SessionSummary {
                identity: format!("{}#{}", path.display(), r.get::<_, String>(0)?),
                id: r.get(0)?,
                title: r.get::<_, String>(1).unwrap_or_default(),
                provider: r.get::<_, String>(2).unwrap_or_default(),
                cwd: r.get::<_, String>(3).unwrap_or_default(),
                archived: r.get::<_, i64>(4).unwrap_or(0) != 0,
                updated_at: r.get::<_, i64>(5).unwrap_or(0),
                source_db: path.display().to_string(),
                source_rollout: None,
                original_provider: r.get::<_, String>(2).unwrap_or_default(),
                has_user_event: false,
            })
        })?;
        for row in rows.flatten() {
            if q.is_empty()
                || format!("{} {} {} {}", row.id, row.title, row.provider, row.cwd)
                    .to_lowercase()
                    .contains(&q)
            {
                out.push(row)
            }
        }
    }
    Ok(out)
}

fn session_list_query(table: &str, id: &str, cols: &[String]) -> String {
    let title = if cols.contains(&"title".into()) {
        "title"
    } else if cols.contains(&"display_title".into()) {
        "display_title"
    } else {
        "''"
    };
    let provider = if cols.contains(&"model_provider".into()) {
        "model_provider"
    } else {
        "''"
    };
    let cwd = if cols.contains(&"cwd".into()) {
        "cwd"
    } else {
        "''"
    };
    let archived = if cols.contains(&"archived".into()) {
        "archived"
    } else {
        "0"
    };
    let updated = if cols.contains(&"updated_at".into()) {
        "updated_at"
    } else if cols.contains(&"source_updated_at".into()) {
        "CAST(source_updated_at AS INTEGER)"
    } else {
        "0"
    };
    format!(
        "SELECT {id},{title},{provider},{cwd},{archived},{updated} AS sort_updated FROM {table} ORDER BY sort_updated DESC LIMIT 1000"
    )
}
fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("codex-tools");
    let tmp = path.with_file_name(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut file = fs::File::create(&tmp)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = replace_file(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(error);
    }
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
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
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
fn backup(label: &str) -> anyhow::Result<PathBuf> {
    let root = std::env::temp_dir()
        .join("codex-tools")
        .join(format!("{label}-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir_all(&root)?;
    for name in [
        "config.toml",
        "auth.json",
        MODEL_CATALOG_FILENAME,
        ".codex-global-state.json",
    ] {
        let src = home().join(name);
        if src.exists() {
            fs::copy(src, root.join(name))?;
        }
    }
    Ok(root)
}
#[allow(dead_code)]
pub fn apply_provider(p: &ProviderProfile, account: &ProviderAccount) -> Result<String, AppError> {
    if p.protocol == ProviderProtocol::ChatCompletions {
        return Err(AppError::InvalidConfig(
            "Chat Completions 需要先启动本地协议代理".into(),
        ));
    }
    apply_provider_with_proxy(p, account, None)
}
pub fn apply_provider_with_proxy(
    p: &ProviderProfile,
    account: &ProviderAccount,
    proxy: Option<&ProxyEndpoint>,
) -> Result<String, AppError> {
    if p.name.trim().is_empty()
        || p.base_url.trim().is_empty()
        || account
            .api_key
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        return Err(AppError::InvalidConfig(
            "provider fields are required".into(),
        ));
    }
    let backup_path = backup("provider").map_err(|e| AppError::Backup(e.to_string()))?;
    let token = match p.protocol {
        ProviderProtocol::Responses => account.api_key.clone().unwrap_or_default(),
        ProviderProtocol::ChatCompletions => {
            let endpoint = proxy.ok_or_else(|| AppError::Proxy("proxy is not running".into()))?;
            endpoint.token.clone()
        }
    };
    let base_url = match p.protocol {
        ProviderProtocol::Responses => p.base_url.trim_end_matches('/'),
        ProviderProtocol::ChatCompletions => proxy
            .ok_or_else(|| AppError::Proxy("proxy is not running".into()))?
            .base_url
            .trim_end_matches('/'),
    };
    let config = build_managed_config(p, account, base_url, &token)?;
    let auth = build_managed_auth(&token);
    let catalog = if p.models.is_empty() {
        None
    } else {
        Some(build_model_catalog(p)?)
    };
    let update = (|| -> Result<(), AppError> {
        fs::create_dir_all(home()).map_err(|error| AppError::Internal(error.to_string()))?;
        let catalog_path = home().join(MODEL_CATALOG_FILENAME);
        if let Some(catalog) = catalog {
            atomic_write(
                &catalog_path,
                &serde_json::to_vec_pretty(&catalog)
                    .map_err(|error| AppError::Internal(error.to_string()))?,
            )
            .map_err(|error| AppError::Internal(error.to_string()))?;
        } else if catalog_path.exists() {
            fs::remove_file(&catalog_path)
                .map_err(|error| AppError::Internal(error.to_string()))?;
        }
        atomic_write(
            &home().join("auth.json"),
            &serde_json::to_vec_pretty(&auth)
                .map_err(|error| AppError::Internal(error.to_string()))?,
        )
        .map_err(|error| AppError::Internal(error.to_string()))?;
        atomic_write(&home().join("config.toml"), config.as_bytes())
            .map_err(|error| AppError::Internal(error.to_string()))?;
        Ok(())
    })();
    match update {
        Ok(()) => Ok(backup_path.display().to_string()),
        Err(error) => match restore_provider_backup(&backup_path.display().to_string()) {
            Ok(()) => Err(error),
            Err(restore_error) => Err(AppError::Backup(format!(
                "configuration update failed: {error}; rollback failed: {restore_error}"
            ))),
        },
    }
}

fn build_managed_config(
    provider: &ProviderProfile,
    account: &ProviderAccount,
    base_url: &str,
    token: &str,
) -> Result<String, AppError> {
    let existing = fs::read_to_string(home().join("config.toml")).unwrap_or_default();
    build_managed_config_from(&existing, provider, account, base_url, token)
}

fn build_managed_config_from(
    existing: &str,
    provider: &ProviderProfile,
    account: &ProviderAccount,
    base_url: &str,
    token: &str,
) -> Result<String, AppError> {
    // Provider switching must not disable Codex desktop capabilities. Keep the
    // user's runtime, plugin, MCP, project trust and UI settings, and replace
    // only fields that select or authenticate a model provider.
    let mut doc = config_without_provider_selection(existing)?;
    doc["model_provider"] = value(MANAGED_PROVIDER_ID);
    if !provider.models.is_empty() {
        doc["model_catalog_json"] = value(MODEL_CATALOG_FILENAME);
    }
    let mut providers = Table::new();
    let mut managed = Table::new();
    managed["name"] = value(provider.name.trim());
    managed["base_url"] = value(base_url);
    managed["wire_api"] = value("responses");
    managed["requires_openai_auth"] = value(true);
    managed["experimental_bearer_token"] = value(token);
    if provider.protocol == ProviderProtocol::Responses {
        let headers = merged_header_table(&provider.headers, &account.headers);
        if !headers.is_empty() {
            managed["http_headers"] = Item::Table(headers);
        }
    }
    providers[MANAGED_PROVIDER_ID] = Item::Table(managed);
    doc["model_providers"] = Item::Table(providers);
    Ok(doc.to_string())
}

fn config_without_provider_selection(config: &str) -> Result<DocumentMut, AppError> {
    let mut doc = if config.trim().is_empty() {
        DocumentMut::new()
    } else {
        config.parse::<DocumentMut>().map_err(|error| {
            AppError::InvalidConfig(format!("Codex config.toml 语法无效：{error}"))
        })?
    };
    for key in [
        "model",
        "model_provider",
        "model_catalog_json",
        "model_reasoning_effort",
        "model_reasoning_summary",
        "base_url",
        "wire_api",
        "experimental_bearer_token",
        "model_providers",
    ] {
        doc.as_table_mut().remove(key);
    }
    Ok(doc)
}

fn build_official_config(current: &str, snapshot: Option<&str>) -> Result<String, AppError> {
    let mut restored = snapshot
        .filter(|value| !value.trim().is_empty())
        .map(config_without_provider_selection)
        .transpose()?
        .unwrap_or_default();
    let current = config_without_provider_selection(current)?;
    // Current desktop/plugin settings are newer than an account snapshot. They
    // win when present, while a snapshot still restores settings missing from
    // legacy provider-only configurations.
    for (key, item) in current.as_table().iter() {
        restored.as_table_mut().insert(key, item.clone());
    }
    Ok(restored.to_string())
}

fn merged_header_table(provider: &serde_json::Value, account: &serde_json::Value) -> Table {
    let mut headers = std::collections::BTreeMap::new();
    for source in [provider, account] {
        if let Some(values) = source.as_object() {
            for (name, value) in values {
                if let Some(value) = value.as_str() {
                    let name = name.trim();
                    if !name.is_empty() && !value.is_empty() {
                        headers.insert(name.to_string(), value.to_string());
                    }
                }
            }
        }
    }
    let mut table = Table::new();
    for (name, header_value) in headers {
        table[&name] = value(header_value);
    }
    table
}

fn build_managed_auth(token: &str) -> serde_json::Value {
    serde_json::json!({
        "OPENAI_API_KEY": token,
    })
}

fn build_model_catalog(provider: &ProviderProfile) -> Result<serde_json::Value, AppError> {
    let profile = match provider.protocol {
        ProviderProtocol::ChatCompletions => CatalogProfile::ProxyChat,
        ProviderProtocol::Responses => CatalogProfile::NativeResponses,
    };
    let template = match profile {
        CatalogProfile::ProxyChat => load_proxy_chat_template()?,
        CatalogProfile::NativeResponses => load_native_responses_template()?,
    };
    Ok(build_model_catalog_from_template(
        provider, &template, profile,
    ))
}

fn build_model_catalog_from_template(
    provider: &ProviderProfile,
    template: &serde_json::Value,
    profile: CatalogProfile,
) -> serde_json::Value {
    let mut seen = std::collections::HashSet::new();
    let models = provider.models.iter().filter_map(|raw_model| {
        let model = raw_model.trim();
        if model.is_empty() || !seen.insert(model.to_string()) {
            return None;
        }
        let upstream = provider.model_metadata.iter().find(|item| item.id == model);
        let mut entry = template.clone();
        let object = entry
            .as_object_mut()
            .expect("model template must be an object");
        object.insert("slug".into(), model.into());
        object.insert("display_name".into(), model.into());
        object.insert("description".into(), model.into());
        object.insert("priority".into(), (1000 + seen.len()).into());
        object.insert("additional_speed_tiers".into(), serde_json::json!([]));
        object.insert("service_tiers".into(), serde_json::json!([]));
        object.insert("availability_nux".into(), serde_json::Value::Null);
        object.insert("upgrade".into(), serde_json::Value::Null);
        let context_window = upstream
            .and_then(model_context_window)
            .or(provider.context_window)
            .filter(|value| *value > 0);
        if let Some(context_window) = context_window {
            object.insert("context_window".into(), context_window.into());
            object.insert("max_context_window".into(), context_window.into());
        }
        if profile == CatalogProfile::NativeResponses {
            apply_native_model_metadata(object, upstream);
        }
        Some(entry)
    });
    serde_json::json!({ "models": models.collect::<Vec<_>>() })
}

fn find_model_template(catalog: &serde_json::Value) -> Option<serde_json::Value> {
    catalog
        .get("models")
        .and_then(serde_json::Value::as_array)
        .and_then(|models| {
            models.iter().find(|model| {
                model.get("slug").and_then(serde_json::Value::as_str)
                    == Some(MODEL_CATALOG_TEMPLATE_SLUG)
            })
        })
        .filter(|model| {
            model
                .get("base_instructions")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.trim().is_empty())
                && model.get("model_messages").is_some()
                && model.get("apply_patch_tool_type").is_some()
        })
        .cloned()
}

fn load_proxy_chat_template() -> Result<serde_json::Value, AppError> {
    let cache_path = home().join("models_cache.json");
    if cache_path.exists() {
        let bytes = fs::read(&cache_path).map_err(|error| AppError::Internal(error.to_string()))?;
        let catalog: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| AppError::InvalidConfig(format!("models_cache.json 无效：{error}")))?;
        if let Some(template) = find_model_template(&catalog) {
            return Ok(template);
        }
    }
    if let Ok(output) = Command::new("codex")
        .args(["debug", "models", "--bundled"])
        .output()
        && output.status.success()
        && let Ok(catalog) = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        && let Some(template) = find_model_template(&catalog)
    {
        return Ok(template);
    }
    Err(AppError::InvalidConfig(format!(
        "找不到本机 Codex 的完整 {MODEL_CATALOG_TEMPLATE_SLUG} agent 模板；请先启动一次 Codex 或确保 codex CLI 可用。已取消切换，以免生成会丢失工具能力的模型目录"
    )))
}

fn load_native_responses_template() -> Result<serde_json::Value, AppError> {
    Ok(serde_json::json!({
        "slug": "native-responses-template",
        "display_name": "native-responses-template",
        "description": "native-responses-template",
        "base_instructions": "You are Codex, a coding agent. You and the user share the same workspace and collaborate to achieve the user's goals.",
        "default_reasoning_level": "high",
        "supported_reasoning_levels": [
            {"effort": "none", "description": "Disable reasoning"},
            {"effort": "high", "description": "Enable reasoning"}
        ],
        "shell_type": "shell_command",
        "visibility": "list",
        "supported_in_api": true,
        "priority": 0,
        "supports_reasoning_summaries": true,
        "default_reasoning_summary": "none",
        "support_verbosity": false,
        "truncation_policy": {"mode": "bytes", "limit": 10000},
        "supports_parallel_tool_calls": false,
        "supports_image_detail_original": false,
        "context_window": 262144,
        "max_context_window": 262144,
        "effective_context_window_percent": 95,
        "experimental_supported_tools": [],
        "input_modalities": ["text"],
        "supports_search_tool": false
    }))
}

fn model_context_window(model: &FetchedModel) -> Option<u64> {
    ["context_window", "contextWindow", "max_context_window"]
        .iter()
        .find_map(|name| model.metadata.get(*name))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.trim().parse().ok())
        })
        .filter(|value| *value > 0)
}

fn apply_native_model_metadata(
    target: &mut serde_json::Map<String, serde_json::Value>,
    upstream: Option<&FetchedModel>,
) {
    let Some(upstream) = upstream else { return };
    const FIELDS: &[(&str, &[&str])] = &[
        (
            "supports_parallel_tool_calls",
            &["supports_parallel_tool_calls", "supportsParallelToolCalls"],
        ),
        ("input_modalities", &["input_modalities", "inputModalities"]),
        (
            "base_instructions",
            &["base_instructions", "baseInstructions"],
        ),
    ];
    for (target_name, aliases) in FIELDS {
        if let Some(value) = aliases.iter().find_map(|name| upstream.metadata.get(*name)) {
            target.insert((*target_name).into(), value.clone());
        }
    }
    for key in [
        "apply_patch_tool_type",
        "web_search_tool_type",
        "tools",
        "model_messages",
    ] {
        target.remove(key);
    }
    target.insert("shell_type".into(), "shell_command".into());
}

pub fn restore_official_snapshot(
    auth: &serde_json::Value,
    config_snapshot: Option<&str>,
) -> Result<String, AppError> {
    let backup_path =
        backup("official-account").map_err(|error| AppError::Backup(error.to_string()))?;
    let update = (|| -> Result<(), AppError> {
        let current = fs::read_to_string(home().join("config.toml")).unwrap_or_default();
        let config = build_official_config(&current, config_snapshot)?;
        atomic_write(&home().join("config.toml"), config.as_bytes())?;
        atomic_write(
            &home().join("auth.json"),
            &serde_json::to_vec_pretty(auth)
                .map_err(|error| AppError::Internal(error.to_string()))?,
        )?;
        let catalog = home().join(MODEL_CATALOG_FILENAME);
        if catalog.exists() {
            fs::remove_file(catalog).map_err(|error| AppError::Internal(error.to_string()))?;
        }
        Ok(())
    })();
    match update {
        Ok(()) => Ok(backup_path.display().to_string()),
        Err(error) => {
            restore_provider_backup(&backup_path.display().to_string())?;
            Err(error)
        }
    }
}

pub fn restore_provider_backup(backup_path: &str) -> Result<(), AppError> {
    let backup_path = Path::new(backup_path);
    for name in ["config.toml", "auth.json", MODEL_CATALOG_FILENAME] {
        let source = backup_path.join(name);
        if source.exists() {
            let bytes = fs::read(&source).map_err(|error| AppError::Backup(error.to_string()))?;
            atomic_write(&home().join(name), &bytes)
                .map_err(|error| AppError::Backup(error.to_string()))?;
        } else {
            let target = home().join(name);
            if target.exists() {
                fs::remove_file(target).map_err(|error| AppError::Backup(error.to_string()))?;
            }
        }
    }
    discard_provider_backup(backup_path.to_string_lossy().as_ref());
    Ok(())
}

pub fn discard_provider_backup(backup_path: &str) {
    let path = Path::new(backup_path);
    let _ = fs::remove_dir_all(path);
    if let Some(parent) = path.parent() {
        let _ = fs::remove_dir(parent);
    }
}

pub fn repair(provider: &str) -> Result<RepairResult, AppError> {
    crate::provider_sync::synchronize(&home(), provider)
}
#[allow(dead_code)]
pub fn restore_sessions_exact(
    provider: &str,
    thread_ids: &[String],
) -> Result<RepairResult, AppError> {
    crate::provider_sync::restore_exact(&home(), provider, thread_ids)
}
pub fn delete_sessions(ids: &[String]) -> anyhow::Result<usize> {
    let sessions = crate::session_index::rebuild()?;
    let rollout_paths = sessions
        .iter()
        .filter(|session| ids.contains(&session.id))
        .filter_map(|session| session.source_rollout.as_deref())
        .map(PathBuf::from)
        .collect::<std::collections::HashSet<_>>();
    let mut n = 0;
    for p in databases() {
        let mut db = Connection::open(p)?;
        let (table, id_column) = if has_threads(&db) {
            ("threads", "id")
        } else if table_columns(&db, "local_thread_catalog").contains(&"thread_id".into()) {
            ("local_thread_catalog", "thread_id")
        } else {
            continue;
        };
        let tx = db.transaction()?;
        for id in ids {
            n += tx.execute(
                &format!("DELETE FROM {table} WHERE {id_column}=?1"),
                params![id],
            )?
        }
        tx.commit()?
    }
    for path in rollout_paths {
        if path.exists() {
            fs::remove_file(path)?;
            n += 1;
        }
    }
    Ok(n)
}
pub fn export_sessions(ids: &[String], target: &Path) -> anyhow::Result<String> {
    let sessions = list_sessions(None)?;
    let mut text = String::from("# Codex 会话导出\n\n");
    for s in sessions
        .into_iter()
        .filter(|x| ids.is_empty() || ids.contains(&x.id))
    {
        text.push_str(&format!(
            "## {}\n\n- ID: `{}`\n- Provider: `{}`\n- 项目: `{}`\n- 更新时间: {}\n\n",
            if s.title.is_empty() {
                "未命名会话"
            } else {
                &s.title
            },
            s.id,
            s.provider,
            s.cwd,
            s.updated_at
        ))
    }
    fs::write(target, &text)?;
    Ok(target.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn current_catalog_query_uses_named_sort_alias() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE local_thread_catalog(
                thread_id TEXT PRIMARY KEY,
                display_title TEXT NOT NULL,
                model_provider TEXT NOT NULL,
                cwd TEXT NOT NULL,
                source_updated_at REAL NOT NULL
            );
            INSERT INTO local_thread_catalog VALUES('thread-1','Title','custom','C:/work',123.0);",
        )
        .unwrap();
        let columns = table_columns(&db, "local_thread_catalog");
        let sql = session_list_query("local_thread_catalog", "thread_id", &columns);
        let row: (String, String, i64) = db
            .query_row(&sql, [], |row| Ok((row.get(0)?, row.get(1)?, row.get(5)?)))
            .unwrap();
        assert_eq!(row, ("thread-1".into(), "Title".into(), 123));
        assert!(sql.contains("ORDER BY sort_updated"));
    }

    fn test_provider() -> ProviderProfile {
        ProviderProfile {
            id: "provider-1".into(),
            name: "Example Gateway".into(),
            protocol: ProviderProtocol::Responses,
            base_url: "https://example.test/v1".into(),
            models: vec!["model-b".into(), "model-a".into(), " ".into()],
            model_metadata: vec![],
            codex_chat_reasoning: None,
            headers: json!({"X-Provider": "provider", "X-Override": "provider"}),
            timeout_secs: 30,
            context_window: Some(64_000),
            auto_compact_threshold: Some(48_000),
            enabled: true,
            active: false,
            active_account_id: None,
            account_count: 1,
        }
    }

    fn test_account() -> ProviderAccount {
        ProviderAccount {
            id: "account-1".into(),
            provider_id: Some("provider-1".into()),
            name: "Account".into(),
            auth_kind: AccountAuthKind::ApiKey,
            api_key: Some("secret-key".into()),
            auth_json: None,
            headers: json!({"X-Account": "account", "X-Override": "account"}),
            active: false,
            email: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn provider_switch_rebuilds_minimal_config() {
        let config = build_managed_config_from(
            "",
            &test_provider(),
            &test_account(),
            "https://example.test/v1",
            "secret-key",
        )
        .unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert_eq!(
            parsed.get("model_provider").and_then(Item::as_str),
            Some(MANAGED_PROVIDER_ID)
        );
        assert!(parsed.get("model").is_none());
        assert_eq!(
            parsed.get("model_catalog_json").and_then(Item::as_str),
            Some(MODEL_CATALOG_FILENAME)
        );
        assert_eq!(
            parsed
                .get("model_providers")
                .and_then(Item::as_table)
                .and_then(|providers| providers.get(MANAGED_PROVIDER_ID))
                .and_then(Item::as_table)
                .and_then(|provider| provider.get("http_headers"))
                .and_then(Item::as_table)
                .and_then(|headers| headers.get("X-Override"))
                .and_then(Item::as_str),
            Some("account")
        );
        assert!(parsed.get("mcp_servers").is_none());
        assert!(parsed.get("features").is_none());
    }

    #[test]
    fn provider_switch_preserves_desktop_tool_configuration() {
        let existing = r#"
model = "old-model"
model_provider = "old-provider"
model_reasoning_effort = "high"

[model_providers.old-provider]
base_url = "https://old.invalid/v1"

[plugins."computer-use@openai-bundled"]
enabled = true

[mcp_servers.node_repl]
command = "node_repl.exe"

[desktop]
conversationDetailMode = "STEPS_COMMANDS"

[windows]
sandbox = "elevated"
"#;
        let config = build_managed_config_from(
            existing,
            &test_provider(),
            &test_account(),
            "https://example.test/v1",
            "secret-key",
        )
        .unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert_eq!(
            parsed.get("model_provider").and_then(Item::as_str),
            Some(MANAGED_PROVIDER_ID)
        );
        assert!(parsed.get("model").is_none());
        assert!(parsed["model_providers"].get("old-provider").is_none());
        assert_eq!(
            parsed["plugins"]["computer-use@openai-bundled"]["enabled"].as_bool(),
            Some(true)
        );
        assert_eq!(
            parsed["mcp_servers"]["node_repl"]["command"].as_str(),
            Some("node_repl.exe")
        );
        assert_eq!(parsed["windows"]["sandbox"].as_str(), Some("elevated"));
    }

    #[test]
    fn official_switch_merges_snapshot_without_losing_current_tools() {
        let current = r#"
model_provider = "custom"
[model_providers.custom]
base_url = "http://127.0.0.1:1234/v1"
[plugins."computer-use@openai-bundled"]
enabled = true
"#;
        let snapshot = r#"
model = "official-model"
[desktop]
localeOverride = "zh-CN"
[plugins."computer-use@openai-bundled"]
enabled = false
"#;
        let config = build_official_config(current, Some(snapshot)).unwrap();
        let parsed = config.parse::<DocumentMut>().unwrap();
        assert!(parsed.get("model").is_none());
        assert!(parsed.get("model_provider").is_none());
        assert!(parsed.get("model_providers").is_none());
        assert_eq!(parsed["desktop"]["localeOverride"].as_str(), Some("zh-CN"));
        assert_eq!(
            parsed["plugins"]["computer-use@openai-bundled"]["enabled"].as_bool(),
            Some(true)
        );
    }

    #[test]
    fn provider_switch_rejects_invalid_existing_config() {
        let error = build_managed_config_from(
            "[broken",
            &test_provider(),
            &test_account(),
            "https://example.test/v1",
            "secret-key",
        )
        .unwrap_err();
        assert!(matches!(error, AppError::InvalidConfig(_)));
    }

    #[test]
    fn model_catalog_preserves_selected_model_order_and_deduplicates() {
        let mut provider = test_provider();
        provider.model_metadata = vec![FetchedModel {
            id: "model-b".into(),
            owned_by: Some("upstream".into()),
            metadata: serde_json::from_value(json!({
                "context_window": 96_000,
                "default_reasoning_level": "high",
                "supported_reasoning_levels": [{"effort": "high"}],
                "input_modalities": ["text", "image"],
                "base_instructions": "upstream instructions must not replace Codex",
                "apply_patch_tool_type": "disabled"
            }))
            .unwrap(),
        }];
        let template = json!({
            "slug": "gpt-5.5",
            "base_instructions": "full Codex instructions",
            "model_messages": {"instructions_template": "Codex agent template"},
            "apply_patch_tool_type": "freeform",
            "tool_mode": "default",
            "shell_type": "shell_command",
            "supports_parallel_tool_calls": true
        });
        let catalog =
            build_model_catalog_from_template(&provider, &template, CatalogProfile::ProxyChat);
        let models = catalog["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], "model-b");
        assert_eq!(models[1]["slug"], "model-a");
        assert_eq!(models[0]["context_window"], 96_000);
        assert_eq!(models[0]["max_context_window"], 96_000);
        assert_eq!(models[0]["base_instructions"], "full Codex instructions");
        assert_eq!(models[0]["tool_mode"], "default");
        assert_eq!(models[0]["apply_patch_tool_type"], "freeform");
        assert_eq!(models[0]["supports_parallel_tool_calls"], true);
        assert!(models[0].get("default_reasoning_level").is_none());
        assert!(models[0].get("supported_reasoning_levels").is_none());
        assert!(models[0].get("input_modalities").is_none());
    }

    #[test]
    fn native_catalog_uses_clean_template_and_only_safe_upstream_metadata() {
        let mut provider = test_provider();
        provider.model_metadata = vec![FetchedModel {
            id: "model-b".into(),
            owned_by: None,
            metadata: serde_json::from_value(json!({
                "context_window": 120_000,
                "supports_parallel_tool_calls": true,
                "input_modalities": ["text", "image"],
                "base_instructions": "upstream native instructions",
                "apply_patch_tool_type": "freeform",
                "model_messages": {"instructions_template": "unsafe"},
                "default_reasoning_level": "invented"
            }))
            .unwrap(),
        }];
        let template = json!({
            "slug": "native-responses-template",
            "base_instructions": "full Codex instructions",
            "shell_type": "shell_command",
            "apply_patch_tool_type": "freeform",
            "model_messages": {"instructions_template": "unsafe"},
            "supports_parallel_tool_calls": false
        });
        let catalog = build_model_catalog_from_template(
            &provider,
            &template,
            CatalogProfile::NativeResponses,
        );
        let model = &catalog["models"][0];
        assert_eq!(model["slug"], "model-b");
        assert_eq!(model["context_window"], 120_000);
        assert_eq!(model["base_instructions"], "upstream native instructions");
        assert_eq!(model["supports_parallel_tool_calls"], true);
        assert_eq!(model["input_modalities"], json!(["text", "image"]));
        assert!(model.get("apply_patch_tool_type").is_none());
        assert!(model.get("model_messages").is_none());
        assert!(model.get("default_reasoning_level").is_none());
    }

    #[test]
    fn chat_template_requires_exact_complete_codex_agent_entry() {
        let catalog = json!({"models": [
            {"slug": "gpt-5.5", "base_instructions": "missing tool fields"},
            {
                "slug": "another-model",
                "base_instructions": "complete but wrong model",
                "model_messages": {},
                "apply_patch_tool_type": "freeform"
            }
        ]});
        assert!(find_model_template(&catalog).is_none());

        let complete = json!({
            "slug": "gpt-5.5",
            "base_instructions": "complete Codex agent instructions",
            "model_messages": {},
            "apply_patch_tool_type": "freeform"
        });
        assert!(find_model_template(&json!({"models": [complete]})).is_some());
    }

    #[test]
    fn third_party_auth_is_rebuilt_with_only_active_key() {
        assert_eq!(
            build_managed_auth("secret-key"),
            json!({"OPENAI_API_KEY": "secret-key"})
        );
    }
}
