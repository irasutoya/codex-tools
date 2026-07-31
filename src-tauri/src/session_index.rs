use crate::{models::SessionSummary, provider_sync};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, Read},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant, SystemTime},
};

const CACHE_RECHECK_INTERVAL: Duration = Duration::from_secs(1);
const MAX_ROLLOUT_INDEX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ROLLOUT_LINE_BYTES: usize = 256 * 1024;
const MAX_SESSION_ID_CHARS: usize = 512;
const MAX_SESSION_PROVIDER_CHARS: usize = 128;
const MAX_SESSION_PATH_CHARS: usize = 4_096;

#[derive(Clone, Default)]
pub struct SessionIndex {
    cache: Arc<RwLock<Option<CachedIndex>>>,
}

struct CachedIndex {
    home: PathBuf,
    sources: Vec<SourceStamp>,
    sessions: Arc<Vec<SessionSummary>>,
    checked_at: Instant,
}

#[derive(Clone, PartialEq, Eq)]
struct SourceStamp {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
}

impl SessionIndex {
    pub fn load_recent(&self, codex_home: &Path) -> anyhow::Result<Arc<Vec<SessionSummary>>> {
        if let Some(sessions) = self
            .cache
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .filter(|cache| {
                cache.home == codex_home && cache.checked_at.elapsed() < CACHE_RECHECK_INTERVAL
            })
            .map(|cache| cache.sessions.clone())
        {
            return Ok(sessions);
        }
        self.load(codex_home)
    }

    pub fn load(&self, codex_home: &Path) -> anyhow::Result<Arc<Vec<SessionSummary>>> {
        let (database_paths, rollout_paths, sources) = session_sources(codex_home);
        let checked_at = Instant::now();
        if let Some(sessions) = self
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
            .filter(|cache| cache.home == codex_home && cache.sources == sources)
            .map(|cache| {
                cache.checked_at = checked_at;
                cache.sessions.clone()
            })
        {
            return Ok(sessions);
        }

        let sessions = Arc::new(rebuild_from_paths(&database_paths, &rollout_paths)?);
        *self
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(CachedIndex {
            home: codex_home.to_path_buf(),
            sources,
            sessions: sessions.clone(),
            checked_at,
        });
        Ok(sessions)
    }

    pub fn invalidate(&self) {
        *self
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

fn session_sources(codex_home: &Path) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<SourceStamp>) {
    let database_paths = provider_sync::database_paths(codex_home);
    let mut rollout_paths = provider_sync::rollout_files(codex_home);
    rollout_paths.sort();
    let sources = database_paths
        .iter()
        .flat_map(|path| [path.clone(), sqlite_sidecar(path, "-wal")])
        .chain(rollout_paths.iter().cloned())
        .map(|path| {
            let metadata = fs::metadata(&path).ok();
            SourceStamp {
                path,
                length: metadata.as_ref().map(fs::Metadata::len).unwrap_or(0),
                modified: metadata.and_then(|value| value.modified().ok()),
            }
        })
        .collect();
    (database_paths, rollout_paths, sources)
}

fn sqlite_sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn rebuild_from_paths(
    database_paths: &[PathBuf],
    rollout_paths: &[PathBuf],
) -> anyhow::Result<Vec<SessionSummary>> {
    let mut sessions = HashMap::<String, SessionSummary>::new();
    for mut session in provider_sync::list_database_sessions_from_paths(database_paths)? {
        session.id = truncate_text(&session.id, MAX_SESSION_ID_CHARS);
        session.title = truncate_text(&session.title, 512);
        session.provider = truncate_text(&session.provider, MAX_SESSION_PROVIDER_CHARS);
        session.original_provider =
            truncate_text(&session.original_provider, MAX_SESSION_PROVIDER_CHARS);
        session.cwd = truncate_text(&session.cwd, MAX_SESSION_PATH_CHARS);
        sessions.insert(session.id.clone(), session);
    }
    for path in rollout_paths {
        let Ok(file) = fs::File::open(path) else {
            continue;
        };
        let mut metadata = None;
        let mut title = String::new();
        let mut has_user_event = false;
        for line in std::io::BufReader::new(file)
            .take(MAX_ROLLOUT_INDEX_BYTES)
            .lines()
        {
            let Ok(line) = line else { break };
            if line.len() > MAX_ROLLOUT_LINE_BYTES {
                continue;
            }
            let Ok(record) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if record.get("type").and_then(Value::as_str) == Some("session_meta") {
                metadata = record.get("payload").cloned();
            }
            if matches!(
                record.pointer("/payload/type").and_then(Value::as_str),
                Some("user_message" | "user_input")
            ) {
                has_user_event = true;
                if title.is_empty() {
                    title = record
                        .pointer("/payload/message")
                        .or_else(|| record.pointer("/payload/text"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or_default()
                        .chars()
                        .take(160)
                        .collect();
                }
            }
            if metadata.is_some() && has_user_event && !title.is_empty() {
                break;
            }
        }
        let Some(metadata) = metadata else { continue };
        if metadata.pointer("/source/subagent").is_some() {
            continue;
        }
        let Some(raw_id) = metadata.get("id").and_then(Value::as_str) else {
            continue;
        };
        let id = truncate_text(raw_id, MAX_SESSION_ID_CHARS);
        let provider = truncate_text(
            metadata
                .get("model_provider")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            MAX_SESSION_PROVIDER_CHARS,
        );
        let updated_at = fs::metadata(path)
            .ok()
            .and_then(|value| value.modified().ok())
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|value| value.as_secs() as i64)
            .unwrap_or_default();
        let rollout = path.display().to_string();
        sessions
            .entry(id.clone())
            .and_modify(|session| {
                session.provider = provider.clone();
                session.original_provider = provider.clone();
                session.source_rollout = Some(rollout.clone());
                session.has_user_event |= has_user_event;
                session.updated_at = session.updated_at.max(updated_at);
                if session.title.is_empty() {
                    session.title = title.clone();
                }
            })
            .or_insert(SessionSummary {
                identity: format!("rollout:{rollout}"),
                id,
                title,
                provider: provider.clone(),
                cwd: truncate_text(
                    &metadata
                        .get("cwd")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim_start_matches(r"\\?\")
                        .replace('\\', "/"),
                    MAX_SESSION_PATH_CHARS,
                ),
                archived: path
                    .components()
                    .any(|part| part.as_os_str() == "archived_sessions"),
                updated_at,
                source_db: String::new(),
                source_rollout: Some(rollout),
                original_provider: provider,
                has_user_event,
            });
    }
    let mut output = sessions.into_values().collect::<Vec<_>>();
    output.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    Ok(output)
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn reuses_unchanged_index_and_rebuilds_after_source_change() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join("rollout.jsonl");
        fs::write(
            &rollout,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"one\",\"model_provider\":\"custom\",\"cwd\":\"C:/work\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"first\"}}\n"
            ),
        )
        .unwrap();
        let index = SessionIndex::default();

        let first = index.load(temp.path()).unwrap();
        let unchanged = index.load(temp.path()).unwrap();
        assert!(Arc::ptr_eq(&first, &unchanged));
        assert_eq!(first[0].title, "first");

        fs::write(
            &rollout,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"one\",\"model_provider\":\"custom\",\"cwd\":\"C:/work\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"updated title\"}}\n"
            ),
        )
        .unwrap();
        let updated = index.load(temp.path()).unwrap();
        assert!(!Arc::ptr_eq(&first, &updated));
        assert_eq!(updated[0].title, "updated title");
    }

    #[test]
    fn recent_load_skips_rewalking_sources_until_forced_refresh() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join("rollout.jsonl");
        fs::write(
            &rollout,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"one\",\"model_provider\":\"custom\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"first\"}}\n"
            ),
        )
        .unwrap();
        let index = SessionIndex::default();
        let first = index.load(temp.path()).unwrap();

        fs::write(
            &rollout,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"one\",\"model_provider\":\"custom\"}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"changed\"}}\n"
            ),
        )
        .unwrap();
        let recent = index.load_recent(temp.path()).unwrap();
        assert!(Arc::ptr_eq(&first, &recent));

        let refreshed = index.load(temp.path()).unwrap();
        assert!(!Arc::ptr_eq(&first, &refreshed));
        assert_eq!(refreshed[0].title, "changed");
    }

    #[test]
    fn sqlite_wal_changes_invalidate_the_index() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("state_5.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads(
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    model_provider TEXT,
                    cwd TEXT,
                    archived INTEGER,
                    updated_at INTEGER
                );
                INSERT INTO threads VALUES('one','First','custom','C:/one',0,1);",
            )
            .unwrap();
        let index = SessionIndex::default();
        let first = index.load(temp.path()).unwrap();
        assert_eq!(first.len(), 1);

        connection
            .execute(
                "INSERT INTO threads VALUES('two','Second','custom','C:/two',0,2)",
                [],
            )
            .unwrap();
        let updated = index.load(temp.path()).unwrap();
        assert_eq!(updated.len(), 2);
        assert!(!Arc::ptr_eq(&first, &updated));
    }

    #[test]
    fn oversized_rollout_lines_do_not_block_later_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let rollout = sessions.join("rollout.jsonl");
        let mut content = "x".repeat(MAX_ROLLOUT_LINE_BYTES + 1);
        content.push('\n');
        content.push_str(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"safe\",\"model_provider\":\"openai\"}}\n",
        );
        content.push_str(
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\",\"message\":\"kept\"}}\n",
        );
        fs::write(&rollout, content).unwrap();

        let index = SessionIndex::default();
        let indexed = index.load(temp.path()).unwrap();

        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[0].id, "safe");
        assert_eq!(indexed[0].title, "kept");
    }
}
