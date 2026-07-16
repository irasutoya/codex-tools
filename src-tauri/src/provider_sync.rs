use crate::{
    models::{AppError, DatabaseScan, RepairResult, RepairScan, RepairTarget, SessionSummary},
    storage::atomic_write_if_unchanged,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    io::BufRead,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

pub fn scan(codex_home: &Path) -> RepairScan {
    let mut warnings = vec![];
    let rollouts = rollout_files(codex_home);
    let mut providers = BTreeMap::<String, BTreeSet<String>>::new();
    let mut session_meta_count = 0;
    for path in &rollouts {
        match rollout_provider(path) {
            Ok(Some(provider)) => {
                session_meta_count += 1;
                providers
                    .entry(provider)
                    .or_default()
                    .insert("rollout".into());
            }
            Ok(None) => {}
            Err(error) => warnings.push(format!("无法读取 {}：{error}", path.display())),
        }
    }
    let mut databases = vec![];
    for path in database_paths(codex_home) {
        match inspect_database(&path) {
            Ok(Some((schema, count, found))) => {
                for provider in found {
                    providers
                        .entry(provider)
                        .or_default()
                        .insert("sqlite".into());
                }
                databases.push(DatabaseScan {
                    path: path.display().to_string(),
                    schema,
                    thread_count: count,
                });
            }
            Ok(None) => {}
            Err(error) => warnings.push(format!("无法检查 {}：{error}", path.display())),
        }
    }
    let current_provider = configured_provider(codex_home);
    providers
        .entry(current_provider.clone())
        .or_default()
        .insert("config".into());
    RepairScan {
        current_provider: current_provider.clone(),
        targets: providers
            .into_iter()
            .map(|(id, sources)| RepairTarget {
                current: id == current_provider,
                id,
                sources: sources.into_iter().collect(),
            })
            .collect(),
        rollout_files: rollouts.len(),
        session_meta_count,
        databases,
        warnings,
    }
}

pub fn configured_provider(codex_home: &Path) -> String {
    fs::read_to_string(codex_home.join("config.toml"))
        .ok()
        .and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        .and_then(|doc| {
            doc.get("model_provider")
                .and_then(toml_edit::Item::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "openai".into())
}

pub fn repair(codex_home: &Path, target: &str) -> Result<RepairResult, AppError> {
    let target = target.trim();
    if !matches!(target, "openai" | "custom") {
        return Err(AppError::InvalidConfig(
            "只能在 OpenAI 官方账号与第三方 API 之间更新会话归属。".into(),
        ));
    }
    let rollouts = rollout_files(codex_home);
    let mut result = RepairResult {
        target_provider: target.to_owned(),
        files_scanned: rollouts.len(),
        ..RepairResult::default()
    };

    for path in rollouts {
        match repair_rollout(&path, target) {
            Ok((changed, metas)) => {
                result.session_meta_updated += metas;
                if changed {
                    result.files_modified += 1
                } else {
                    result.files_skipped += 1
                }
            }
            Err(error) => {
                result.files_failed += 1;
                result.warnings.push(format!("{}：{error}", path.display()));
            }
        }
    }

    for path in database_paths(codex_home) {
        match repair_database(&path, target) {
            Ok(rows) => result.rows_updated += rows,
            Err(error) => result.warnings.push(format!("{}：{error}", path.display())),
        }
    }
    Ok(result)
}

pub fn list_database_sessions_from_paths(paths: &[PathBuf]) -> anyhow::Result<Vec<SessionSummary>> {
    let mut sessions = vec![];
    for path in paths {
        let db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        let Some((table, id, columns)) = session_table(&db)? else {
            continue;
        };
        let title = choose(&columns, &["title", "display_title"], "''");
        let provider = choose(&columns, &["model_provider"], "''");
        let cwd = choose(&columns, &["cwd"], "''");
        let archived = choose(&columns, &["archived"], "0");
        let updated = choose(&columns, &["updated_at", "source_updated_at"], "0");
        let sql = format!(
            "SELECT {id},{title},{provider},{cwd},{archived},CAST({updated} AS INTEGER) FROM {table} ORDER BY {updated} DESC LIMIT 2000"
        );
        let mut statement = db.prepare(&sql)?;
        let rows = statement.query_map([], |row| {
            let id: String = row.get(0)?;
            let provider: String = row.get(2).unwrap_or_default();
            Ok(SessionSummary {
                identity: format!("{}#{id}", path.display()),
                id,
                title: row.get(1).unwrap_or_default(),
                provider: provider.clone(),
                cwd: row.get(3).unwrap_or_default(),
                archived: row.get::<_, i64>(4).unwrap_or_default() != 0,
                updated_at: row.get(5).unwrap_or_default(),
                source_db: path.display().to_string(),
                source_rollout: None,
                original_provider: provider,
                has_user_event: false,
            })
        })?;
        sessions.extend(rows.flatten());
    }
    Ok(sessions)
}

pub fn rollout_files(codex_home: &Path) -> Vec<PathBuf> {
    [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ]
    .into_iter()
    .flat_map(|directory| {
        WalkDir::new(directory)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.into_path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "jsonl")
            })
    })
    .collect()
}

pub fn database_paths(codex_home: &Path) -> Vec<PathBuf> {
    let mut output = vec![];
    let root_database = codex_home.join("state_5.sqlite");
    if root_database.is_file() {
        output.push(root_database);
    }
    collect_databases(&codex_home.join("sqlite"), &mut output);
    if let Some(path) = std::env::var_os("CODEX_SQLITE_HOME") {
        collect_databases(&PathBuf::from(path), &mut output);
    }
    if let Ok(text) = fs::read_to_string(codex_home.join("config.toml"))
        && let Ok(document) = text.parse::<toml_edit::DocumentMut>()
        && let Some(path) = document
            .get("sqlite_home")
            .and_then(toml_edit::Item::as_str)
    {
        collect_databases(&PathBuf::from(path), &mut output);
    }
    output.sort();
    output.dedup();
    output
}

fn collect_databases(path: &Path, output: &mut Vec<PathBuf>) {
    if path.is_file() {
        output.push(path.to_path_buf());
    } else if let Ok(entries) = fs::read_dir(path) {
        output.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "db" || extension == "sqlite")
        }));
    }
}

fn repair_rollout(path: &Path, target: &str) -> anyhow::Result<(bool, usize)> {
    let original_bytes = fs::read(path)?;
    let original = std::str::from_utf8(&original_bytes)?;
    let mut changed = false;
    let mut meta_count = 0;
    let mut output = String::with_capacity(original.len());
    for segment in original.split_inclusive('\n') {
        let (line, ending) = segment.strip_suffix('\n').map_or((segment, ""), |line| {
            (
                line.strip_suffix('\r').unwrap_or(line),
                if line.ends_with('\r') { "\r\n" } else { "\n" },
            )
        });
        let Ok(mut record) = serde_json::from_str::<Value>(line) else {
            output.push_str(segment);
            continue;
        };
        let mut record_changed = false;
        if record.get("type").and_then(Value::as_str) == Some("session_meta") {
            let original_provider = record
                .pointer("/payload/model_provider")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if original_provider != target {
                record["payload"]["model_provider"] = Value::String(target.to_owned());
                changed = true;
                record_changed = true;
                meta_count += 1;
            }
        }
        if record_changed {
            output.push_str(&serde_json::to_string(&record)?);
            output.push_str(ending);
        } else {
            output.push_str(segment);
        }
    }
    if changed && !atomic_write_if_unchanged(path, &original_bytes, output.as_bytes())? {
        anyhow::bail!("Codex 正在更新这个会话，文件已保持原样。请关闭该会话后重试");
    }
    Ok((changed, meta_count))
}

fn repair_database(path: &Path, target: &str) -> anyhow::Result<usize> {
    let mut db = Connection::open(path)?;
    let Some((table, _, columns)) = session_table(&db)? else {
        return Ok(0);
    };
    if !columns.contains("model_provider") {
        return Ok(0);
    }
    let transaction = db.transaction()?;
    let rows = transaction.execute(
        &format!("UPDATE {table} SET model_provider=?1 WHERE COALESCE(model_provider,'')<>?1"),
        [target],
    )?;
    transaction.commit()?;
    Ok(rows)
}

fn inspect_database(path: &Path) -> anyhow::Result<Option<(String, u64, Vec<String>)>> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let Some((table, _, columns)) = session_table(&db)? else {
        return Ok(None);
    };
    if !columns.contains("model_provider") {
        anyhow::bail!("会话数据库格式不受支持，无法更新归属信息");
    }
    let count = db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })?;
    let mut statement = db.prepare(&format!(
        "SELECT DISTINCT model_provider FROM {table} WHERE model_provider IS NOT NULL"
    ))?;
    let providers = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .flatten()
        .collect();
    Ok(Some((table.into(), count, providers)))
}

fn session_table(
    db: &Connection,
) -> anyhow::Result<Option<(&'static str, &'static str, HashSet<String>)>> {
    for (table, id) in [("threads", "id"), ("local_thread_catalog", "thread_id")] {
        let columns = table_columns(db, table)?;
        if columns.contains(id) {
            return Ok(Some((table, id, columns)));
        }
    }
    Ok(None)
}

fn table_columns(db: &Connection, table: &str) -> anyhow::Result<HashSet<String>> {
    let mut statement = db.prepare(&format!("PRAGMA table_info({table})"))?;
    Ok(statement
        .query_map([], |row| row.get::<_, String>(1))?
        .flatten()
        .collect())
}

fn choose<'a>(columns: &HashSet<String>, names: &'a [&'a str], fallback: &'a str) -> &'a str {
    names
        .iter()
        .copied()
        .find(|name| columns.contains(*name))
        .unwrap_or(fallback)
}

fn rollout_provider(path: &Path) -> anyhow::Result<Option<String>> {
    for line in std::io::BufReader::new(fs::File::open(path)?).lines() {
        let line = line?;
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) == Some("session_meta") {
            return Ok(Some(
                record
                    .pointer("/payload/model_provider")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            ));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repair_unifies_all_provider_metadata_without_app_state_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let data = temp.path().join("data");
        fs::create_dir_all(home.join("sessions/2026")).unwrap();
        let rollout = home.join("sessions/2026/rollout.jsonl");
        let before = format!(
            "{}\n{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"other-provider","cwd":"C:/keep"}}),
            serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"unchanged"}})
        );
        fs::write(&rollout, before).unwrap();
        let result = repair(&home, "custom").unwrap();
        assert_eq!(result.session_meta_updated, 1);
        let after = fs::read_to_string(rollout).unwrap();
        assert!(after.contains("\"model_provider\":\"custom\""));
        assert!(after.contains("\"message\":\"unchanged\""));
        assert!(!data.exists());
        assert!(!data.join("backup").exists());
    }

    #[test]
    fn repair_preserves_already_matching_metadata_byte_for_byte() {
        let temp = tempfile::tempdir().unwrap();
        let rollout = temp.path().join("rollout.jsonl");
        let unchanged = r#"{ "type": "session_meta", "payload": { "id": "two", "model_provider": "custom", "future": true } }"#;
        let original = format!(
            "{}\n{unchanged}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"openai"}})
        );
        fs::write(&rollout, &original).unwrap();
        let (changed, count) = repair_rollout(&rollout, "custom").unwrap();

        assert!(changed);
        assert_eq!(count, 1);
        let repaired = fs::read_to_string(rollout).unwrap();
        assert!(repaired.ends_with(&format!("{unchanged}\n")));
    }

    #[test]
    fn sqlite_update_is_narrow_and_transactional() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let connection = Connection::open(&db).unwrap();
        connection.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT, title TEXT); INSERT INTO threads VALUES('one','other-provider','keep'); INSERT INTO threads VALUES('two','custom','same'); INSERT INTO threads VALUES('three',NULL,'missing');").unwrap();
        drop(connection);
        assert_eq!(repair_database(&db, "custom").unwrap(), 2);
        let connection = Connection::open(db).unwrap();
        let providers = connection
            .prepare("SELECT model_provider FROM threads ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(providers, vec!["custom", "custom", "custom"]);
        let title: String = connection
            .query_row("SELECT title FROM threads WHERE id='one'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(title, "keep");
    }

    #[test]
    fn repair_toggles_all_metadata_between_managed_providers() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        fs::write(
            &rollout,
            format!(
                "{}\n",
                serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"other-provider"}})
            ),
        )
        .unwrap();

        let to_custom = repair(&home, "custom").unwrap();
        assert_eq!(to_custom.session_meta_updated, 1);
        assert!(
            fs::read_to_string(&rollout)
                .unwrap()
                .contains("\"model_provider\":\"custom\"")
        );

        let to_openai = repair(&home, "openai").unwrap();
        assert_eq!(to_openai.session_meta_updated, 1);
        assert!(
            fs::read_to_string(rollout)
                .unwrap()
                .contains("\"model_provider\":\"openai\"")
        );
    }

    #[test]
    fn repair_rejects_unmanaged_provider_targets() {
        let temp = tempfile::tempdir().unwrap();
        let error = repair(temp.path(), "third-party").unwrap_err();
        assert!(error.to_string().contains("只能在 OpenAI"));
    }
}
