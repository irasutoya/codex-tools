use crate::{
    models::{AppError, PageResult, SessionSummary},
    provider_sync,
};
use serde_json::Value;
use std::{
    borrow::Cow,
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
    database_count: usize,
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
        Ok(self.load_with_database_count(codex_home)?.0)
    }

    pub fn load_with_database_count(
        &self,
        codex_home: &Path,
    ) -> anyhow::Result<(Arc<Vec<SessionSummary>>, usize)> {
        let (database_paths, rollout_paths, sources) = session_sources(codex_home);
        let checked_at = Instant::now();
        if let Some((sessions, database_count)) = self
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
            .filter(|cache| cache.home == codex_home && cache.sources == sources)
            .map(|cache| {
                cache.checked_at = checked_at;
                (cache.sessions.clone(), cache.database_count)
            })
        {
            return Ok((sessions, database_count));
        }

        let sessions = Arc::new(rebuild_from_paths(&database_paths, &rollout_paths)?);
        let database_count = database_paths.len();
        let result = (sessions.clone(), database_count);
        *self
            .cache
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(CachedIndex {
            home: codex_home.to_path_buf(),
            sources,
            sessions,
            database_count,
            checked_at,
        });
        Ok(result)
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

pub fn session_page(
    index: &SessionIndex,
    home: &Path,
    query: &str,
    page: usize,
    page_size: usize,
) -> Result<PageResult<SessionSummary>, AppError> {
    let sessions = index.load_recent(home)?;
    let query = query.trim();
    let normalized_query = if query.is_ascii() {
        Cow::Borrowed(query)
    } else {
        Cow::Owned(query.to_lowercase())
    };
    let query = normalized_query.as_ref();
    let start = (page - 1).saturating_mul(page_size);
    if query.is_empty() {
        return Ok(PageResult {
            items: sessions
                .iter()
                .skip(start)
                .take(page_size)
                .cloned()
                .collect(),
            total: sessions.len(),
            page,
            page_size,
        });
    }
    let mut total = 0;
    let mut items = Vec::with_capacity(page_size);
    for session in sessions.iter() {
        if !session_matches_query(session, query) {
            continue;
        }
        if total >= start && items.len() < page_size {
            items.push(session.clone());
        }
        total += 1;
    }
    Ok(PageResult {
        items,
        total,
        page,
        page_size,
    })
}

fn session_matches_query(session: &SessionSummary, normalized_query: &str) -> bool {
    if normalized_query.is_empty() {
        return true;
    }
    let query_is_ascii = normalized_query.is_ascii();
    [
        session.id.as_str(),
        session.title.as_str(),
        session.provider.as_str(),
        session.cwd.as_str(),
    ]
    .into_iter()
    .any(|value| {
        if query_is_ascii {
            value
                .as_bytes()
                .windows(normalized_query.len())
                .any(|window| window.eq_ignore_ascii_case(normalized_query.as_bytes()))
        } else {
            !value.is_ascii() && value.to_lowercase().contains(normalized_query)
        }
    })
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    // 单趟定位截断边界，避免先 `chars().count()` 再 `chars().take()`
    // 造成的双遍历（会话索引重建时对每个字段都会调用）。
    let cut = value
        .char_indices()
        .enumerate()
        .find_map(|(count, (index, _))| (count == max_chars).then_some(index));
    cut.map_or_else(|| value.to_owned(), |index| value[..index].to_owned())
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

    #[test]
    fn session_pages_filter_without_changing_totals_or_page_boundaries() {
        let temp = tempfile::tempdir().unwrap();
        let sessions = temp.path().join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        for (id, provider, cwd, title) in [
            ("one", "custom", "C:/alpha", "Alpha task"),
            ("two", "openai", "C:/beta", "中文任务"),
            ("three", "custom", "C:/gamma", "Gamma task"),
        ] {
            let contents = format!(
                "{}\n{}\n",
                serde_json::json!({
                    "type": "session_meta",
                    "payload": {"id": id, "model_provider": provider, "cwd": cwd}
                }),
                serde_json::json!({
                    "type": "event_msg",
                    "payload": {"type": "user_message", "message": title}
                })
            );
            fs::write(sessions.join(format!("{id}.jsonl")), contents).unwrap();
        }
        let index = SessionIndex::default();

        let first = session_page(&index, temp.path(), "", 1, 2).unwrap();
        let second = session_page(&index, temp.path(), "", 2, 2).unwrap();
        assert_eq!(first.total, 3);
        assert_eq!(first.items.len(), 2);
        assert_eq!(second.total, 3);
        assert_eq!(second.items.len(), 1);
        let mut ids = first
            .items
            .into_iter()
            .chain(second.items)
            .map(|session| session.id)
            .collect::<Vec<_>>();
        ids.sort();
        assert_eq!(ids, ["one", "three", "two"]);

        let ascii = session_page(&index, temp.path(), "ALPHA", 1, 10).unwrap();
        assert_eq!(ascii.total, 1);
        assert_eq!(ascii.items[0].id, "one");
        let unicode = session_page(&index, temp.path(), "中文", 1, 10).unwrap();
        assert_eq!(unicode.total, 1);
        assert_eq!(unicode.items[0].id, "two");
    }

    fn summary(id: &str, title: &str, provider: &str, cwd: &str) -> SessionSummary {
        SessionSummary {
            identity: id.into(),
            id: id.into(),
            title: title.into(),
            provider: provider.into(),
            cwd: cwd.into(),
            archived: false,
            updated_at: 0,
            source_db: "rollout.sqlite".into(),
            source_rollout: None,
            original_provider: provider.into(),
            has_user_event: true,
        }
    }

    #[test]
    fn session_query_matches_ascii_fields_case_insensitively() {
        let session = summary(
            "session-01",
            "修复登录 Bug",
            "openai-official",
            "/Users/iqboost/project",
        );
        assert!(session_matches_query(&session, "OPENAI"));
        assert!(session_matches_query(&session, "bug"));
        assert!(session_matches_query(&session, "project"));
        assert!(session_matches_query(&session, ""));
        assert!(!session_matches_query(&session, "absent"));
    }

    #[test]
    fn session_query_never_matches_ascii_field_with_cjk_query() {
        let session = summary(
            "session-01",
            "fix login bug",
            "openai-official",
            "/Users/iqboost/project",
        );
        assert!(!session_matches_query(&session, "登录"));
    }

    #[test]
    fn session_query_matches_cjk_title_with_normalized_lowercase() {
        let session = summary(
            "session-01",
            "修复 登录 Bug 与 Setup",
            "openai-official",
            "/Users/iqboost/project",
        );
        assert!(session_matches_query(&session, "登录"));
        assert!(session_matches_query(&session, "setup"));
        assert!(!session_matches_query(&session, "配额"));
    }
}
