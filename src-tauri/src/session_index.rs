use crate::{codex, models::SessionSummary};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn rebuild() -> anyhow::Result<Vec<SessionSummary>> {
    let mut by_thread = HashMap::<String, SessionSummary>::new();
    let indexed_titles = read_indexed_titles(&codex::home().join("session_index.jsonl"));
    for session in codex::list_sessions(None)? {
        merge(&mut by_thread, session);
    }
    for path in codex::rollout_files() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let mut meta = None;
        let mut has_user_event = false;
        let mut first_user_title = None;
        let mut is_subagent = false;
        for line in text.lines() {
            let Ok(record) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if record.get("type").and_then(Value::as_str) == Some("session_meta") {
                meta = record.get("payload").cloned();
                is_subagent = record.pointer("/payload/source/subagent").is_some();
            }
            if matches!(
                record.pointer("/payload/type").and_then(Value::as_str),
                Some("user_message" | "user_input")
            ) {
                has_user_event = true;
                if first_user_title.is_none() {
                    first_user_title = user_title(&record);
                }
            }
        }
        if is_subagent {
            continue;
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
            title: indexed_titles
                .get(id)
                .cloned()
                .or(first_user_title)
                .or_else(|| project_name(meta.get("cwd").and_then(Value::as_str)))
                .unwrap_or_default(),
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
    Ok(sessions)
}

fn read_indexed_titles(path: &Path) -> HashMap<String, String> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|value| {
            let id = value
                .get("id")
                .or_else(|| value.get("thread_id"))
                .and_then(Value::as_str)?;
            let title = value
                .get("title")
                .or_else(|| value.get("display_title"))
                .and_then(Value::as_str)?
                .trim();
            (!title.is_empty()).then(|| (id.to_string(), title.to_string()))
        })
        .collect()
}

fn user_title(record: &Value) -> Option<String> {
    let text = record
        .pointer("/payload/message")
        .or_else(|| record.pointer("/payload/text"))
        .or_else(|| record.pointer("/payload/content"))
        .and_then(Value::as_str)?
        .trim();
    if text.starts_with("# AGENTS.md")
        || text.starts_with("<environment_context>")
        || text.starts_with("<environment_context ")
    {
        return None;
    }
    let text = text
        .split_once("## My request for Codex:")
        .map(|(_, request)| request.trim())
        .unwrap_or(text);
    let title = text.lines().find(|line| !line.trim().is_empty())?.trim();
    (!title.starts_with("# Context from my IDE setup:")).then(|| title.chars().take(160).collect())
}

fn project_name(cwd: Option<&str>) -> Option<String> {
    let cwd = cwd?.trim_start_matches(r"\\?\").replace('\\', "/");
    PathBuf::from(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
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

    #[test]
    fn title_filter_ignores_injected_context_and_extracts_ide_request() {
        assert!(
            user_title(&serde_json::json!({
                "payload":{"message":"<environment_context>hidden</environment_context>"}
            }))
            .is_none()
        );
        let value = serde_json::json!({
            "payload":{"message":"# Context from my IDE setup:\nanything\n## My request for Codex:\nFix the route"}
        });
        assert_eq!(user_title(&value).as_deref(), Some("Fix the route"));
    }
}
