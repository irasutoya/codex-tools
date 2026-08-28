use crate::{
    models::{AppError, DatabaseScan, RepairResult, RepairScan, RepairTarget, SessionSummary},
    platform,
    storage::{atomic_write, atomic_write_if_unchanged},
};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    fs,
    io::{BufRead, Read},
    path::{Path, PathBuf},
    time::Instant,
};
use walkdir::WalkDir;

const MAX_ROLLOUT_SCAN_BYTES: u64 = 2 * 1024 * 1024;
const MAX_REPAIR_ROLLOUT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_REPAIR_WARNINGS: usize = 100;
const MAX_WARNING_CHARS: usize = 1_000;
// v3 invalidates entries produced by the former rewrite-and-clear-projection
// policy.  A cached rollout must have been checked by the byte-preserving
// policy before it can be skipped again.
const MANIFEST_VERSION: u32 = 3;
const MANIFEST_PREFIX_BYTES: usize = 4 * 1024;
const THREAD_HISTORY_FILE: &str = "thread_history_1.sqlite";

/// 修复引擎唯一接受的路由目标。两个变体都表示清除会话模型覆盖，让 Codex
/// 继承刚刚激活的连接的当前默认模型；不把任何历史 model 名称带到新上游。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionRoutingTarget {
    OpenAi,
    Custom,
}

impl SessionRoutingTarget {
    fn parse(value: &str) -> Result<Self, AppError> {
        match value.trim() {
            "openai" => Ok(Self::OpenAi),
            "custom" => Ok(Self::Custom),
            _ => Err(AppError::InvalidConfig(
                "只能在 OpenAI 账号与第三方 API 之间更新会话归属。".into(),
            )),
        }
    }

    fn provider(self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Custom => "custom",
        }
    }
}

/// 可展示和可修复的本地会话范围。根会话只用于列表；其子代理后代仅用于修复。
#[derive(Default)]
pub struct SessionScope {
    roots: HashMap<String, bool>,
    eligible: HashSet<String>,
    eligible_rollouts: HashSet<PathBuf>,
}

impl SessionScope {
    pub fn root_archived(&self, id: &str) -> Option<bool> {
        self.roots.get(id).copied()
    }

    #[cfg(test)]
    pub fn contains(&self, id: &str) -> bool {
        self.eligible.contains(id)
    }

    fn rollout_is_eligible(&self, path: &Path) -> bool {
        self.eligible_rollouts.contains(path)
    }

    fn eligible_ids(&self) -> Vec<&str> {
        let mut ids = self.eligible.iter().map(String::as_str).collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }
}

#[derive(Default)]
struct ScopeFacts {
    has_catalog: bool,
    roots: HashMap<String, bool>,
    parents: HashMap<String, String>,
    rollout_ids: Vec<(PathBuf, String)>,
}

/// 统一识别会话根及其子代理后代。新版 catalog 是活跃根的唯一权威来源；旧库
/// 没有 catalog 时回退到非归档、非网页端且没有父会话的 threads 记录。
pub fn session_scope(
    database_paths: &[PathBuf],
    rollout_paths: &[PathBuf],
) -> anyhow::Result<SessionScope> {
    let mut facts = ScopeFacts::default();
    for path in database_paths {
        collect_database_scope(path, &mut facts)?;
    }
    for path in rollout_paths {
        collect_rollout_scope(path, &mut facts)?;
    }

    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for (child, parent) in &facts.parents {
        if child != parent {
            children
                .entry(parent.clone())
                .or_default()
                .push(child.clone());
        }
    }
    let mut eligible = HashSet::new();
    let mut queue = facts.roots.keys().cloned().collect::<VecDeque<_>>();
    while let Some(id) = queue.pop_front() {
        if !eligible.insert(id.clone()) {
            continue;
        }
        if let Some(next) = children.get(&id) {
            queue.extend(next.iter().cloned());
        }
    }
    let eligible_rollouts = facts
        .rollout_ids
        .into_iter()
        .filter_map(|(path, id)| eligible.contains(&id).then_some(path))
        .collect();
    Ok(SessionScope {
        roots: facts.roots,
        eligible,
        eligible_rollouts,
    })
}

fn collect_database_scope(path: &Path, facts: &mut ScopeFacts) -> anyhow::Result<()> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let catalog = table_columns(&db, "local_thread_catalog")?;
    let hosts = table_columns(&db, "local_thread_catalog_hosts")?;
    let has_catalog = catalog.contains("thread_id");
    facts.has_catalog |= has_catalog;
    if has_catalog
        && let Some((catalog_key, host_key)) = catalog_host_join(&catalog, &hosts)
        && catalog.contains("source_kind")
        && catalog.contains("missing_candidate")
        && hosts.contains("host_kind")
    {
        let source_kind = "source_kind";
        let missing = "missing_candidate";
        let host_kind = "host_kind";
        let sql = format!(
            "SELECT c.thread_id FROM local_thread_catalog c JOIN local_thread_catalog_hosts h ON c.{catalog_key}=h.{host_key} WHERE h.{host_kind}='local' AND COALESCE(c.{source_kind},'')<>'chatgpt' AND COALESCE(c.{missing},0)=0"
        );
        let mut statement = db.prepare(&sql)?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .flatten();
        for id in ids {
            facts.roots.entry(id).or_insert(false);
        }
    }

    let thread_columns = table_columns(&db, "threads")?;
    if !thread_columns.contains("id") {
        return Ok(());
    }
    let archived = choose(&thread_columns, &["archived"], "0");
    let source = choose(&thread_columns, &["source"], "NULL");
    let sql = format!("SELECT id, COALESCE({archived},0), {source} FROM threads");
    let mut statement = db.prepare(&sql)?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1).unwrap_or_default() != 0,
            row.get::<_, Option<String>>(2).unwrap_or_default(),
        ))
    })?;
    for row in rows.flatten() {
        let (id, archived, source) = row;
        let (parent, chatgpt) = source.as_deref().map(scope_source).unwrap_or((None, false));
        if let Some(parent) = parent {
            facts.parents.insert(id.clone(), parent);
        }
        if chatgpt {
            facts.roots.remove(&id);
            continue;
        }
        if archived && !facts.parents.contains_key(&id) {
            facts.roots.insert(id.clone(), true);
        } else if !has_catalog && !facts.parents.contains_key(&id) {
            facts.roots.entry(id).or_insert(false);
        }
    }
    Ok(())
}

fn catalog_host_join(
    catalog: &HashSet<String>,
    hosts: &HashSet<String>,
) -> Option<(&'static str, &'static str)> {
    [
        ("host_id", "id"),
        ("host_id", "host_id"),
        ("thread_id", "thread_id"),
    ]
    .into_iter()
    .find(|(left, right)| catalog.contains(*left) && hosts.contains(*right))
}

fn collect_rollout_scope(path: &Path, facts: &mut ScopeFacts) -> anyhow::Result<()> {
    let file = fs::File::open(path)?;
    for line in std::io::BufReader::new(file)
        .take(MAX_ROLLOUT_SCAN_BYTES)
        .lines()
    {
        let line = line?;
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if record.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        let payload = record.get("payload").unwrap_or(&Value::Null);
        let Some(id) = payload.get("id").and_then(Value::as_str) else {
            break;
        };
        let (parent, chatgpt) = payload
            .get("source")
            .map(scope_source_value)
            .unwrap_or((None, false));
        if let Some(parent) = parent {
            facts.parents.insert(id.to_owned(), parent);
        }
        if !chatgpt
            && !facts.parents.contains_key(id)
            && (path
                .components()
                .any(|part| part.as_os_str() == "archived_sessions")
                || !facts.has_catalog)
        {
            facts.roots.insert(
                id.to_owned(),
                path.components()
                    .any(|part| part.as_os_str() == "archived_sessions"),
            );
        }
        facts.rollout_ids.push((path.to_path_buf(), id.to_owned()));
        break;
    }
    Ok(())
}

fn scope_source(source: &str) -> (Option<String>, bool) {
    serde_json::from_str::<Value>(source)
        .ok()
        .map(|value| scope_source_value(&value))
        .unwrap_or((None, source.eq_ignore_ascii_case("chatgpt")))
}

fn scope_source_value(value: &Value) -> (Option<String>, bool) {
    let parent = value
        .pointer("/subagent/thread_spawn/parent_thread_id")
        .or_else(|| value.pointer("/subagent/parent_thread_id"))
        .or_else(|| value.get("parent_thread_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let chatgpt = value == "chatgpt"
        || value
            .get("source_kind")
            .or_else(|| value.get("kind"))
            .or_else(|| value.get("type"))
            .and_then(Value::as_str)
            .is_some_and(|value| value.eq_ignore_ascii_case("chatgpt"));
    (parent, chatgpt)
}

pub fn scan(codex_home: &Path) -> RepairScan {
    let mut warnings = vec![];
    let mut omitted_warnings = 0;
    let all_rollouts = rollout_files(codex_home);
    let databases = database_paths(codex_home);
    let scope = match session_scope(&databases, &all_rollouts) {
        Ok(scope) => scope,
        Err(error) => {
            return RepairScan {
                current_provider: configured_provider(codex_home),
                targets: vec![],
                rollout_files: 0,
                session_meta_count: 0,
                databases: vec![],
                warnings: vec![format!("无法确定本地会话范围：{error}")],
            };
        }
    };
    let rollouts = all_rollouts
        .into_iter()
        .filter(|path| scope.rollout_is_eligible(path))
        .collect::<Vec<_>>();
    let mut providers = BTreeMap::<String, BTreeSet<String>>::new();
    let mut counts = BTreeMap::<String, usize>::new();
    let mut session_meta_count = 0;
    for path in &rollouts {
        match rollout_provider(path) {
            Ok(Some(provider)) => {
                session_meta_count += 1;
                *counts.entry(provider.clone()).or_default() += 1;
                providers
                    .entry(provider)
                    .or_default()
                    .insert("rollout".into());
            }
            Ok(None) => {}
            Err(error) => push_warning(
                &mut warnings,
                &mut omitted_warnings,
                format!("无法读取 {}：{error}", path.display()),
            ),
        }
    }
    let mut database_scans = vec![];
    for path in databases {
        match inspect_database(&path, &scope) {
            Ok(Some(inspection)) => {
                for (provider, provider_count) in &inspection.providers {
                    *counts.entry(provider.clone()).or_default() += *provider_count as usize;
                    providers
                        .entry(provider.clone())
                        .or_default()
                        .insert("sqlite".into());
                }
                database_scans.push(DatabaseScan {
                    path: path.display().to_string(),
                    schema: inspection.schema,
                    thread_count: inspection.thread_count,
                });
            }
            Ok(None) => {}
            Err(error) => push_warning(
                &mut warnings,
                &mut omitted_warnings,
                format!("无法检查 {}：{error}", path.display()),
            ),
        }
    }
    finish_warnings(&mut warnings, omitted_warnings);
    let current_provider = configured_provider(codex_home);
    providers
        .entry(current_provider.clone())
        .or_default()
        .insert("config".into());
    RepairScan {
        current_provider: current_provider.clone(),
        targets: providers
            .into_iter()
            .map(|(id, sources)| {
                let count = counts.get(&id).copied().unwrap_or(0);
                RepairTarget {
                    current: id == current_provider,
                    id,
                    sources: sources.into_iter().collect(),
                    count,
                }
            })
            .collect(),
        rollout_files: rollouts.len(),
        session_meta_count,
        databases: database_scans,
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
                .map(normalize_provider)
        })
        .unwrap_or_else(|| "openai".into())
}

pub fn normalize_provider(value: &str) -> String {
    if value.trim().is_empty() || value.eq_ignore_ascii_case("openai") {
        "openai".into()
    } else {
        "custom".into()
    }
}

#[cfg(test)]
pub fn repair(codex_home: &Path, target: &str) -> Result<RepairResult, AppError> {
    repair_with_history_mode_and_guard_with_paths(codex_home, target, false, None, || Ok(true))
        .map(|(result, _)| result)
}

#[cfg(test)]
pub fn repair_after_connection_switch(
    codex_home: &Path,
    target: &str,
) -> Result<RepairResult, AppError> {
    repair_with_history_mode_and_guard_with_paths(codex_home, target, true, None, || Ok(true))
        .map(|(result, _)| result)
}

#[cfg(test)]
pub(crate) fn repair_with_guard(
    codex_home: &Path,
    target: &str,
    may_write: impl FnMut() -> Result<bool, AppError>,
) -> Result<RepairResult, AppError> {
    Ok(repair_with_guard_with_paths(codex_home, target, may_write)?.0)
}

/// 与 [`repair_with_guard`] 相同，但额外返回本次实际修改过的会话文件路径，
/// 供上层只刷新受影响来源的会话索引，避免全量重建。
#[cfg(test)]
pub(crate) fn repair_with_guard_with_paths(
    codex_home: &Path,
    target: &str,
    may_write: impl FnMut() -> Result<bool, AppError>,
) -> Result<(RepairResult, Vec<PathBuf>), AppError> {
    repair_with_guard_with_paths_for_app(codex_home, target, None, may_write)
}

/// 与 [`repair_with_guard_with_paths`] 相同，但为历史迁移器指定 Codex Desktop
/// 的配置路径，避免自定义安装回退到 PATH 中的其他 CLI。
pub(crate) fn repair_with_guard_with_paths_for_app(
    codex_home: &Path,
    target: &str,
    configured_app: Option<&str>,
    may_write: impl FnMut() -> Result<bool, AppError>,
) -> Result<(RepairResult, Vec<PathBuf>), AppError> {
    repair_with_history_mode_and_guard_with_paths(
        codex_home,
        target,
        false,
        configured_app,
        may_write,
    )
}

/// 切换账号/服务后只更新会话的 provider 元数据，不重序列化 JSONL 历史。
///
/// Codex 新版用 rollout 的字节偏移和 ordinal 关联 `thread_history_1.sqlite`。
/// `repair_rollout` 适合用户主动执行的完整修复，但重序列化会改变偏移；
/// 激活切换路径必须使用这个原位版本。`openai` 和 `custom` 长度相同，因此
/// 常见的切换可以在不改变文件布局的情况下修复旧会话的路由。
///
/// 修复采用持久化清单做增量：清单命中（路径+长度+修改时间+前缀指纹一致且已
/// 指向目标 provider）的 rollout 不再读取正文；未命中或内容变化的 rollout 才
/// 流式解析。整次切换先只读预检，任一 rollout 或数据库无法确认可修复时在写入
/// 前整体终止；写入阶段任何一步失败都会用备份恢复所有已修改文件并返回错误，
/// 让上层回滚连接切换，绝不返回部分成功。
#[cfg(test)]
pub(crate) fn repair_after_connection_switch_preserving_history_with_guard(
    codex_home: &Path,
    target: &str,
    may_write: impl FnMut() -> Result<bool, AppError>,
) -> Result<RepairResult, AppError> {
    repair_after_connection_switch_preserving_history_with_guard_at_with_paths_for_app(
        codex_home, target, None, None, may_write,
    )
    .map(|(result, _)| result)
}

#[cfg(test)]
pub(crate) fn repair_after_connection_switch_preserving_history_with_guard_at(
    codex_home: &Path,
    target: &str,
    manifest_path: Option<&Path>,
    may_write: impl FnMut() -> Result<bool, AppError>,
) -> Result<RepairResult, AppError> {
    repair_after_connection_switch_preserving_history_with_guard_at_with_paths(
        codex_home,
        target,
        manifest_path,
        may_write,
    )
    .map(|(result, _)| result)
}

/// 与 [`repair_after_connection_switch_preserving_history_with_guard_at`] 相同，
/// 但额外返回本次实际修改过的 rollout 与数据库路径，供上层只刷新受影响来源。
#[cfg(test)]
pub(crate) fn repair_after_connection_switch_preserving_history_with_guard_at_with_paths(
    codex_home: &Path,
    target: &str,
    manifest_path: Option<&Path>,
    may_write: impl FnMut() -> Result<bool, AppError>,
) -> Result<(RepairResult, Vec<PathBuf>), AppError> {
    repair_after_connection_switch_preserving_history_with_guard_at_with_paths_for_app(
        codex_home,
        target,
        manifest_path,
        None,
        may_write,
    )
}

/// 与 [`repair_after_connection_switch_preserving_history_with_guard_at_with_paths`] 相同，
/// 但历史迁移器将使用传入的已配置 Codex Desktop 路径定位内置 CLI。
pub(crate) fn repair_after_connection_switch_preserving_history_with_guard_at_with_paths_for_app(
    codex_home: &Path,
    target: &str,
    manifest_path: Option<&Path>,
    configured_app: Option<&str>,
    mut may_write: impl FnMut() -> Result<bool, AppError>,
) -> Result<(RepairResult, Vec<PathBuf>), AppError> {
    let started = Instant::now();
    let target = SessionRoutingTarget::parse(target)?;
    let target_provider = target.provider();
    let manifest = load_manifest(manifest_path);
    let all_rollouts = rollout_files(codex_home);
    let all_databases = database_paths(codex_home);
    let databases = repair_database_paths(codex_home, &all_databases)
        .map_err(|error| AppError::Internal(format!("会话数据库：{error}")))?;
    let scope = session_scope(&all_databases, &all_rollouts)
        .map_err(|error| AppError::Internal(format!("无法确定本地会话范围：{error}")))?;
    let rollouts = all_rollouts
        .into_iter()
        .filter(|path| scope.rollout_is_eligible(path))
        .collect::<Vec<_>>();
    let mut affected_paths: Vec<PathBuf> = Vec::new();
    let mut result = RepairResult {
        target_provider: target_provider.to_owned(),
        files_scanned: rollouts.len(),
        databases_scanned: databases.len(),
        ..RepairResult::default()
    };

    // ---- 只读预检：任一 rollout / 数据库无法确认可修复即整体终止 ----
    // 并行读取+解析各 rollout 文件，减少 I/O 等待时间。
    let planned: Vec<(PathBuf, PlannedRollout)> = std::thread::scope(|s| {
        let handles: Vec<_> = rollouts
            .iter()
            .map(|path| {
                let path = path.clone();
                let manifest = manifest.clone();
                s.spawn(move || {
                    let result =
                        plan_rollout(&path, target_provider, &manifest).map_err(|error| {
                            AppError::Internal(format!("会话文件 {}：{error}", path.display()))
                        })?;
                    Ok::<_, AppError>((path, result))
                })
            })
            .collect();
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.join() {
                Ok(Ok(result)) => results.push(result),
                Ok(Err(error)) => return Err(error),
                Err(error) => {
                    return Err(AppError::Internal(format!("会话修复线程异常：{error:?}")));
                }
            }
        }
        Ok(results)
    })?;
    let mut repair_plans: Vec<RolloutPlan> = Vec::new();
    let mut cached_entries: Vec<SessionRepairManifestEntry> = Vec::new();
    let mut recorded: Vec<RecordedRollout> = Vec::new();
    for (path, PlannedRollout { plan, cached_entry }) in planned {
        match plan {
            RolloutPlan::Cached => {
                result.files_cached += 1;
                if let Some(entry) = cached_entry {
                    cached_entries.push(entry);
                }
            }
            RolloutPlan::Matching { session_meta_count } => {
                result.files_opened += 1;
                result.files_skipped += 1;
                recorded.push(RecordedRollout {
                    path,
                    session_meta_count,
                });
            }
            plan @ RolloutPlan::Repair {
                session_meta_count, ..
            } => {
                result.files_opened += 1;
                recorded.push(RecordedRollout {
                    path,
                    session_meta_count,
                });
                repair_plans.push(plan);
            }
        }
    }

    let mut database_plans: Vec<(PathBuf, usize, String)> = Vec::new();
    for path in &databases {
        match preflight_database(path, target_provider, &scope) {
            Ok(rows) if rows > 0 => database_plans.push((path.clone(), rows, file_sha256(path)?)),
            Ok(_) => {}
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "会话数据库 {}：{error}",
                    path.display()
                )));
            }
        }
    }
    let recovery_plans = preflight_paginated_history_recovery(codex_home, &scope, &rollouts)
        .map_err(|error| AppError::Internal(format!("会话历史恢复：{error}")))?;
    let history_database = codex_home.join(THREAD_HISTORY_FILE);
    let history_database_hash = history_database
        .is_file()
        .then(|| file_sha256(&history_database))
        .transpose()?;
    // ---- 执行阶段：备份 + 写入，任一步失败整体回滚 ----
    if !repair_plans.is_empty() || !database_plans.is_empty() || !recovery_plans.is_empty() {
        if !may_write()? {
            return Err(AppError::Internal(
                "Codex 已重新运行或修复目标已变化，切换已终止。".into(),
            ));
        }
        let mut backup = RepairBackup::create()
            .map_err(|error| AppError::Internal(format!("无法创建回滚备份：{error}")))?;
        let expected_rollouts = repair_plans
            .iter()
            .filter_map(|plan| match plan {
                RolloutPlan::Repair { path, repaired, .. } => {
                    Some((path.clone(), repaired.clone()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let stable_expected_rollouts = expected_rollouts
            .iter()
            .filter(|(path, _)| !recovery_plans.iter().any(|plan| plan.path == *path))
            .cloned()
            .collect::<Vec<_>>();
        let mut backed_paths = HashSet::new();
        let commit = (|| -> anyhow::Result<()> {
            for plan in repair_plans.drain(..) {
                let RolloutPlan::Repair {
                    path,
                    original,
                    repaired,
                    original_sha256,
                    meta_count,
                    ..
                } = plan
                else {
                    unreachable!("只有需要写入的 rollout 才会进入执行阶段");
                };
                if !may_write()? {
                    anyhow::bail!("Codex 已重新运行或修复目标已变化，切换已终止并回滚。");
                }
                if file_sha256(&path)? != original_sha256 {
                    anyhow::bail!("Codex 正在更新会话 {}，切换已终止并回滚。", path.display());
                }
                backup.add_bytes(&path, &original)?;
                backed_paths.insert(path.clone());
                if !atomic_write_if_unchanged(&path, &original, &repaired)? {
                    anyhow::bail!("Codex 正在更新会话 {}，切换已终止并回滚。", path.display());
                }
                result.files_modified += 1;
                result.session_meta_updated += meta_count;
                affected_paths.push(path);
            }
            for (path, _, expected_hash) in &database_plans {
                if !may_write()? {
                    anyhow::bail!("Codex 已重新运行或修复目标已变化，切换已终止并回滚。");
                }
                if &file_sha256(path)? != expected_hash {
                    anyhow::bail!(
                        "Codex 正在更新会话数据库 {}，切换已终止并回滚。",
                        path.display()
                    );
                }
                backup.add_sqlite_database(path)?;
                backed_paths.insert(path.clone());
                let rows = repair_database_commit(path, target_provider, &scope, &mut may_write)?;
                result.rows_updated += rows;
                result.databases_updated += 1;
                affected_paths.push(path.clone());
            }
            if !recovery_plans.is_empty() {
                if let Some(expected_hash) = history_database_hash.as_ref()
                    && file_sha256(&history_database)? != *expected_hash
                {
                    anyhow::bail!("Codex 正在更新历史投影，切换已终止并回滚。");
                }
                if backed_paths.insert(history_database.clone()) {
                    backup.add_sqlite_database(&history_database)?;
                }
                let state_database = codex_home.join("state_5.sqlite");
                if backed_paths.insert(state_database.clone()) {
                    backup.add_sqlite_database(&state_database)?;
                }
                for plan in &recovery_plans {
                    if !may_write()? {
                        anyhow::bail!("Codex 已重新运行或修复目标已变化，历史恢复已终止并回滚。");
                    }
                    let current = fs::read(&plan.path)?;
                    if file_sha256_bytes(&current) != plan.original_sha256
                        && !expected_rollouts
                            .iter()
                            .any(|(path, expected)| path == &plan.path && expected == &current)
                    {
                        anyhow::bail!(
                            "Codex 正在更新会话 {}，历史恢复已终止并回滚。",
                            plan.path.display()
                        );
                    }
                    if backed_paths.insert(plan.path.clone()) {
                        backup.add_bytes(&plan.path, &current)?;
                    }
                    let legacy = mark_rollout_history_legacy(&current, &plan.thread_id)?;
                    if !atomic_write_if_unchanged(&plan.path, &current, &legacy)? {
                        anyhow::bail!(
                            "Codex 正在更新会话 {}，历史恢复已终止并回滚。",
                            plan.path.display()
                        );
                    }
                    affected_paths.push(plan.path.clone());
                }
                mark_database_history_legacy(&state_database, &recovery_plans, &mut may_write)?;
                let migration_report =
                    run_codex_history_migration(codex_home, configured_app, &recovery_plans)?;
                verify_paginated_history_recovery(
                    &state_database,
                    &history_database,
                    &recovery_plans,
                    &migration_report,
                )?;
                affected_paths.push(state_database);
                affected_paths.push(history_database.clone());
            }
            verify_repair_commit(
                &stable_expected_rollouts,
                &database_plans,
                target_provider,
                &scope,
            )?;
            Ok(())
        })();
        if let Err(error) = commit {
            let restore_error = backup.restore();
            let _ = backup.cleanup();
            return match restore_error {
                Ok(()) => Err(AppError::Internal(format!(
                    "会话归属修复未完成，已回滚全部修改：{error}"
                ))),
                Err(restore) => Err(AppError::Internal(format!(
                    "会话归属修复未完成且回滚失败：{error}（恢复错误：{restore}）"
                ))),
            };
        }
        let _ = backup.cleanup();
    }

    // ---- 成功后按修复结果重建清单，淘汰过期条目 ----
    let mut next_manifest = SessionRepairManifest {
        version: MANIFEST_VERSION,
        ..SessionRepairManifest::default()
    };
    next_manifest.entries.extend(cached_entries);
    next_manifest.entries.extend(
        recorded
            .iter()
            .map(|record| record.to_entry(target_provider)),
    );
    save_manifest(manifest_path, &next_manifest);

    result.verification_passed = result.files_failed == 0 && result.warnings.is_empty();
    result.repair_complete = result.verification_passed;
    result.elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    Ok((result, affected_paths))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SessionRepairManifestEntry {
    path: String,
    file_length: u64,
    file_modified_at_ms: Option<i64>,
    prefix_sha256: Option<String>,
    provider: String,
    target: String,
    session_meta_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SessionRepairManifest {
    version: u32,
    entries: Vec<SessionRepairManifestEntry>,
}

fn default_manifest_path() -> PathBuf {
    crate::storage::data_root().join("session_repair_manifest.json")
}

fn load_manifest(manifest_path: Option<&Path>) -> SessionRepairManifest {
    match manifest_path {
        Some(path) => load_manifest_at(path),
        None => load_manifest_at(&default_manifest_path()),
    }
}

fn load_manifest_at(path: &Path) -> SessionRepairManifest {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(|manifest: &SessionRepairManifest| manifest.version == MANIFEST_VERSION)
        .unwrap_or_default()
}

fn save_manifest(manifest_path: Option<&Path>, manifest: &SessionRepairManifest) {
    match manifest_path {
        Some(path) => save_manifest_at(path, manifest),
        None => save_manifest_at(&default_manifest_path(), manifest),
    }
}

fn save_manifest_at(path: &Path, manifest: &SessionRepairManifest) {
    let Ok(json) = serde_json::to_vec_pretty(manifest) else {
        return;
    };
    // 清单只是增量缓存；写失败不影响本次修复结果，下次切换会重建。
    let _ = atomic_write(path, &json);
}

fn rollout_prefix_sha256(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut buffer = vec![0u8; MANIFEST_PREFIX_BYTES];
    let mut filled = 0;
    while filled < MANIFEST_PREFIX_BYTES {
        let count = file.read(&mut buffer[filled..]).ok()?;
        if count == 0 {
            break;
        }
        filled += count;
    }
    let mut hasher = Sha256::new();
    hasher.update(&buffer[..filled]);
    Some(format!("{:x}", hasher.finalize()))
}

fn file_sha256(path: &Path) -> anyhow::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn file_sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug)]
struct PaginatedHistoryRecoveryPlan {
    thread_id: String,
    path: PathBuf,
    original_sha256: String,
    is_subagent: bool,
}

#[derive(Debug)]
struct RolloutRecoveryInfo {
    thread_id: String,
    history_mode: Option<String>,
    has_history: bool,
    is_subagent: bool,
}

fn preflight_paginated_history_recovery(
    codex_home: &Path,
    scope: &SessionScope,
    rollouts: &[PathBuf],
) -> anyhow::Result<Vec<PaginatedHistoryRecoveryPlan>> {
    let state_path = codex_home.join("state_5.sqlite");
    if !state_path.is_file() {
        return Ok(Vec::new());
    }
    let state = Connection::open_with_flags(
        &state_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let columns = table_columns(&state, "threads")?;
    if !columns.contains("id") || !columns.contains("history_mode") {
        return Ok(Vec::new());
    }

    let history_path = codex_home.join(THREAD_HISTORY_FILE);
    let history = if history_path.is_file() {
        Some(Connection::open_with_flags(
            &history_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?)
    } else {
        None
    };
    if let Some(history) = history.as_ref() {
        for (table, required) in [
            (
                "thread_history_projection_state",
                &[
                    "thread_id",
                    "next_rollout_byte_offset",
                    "next_rollout_ordinal",
                ][..],
            ),
            (
                "thread_items",
                &["thread_id", "rollout_ordinal", "item_json"][..],
            ),
            (
                "thread_turns",
                &[
                    "thread_id",
                    "rollout_ordinal",
                    "rollout_byte_offset",
                    "rollout_end_ordinal",
                    "rollout_end_byte_offset",
                ][..],
            ),
        ] {
            let columns = table_columns(history, table)?;
            for column in required {
                if !columns.contains(*column) {
                    anyhow::bail!("历史投影表 {table} 缺少必要列 {column}");
                }
            }
        }
    }

    let mut plans = Vec::new();
    for path in rollouts {
        if !scope.rollout_is_eligible(path) {
            continue;
        }
        let original = fs::read(path)?;
        let info = rollout_recovery_info(&original)?;
        if !scope.eligible.contains(&info.thread_id) || !info.has_history {
            continue;
        }
        let state_mode = state
            .query_row(
                "SELECT history_mode FROM threads WHERE id=?1",
                [&info.thread_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if state_mode.as_deref() != Some("paginated") {
            continue;
        }
        if info
            .history_mode
            .as_deref()
            .is_some_and(|mode| mode != "paginated")
        {
            continue;
        }
        if paginated_projection_is_valid(
            history.as_ref(),
            &info.thread_id,
            original.len() as u64,
            info.is_subagent,
        )? {
            continue;
        }
        plans.push(PaginatedHistoryRecoveryPlan {
            thread_id: info.thread_id,
            path: path.clone(),
            original_sha256: file_sha256_bytes(&original),
            is_subagent: info.is_subagent,
        });
    }
    Ok(plans)
}

fn rollout_recovery_info(original: &[u8]) -> anyhow::Result<RolloutRecoveryInfo> {
    let mut thread_id = None;
    let mut history_mode = None;
    let mut has_history = false;
    let mut is_subagent = false;
    for line in std::str::from_utf8(original)?.lines() {
        let record: Value = serde_json::from_str(line)?;
        match record.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                thread_id = record
                    .pointer("/payload/id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                history_mode = record
                    .pointer("/payload/history_mode")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                is_subagent = session_meta_parent_thread_id(&record).is_some();
            }
            Some("event_msg")
                if record.pointer("/payload/type").and_then(Value::as_str)
                    == Some("task_started") =>
            {
                has_history = true;
            }
            Some("response_item")
                if record.pointer("/payload/type").and_then(Value::as_str) == Some("message")
                    && record.pointer("/payload/role").and_then(Value::as_str) == Some("user") =>
            {
                has_history = true;
            }
            _ => {}
        }
    }
    Ok(RolloutRecoveryInfo {
        thread_id: thread_id.ok_or_else(|| anyhow::anyhow!("未找到可验证的 session_meta.id"))?,
        history_mode,
        has_history,
        is_subagent,
    })
}

/// 只识别 Codex 已知的子代理父线程字段。未知形态一律按根会话校验，避免把
/// 根会话误放宽为只需要 projection state。
fn session_meta_parent_thread_id(record: &Value) -> Option<&str> {
    [
        "/payload/parent_thread_id",
        "/parent_thread_id",
        "/payload/source/subagent/thread_spawn/parent_thread_id",
        "/payload/source/subagent/parent_thread_id",
        "/payload/source/parent_thread_id",
    ]
    .into_iter()
    .find_map(|path| record.pointer(path).and_then(Value::as_str))
    .filter(|parent| !parent.trim().is_empty())
}

fn paginated_projection_is_valid(
    history: Option<&Connection>,
    thread_id: &str,
    rollout_length: u64,
    is_subagent: bool,
) -> anyhow::Result<bool> {
    let Some(history) = history else {
        return Ok(false);
    };
    let state = history
        .query_row(
            "SELECT next_rollout_byte_offset, next_rollout_ordinal
             FROM thread_history_projection_state WHERE thread_id=?1",
            [thread_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((next_offset, next_ordinal)) = state else {
        return Ok(false);
    };
    if next_offset < 0 || next_ordinal <= 0 || next_offset as u64 > rollout_length {
        return Ok(false);
    }
    let item_count: i64 = history.query_row(
        "SELECT COUNT(*) FROM thread_items WHERE thread_id=?1",
        [thread_id],
        |row| row.get(0),
    )?;
    let (turn_count, max_end): (i64, Option<i64>) = history.query_row(
        "SELECT COUNT(*), MAX(rollout_end_byte_offset)
         FROM thread_turns WHERE thread_id=?1",
        [thread_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if item_count == 0 && turn_count == 0 {
        return Ok(is_subagent);
    }
    if item_count <= 0 || turn_count <= 0 {
        return Ok(false);
    }
    let invalid_turn_offset_count: i64 = history.query_row(
        "SELECT COUNT(*)
         FROM thread_turns
         WHERE thread_id=?1
           AND (
               rollout_byte_offset < 0
               OR rollout_end_byte_offset < rollout_byte_offset
               OR rollout_end_byte_offset < 0
               OR rollout_byte_offset > ?2
               OR rollout_end_byte_offset > ?2
           )",
        (thread_id, i64::try_from(rollout_length)?),
        |row| row.get(0),
    )?;
    Ok(invalid_turn_offset_count == 0
        && max_end.is_some_and(|value| value >= 0 && value as u64 <= rollout_length))
}

fn mark_rollout_history_legacy(
    original: &[u8],
    expected_thread_id: &str,
) -> anyhow::Result<Vec<u8>> {
    let text = std::str::from_utf8(original)?;
    let mut output = String::with_capacity(text.len());
    let mut changed = false;
    for segment in text.split_inclusive('\n') {
        let (line, ending) = segment.strip_suffix('\n').map_or((segment, ""), |line| {
            (
                line.strip_suffix('\r').unwrap_or(line),
                if line.ends_with('\r') { "\r\n" } else { "\n" },
            )
        });
        let mut record: Value = serde_json::from_str(line)?;
        let is_target_session_meta = record.get("type").and_then(Value::as_str)
            == Some("session_meta")
            && record.pointer("/payload/id").and_then(Value::as_str) == Some(expected_thread_id);
        if is_target_session_meta {
            let payload = record
                .get_mut("payload")
                .and_then(Value::as_object_mut)
                .ok_or_else(|| anyhow::anyhow!("session_meta.payload 结构未知"))?;
            if payload
                .get("history_mode")
                .and_then(Value::as_str)
                .is_some_and(|mode| mode != "paginated")
            {
                anyhow::bail!("会话不是可恢复的 paginated 历史");
            }
            payload.insert("history_mode".into(), Value::String("legacy".into()));
            output.push_str(&serde_json::to_string(&record)?);
            output.push_str(ending);
            changed = true;
        } else {
            output.push_str(segment);
        }
    }
    if !changed {
        anyhow::bail!("未找到可恢复的 session_meta.history_mode");
    }
    Ok(output.into_bytes())
}

fn mark_database_history_legacy(
    state_path: &Path,
    plans: &[PaginatedHistoryRecoveryPlan],
    may_write: &mut impl FnMut() -> Result<bool, AppError>,
) -> anyhow::Result<()> {
    let mut state = Connection::open(state_path)?;
    if !table_columns(&state, "threads")?.contains("history_mode") {
        anyhow::bail!("state_5.sqlite 缺少 history_mode 列");
    }
    let transaction = state.transaction()?;
    for plan in plans {
        if !may_write()? {
            anyhow::bail!("Codex 已重新运行或修复目标已变化，历史恢复已终止并回滚。");
        }
        let rows = transaction.execute(
            "UPDATE threads SET history_mode='legacy'
             WHERE id=?1 AND history_mode='paginated'",
            [&plan.thread_id],
        )?;
        if rows != 1 {
            anyhow::bail!("会话 {} 的 history_mode 已并发变化", plan.thread_id);
        }
    }
    if !may_write()? {
        anyhow::bail!("Codex 已重新运行或修复目标已变化，历史恢复已终止并回滚。");
    }
    transaction.commit()?;
    Ok(())
}

#[derive(Deserialize)]
struct MigrationReport {
    outcomes: Vec<MigrationOutcome>,
}

#[derive(Deserialize)]
struct MigrationOutcome {
    thread_id: String,
    status: String,
    message: Option<String>,
}

fn run_codex_history_migration(
    codex_home: &Path,
    configured_app: Option<&str>,
    plans: &[PaginatedHistoryRecoveryPlan],
) -> anyhow::Result<MigrationReport> {
    let output = codex_history_migration_command(codex_home, configured_app, plans)?
        .output()
        .map_err(|error| anyhow::anyhow!("无法启动 Codex 历史迁移器：{error}"))?;
    if !output.status.success() {
        anyhow::bail!(
            "Codex 历史迁移器失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let report: MigrationReport = serde_json::from_slice(&output.stdout)
        .map_err(|error| anyhow::anyhow!("无法解析 Codex 历史迁移结果：{error}"))?;
    for plan in plans {
        let outcome = report
            .outcomes
            .iter()
            .find(|outcome| outcome.thread_id == plan.thread_id)
            .ok_or_else(|| anyhow::anyhow!("迁移器未返回会话 {}", plan.thread_id))?;
        if outcome.status != "migrated" {
            anyhow::bail!(
                "会话 {} 未完成历史迁移：{}{}",
                plan.thread_id,
                outcome.status,
                outcome
                    .message
                    .as_deref()
                    .map(|message| format!("（{message}）"))
                    .unwrap_or_default()
            );
        }
    }
    Ok(report)
}

fn codex_history_migration_command(
    codex_home: &Path,
    configured_app: Option<&str>,
    plans: &[PaginatedHistoryRecoveryPlan],
) -> anyhow::Result<std::process::Command> {
    let mut command = platform::codex_cli_command_for_app(configured_app)
        .map_err(|error| anyhow::anyhow!("无法启动 Codex 历史迁移器：{error}"))?;
    command
        .env("CODEX_HOME", codex_home)
        .arg("migrate-rollouts")
        .arg("--apply")
        .arg("--json");
    for plan in plans {
        command.arg("--thread").arg(&plan.thread_id);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(command)
}

fn verify_paginated_history_recovery(
    state_path: &Path,
    history_path: &Path,
    plans: &[PaginatedHistoryRecoveryPlan],
    migration_report: &MigrationReport,
) -> anyhow::Result<()> {
    let state = Connection::open_with_flags(
        state_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let history = Connection::open_with_flags(
        history_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    for plan in plans {
        let outcome = migration_report
            .outcomes
            .iter()
            .find(|outcome| outcome.thread_id == plan.thread_id)
            .ok_or_else(|| anyhow::anyhow!("迁移器未返回会话 {}", plan.thread_id))?;
        if outcome.status != "migrated" {
            anyhow::bail!(
                "会话 {} 未完成历史迁移：{}{}",
                plan.thread_id,
                outcome.status,
                outcome
                    .message
                    .as_deref()
                    .map(|message| format!("（{message}）"))
                    .unwrap_or_default()
            );
        }
        let mode: Option<String> = state
            .query_row(
                "SELECT history_mode FROM threads WHERE id=?1",
                [&plan.thread_id],
                |row| row.get(0),
            )
            .optional()?;
        let rollout_length = fs::metadata(&plan.path)?.len();
        if !paginated_projection_is_valid(
            Some(&history),
            &plan.thread_id,
            rollout_length,
            plan.is_subagent,
        )? {
            anyhow::bail!("会话 {} 的分页历史投影仍不可用", plan.thread_id);
        }
        match mode.as_deref() {
            Some("paginated") => {}
            Some("legacy") => {}
            Some(other) => anyhow::bail!(
                "会话 {} 迁移后 history_mode 不受支持：{other}",
                plan.thread_id
            ),
            None => anyhow::bail!("会话 {} 迁移后缺少 history_mode", plan.thread_id),
        }
    }
    Ok(())
}

fn entry_matches_file(entry: &SessionRepairManifestEntry, path: &Path, target: &str) -> bool {
    if entry.path.as_str() != path.to_string_lossy().as_ref()
        || entry.target != target
        || entry.provider != target
    {
        return false;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if entry.file_length != metadata.len() {
        return false;
    }
    let modified_at_ms = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_millis()).ok());
    if entry.file_modified_at_ms != modified_at_ms {
        return false;
    }
    entry.prefix_sha256.as_deref() == rollout_prefix_sha256(path).as_deref()
}

struct PlannedRollout {
    plan: RolloutPlan,
    cached_entry: Option<SessionRepairManifestEntry>,
}

fn plan_rollout(
    path: &Path,
    target: &str,
    manifest: &SessionRepairManifest,
) -> anyhow::Result<PlannedRollout> {
    if let Some(entry) = manifest
        .entries
        .iter()
        .find(|entry| entry_matches_file(entry, path, target))
    {
        return Ok(PlannedRollout {
            plan: RolloutPlan::Cached,
            cached_entry: Some(entry.clone()),
        });
    }
    let analysis = analyze_rollout_metadata_in_place(path, target)?;
    Ok(PlannedRollout {
        plan: match analysis.write {
            Some(write) => RolloutPlan::Repair {
                path: path.to_path_buf(),
                original: write.original,
                repaired: write.repaired,
                original_sha256: write.original_sha256,
                meta_count: write.meta_count,
                session_meta_count: analysis.session_meta_count,
            },
            None => RolloutPlan::Matching {
                session_meta_count: analysis.session_meta_count,
            },
        },
        cached_entry: None,
    })
}

#[derive(Debug)]
enum RolloutPlan {
    Cached,
    Matching {
        session_meta_count: usize,
    },
    Repair {
        path: PathBuf,
        original: Vec<u8>,
        repaired: Vec<u8>,
        original_sha256: String,
        meta_count: usize,
        session_meta_count: usize,
    },
}

struct RecordedRollout {
    path: PathBuf,
    session_meta_count: usize,
}

impl RecordedRollout {
    fn to_entry(&self, target: &str) -> SessionRepairManifestEntry {
        let metadata = fs::metadata(&self.path).ok();
        let file_length = metadata.as_ref().map(|meta| meta.len()).unwrap_or(0);
        let file_modified_at_ms = metadata
            .as_ref()
            .and_then(|meta| meta.modified().ok())
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|duration| i64::try_from(duration.as_millis()).ok());
        SessionRepairManifestEntry {
            path: self.path.to_string_lossy().into_owned(),
            file_length,
            file_modified_at_ms,
            prefix_sha256: rollout_prefix_sha256(&self.path),
            provider: target.to_owned(),
            target: target.to_owned(),
            session_meta_count: self.session_meta_count,
        }
    }
}

struct BackupEntry {
    original: PathBuf,
    backup: PathBuf,
}

struct RepairBackup {
    dir: PathBuf,
    entries: Vec<BackupEntry>,
}

impl RepairBackup {
    fn create() -> anyhow::Result<Self> {
        let dir = std::env::temp_dir().join(format!(
            "codex-tools-session-repair-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            entries: Vec::new(),
        })
    }

    fn add_bytes(&mut self, original: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        let backup = self.dir.join(format!("{:04}.bak", self.entries.len()));
        fs::write(&backup, bytes)?;
        self.entries.push(BackupEntry {
            original: original.to_path_buf(),
            backup,
        });
        Ok(())
    }

    /// SQLite 的提交可能依赖 WAL/SHM；主文件单独备份会在回滚时丢失尚未
    /// checkpoint 的页。把存在的 sidecar 一并记录，原来不存在的也记录下来，
    /// 以便恢复时清除本次写入新建的 sidecar。
    fn add_sqlite_database(&mut self, original: &Path) -> anyhow::Result<()> {
        self.add_optional_file(original)?;
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = original.as_os_str().to_os_string();
            sidecar.push(suffix);
            self.add_optional_file(Path::new(&sidecar))?;
        }
        Ok(())
    }

    fn add_optional_file(&mut self, original: &Path) -> anyhow::Result<()> {
        let backup = self.dir.join(format!("{:04}.bak", self.entries.len()));
        if original.exists() {
            fs::copy(original, &backup)?;
        }
        self.entries.push(BackupEntry {
            original: original.to_path_buf(),
            backup,
        });
        Ok(())
    }

    fn restore(&self) -> anyhow::Result<()> {
        for entry in self.entries.iter().rev() {
            if entry.backup.exists() {
                fs::copy(&entry.backup, &entry.original)?;
            } else if entry.original.exists() {
                fs::remove_file(&entry.original)?;
            }
        }
        Ok(())
    }

    fn cleanup(&self) -> anyhow::Result<()> {
        fs::remove_dir_all(&self.dir)?;
        Ok(())
    }
}

fn verify_repair_commit(
    expected_rollouts: &[(PathBuf, Vec<u8>)],
    database_plans: &[(PathBuf, usize, String)],
    target: &str,
    scope: &SessionScope,
) -> anyhow::Result<()> {
    for (path, expected) in expected_rollouts {
        if fs::read(path)? != *expected {
            anyhow::bail!("会话文件 {} 在写入后发生变化", path.display());
        }
        verify_rollout_route(path, target)?;
    }
    for (path, _, _) in database_plans {
        verify_database_route(path, target, scope)?;
    }
    Ok(())
}

fn verify_rollout_route(path: &Path, target: &str) -> anyhow::Result<()> {
    let bytes = fs::read(path)?;
    for line in std::str::from_utf8(&bytes)?.lines() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let Some((_, provider)) = provider_metadata_field(&record)
            && provider != target
        {
            anyhow::bail!("会话文件 {} 仍包含旧 provider", path.display());
        }
    }
    Ok(())
}

fn verify_database_route(path: &Path, target: &str, scope: &SessionScope) -> anyhow::Result<()> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let Some((table, id, columns)) = session_table(&db)? else {
        anyhow::bail!("会话数据库格式在写入后发生变化");
    };
    for ids in scope.eligible_ids().chunks(900) {
        if ids.is_empty() {
            continue;
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let model_violation = if columns.contains("model") {
            " OR model IS NOT NULL"
        } else {
            ""
        };
        let sql = format!(
            "SELECT COUNT(*) FROM {table} WHERE {id} IN ({placeholders}) AND (COALESCE(model_provider,'')<>?{model_violation})"
        );
        let mut params = Vec::<&dyn rusqlite::ToSql>::with_capacity(ids.len() + 1);
        params.extend(ids.iter().map(|id| id as &dyn rusqlite::ToSql));
        params.push(&target);
        let remaining: i64 = db.query_row(&sql, params.as_slice(), |row| row.get(0))?;
        if remaining != 0 {
            anyhow::bail!("会话数据库 {} 未完全写入目标路由", path.display());
        }
    }
    Ok(())
}

fn eligible_database_ids(
    db: &Connection,
    table: &str,
    id_column: &str,
    scope: &SessionScope,
) -> anyhow::Result<Vec<String>> {
    let mut output = Vec::new();
    for ids in scope.eligible_ids().chunks(900) {
        if ids.is_empty() {
            continue;
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!("SELECT {id_column} FROM {table} WHERE {id_column} IN ({placeholders})");
        let mut statement = db.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            row.get::<_, String>(0)
        })?;
        output.extend(rows.flatten());
    }
    Ok(output)
}

fn eligible_database_changes(
    db: &Connection,
    table: &str,
    id_column: &str,
    target: &str,
    scope: &SessionScope,
) -> anyhow::Result<Vec<String>> {
    let mut output = Vec::new();
    for ids in scope.eligible_ids().chunks(900) {
        if ids.is_empty() {
            continue;
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let model_change = if table_columns(db, table)?.contains("model") {
            " OR model IS NOT NULL"
        } else {
            ""
        };
        let sql = format!(
            "SELECT {id_column} FROM {table} WHERE (COALESCE(model_provider,'')<>?1{model_change}) AND {id_column} IN ({placeholders})"
        );
        let mut params = Vec::<&dyn rusqlite::ToSql>::with_capacity(ids.len() + 1);
        params.push(&target);
        params.extend(ids.iter().map(|id| id as &dyn rusqlite::ToSql));
        let mut statement = db.prepare(&sql)?;
        let rows = statement.query_map(params.as_slice(), |row| row.get::<_, String>(0))?;
        output.extend(rows.flatten());
    }
    Ok(output)
}

fn preflight_database(path: &Path, target: &str, scope: &SessionScope) -> anyhow::Result<usize> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let Some((table, id, columns)) = session_table(&db)? else {
        anyhow::bail!("未知的 Codex 会话数据库结构（未找到 threads/local_thread_catalog 表）");
    };
    if !columns.contains("model_provider") {
        anyhow::bail!("会话数据库格式不受支持，缺少 model_provider 列");
    }
    Ok(eligible_database_changes(&db, table, id, target, scope)?.len())
}

fn repair_database_commit(
    path: &Path,
    target: &str,
    scope: &SessionScope,
    may_write: &mut impl FnMut() -> Result<bool, AppError>,
) -> anyhow::Result<usize> {
    let mut db = Connection::open(path)?;
    let Some((table, id, columns)) = session_table(&db)? else {
        anyhow::bail!("未知的 Codex 会话数据库结构（未找到 threads/local_thread_catalog 表）");
    };
    if !columns.contains("model_provider") {
        anyhow::bail!("会话数据库格式不受支持，缺少 model_provider 列");
    }
    let ids = eligible_database_changes(&db, table, id, target, scope)?;
    let clears_model = columns.contains("model");
    let transaction = db.transaction()?;
    let mut rows = 0;
    for ids in ids.chunks(900) {
        if !may_write()? {
            anyhow::bail!("Codex 已重新运行或修复目标已变化，切换已终止并回滚。");
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let model_assignment = if clears_model { ", model=NULL" } else { "" };
        let model_change = if clears_model {
            " OR model IS NOT NULL"
        } else {
            ""
        };
        let sql = format!(
            "UPDATE {table} SET model_provider=?1{model_assignment} WHERE (COALESCE(model_provider,'')<>?1{model_change}) AND {id} IN ({placeholders})"
        );
        let mut params = Vec::<&dyn rusqlite::ToSql>::with_capacity(ids.len() + 1);
        params.push(&target);
        params.extend(ids.iter().map(|id| id as &dyn rusqlite::ToSql));
        rows += transaction.execute(&sql, params.as_slice())?;
    }
    if !may_write()? {
        anyhow::bail!("Codex 已重新运行或修复目标已变化，切换已终止并回滚。");
    }
    transaction.commit()?;
    Ok(rows)
}

/// 激活修复只处理真正保存会话的数据库。`state_5.sqlite` 是根状态库，若它存在
/// 却无法识别为会话表则必须 fail closed；sqlite 目录中的辅助库可安全跳过。
fn repair_database_paths(codex_home: &Path, paths: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let root_state = codex_home.join("state_5.sqlite");
    let mut output = Vec::new();
    for path in paths {
        let db = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        if session_table(&db)?.is_some() {
            output.push(path.clone());
        } else if path == &root_state {
            anyhow::bail!("未知的 Codex 会话数据库结构（未找到 threads/local_thread_catalog 表）");
        }
    }
    Ok(output)
}

fn repair_with_history_mode_and_guard_with_paths(
    codex_home: &Path,
    target: &str,
    _force_portable_history: bool,
    configured_app: Option<&str>,
    may_write: impl FnMut() -> Result<bool, AppError>,
) -> Result<(RepairResult, Vec<PathBuf>), AppError> {
    // 手动 IPC 与连接激活共用同一 fail-closed 原子引擎，不能让手动修复留下
    // 只改 JSONL、未同步 state/history SQLite 的半修复状态。
    repair_after_connection_switch_preserving_history_with_guard_at_with_paths_for_app(
        codex_home,
        target,
        None,
        configured_app,
        may_write,
    )
}

struct RolloutWrite {
    original: Vec<u8>,
    repaired: Vec<u8>,
    original_sha256: String,
    meta_count: usize,
}

struct RolloutAnalysis {
    session_meta_count: usize,
    write: Option<RolloutWrite>,
}

/// 流式分析一个 rollout：只读，不写盘。返回需要原位修改的字节与统计。
/// provider 名称长度不同时返回错误，由调用方在写入前整体终止切换。
fn analyze_rollout_metadata_in_place(path: &Path, target: &str) -> anyhow::Result<RolloutAnalysis> {
    if fs::metadata(path)?.len() > MAX_REPAIR_ROLLOUT_BYTES {
        anyhow::bail!("会话文件超过 256 MB，已跳过以避免占用过多内存");
    }
    let original = fs::read(path)?;
    let mut output = original.clone();
    let mut session_meta_count = 0;
    let mut meta_count = 0;
    let mut saw_session_meta = false;
    let mut offset = 0;

    for segment in original.split_inclusive(|byte| *byte == b'\n') {
        let line_len = segment.len();
        let line = segment.strip_suffix(b"\n").unwrap_or(segment);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let record = serde_json::from_slice::<Value>(line)
            .map_err(|error| anyhow::anyhow!("未知 JSONL 结构，拒绝原位写入：{error}"))?;
        let record_type = record.get("type").and_then(Value::as_str);
        if record_type == Some("session_meta") {
            saw_session_meta = true;
            session_meta_count += 1;
            if record
                .pointer("/payload/id")
                .and_then(Value::as_str)
                .is_none()
            {
                anyhow::bail!("session_meta 缺少 id，拒绝原位写入");
            }
        }
        let known_metadata = matches!(record_type, Some("session_meta") | Some("turn_context"))
            || (record_type == Some("event_msg")
                && record.pointer("/payload/type").and_then(Value::as_str)
                    == Some("thread_settings_applied"));
        if !known_metadata {
            offset += line_len;
            continue;
        }
        // legacy rollout 的 turn_context 和 thread_settings_applied 记录经常
        // 没有 provider 字段；它们仍是合法历史，不能因为缺少可更新字段而
        // 阻断整个会话。session_meta 则必须包含 provider，才能确认路由目标。
        let Some((field, provider)) = provider_metadata_field(&record) else {
            if record_type == Some("session_meta") {
                anyhow::bail!("session_meta 缺少可原位更新的 model_provider 字段");
            }
            offset += line_len;
            continue;
        };
        let (value_start, value_end) = unique_json_string_field_range(line, field)
            .ok_or_else(|| anyhow::anyhow!("会话元数据结构未知，拒绝原位写入"))?;
        let current = &line[value_start..value_end];
        if current != provider.as_bytes() {
            anyhow::bail!("会话元数据与原始字节不一致，拒绝原位写入");
        }
        let target_bytes = target.as_bytes();
        if value_end - value_start != target_bytes.len() {
            anyhow::bail!("旧 provider 与目标 provider 长度不同，拒绝原位写入");
        }
        if provider != target {
            let output_start = offset + value_start;
            output[output_start..output_start + target_bytes.len()].copy_from_slice(target_bytes);
            meta_count += 1;
        }
        offset += line_len;
    }

    if !saw_session_meta {
        anyhow::bail!("未找到可验证的 session_meta，拒绝原位写入");
    }
    let write = if meta_count > 0 {
        Some(RolloutWrite {
            original_sha256: {
                let mut hasher = Sha256::new();
                hasher.update(&original);
                format!("{:x}", hasher.finalize())
            },
            original,
            repaired: output,
            meta_count,
        })
    } else {
        None
    };
    Ok(RolloutAnalysis {
        session_meta_count,
        write,
    })
}

fn provider_metadata_field(record: &Value) -> Option<(&'static [u8], &str)> {
    let payload = record.get("payload")?;
    match record.get("type").and_then(Value::as_str) {
        Some("session_meta") => payload
            .get("model_provider")
            .or_else(|| payload.get("model_provider_id"))
            .and_then(Value::as_str)
            .map(|provider| {
                if payload.get("model_provider").is_some() {
                    (b"model_provider".as_slice(), provider)
                } else {
                    (b"model_provider_id".as_slice(), provider)
                }
            }),
        Some("turn_context") => payload
            .get("model_provider")
            .or_else(|| payload.get("model_provider_id"))
            .and_then(Value::as_str)
            .map(|provider| {
                if payload.get("model_provider").is_some() {
                    (b"model_provider".as_slice(), provider)
                } else {
                    (b"model_provider_id".as_slice(), provider)
                }
            }),
        Some("event_msg")
            if payload.get("type").and_then(Value::as_str) == Some("thread_settings_applied") =>
        {
            let settings = payload.get("thread_settings")?;
            settings
                .get("model_provider_id")
                .or_else(|| settings.get("model_provider"))
                .and_then(Value::as_str)
                .map(|provider| {
                    if settings.get("model_provider_id").is_some() {
                        (b"model_provider_id".as_slice(), provider)
                    } else {
                        (b"model_provider".as_slice(), provider)
                    }
                })
        }
        _ => None,
    }
}

/// Only accept a single textual occurrence of the expected key.  This avoids
/// guessing which duplicate/nested key a JSON parser selected and keeps the
/// write set limited to the provider value bytes.
fn unique_json_string_field_range(line: &[u8], field: &[u8]) -> Option<(usize, usize)> {
    let mut cursor = 0;
    let mut found = None;
    let mut key = Vec::with_capacity(field.len() + 2);
    key.push(b'"');
    key.extend_from_slice(field);
    key.push(b'"');
    while cursor + key.len() <= line.len() {
        let Some(relative) = line[cursor..]
            .windows(key.len())
            .position(|window| window == key.as_slice())
        else {
            break;
        };
        let key_start = cursor + relative;
        let mut value_start = key_start + key.len();
        while line.get(value_start).is_some_and(u8::is_ascii_whitespace) {
            value_start += 1;
        }
        if line.get(value_start) != Some(&b':') {
            cursor = key_start + key.len();
            continue;
        }
        value_start += 1;
        while line.get(value_start).is_some_and(u8::is_ascii_whitespace) {
            value_start += 1;
        }
        if line.get(value_start) != Some(&b'"') {
            cursor = key_start + key.len();
            continue;
        }
        let mut value_end = value_start + 1;
        let mut escaped = false;
        while let Some(byte) = line.get(value_end) {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                if found.replace((value_start + 1, value_end)).is_some() {
                    return None;
                }
                cursor = value_end + 1;
                break;
            }
            value_end += 1;
        }
        if value_end >= line.len() {
            return None;
        }
    }
    found
}

pub fn list_database_sessions_from_paths(
    paths: &[PathBuf],
    scope: &SessionScope,
) -> anyhow::Result<Vec<SessionSummary>> {
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
            let archived = scope.root_archived(&id);
            let provider = normalize_provider(&row.get::<_, String>(2).unwrap_or_default());
            Ok(archived.map(|archived| SessionSummary {
                identity: format!("{}#{id}", path.display()),
                id,
                title: row.get(1).unwrap_or_default(),
                provider: provider.clone(),
                cwd: row.get(3).unwrap_or_default(),
                archived,
                updated_at: row.get(5).unwrap_or_default(),
                source_db: path.display().to_string(),
                source_rollout: None,
                original_provider: provider,
                has_user_event: false,
            }))
        })?;
        sessions.extend(rows.flatten().flatten());
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

#[cfg(test)]
fn repair_database(
    path: &Path,
    target: &str,
    scope: &SessionScope,
    may_write: &mut impl FnMut() -> Result<bool, AppError>,
) -> anyhow::Result<Option<usize>> {
    let mut db = Connection::open(path)?;
    let Some((table, id, columns)) = session_table(&db)? else {
        return Ok(Some(0));
    };
    if !columns.contains("model_provider") {
        return Ok(Some(0));
    }
    if !may_write()? {
        return Ok(None);
    }
    let ids = eligible_database_changes(&db, table, id, target, scope)?;
    let transaction = db.transaction()?;
    let mut rows = 0;
    for ids in ids.chunks(900) {
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE {table} SET model_provider=?1 WHERE COALESCE(model_provider,'')<>?1 AND {id} IN ({placeholders})"
        );
        let mut params = Vec::<&dyn rusqlite::ToSql>::with_capacity(ids.len() + 1);
        params.push(&target);
        params.extend(ids.iter().map(|id| id as &dyn rusqlite::ToSql));
        rows += transaction.execute(&sql, params.as_slice())?;
    }
    if !may_write()? {
        return Ok(None);
    }
    transaction.commit()?;
    Ok(Some(rows))
}

fn inspect_database(
    path: &Path,
    scope: &SessionScope,
) -> anyhow::Result<Option<DatabaseInspection>> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let Some((table, id, columns)) = session_table(&db)? else {
        return Ok(None);
    };
    if !columns.contains("model_provider") {
        anyhow::bail!("会话数据库格式不受支持，无法更新归属信息");
    }
    let ids = eligible_database_ids(&db, table, id, scope)?;
    let count = ids.len() as u64;
    let mut providers = BTreeMap::<String, u64>::new();
    for ids in ids.chunks(900) {
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT model_provider, count(*) FROM {table} WHERE model_provider IS NOT NULL AND {id} IN ({placeholders}) GROUP BY model_provider"
        );
        let mut statement = db.prepare(&sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
        })?;
        for (provider, count) in rows.flatten() {
            *providers.entry(provider).or_default() += count;
        }
    }
    Ok(Some(DatabaseInspection {
        schema: table.into(),
        thread_count: count,
        providers: providers.into_iter().collect(),
    }))
}

struct DatabaseInspection {
    schema: String,
    thread_count: u64,
    providers: Vec<(String, u64)>,
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
    let file_size = fs::metadata(path)?.len();
    for line in std::io::BufReader::new(fs::File::open(path)?)
        .take(MAX_ROLLOUT_SCAN_BYTES)
        .lines()
    {
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
    if file_size > MAX_ROLLOUT_SCAN_BYTES {
        anyhow::bail!("前 2 MB 内没有找到会话元数据，已停止继续扫描");
    }
    Ok(None)
}

fn push_warning(warnings: &mut Vec<String>, omitted: &mut usize, warning: String) {
    if warnings.len() < MAX_REPAIR_WARNINGS.saturating_sub(1) {
        warnings.push(warning.chars().take(MAX_WARNING_CHARS).collect());
    } else {
        *omitted += 1;
    }
}

fn finish_warnings(warnings: &mut Vec<String>, omitted: usize) {
    if omitted > 0 {
        warnings.push(format!("另有 {omitted} 项警告未显示。"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn history_migration_command_uses_configured_desktop_cli() {
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("Custom/Codex.exe");
        let cli = temp.path().join("Custom/resources/codex.exe");
        fs::create_dir_all(cli.parent().unwrap()).unwrap();
        fs::write(&app, b"desktop").unwrap();
        fs::write(&cli, b"cli").unwrap();
        let plans = [PaginatedHistoryRecoveryPlan {
            thread_id: "thread-one".into(),
            path: temp.path().join("rollout.jsonl"),
            original_sha256: "unused".into(),
            is_subagent: false,
        }];

        let command =
            codex_history_migration_command(temp.path(), Some(app.to_str().unwrap()), &plans)
                .unwrap();
        if let Some(configured) = std::env::var_os("CODEX_BIN") {
            assert_eq!(command.get_program(), configured);
        } else {
            assert_eq!(
                fs::canonicalize(command.get_program()).unwrap(),
                fs::canonicalize(cli).unwrap()
            );
        }
        assert!(
            command
                .get_args()
                .any(|argument| argument == "--thread" || argument == "thread-one")
        );
    }

    #[cfg(windows)]
    #[test]
    fn history_migration_command_reports_missing_configured_cli() {
        if std::env::var_os("CODEX_BIN").is_some() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let app = temp.path().join("Custom/Codex.exe");
        fs::create_dir_all(app.parent().unwrap()).unwrap();
        fs::write(&app, b"desktop").unwrap();
        let plans = [PaginatedHistoryRecoveryPlan {
            thread_id: "thread-one".into(),
            path: temp.path().join("rollout.jsonl"),
            original_sha256: "unused".into(),
            is_subagent: false,
        }];

        let error =
            codex_history_migration_command(temp.path(), Some(app.to_str().unwrap()), &plans)
                .unwrap_err();
        assert!(error.to_string().contains("无法定位 Codex 内置 CLI"));
        assert!(error.to_string().contains("resources"));
    }

    #[test]
    fn scope_repairs_only_local_roots_and_their_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::create_dir_all(home.join("archived_sessions")).unwrap();
        let database = home.join("state_5.sqlite");
        let db = Connection::open(&database).unwrap();
        db.execute_batch(
            "CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT, source TEXT, archived INTEGER, title TEXT, cwd TEXT);
             CREATE TABLE local_thread_catalog(thread_id TEXT, host_id TEXT, source_kind TEXT, missing_candidate INTEGER);
             CREATE TABLE local_thread_catalog_hosts(id TEXT, host_kind TEXT);
             INSERT INTO local_thread_catalog_hosts VALUES('local-host','local'),('web-host','local');
             INSERT INTO local_thread_catalog VALUES('active','local-host','local',0),('web','web-host','chatgpt',0),('deleted','local-host','local',1);
             INSERT INTO threads VALUES
               ('active','openai',NULL,0,'active','C:/active'),
               ('active-child','openai','{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"active\"}}}',0,'child','C:/child'),
               ('archived','openai',NULL,1,'archived','C:/archived'),
               ('archived-child','openai','{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"archived\"}}}',1,'archived child','C:/archived'),
               ('web','openai','{\"source_kind\":\"chatgpt\"}',0,'web','C:/web'),
               ('deleted','openai',NULL,0,'deleted','C:/deleted'),
               ('deleted-child','openai','{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"deleted\"}}}',0,'deleted child','C:/deleted'),
               ('orphan','openai','{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"missing\"}}}',0,'orphan','C:/orphan'),
               ('cycle-a','openai','{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"cycle-b\"}}}',0,'cycle a','C:/cycle'),
               ('cycle-b','openai','{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"cycle-a\"}}}',0,'cycle b','C:/cycle');",
        )
        .unwrap();
        drop(db);

        let write_rollout = |path: &Path, id: &str, parent: Option<&str>| {
            let source = parent.map(|parent| serde_json::json!({"subagent":{"thread_spawn":{"parent_thread_id":parent}}}));
            fs::write(path, format!("{}\n", serde_json::json!({"type":"session_meta","payload":{"id":id,"model_provider":"openai","source":source}}))).unwrap();
        };
        let active = home.join("sessions/active.jsonl");
        let active_child = home.join("sessions/active-child.jsonl");
        let archived = home.join("archived_sessions/archived.jsonl");
        let web = home.join("sessions/web.jsonl");
        let deleted = home.join("sessions/deleted.jsonl");
        write_rollout(&active, "active", None);
        write_rollout(&active_child, "active-child", Some("active"));
        write_rollout(&archived, "archived", None);
        write_rollout(&web, "web", None);
        write_rollout(&deleted, "deleted", None);
        let before_web = fs::read(&web).unwrap();
        let before_deleted = fs::read(&deleted).unwrap();
        let rollouts = rollout_files(&home);
        let scope = session_scope(std::slice::from_ref(&database), &rollouts).unwrap();

        assert_eq!(scope.root_archived("active"), Some(false));
        assert_eq!(scope.root_archived("archived"), Some(true));
        assert!(scope.contains("active-child"));
        for id in [
            "web",
            "deleted",
            "deleted-child",
            "orphan",
            "cycle-a",
            "cycle-b",
        ] {
            assert!(
                !scope.contains(id),
                "{id} must remain outside the repair scope"
            );
        }

        let (result, _) =
            repair_after_connection_switch_preserving_history_with_guard_at_with_paths(
                &home,
                "custom",
                Some(&temp.path().join("manifest.json")),
                || Ok(true),
            )
            .unwrap();
        assert_eq!(result.files_modified, 3);
        assert_eq!(result.rows_updated, 4);
        assert_eq!(fs::read(&web).unwrap(), before_web);
        assert_eq!(fs::read(&deleted).unwrap(), before_deleted);
        let db = Connection::open(database).unwrap();
        let provider = |id: &str| {
            db.query_row(
                "SELECT model_provider FROM threads WHERE id=?1",
                [id],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        for id in ["active", "active-child", "archived", "archived-child"] {
            assert_eq!(provider(id), "custom");
        }
        for id in ["web", "deleted"] {
            assert_eq!(provider(id), "openai");
        }
    }

    #[test]
    fn repair_unifies_all_provider_metadata_without_app_state_files() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let data = temp.path().join("data");
        fs::create_dir_all(home.join("sessions/2026")).unwrap();
        let rollout = home.join("sessions/2026/rollout.jsonl");
        let before = format!(
            "{}\n{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"openai","cwd":"C:/keep"}}),
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
    fn repair_accepts_legacy_metadata_without_provider_on_context_records() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/legacy.jsonl");
        let original = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"legacy\",\"model_provider\":\"custom\",\"history_mode\":\"legacy\"}}\n",
            "{\"type\":\"turn_context\",\"payload\":{\"turn_id\":\"turn-1\",\"model\":\"custom-model\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_settings_applied\",\"thread_settings\":{\"model\":\"custom-model\"}}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"id\":\"msg-keep\",\"content\":[]}}\n"
        );
        fs::write(&rollout, original).unwrap();

        let result = repair(&home, "openai").unwrap();

        assert_eq!(result.files_modified, 1);
        assert_eq!(result.session_meta_updated, 1);
        let repaired = fs::read(&rollout).unwrap();
        assert_eq!(repaired.len(), original.len());
        assert!(String::from_utf8_lossy(&repaired).contains("\"model_provider\":\"openai\""));
        assert!(String::from_utf8_lossy(&repaired).contains("\"model\":\"custom-model\""));
        assert!(String::from_utf8_lossy(&repaired).contains("msg-keep"));
    }

    #[test]
    fn scan_reports_per_provider_counts() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::write(
            home.join("sessions/openai.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({"type":"session_meta","payload":{"id":"openai","model_provider":"openai"}})
            ),
        )
        .unwrap();
        fs::write(
            home.join("sessions/custom.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({"type":"session_meta","payload":{"id":"custom","model_provider":"custom"}})
            ),
        )
        .unwrap();
        let db = home.join("state_5.sqlite");
        let connection = Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT); INSERT INTO threads VALUES('one','openai'); INSERT INTO threads VALUES('two','openai'); INSERT INTO threads VALUES('three','custom');",
            )
            .unwrap();
        drop(connection);

        let result = scan(&home);

        assert_eq!(result.session_meta_count, 2);
        assert_eq!(result.rollout_files, 2);
        assert_eq!(result.databases[0].thread_count, 3);
        let by_id: BTreeMap<_, _> = result
            .targets
            .iter()
            .map(|target| (target.id.as_str(), target))
            .collect();
        assert_eq!(by_id["openai"].count, 3);
        assert!(by_id["openai"].sources.contains(&"sqlite".to_string()));
        assert!(by_id["openai"].sources.contains(&"rollout".to_string()));
        assert_eq!(by_id["custom"].count, 2);
        assert!(by_id["custom"].sources.contains(&"sqlite".to_string()));
        assert!(by_id["custom"].sources.contains(&"rollout".to_string()));
    }

    #[test]
    fn repair_preserves_already_matching_metadata_byte_for_byte() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        let unchanged = r#"{ "type": "session_meta", "payload": { "id": "two", "model_provider": "custom", "future": true } }"#;
        let original = format!(
            "{}\n{unchanged}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"openai"}})
        );
        fs::write(&rollout, &original).unwrap();
        let outcome = repair(&home, "custom").unwrap();
        assert_eq!(outcome.session_meta_updated, 1);
        let repaired = fs::read_to_string(rollout).unwrap();
        assert!(repaired.ends_with(&format!("{unchanged}\n")));
    }

    #[test]
    fn repair_reports_guard_conflict_as_skipped_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        let original = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"guard-conflict","model_provider":"openai"}})
        );
        fs::write(&rollout, &original).unwrap();

        let error = repair_with_guard(&home, "custom", || Ok(false)).unwrap_err();

        assert!(error.to_string().contains("切换已终止"));
        assert_eq!(fs::read_to_string(rollout).unwrap(), original);
    }

    #[test]
    fn sqlite_update_is_narrow_and_transactional() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("state.sqlite");
        let connection = Connection::open(&db).unwrap();
        connection.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT, title TEXT); INSERT INTO threads VALUES('one','other-provider','keep'); INSERT INTO threads VALUES('two','custom','same'); INSERT INTO threads VALUES('three',NULL,'missing');").unwrap();
        drop(connection);
        assert_eq!(
            repair_database(
                &db,
                "custom",
                &session_scope(std::slice::from_ref(&db), &[]).unwrap(),
                &mut || Ok(true),
            )
            .unwrap(),
            Some(2)
        );
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
                serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"openai"}})
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
    fn provider_change_preserves_response_items_and_ids_byte_for_byte() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        let records = [
            serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"custom"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"reasoning","id":"msg_wrong-for-reasoning","summary":[]}}),
            serde_json::json!({"type":"response_item","payload":{"type":"message","id":"msg_other-provider","role":"assistant","content":[{"type":"output_text","text":"keep text"}]}}),
            serde_json::json!({"type":"response_item","payload":{"type":"function_call","id":"msg_wrong-for-call","call_id":"call_keep","name":"exec","arguments":"{}"}}),
            serde_json::json!({"type":"response_item","payload":{"type":"function_call_output","id":"fco_local","call_id":"call_keep","output":"keep output"}}),
        ];
        fs::write(
            &rollout,
            records
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .unwrap();
        let before = fs::read(&rollout).unwrap();

        let result = repair(&home, "openai").unwrap();

        assert_eq!(result.files_modified, 1);
        let repaired = fs::read(&rollout).unwrap();
        assert_eq!(repaired.len(), before.len());
        let repaired = String::from_utf8(repaired).unwrap();
        assert!(repaired.contains("msg_wrong-for-reasoning"));
        assert!(repaired.contains("msg_other-provider"));
        assert!(repaired.contains("msg_wrong-for-call"));
        assert!(repaired.contains("keep text"));
        assert!(repaired.contains("\"call_id\":\"call_keep\""));
        assert!(repaired.contains("\"id\":\"fco_local\""));
        assert!(repaired.contains("keep output"));
    }

    #[test]
    fn third_party_to_third_party_switch_portabilizes_same_provider_family() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        fs::write(
            &rollout,
            format!(
                "{}\n{}\n",
                serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"custom"}}),
                serde_json::json!({"type":"response_item","payload":{"type":"function_call","id":"msg_provider-a","call_id":"call_keep","name":"exec","arguments":"{}"}})
            ),
        )
        .unwrap();
        let before = fs::read(&rollout).unwrap();
        let normal = repair(&home, "custom").unwrap();
        assert_eq!(normal.files_modified, 0);
        assert_eq!(fs::read(&rollout).unwrap(), before);

        let switched = repair_after_connection_switch(&home, "custom").unwrap();
        assert_eq!(switched.files_modified, 0);
        let repaired = fs::read_to_string(&rollout).unwrap();
        assert!(repaired.contains("msg_provider-a"));
        assert!(repaired.contains("\"call_id\":\"call_keep\""));
    }

    #[test]
    fn activation_repair_updates_provider_in_place_and_preserves_rollout_layout() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        let original =
            br#"{ "type": "session_meta", "payload": { "id": "one", "model_provider": "openai" } }
{"type":"turn_context","payload":{"model_provider":"openai"}}
{"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"model_provider_id":"openai"}}}
{"type":"event_msg","payload":{"type":"user_message","message":"keep exact bytes except provider"}}
"#;
        fs::write(&rollout, original).unwrap();
        let database = home.join("state_5.sqlite");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT, title TEXT);
                 INSERT INTO threads VALUES('one','openai','keep title');",
            )
            .unwrap();
        drop(connection);

        let result =
            repair_after_connection_switch_preserving_history_with_guard(&home, "custom", || {
                Ok(true)
            })
            .unwrap();

        assert_eq!(result.files_scanned, 1);
        assert_eq!(result.files_modified, 1);
        assert_eq!(result.session_meta_updated, 3);
        assert_eq!(result.rows_updated, 1);
        let repaired = fs::read(&rollout).unwrap();
        assert_eq!(repaired.len(), original.len());
        assert!(String::from_utf8_lossy(&repaired).contains("\"model_provider\": \"custom\""));
        assert!(String::from_utf8_lossy(&repaired).contains("\"model_provider\":\"custom\""));
        assert!(String::from_utf8_lossy(&repaired).contains("\"model_provider_id\":\"custom\""));
        assert!(String::from_utf8_lossy(&repaired).contains("keep exact bytes except provider"));
        let connection = Connection::open(database).unwrap();
        let provider: String = connection
            .query_row(
                "SELECT model_provider FROM threads WHERE id='one'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(provider, "custom");
        let title: String = connection
            .query_row("SELECT title FROM threads WHERE id='one'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(title, "keep title");
    }

    #[test]
    fn repair_rejects_unmanaged_provider_targets() {
        let temp = tempfile::tempdir().unwrap();
        let error = repair(temp.path(), "third-party").unwrap_err();
        assert!(error.to_string().contains("只能在 OpenAI"));
    }

    #[test]
    fn warning_lists_are_bounded_and_report_omissions() {
        let mut warnings = Vec::new();
        let mut omitted = 0;
        for index in 0..150 {
            push_warning(&mut warnings, &mut omitted, format!("warning-{index}"));
        }
        finish_warnings(&mut warnings, omitted);

        assert_eq!(warnings.len(), MAX_REPAIR_WARNINGS);
        assert_eq!(warnings.last().unwrap(), "另有 51 项警告未显示。");
    }

    #[test]
    fn incremental_switch_caches_unchanged_rollouts_from_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let manifest_path = temp.path().join("manifest.json");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        fs::write(
            &rollout,
            format!(
                "{}\n{}\n",
                serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"custom"}}),
                serde_json::json!({"type":"turn_context","payload":{"model_provider":"custom"}})
            ),
        )
        .unwrap();

        let first = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "custom",
            Some(&manifest_path),
            || Ok(true),
        )
        .unwrap();
        assert_eq!(first.files_opened, 1);
        assert_eq!(first.files_skipped, 1);
        assert_eq!(first.files_cached, 0);
        assert!(first.repair_complete);
        assert!(manifest_path.exists());

        let second = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "custom",
            Some(&manifest_path),
            || Ok(true),
        )
        .unwrap();
        assert_eq!(second.files_cached, 1);
        assert_eq!(second.files_opened, 0);
        assert_eq!(second.files_modified, 0);
        assert!(second.repair_complete);
    }

    #[test]
    fn incremental_switch_flips_provider_and_rebuilds_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let manifest_path = temp.path().join("manifest.json");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        fs::write(
            &rollout,
            format!(
                "{}\n",
                serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"openai"}})
            ),
        )
        .unwrap();

        let to_custom = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "custom",
            Some(&manifest_path),
            || Ok(true),
        )
        .unwrap();
        assert_eq!(to_custom.files_modified, 1);
        assert_eq!(to_custom.files_cached, 0);

        let to_custom_again = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "custom",
            Some(&manifest_path),
            || Ok(true),
        )
        .unwrap();
        assert_eq!(to_custom_again.files_cached, 1);
        assert_eq!(to_custom_again.files_opened, 0);

        let to_openai = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "openai",
            Some(&manifest_path),
            || Ok(true),
        )
        .unwrap();
        assert_eq!(to_openai.files_modified, 1);
        assert_eq!(to_openai.files_cached, 0);
        assert!(
            fs::read_to_string(&rollout)
                .unwrap()
                .contains("\"model_provider\":\"openai\"")
        );
    }

    #[test]
    fn unknown_database_schema_aborts_switch_before_write() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let manifest_path = temp.path().join("manifest.json");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        let original = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"openai"}})
        );
        fs::write(&rollout, &original).unwrap();
        let db = home.join("state_5.sqlite");
        let connection = Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE unrelated(id TEXT PRIMARY KEY); INSERT INTO unrelated VALUES('x');",
            )
            .unwrap();
        drop(connection);
        let db_before = fs::read(&db).unwrap();

        let error = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "custom",
            Some(&manifest_path),
            || Ok(true),
        )
        .unwrap_err();

        assert!(error.to_string().contains("未知的 Codex 会话数据库结构"));
        assert_eq!(fs::read_to_string(&rollout).unwrap(), original);
        assert_eq!(fs::read(&db).unwrap(), db_before);
        assert!(!manifest_path.exists());
    }

    #[test]
    fn activation_repair_ignores_auxiliary_sqlite_databases_without_session_tables() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let manifest_path = temp.path().join("manifest.json");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::create_dir_all(home.join("sqlite")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        fs::write(
            &rollout,
            format!(
                "{}\n",
                serde_json::json!({"type":"session_meta","payload":{"id":"active","model_provider":"openai"}})
            ),
        )
        .unwrap();
        let session_db = home.join("sqlite/codex-dev.db");
        let db = Connection::open(&session_db).unwrap();
        db.execute_batch(
            "CREATE TABLE local_thread_catalog(thread_id TEXT PRIMARY KEY, host_id TEXT, source_kind TEXT, missing_candidate INTEGER, model_provider TEXT);
             CREATE TABLE local_thread_catalog_hosts(id TEXT PRIMARY KEY, host_kind TEXT);
             INSERT INTO local_thread_catalog_hosts VALUES('host','local');
             INSERT INTO local_thread_catalog VALUES('active','host','local',0,'openai');",
        )
        .unwrap();
        drop(db);
        let auxiliary = home.join("sqlite/codex-history-snapshots-dev.db");
        let db = Connection::open(&auxiliary).unwrap();
        db.execute_batch("CREATE TABLE snapshots(id TEXT PRIMARY KEY, payload TEXT); INSERT INTO snapshots VALUES('keep','unchanged');").unwrap();
        drop(db);
        let auxiliary_before = fs::read(&auxiliary).unwrap();

        let result = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "custom",
            Some(&manifest_path),
            || Ok(true),
        )
        .unwrap();

        assert_eq!(result.databases_scanned, 1);
        assert_eq!(result.databases_updated, 1);
        assert_eq!(result.rows_updated, 1);
        assert_eq!(fs::read(&auxiliary).unwrap(), auxiliary_before);
        let db = Connection::open(session_db).unwrap();
        let provider: String = db
            .query_row(
                "SELECT model_provider FROM local_thread_catalog WHERE thread_id='active'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(provider, "custom");
    }

    #[test]
    fn failed_switch_rolls_back_rollout_writes() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let manifest_path = temp.path().join("manifest.json");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        let original = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"openai"}})
        );
        fs::write(&rollout, &original).unwrap();
        let db = home.join("state_5.sqlite");
        let connection = Connection::open(&db).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT); INSERT INTO threads VALUES('one','openai');",
            )
            .unwrap();
        drop(connection);
        let db_before = fs::read(&db).unwrap();

        let mut guard_calls = 0;
        let error = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "custom",
            Some(&manifest_path),
            || {
                guard_calls += 1;
                Ok(guard_calls < 3)
            },
        )
        .unwrap_err();

        // 调用顺序：1 预检门禁；2 rollout 写入；3 数据库门禁（返回 false 触发回滚）
        assert_eq!(guard_calls, 3);
        assert!(error.to_string().contains("已终止并回滚"));
        assert_eq!(fs::read_to_string(&rollout).unwrap(), original);
        assert_eq!(fs::read(&db).unwrap(), db_before);
        assert!(!manifest_path.exists());
    }

    #[test]
    fn database_phase_failure_restores_rollout_and_database() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let manifest_path = temp.path().join("manifest.json");
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::create_dir_all(home.join("sqlite")).unwrap();
        let rollout = home.join("sessions/rollout.jsonl");
        let original = format!(
            "{}\n",
            serde_json::json!({"type":"session_meta","payload":{"id":"one","model_provider":"openai"}})
        );
        fs::write(&rollout, &original).unwrap();
        let db_a = home.join("sqlite/a.sqlite");
        let db_b = home.join("sqlite/b.sqlite");
        for db_path in [&db_a, &db_b] {
            let connection = Connection::open(db_path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT); INSERT INTO threads VALUES('one','openai');",
                )
                .unwrap();
            drop(connection);
        }
        let db_a_before = fs::read(&db_a).unwrap();
        let db_b_before = fs::read(&db_b).unwrap();

        let mut guard_calls = 0;
        let error = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "custom",
            Some(&manifest_path),
            || {
                guard_calls += 1;
                Ok(guard_calls < 4)
            },
        )
        .unwrap_err();

        // 调用顺序：1 预检门禁；2 rollout；3 数据库 a；4 数据库 b（返回 false 触发回滚）
        assert_eq!(guard_calls, 4);
        assert!(error.to_string().contains("已终止并回滚"));
        assert_eq!(fs::read_to_string(&rollout).unwrap(), original);
        assert_eq!(fs::read(&db_a).unwrap(), db_a_before);
        assert_eq!(fs::read(&db_b).unwrap(), db_b_before);
        assert!(!manifest_path.exists());
    }

    #[test]
    fn malformed_history_database_is_never_touched() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let rollout = home.join("sessions/one.jsonl");
        let original = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"one\",\"model_provider\":\"openai\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"reasoning\",\"id\":\"rs-old\"}}\n"
        );
        fs::write(&rollout, original).unwrap();
        let state = home.join("state_5.sqlite");
        let db = Connection::open(&state).unwrap();
        db.execute_batch(
            "CREATE TABLE threads(id TEXT PRIMARY KEY, model_provider TEXT, model TEXT);
             INSERT INTO threads VALUES('one','openai','old');",
        )
        .unwrap();
        drop(db);
        let state_before = fs::read(&state).unwrap();
        let history = Connection::open(home.join(THREAD_HISTORY_FILE)).unwrap();
        history
            .execute_batch(
                "CREATE TABLE thread_history_projection_state(thread_id TEXT);
                 CREATE TABLE thread_items(thread_id TEXT, turn_id TEXT, item_id TEXT, rollout_ordinal INTEGER, item_json TEXT);
                 CREATE TABLE thread_turns(thread_id TEXT, turn_id TEXT, rollout_ordinal INTEGER, rollout_byte_offset INTEGER, rollout_end_ordinal INTEGER, rollout_end_byte_offset INTEGER);",
            )
            .unwrap();
        drop(history);
        let history_before = fs::read(home.join(THREAD_HISTORY_FILE)).unwrap();

        let result = repair_after_connection_switch_preserving_history_with_guard_at(
            &home,
            "custom",
            Some(&temp.path().join("manifest.json")),
            || Ok(true),
        )
        .unwrap();
        assert!(result.repair_complete);
        assert_eq!(fs::read_to_string(&rollout).unwrap().len(), original.len());
        assert!(
            fs::read_to_string(&rollout)
                .unwrap()
                .contains("\"model_provider\":\"custom\"")
        );
        assert_ne!(fs::read(&state).unwrap(), state_before);
        let db = Connection::open(state).unwrap();
        let route: (String, Option<String>) = db
            .query_row(
                "SELECT model_provider, model FROM threads WHERE id='one'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(route, ("custom".into(), None));
        assert_eq!(
            fs::read(home.join(THREAD_HISTORY_FILE)).unwrap(),
            history_before
        );
    }

    #[test]
    fn legacy_recovery_marker_preserves_every_non_session_record_byte_for_byte() {
        let response = concat!(
            "{\"ordinal\":1,\"type\":\"response_item\",\"payload\":{\"type\":\"reasoning\",\"id\":\"rs-keep\"}}\r\n",
            "{\"ordinal\":2,\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"keep context\"}]}}\n"
        );
        let original = format!(
            "{}\n{response}",
            serde_json::json!({
                "ordinal": 0,
                "type": "session_meta",
                "payload": {
                    "id": "one",
                    "model_provider": "custom",
                    "history_mode": "paginated"
                }
            })
        );

        let recovered = mark_rollout_history_legacy(original.as_bytes(), "one").unwrap();
        let recovered = String::from_utf8(recovered).unwrap();

        assert!(recovered.ends_with(response));
        let session: Value = serde_json::from_str(recovered.lines().next().unwrap()).unwrap();
        assert_eq!(
            session
                .pointer("/payload/history_mode")
                .and_then(Value::as_str),
            Some("legacy")
        );
        assert_eq!(session.get("ordinal").and_then(Value::as_i64), Some(0));
        assert!(recovered.contains("rs-keep"));
        assert!(recovered.contains("keep context"));
    }

    #[test]
    fn rollout_recovery_info_recognizes_known_subagent_parent_fields_and_fails_closed() {
        let direct = rollout_recovery_info(
            br#"{"type":"session_meta","payload":{"id":"child","parent_thread_id":"parent","history_mode":"paginated"}}"#,
        )
        .unwrap();
        assert!(direct.is_subagent);

        let nested = rollout_recovery_info(
            br#"{"type":"session_meta","payload":{"id":"child","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent"}}},"history_mode":"paginated"}}"#,
        )
        .unwrap();
        assert!(nested.is_subagent);

        let unknown = rollout_recovery_info(
            br#"{"type":"session_meta","payload":{"id":"root","source":{"worker":{"parent":"parent"}},"history_mode":"paginated"}}"#,
        )
        .unwrap();
        assert!(!unknown.is_subagent);
    }

    #[test]
    fn state_only_paginated_projection_is_valid_only_for_subagent() {
        let temp = tempfile::tempdir().unwrap();
        let history_path = temp.path().join(THREAD_HISTORY_FILE);
        let history = Connection::open(&history_path).unwrap();
        history
            .execute_batch(
                "CREATE TABLE thread_history_projection_state(
                     thread_id TEXT PRIMARY KEY,
                     next_rollout_byte_offset INTEGER,
                     next_rollout_ordinal INTEGER
                 );
                 CREATE TABLE thread_items(thread_id TEXT, rollout_ordinal INTEGER, item_json TEXT);
                 CREATE TABLE thread_turns(
                     thread_id TEXT,
                     rollout_ordinal INTEGER,
                     rollout_byte_offset INTEGER,
                     rollout_end_ordinal INTEGER,
                     rollout_end_byte_offset INTEGER
                 );
                 INSERT INTO thread_history_projection_state VALUES('child',100,7);",
            )
            .unwrap();

        assert!(paginated_projection_is_valid(Some(&history), "child", 100, true).unwrap());
        assert!(!paginated_projection_is_valid(Some(&history), "child", 100, false).unwrap());
    }

    #[test]
    fn verify_accepts_paginated_projection_rebuilt_from_empty_history() {
        let temp = tempfile::tempdir().unwrap();
        let state_path = temp.path().join("state_5.sqlite");
        let history_path = temp.path().join(THREAD_HISTORY_FILE);
        let rollout_path = temp.path().join("rollout.jsonl");
        let rollout = concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-one\",\"history_mode\":\"legacy\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-one\"}}\n"
        );
        fs::write(&rollout_path, rollout).unwrap();

        let state = Connection::open(&state_path).unwrap();
        state
            .execute_batch(
                "CREATE TABLE threads(id TEXT PRIMARY KEY, history_mode TEXT);
                 INSERT INTO threads VALUES('thread-one','paginated');",
            )
            .unwrap();
        drop(state);

        let history = Connection::open(&history_path).unwrap();
        let rollout_length = fs::metadata(&rollout_path).unwrap().len() as i64;
        history
            .execute_batch(
                "CREATE TABLE thread_history_projection_state(
                     thread_id TEXT PRIMARY KEY,
                     next_rollout_byte_offset INTEGER,
                     next_rollout_ordinal INTEGER
                 );
                 CREATE TABLE thread_items(
                     thread_id TEXT,
                     rollout_ordinal INTEGER,
                     item_json TEXT
                 );
                 CREATE TABLE thread_turns(
                     thread_id TEXT,
                     rollout_ordinal INTEGER,
                     rollout_byte_offset INTEGER,
                     rollout_end_ordinal INTEGER,
                     rollout_end_byte_offset INTEGER
                 );",
            )
            .unwrap();
        drop(history);

        let plan = PaginatedHistoryRecoveryPlan {
            thread_id: "thread-one".into(),
            path: rollout_path.clone(),
            original_sha256: file_sha256(&rollout_path).unwrap(),
            is_subagent: false,
        };
        let report = MigrationReport {
            outcomes: vec![MigrationOutcome {
                thread_id: "thread-one".into(),
                status: "migrated".into(),
                message: None,
            }],
        };
        assert!(
            verify_paginated_history_recovery(
                &state_path,
                &history_path,
                std::slice::from_ref(&plan),
                &report,
            )
            .is_err()
        );

        let history = Connection::open(&history_path).unwrap();
        history
            .execute(
                "INSERT INTO thread_history_projection_state VALUES('thread-one',?1,2)",
                [rollout_length],
            )
            .unwrap();
        history
            .execute("INSERT INTO thread_items VALUES('thread-one',1,'{}')", [])
            .unwrap();
        history
            .execute(
                "INSERT INTO thread_turns VALUES('thread-one',1,0,2,?1)",
                [rollout_length],
            )
            .unwrap();
        drop(history);

        verify_paginated_history_recovery(
            &state_path,
            &history_path,
            std::slice::from_ref(&plan),
            &report,
        )
        .unwrap();
    }

    #[test]
    fn verify_accepts_state_only_subagent_projection_after_migration_for_both_modes() {
        for mode in ["legacy", "paginated"] {
            let temp = tempfile::tempdir().unwrap();
            let state_path = temp.path().join("state_5.sqlite");
            let history_path = temp.path().join(THREAD_HISTORY_FILE);
            let rollout_path = temp.path().join("rollout.jsonl");
            fs::write(
                &rollout_path,
                format!(
                    "{}\n",
                    serde_json::json!({
                        "type": "session_meta",
                        "payload": {
                            "id": "child",
                            "parent_thread_id": "parent",
                            "history_mode": mode
                        }
                    })
                ),
            )
            .unwrap();
            let rollout_length = fs::metadata(&rollout_path).unwrap().len() as i64;

            let state = Connection::open(&state_path).unwrap();
            state
                .execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, history_mode TEXT);")
                .unwrap();
            state
                .execute("INSERT INTO threads VALUES('child',?1)", [mode])
                .unwrap();
            drop(state);

            let history = Connection::open(&history_path).unwrap();
            history
                .execute_batch(
                    "CREATE TABLE thread_history_projection_state(
                         thread_id TEXT PRIMARY KEY,
                         next_rollout_byte_offset INTEGER,
                         next_rollout_ordinal INTEGER
                     );
                     CREATE TABLE thread_items(thread_id TEXT, rollout_ordinal INTEGER, item_json TEXT);
                     CREATE TABLE thread_turns(
                         thread_id TEXT,
                         rollout_ordinal INTEGER,
                         rollout_byte_offset INTEGER,
                         rollout_end_ordinal INTEGER,
                         rollout_end_byte_offset INTEGER
                     );",
                )
                .unwrap();
            history
                .execute(
                    "INSERT INTO thread_history_projection_state VALUES('child',?1,827)",
                    [rollout_length],
                )
                .unwrap();
            drop(history);

            let plan = PaginatedHistoryRecoveryPlan {
                thread_id: "child".into(),
                path: rollout_path,
                original_sha256: "unused".into(),
                is_subagent: true,
            };
            let report = MigrationReport {
                outcomes: vec![MigrationOutcome {
                    thread_id: "child".into(),
                    status: "migrated".into(),
                    message: None,
                }],
            };
            verify_paginated_history_recovery(
                &state_path,
                &history_path,
                std::slice::from_ref(&plan),
                &report,
            )
            .unwrap();
        }
    }

    #[test]
    fn verify_accepts_legacy_state_when_migration_report_and_projection_are_valid() {
        let temp = tempfile::tempdir().unwrap();
        let state_path = temp.path().join("state_5.sqlite");
        let history_path = temp.path().join(THREAD_HISTORY_FILE);
        let rollout_path = temp.path().join("rollout.jsonl");
        fs::write(
            &rollout_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"thread-one\",\"history_mode\":\"legacy\"}}\n",
        )
        .unwrap();

        let state = Connection::open(&state_path).unwrap();
        state
            .execute_batch(
                "CREATE TABLE threads(id TEXT PRIMARY KEY, history_mode TEXT);
                 INSERT INTO threads VALUES('thread-one','legacy');",
            )
            .unwrap();
        drop(state);

        let history = Connection::open(&history_path).unwrap();
        let rollout_length = fs::metadata(&rollout_path).unwrap().len() as i64;
        history
            .execute_batch(
                "CREATE TABLE thread_history_projection_state(
                     thread_id TEXT PRIMARY KEY,
                     next_rollout_byte_offset INTEGER,
                     next_rollout_ordinal INTEGER
                 );
                 CREATE TABLE thread_items(thread_id TEXT, rollout_ordinal INTEGER, item_json TEXT);
                 CREATE TABLE thread_turns(
                     thread_id TEXT,
                     rollout_ordinal INTEGER,
                     rollout_byte_offset INTEGER,
                     rollout_end_ordinal INTEGER,
                     rollout_end_byte_offset INTEGER
                 );
                 INSERT INTO thread_items VALUES('thread-one',1,'{}');",
            )
            .unwrap();
        history
            .execute(
                "INSERT INTO thread_history_projection_state VALUES('thread-one',?1,2)",
                [rollout_length],
            )
            .unwrap();
        history
            .execute(
                "INSERT INTO thread_turns VALUES('thread-one',1,0,2,?1)",
                [rollout_length],
            )
            .unwrap();
        drop(history);

        let plan = PaginatedHistoryRecoveryPlan {
            thread_id: "thread-one".into(),
            path: rollout_path,
            original_sha256: "unused".into(),
            is_subagent: false,
        };
        let report = MigrationReport {
            outcomes: vec![MigrationOutcome {
                thread_id: "thread-one".into(),
                status: "migrated".into(),
                message: Some("projection rebuilt".into()),
            }],
        };

        verify_paginated_history_recovery(
            &state_path,
            &history_path,
            std::slice::from_ref(&plan),
            &report,
        )
        .unwrap();
    }

    #[test]
    fn sqlite_backup_restores_sidecars_and_removes_new_ones() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("state_5.sqlite");
        let wal = temp.path().join("state_5.sqlite-wal");
        let shm = temp.path().join("state_5.sqlite-shm");
        fs::write(&database, b"database-before").unwrap();
        fs::write(&wal, b"wal-before").unwrap();
        fs::write(&shm, b"shm-before").unwrap();

        let backup = RepairBackup::create().unwrap();
        let mut backup = backup;
        backup.add_sqlite_database(&database).unwrap();
        fs::write(&database, b"database-after").unwrap();
        fs::write(&wal, b"wal-after").unwrap();
        fs::remove_file(&shm).unwrap();
        backup.restore().unwrap();
        assert_eq!(fs::read(&database).unwrap(), b"database-before");
        assert_eq!(fs::read(&wal).unwrap(), b"wal-before");
        assert_eq!(fs::read(&shm).unwrap(), b"shm-before");
        backup.cleanup().unwrap();

        let database = temp.path().join("history.sqlite");
        let wal = temp.path().join("history.sqlite-wal");
        let shm = temp.path().join("history.sqlite-shm");
        fs::write(&database, b"database-before").unwrap();
        let mut backup = RepairBackup::create().unwrap();
        backup.add_sqlite_database(&database).unwrap();
        fs::write(&database, b"database-after").unwrap();
        fs::write(&wal, b"wal-created").unwrap();
        fs::write(&shm, b"shm-created").unwrap();
        backup.restore().unwrap();
        assert_eq!(fs::read(&database).unwrap(), b"database-before");
        assert!(!wal.exists());
        assert!(!shm.exists());
        backup.cleanup().unwrap();
    }

    #[test]
    fn preflight_skips_healthy_state_only_subagent_projection() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let sessions = home.join("sessions");
        fs::create_dir_all(&sessions).unwrap();
        let parent = sessions.join("parent.jsonl");
        let child = sessions.join("child.jsonl");
        fs::write(
            &parent,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"parent\",\"history_mode\":\"paginated\"}}\n",
        )
        .unwrap();
        fs::write(
            &child,
            concat!(
                "{\"type\":\"session_meta\",\"payload\":{\"id\":\"child\",\"parent_thread_id\":\"parent\",\"history_mode\":\"paginated\",\"source\":{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"parent\"}}}}}\n",
                "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\",\"turn_id\":\"turn-child\"}}\n"
            ),
        )
        .unwrap();
        let state = Connection::open(home.join("state_5.sqlite")).unwrap();
        state
            .execute_batch(
                "CREATE TABLE threads(
                     id TEXT PRIMARY KEY,
                     history_mode TEXT,
                     source TEXT,
                     archived INTEGER
                 );
                 INSERT INTO threads VALUES('parent','paginated',NULL,0);
                 INSERT INTO threads VALUES(
                     'child',
                     'paginated',
                     '{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"parent\"}}}',
                     0
                 );",
            )
            .unwrap();
        drop(state);
        let history = Connection::open(home.join(THREAD_HISTORY_FILE)).unwrap();
        history
            .execute_batch(
                "CREATE TABLE thread_history_projection_state(
                     thread_id TEXT PRIMARY KEY,
                     next_rollout_byte_offset INTEGER,
                     next_rollout_ordinal INTEGER
                 );
                 CREATE TABLE thread_items(thread_id TEXT, rollout_ordinal INTEGER, item_json TEXT);
                 CREATE TABLE thread_turns(
                     thread_id TEXT,
                     rollout_ordinal INTEGER,
                     rollout_byte_offset INTEGER,
                     rollout_end_ordinal INTEGER,
                     rollout_end_byte_offset INTEGER
                 );",
            )
            .unwrap();
        history
            .execute(
                "INSERT INTO thread_history_projection_state VALUES('child',?1,827)",
                [i64::try_from(fs::metadata(&child).unwrap().len()).unwrap()],
            )
            .unwrap();
        drop(history);

        let rollouts = rollout_files(&home);
        let scope = session_scope(
            std::slice::from_ref(&home.join("state_5.sqlite")),
            &rollouts,
        )
        .unwrap();
        let plans = preflight_paginated_history_recovery(&home, &scope, &rollouts).unwrap();

        assert!(plans.is_empty());
    }

    #[test]
    fn preflight_recovers_only_missing_or_out_of_range_paginated_projection() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        let missing = home.join("sessions/missing.jsonl");
        let healthy = home.join("sessions/healthy.jsonl");
        for (path, id) in [(&missing, "missing"), (&healthy, "healthy")] {
            fs::write(
                path,
                format!(
                    "{}\n{}\n",
                    serde_json::json!({
                        "ordinal": 0,
                        "type": "session_meta",
                        "payload": {
                            "id": id,
                            "model_provider": "custom",
                            "history_mode": if id == "healthy" {
                                serde_json::Value::String("paginated".into())
                            } else {
                                serde_json::Value::Null
                            }
                        }
                    }),
                    serde_json::json!({
                        "ordinal": 1,
                        "type": "event_msg",
                        "payload": {"type": "task_started", "turn_id": format!("turn-{id}")}
                    })
                ),
            )
            .unwrap();
        }
        let state = Connection::open(home.join("state_5.sqlite")).unwrap();
        state
            .execute_batch(
                "CREATE TABLE threads(
                     id TEXT PRIMARY KEY,
                     model_provider TEXT,
                     history_mode TEXT,
                     archived INTEGER,
                     source TEXT
                 );
                 INSERT INTO threads VALUES('missing','custom','paginated',0,NULL);
                 INSERT INTO threads VALUES('healthy','custom','paginated',0,NULL);",
            )
            .unwrap();
        drop(state);
        let history = Connection::open(home.join(THREAD_HISTORY_FILE)).unwrap();
        history
            .execute_batch(
                "CREATE TABLE thread_history_projection_state(
                     thread_id TEXT PRIMARY KEY,
                     next_rollout_byte_offset INTEGER,
                     next_rollout_ordinal INTEGER
                 );
                 CREATE TABLE thread_items(
                     thread_id TEXT,
                     rollout_ordinal INTEGER,
                     item_json TEXT
                 );
                 CREATE TABLE thread_turns(
                     thread_id TEXT,
                     rollout_ordinal INTEGER,
                     rollout_byte_offset INTEGER,
                     rollout_end_ordinal INTEGER,
                     rollout_end_byte_offset INTEGER
                 );",
            )
            .unwrap();
        let healthy_length = fs::metadata(&healthy).unwrap().len();
        history
            .execute(
                "INSERT INTO thread_history_projection_state VALUES('healthy',?1,2)",
                [i64::try_from(healthy_length).unwrap()],
            )
            .unwrap();
        history
            .execute("INSERT INTO thread_items VALUES('healthy',1,'{}')", [])
            .unwrap();
        history
            .execute(
                "INSERT INTO thread_turns VALUES('healthy',1,0,2,?1)",
                [i64::try_from(healthy_length).unwrap()],
            )
            .unwrap();
        drop(history);

        let rollouts = rollout_files(&home);
        let scope = session_scope(
            std::slice::from_ref(&home.join("state_5.sqlite")),
            &rollouts,
        )
        .unwrap();
        let plans = preflight_paginated_history_recovery(&home, &scope, &rollouts).unwrap();

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].thread_id, "missing");
        assert_eq!(plans[0].path, missing);
    }
}
