use crate::models::*;
use crate::protocol_proxy::ProxyEndpoint;
use chrono::Utc;
use rusqlite::{Connection, params};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item, Table, value};
use walkdir::WalkDir;

pub const MANAGED_PROVIDER_ID: &str = "custom";

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
    let root = home()
        .join("backups_state")
        .join("codex-tools")
        .join(format!(
            "{}-{label}-{}",
            Utc::now().format("%Y%m%d%H%M%S"),
            uuid::Uuid::new_v4().simple()
        ));
    fs::create_dir_all(&root)?;
    for name in ["config.toml", "auth.json", ".codex-global-state.json"] {
        let src = home().join(name);
        if src.exists() {
            fs::copy(src, root.join(name))?;
        }
    }
    for db in databases() {
        let dest = root.join("db").join(db.file_name().unwrap());
        fs::create_dir_all(dest.parent().unwrap())?;
        fs::copy(db, dest)?;
    }
    Ok(root)
}
#[allow(dead_code)]
pub fn apply_provider(p: &ProviderProfile, account: &ProviderAccount) -> Result<String, AppError> {
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
            "名称、URL、Key 和模型不能为空".into(),
        ));
    }
    if p.protocol == ProviderProtocol::ChatCompletions {
        return Err(AppError::InvalidConfig(
            "Chat Completions 需要本地协议代理；当前构建尚未启动代理".into(),
        ));
    }
    let b = backup("provider").map_err(|e| AppError::Backup(e.to_string()))?;
    let path = home().join("config.toml");
    let old = fs::read_to_string(&path).unwrap_or_default();
    let mut doc = old
        .parse::<DocumentMut>()
        .map_err(|e| AppError::InvalidConfig(e.to_string()))?;
    let id = MANAGED_PROVIDER_ID;
    doc["model_provider"] = value(id);
    doc["model"] = value(&p.default_model);
    if !doc.as_table().contains_key("model_providers") {
        doc["model_providers"] = Item::Table(Table::new())
    }
    let providers = doc["model_providers"]
        .as_table_mut()
        .ok_or_else(|| AppError::InvalidConfig("model_providers 必须是 table".into()))?;
    let mut t = Table::new();
    t["name"] = value(&p.name);
    t["wire_api"] = value("responses");
    t["base_url"] = value(p.base_url.trim_end_matches('/'));
    t["requires_openai_auth"] = value(true);
    t["experimental_bearer_token"] = value(account.api_key.as_deref().unwrap_or_default());
    providers[id] = Item::Table(t);
    atomic_write(&path, doc.to_string().as_bytes())?;
    Ok(b.display().to_string())
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
    let path = home().join("config.toml");
    let old = fs::read_to_string(&path).unwrap_or_default();
    let mut doc = old
        .parse::<DocumentMut>()
        .map_err(|e| AppError::InvalidConfig(e.to_string()))?;
    let id = MANAGED_PROVIDER_ID;
    doc["model_provider"] = value(id);
    doc["model"] = value(&p.default_model);
    if !doc.as_table().contains_key("model_providers") {
        doc["model_providers"] = Item::Table(Table::new());
    }
    let providers = doc["model_providers"]
        .as_table_mut()
        .ok_or_else(|| AppError::InvalidConfig("model_providers must be a table".into()))?;
    let mut table = Table::new();
    table["name"] = value(&p.name);
    table["wire_api"] = value("responses");
    table["requires_openai_auth"] = value(true);
    let token = match p.protocol {
        ProviderProtocol::Responses => {
            table["base_url"] = value(p.base_url.trim_end_matches('/'));
            account.api_key.clone().unwrap_or_default()
        }
        ProviderProtocol::ChatCompletions => {
            let endpoint = proxy.ok_or_else(|| AppError::Proxy("proxy is not running".into()))?;
            table["base_url"] = value(endpoint.base_url.trim_end_matches('/'));
            endpoint.token.clone()
        }
    };
    table["experimental_bearer_token"] = value(&token);
    providers[id] = Item::Table(table);
    let update = (|| -> Result<(), AppError> {
        atomic_write(&path, doc.to_string().as_bytes())?;
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

pub fn capture_official_auth() -> Result<serde_json::Value, AppError> {
    let path = home().join("auth.json");
    let text = fs::read_to_string(path).map_err(|_| AppError::OfficialAuthMissing)?;
    serde_json::from_str(&text).map_err(|error| AppError::InvalidConfig(error.to_string()))
}

pub fn capture_official_config() -> String {
    fs::read_to_string(home().join("config.toml")).unwrap_or_default()
}

pub fn is_official_mode() -> bool {
    let config = capture_official_config();
    let Ok(doc) = config.parse::<DocumentMut>() else {
        return false;
    };
    doc.get("model_provider")
        .and_then(Item::as_str)
        .is_none_or(|provider| {
            provider != MANAGED_PROVIDER_ID && !provider.starts_with("codex_tools_")
        })
}

pub fn restore_official_snapshot(
    auth: &serde_json::Value,
    config_snapshot: Option<&str>,
) -> Result<String, AppError> {
    let backup_path =
        backup("official-account").map_err(|error| AppError::Backup(error.to_string()))?;
    let config_path = home().join("config.toml");
    let config = config_snapshot.unwrap_or_default();
    let update = (|| -> Result<(), AppError> {
        if config.is_empty() {
            let current = fs::read_to_string(&config_path).unwrap_or_default();
            let mut doc = current
                .parse::<DocumentMut>()
                .map_err(|error| AppError::InvalidConfig(error.to_string()))?;
            doc.as_table_mut().remove("model_provider");
            doc.as_table_mut().remove("model");
            if let Some(providers) = doc.get_mut("model_providers").and_then(Item::as_table_mut) {
                providers.remove(MANAGED_PROVIDER_ID);
            }
            atomic_write(&config_path, doc.to_string().as_bytes())?;
        } else {
            config
                .parse::<DocumentMut>()
                .map_err(|error| AppError::InvalidConfig(error.to_string()))?;
            atomic_write(&config_path, config.as_bytes())?;
        }
        atomic_write(
            &home().join("auth.json"),
            &serde_json::to_vec_pretty(auth)
                .map_err(|error| AppError::Internal(error.to_string()))?,
        )?;
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

pub fn restore_official_account(account: &ProviderAccount) -> Result<String, AppError> {
    let auth = account
        .auth_json
        .as_ref()
        .ok_or(AppError::OfficialAuthMissing)?;
    let backup_path =
        backup("official-account").map_err(|error| AppError::Backup(error.to_string()))?;
    let config_path = home().join("config.toml");
    let original = fs::read_to_string(&config_path).unwrap_or_default();
    let mut doc = original
        .parse::<DocumentMut>()
        .map_err(|error| AppError::InvalidConfig(error.to_string()))?;
    doc["model_provider"] = value(MANAGED_PROVIDER_ID);
    if !doc.as_table().contains_key("model_providers") {
        doc["model_providers"] = Item::Table(Table::new());
    }
    let providers = doc["model_providers"]
        .as_table_mut()
        .ok_or_else(|| AppError::InvalidConfig("model_providers 必须是 table".into()))?;
    let mut official = Table::new();
    official["name"] = value("OpenAI");
    official["requires_openai_auth"] = value(true);
    official["supports_websockets"] = value(true);
    official["wire_api"] = value("responses");
    providers[MANAGED_PROVIDER_ID] = Item::Table(official);
    let update = (|| -> Result<(), AppError> {
        atomic_write(&config_path, doc.to_string().as_bytes())?;
        atomic_write(
            &home().join("auth.json"),
            &serde_json::to_vec_pretty(auth)
                .map_err(|error| AppError::Internal(error.to_string()))?,
        )?;
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
    for name in ["config.toml", "auth.json"] {
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
    Ok(())
}

pub fn repair(provider: &str) -> Result<RepairResult, AppError> {
    crate::provider_sync::synchronize(&home(), provider)
}
pub fn delete_sessions(ids: &[String]) -> anyhow::Result<usize> {
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
}
