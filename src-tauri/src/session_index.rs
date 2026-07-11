use crate::{codex, models::SessionSummary, storage::Store};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;

pub fn rebuild(store: &Store) -> anyhow::Result<Vec<SessionSummary>> {
    let mut by_thread = HashMap::<String, SessionSummary>::new();
    for session in codex::list_sessions(None)? {
        merge(&mut by_thread, session);
    }
    for path in codex::rollout_files() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let mut meta = None;
        let mut has_user_event = false;
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if record.get("type").and_then(Value::as_str) == Some("session_meta") {
                meta = record.get("payload").cloned();
            }
            has_user_event |= matches!(
                record.pointer("/payload/type").and_then(Value::as_str),
                Some("user_message" | "user_input")
            );
        }
        let Some(meta) = meta else { continue };
        let Some(id) = meta.get("id").and_then(Value::as_str) else {
            continue;
        };
        let provider = meta
            .get("model_provider")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let updated_at = fs::metadata(&path)
            .and_then(|value| value.modified())
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|value| value.as_secs() as i64)
            .unwrap_or_default();
        let archived = path
            .components()
            .any(|part| part.as_os_str() == "archived_sessions");
        let session = SessionSummary {
            identity: format!("rollout:{}", path.display()),
            id: id.to_string(),
            title: meta
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            provider: provider.clone(),
            cwd: meta
                .get("cwd")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim_start_matches(r"\\?\")
                .replace('\\', "/"),
            archived,
            updated_at,
            source_db: String::new(),
            source_rollout: Some(path.display().to_string()),
            original_provider: provider,
            has_user_event,
        };
        merge(&mut by_thread, session);
    }
    let mut sessions = by_thread.into_values().collect::<Vec<_>>();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    store.replace_unified_sessions(&sessions)?;
    Ok(sessions)
}

fn merge(target: &mut HashMap<String, SessionSummary>, next: SessionSummary) {
    match target.get_mut(&next.id) {
        Some(current) => {
            if current.title.is_empty() {
                current.title = next.title;
            }
            if current.cwd.is_empty() {
                current.cwd = next.cwd;
            }
            if current.provider.is_empty() {
                current.provider = next.provider;
            }
            if current.original_provider.is_empty() {
                current.original_provider = next.original_provider;
            }
            if current.source_db.is_empty() {
                current.source_db = next.source_db;
            }
            if next.source_rollout.is_some() {
                current.source_rollout = next.source_rollout;
                current.identity = next.identity;
            }
            current.archived |= next.archived;
            current.has_user_event |= next.has_user_event;
            current.updated_at = current.updated_at.max(next.updated_at);
        }
        None => {
            target.insert(next.id.clone(), next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_preserves_database_title_and_adds_rollout_source() {
        let mut sessions = HashMap::new();
        let database = SessionSummary {
            identity: "db#1".into(),
            id: "1".into(),
            title: "Title".into(),
            provider: "custom".into(),
            cwd: String::new(),
            archived: false,
            updated_at: 1,
            source_db: "db".into(),
            source_rollout: None,
            original_provider: "custom".into(),
            has_user_event: false,
        };
        let rollout = SessionSummary {
            identity: "rollout:file".into(),
            id: "1".into(),
            title: String::new(),
            provider: "openai".into(),
            cwd: "C:/work".into(),
            archived: true,
            updated_at: 2,
            source_db: String::new(),
            source_rollout: Some("file".into()),
            original_provider: "openai".into(),
            has_user_event: true,
        };
        merge(&mut sessions, database);
        merge(&mut sessions, rollout);
        let result = &sessions["1"];
        assert_eq!(result.title, "Title");
        assert_eq!(result.cwd, "C:/work");
        assert!(result.archived && result.has_user_event);
    }
}
