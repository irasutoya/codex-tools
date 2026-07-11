use crate::models::{AppError, RepairResult};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

const BACKUP_LIMIT: usize = 10;

#[derive(Clone)]
struct RolloutChange {
    path: PathBuf,
    original: String,
    updated: String,
    mtime: Option<SystemTime>,
    thread_id: String,
    cwd: Option<String>,
    has_user_event: bool,
    changed: bool,
}

#[derive(Default)]
struct UpdateCounts {
    provider: usize,
    user_event: usize,
    cwd: usize,
}

#[derive(Clone, Copy)]
enum DatabaseKind {
    LegacyThreads,
    LocalThreadCatalog,
}

#[derive(Clone)]
struct DatabaseTarget {
    path: PathBuf,
    kind: DatabaseKind,
}

impl UpdateCounts {
    fn total(&self) -> usize {
        self.provider + self.user_event + self.cwd
    }
}

pub fn synchronize(home: &Path, provider: &str) -> Result<RepairResult, AppError> {
    validate_provider(provider)?;
    if !home.exists() {
        return Err(AppError::Internal(format!(
            "Codex 目录不存在：{}",
            home.display()
        )));
    }
    let lock = SyncLock::acquire(&home.join("tmp/codex-tools-provider-sync.lock"))?;
    let source_providers = known_source_providers(home, provider);
    let changes = collect_rollouts(home, provider, &source_providers)?;
    let projectless = projectless_threads(&home.join(".codex-global-state.json"))?;
    let user_threads = changes
        .iter()
        .filter(|item| item.has_user_event)
        .map(|item| item.thread_id.clone())
        .collect::<HashSet<_>>();
    let cwd_by_thread = changes
        .iter()
        .filter(|item| !projectless.contains(&item.thread_id))
        .filter_map(|item| Some((item.thread_id.clone(), item.cwd.clone()?)))
        .collect::<HashMap<_, _>>();
    let discovered_dbs = database_paths(home);
    let (dbs, mut database_warnings) = classify_databases(&discovered_dbs)?;
    let db_paths = dbs.iter().map(|db| db.path.clone()).collect::<Vec<_>>();
    let backup = create_backup(home, provider, &changes, &db_paths)?;
    let global_path = home.join(".codex-global-state.json");
    let original_global = fs::read(&global_path).ok();
    let applied = apply_rollouts(&changes)?;
    let result = (|| -> anyhow::Result<(UpdateCounts, usize)> {
        let counts = update_databases(
            &dbs,
            provider,
            &source_providers,
            &user_threads,
            &cwd_by_thread,
        )?;
        let globals = normalize_global_state(&global_path)?;
        prune_backups(home)?;
        Ok((counts, globals))
    })();
    drop(lock);
    match result {
        Ok((counts, global_updates)) => {
            let mut warnings = Vec::new();
            warnings.append(&mut database_warnings);
            let encrypted = changes
                .iter()
                .filter(|item| item.original.contains("encrypted_content"))
                .count();
            if encrypted > 0 {
                warnings.push(format!(
                    "{encrypted} 个历史会话包含 encrypted_content，切换供应商后继续对话可能不兼容"
                ));
            }
            if global_updates > 0 {
                warnings.push(format!("规范化了 {global_updates} 个工作区状态字段"));
            }
            Ok(RepairResult {
                backup_path: backup.display().to_string(),
                databases_repaired: dbs.len(),
                rows_updated: counts.total(),
                warnings,
            })
        }
        Err(error) => {
            restore_rollouts(&applied);
            restore_file(&global_path, original_global.as_deref());
            match restore_database_backup(home, &backup, &db_paths) {
                Ok(()) => Err(AppError::Internal(format!(
                    "修复失败，已回滚会话文件、全局状态和数据库：{error}"
                ))),
                Err(restore_error) => Err(AppError::Backup(format!(
                    "修复失败：{error}；数据库回滚也失败：{restore_error}。备份位于 {}",
                    backup.display()
                ))),
            }
        }
    }
}

pub fn restore_exact(
    home: &Path,
    provider: &str,
    thread_ids: &[String],
) -> Result<RepairResult, AppError> {
    validate_provider(provider)?;
    if thread_ids.is_empty() {
        return Ok(RepairResult {
            backup_path: String::new(),
            databases_repaired: 0,
            rows_updated: 0,
            warnings: vec!["没有需要恢复归属的官方历史会话。".into()],
        });
    }
    let wanted = thread_ids.iter().cloned().collect::<HashSet<_>>();
    let lock = SyncLock::acquire(&home.join("tmp/codex-tools-provider-sync.lock"))?;
    let sources = HashSet::from(["custom".to_string()]);
    let mut changes = collect_rollouts(home, provider, &sources)?;
    changes.retain(|change| wanted.contains(&change.thread_id));
    let discovered = database_paths(home);
    let (dbs, warnings) = classify_databases(&discovered)?;
    let db_paths = dbs.iter().map(|db| db.path.clone()).collect::<Vec<_>>();
    let backup = create_backup(home, provider, &changes, &db_paths)?;
    let applied = apply_rollouts(&changes)?;
    let result = update_database_threads_exact(&dbs, provider, &wanted);
    drop(lock);
    match result {
        Ok(rows) => Ok(RepairResult {
            backup_path: backup.display().to_string(),
            databases_repaired: dbs.len(),
            rows_updated: rows,
            warnings,
        }),
        Err(error) => {
            restore_rollouts(&applied);
            match restore_database_backup(home, &backup, &db_paths) {
                Ok(()) => Err(AppError::Internal(format!(
                    "官方历史恢复失败，已按迁移账本回滚：{error}"
                ))),
                Err(rollback) => Err(AppError::Backup(format!(
                    "官方历史恢复失败：{error}；数据库回滚失败：{rollback}。备份位于 {}",
                    backup.display()
                ))),
            }
        }
    }
}

fn update_database_threads_exact(
    targets: &[DatabaseTarget],
    provider: &str,
    thread_ids: &HashSet<String>,
) -> anyhow::Result<usize> {
    let mut updated = 0;
    for target in targets {
        let mut db = Connection::open(&target.path)?;
        db.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")?;
        let (table, id_column) = match target.kind {
            DatabaseKind::LegacyThreads => ("threads", "id"),
            DatabaseKind::LocalThreadCatalog => ("local_thread_catalog", "thread_id"),
        };
        let tx = db.transaction()?;
        for chunk in thread_ids.iter().collect::<Vec<_>>().chunks(500) {
            let placeholders = (1..=chunk.len())
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "UPDATE {table} SET model_provider='{}' WHERE model_provider='custom' AND {id_column} IN ({placeholders})",
                provider.replace('\'', "''")
            );
            updated += tx.execute(
                &sql,
                rusqlite::params_from_iter(chunk.iter().map(|id| id.as_str())),
            )?;
        }
        tx.commit()?;
    }
    Ok(updated)
}

fn validate_provider(provider: &str) -> Result<(), AppError> {
    if provider.is_empty()
        || !provider
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(AppError::InvalidConfig(format!(
            "非法 Provider ID：{provider:?}"
        )));
    }
    Ok(())
}

fn collect_rollouts(
    home: &Path,
    provider: &str,
    source_providers: &HashSet<String>,
) -> Result<Vec<RolloutChange>, AppError> {
    let mut result = Vec::new();
    for root in [home.join("sessions"), home.join("archived_sessions")] {
        if !root.exists() {
            continue;
        }
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            let is_rollout = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rollout-") && name.ends_with(".jsonl"));
            if !is_rollout {
                continue;
            }
            let original = match fs::read_to_string(path) {
                Ok(value) => value,
                Err(error) if is_locked(&error) => continue,
                Err(error) => return Err(AppError::Internal(error.to_string())),
            };
            if let Some(change) = rewrite_rollout(path, original, provider, source_providers)? {
                result.push(change);
            }
        }
    }
    result.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(result)
}

fn rewrite_rollout(
    path: &Path,
    original: String,
    provider: &str,
    source_providers: &HashSet<String>,
) -> Result<Option<RolloutChange>, AppError> {
    let mut updated = String::new();
    let mut thread_id = None;
    let mut cwd = None;
    let mut changed = false;
    for segment in original.split_inclusive('\n') {
        let (line, ending) = split_ending(segment);
        let mut next = line.to_string();
        if let Ok(mut record) = serde_json::from_str::<Value>(line)
            && record.get("type").and_then(Value::as_str) == Some("session_meta")
            && let Some(payload) = record.get_mut("payload").and_then(Value::as_object_mut)
        {
            thread_id =
                thread_id.or_else(|| payload.get("id").and_then(Value::as_str).map(str::to_owned));
            cwd = cwd.or_else(|| {
                payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .and_then(normalize_path)
            });
            let current = payload.get("model_provider").and_then(Value::as_str);
            if current != Some(provider)
                && current.is_some_and(|value| source_providers.contains(value))
            {
                payload.insert("model_provider".into(), json!(provider));
                next = serde_json::to_string(&record)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                changed = true;
            }
        }
        updated.push_str(&next);
        updated.push_str(ending);
    }
    let Some(thread_id) = thread_id else {
        return Ok(None);
    };
    let mtime = fs::metadata(path).and_then(|m| m.modified()).ok();
    Ok(Some(RolloutChange {
        path: path.to_path_buf(),
        original: original.clone(),
        updated,
        mtime,
        thread_id,
        cwd,
        has_user_event: original.contains("\"user_message\"")
            || original.contains("\"user_input\""),
        changed,
    }))
}

fn apply_rollouts(changes: &[RolloutChange]) -> Result<Vec<RolloutChange>, AppError> {
    let mut applied = Vec::new();
    for item in changes.iter().filter(|item| item.changed) {
        if let Err(error) = fs::write(&item.path, &item.updated) {
            if is_locked(&error) {
                continue;
            }
            restore_rollouts(&applied);
            return Err(AppError::Internal(error.to_string()));
        }
        restore_mtime(&item.path, item.mtime);
        applied.push(item.clone());
    }
    Ok(applied)
}

fn restore_rollouts(changes: &[RolloutChange]) {
    for item in changes {
        let _ = fs::write(&item.path, &item.original);
        restore_mtime(&item.path, item.mtime);
    }
}

fn classify_databases(paths: &[PathBuf]) -> Result<(Vec<DatabaseTarget>, Vec<String>), AppError> {
    let mut targets = Vec::new();
    let mut warnings = Vec::new();
    for path in paths {
        let db = open_read_only(path)?;
        let quick: String = db
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .map_err(|e| AppError::Internal(e.to_string()))?;
        if quick != "ok" {
            let detail: String = db
                .query_row("PRAGMA integrity_check", [], |row| row.get(0))
                .unwrap_or(quick);
            return Err(AppError::Internal(format!(
                "数据库完整性检查失败 {}：{detail}",
                path.display()
            )));
        }
        if table_exists(&db, "threads")? {
            let columns = table_columns(&db, "threads")?;
            if !columns.contains("id") || !columns.contains("model_provider") {
                return Err(AppError::UnknownSchema(path.display().to_string()));
            }
            targets.push(DatabaseTarget {
                path: path.clone(),
                kind: DatabaseKind::LegacyThreads,
            });
        } else if table_exists(&db, "local_thread_catalog")? {
            let columns = table_columns(&db, "local_thread_catalog")?;
            if !columns.contains("thread_id") || !columns.contains("model_provider") {
                return Err(AppError::UnknownSchema(path.display().to_string()));
            }
            targets.push(DatabaseTarget {
                path: path.clone(),
                kind: DatabaseKind::LocalThreadCatalog,
            });
        } else {
            warnings.push(format!(
                "已跳过不包含会话目录的辅助数据库：{}",
                path.display()
            ));
        }
    }
    Ok((targets, warnings))
}

fn update_databases(
    targets: &[DatabaseTarget],
    provider: &str,
    source_providers: &HashSet<String>,
    users: &HashSet<String>,
    cwd: &HashMap<String, String>,
) -> anyhow::Result<UpdateCounts> {
    let mut total = UpdateCounts::default();
    for target in targets {
        let mut db = Connection::open(&target.path)?;
        db.execute_batch("PRAGMA wal_checkpoint(PASSIVE);")?;
        let (table, id_column) = match target.kind {
            DatabaseKind::LegacyThreads => ("threads", "id"),
            DatabaseKind::LocalThreadCatalog => ("local_thread_catalog", "thread_id"),
        };
        let columns = table_columns_anyhow(&db, table)?;
        let tx = db.transaction()?;
        for source in source_providers {
            let sql = format!("UPDATE {table} SET model_provider=?1 WHERE model_provider=?2");
            total.provider += tx.execute(&sql, (provider, source))?;
        }
        if columns.contains("has_user_event") {
            for id in users {
                let sql = format!(
                    "UPDATE {table} SET has_user_event=1 WHERE {id_column}=?1 AND COALESCE(has_user_event,0)<>1"
                );
                total.user_event += tx.execute(&sql, [id])?;
            }
        }
        if columns.contains("cwd") {
            for (id, path) in cwd {
                let sql = format!(
                    "UPDATE {table} SET cwd=?1 WHERE {id_column}=?2 AND COALESCE(cwd,'')<>?1"
                );
                total.cwd += tx.execute(&sql, (path, id))?;
            }
        }
        tx.commit()?;
    }
    Ok(total)
}

fn known_source_providers(home: &Path, target: &str) -> HashSet<String> {
    let mut providers = HashSet::from(["openai".to_string()]);
    let config = fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    if let Ok(doc) = config.parse::<toml_edit::DocumentMut>() {
        if let Some(active) = doc.get("model_provider").and_then(toml_edit::Item::as_str)
            && (active == target || active == "custom" || active.starts_with("codex_tools_"))
        {
            providers.insert(active.to_string());
        }
        if let Some(tables) = doc
            .get("model_providers")
            .and_then(toml_edit::Item::as_table)
        {
            providers.extend(
                tables
                    .iter()
                    .map(|(name, _)| name.to_string())
                    .filter(|name| {
                        name == target || name == "custom" || name.starts_with("codex_tools_")
                    }),
            );
        }
    }
    for rollout in [home.join("sessions"), home.join("archived_sessions")] {
        if !rollout.exists() {
            continue;
        }
        for entry in WalkDir::new(rollout).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            {
                continue;
            }
            if let Ok(text) = fs::read_to_string(path) {
                for line in text.lines().filter(|line| line.contains("model_provider")) {
                    if let Ok(value) = serde_json::from_str::<Value>(line)
                        && value.get("type").and_then(Value::as_str) == Some("session_meta")
                        && let Some(id) = value
                            .pointer("/payload/model_provider")
                            .and_then(Value::as_str)
                        && (id == target || id == "custom" || id.starts_with("codex_tools_"))
                    {
                        providers.insert(id.to_string());
                    }
                }
            }
        }
    }
    providers.remove(target);
    providers
}

fn create_backup(
    home: &Path,
    provider: &str,
    changes: &[RolloutChange],
    dbs: &[PathBuf],
) -> Result<PathBuf, AppError> {
    let root = home.join("backups_state/codex-tools-provider-sync");
    let dir = root.join(format!(
        "{}-{}",
        chrono::Local::now().format("%Y%m%d%H%M%S"),
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&dir).map_err(|e| AppError::Backup(e.to_string()))?;
    for name in [
        "config.toml",
        ".codex-global-state.json",
        ".codex-global-state.json.bak",
    ] {
        let source = home.join(name);
        if source.exists() {
            fs::copy(&source, dir.join(name)).map_err(|e| AppError::Backup(e.to_string()))?;
        }
    }
    let mut db_files = Vec::new();
    for db in dbs {
        for source in sqlite_files(db) {
            if !source.exists() {
                continue;
            }
            let relative = backup_relative_path(home, &source);
            let target = dir.join("db").join(&relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| AppError::Backup(e.to_string()))?;
            }
            fs::copy(&source, &target).map_err(|e| AppError::Backup(e.to_string()))?;
            db_files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    let manifest = changes
        .iter()
        .filter(|item| item.changed)
        .map(|item| json!({"path":item.path,"threadId":item.thread_id}))
        .collect::<Vec<_>>();
    fs::write(dir.join("metadata.json"), serde_json::to_vec_pretty(&json!({"version":1,"managedBy":"Codex Tools provider sync","provider":provider,"dbFiles":db_files,"rollouts":manifest})).map_err(|e|AppError::Backup(e.to_string()))?).map_err(|e|AppError::Backup(e.to_string()))?;
    Ok(dir)
}

fn restore_database_backup(home: &Path, backup: &Path, dbs: &[PathBuf]) -> anyhow::Result<()> {
    for db in dbs {
        for destination in sqlite_files(db) {
            let relative = backup_relative_path(home, &destination);
            let source = backup.join("db").join(relative);
            if source.exists() {
                let bytes = fs::read(&source)?;
                atomic_write(&destination, &bytes)?;
            } else if destination.exists() {
                fs::remove_file(&destination)?;
            }
        }
    }
    Ok(())
}

fn backup_relative_path(home: &Path, source: &Path) -> PathBuf {
    if let Ok(relative) = source.strip_prefix(home) {
        return relative.to_path_buf();
    }
    let identity = source
        .to_string_lossy()
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    PathBuf::from("external")
        .join(format!("{identity:016x}"))
        .join(source.file_name().unwrap_or_default())
}

fn normalize_global_state(path: &Path) -> anyhow::Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let original = fs::read_to_string(path)?;
    let mut state = serde_json::from_str::<Value>(&original)?
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut changed = 0;
    for key in [
        "electron-saved-workspace-roots",
        "project-order",
        "active-workspace-roots",
    ] {
        if let Some(value) = state.get(key).cloned() {
            let was_array = value.is_array();
            let normalized = dedupe_paths(value.clone());
            let next = if was_array {
                json!(normalized)
            } else {
                normalized.first().map_or(value, |v| json!(v))
            };
            if state.get(key) != Some(&next) {
                state.insert(key.into(), next);
                changed += 1;
            }
        }
    }
    for key in ["electron-workspace-root-labels"] {
        if let Some(object) = state.get(key).and_then(Value::as_object).cloned() {
            let next = normalize_object_keys(object);
            if state.get(key) != Some(&Value::Object(next.clone())) {
                state.insert(key.into(), Value::Object(next));
                changed += 1;
            }
        }
    }
    if let Some(mut targets) = state
        .get("open-in-target-preferences")
        .and_then(Value::as_object)
        .cloned()
    {
        if let Some(paths) = targets.get("perPath").and_then(Value::as_object).cloned() {
            targets.insert(
                "perPath".into(),
                Value::Object(normalize_object_keys(paths)),
            );
        }
        let next = Value::Object(targets);
        if state.get("open-in-target-preferences") != Some(&next) {
            state.insert("open-in-target-preferences".into(), next);
            changed += 1;
        }
    }
    if changed > 0 {
        let text = serde_json::to_vec_pretty(&Value::Object(state))?;
        atomic_write(path, &text)?;
        fs::write(path.with_extension("json.bak"), &text)?;
    }
    Ok(changed)
}

fn projectless_threads(path: &Path) -> Result<HashSet<String>, AppError> {
    let value = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    Ok(value
        .as_ref()
        .and_then(|v| v.get("projectless-thread-ids"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect())
}

fn normalize_object_keys(value: Map<String, Value>) -> Map<String, Value> {
    value
        .into_iter()
        .map(|(k, v)| (normalize_path(&k).unwrap_or(k), v))
        .collect()
}
fn dedupe_paths(value: Value) -> Vec<String> {
    let items = if let Some(a) = value.as_array() {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    } else {
        value
            .as_str()
            .map(|s| vec![s.to_owned()])
            .unwrap_or_default()
    };
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter_map(|p| normalize_path(&p))
        .filter(|p| {
            seen.insert(
                p.replace('/', "\\")
                    .trim_end_matches('\\')
                    .to_ascii_lowercase(),
            )
        })
        .collect()
}
fn normalize_path(value: &str) -> Option<String> {
    let s = value.trim();
    if s.is_empty() {
        None
    } else if s.to_ascii_lowercase().starts_with(r"\\?\unc\") {
        Some(format!(r"\\{}", s[8..].replace('/', "\\")))
    } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
        Some(stripped.replace('\\', "/"))
    } else {
        Some(s.to_owned())
    }
}
fn database_paths(home: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    let old = home.join("state_5.sqlite");
    if old.exists() {
        v.push(old)
    }
    if let Ok(entries) = fs::read_dir(home.join("sqlite")) {
        v.extend(
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "db" || x == "sqlite")),
        )
    }
    if let Ok(value) = std::env::var("CODEX_SQLITE_HOME") {
        collect_sqlite_home(Path::new(&value), &mut v);
    }
    let config = fs::read_to_string(home.join("config.toml")).unwrap_or_default();
    if let Ok(doc) = config.parse::<toml_edit::DocumentMut>()
        && let Some(value) = doc.get("sqlite_home").and_then(toml_edit::Item::as_str)
    {
        collect_sqlite_home(Path::new(value), &mut v);
    }
    v.sort();
    v.dedup();
    v
}

fn collect_sqlite_home(path: &Path, output: &mut Vec<PathBuf>) {
    if path.is_file() {
        output.push(path.to_path_buf());
        return;
    }
    if let Ok(entries) = fs::read_dir(path) {
        output.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "db" || extension == "sqlite")
        }));
    }
}
fn sqlite_files(path: &Path) -> Vec<PathBuf> {
    let s = path.to_string_lossy();
    vec![
        path.to_path_buf(),
        PathBuf::from(format!("{s}-wal")),
        PathBuf::from(format!("{s}-shm")),
    ]
}
fn open_read_only(path: &Path) -> Result<Connection, AppError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| AppError::Internal(error.to_string()))
}

fn table_exists(db: &Connection, table: &str) -> Result<bool, AppError> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(|error| AppError::Internal(error.to_string()))
}

fn table_columns(db: &Connection, table: &str) -> Result<HashSet<String>, AppError> {
    table_columns_anyhow(db, table).map_err(|e| AppError::Internal(e.to_string()))
}
fn table_columns_anyhow(db: &Connection, table: &str) -> anyhow::Result<HashSet<String>> {
    let mut s = db.prepare(&format!("PRAGMA table_info({table})"))?;
    Ok(s.query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?)
}
fn split_ending(s: &str) -> (&str, &str) {
    if let Some(v) = s.strip_suffix("\r\n") {
        (v, "\r\n")
    } else if let Some(v) = s.strip_suffix('\n') {
        (v, "\n")
    } else {
        (s, "")
    }
}
fn restore_mtime(path: &Path, mtime: Option<SystemTime>) {
    if let (Some(time), Ok(file)) = (mtime, fs::File::options().write(true).open(path)) {
        let _ = file.set_times(fs::FileTimes::new().set_modified(time));
    }
}
fn is_locked(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(32 | 33))
}
fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("codex-tools");
    let temp = path.with_file_name(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let mut file = fs::File::create(&temp)?;
    file.write_all(data)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = replace_file(&temp, path) {
        let _ = fs::remove_file(&temp);
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
fn restore_file(path: &Path, data: Option<&[u8]>) {
    match data {
        Some(v) => {
            let _ = fs::write(path, v);
        }
        None => {
            let _ = fs::remove_file(path);
        }
    }
}
fn prune_backups(home: &Path) -> anyhow::Result<()> {
    let root = home.join("backups_state/codex-tools-provider-sync");
    if !root.exists() {
        return Ok(());
    }
    let mut dirs = fs::read_dir(root)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect::<Vec<_>>();
    dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    for p in dirs.into_iter().skip(BACKUP_LIMIT) {
        let _ = fs::remove_dir_all(p);
    }
    Ok(())
}

struct SyncLock(PathBuf);
impl SyncLock {
    fn acquire(path: &Path) -> Result<Self, AppError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| AppError::Internal(e.to_string()))?
        }
        fs::create_dir(path)
            .map_err(|_| AppError::Internal("已有数据库修复任务正在运行".into()))?;
        fs::write(
            path.join("owner.json"),
            json!({"pid":std::process::id()}).to_string(),
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(Self(path.to_path_buf()))
    }
}
impl Drop for SyncLock {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use tempfile::tempdir;

    fn fixture(projectless: bool) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempdir().unwrap();
        let home = tmp.path().join(".codex");
        fs::create_dir_all(home.join("sessions/2026")).unwrap();
        fs::write(
            home.join("sessions/2026/rollout-test.jsonl"),
            format!(
                "{}\n{}\n",
                json!({"type":"session_meta","payload":{"id":"thread-1","model_provider":"openai","cwd":r"\\?\C:\workspace"}}),
                json!({"type":"event_msg","payload":{"type":"user_message"}})
            ),
        )
        .unwrap();
        if projectless {
            fs::write(
                home.join(".codex-global-state.json"),
                json!({"projectless-thread-ids":["thread-1"]}).to_string(),
            )
            .unwrap();
        }
        let db = Connection::open(home.join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT, has_user_event INTEGER, cwd TEXT); INSERT INTO threads VALUES('thread-1','openai',0,'C:/old');").unwrap();
        drop(db);
        (tmp, home)
    }

    #[test]
    fn syncs_rollout_provider_and_targeted_sqlite_fields() {
        let (_tmp, home) = fixture(false);
        let result = synchronize(&home, "custom").unwrap();
        assert_eq!(result.rows_updated, 3);
        let rollout = fs::read_to_string(home.join("sessions/2026/rollout-test.jsonl")).unwrap();
        assert!(rollout.contains("\"model_provider\":\"custom\""));
        let db = Connection::open(home.join("state_5.sqlite")).unwrap();
        let row: (String, i64, String) = db
            .query_row(
                "SELECT model_provider,has_user_event,cwd FROM threads",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("custom".into(), 1, "C:/workspace".into()));
        assert!(
            Path::new(&result.backup_path)
                .join("db/state_5.sqlite")
                .exists()
        );
    }

    #[test]
    fn projectless_thread_keeps_existing_cwd() {
        let (_tmp, home) = fixture(true);
        synchronize(&home, "custom").unwrap();
        let db = Connection::open(home.join("state_5.sqlite")).unwrap();
        let cwd: String = db
            .query_row("SELECT cwd FROM threads", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cwd, "C:/old");
    }

    #[test]
    fn syncs_current_local_thread_catalog_schema() {
        let (_tmp, home) = fixture(false);
        fs::create_dir_all(home.join("sqlite")).unwrap();
        let catalog_path = home.join("sqlite/codex-dev.db");
        let catalog = Connection::open(&catalog_path).unwrap();
        catalog
            .execute_batch(
                "CREATE TABLE local_thread_catalog(
                    host_id TEXT NOT NULL,
                    thread_id TEXT NOT NULL,
                    display_title TEXT NOT NULL,
                    source_created_at REAL NOT NULL,
                    source_updated_at REAL NOT NULL,
                    cwd TEXT NOT NULL,
                    source_kind TEXT NOT NULL,
                    source_detail TEXT,
                    model_provider TEXT NOT NULL,
                    git_branch TEXT,
                    observation_sequence INTEGER NOT NULL,
                    missing_candidate INTEGER NOT NULL DEFAULT 0,
                    PRIMARY KEY(host_id, thread_id)
                );
                INSERT INTO local_thread_catalog VALUES(
                    'local','thread-1','Test',0,0,'C:/old','cli',NULL,
                    'openai',NULL,0,0
                );",
            )
            .unwrap();
        drop(catalog);

        let result = synchronize(&home, "custom").unwrap();

        assert_eq!(result.databases_repaired, 2);
        let catalog = Connection::open(catalog_path).unwrap();
        let row: (String, String) = catalog
            .query_row(
                "SELECT model_provider,cwd FROM local_thread_catalog WHERE thread_id='thread-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, ("custom".into(), "C:/workspace".into()));
    }

    #[test]
    fn skips_auxiliary_database_without_session_tables() {
        let (_tmp, home) = fixture(false);
        fs::create_dir_all(home.join("sqlite")).unwrap();
        let auxiliary_path = home.join("sqlite/auxiliary.db");
        let auxiliary = Connection::open(&auxiliary_path).unwrap();
        auxiliary
            .execute_batch("CREATE TABLE automations(id TEXT PRIMARY KEY);")
            .unwrap();
        drop(auxiliary);

        let result = synchronize(&home, "custom").unwrap();

        assert_eq!(result.databases_repaired, 1);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("辅助数据库"))
        );
        let auxiliary = Connection::open(auxiliary_path).unwrap();
        let table_count: i64 = auxiliary
            .query_row("SELECT count(*) FROM automations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(table_count, 0);
    }

    #[test]
    fn rejects_unknown_session_catalog_schema() {
        let (_tmp, home) = fixture(false);
        fs::create_dir_all(home.join("sqlite")).unwrap();
        let path = home.join("sqlite/unknown.db");
        let database = Connection::open(&path).unwrap();
        database
            .execute_batch("CREATE TABLE local_thread_catalog(thread_id TEXT PRIMARY KEY);")
            .unwrap();
        drop(database);

        let error = synchronize(&home, "custom").unwrap_err();
        assert!(matches!(error, AppError::UnknownSchema(_)));
    }

    #[test]
    fn normalizes_workspace_state_paths() {
        let (_tmp, home) = fixture(false);
        fs::write(
            home.join(".codex-global-state.json"),
            json!({
                "electron-saved-workspace-roots":[r"\\?\C:\workspace", "C:/workspace"],
                "electron-workspace-root-labels":{r"\\?\C:\workspace":"Workspace"}
            })
            .to_string(),
        )
        .unwrap();
        synchronize(&home, "custom").unwrap();
        let state: Value = serde_json::from_str(
            &fs::read_to_string(home.join(".codex-global-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            state["electron-saved-workspace-roots"],
            json!(["C:/workspace"])
        );
        assert_eq!(
            state["electron-workspace-root-labels"],
            json!({"C:/workspace":"Workspace"})
        );
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("state.json");
        fs::write(&path, b"old").unwrap();
        atomic_write(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(
            fs::read_dir(tmp.path()).unwrap().count(),
            1,
            "temporary file should be removed after replacement"
        );
    }

    #[test]
    fn database_backup_restores_database_and_removes_new_sidecars() {
        let (_tmp, home) = fixture(false);
        let db = home.join("state_5.sqlite");
        let sources = known_source_providers(&home, "custom");
        let changes = collect_rollouts(&home, "custom", &sources).unwrap();
        let backup = create_backup(&home, "custom", &changes, std::slice::from_ref(&db)).unwrap();

        {
            let connection = Connection::open(&db).unwrap();
            connection
                .execute("UPDATE threads SET model_provider='broken'", [])
                .unwrap();
        }
        let wal = PathBuf::from(format!("{}-wal", db.display()));
        fs::write(&wal, b"new sidecar").unwrap();

        restore_database_backup(&home, &backup, std::slice::from_ref(&db)).unwrap();

        let connection = Connection::open(&db).unwrap();
        let provider: String = connection
            .query_row("SELECT model_provider FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(provider, "openai");
        assert!(!wal.exists());
    }

    #[test]
    fn leaves_unknown_provider_history_untouched() {
        let (_tmp, home) = fixture(false);
        let rollout_path = home.join("sessions/2026/rollout-test.jsonl");
        let original = fs::read_to_string(&rollout_path)
            .unwrap()
            .replace("\"openai\"", "\"user_owned_provider\"");
        fs::write(&rollout_path, original).unwrap();
        let db = Connection::open(home.join("state_5.sqlite")).unwrap();
        db.execute(
            "UPDATE threads SET model_provider='user_owned_provider'",
            [],
        )
        .unwrap();
        drop(db);

        synchronize(&home, "custom").unwrap();

        let rollout = fs::read_to_string(rollout_path).unwrap();
        assert!(rollout.contains("\"model_provider\":\"user_owned_provider\""));
        let db = Connection::open(home.join("state_5.sqlite")).unwrap();
        let provider: String = db
            .query_row("SELECT model_provider FROM threads", [], |row| row.get(0))
            .unwrap();
        assert_eq!(provider, "user_owned_provider");
    }

    #[test]
    fn active_unknown_provider_is_not_a_trusted_migration_source() {
        let (_tmp, home) = fixture(false);
        fs::write(
            home.join("config.toml"),
            "model_provider = \"user_owned_provider\"\n[model_providers.user_owned_provider]\nname = \"Private\"\n",
        )
        .unwrap();
        let sources = known_source_providers(&home, "custom");
        assert!(!sources.contains("user_owned_provider"));
        assert!(sources.contains("openai"));
    }

    #[test]
    fn external_database_backup_round_trip_uses_stable_mapping() {
        let (_tmp, home) = fixture(false);
        let external_root = tempdir().unwrap();
        let external_db = external_root.path().join("state.sqlite");
        let connection = Connection::open(&external_db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE marker(value TEXT); INSERT INTO marker VALUES('original');",
            )
            .unwrap();
        drop(connection);

        let backup =
            create_backup(&home, "custom", &[], std::slice::from_ref(&external_db)).unwrap();
        let relative = backup_relative_path(&home, &external_db);
        assert!(relative.starts_with("external"));
        assert!(backup.join("db").join(&relative).exists());

        let connection = Connection::open(&external_db).unwrap();
        connection
            .execute("UPDATE marker SET value='changed'", [])
            .unwrap();
        drop(connection);
        restore_database_backup(&home, &backup, std::slice::from_ref(&external_db)).unwrap();

        let connection = Connection::open(&external_db).unwrap();
        let value: String = connection
            .query_row("SELECT value FROM marker", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "original");
    }
}
