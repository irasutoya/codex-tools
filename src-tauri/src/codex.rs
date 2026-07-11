use crate::models::*;
use crate::protocol_proxy::ProxyEndpoint;
use rusqlite::{Connection, params};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table, value};
use walkdir::WalkDir;

pub const MANAGED_PROVIDER_ID: &str = "custom";
pub const MODEL_CATALOG_FILENAME: &str = "codex-tools-model-catalog.json";

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
                if !known {
                    warnings.push(format!("未知 schema：{}", p.display()))
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
        || p.default_model.trim().is_empty()
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
    let catalog = (!p.models.is_empty()).then(|| build_model_catalog(p));
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
    let mut doc = DocumentMut::new();
    doc["model_provider"] = value(MANAGED_PROVIDER_ID);
    doc["model"] = value(provider.default_model.trim());
    if !provider.models.is_empty() {
        doc["model_catalog_json"] = value(MODEL_CATALOG_FILENAME);
    }
    if let Some(context_window) = provider.context_window.filter(|value| *value > 0) {
        doc["model_context_window"] = value(
            i64::try_from(context_window)
                .map_err(|_| AppError::InvalidConfig("上下文窗口数值过大".into()))?,
        );
    }
    if let Some(threshold) = provider.auto_compact_threshold.filter(|value| *value > 0) {
        doc["model_auto_compact_token_limit"] = value(
            i64::try_from(threshold)
                .map_err(|_| AppError::InvalidConfig("自动压缩阈值数值过大".into()))?,
        );
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

fn build_model_catalog(provider: &ProviderProfile) -> serde_json::Value {
    let context_window = provider
        .context_window
        .filter(|value| *value > 0)
        .unwrap_or(128_000);
    let models = provider.models.iter().map(|model| model.trim().to_string());
    let (default_reasoning_level, supported_reasoning_levels) = match provider.protocol {
        ProviderProtocol::ChatCompletions => (
            "medium",
            serde_json::json!([
                {"effort":"low","description":"Fast responses with lighter reasoning"},
                {"effort":"medium","description":"Balances speed and reasoning depth"},
                {"effort":"high","description":"Greater reasoning depth for complex problems"},
                {"effort":"xhigh","description":"Extra high reasoning depth"}
            ]),
        ),
        ProviderProtocol::Responses => (
            "high",
            serde_json::json!([
                {"effort":"none","description":"Disable Thinking"},
                {"effort":"high","description":"Enable Thinking"}
            ]),
        ),
    };
    let mut seen = std::collections::HashSet::new();
    let models = models
        .filter(|model| !model.is_empty() && seen.insert(model.clone()))
        .enumerate()
        .map(|(priority, model)| {
            serde_json::json!({
                "slug": model,
                "display_name": model,
                "description": format!("{} · {}", provider.name.trim(), model),
                "base_instructions": "You are Codex, a coding agent. You and the user share the same workspace and collaborate to achieve the user's goals.",
                "default_reasoning_level": default_reasoning_level,
                "supported_reasoning_levels": supported_reasoning_levels,
                "shell_type": "shell_command",
                "visibility": "list",
                "supported_in_api": true,
                "priority": priority,
                "supports_reasoning_summaries": true,
                "default_reasoning_summary": "none",
                "support_verbosity": false,
                "truncation_policy": {
                    "mode": "bytes",
                    "limit": 10000
                },
                "supports_parallel_tool_calls": false,
                "supports_image_detail_original": false,
                "context_window": context_window,
                "max_context_window": context_window,
                "effective_context_window_percent": 95,
                "experimental_supported_tools": [],
                "input_modalities": ["text"],
                "supports_search_tool": false
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "models": models })
}

pub fn restore_official_snapshot(
    auth: &serde_json::Value,
    _config_snapshot: Option<&str>,
) -> Result<String, AppError> {
    let backup_path =
        backup("official-account").map_err(|error| AppError::Backup(error.to_string()))?;
    let update = (|| -> Result<(), AppError> {
        atomic_write(&home().join("config.toml"), b"")?;
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
            default_model: "model-a".into(),
            models: vec!["model-b".into(), "model-a".into(), " ".into()],
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
        let config = build_managed_config(
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
    fn model_catalog_preserves_selected_model_order_and_deduplicates() {
        let catalog = build_model_catalog(&test_provider());
        let models = catalog["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0]["slug"], "model-b");
        assert_eq!(models[1]["slug"], "model-a");
        assert_eq!(models[0]["context_window"], 64_000);
        assert!(
            models[0]["base_instructions"]
                .as_str()
                .is_some_and(|v| !v.is_empty())
        );
    }

    #[test]
    fn third_party_auth_is_rebuilt_with_only_active_key() {
        assert_eq!(
            build_managed_auth("secret-key"),
            json!({"OPENAI_API_KEY": "secret-key"})
        );
    }
}
