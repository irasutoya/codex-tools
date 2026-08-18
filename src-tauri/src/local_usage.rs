use crate::{
    models::{
        ActivationRecord, AppError, BillingMode, CostStatus, PricingMatchKind, PricingRule,
        PricingScope, PricingScopeKind, RepriceResult, SavePricingRule, TokenBreakdown,
        UsageGroupBy, UsageOverview, UsageQuery, UsageRange, UsageRefreshResult, UsageRow,
        UsageSourceKind, UsageTotals, UsageTrend, UsageTrendPoint, UsageWarning,
    },
    official_pricing::OfficialPricingCatalog,
    pricing::{
        PricingContext, PricingOutcome, PricingRuleRecord, official_pricing_rule_name,
        parse_usd_microusd, price_for_source,
    },
    provider_sync,
    storage::{secure_directory, secure_file},
    usage_log::{
        LineResult, ParsedUsageEvent, ParserState, UsageBoundaryState, is_inter_agent_trigger_line,
        parse_line,
    },
};
use chrono::{Local, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, btree_map::Entry},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 7;
const MAX_ROLLOUT_BYTES: u64 = 256 * 1024 * 1024;
const MAX_METADATA_SCAN_BYTES: usize = 4 * 1024 * 1024;
const MAX_RECORD_LINE_BYTES: usize = 8 * 1024 * 1024;
const COLLECTION_EPOCH_METADATA_KEY: &str = "usage_collection_started_at_ms";
const COLLECTION_VERSION_METADATA_KEY: &str = "usage_collection_started_version";
const COLLECTION_MODE_METADATA_KEY: &str = "usage_collection_mode";
const PARSER_VERSION_METADATA_KEY: &str = "usage_parser_version";
const COLLECTION_MODE: &str = "after_update";
const COLLECTION_VERSION: &str = env!("CARGO_PKG_VERSION");
const PARSER_VERSION: &str = "5";

struct BoundedLine {
    bytes: Vec<u8>,
    bytes_read: usize,
    complete: bool,
    truncated: bool,
}

fn read_bounded_line<R: BufRead>(
    reader: &mut R,
    max_content_bytes: usize,
    mut consume: impl FnMut(&[u8]),
) -> std::io::Result<BoundedLine> {
    // Keep enough room for a valid maximum-length line plus CRLF.
    let capacity = max_content_bytes.saturating_add(2);
    let mut bytes = Vec::new();
    let mut bytes_read = 0usize;
    let mut complete = false;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break;
        }
        let consumed = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        let chunk = &available[..consumed];
        consume(chunk);
        bytes_read = bytes_read.saturating_add(consumed);
        let retained = capacity.saturating_sub(bytes.len()).min(consumed);
        bytes.extend_from_slice(&chunk[..retained]);
        complete = chunk.ends_with(b"\n");
        reader.consume(consumed);
        if complete {
            break;
        }
    }

    Ok(BoundedLine {
        bytes,
        bytes_read,
        complete,
        truncated: bytes_read > capacity,
    })
}

#[derive(Clone)]
pub(crate) struct UsageLedger {
    database_path: Arc<PathBuf>,
    refresh_lock: Arc<Mutex<()>>,
}

struct FileRefreshResult {
    events_added: usize,
    events_skipped: usize,
    partial_lines: usize,
    warnings: Vec<UsageWarning>,
}

#[derive(Debug, Clone)]
struct DiscoveredRollout {
    rollout_id: String,
    model_provider: Option<String>,
    is_subagent: bool,
    boundary_marker_required: bool,
}

struct RepriceEvent {
    event_id: String,
    occurred_at_ms: i64,
    model: String,
    provider_id: Option<String>,
    account_id: Option<String>,
    usage: crate::usage_log::TokenUsage,
    usage_quality: String,
    source_kind: UsageSourceKind,
}

#[derive(Debug, Clone)]
struct StoredCursor {
    last_path: String,
    byte_offset: u64,
    next_event_ordinal: u64,
    last_model: Option<String>,
    last_model_provider: Option<String>,
    usage_boundary_passed: bool,
    usage_boundary: i64,
    subagent_boundary_mode: i64,
    file_length: u64,
    file_modified_at_ms: Option<i64>,
    prefix_sha256: Option<String>,
}

#[derive(Debug, Clone)]
struct UsageAggregate {
    key: String,
    model: String,
    source_kind: UsageSourceKind,
    provider_id: Option<String>,
    account_id: Option<String>,
    source_name: String,
    tokens: TokenBreakdown,
    requests: u64,
    estimated_cost_microusd: u64,
    has_estimated: bool,
    has_subscription: bool,
    has_unpriced: bool,
    has_partial: bool,
    has_unattributed: bool,
    pricing_rule_name: Option<String>,
    pricing_rule_version: Option<u64>,
}

#[derive(Debug, Clone)]
struct ActivationSnapshot {
    effective_at_ms: i64,
    source_kind: UsageSourceKind,
    provider_id: Option<String>,
    account_id: Option<String>,
    #[allow(dead_code)]
    model_provider: Option<String>,
    display_name_snapshot: String,
}

enum ActivationResolution {
    Matched(ActivationSnapshot),
    Missing,
}

#[derive(Debug, Clone)]
enum AttributionOutcome {
    Confirmed(ActivationSnapshot),
    SourceOnly {
        source_kind: UsageSourceKind,
        display_name: String,
        allows_pricing: bool,
    },
    Unknown,
}

impl UsageLedger {
    pub(crate) fn open(app_data_root: &Path) -> anyhow::Result<Self> {
        fs::create_dir_all(app_data_root)?;
        secure_directory(app_data_root)?;
        let database_path = app_data_root.join("usage.sqlite3");
        let ledger = Self {
            database_path: Arc::new(database_path),
            refresh_lock: Arc::new(Mutex::new(())),
        };
        let connection = ledger.open_connection()?;
        initialize_schema(&connection)?;
        secure_file(&ledger.database_path)?;
        Ok(ledger)
    }

    pub(crate) fn refresh(
        &self,
        codex_home: &Path,
        now_utc_ms: i64,
    ) -> Result<UsageRefreshResult, AppError> {
        let _guard = self
            .refresh_lock
            .lock()
            .map_err(|_| AppError::Internal("本机用量刷新锁已损坏，请重启应用。".into()))?;
        let mut connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let mut result = UsageRefreshResult {
            files_scanned: 0,
            events_added: 0,
            events_skipped: 0,
            partial_lines: 0,
            warnings: vec![],
            last_refreshed_at_ms: now_utc_ms,
        };
        let mut paths = provider_sync::rollout_files(codex_home);
        paths.sort();
        paths.dedup();

        let collection_epoch = match read_collection_epoch(&connection)? {
            Some(epoch) => epoch,
            None => {
                initialize_collection_epoch(&mut connection, &paths, now_utc_ms)?;
                now_utc_ms
            }
        };
        rebuild_usage_for_parser_upgrade(&mut connection, collection_epoch)?;

        let activations = load_activations(&connection).map_err(AppError::from)?;
        let official_catalog = load_official_catalog(&connection).map_err(AppError::from)?;
        let pricing_rules = load_pricing_rule_records(&connection).map_err(AppError::from)?;

        for path in paths {
            result.files_scanned += 1;
            match refresh_file(
                &mut connection,
                &path,
                now_utc_ms,
                collection_epoch,
                &activations,
                &pricing_rules,
                official_catalog.as_ref(),
            ) {
                Ok(file_result) => {
                    result.events_added += file_result.events_added;
                    result.events_skipped += file_result.events_skipped;
                    result.partial_lines += file_result.partial_lines;
                    result.warnings.extend(file_result.warnings);
                }
                Err(error) => result.warnings.push(UsageWarning {
                    path: Some(path.display().to_string()),
                    message: error,
                }),
            }
        }

        connection
            .execute(
                "INSERT INTO usage_metadata(key, value) VALUES ('last_refreshed_at_ms', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![now_utc_ms.to_string()],
            )
            .map_err(|error| AppError::Internal(format!("保存本机用量刷新时间失败：{error}")))?;

        Ok(result)
    }

    pub(crate) fn query(&self, query: UsageQuery) -> Result<UsageOverview, AppError> {
        query.range.validate()?;
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;

        let collection_epoch = read_collection_epoch(&connection)?;
        let effective_start = collection_epoch
            .map(|epoch| query.range.start_at_ms.max(epoch))
            .unwrap_or(query.range.end_at_ms);
        let mut statement = connection
            .prepare(
                "SELECT occurred_at_ms, model, usage_events.source_kind, usage_events.provider_id,
                        COALESCE(account_identity_aliases.canonical_account_id, usage_events.account_id),
                        usage_events.source_name,
                        input_tokens, cached_input_tokens, cache_write_input_tokens,
                        output_tokens, reasoning_output_tokens, total_tokens,
                        cost_status, estimated_cost_microusd, pricing_rule_name,
                        pricing_rule_version
                 FROM usage_events
                 LEFT JOIN account_identity_aliases
                   ON account_identity_aliases.source_kind = usage_events.source_kind
                  AND account_identity_aliases.provider_id IS usage_events.provider_id
                  AND account_identity_aliases.local_account_id = usage_events.account_id
                WHERE occurred_at_ms >= ?1 AND occurred_at_ms < ?2
                 ORDER BY occurred_at_ms, event_ordinal",
            )
            .map_err(|error| AppError::Internal(format!("读取本机用量失败：{error}")))?;

        let rows = statement
            .query_map(params![effective_start, query.range.end_at_ms], |row| {
                read_usage_row(row, query.group_by)
            })
            .map_err(|error| AppError::Internal(format!("读取本机用量失败：{error}")))?;

        let mut aggregates = BTreeMap::<String, UsageAggregate>::new();
        let mut totals = UsageTotals {
            tokens: TokenBreakdown::default(),
            requests: 0,
            estimated_cost_microusd: 0,
            subscription_tokens: 0,
            unpriced_tokens: 0,
            partial_tokens: 0,
            unattributed_tokens: 0,
        };
        // 与 totals 同一趟查询产出按日趋势，避免对同一范围做第二趟全量扫描。
        let mut trend_points = BTreeMap::<i64, UsageTrendPoint>::new();

        for row in rows {
            let row =
                row.map_err(|error| AppError::Internal(format!("读取本机用量失败：{error}")))?;
            let key = aggregate_key(&row, query.group_by);
            // 只在新建分组时克隆 key，命中已有分组时通过 entry 直接取到可变引用，
            // 避免每行都克隆一次聚合键字符串。
            let aggregate = match aggregates.entry(key) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    let key = entry.key().clone();
                    entry.insert(UsageAggregate {
                        key,
                        model: if query.group_by == UsageGroupBy::Model {
                            row.model.clone()
                        } else {
                            "多个模型".into()
                        },
                        source_kind: row.source_kind,
                        provider_id: row.provider_id.clone(),
                        account_id: row.account_id.clone(),
                        source_name: row.source_name.clone(),
                        tokens: TokenBreakdown::default(),
                        requests: 0,
                        estimated_cost_microusd: 0,
                        has_estimated: false,
                        has_subscription: false,
                        has_unpriced: false,
                        has_partial: false,
                        has_unattributed: false,
                        pricing_rule_name: row.pricing_rule_name.clone(),
                        pricing_rule_version: row.pricing_rule_version,
                    })
                }
            };
            if query.group_by == UsageGroupBy::Model
                && (aggregate.source_kind != row.source_kind
                    || aggregate.provider_id != row.provider_id
                    || aggregate.account_id != row.account_id)
            {
                // “按模型”必须保证一个模型 ID 只出现一次。来源元数据不一致时，
                // 清除单一来源标识，避免把合并后的用量错误归到第一条事件的账号上。
                aggregate.source_kind = UsageSourceKind::Unattributed;
                aggregate.provider_id = None;
                aggregate.account_id = None;
                aggregate.source_name = "多个账号/来源".into();
            }
            add_tokens(&mut aggregate.tokens, &row.tokens);
            aggregate.requests = aggregate.requests.saturating_add(1);
            aggregate.estimated_cost_microusd = aggregate
                .estimated_cost_microusd
                .saturating_add(row.estimated_cost_microusd.unwrap_or(0));
            aggregate.has_estimated |= row.cost_status == CostStatus::Estimated;
            aggregate.has_subscription |= row.cost_status == CostStatus::Subscription;
            aggregate.has_unpriced |= row.cost_status == CostStatus::Unpriced;
            aggregate.has_partial |= row.cost_status == CostStatus::Partial;
            aggregate.has_unattributed |= row.cost_status == CostStatus::Unattributed
                || row.source_kind == UsageSourceKind::Unattributed;
            if aggregate.pricing_rule_version != row.pricing_rule_version {
                aggregate.pricing_rule_version = None;
            }

            add_tokens(&mut totals.tokens, &row.tokens);
            totals.requests = totals.requests.saturating_add(1);
            if row.cost_status == CostStatus::Estimated
                || (row.cost_status == CostStatus::Partial && row.estimated_cost_microusd.is_some())
            {
                totals.estimated_cost_microusd = totals
                    .estimated_cost_microusd
                    .saturating_add(row.estimated_cost_microusd.unwrap_or(0));
            }
            if row.cost_status == CostStatus::Unpriced {
                totals.unpriced_tokens = totals
                    .unpriced_tokens
                    .saturating_add(row.tokens.total_tokens);
            }
            if row.cost_status == CostStatus::Subscription {
                totals.subscription_tokens = totals
                    .subscription_tokens
                    .saturating_add(row.tokens.total_tokens);
            }
            if row.cost_status == CostStatus::Partial {
                totals.partial_tokens = totals
                    .partial_tokens
                    .saturating_add(row.tokens.total_tokens);
            }
            if row.cost_status == CostStatus::Unattributed
                || row.source_kind == UsageSourceKind::Unattributed
            {
                totals.unattributed_tokens = totals
                    .unattributed_tokens
                    .saturating_add(row.tokens.total_tokens);
            }

            accumulate_day_point(
                &mut trend_points,
                row.occurred_at_ms,
                &row.tokens,
                row.cost_status,
                row.estimated_cost_microusd,
                row.source_kind,
            );
        }

        let last_refreshed_at_ms = connection
            .query_row(
                "SELECT value FROM usage_metadata WHERE key = 'last_refreshed_at_ms'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| AppError::Internal(format!("读取本机用量刷新时间失败：{error}")))?
            .and_then(|value| value.parse::<i64>().ok());
        let collection_started_version =
            read_metadata(&connection, COLLECTION_VERSION_METADATA_KEY)?;

        let rows = aggregates
            .into_values()
            .map(|aggregate| {
                let cost_status = aggregate_cost_status(&aggregate);
                UsageRow {
                    key: aggregate.key,
                    model: aggregate.model,
                    source_kind: aggregate.source_kind,
                    provider_id: aggregate.provider_id,
                    account_id: aggregate.account_id,
                    source_name: aggregate.source_name,
                    tokens: aggregate.tokens,
                    requests: aggregate.requests,
                    estimated_cost_microusd: (aggregate.has_estimated
                        || aggregate.estimated_cost_microusd > 0)
                        .then_some(aggregate.estimated_cost_microusd),
                    cost_status,
                    pricing_rule_name: aggregate.pricing_rule_name,
                    pricing_rule_version: aggregate.pricing_rule_version,
                }
            })
            .collect();

        Ok(UsageOverview {
            range: query.range,
            totals,
            rows,
            last_refreshed_at_ms,
            collection_started_at_ms: collection_epoch,
            collection_started_version,
            warnings: vec![],
            trend_points: trend_points.into_values().collect(),
        })
    }

    /// 按本机自然日聚合用量事件，返回趋势序列（受统计周期起点约束）。
    pub(crate) fn trend(&self, range: UsageRange) -> Result<UsageTrend, AppError> {
        range.validate()?;
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;

        let collection_epoch = read_collection_epoch(&connection)?;
        let effective_start = collection_epoch
            .map(|epoch| range.start_at_ms.max(epoch))
            .unwrap_or(range.end_at_ms);
        let mut statement = connection
            .prepare(
                "SELECT occurred_at_ms, source_kind,
                        input_tokens, cached_input_tokens, cache_write_input_tokens,
                        output_tokens, reasoning_output_tokens, total_tokens,
                        cost_status, estimated_cost_microusd
                 FROM usage_events
                WHERE occurred_at_ms >= ?1 AND occurred_at_ms < ?2
                 ORDER BY occurred_at_ms, event_ordinal",
            )
            .map_err(|error| AppError::Internal(format!("读取本机用量趋势失败：{error}")))?;

        let rows = statement
            .query_map(params![effective_start, range.end_at_ms], |row| {
                let tokens = TokenBreakdown {
                    input_tokens: i64_to_u64(row.get(2)?)?,
                    cached_input_tokens: i64_to_u64(row.get(3)?)?,
                    cache_write_input_tokens: i64_to_u64(row.get(4)?)?,
                    output_tokens: i64_to_u64(row.get(5)?)?,
                    reasoning_output_tokens: i64_to_u64(row.get(6)?)?,
                    total_tokens: i64_to_u64(row.get(7)?)?,
                };
                Ok(DbTrendRow {
                    occurred_at_ms: row.get(0)?,
                    source_kind: parse_source_kind(&row.get::<_, String>(1)?),
                    tokens,
                    cost_status: parse_cost_status(&row.get::<_, String>(8)?),
                    estimated_cost_microusd: row
                        .get::<_, Option<i64>>(9)?
                        .map(i64_to_u64)
                        .transpose()?,
                })
            })
            .map_err(|error| AppError::Internal(format!("读取本机用量趋势失败：{error}")))?;

        let mut points = BTreeMap::<i64, UsageTrendPoint>::new();
        for row in rows {
            let row =
                row.map_err(|error| AppError::Internal(format!("读取本机用量趋势失败：{error}")))?;
            accumulate_day_point(
                &mut points,
                row.occurred_at_ms,
                &row.tokens,
                row.cost_status,
                row.estimated_cost_microusd,
                row.source_kind,
            );
        }

        Ok(UsageTrend {
            range,
            points: points.into_values().collect(),
        })
    }

    pub(crate) fn begin_activation(
        &self,
        activation: &ActivationRecord,
    ) -> Result<String, AppError> {
        validate_activation(activation)?;
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let id = Uuid::new_v4().to_string();
        insert_activation_row(&connection, &id, activation, "pending")?;
        Ok(id)
    }

    pub(crate) fn sync_official_account_identities(
        &self,
        accounts: &[(String, String)],
    ) -> Result<(), AppError> {
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let now = Utc::now().timestamp_millis();
        for (local_account_id, external_account_id) in accounts {
            if local_account_id.trim().is_empty() || external_account_id.trim().is_empty() {
                continue;
            }
            let canonical_account_id =
                crate::models::canonical_official_account_id(external_account_id);
            connection
                .execute(
                    "INSERT INTO account_identity_aliases(
                        source_kind, provider_id, local_account_id,
                        canonical_account_id, identity_source, created_at_ms
                     ) VALUES ('official', NULL, ?1, ?2, 'official_external_id', ?3)
                     ON CONFLICT(source_kind, provider_id, local_account_id) DO UPDATE SET
                        canonical_account_id = excluded.canonical_account_id,
                        identity_source = excluded.identity_source",
                    params![local_account_id.trim(), canonical_account_id, now],
                )
                .map_err(|error| AppError::Internal(format!("保存账号归一化映射失败：{error}")))?;
        }
        Ok(())
    }

    pub(crate) fn confirm_activation(&self, id: &str) -> Result<(), AppError> {
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        update_activation_status(&connection, id, "confirmed")
    }

    pub(crate) fn cancel_activation(&self, id: &str) -> Result<(), AppError> {
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        update_activation_status(&connection, id, "cancelled")
    }

    pub(crate) fn cancel_pending_activations(&self) -> Result<(), AppError> {
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        connection
            .execute(
                "UPDATE activation_history SET status = 'cancelled'
                 WHERE status = 'pending'",
                [],
            )
            .map_err(|error| AppError::Internal(format!("清理未完成账号切换记录失败：{error}")))?;
        Ok(())
    }

    pub(crate) fn record_activation(&self, activation: ActivationRecord) -> Result<(), AppError> {
        validate_activation(&activation)?;
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let duplicate = connection
            .query_row(
                "SELECT source_kind, provider_id, account_id, model_provider,
                        display_name_snapshot
                 FROM activation_history
                 WHERE status = 'confirmed'
                 ORDER BY effective_at_ms DESC, created_at_ms DESC
                 LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| AppError::Internal(format!("检查账号激活记录失败：{error}")))?;
        let duplicate = duplicate.is_some_and(
            |(source_kind, provider_id, account_id, model_provider, display_name)| {
                source_kind == source_kind_text(activation.source_kind)
                    && provider_id == activation.provider_id
                    && account_id == activation.account_id
                    && model_provider == activation.model_provider
                    && display_name == activation.display_name_snapshot.trim()
            },
        );
        if duplicate {
            return Ok(());
        }
        let id = Uuid::new_v4().to_string();
        insert_activation_row(&connection, &id, &activation, "confirmed")
    }

    pub(crate) fn usage_list_pricing_rules(
        &self,
        scope: Option<PricingScope>,
    ) -> Result<Vec<PricingRule>, AppError> {
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let rules = load_pricing_rule_dtos(&connection)?;
        Ok(rules
            .into_iter()
            .filter(|rule| {
                rule.active
                    && scope.as_ref().is_none_or(|scope| {
                        rule.scope_kind == scope.scope_kind
                            && rule.provider_id == scope.provider_id
                            && rule.account_id == scope.account_id
                    })
            })
            .collect())
    }

    pub(crate) fn official_pricing_catalog(
        &self,
    ) -> Result<Option<OfficialPricingCatalog>, AppError> {
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        load_official_catalog(&connection).map_err(AppError::from)
    }

    pub(crate) fn save_official_pricing_catalog(
        &self,
        catalog: &OfficialPricingCatalog,
        _now_utc_ms: i64,
    ) -> Result<bool, AppError> {
        let serialized = serde_json::to_string(catalog)
            .map_err(|error| AppError::Internal(format!("序列化官方价格目录失败：{error}")))?;
        let mut connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| AppError::Internal(format!("开始保存官方价格目录失败：{error}")))?;
        let existing = transaction
            .query_row(
                "SELECT version FROM official_pricing_catalogs
                 WHERE content_sha256 = ?1 LIMIT 1",
                params![catalog.content_sha256],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| AppError::Internal(format!("检查官方价格目录版本失败：{error}")))?;
        if let Some(existing_version) = existing {
            transaction
                .execute("UPDATE official_pricing_catalogs SET active = 0", [])
                .map_err(|error| AppError::Internal(format!("停用旧官方价格目录失败：{error}")))?;
            transaction
                .execute(
                    "UPDATE official_pricing_catalogs SET active = 1 WHERE version = ?1",
                    params![existing_version],
                )
                .map_err(|error| AppError::Internal(format!("激活官方价格目录失败：{error}")))?;
            transaction
                .commit()
                .map_err(|error| AppError::Internal(format!("提交官方价格目录失败：{error}")))?;
            return Ok(false);
        }
        transaction
            .execute("UPDATE official_pricing_catalogs SET active = 0", [])
            .map_err(|error| AppError::Internal(format!("停用旧官方价格目录失败：{error}")))?;
        transaction
            .execute(
                "INSERT INTO official_pricing_catalogs(
                    version, content_sha256, source_url, etag, last_modified,
                    fetched_at_ms, normalized_json, active
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
                params![
                    catalog.version,
                    catalog.content_sha256,
                    catalog.source_url,
                    catalog.etag,
                    catalog.last_modified,
                    catalog.fetched_at_ms,
                    serialized,
                ],
            )
            .map_err(|error| AppError::Internal(format!("写入官方价格目录失败：{error}")))?;
        transaction
            .execute(
                "DELETE FROM official_pricing_catalogs
                 WHERE version NOT IN (
                   SELECT version FROM official_pricing_catalogs
                   ORDER BY fetched_at_ms DESC LIMIT 3
                 )",
                [],
            )
            .map_err(|error| AppError::Internal(format!("清理旧官方价格目录失败：{error}")))?;
        transaction
            .commit()
            .map_err(|error| AppError::Internal(format!("提交官方价格目录失败：{error}")))?;
        Ok(true)
    }

    pub(crate) fn reprice_current_cycle(&self, now_utc_ms: i64) -> Result<RepriceResult, AppError> {
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let start = read_collection_epoch(&connection)?.unwrap_or(now_utc_ms);
        drop(connection);
        self.reprice(UsageRange {
            start_at_ms: start,
            end_at_ms: now_utc_ms.saturating_add(1),
        })
    }

    pub(crate) fn usage_save_pricing_rule(
        &self,
        input: SavePricingRule,
    ) -> Result<PricingRule, AppError> {
        let now = Utc::now().timestamp_millis();
        let (mut input, mut internal) = normalize_pricing_input(input, now)?;
        let mut connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| AppError::Internal(format!("开始保存美元价格规则失败：{error}")))?;
        let version: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM pricing_rules
                 WHERE scope_kind = ?1 AND provider_id IS ?2 AND account_id IS ?3
                   AND model_pattern = ?4 AND match_kind = ?5",
                params![
                    pricing_scope_text(internal.scope_kind),
                    internal.provider_id,
                    internal.account_id,
                    internal.model_pattern,
                    pricing_match_text(internal.match_kind),
                ],
                |row| row.get(0),
            )
            .map_err(|error| AppError::Internal(format!("读取美元价格规则版本失败：{error}")))?;
        if !input.id.trim().is_empty() {
            transaction
                .execute(
                    "UPDATE pricing_rules SET active = 0, updated_at_ms = ?1 WHERE id = ?2",
                    params![now, input.id.trim()],
                )
                .map_err(|error| AppError::Internal(format!("停用旧美元价格规则失败：{error}")))?;
        }
        internal.id = Uuid::new_v4().to_string();
        internal.version = version;
        internal.active = true;
        transaction
            .execute(
                "INSERT INTO pricing_rules(
                    id, version, active, scope_kind, provider_id, account_id,
                    model_pattern, match_kind, billing_mode,
                    input_microusd_per_million, cached_read_microusd_per_million,
                    cache_write_microusd_per_million, output_microusd_per_million,
                    request_fee_microusd, cache_write_included_in_input,
                    effective_from_ms, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6, ?7, ?8,
                           ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![
                    internal.id,
                    internal.version,
                    pricing_scope_text(internal.scope_kind),
                    internal.provider_id,
                    internal.account_id,
                    internal.model_pattern,
                    pricing_match_text(internal.match_kind),
                    billing_mode_text(internal.billing_mode),
                    internal.input_microusd_per_million,
                    internal.cached_read_microusd_per_million,
                    internal.cache_write_microusd_per_million,
                    internal.output_microusd_per_million,
                    internal.request_fee_microusd,
                    if internal.cache_write_included_in_input {
                        1
                    } else {
                        0
                    },
                    internal.effective_from_ms,
                    now,
                    now,
                ],
            )
            .map_err(|error| AppError::Internal(format!("保存美元价格规则失败：{error}")))?;
        transaction
            .commit()
            .map_err(|error| AppError::Internal(format!("提交美元价格规则失败：{error}")))?;
        input.id = internal.id.clone();
        input.version = u64::try_from(internal.version).unwrap_or_default();
        input.active = true;
        input.created_at_ms = now;
        input.updated_at_ms = now;
        Ok(input)
    }

    pub(crate) fn usage_delete_pricing_rule(&self, id: &str) -> Result<(), AppError> {
        let id = id.trim();
        if id.is_empty() {
            return Err(AppError::InvalidConfig("美元价格规则 ID 不能为空。".into()));
        }
        let connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let changed = connection
            .execute(
                "UPDATE pricing_rules SET active = 0, updated_at_ms = ?1 WHERE id = ?2 AND active = 1",
                params![Utc::now().timestamp_millis(), id],
            )
            .map_err(|error| AppError::Internal(format!("停用美元价格规则失败：{error}")))?;
        if changed == 0 {
            return Err(AppError::InvalidConfig(
                "美元价格规则不存在或已经停用。".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn reprice(&self, range: UsageRange) -> Result<RepriceResult, AppError> {
        range.validate()?;
        let mut connection = self.open_connection().map_err(AppError::from)?;
        initialize_schema(&connection).map_err(AppError::from)?;
        let collection_epoch = read_collection_epoch(&connection)?;
        let effective_start = collection_epoch
            .map(|epoch| range.start_at_ms.max(epoch))
            .unwrap_or(range.end_at_ms);
        let rules = load_pricing_rule_records(&connection).map_err(AppError::from)?;
        let official_catalog = load_official_catalog(&connection).map_err(AppError::from)?;
        let mut statement = connection
            .prepare(
                "SELECT event_id, occurred_at_ms, model, provider_id, account_id,
                        input_tokens, cached_input_tokens, cache_write_input_tokens,
                        output_tokens, reasoning_output_tokens, total_tokens,
                        usage_quality, source_kind
                 FROM usage_events
                 WHERE occurred_at_ms >= ?1 AND occurred_at_ms < ?2",
            )
            .map_err(|error| AppError::Internal(format!("读取待重算用量失败：{error}")))?;
        let events = statement
            .query_map(params![effective_start, range.end_at_ms], |row| {
                Ok(RepriceEvent {
                    event_id: row.get(0)?,
                    occurred_at_ms: row.get(1)?,
                    model: row.get(2)?,
                    provider_id: row.get(3)?,
                    account_id: row.get(4)?,
                    usage: crate::usage_log::TokenUsage {
                        input_tokens: i64_to_u64(row.get(5)?)?,
                        cached_input_tokens: i64_to_u64(row.get(6)?)?,
                        cache_write_input_tokens: i64_to_u64(row.get(7)?)?,
                        output_tokens: i64_to_u64(row.get(8)?)?,
                        reasoning_output_tokens: i64_to_u64(row.get(9)?)?,
                        total_tokens: i64_to_u64(row.get(10)?)?,
                    },
                    usage_quality: row.get(11)?,
                    source_kind: parse_source_kind(&row.get::<_, String>(12)?),
                })
            })
            .map_err(|error| AppError::Internal(format!("读取待重算用量失败：{error}")))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| AppError::Internal(format!("读取待重算用量失败：{error}")))?;
        drop(statement);
        let transaction = connection
            .transaction()
            .map_err(|error| AppError::Internal(format!("开始重算用量事务失败：{error}")))?;
        let mut result = RepriceResult {
            events_repriced: 0,
            estimated_cost_microusd: 0,
            unpriced_events: 0,
        };
        for event in events {
            if event.source_kind == UsageSourceKind::Unattributed
                || (event.source_kind == UsageSourceKind::Provider
                    && event.provider_id.is_none()
                    && event.account_id.is_none())
            {
                transaction
                    .execute(
                        "UPDATE usage_events SET cost_status = 'unattributed',
                                estimated_cost_microusd = NULL,
                                pricing_rule_id = NULL, pricing_rule_version = NULL,
                                pricing_rule_name = NULL WHERE event_id = ?1",
                        params![event.event_id],
                    )
                    .map_err(|error| AppError::Internal(format!("更新未归属用量失败：{error}")))?;
                result.events_repriced += 1;
                continue;
            }
            let outcome = price_for_source(
                event.source_kind,
                &rules,
                official_catalog.as_ref(),
                &event.usage,
                &PricingContext {
                    model: &event.model,
                    provider_id: event.provider_id.as_deref(),
                    account_id: event.account_id.as_deref(),
                    effective_at_ms: event.occurred_at_ms,
                },
            );
            let (rule_id, version, cost, status, is_unpriced) = match outcome {
                PricingOutcome::Estimated {
                    cost_microusd,
                    rule_id,
                    version,
                } => (
                    Some(rule_id),
                    Some(version),
                    Some(cost_microusd),
                    if event.usage_quality == "complete" {
                        "estimated"
                    } else {
                        "partial"
                    },
                    false,
                ),
                PricingOutcome::Subscription { rule_id, version } => (
                    Some(rule_id),
                    Some(version),
                    None,
                    if event.usage_quality == "complete" {
                        "subscription"
                    } else {
                        "partial"
                    },
                    false,
                ),
                PricingOutcome::Unpriced { rule_id, .. } => (
                    rule_id,
                    None,
                    None,
                    if event.usage_quality == "complete" {
                        "unpriced"
                    } else {
                        "partial"
                    },
                    true,
                ),
            };
            let rule_name = pricing_rule_label(rule_id.as_deref(), &rules);
            transaction
                .execute(
                    "UPDATE usage_events SET cost_status = ?1,
                            estimated_cost_microusd = ?2,
                            pricing_rule_id = ?3, pricing_rule_version = ?4,
                            pricing_rule_name = ?5 WHERE event_id = ?6",
                    params![status, cost, rule_id, version, rule_name, event.event_id],
                )
                .map_err(|error| AppError::Internal(format!("更新用量价格失败：{error}")))?;
            result.events_repriced += 1;
            result.estimated_cost_microusd = result
                .estimated_cost_microusd
                .saturating_add(u64::try_from(cost.unwrap_or(0)).unwrap_or_default());
            if is_unpriced {
                result.unpriced_events += 1;
            }
        }
        transaction
            .commit()
            .map_err(|error| AppError::Internal(format!("提交重算用量事务失败：{error}")))?;
        Ok(result)
    }

    fn open_connection(&self) -> anyhow::Result<Connection> {
        let connection = Connection::open(self.database_path.as_ref())?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        Ok(connection)
    }
}

fn initialize_schema(connection: &Connection) -> anyhow::Result<()> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        anyhow::bail!("本机用量数据库版本较新，当前版本无法读取。")
    }
    if version == 0 {
        connection.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS usage_metadata (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS account_identity_aliases (
               source_kind TEXT NOT NULL,
               provider_id TEXT,
               local_account_id TEXT NOT NULL,
               canonical_account_id TEXT NOT NULL,
               identity_source TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL,
               PRIMARY KEY (source_kind, provider_id, local_account_id)
             );
             CREATE TABLE IF NOT EXISTS usage_events (
               event_id TEXT PRIMARY KEY,
               rollout_id TEXT NOT NULL,
               event_ordinal INTEGER NOT NULL,
               occurred_at_ms INTEGER NOT NULL,
               model TEXT NOT NULL,
               model_provider TEXT,
               source_kind TEXT NOT NULL CHECK (source_kind IN ('official', 'provider', 'unattributed')),
               provider_id TEXT,
               account_id TEXT,
               source_name TEXT NOT NULL,
               input_tokens INTEGER NOT NULL CHECK (input_tokens >= 0),
               cached_input_tokens INTEGER NOT NULL CHECK (cached_input_tokens >= 0),
               cache_write_input_tokens INTEGER NOT NULL CHECK (cache_write_input_tokens >= 0),
               output_tokens INTEGER NOT NULL CHECK (output_tokens >= 0),
               reasoning_output_tokens INTEGER NOT NULL CHECK (reasoning_output_tokens >= 0),
               total_tokens INTEGER NOT NULL CHECK (total_tokens >= 0),
               usage_quality TEXT NOT NULL CHECK (usage_quality IN ('complete', 'partial', 'compatible_fallback')),
               pricing_rule_id TEXT,
               pricing_rule_version INTEGER,
               pricing_rule_name TEXT,
               estimated_cost_microusd INTEGER CHECK (estimated_cost_microusd IS NULL OR estimated_cost_microusd >= 0),
               cost_status TEXT NOT NULL CHECK (cost_status IN ('estimated', 'subscription', 'unpriced', 'partial', 'unattributed', 'zero')),
               created_at_ms INTEGER NOT NULL,
               UNIQUE (rollout_id, event_ordinal)
             );
             CREATE INDEX IF NOT EXISTS usage_events_time_idx
               ON usage_events(occurred_at_ms);
             CREATE INDEX IF NOT EXISTS usage_events_account_idx
               ON usage_events(account_id, occurred_at_ms);
             CREATE INDEX IF NOT EXISTS usage_events_provider_idx
               ON usage_events(provider_id, occurred_at_ms);
             CREATE INDEX IF NOT EXISTS usage_events_model_idx
               ON usage_events(model, occurred_at_ms);
             CREATE TABLE IF NOT EXISTS usage_cursors (
               rollout_id TEXT PRIMARY KEY,
               last_path TEXT NOT NULL,
               byte_offset INTEGER NOT NULL,
               next_event_ordinal INTEGER NOT NULL,
               last_model TEXT,
               last_model_provider TEXT,
               usage_boundary_passed INTEGER NOT NULL DEFAULT 1
                 CHECK (usage_boundary_passed IN (0, 1)),
               usage_boundary_state INTEGER NOT NULL DEFAULT 0
                 CHECK (usage_boundary_state IN (0, 1, 2, 3)),
               subagent_boundary_mode INTEGER NOT NULL DEFAULT 0
                 CHECK (subagent_boundary_mode IN (0, 1, 2)),
                file_length INTEGER NOT NULL,
                file_modified_at_ms INTEGER,
                prefix_sha256 TEXT,
                updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS activation_history (
               id TEXT PRIMARY KEY,
               effective_at_ms INTEGER NOT NULL,
               source_kind TEXT NOT NULL CHECK (source_kind IN ('official', 'provider', 'unattributed')),
               provider_id TEXT,
               account_id TEXT,
               model_provider TEXT,
               display_name_snapshot TEXT NOT NULL,
               auth_source TEXT,
               status TEXT NOT NULL CHECK (status IN ('pending', 'confirmed', 'cancelled')),
               created_at_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS activation_history_time_idx
               ON activation_history(effective_at_ms);
             CREATE TABLE IF NOT EXISTS pricing_rules (
               id TEXT PRIMARY KEY,
               version INTEGER NOT NULL,
               active INTEGER NOT NULL,
               scope_kind TEXT NOT NULL CHECK (scope_kind IN ('account_model', 'provider_model', 'global_model', 'provider_default')),
               provider_id TEXT,
               account_id TEXT,
               model_pattern TEXT NOT NULL,
               match_kind TEXT NOT NULL CHECK (match_kind IN ('exact', 'prefix')),
               billing_mode TEXT NOT NULL CHECK (billing_mode IN ('token', 'unpriced')),
               input_microusd_per_million INTEGER,
               cached_read_microusd_per_million INTEGER,
               cache_write_microusd_per_million INTEGER,
               output_microusd_per_million INTEGER,
               request_fee_microusd INTEGER,
               cache_write_included_in_input INTEGER NOT NULL,
               effective_from_ms INTEGER NOT NULL,
               created_at_ms INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS official_pricing_catalogs (
               version INTEGER PRIMARY KEY,
               content_sha256 TEXT NOT NULL UNIQUE,
               source_url TEXT NOT NULL,
               etag TEXT,
               last_modified TEXT,
               fetched_at_ms INTEGER NOT NULL,
               normalized_json TEXT NOT NULL,
               active INTEGER NOT NULL CHECK (active IN (0, 1))
             );
             CREATE INDEX IF NOT EXISTS official_pricing_catalog_active_idx
               ON official_pricing_catalogs(active, fetched_at_ms DESC);
              PRAGMA user_version = 7;
             COMMIT;",
        )?;
        return Ok(());
    }
    // 旧版数据库（v1–v3）按序迁移到 v4，保留用户已有的用量历史与定价规则，
    // 而不是让用户删除数据库重新采集。
    if version == 1 {
        connection.execute_batch(
            "BEGIN;
             ALTER TABLE usage_cursors
               ADD COLUMN usage_boundary_passed INTEGER NOT NULL DEFAULT 1
                 CHECK (usage_boundary_passed IN (0, 1));
             ALTER TABLE usage_cursors
               ADD COLUMN usage_boundary_state INTEGER NOT NULL DEFAULT 0
                 CHECK (usage_boundary_state IN (0, 1, 2, 3));
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
    }
    if version == 2 {
        connection.execute_batch(
            "BEGIN;
             ALTER TABLE usage_cursors
               ADD COLUMN usage_boundary_state INTEGER NOT NULL DEFAULT 0
                 CHECK (usage_boundary_state IN (0, 1, 2, 3));
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
    }
    if version == 3 {
        connection.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS official_pricing_catalogs (
               version INTEGER PRIMARY KEY,
               content_sha256 TEXT NOT NULL UNIQUE,
               source_url TEXT NOT NULL,
               etag TEXT,
               last_modified TEXT,
               fetched_at_ms INTEGER NOT NULL,
               normalized_json TEXT NOT NULL,
               active INTEGER NOT NULL CHECK (active IN (0, 1))
             );
             CREATE INDEX IF NOT EXISTS official_pricing_catalog_active_idx
               ON official_pricing_catalogs(active, fetched_at_ms DESC);
             PRAGMA user_version = 4;
             COMMIT;",
        )?;
    }
    // subagent_boundary_mode was introduced in v5. Keep this condition tied to
    // that migration instead of SCHEMA_VERSION so opening a v5 database during
    // later upgrades does not try to add the existing column again.
    if version > 0 && version < 5 {
        connection.execute_batch(
            "BEGIN;
             ALTER TABLE usage_cursors
               ADD COLUMN subagent_boundary_mode INTEGER NOT NULL DEFAULT 0
                 CHECK (subagent_boundary_mode IN (0, 1, 2));
             COMMIT;",
        )?;
    }
    if version < 5 {
        connection.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS account_identity_aliases (
               source_kind TEXT NOT NULL,
               provider_id TEXT,
               local_account_id TEXT NOT NULL,
               canonical_account_id TEXT NOT NULL,
               identity_source TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL,
               PRIMARY KEY (source_kind, provider_id, local_account_id)
             );
             CREATE TABLE IF NOT EXISTS official_pricing_catalogs (
               version INTEGER PRIMARY KEY,
               content_sha256 TEXT NOT NULL UNIQUE,
               source_url TEXT NOT NULL,
               etag TEXT,
               last_modified TEXT,
               fetched_at_ms INTEGER NOT NULL,
               normalized_json TEXT NOT NULL,
               active INTEGER NOT NULL CHECK (active IN (0, 1))
             );
             CREATE INDEX IF NOT EXISTS official_pricing_catalog_active_idx
               ON official_pricing_catalogs(active, fetched_at_ms DESC);
             PRAGMA user_version = 5;
             COMMIT;",
        )?;
    }
    if version > 0 && version < 6 {
        connection.execute_batch(
            "BEGIN;
             ALTER TABLE pricing_rules RENAME TO pricing_rules_v5;
             CREATE TABLE pricing_rules (
               id TEXT PRIMARY KEY,
               version INTEGER NOT NULL,
               active INTEGER NOT NULL,
               scope_kind TEXT NOT NULL CHECK (scope_kind IN ('account_model', 'provider_model', 'global_model', 'provider_default')),
               provider_id TEXT,
               account_id TEXT,
               model_pattern TEXT NOT NULL,
               match_kind TEXT NOT NULL CHECK (match_kind IN ('exact', 'prefix')),
               billing_mode TEXT NOT NULL CHECK (billing_mode IN ('token', 'unpriced')),
               input_microusd_per_million INTEGER,
               cached_read_microusd_per_million INTEGER,
               cache_write_microusd_per_million INTEGER,
               output_microusd_per_million INTEGER,
               request_fee_microusd INTEGER,
               cache_write_included_in_input INTEGER NOT NULL,
               effective_from_ms INTEGER NOT NULL,
               created_at_ms INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL
             );
             INSERT INTO pricing_rules(
               id, version, active, scope_kind, provider_id, account_id,
               model_pattern, match_kind, billing_mode,
               input_microusd_per_million, cached_read_microusd_per_million,
               cache_write_microusd_per_million, output_microusd_per_million,
               request_fee_microusd, cache_write_included_in_input,
               effective_from_ms, created_at_ms, updated_at_ms
             )
             SELECT
               id, version, active, scope_kind, provider_id, account_id,
               model_pattern, match_kind,
               CASE billing_mode WHEN 'subscription' THEN 'unpriced' ELSE billing_mode END,
               input_microusd_per_million, cached_read_microusd_per_million,
               cache_write_microusd_per_million, output_microusd_per_million,
               request_fee_microusd, cache_write_included_in_input,
               effective_from_ms, created_at_ms, updated_at_ms
             FROM pricing_rules_v5;
             DROP TABLE pricing_rules_v5;
             PRAGMA user_version = 6;
             COMMIT;",
        )?;
    }
    if version > 0 && version < 7 {
        connection.execute_batch(
            "BEGIN;
             ALTER TABLE usage_cursors ADD COLUMN prefix_sha256 TEXT;
             PRAGMA user_version = 7;
             COMMIT;",
        )?;
    }
    Ok(())
}

fn read_metadata(connection: &Connection, key: &str) -> Result<Option<String>, AppError> {
    connection
        .query_row(
            "SELECT value FROM usage_metadata WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| AppError::Internal(format!("读取本机用量元数据失败：{error}")))
}

fn read_collection_epoch(connection: &Connection) -> Result<Option<i64>, AppError> {
    Ok(read_metadata(connection, COLLECTION_EPOCH_METADATA_KEY)?
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0))
}

fn rebuild_usage_for_parser_upgrade(
    connection: &mut Connection,
    collection_epoch: i64,
) -> Result<(), AppError> {
    if read_metadata(connection, PARSER_VERSION_METADATA_KEY)?.as_deref() == Some(PARSER_VERSION) {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .map_err(|error| AppError::Internal(format!("开始升级本机用量解析器失败：{error}")))?;
    transaction
        .execute(
            "DELETE FROM usage_events WHERE occurred_at_ms >= ?1",
            params![collection_epoch],
        )
        .map_err(|error| AppError::Internal(format!("清理当前统计周期用量失败：{error}")))?;
    transaction
        .execute("DELETE FROM usage_cursors", [])
        .map_err(|error| AppError::Internal(format!("重置本机用量游标失败：{error}")))?;
    transaction
        .execute(
            "INSERT INTO usage_metadata(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![PARSER_VERSION_METADATA_KEY, PARSER_VERSION],
        )
        .map_err(|error| AppError::Internal(format!("保存本机用量解析器版本失败：{error}")))?;
    transaction
        .commit()
        .map_err(|error| AppError::Internal(format!("提交本机用量解析器升级失败：{error}")))?;
    Ok(())
}

fn initialize_collection_epoch(
    connection: &mut Connection,
    paths: &[PathBuf],
    now_utc_ms: i64,
) -> Result<(), AppError> {
    let transaction = connection
        .transaction()
        .map_err(|error| AppError::Internal(format!("初始化本机用量统计周期失败：{error}")))?;

    transaction
        .execute(
            "INSERT INTO usage_metadata(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO NOTHING",
            params![COLLECTION_EPOCH_METADATA_KEY, now_utc_ms.to_string()],
        )
        .map_err(|error| AppError::Internal(format!("保存本机用量统计起点失败：{error}")))?;
    transaction
        .execute(
            "INSERT INTO usage_metadata(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO NOTHING",
            params![COLLECTION_VERSION_METADATA_KEY, COLLECTION_VERSION],
        )
        .map_err(|error| AppError::Internal(format!("保存本机用量版本失败：{error}")))?;
    transaction
        .execute(
            "INSERT INTO usage_metadata(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO NOTHING",
            params![COLLECTION_MODE_METADATA_KEY, COLLECTION_MODE],
        )
        .map_err(|error| AppError::Internal(format!("保存本机用量统计模式失败：{error}")))?;
    transaction
        .execute(
            "INSERT INTO usage_metadata(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO NOTHING",
            params![PARSER_VERSION_METADATA_KEY, PARSER_VERSION],
        )
        .map_err(|error| AppError::Internal(format!("保存本机用量解析器版本失败：{error}")))?;

    for path in paths {
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        let Some(discovered) = discover_rollout(path)
            .map_err(|error| AppError::Internal(format!("初始化本机用量文件游标失败：{error}")))?
        else {
            continue;
        };
        let replayed_state = if discovered.is_subagent {
            rollout_state(path, discovered.boundary_marker_required).map_err(|error| {
                AppError::Internal(format!("初始化本机用量子任务边界失败：{error}"))
            })?
        } else {
            rollout_state(path, discovered.boundary_marker_required)
                .map_err(|error| AppError::Internal(format!("初始化本机用量上下文失败：{error}")))?
        };
        let usage_boundary = if discovered.is_subagent {
            replayed_state.usage_boundary
        } else {
            UsageBoundaryState::Regular
        };
        let rollout_id = discovered.rollout_id.clone();
        let prefix_sha256 = rollout_prefix_hasher(path, metadata.len())
            .map_err(|error| AppError::Internal(format!("初始化会话文件指纹失败：{error}")))?
            .finalize_hex();
        let (next_event_ordinal, last_model, last_model_provider) = transaction
            .query_row(
                "SELECT COALESCE(MAX(event_ordinal), -1) + 1, model, model_provider
                 FROM usage_events WHERE rollout_id = ?1",
                params![rollout_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                },
            )
            .map_err(|error| AppError::Internal(format!("读取本机用量文件状态失败：{error}")))?;
        transaction
            .execute(
                "INSERT INTO usage_cursors(
                    rollout_id, last_path, byte_offset, next_event_ordinal,
                    last_model, last_model_provider, usage_boundary_passed,
                    usage_boundary_state, subagent_boundary_mode,
                    file_length, file_modified_at_ms, prefix_sha256, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(rollout_id) DO UPDATE SET
                   last_path = excluded.last_path,
                   byte_offset = excluded.byte_offset,
                   next_event_ordinal = excluded.next_event_ordinal,
                   last_model = excluded.last_model,
                   last_model_provider = excluded.last_model_provider,
                   usage_boundary_passed = excluded.usage_boundary_passed,
                   usage_boundary_state = excluded.usage_boundary_state,
                   subagent_boundary_mode = excluded.subagent_boundary_mode,
                   file_length = excluded.file_length,
                   file_modified_at_ms = excluded.file_modified_at_ms,
                   prefix_sha256 = excluded.prefix_sha256,
                   updated_at_ms = excluded.updated_at_ms",
                params![
                    rollout_id,
                    path.display().to_string(),
                    i64::try_from(metadata.len()).map_err(|_| {
                        AppError::Internal("会话文件大小超过数据库范围。".into())
                    })?,
                    next_event_ordinal
                        .max(0)
                        .max(i64::try_from(replayed_state.next_event_ordinal).unwrap_or(i64::MAX)),
                    replayed_state.model.or(last_model),
                    replayed_state
                        .model_provider
                        .or(last_model_provider)
                        .or(discovered.model_provider.clone()),
                    i64::from(usage_boundary_passed(usage_boundary)),
                    usage_boundary_code(usage_boundary),
                    subagent_boundary_mode(&discovered),
                    i64::try_from(metadata.len()).map_err(|_| {
                        AppError::Internal("会话文件大小超过数据库范围。".into())
                    })?,
                    metadata
                        .modified()
                        .ok()
                        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                        .and_then(|value| i64::try_from(value.as_millis()).ok()),
                    prefix_sha256,
                    now_utc_ms,
                ],
            )
            .map_err(|error| AppError::Internal(format!("保存本机用量文件游标失败：{error}")))?;
    }

    transaction
        .commit()
        .map_err(|error| AppError::Internal(format!("提交本机用量统计周期失败：{error}")))?;
    Ok(())
}

fn validate_activation(activation: &ActivationRecord) -> Result<(), AppError> {
    if activation.effective_at_ms <= 0 || activation.display_name_snapshot.trim().is_empty() {
        return Err(AppError::InvalidConfig(
            "账号激活记录缺少有效时间或名称。".into(),
        ));
    }
    Ok(())
}

fn insert_activation_row(
    connection: &Connection,
    id: &str,
    activation: &ActivationRecord,
    status: &str,
) -> Result<(), AppError> {
    connection
        .execute(
            "INSERT INTO activation_history(
                id, effective_at_ms, source_kind, provider_id, account_id,
                model_provider, display_name_snapshot, auth_source,
                status, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                activation.effective_at_ms,
                source_kind_text(activation.source_kind),
                activation.provider_id,
                activation.account_id,
                activation.model_provider,
                activation.display_name_snapshot.trim(),
                activation.auth_source,
                status,
                Utc::now().timestamp_millis(),
            ],
        )
        .map_err(|error| AppError::Internal(format!("保存账号激活记录失败：{error}")))?;
    Ok(())
}

fn update_activation_status(
    connection: &Connection,
    id: &str,
    status: &str,
) -> Result<(), AppError> {
    if id.trim().is_empty() {
        return Err(AppError::InvalidConfig("账号切换记录 ID 不能为空。".into()));
    }
    connection
        .execute(
            "UPDATE activation_history SET status = ?1
             WHERE id = ?2 AND status = 'pending'",
            params![status, id.trim()],
        )
        .map_err(|error| AppError::Internal(format!("更新账号切换记录失败：{error}")))?;
    Ok(())
}

fn refresh_file(
    connection: &mut Connection,
    path: &Path,
    now_utc_ms: i64,
    collection_epoch: i64,
    activations: &[ActivationSnapshot],
    pricing_rules: &[PricingRuleRecord],
    official_catalog: Option<&OfficialPricingCatalog>,
) -> Result<FileRefreshResult, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("无法读取会话文件：{error}"))?;
    if metadata.len() > MAX_ROLLOUT_BYTES {
        return Err("会话文件超过 256 MB，已跳过以避免占用过多内存。".into());
    }
    let Some(discovered) = discover_rollout(path)? else {
        return Ok(FileRefreshResult {
            events_added: 0,
            events_skipped: 0,
            partial_lines: 0,
            warnings: vec![UsageWarning {
                path: Some(path.display().to_string()),
                message: "会话文件缺少有效 session_meta.id，未统计 Token。".into(),
            }],
        });
    };
    let rollout_id = discovered.rollout_id.clone();

    let cursor = load_cursor(connection, &rollout_id)?;
    let file_modified_at_ms = file_modified_at_ms(&metadata);
    let mut rebuild_rollout = discovered.is_subagent
        && discovered.boundary_marker_required
        && cursor
            .as_ref()
            .is_some_and(|cursor| cursor.subagent_boundary_mode == 2);
    let mut prefix_hasher = Sha256::new();
    if !rebuild_rollout {
        rebuild_rollout = match cursor.as_ref() {
            Some(cursor) if cursor.byte_offset <= metadata.len() => {
                prefix_hasher = rollout_prefix_hasher(path, cursor.byte_offset)?;
                match cursor.prefix_sha256.as_deref() {
                    Some(expected) => prefix_hasher.clone().finalize_hex() != expected,
                    None => !committed_prefix_matches(
                        connection,
                        path,
                        &discovered,
                        cursor,
                        collection_epoch,
                    )?,
                }
            }
            Some(_) => true,
            None => false,
        };
    }
    if !rebuild_rollout
        && cursor.as_ref().is_some_and(|cursor| {
            cursor.last_path == path.display().to_string()
                && cursor.file_length == metadata.len()
                && cursor.byte_offset == metadata.len()
                && cursor.file_modified_at_ms == file_modified_at_ms
                && cursor.prefix_sha256.is_some()
        })
    {
        return Ok(FileRefreshResult {
            events_added: 0,
            events_skipped: 0,
            partial_lines: 0,
            warnings: vec![],
        });
    }
    let (offset, mut state) = match cursor {
        Some(cursor) if !rebuild_rollout && cursor.byte_offset <= metadata.len() => (
            cursor.byte_offset,
            ParserState {
                rollout_id: Some(rollout_id.clone()),
                model_provider: cursor.last_model_provider,
                model: cursor.last_model,
                next_event_ordinal: cursor.next_event_ordinal,
                usage_boundary: if discovered.is_subagent {
                    usage_boundary_from_cursor(cursor.usage_boundary, cursor.usage_boundary_passed)
                } else {
                    UsageBoundaryState::Regular
                },
                boundary_marker_required: discovered.boundary_marker_required,
            },
        ),
        _ => {
            prefix_hasher = Sha256::new();
            (0, initial_parser_state(&discovered))
        }
    };

    let file = File::open(path).map_err(|error| format!("无法打开会话文件：{error}"))?;
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(offset))
        .map_err(|error| format!("无法定位会话文件游标：{error}"))?;

    let mut current_offset = offset;
    let mut events = vec![];
    let mut warnings = vec![];
    let mut partial_lines = 0;
    let mut events_skipped = 0;
    let mut inherited_events_skipped = 0;

    loop {
        let mut line_hasher = prefix_hasher.clone();
        let line = read_bounded_line(&mut reader, MAX_RECORD_LINE_BYTES, |chunk| {
            line_hasher.update(chunk);
        })
        .map_err(|error| format!("读取会话文件失败：{error}"))?;
        if line.bytes_read == 0 {
            break;
        }
        if !line.complete {
            partial_lines += 1;
            break;
        }
        prefix_hasher = line_hasher;
        current_offset = current_offset.saturating_add(line.bytes_read as u64);
        let trimmed = line.bytes.strip_suffix(b"\n").unwrap_or(&line.bytes);
        let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
        if trimmed.is_empty() {
            continue;
        }
        if line.truncated || trimmed.len() > MAX_RECORD_LINE_BYTES {
            events_skipped += 1;
            warnings.push(UsageWarning {
                path: Some(path.display().to_string()),
                message: "跳过超过 8 MB 的会话日志行。".into(),
            });
            continue;
        }

        match parse_line(trimmed, &mut state) {
            Ok(LineResult::Event(event)) => events.push(event),
            Ok(LineResult::Ignored) => {}
            Ok(LineResult::SkippedInheritedUsage) => {
                events_skipped += 1;
                inherited_events_skipped += 1;
            }
            Ok(LineResult::Warning(message)) => {
                events_skipped += 1;
                warnings.push(UsageWarning {
                    path: Some(path.display().to_string()),
                    message,
                });
            }
            Err(error) => {
                events_skipped += 1;
                warnings.push(UsageWarning {
                    path: Some(path.display().to_string()),
                    message: error,
                });
            }
        }
    }

    let transaction = connection
        .transaction()
        .map_err(|error| format!("开始保存本机用量事务失败：{error}"))?;
    if rebuild_rollout {
        transaction
            .execute(
                "DELETE FROM usage_events WHERE rollout_id = ?1",
                params![rollout_id],
            )
            .map_err(|error| format!("重建会话用量失败：{error}"))?;
    }
    let mut events_added = 0;
    for event in events {
        if event.occurred_at_ms < collection_epoch {
            events_skipped += 1;
            continue;
        }
        if insert_event(
            &transaction,
            &rollout_id,
            &event,
            now_utc_ms,
            activations,
            pricing_rules,
            official_catalog,
        )? {
            events_added += 1;
        }
    }
    transaction
        .execute(
            "INSERT INTO usage_cursors(
                rollout_id, last_path, byte_offset, next_event_ordinal,
                last_model, last_model_provider, usage_boundary_passed,
                usage_boundary_state, subagent_boundary_mode,
                file_length, file_modified_at_ms, prefix_sha256, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(rollout_id) DO UPDATE SET
               last_path = excluded.last_path,
               byte_offset = excluded.byte_offset,
               next_event_ordinal = excluded.next_event_ordinal,
               last_model = excluded.last_model,
               last_model_provider = excluded.last_model_provider,
               usage_boundary_passed = excluded.usage_boundary_passed,
               usage_boundary_state = excluded.usage_boundary_state,
               subagent_boundary_mode = excluded.subagent_boundary_mode,
               file_length = excluded.file_length,
               file_modified_at_ms = excluded.file_modified_at_ms,
               prefix_sha256 = excluded.prefix_sha256,
               updated_at_ms = excluded.updated_at_ms",
            params![
                rollout_id,
                path.display().to_string(),
                i64::try_from(current_offset).map_err(|_| "会话文件游标超过数据库范围。")?,
                i64::try_from(state.next_event_ordinal)
                    .map_err(|_| "Token 事件序号超过数据库范围。")?,
                state.model,
                state.model_provider,
                i64::from(usage_boundary_passed(state.usage_boundary)),
                usage_boundary_code(state.usage_boundary),
                subagent_boundary_mode(&discovered),
                i64::try_from(metadata.len()).map_err(|_| "会话文件大小超过数据库范围。")?,
                file_modified_at_ms,
                prefix_hasher.finalize_hex(),
                now_utc_ms,
            ],
        )
        .map_err(|error| format!("保存本机用量游标失败：{error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("提交本机用量事务失败：{error}"))?;

    if inherited_events_skipped > 0
        && matches!(
            state.usage_boundary,
            UsageBoundaryState::AwaitingSubagentTaskStart
                | UsageBoundaryState::AwaitingSubagentBoundaryMarker
        )
    {
        warnings.push(UsageWarning {
            path: Some(path.display().to_string()),
            message:
                "子任务日志尚未到达真实任务边界，已暂不统计继承历史 Token；后续刷新会继续处理。"
                    .into(),
        });
    }

    Ok(FileRefreshResult {
        events_added,
        events_skipped,
        partial_lines,
        warnings,
    })
}

fn file_modified_at_ms(metadata: &fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|value| i64::try_from(value.as_millis()).ok())
}

trait Sha256Hex {
    fn finalize_hex(self) -> String;
}

impl Sha256Hex for Sha256 {
    fn finalize_hex(self) -> String {
        format!("{:x}", self.finalize())
    }
}

fn rollout_prefix_hasher(path: &Path, length: u64) -> Result<Sha256, String> {
    let file = File::open(path).map_err(|error| format!("无法打开会话文件：{error}"))?;
    let mut reader = file.take(length);
    let mut hasher = Sha256::new();
    let copied = std::io::copy(&mut reader, &mut hasher)
        .map_err(|error| format!("验证会话文件前缀失败：{error}"))?;
    if copied != length {
        return Err("验证会话文件前缀时文件长度发生变化。".into());
    }
    Ok(hasher)
}

fn initial_parser_state(discovered: &DiscoveredRollout) -> ParserState {
    ParserState {
        rollout_id: Some(discovered.rollout_id.clone()),
        model_provider: discovered.model_provider.clone(),
        model: None,
        next_event_ordinal: 0,
        usage_boundary: if discovered.is_subagent {
            UsageBoundaryState::AwaitingSubagentTaskStart
        } else {
            UsageBoundaryState::Regular
        },
        boundary_marker_required: discovered.boundary_marker_required,
    }
}

fn committed_prefix_matches(
    connection: &Connection,
    path: &Path,
    discovered: &DiscoveredRollout,
    cursor: &StoredCursor,
    collection_epoch: i64,
) -> Result<bool, String> {
    let file = File::open(path).map_err(|error| format!("无法打开会话文件：{error}"))?;
    let mut reader = BufReader::new(file);
    let mut state = initial_parser_state(discovered);
    let mut offset = 0u64;
    let mut expected_events = Vec::new();

    while offset < cursor.byte_offset {
        let line = read_bounded_line(&mut reader, MAX_RECORD_LINE_BYTES, |_| {})
            .map_err(|error| format!("验证会话文件前缀失败：{error}"))?;
        if line.bytes_read == 0
            || !line.complete
            || offset.saturating_add(line.bytes_read as u64) > cursor.byte_offset
        {
            return Ok(false);
        }
        offset = offset.saturating_add(line.bytes_read as u64);
        let trimmed = line.bytes.strip_suffix(b"\n").unwrap_or(&line.bytes);
        let trimmed = trimmed.strip_suffix(b"\r").unwrap_or(trimmed);
        if trimmed.is_empty() || line.truncated || trimmed.len() > MAX_RECORD_LINE_BYTES {
            continue;
        }
        if let Ok(LineResult::Event(event)) = parse_line(trimmed, &mut state)
            && event.occurred_at_ms >= collection_epoch
        {
            expected_events.push((event.ordinal, event_id(&discovered.rollout_id, &event)));
        }
    }

    let expected_boundary = if discovered.is_subagent {
        usage_boundary_from_cursor(cursor.usage_boundary, cursor.usage_boundary_passed)
    } else {
        UsageBoundaryState::Regular
    };
    if state.rollout_id.as_deref() != Some(discovered.rollout_id.as_str())
        || state.model != cursor.last_model
        || state.model_provider != cursor.last_model_provider
        || state.next_event_ordinal != cursor.next_event_ordinal
        || state.usage_boundary != expected_boundary
    {
        return Ok(false);
    }

    let mut statement = connection
        .prepare(
            "SELECT event_ordinal, event_id FROM usage_events
             WHERE rollout_id = ?1 ORDER BY event_ordinal",
        )
        .map_err(|error| format!("读取待验证会话用量失败：{error}"))?;
    let stored_events = statement
        .query_map(params![discovered.rollout_id], |row| {
            Ok((i64_to_u64(row.get(0)?)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("验证会话用量失败：{error}"))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| format!("验证会话用量失败：{error}"))?;
    Ok(expected_events == stored_events)
}

fn rollout_state(path: &Path, boundary_marker_required: bool) -> Result<ParserState, String> {
    let file = File::open(path).map_err(|error| format!("无法打开会话文件：{error}"))?;
    let mut reader = BufReader::new(file);
    let mut state = ParserState {
        usage_boundary: UsageBoundaryState::AwaitingSubagentTaskStart,
        boundary_marker_required,
        ..ParserState::default()
    };
    loop {
        let line = read_bounded_line(&mut reader, MAX_RECORD_LINE_BYTES, |_| {})
            .map_err(|error| format!("读取子任务边界失败：{error}"))?;
        if line.bytes_read == 0 {
            return Ok(state);
        }
        if line.truncated {
            continue;
        }
        let line = line.bytes.strip_suffix(b"\n").unwrap_or(&line.bytes);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.len() > MAX_RECORD_LINE_BYTES {
            continue;
        }
        let _ = parse_line(line, &mut state);
    }
}

fn usage_boundary_code(value: UsageBoundaryState) -> i64 {
    match value {
        UsageBoundaryState::Regular => 0,
        UsageBoundaryState::AwaitingSubagentTaskStart => 1,
        UsageBoundaryState::AwaitingSubagentBoundaryMarker => 2,
        UsageBoundaryState::Started => 3,
    }
}

fn usage_boundary_from_cursor(code: i64, passed: bool) -> UsageBoundaryState {
    match code {
        1 => UsageBoundaryState::AwaitingSubagentTaskStart,
        2 => UsageBoundaryState::AwaitingSubagentBoundaryMarker,
        3 => UsageBoundaryState::Started,
        _ if passed => UsageBoundaryState::Started,
        _ => UsageBoundaryState::AwaitingSubagentTaskStart,
    }
}

fn usage_boundary_passed(value: UsageBoundaryState) -> bool {
    matches!(
        value,
        UsageBoundaryState::Regular | UsageBoundaryState::Started
    )
}

fn subagent_boundary_mode(discovered: &DiscoveredRollout) -> i64 {
    if !discovered.is_subagent {
        0
    } else if discovered.boundary_marker_required {
        1
    } else {
        2
    }
}

fn discover_rollout(path: &Path) -> Result<Option<DiscoveredRollout>, String> {
    let file = File::open(path).map_err(|error| format!("无法打开会话文件：{error}"))?;
    let mut reader = BufReader::new(file);
    let mut lines = Vec::<Vec<u8>>::new();
    let mut scanned = 0usize;
    loop {
        if scanned >= MAX_METADATA_SCAN_BYTES {
            break;
        }
        let line = read_bounded_line(&mut reader, MAX_METADATA_SCAN_BYTES, |_| {})
            .map_err(|error| format!("读取会话元数据失败：{error}"))?;
        if line.bytes_read == 0 {
            break;
        }
        scanned = scanned.saturating_add(line.bytes_read);
        if !line.complete {
            break;
        }
        if !line.truncated {
            lines.push(line.bytes);
        }
    }
    let boundary_marker_required = lines.iter().any(|line| is_inter_agent_trigger_line(line));
    let mut state = ParserState {
        boundary_marker_required,
        ..ParserState::default()
    };
    for line in lines {
        let _ = parse_line(&line, &mut state);
    }
    let Some(rollout_id) = state.rollout_id else {
        return Ok(None);
    };
    let is_subagent = state.usage_boundary != UsageBoundaryState::Regular;
    Ok(Some(DiscoveredRollout {
        rollout_id,
        model_provider: state.model_provider,
        is_subagent,
        boundary_marker_required: is_subagent && boundary_marker_required,
    }))
}

fn load_cursor(connection: &Connection, rollout_id: &str) -> Result<Option<StoredCursor>, String> {
    connection
        .query_row(
            "SELECT last_path, byte_offset, next_event_ordinal, last_model,
                    last_model_provider, usage_boundary_passed, usage_boundary_state,
                    subagent_boundary_mode, file_length, file_modified_at_ms, prefix_sha256
             FROM usage_cursors WHERE rollout_id = ?1",
            params![rollout_id],
            |row| {
                let byte_offset = i64_to_u64(row.get::<_, i64>(1)?)?;
                let next_event_ordinal = i64_to_u64(row.get::<_, i64>(2)?)?;
                Ok(StoredCursor {
                    last_path: row.get(0)?,
                    byte_offset,
                    next_event_ordinal,
                    last_model: row.get(3)?,
                    last_model_provider: row.get(4)?,
                    usage_boundary_passed: row.get::<_, i64>(5)? != 0,
                    usage_boundary: row.get(6)?,
                    subagent_boundary_mode: row.get(7)?,
                    file_length: i64_to_u64(row.get::<_, i64>(8)?)?,
                    file_modified_at_ms: row.get(9)?,
                    prefix_sha256: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(|error| format!("读取本机用量游标失败：{error}"))
}

fn load_activations(connection: &Connection) -> anyhow::Result<Vec<ActivationSnapshot>> {
    let mut statement = connection.prepare(
        "SELECT effective_at_ms, source_kind, provider_id, account_id,
                model_provider, display_name_snapshot
         FROM activation_history
         WHERE status = 'confirmed'
         ORDER BY effective_at_ms ASC, created_at_ms ASC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(ActivationSnapshot {
            effective_at_ms: row.get(0)?,
            source_kind: parse_source_kind(&row.get::<_, String>(1)?),
            provider_id: row.get(2)?,
            account_id: row.get(3)?,
            model_provider: row.get(4)?,
            display_name_snapshot: row.get(5)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn load_pricing_rule_records(connection: &Connection) -> anyhow::Result<Vec<PricingRuleRecord>> {
    let mut statement = connection.prepare(
        "SELECT id, version, active, scope_kind, provider_id, account_id,
                model_pattern, match_kind, billing_mode,
                input_microusd_per_million, cached_read_microusd_per_million,
                cache_write_microusd_per_million, output_microusd_per_million,
                request_fee_microusd, cache_write_included_in_input,
                effective_from_ms
         FROM pricing_rules
         WHERE active = 1",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(PricingRuleRecord {
            id: row.get(0)?,
            version: row.get(1)?,
            active: row.get::<_, i64>(2)? != 0,
            scope_kind: parse_pricing_scope(&row.get::<_, String>(3)?),
            provider_id: row.get(4)?,
            account_id: row.get(5)?,
            model_pattern: row.get(6)?,
            match_kind: parse_pricing_match(&row.get::<_, String>(7)?),
            billing_mode: parse_billing_mode(&row.get::<_, String>(8)?),
            input_microusd_per_million: row.get(9)?,
            cached_read_microusd_per_million: row.get(10)?,
            cache_write_microusd_per_million: row.get(11)?,
            output_microusd_per_million: row.get(12)?,
            request_fee_microusd: row.get(13)?,
            cache_write_included_in_input: row.get::<_, i64>(14)? != 0,
            effective_from_ms: row.get(15)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn load_official_catalog(
    connection: &Connection,
) -> anyhow::Result<Option<OfficialPricingCatalog>> {
    let json = connection
        .query_row(
            "SELECT normalized_json FROM official_pricing_catalogs
             WHERE active = 1 ORDER BY fetched_at_ms DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    json.map(|value| serde_json::from_str(&value).map_err(Into::into))
        .transpose()
}

fn load_pricing_rule_dtos(connection: &Connection) -> Result<Vec<PricingRule>, AppError> {
    let mut statement = connection
        .prepare(
            "SELECT id, version, active, scope_kind, provider_id, account_id,
                    model_pattern, match_kind, billing_mode,
                    input_microusd_per_million, cached_read_microusd_per_million,
                    cache_write_microusd_per_million, output_microusd_per_million,
                    request_fee_microusd, cache_write_included_in_input,
                    effective_from_ms, created_at_ms, updated_at_ms
             FROM pricing_rules ORDER BY updated_at_ms DESC, version DESC",
        )
        .map_err(|error| AppError::Internal(format!("读取美元价格规则失败：{error}")))?;
    let rows = statement
        .query_map([], |row| {
            Ok(PricingRule {
                id: row.get(0)?,
                version: i64_to_u64(row.get(1)?)?,
                active: row.get::<_, i64>(2)? != 0,
                scope_kind: parse_pricing_scope(&row.get::<_, String>(3)?),
                provider_id: row.get(4)?,
                account_id: row.get(5)?,
                model_pattern: row.get(6)?,
                match_kind: parse_pricing_match(&row.get::<_, String>(7)?),
                billing_mode: parse_billing_mode(&row.get::<_, String>(8)?),
                input_usd_per_million: microusd_to_usd(row.get(9)?),
                cached_read_usd_per_million: microusd_to_usd(row.get(10)?),
                cache_write_usd_per_million: microusd_to_usd(row.get(11)?),
                output_usd_per_million: microusd_to_usd(row.get(12)?),
                request_fee_usd: microusd_to_usd(row.get(13)?),
                cache_write_included_in_input: row.get::<_, i64>(14)? != 0,
                effective_from_ms: row.get(15)?,
                created_at_ms: row.get(16)?,
                updated_at_ms: row.get(17)?,
            })
        })
        .map_err(|error| AppError::Internal(format!("读取美元价格规则失败：{error}")))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AppError::Internal(format!("读取美元价格规则失败：{error}")))
}

fn normalize_pricing_input(
    mut input: SavePricingRule,
    now: i64,
) -> Result<(PricingRule, PricingRuleRecord), AppError> {
    if input.billing_mode == BillingMode::Subscription {
        return Err(AppError::InvalidConfig(
            "订阅制价格规则已不再支持，请选择按 Token 计价或不计价。".into(),
        ));
    }
    input.model_pattern = input.model_pattern.trim().to_owned();
    if input.scope_kind == PricingScopeKind::ProviderDefault {
        input.model_pattern = "*".into();
    } else if input.model_pattern.is_empty() {
        return Err(AppError::InvalidConfig("请填写模型匹配规则。".into()));
    }
    match input.scope_kind {
        PricingScopeKind::AccountModel
            if input.account_id.as_deref().unwrap_or_default().is_empty() =>
        {
            return Err(AppError::InvalidConfig(
                "账号级价格规则缺少账号 ID。".into(),
            ));
        }
        PricingScopeKind::ProviderModel | PricingScopeKind::ProviderDefault
            if input.provider_id.as_deref().unwrap_or_default().is_empty() =>
        {
            return Err(AppError::InvalidConfig(
                "服务级价格规则缺少 Provider ID。".into(),
            ));
        }
        _ => {}
    }
    if input.scope_kind == PricingScopeKind::GlobalModel {
        input.provider_id = None;
        input.account_id = None;
    }
    let input_microusd_per_million =
        parse_optional_price(input.input_usd_per_million.as_deref(), "输入")?;
    let cached_read_microusd_per_million =
        parse_optional_price(input.cached_read_usd_per_million.as_deref(), "缓存读取")?;
    let cache_write_microusd_per_million =
        parse_optional_price(input.cache_write_usd_per_million.as_deref(), "缓存写入")?;
    let output_microusd_per_million =
        parse_optional_price(input.output_usd_per_million.as_deref(), "输出")?;
    let request_fee_microusd =
        parse_optional_price(input.request_fee_usd.as_deref(), "请求固定费")?;
    input.input_usd_per_million = microusd_to_usd(input_microusd_per_million);
    input.cached_read_usd_per_million = microusd_to_usd(cached_read_microusd_per_million);
    input.cache_write_usd_per_million = microusd_to_usd(cache_write_microusd_per_million);
    input.output_usd_per_million = microusd_to_usd(output_microusd_per_million);
    input.request_fee_usd = microusd_to_usd(request_fee_microusd);
    input.effective_from_ms = if input.effective_from_ms > 0 {
        input.effective_from_ms
    } else {
        now
    };
    let internal = PricingRuleRecord {
        id: input.id.clone(),
        version: i64::try_from(input.version).unwrap_or_default(),
        active: true,
        scope_kind: input.scope_kind,
        provider_id: input.provider_id.clone(),
        account_id: input.account_id.clone(),
        model_pattern: input.model_pattern.clone(),
        match_kind: input.match_kind,
        billing_mode: input.billing_mode,
        input_microusd_per_million,
        cached_read_microusd_per_million,
        cache_write_microusd_per_million,
        output_microusd_per_million,
        request_fee_microusd,
        cache_write_included_in_input: input.cache_write_included_in_input,
        effective_from_ms: input.effective_from_ms,
    };
    Ok((input, internal))
}

fn parse_optional_price(value: Option<&str>, label: &str) -> Result<Option<i64>, AppError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let parsed = parse_usd_microusd(value)
        .map_err(|error| AppError::InvalidConfig(format!("{label}价格无效：{error}")))?;
    if parsed > 1_000_000_000_000 {
        return Err(AppError::InvalidConfig(format!(
            "{label}价格超过允许上限。"
        )));
    }
    Ok(Some(parsed))
}

fn microusd_to_usd(value: Option<i64>) -> Option<String> {
    let value = value?;
    if value < 0 {
        return None;
    }
    let whole = value / 1_000_000;
    let fraction = value % 1_000_000;
    if fraction == 0 {
        return Some(whole.to_string());
    }
    let fraction = format!("{fraction:06}").trim_end_matches('0').to_owned();
    Some(format!("{whole}.{fraction}"))
}

fn resolve_activation(
    activations: &[ActivationSnapshot],
    event: &ParsedUsageEvent,
) -> ActivationResolution {
    let Some(activation) = activations
        .iter()
        .rfind(|activation| activation.effective_at_ms <= event.occurred_at_ms)
    else {
        return ActivationResolution::Missing;
    };
    // The confirmed activation timeline is the authoritative account source.
    // `model_provider` can be `custom` even when the active account is official
    // (for example, custom model names used through the official session).
    ActivationResolution::Matched(activation.clone())
}

fn source_only_attribution(event: &ParsedUsageEvent) -> AttributionOutcome {
    let Some(source_kind) = classify_model_provider(event.model_provider.as_deref()) else {
        return AttributionOutcome::Unknown;
    };
    match source_kind {
        UsageSourceKind::Official => AttributionOutcome::SourceOnly {
            source_kind,
            display_name: "官方 OpenAI（账号未确认）".into(),
            allows_pricing: true,
        },
        UsageSourceKind::Provider => AttributionOutcome::SourceOnly {
            source_kind,
            display_name: "API 服务（账号未确认）".into(),
            allows_pricing: false,
        },
        UsageSourceKind::Unattributed => AttributionOutcome::Unknown,
    }
}

fn resolve_attribution(
    activations: &[ActivationSnapshot],
    event: &ParsedUsageEvent,
) -> AttributionOutcome {
    match resolve_activation(activations, event) {
        ActivationResolution::Matched(activation) => AttributionOutcome::Confirmed(activation),
        ActivationResolution::Missing => source_only_attribution(event),
    }
}

fn classify_model_provider(value: Option<&str>) -> Option<UsageSourceKind> {
    let value = value?.trim().to_ascii_lowercase();
    match value.as_str() {
        "openai" | "official" | "openai_oauth" | "openai-official" => {
            Some(UsageSourceKind::Official)
        }
        "custom" | "proxy" | "relay" | "openrouter" => Some(UsageSourceKind::Provider),
        _ => None,
    }
}

fn to_internal_usage(event: &ParsedUsageEvent) -> crate::usage_log::TokenUsage {
    event.usage
}

fn parse_pricing_scope(value: &str) -> PricingScopeKind {
    match value {
        "account_model" => PricingScopeKind::AccountModel,
        "provider_model" => PricingScopeKind::ProviderModel,
        "provider_default" => PricingScopeKind::ProviderDefault,
        _ => PricingScopeKind::GlobalModel,
    }
}

fn parse_pricing_match(value: &str) -> PricingMatchKind {
    match value {
        "prefix" => PricingMatchKind::Prefix,
        _ => PricingMatchKind::Exact,
    }
}

fn parse_billing_mode(value: &str) -> BillingMode {
    match value {
        "subscription" => BillingMode::Subscription,
        "unpriced" => BillingMode::Unpriced,
        _ => BillingMode::Token,
    }
}

fn pricing_scope_text(value: PricingScopeKind) -> &'static str {
    match value {
        PricingScopeKind::AccountModel => "account_model",
        PricingScopeKind::ProviderModel => "provider_model",
        PricingScopeKind::GlobalModel => "global_model",
        PricingScopeKind::ProviderDefault => "provider_default",
    }
}

fn pricing_match_text(value: PricingMatchKind) -> &'static str {
    match value {
        PricingMatchKind::Exact => "exact",
        PricingMatchKind::Prefix => "prefix",
    }
}

fn billing_mode_text(value: BillingMode) -> &'static str {
    match value {
        BillingMode::Token => "token",
        BillingMode::Subscription => "subscription",
        BillingMode::Unpriced => "unpriced",
    }
}

fn insert_event(
    transaction: &rusqlite::Transaction<'_>,
    rollout_id: &str,
    event: &ParsedUsageEvent,
    created_at_ms: i64,
    activations: &[ActivationSnapshot],
    pricing_rules: &[PricingRuleRecord],
    official_catalog: Option<&OfficialPricingCatalog>,
) -> Result<bool, String> {
    let event_id = event_id(rollout_id, event);
    let usage_quality = match (event.quality, event.model_provider.is_none()) {
        (crate::usage_log::UsageQuality::Complete, true) => "compatible_fallback",
        (crate::usage_log::UsageQuality::Complete, false) => "complete",
        (crate::usage_log::UsageQuality::Partial, _) => "partial",
        (crate::usage_log::UsageQuality::CompatibleFallback, _) => "compatible_fallback",
    };
    let attribution = resolve_attribution(activations, event);
    let (source_kind, provider_id, account_id, source_name, pricing) = match &attribution {
        AttributionOutcome::Confirmed(value) => {
            let provider_id = value.provider_id.as_deref();
            let account_id = value.account_id.as_deref();
            (
                source_kind_text(value.source_kind),
                provider_id,
                account_id,
                value.display_name_snapshot.as_str(),
                Some(price_for_source(
                    value.source_kind,
                    pricing_rules,
                    official_catalog,
                    &to_internal_usage(event),
                    &PricingContext {
                        model: &event.model,
                        provider_id,
                        account_id,
                        effective_at_ms: event.occurred_at_ms,
                    },
                )),
            )
        }
        AttributionOutcome::SourceOnly {
            source_kind,
            display_name,
            allows_pricing,
        } => (
            source_kind_text(*source_kind),
            None,
            None,
            display_name.as_str(),
            (*allows_pricing).then(|| {
                price_for_source(
                    *source_kind,
                    pricing_rules,
                    official_catalog,
                    &to_internal_usage(event),
                    &PricingContext {
                        model: &event.model,
                        provider_id: None,
                        account_id: None,
                        effective_at_ms: event.occurred_at_ms,
                    },
                )
            }),
        ),
        AttributionOutcome::Unknown => ("unattributed", None, None, "未归属", None),
    };
    let (
        pricing_rule_id,
        pricing_rule_version,
        pricing_rule_name,
        estimated_cost,
        mut cost_status,
    ): (Option<String>, Option<i64>, Option<String>, Option<i64>, &str) = match pricing {
            Some(PricingOutcome::Estimated {
                cost_microusd,
                rule_id,
                version,
            }) => (
                Some(rule_id.clone()),
                Some(version),
                pricing_rule_label(Some(&rule_id), pricing_rules),
                Some(cost_microusd),
                "estimated",
            ),
            Some(PricingOutcome::Subscription { rule_id, version }) => (
                Some(rule_id.clone()),
                Some(version),
                pricing_rule_label(Some(&rule_id), pricing_rules),
                None,
                "subscription",
            ),
            Some(PricingOutcome::Unpriced { rule_id, .. }) => (
                rule_id.clone(),
                rule_id
                    .as_deref()
                    .and_then(|id| pricing_rules.iter().find(|rule| rule.id == id))
                    .map(|rule| rule.version),
                pricing_rule_label(rule_id.as_deref(), pricing_rules),
                None,
                "unpriced",
            ),
            None => (None, None, None, None, "unattributed"),
        };
    if matches!(
        attribution,
        AttributionOutcome::Unknown
            | AttributionOutcome::SourceOnly {
                allows_pricing: false,
                ..
            }
    ) {
        cost_status = "unattributed";
    } else if !matches!(event.quality, crate::usage_log::UsageQuality::Complete) {
        cost_status = "partial";
    }
    let affected = transaction
        .execute(
            "INSERT OR IGNORE INTO usage_events(
                event_id, rollout_id, event_ordinal, occurred_at_ms, model,
                model_provider, source_kind, provider_id, account_id, source_name,
                input_tokens, cached_input_tokens, cache_write_input_tokens,
                output_tokens, reasoning_output_tokens, total_tokens, usage_quality,
                pricing_rule_id, pricing_rule_version, pricing_rule_name,
                estimated_cost_microusd, cost_status, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                       ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                       ?18, ?19, ?20, ?21, ?22, ?23)",
            params![
                event_id,
                rollout_id,
                i64::try_from(event.ordinal).map_err(|_| "Token 事件序号超过数据库范围。")?,
                event.occurred_at_ms,
                event.model,
                event.model_provider,
                source_kind,
                provider_id,
                account_id,
                source_name,
                u64_db(event.usage.input_tokens)?,
                u64_db(event.usage.cached_input_tokens)?,
                u64_db(event.usage.cache_write_input_tokens)?,
                u64_db(event.usage.output_tokens)?,
                u64_db(event.usage.reasoning_output_tokens)?,
                u64_db(event.usage.total_tokens)?,
                usage_quality,
                pricing_rule_id,
                pricing_rule_version,
                pricing_rule_name,
                estimated_cost,
                cost_status,
                created_at_ms,
            ],
        )
        .map_err(|error| format!("保存本机 Token 事件失败：{error}"))?;
    Ok(affected > 0)
}

fn event_id(rollout_id: &str, event: &ParsedUsageEvent) -> String {
    let mut hasher = Sha256::new();
    hasher.update(rollout_id.as_bytes());
    hasher.update(event.ordinal.to_le_bytes());
    hasher.update(event.occurred_at_ms.to_le_bytes());
    hasher.update(event.model.as_bytes());
    hasher.update(event.usage.input_tokens.to_le_bytes());
    hasher.update(event.usage.cached_input_tokens.to_le_bytes());
    hasher.update(event.usage.cache_write_input_tokens.to_le_bytes());
    hasher.update(event.usage.output_tokens.to_le_bytes());
    hasher.update(event.usage.reasoning_output_tokens.to_le_bytes());
    hasher.update(event.usage.total_tokens.to_le_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn pricing_rule_label(id: Option<&str>, rules: &[PricingRuleRecord]) -> Option<String> {
    let id = id?;
    official_pricing_rule_name(id).or_else(|| {
        rules
            .iter()
            .find(|rule| rule.id == id)
            .map(|rule| rule.model_pattern.clone())
    })
}

#[derive(Debug)]
struct DbUsageRow {
    occurred_at_ms: i64,
    model: String,
    source_kind: UsageSourceKind,
    provider_id: Option<String>,
    account_id: Option<String>,
    source_name: String,
    tokens: TokenBreakdown,
    cost_status: CostStatus,
    estimated_cost_microusd: Option<u64>,
    pricing_rule_name: Option<String>,
    pricing_rule_version: Option<u64>,
}

#[derive(Debug)]
struct DbTrendRow {
    occurred_at_ms: i64,
    source_kind: UsageSourceKind,
    tokens: TokenBreakdown,
    cost_status: CostStatus,
    estimated_cost_microusd: Option<u64>,
}

fn read_usage_row(row: &Row<'_>, _group_by: UsageGroupBy) -> rusqlite::Result<DbUsageRow> {
    Ok(DbUsageRow {
        occurred_at_ms: row.get(0)?,
        model: row.get(1)?,
        source_kind: parse_source_kind(&row.get::<_, String>(2)?),
        provider_id: row.get(3)?,
        account_id: row.get(4)?,
        source_name: row.get(5)?,
        tokens: TokenBreakdown {
            input_tokens: i64_to_u64(row.get(6)?)?,
            cached_input_tokens: i64_to_u64(row.get(7)?)?,
            cache_write_input_tokens: i64_to_u64(row.get(8)?)?,
            output_tokens: i64_to_u64(row.get(9)?)?,
            reasoning_output_tokens: i64_to_u64(row.get(10)?)?,
            total_tokens: i64_to_u64(row.get(11)?)?,
        },
        cost_status: parse_cost_status(&row.get::<_, String>(12)?),
        estimated_cost_microusd: row.get::<_, Option<i64>>(13)?.map(i64_to_u64).transpose()?,
        pricing_rule_name: row.get(14)?,
        pricing_rule_version: row.get::<_, Option<i64>>(15)?.map(i64_to_u64).transpose()?,
    })
}

fn aggregate_key(row: &DbUsageRow, group_by: UsageGroupBy) -> String {
    match group_by {
        UsageGroupBy::Model => format!("model:{}", row.model),
        UsageGroupBy::Account => format!(
            "account:{}:{}:{}",
            source_kind_text(row.source_kind),
            row.provider_id.as_deref().unwrap_or_default(),
            row.account_id.as_deref().unwrap_or("unattributed")
        ),
    }
}

fn add_tokens(target: &mut TokenBreakdown, source: &TokenBreakdown) {
    target.input_tokens = target.input_tokens.saturating_add(source.input_tokens);
    target.cached_input_tokens = target
        .cached_input_tokens
        .saturating_add(source.cached_input_tokens);
    target.cache_write_input_tokens = target
        .cache_write_input_tokens
        .saturating_add(source.cache_write_input_tokens);
    target.output_tokens = target.output_tokens.saturating_add(source.output_tokens);
    target.reasoning_output_tokens = target
        .reasoning_output_tokens
        .saturating_add(source.reasoning_output_tokens);
    target.total_tokens = target.total_tokens.saturating_add(source.total_tokens);
}

/// 把时间戳归到本机自然日的起始时刻（毫秒）。
/// 把单条用量事件累加到按本机自然日分桶的趋势点里（totals 与 trend
/// 共用同一套聚合规则，避免两趟查询产生不同口径）。
fn accumulate_day_point(
    points: &mut BTreeMap<i64, UsageTrendPoint>,
    occurred_at_ms: i64,
    tokens: &TokenBreakdown,
    cost_status: CostStatus,
    estimated_cost_microusd: Option<u64>,
    source_kind: UsageSourceKind,
) {
    let Some(day_start) = local_day_start_ms(occurred_at_ms) else {
        return;
    };
    let point = match points.entry(day_start) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => entry.insert(UsageTrendPoint {
            day_start_ms: day_start,
            tokens: TokenBreakdown::default(),
            requests: 0,
            estimated_cost_microusd: 0,
            unpriced_tokens: 0,
            partial_tokens: 0,
            unattributed_tokens: 0,
        }),
    };
    add_tokens(&mut point.tokens, tokens);
    point.requests = point.requests.saturating_add(1);
    if cost_status == CostStatus::Estimated
        || (cost_status == CostStatus::Partial && estimated_cost_microusd.is_some())
    {
        point.estimated_cost_microusd = point
            .estimated_cost_microusd
            .saturating_add(estimated_cost_microusd.unwrap_or(0));
    }
    if cost_status == CostStatus::Unpriced {
        point.unpriced_tokens = point.unpriced_tokens.saturating_add(tokens.total_tokens);
    }
    if cost_status == CostStatus::Partial {
        point.partial_tokens = point.partial_tokens.saturating_add(tokens.total_tokens);
    }
    if cost_status == CostStatus::Unattributed || source_kind == UsageSourceKind::Unattributed {
        point.unattributed_tokens = point
            .unattributed_tokens
            .saturating_add(tokens.total_tokens);
    }
}

fn local_day_start_ms(occurred_at_ms: i64) -> Option<i64> {
    let local = Local.timestamp_millis_opt(occurred_at_ms).single()?;
    let midnight = local
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("本机日期的午夜时刻应有效");
    Some(
        Local
            .from_local_datetime(&midnight)
            .single()
            .or_else(|| Local.from_local_datetime(&midnight).earliest())
            .map(|value| value.timestamp_millis())
            .unwrap_or_else(|| {
                // 极少数时区在午夜发生 DST 跳变时兜底：直接按当地日期构造。
                midnight.and_utc().timestamp_millis()
            }),
    )
}

fn aggregate_cost_status(aggregate: &UsageAggregate) -> CostStatus {
    if aggregate.requests == 0 {
        CostStatus::Zero
    } else if aggregate.has_unattributed {
        CostStatus::Unattributed
    } else if aggregate.has_partial {
        CostStatus::Partial
    } else if aggregate.has_unpriced {
        CostStatus::Unpriced
    } else if aggregate.has_estimated {
        CostStatus::Estimated
    } else if aggregate.has_subscription {
        CostStatus::Subscription
    } else {
        CostStatus::Unpriced
    }
}

fn parse_source_kind(value: &str) -> UsageSourceKind {
    match value {
        "official" => UsageSourceKind::Official,
        "provider" => UsageSourceKind::Provider,
        _ => UsageSourceKind::Unattributed,
    }
}

fn source_kind_text(value: UsageSourceKind) -> &'static str {
    match value {
        UsageSourceKind::Official => "official",
        UsageSourceKind::Provider => "provider",
        UsageSourceKind::Unattributed => "unattributed",
    }
}

fn parse_cost_status(value: &str) -> CostStatus {
    match value {
        "estimated" => CostStatus::Estimated,
        "subscription" => CostStatus::Subscription,
        "unpriced" => CostStatus::Unpriced,
        "partial" => CostStatus::Partial,
        "zero" => CostStatus::Zero,
        _ => CostStatus::Unattributed,
    }
}

fn i64_to_u64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(std::fmt::Error),
        )
    })
}

fn u64_db(value: u64) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| "Token 数超过 SQLite 整数范围。".into())
}

#[cfg(test)]
mod tests {
    use super::{ActivationSnapshot, UsageLedger, file_modified_at_ms, read_bounded_line};
    use crate::models::{
        ActivationRecord, AppError, BillingMode, CostStatus, PricingMatchKind, PricingScopeKind,
        SavePricingRule, UsageGroupBy, UsageQuery, UsageRange, UsageSourceKind,
    };
    use crate::official_pricing::build_catalog;
    use crate::usage_log::{ParsedUsageEvent, TokenUsage, UsageQuality};
    use chrono::DateTime;
    use std::{fs, io::Cursor, path::Path};

    #[test]
    fn bounded_line_drains_oversized_records_without_retaining_them() {
        let mut reader = Cursor::new(b"abcdef\nnext\n");
        let mut consumed = Vec::new();
        let oversized =
            read_bounded_line(&mut reader, 4, |chunk| consumed.extend_from_slice(chunk)).unwrap();

        assert_eq!(oversized.bytes_read, 7);
        assert!(oversized.complete);
        assert!(oversized.truncated);
        assert_eq!(oversized.bytes.len(), 6);
        assert_eq!(consumed, b"abcdef\n");

        let next = read_bounded_line(&mut reader, 4, |_| {}).unwrap();
        assert_eq!(next.bytes, b"next\n");
        assert!(!next.truncated);
    }

    fn rollout_prefix() -> &'static str {
        concat!(
            r#"{"type":"session_meta","payload":{"id":"rollout-ledger","model_provider":"openai"}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            "\n",
        )
    }

    fn rollout_event() -> &'static str {
        concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-01T10:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"cache_write_input_tokens":3,"output_tokens":8,"reasoning_output_tokens":2,"total_tokens":108}}}}"#,
            "\n",
        )
    }

    fn rollout_event_later() -> &'static str {
        concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-01T10:00:02Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":20,"cache_write_input_tokens":3,"output_tokens":8,"reasoning_output_tokens":2,"total_tokens":108}}}}"#,
            "\n",
        )
    }

    fn provider_switch_event() -> &'static str {
        concat!(
            r#"{"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"model":"gpt-5.6-luna","model_provider_id":"custom"}}}"#,
            "\n",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-02T04:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":11,"output_tokens":4,"total_tokens":15}}}}"#,
            "\n",
        )
    }

    fn subagent_metadata() -> &'static str {
        r#"{"type":"session_meta","payload":{"id":"subagent-ledger","model_provider":"openai","source":{"subagent":{"thread_spawn":{"parent_thread_id":"parent-ledger"}}}}}"#
    }

    fn subagent_prefix_tail() -> &'static str {
        concat!(
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-01T20:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":10,"total_tokens":110}}}}"#,
            "\n",
        )
    }

    fn subagent_started_event() -> &'static str {
        concat!(
            r#"{"timestamp":"2026-08-01T20:00:00Z","type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"model":"gpt-5.6-luna","model_provider_id":"custom"}}}"#,
            "\n",
            r#"{"timestamp":"2026-08-01T20:00:00Z","type":"event_msg","payload":{"type":"task_started"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-01T20:00:00Z","type":"turn_context","payload":{"model":"gpt-5.6-luna"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-01T20:00:00Z","type":"inter_agent_communication_metadata","payload":{"trigger_turn":true}}"#,
            "\n",
            r#"{"timestamp":"2026-08-01T20:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":11,"output_tokens":4,"total_tokens":15}}}}"#,
            "\n",
        )
    }

    fn subagent_no_marker_event() -> &'static str {
        concat!(
            r#"{"timestamp":"2026-08-01T20:00:00Z","type":"event_msg","payload":{"type":"task_started"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-01T20:00:00Z","type":"turn_context","payload":{"model":"gpt-5.6-luna","model_provider":"custom"}}"#,
            "\n",
            r#"{"timestamp":"2026-08-01T20:00:01Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":11,"output_tokens":4,"total_tokens":15}}}}"#,
            "\n",
        )
    }

    fn append_new_usage(home: &Path) {
        use std::io::Write;

        let path = home.join("sessions/2026/08/rollout.jsonl");
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(rollout_event().as_bytes()).unwrap();
    }

    fn write_rollout(home: &Path, text: &str) -> std::path::PathBuf {
        let path = home.join("sessions/2026/08/rollout.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    fn save_test_catalog(ledger: &UsageLedger) {
        let catalog = build_catalog(
            "# Pricing\n\n### Standard pricing data\n\n| Model | Short context input | Short context cached input | Short context cache writes | Short context output |\n| --- | --- | --- | --- | --- |\n| gpt-5.6-sol | $1 | $1 | $1 | $1 |",
            20260801,
            None,
            None,
        )
        .unwrap();
        ledger
            .save_official_pricing_catalog(&catalog, 20260801)
            .unwrap();
    }

    #[test]
    fn reactivating_official_catalog_keeps_exactly_one_active() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        let catalog_a = build_catalog(
            "# Pricing\n\n### Standard pricing data\n\n| Model | Short context input | Short context output |\n| --- | --- | --- |\n| model-a | $1 | $2 |",
            1,
            None,
            None,
        )
        .unwrap();
        let catalog_b = build_catalog(
            "# Pricing\n\n### Standard pricing data\n\n| Model | Short context input | Short context output |\n| --- | --- | --- |\n| model-b | $2 | $3 |",
            2,
            None,
            None,
        )
        .unwrap();

        assert!(ledger.save_official_pricing_catalog(&catalog_a, 1).unwrap());
        assert!(ledger.save_official_pricing_catalog(&catalog_b, 2).unwrap());
        assert!(!ledger.save_official_pricing_catalog(&catalog_a, 3).unwrap());

        let connection = ledger.open_connection().unwrap();
        let active: Vec<i64> = connection
            .prepare("SELECT version FROM official_pricing_catalogs WHERE active = 1")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(active, vec![catalog_a.version]);
    }

    #[test]
    fn migrates_v5_subscription_rules_to_unpriced_without_readding_cursor_column() {
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE usage_cursors (
                   rollout_id TEXT PRIMARY KEY,
                   subagent_boundary_mode INTEGER NOT NULL DEFAULT 0
                     CHECK (subagent_boundary_mode IN (0, 1, 2))
                 );
                 CREATE TABLE pricing_rules (
                   id TEXT PRIMARY KEY,
                   version INTEGER NOT NULL,
                   active INTEGER NOT NULL,
                   scope_kind TEXT NOT NULL CHECK (scope_kind IN ('account_model', 'provider_model', 'global_model', 'provider_default')),
                   provider_id TEXT,
                   account_id TEXT,
                   model_pattern TEXT NOT NULL,
                   match_kind TEXT NOT NULL CHECK (match_kind IN ('exact', 'prefix')),
                   billing_mode TEXT NOT NULL CHECK (billing_mode IN ('token', 'subscription', 'unpriced')),
                   input_microusd_per_million INTEGER,
                   cached_read_microusd_per_million INTEGER,
                   cache_write_microusd_per_million INTEGER,
                   output_microusd_per_million INTEGER,
                   request_fee_microusd INTEGER,
                   cache_write_included_in_input INTEGER NOT NULL,
                   effective_from_ms INTEGER NOT NULL,
                   created_at_ms INTEGER NOT NULL,
                   updated_at_ms INTEGER NOT NULL
                 );
                 INSERT INTO pricing_rules VALUES (
                   'legacy-subscription', 1, 1, 'global_model', NULL, NULL,
                   'gpt-subscription', 'exact', 'subscription',
                   NULL, NULL, NULL, NULL, NULL, 1, 0, 1, 1
                 );
                 INSERT INTO pricing_rules VALUES (
                   'legacy-token', 1, 1, 'global_model', NULL, NULL,
                   'gpt-token', 'exact', 'token',
                   1000000, NULL, NULL, 2000000, NULL, 1, 0, 1, 1
                 );
                 PRAGMA user_version = 5;",
            )
            .unwrap();

        super::initialize_schema(&connection).unwrap();

        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7);
        let migrated_mode: String = connection
            .query_row(
                "SELECT billing_mode FROM pricing_rules WHERE id = 'legacy-subscription'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migrated_mode, "unpriced");
        let token_mode: String = connection
            .query_row(
                "SELECT billing_mode FROM pricing_rules WHERE id = 'legacy-token'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(token_mode, "token");
        let cursor_column_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('usage_cursors')
                 WHERE name = 'subagent_boundary_mode'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cursor_column_count, 1);
        let prefix_hash_column_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('usage_cursors')
                 WHERE name = 'prefix_sha256'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(prefix_hash_column_count, 1);
        assert!(
            connection
                .execute(
                    "UPDATE pricing_rules SET billing_mode = 'subscription'
                     WHERE id = 'legacy-subscription'",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn rejects_saving_subscription_pricing_rules() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();

        let error = ledger
            .usage_save_pricing_rule(SavePricingRule {
                id: String::new(),
                version: 0,
                active: true,
                scope_kind: PricingScopeKind::GlobalModel,
                provider_id: None,
                account_id: None,
                model_pattern: "gpt-subscription".into(),
                match_kind: PricingMatchKind::Exact,
                billing_mode: BillingMode::Subscription,
                input_usd_per_million: None,
                cached_read_usd_per_million: None,
                cache_write_usd_per_million: None,
                output_usd_per_million: None,
                request_fee_usd: None,
                cache_write_included_in_input: true,
                effective_from_ms: 0,
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap_err();

        match error {
            AppError::InvalidConfig(message) => {
                assert!(message.contains("订阅制"));
                assert!(message.contains("Token"));
            }
            other => panic!("expected invalid configuration error, got {other:?}"),
        }
        assert!(ledger.usage_list_pricing_rules(None).unwrap().is_empty());
        let connection = ledger.open_connection().unwrap();
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 7);
    }

    #[test]
    fn failed_pricing_rule_insert_does_not_deactivate_old_rule() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        let input = SavePricingRule {
            id: String::new(),
            version: 0,
            active: true,
            scope_kind: PricingScopeKind::GlobalModel,
            provider_id: None,
            account_id: None,
            model_pattern: "gpt-test".into(),
            match_kind: PricingMatchKind::Exact,
            billing_mode: BillingMode::Token,
            input_usd_per_million: Some("1".into()),
            cached_read_usd_per_million: None,
            cache_write_usd_per_million: None,
            output_usd_per_million: Some("2".into()),
            request_fee_usd: None,
            cache_write_included_in_input: true,
            effective_from_ms: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
        };
        let saved = ledger.usage_save_pricing_rule(input.clone()).unwrap();
        let connection = ledger.open_connection().unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_pricing_insert BEFORE INSERT ON pricing_rules
                 BEGIN SELECT RAISE(ABORT, 'forced insert failure'); END;",
            )
            .unwrap();
        drop(connection);

        let mut replacement = input;
        replacement.id = saved.id.clone();
        assert!(ledger.usage_save_pricing_rule(replacement).is_err());

        let connection = ledger.open_connection().unwrap();
        let active: i64 = connection
            .query_row(
                "SELECT active FROM pricing_rules WHERE id = ?1",
                rusqlite::params![saved.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 1);
    }

    #[test]
    fn classifies_known_log_providers_without_an_activation_record() {
        assert_eq!(
            super::classify_model_provider(Some("openai")),
            Some(UsageSourceKind::Official)
        );
        assert_eq!(
            super::classify_model_provider(Some("custom")),
            Some(UsageSourceKind::Provider)
        );
        assert_eq!(super::classify_model_provider(Some("unknown")), None);
    }

    #[test]
    fn confirmed_activation_wins_over_conflicting_log_provider() {
        let activation = ActivationSnapshot {
            effective_at_ms: 100,
            source_kind: UsageSourceKind::Provider,
            provider_id: Some("provider-1".into()),
            account_id: Some("account-1".into()),
            model_provider: Some("custom".into()),
            display_name_snapshot: "API 服务 · Key".into(),
        };
        let event = ParsedUsageEvent {
            ordinal: 0,
            occurred_at_ms: 200,
            model: "gpt-5.6-luna".into(),
            model_provider: Some("openai".into()),
            usage: TokenUsage {
                input_tokens: 1,
                cached_input_tokens: 0,
                cache_write_input_tokens: 0,
                output_tokens: 1,
                reasoning_output_tokens: 0,
                total_tokens: 2,
            },
            quality: UsageQuality::Complete,
        };

        assert!(matches!(
            super::resolve_attribution(&[activation], &event),
            super::AttributionOutcome::Confirmed(ActivationSnapshot {
                source_kind: UsageSourceKind::Provider,
                provider_id: Some(_),
                account_id: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn official_activation_keeps_official_pricing_when_log_provider_is_custom() {
        let activation = ActivationSnapshot {
            effective_at_ms: 100,
            source_kind: UsageSourceKind::Official,
            provider_id: None,
            account_id: Some("official-account".into()),
            model_provider: Some("openai".into()),
            display_name_snapshot: "官方账号".into(),
        };
        let event = ParsedUsageEvent {
            ordinal: 0,
            occurred_at_ms: 200,
            model: "gpt-5.6-luna".into(),
            model_provider: Some("custom".into()),
            usage: TokenUsage {
                input_tokens: 1,
                cached_input_tokens: 0,
                cache_write_input_tokens: 0,
                output_tokens: 1,
                reasoning_output_tokens: 0,
                total_tokens: 2,
            },
            quality: UsageQuality::Complete,
        };

        assert!(matches!(
            super::resolve_attribution(&[activation], &event),
            super::AttributionOutcome::Confirmed(ActivationSnapshot {
                source_kind: UsageSourceKind::Official,
                account_id: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn ignores_usage_that_exists_before_the_collection_epoch() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let old_log = format!("{}{}", rollout_prefix(), rollout_event());
        write_rollout(&home, &old_log);
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        save_test_catalog(&ledger);

        let refreshed = ledger.refresh(&home, 1_754_121_000_000).unwrap();
        let overview = query(&ledger);

        assert_eq!(refreshed.events_added, 0);
        assert_eq!(overview.totals.requests, 0);
        assert!(overview.rows.is_empty());
        assert_eq!(overview.collection_started_at_ms, Some(1_754_121_000_000));
    }

    fn query(ledger: &UsageLedger) -> crate::models::UsageOverview {
        ledger
            .query(UsageQuery {
                range: UsageRange {
                    start_at_ms: 1_785_542_400_000,
                    end_at_ms: 1_785_628_800_000,
                },
                group_by: UsageGroupBy::Model,
            })
            .unwrap()
    }

    #[test]
    fn reprice_prices_historical_provider_events_from_the_range_start() {
        // 事件发生在规则创建之前；规则从重算范围起点生效（前端“保存后重算当前范围”的行为），
        // 重算后历史事件也必须按新价格估算。
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        let collection_epoch = DateTime::parse_from_rfc3339("2026-08-02T03:00:00Z")
            .unwrap()
            .timestamp_millis();
        let activation_at = DateTime::parse_from_rfc3339("2026-08-02T03:59:00Z")
            .unwrap()
            .timestamp_millis();
        let event_at = DateTime::parse_from_rfc3339("2026-08-02T04:00:01Z")
            .unwrap()
            .timestamp_millis();

        ledger.refresh(&home, collection_epoch).unwrap();
        ledger
            .record_activation(ActivationRecord {
                effective_at_ms: activation_at,
                source_kind: UsageSourceKind::Provider,
                provider_id: Some("provider-1".into()),
                account_id: None,
                model_provider: Some("custom".into()),
                display_name_snapshot: "API 服务".into(),
                auth_source: Some("api_key".into()),
            })
            .unwrap();
        {
            use std::io::Write;

            let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
            file.write_all(provider_switch_event().as_bytes()).unwrap();
        }
        ledger.refresh(&home, event_at).unwrap();

        // 规则 effective_from = 范围起点（collection_epoch），早于事件时间。
        ledger
            .usage_save_pricing_rule(SavePricingRule {
                id: String::new(),
                version: 0,
                active: true,
                scope_kind: PricingScopeKind::ProviderModel,
                provider_id: Some("provider-1".into()),
                account_id: None,
                model_pattern: "gpt-5.6-luna".into(),
                match_kind: PricingMatchKind::Exact,
                billing_mode: BillingMode::Token,
                input_usd_per_million: Some("2".into()),
                cached_read_usd_per_million: None,
                cache_write_usd_per_million: None,
                output_usd_per_million: Some("8".into()),
                request_fee_usd: None,
                cache_write_included_in_input: true,
                effective_from_ms: collection_epoch,
                created_at_ms: 0,
                updated_at_ms: 0,
            })
            .unwrap();
        assert!(
            collection_epoch < event_at,
            "rule effective_from must precede event"
        );

        let repriced = ledger
            .reprice(UsageRange {
                start_at_ms: collection_epoch,
                end_at_ms: event_at + 1,
            })
            .unwrap();
        assert!(repriced.events_repriced >= 1);

        let overview = ledger
            .query(UsageQuery {
                range: UsageRange {
                    start_at_ms: collection_epoch,
                    end_at_ms: event_at + 1,
                },
                group_by: UsageGroupBy::Model,
            })
            .unwrap();
        let row = overview
            .rows
            .iter()
            .find(|row| row.model == "gpt-5.6-luna")
            .expect("应找到 gpt-5.6-luna 用量行");
        // 事件发生在规则创建之前，重算后必须按新价格估算（partial 表示
        // 日志字段不全但费用已按规则计算），而不是停留在“未配置价格”。
        assert_ne!(row.cost_status, CostStatus::Unpriced);
        assert!(row.estimated_cost_microusd.unwrap_or(0) > 0);
        assert_eq!(row.pricing_rule_name.as_deref(), Some("gpt-5.6-luna"));
    }

    #[test]
    fn refreshes_and_queries_local_usage_through_the_ledger_interface() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        save_test_catalog(&ledger);

        assert_eq!(
            ledger
                .refresh(&home, 1_754_121_000_000)
                .unwrap()
                .events_added,
            0
        );
        append_new_usage(&home);
        let refreshed = ledger.refresh(&home, 1_754_121_001_000).unwrap();
        let overview = query(&ledger);

        assert_eq!(refreshed.files_scanned, 1);
        assert_eq!(
            refreshed.events_added, 1,
            "warnings: {:?}",
            refreshed.warnings
        );
        assert_eq!(overview.totals.requests, 1);
        assert_eq!(overview.totals.tokens.total_tokens, 108);
        assert_eq!(overview.totals.tokens.cached_input_tokens, 20);
        assert_eq!(overview.rows.len(), 1);
        assert_eq!(overview.rows[0].model, "gpt-5.6-sol");
        assert_eq!(overview.rows[0].source_kind, UsageSourceKind::Official);
        assert_eq!(overview.rows[0].source_name, "官方 OpenAI（账号未确认）");
        assert_eq!(
            overview.rows[0].pricing_rule_name.as_deref(),
            Some("OpenAI 官方参考价")
        );
    }

    #[test]
    fn repeated_refresh_does_not_duplicate_events() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();

        ledger.refresh(&home, 1_754_121_000_000).unwrap();
        append_new_usage(&home);
        let refreshed = ledger.refresh(&home, 1_754_121_001_000).unwrap();
        assert_eq!(
            refreshed.events_added, 1,
            "warnings: {:?}",
            refreshed.warnings
        );
        assert_eq!(
            ledger
                .refresh(&home, 1_754_121_001_000)
                .unwrap()
                .events_added,
            0
        );
        assert_eq!(query(&ledger).totals.requests, 1);
    }

    #[test]
    fn canonical_account_aliases_merge_historical_local_account_ids() {
        use std::io::Write;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        let collection_epoch = DateTime::parse_from_rfc3339("2026-08-01T09:00:00Z")
            .unwrap()
            .timestamp_millis();
        let first_activation = collection_epoch + 1_000;
        let second_activation = DateTime::parse_from_rfc3339("2026-08-01T10:00:01.500Z")
            .unwrap()
            .timestamp_millis();

        ledger
            .record_activation(ActivationRecord {
                effective_at_ms: first_activation,
                source_kind: UsageSourceKind::Official,
                provider_id: None,
                account_id: Some("local-account-a".into()),
                model_provider: Some("openai".into()),
                display_name_snapshot: "账号 A".into(),
                auth_source: Some("openai_oauth".into()),
            })
            .unwrap();
        ledger.refresh(&home, collection_epoch).unwrap();
        {
            let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(rollout_event().as_bytes()).unwrap();
        }
        ledger.refresh(&home, collection_epoch + 3_000).unwrap();

        ledger
            .record_activation(ActivationRecord {
                effective_at_ms: second_activation,
                source_kind: UsageSourceKind::Official,
                provider_id: None,
                account_id: Some("local-account-b".into()),
                model_provider: Some("openai".into()),
                display_name_snapshot: "账号 B".into(),
                auth_source: Some("openai_oauth".into()),
            })
            .unwrap();
        {
            let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(rollout_event_later().as_bytes()).unwrap();
        }
        ledger.refresh(&home, collection_epoch + 4_000).unwrap();

        let before = ledger
            .query(UsageQuery {
                range: UsageRange {
                    start_at_ms: collection_epoch,
                    end_at_ms: collection_epoch + 10_000_000,
                },
                group_by: UsageGroupBy::Account,
            })
            .unwrap();
        assert_eq!(before.rows.len(), 2);

        ledger
            .sync_official_account_identities(&[
                ("local-account-a".into(), "external-account-x".into()),
                ("local-account-b".into(), "external-account-x".into()),
            ])
            .unwrap();
        let after = ledger
            .query(UsageQuery {
                range: UsageRange {
                    start_at_ms: collection_epoch,
                    end_at_ms: collection_epoch + 10_000_000,
                },
                group_by: UsageGroupBy::Account,
            })
            .unwrap();
        assert_eq!(after.rows.len(), 1);
        assert_eq!(after.rows[0].requests, 2);
        assert_eq!(after.rows[0].tokens.total_tokens, 216);
    }

    #[test]
    fn model_grouping_merges_the_same_model_across_accounts() {
        use std::io::Write;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        let collection_epoch = DateTime::parse_from_rfc3339("2026-08-01T09:00:00Z")
            .unwrap()
            .timestamp_millis();

        ledger
            .record_activation(ActivationRecord {
                effective_at_ms: collection_epoch + 1_000,
                source_kind: UsageSourceKind::Official,
                provider_id: None,
                account_id: Some("account-a".into()),
                model_provider: Some("openai".into()),
                display_name_snapshot: "账号 A".into(),
                auth_source: Some("openai_oauth".into()),
            })
            .unwrap();
        ledger.refresh(&home, collection_epoch).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(rollout_event().as_bytes())
            .unwrap();
        ledger.refresh(&home, collection_epoch + 3_000).unwrap();

        ledger
            .record_activation(ActivationRecord {
                effective_at_ms: collection_epoch + 3_500,
                source_kind: UsageSourceKind::Official,
                provider_id: None,
                account_id: Some("account-b".into()),
                model_provider: Some("openai".into()),
                display_name_snapshot: "账号 B".into(),
                auth_source: Some("openai_oauth".into()),
            })
            .unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(rollout_event_later().as_bytes())
            .unwrap();
        ledger.refresh(&home, collection_epoch + 4_000).unwrap();

        let overview = ledger
            .query(UsageQuery {
                range: UsageRange {
                    start_at_ms: collection_epoch,
                    end_at_ms: collection_epoch + 10_000_000,
                },
                group_by: UsageGroupBy::Model,
            })
            .unwrap();

        assert_eq!(overview.rows.len(), 1);
        assert_eq!(overview.rows[0].model, "gpt-5.6-sol");
        assert_eq!(overview.rows[0].source_name, "多个账号/来源");
        assert_eq!(overview.rows[0].requests, 2);
        assert_eq!(overview.rows[0].tokens.total_tokens, 216);
    }

    #[test]
    fn subagent_inherited_history_is_not_counted() {
        use std::io::Write;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, &format!("{}\n", subagent_metadata()));
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        let collection_epoch = DateTime::parse_from_rfc3339("2026-08-01T19:00:00Z")
            .unwrap()
            .timestamp_millis();
        ledger.refresh(&home, collection_epoch).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(format!("{}{}", subagent_prefix_tail(), subagent_started_event()).as_bytes())
            .unwrap();

        let refreshed = ledger.refresh(&home, collection_epoch + 1_000).unwrap();
        let overview = query(&ledger);

        assert_eq!(
            refreshed.events_added, 1,
            "warnings: {:?}",
            refreshed.warnings
        );
        assert_eq!(overview.totals.requests, 1);
        assert_eq!(overview.totals.tokens.total_tokens, 15);
        assert_eq!(overview.rows.len(), 1);
        assert_eq!(overview.rows[0].source_kind, UsageSourceKind::Provider);
        assert_eq!(overview.rows[0].source_name, "API 服务（账号未确认）");
        assert_eq!(overview.rows[0].estimated_cost_microusd, None);

        let connection = ledger.open_connection().unwrap();
        let stored_boundary: i64 = connection
            .query_row(
                "SELECT usage_boundary_passed FROM usage_cursors WHERE rollout_id = 'subagent-ledger'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_boundary, 1);
        assert!(path.exists());

        connection
            .execute(
                "UPDATE usage_metadata SET value = '3' WHERE key = 'usage_parser_version'",
                [],
            )
            .unwrap();
        let rebuilt = ledger.refresh(&home, collection_epoch + 2_000).unwrap();
        assert_eq!(rebuilt.events_added, 1, "warnings: {:?}", rebuilt.warnings);
        assert_eq!(query(&ledger).totals.requests, 1);
        assert_eq!(query(&ledger).totals.tokens.total_tokens, 15);
    }

    #[test]
    fn subagent_boundary_survives_incremental_refresh() {
        use std::io::Write;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, &format!("{}\n", subagent_metadata()));
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        let collection_epoch = DateTime::parse_from_rfc3339("2026-08-01T19:00:00Z")
            .unwrap()
            .timestamp_millis();

        let first = ledger.refresh(&home, collection_epoch).unwrap();
        assert_eq!(first.events_added, 0);
        assert!(first.warnings.is_empty());

        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(subagent_prefix_tail().as_bytes())
            .unwrap();

        let second = ledger.refresh(&home, collection_epoch + 1_000).unwrap();
        assert_eq!(second.events_added, 0);
        assert!(
            second
                .warnings
                .iter()
                .any(|warning| warning.message.contains("真实任务边界"))
        );

        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(subagent_started_event().as_bytes())
            .unwrap();

        let third = ledger.refresh(&home, collection_epoch + 2_000).unwrap();
        assert_eq!(third.events_added, 1, "warnings: {:?}", third.warnings);
        assert_eq!(query(&ledger).totals.tokens.total_tokens, 15);
    }

    #[test]
    fn no_marker_subagent_is_counted_by_incremental_refresh() {
        use std::io::Write;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, &format!("{}\n", subagent_metadata()));
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        let collection_epoch = DateTime::parse_from_rfc3339("2026-08-01T19:00:00Z")
            .unwrap()
            .timestamp_millis();

        ledger.refresh(&home, collection_epoch).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(
                format!("{}{}", subagent_prefix_tail(), subagent_no_marker_event()).as_bytes(),
            )
            .unwrap();

        let refreshed = ledger.refresh(&home, collection_epoch + 1_000).unwrap();
        let overview = query(&ledger);

        assert_eq!(
            refreshed.events_added, 1,
            "warnings: {:?}",
            refreshed.warnings
        );
        assert_eq!(overview.totals.requests, 1);
        assert_eq!(overview.totals.tokens.total_tokens, 15);
        assert_eq!(overview.rows[0].model, "gpt-5.6-luna");
        assert_eq!(
            ledger
                .refresh(&home, collection_epoch + 2_000)
                .unwrap()
                .events_added,
            0
        );
    }

    #[test]
    fn parser_upgrade_rebuilds_current_cycle_with_provider_context() {
        use chrono::DateTime;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, rollout_prefix());
        let app = temp.path().join("app");
        let ledger = UsageLedger::open(&app).unwrap();
        let collection_epoch = DateTime::parse_from_rfc3339("2026-08-02T03:00:00Z")
            .unwrap()
            .timestamp_millis();
        let activation_at = DateTime::parse_from_rfc3339("2026-08-02T03:59:00Z")
            .unwrap()
            .timestamp_millis();
        let event_at = DateTime::parse_from_rfc3339("2026-08-02T04:00:01Z")
            .unwrap()
            .timestamp_millis();

        ledger.refresh(&home, collection_epoch).unwrap();
        ledger
            .record_activation(ActivationRecord {
                effective_at_ms: activation_at,
                source_kind: UsageSourceKind::Provider,
                provider_id: Some("provider-1".into()),
                account_id: Some("account-1".into()),
                model_provider: Some("custom".into()),
                display_name_snapshot: "API 服务 · Key".into(),
                auth_source: Some("api_key".into()),
            })
            .unwrap();
        {
            use std::io::Write;

            let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
            file.write_all(provider_switch_event().as_bytes()).unwrap();
        }
        ledger.refresh(&home, event_at).unwrap();

        let connection = ledger.open_connection().unwrap();
        connection
            .execute(
                "UPDATE usage_events
                 SET model_provider = 'openai', source_kind = 'official',
                     provider_id = NULL, account_id = NULL,
                     source_name = '官方 OpenAI（账号未确认）'
                 WHERE model = 'gpt-5.6-luna'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE usage_cursors SET last_model_provider = 'openai'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "DELETE FROM usage_metadata WHERE key = 'usage_parser_version'",
                [],
            )
            .unwrap();

        ledger.refresh(&home, event_at + 1_000).unwrap();
        let overview = ledger
            .query(UsageQuery {
                range: UsageRange {
                    start_at_ms: collection_epoch,
                    end_at_ms: event_at + 1_000,
                },
                group_by: UsageGroupBy::Model,
            })
            .unwrap();

        assert_eq!(overview.rows.len(), 1);
        assert_eq!(overview.rows[0].source_kind, UsageSourceKind::Provider);
        assert_eq!(overview.rows[0].provider_id.as_deref(), Some("provider-1"));
        assert_eq!(overview.rows[0].account_id.as_deref(), Some("account-1"));
    }

    #[test]
    fn moving_a_rollout_to_archived_sessions_does_not_duplicate_events() {
        use std::io::Write;

        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();

        ledger.refresh(&home, 1_754_121_000_000).unwrap();
        append_new_usage(&home);
        let refreshed = ledger.refresh(&home, 1_754_121_001_000).unwrap();
        assert_eq!(
            refreshed.events_added, 1,
            "warnings: {:?}",
            refreshed.warnings
        );
        let archived = home.join("archived_sessions/rollout.jsonl");
        fs::create_dir_all(archived.parent().unwrap()).unwrap();
        fs::rename(path, &archived).unwrap();

        assert_eq!(
            ledger
                .refresh(&home, 1_754_121_002_000)
                .unwrap()
                .events_added,
            0
        );
        assert_eq!(query(&ledger).totals.requests, 1);

        fs::OpenOptions::new()
            .append(true)
            .open(&archived)
            .unwrap()
            .write_all(rollout_event_later().as_bytes())
            .unwrap();
        assert_eq!(
            ledger
                .refresh(&home, 1_754_121_003_000)
                .unwrap()
                .events_added,
            1
        );
        assert_eq!(query(&ledger).totals.requests, 2);
    }

    fn assert_rollout_replacement(old_event: &str, replacement: &str, expected_total: u64) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();

        ledger.refresh(&home, 1_754_121_000_000).unwrap();
        fs::write(&path, format!("{}{}", rollout_prefix(), old_event)).unwrap();
        ledger.refresh(&home, 1_785_624_001_000).unwrap();
        assert_eq!(query(&ledger).totals.requests, 1);

        fs::write(&path, format!("{}{}", rollout_prefix(), replacement)).unwrap();
        let refreshed = ledger.refresh(&home, 1_785_624_002_000).unwrap();
        let overview = query(&ledger);

        assert_eq!(
            refreshed.events_added, 1,
            "warnings: {:?}",
            refreshed.warnings
        );
        assert_eq!(overview.totals.requests, 1);
        assert_eq!(overview.totals.tokens.total_tokens, expected_total);
    }

    #[test]
    fn same_length_rollout_replacement_rebuilds_events() {
        let replacement = rollout_event().replace("108", "109");
        assert_eq!(replacement.len(), rollout_event().len());
        assert_rollout_replacement(rollout_event(), &replacement, 109);
    }

    #[test]
    fn same_length_same_mtime_replacement_rebuilds_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, rollout_prefix());
        let app = temp.path().join("app");
        let ledger = UsageLedger::open(&app).unwrap();
        ledger.refresh(&home, 1_754_121_000_000).unwrap();
        fs::write(&path, format!("{}{}", rollout_prefix(), rollout_event())).unwrap();
        ledger.refresh(&home, 1_785_624_001_000).unwrap();
        assert_eq!(query(&ledger).totals.tokens.total_tokens, 108);

        let original_metadata = fs::metadata(&path).unwrap();
        let original_modified = original_metadata.modified().unwrap();
        let replacement = rollout_event().replace("108", "109");
        fs::write(&path, format!("{}{}", rollout_prefix(), replacement)).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(original_modified))
            .unwrap();
        let replacement_metadata = fs::metadata(&path).unwrap();
        assert_eq!(replacement_metadata.len(), original_metadata.len());
        assert_eq!(
            file_modified_at_ms(&replacement_metadata),
            file_modified_at_ms(&original_metadata)
        );

        drop(ledger);
        let reopened = UsageLedger::open(&app).unwrap();
        let refreshed = reopened.refresh(&home, 1_785_624_002_000).unwrap();

        assert_eq!(
            refreshed.events_added, 1,
            "warnings: {:?}",
            refreshed.warnings
        );
        let overview = query(&reopened);
        assert_eq!(overview.totals.requests, 1);
        assert_eq!(overview.totals.tokens.total_tokens, 109);
    }

    #[test]
    fn longer_rollout_replacement_rebuilds_events() {
        let replacement = rollout_event().replace("108", "1008");
        assert!(replacement.len() > rollout_event().len());
        assert_rollout_replacement(rollout_event(), &replacement, 1008);
    }

    #[test]
    fn truncated_rollout_replacement_removes_old_events() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        let path = write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();

        ledger.refresh(&home, 1_754_121_000_000).unwrap();
        fs::write(&path, format!("{}{}", rollout_prefix(), rollout_event())).unwrap();
        ledger.refresh(&home, 1_785_624_001_000).unwrap();
        assert_eq!(query(&ledger).totals.requests, 1);

        fs::write(&path, rollout_prefix()).unwrap();
        let refreshed = ledger.refresh(&home, 1_785_624_002_000).unwrap();

        assert_eq!(
            refreshed.events_added, 0,
            "warnings: {:?}",
            refreshed.warnings
        );
        assert_eq!(query(&ledger).totals.requests, 0);
    }

    #[test]
    fn activation_lifecycle_keeps_pending_rows_out_of_attribution() {
        let temp = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        let activation = ActivationRecord {
            effective_at_ms: 1_785_542_400_000,
            source_kind: UsageSourceKind::Provider,
            provider_id: Some("provider-1".into()),
            account_id: Some("account-1".into()),
            model_provider: Some("custom".into()),
            display_name_snapshot: "API 服务 · Key".into(),
            auth_source: Some("api_key".into()),
        };

        let pending_id = ledger.begin_activation(&activation).unwrap();
        let connection = ledger.open_connection().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM activation_history WHERE id = ?1",
                    [&pending_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "pending"
        );
        ledger.cancel_activation(&pending_id).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM activation_history WHERE id = ?1",
                    [&pending_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "cancelled"
        );

        let confirmed_id = ledger.begin_activation(&activation).unwrap();
        ledger.confirm_activation(&confirmed_id).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT status FROM activation_history WHERE id = ?1",
                    [&confirmed_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "confirmed"
        );
    }

    #[cfg(unix)]
    #[test]
    fn usage_database_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app");
        UsageLedger::open(&root).unwrap();
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.join("usage.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn attributes_official_usage_and_uses_the_runtime_price_catalog() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        save_test_catalog(&ledger);
        ledger.refresh(&home, 1_754_121_000_000).unwrap();
        append_new_usage(&home);
        ledger
            .record_activation(ActivationRecord {
                effective_at_ms: 1_785_542_400_000,
                source_kind: UsageSourceKind::Official,
                provider_id: None,
                account_id: Some("official-account".into()),
                model_provider: Some("openai".into()),
                display_name_snapshot: "工作账号".into(),
                auth_source: Some("openai_oauth".into()),
            })
            .unwrap();
        ledger.refresh(&home, 1_785_624_000_000).unwrap();
        let overview = query(&ledger);
        assert_eq!(overview.rows[0].source_kind, UsageSourceKind::Official);
        assert_eq!(overview.rows[0].source_name, "工作账号");
        assert_eq!(
            overview.rows[0].cost_status,
            crate::models::CostStatus::Estimated
        );
        assert_eq!(overview.rows[0].estimated_cost_microusd, Some(108));
        assert_eq!(
            overview.rows[0].pricing_rule_name.as_deref(),
            Some("OpenAI 官方参考价")
        );
        assert_eq!(overview.rows[0].pricing_rule_version, Some(20260801));
    }

    fn token_event_full(timestamp: &str, input: u64, output: u64, total: u64) -> String {
        let line = format!(
            "{}\n",
            serde_json::to_string(&serde_json::json!({
                "timestamp": timestamp,
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "last_token_usage": {
                            "input_tokens": input,
                            "cached_input_tokens": 0,
                            "cache_write_input_tokens": 0,
                            "output_tokens": output,
                            "reasoning_output_tokens": 0,
                            "total_tokens": total,
                        }
                    }
                }
            }))
            .expect("测试事件 JSON 序列化应成功")
        );
        format!(
            "{}\n{}",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#, line
        )
    }

    fn token_event_partial(timestamp: &str, input: u64, output: u64, total: u64) -> String {
        let line = format!(
            "{}\n",
            serde_json::to_string(&serde_json::json!({
                "timestamp": timestamp,
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "last_token_usage": {
                            "input_tokens": input,
                            "output_tokens": output,
                            "total_tokens": total,
                        }
                    }
                }
            }))
            .expect("测试事件 JSON 序列化应成功")
        );
        format!(
            "{}\n{}",
            r#"{"type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#, line
        )
    }

    fn append_events(home: &Path, lines: &[String]) {
        use std::io::Write;

        let path = home.join("sessions/2026/08/rollout.jsonl");
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        for line in lines {
            file.write_all(line.as_bytes()).unwrap();
        }
    }

    fn trend(ledger: &UsageLedger, start_at_ms: i64, end_at_ms: i64) -> crate::models::UsageTrend {
        ledger
            .trend(UsageRange {
                start_at_ms,
                end_at_ms,
            })
            .unwrap()
    }

    #[test]
    fn trend_groups_events_by_local_day_in_ascending_order() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        // 首次刷新建立统计周期与游标，不读取已有事件。
        ledger.refresh(&home, 1_785_542_400_000).unwrap();
        append_events(
            &home,
            &[
                token_event_full("2026-08-01T00:30:00Z", 100, 8, 108),
                token_event_full("2026-08-03T12:00:00Z", 11, 4, 15),
            ],
        );
        ledger.refresh(&home, 1_785_758_400_000).unwrap();
        let result = trend(&ledger, 1_785_542_400_000, 1_785_801_600_000);

        assert_eq!(result.range.start_at_ms, 1_785_542_400_000);
        assert_eq!(result.range.end_at_ms, 1_785_801_600_000);
        assert_eq!(result.points.len(), 2);
        // 按日升序；桶边界必须是本机自然日起点。
        assert!(
            result
                .points
                .windows(2)
                .all(|window| window[0].day_start_ms < window[1].day_start_ms)
        );
        for point in &result.points {
            assert_eq!(
                super::local_day_start_ms(point.day_start_ms),
                Some(point.day_start_ms)
            );
        }
        // 跨日总和与全部事件一致。
        let total_tokens: u64 = result
            .points
            .iter()
            .map(|point| point.tokens.total_tokens)
            .sum();
        let total_requests: u64 = result.points.iter().map(|point| point.requests).sum();
        assert_eq!(total_tokens, 108 + 15);
        assert_eq!(total_requests, 2);
        // 每个事件归属到正确的一天（两个事件相隔超过 2 天，任何时区都落在不同本地日）。
        let day1 = super::local_day_start_ms(1_785_544_200_000).unwrap();
        let day2 = super::local_day_start_ms(1_785_758_400_000).unwrap();
        assert_ne!(day1, day2);
        let first = result
            .points
            .iter()
            .find(|point| point.day_start_ms == day1)
            .expect("应找到第一天的点");
        assert_eq!(first.requests, 1);
        assert_eq!(first.tokens.total_tokens, 108);
        let second = result
            .points
            .iter()
            .find(|point| point.day_start_ms == day2)
            .expect("应找到第二天的点");
        assert_eq!(second.tokens.total_tokens, 15);
    }

    #[test]
    fn out_of_range_timestamp_does_not_create_a_trend_point() {
        assert_eq!(super::local_day_start_ms(i64::MAX), None);
    }

    #[test]
    fn trend_respects_collection_epoch_and_range_bounds() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        // 首次刷新前已存在的事件不进入统计周期。
        let pre_existing = format!("{}{}", rollout_prefix(), rollout_event());
        write_rollout(&home, &pre_existing);
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        ledger.refresh(&home, 1_785_542_400_000).unwrap();
        append_events(
            &home,
            &[token_event_full("2026-08-03T12:00:00Z", 11, 4, 15)],
        );
        ledger.refresh(&home, 1_785_758_400_000).unwrap();

        // 只查询 8/2 一天：既排除周期前的旧事件，也排除范围外的新事件。
        let result = trend(&ledger, 1_785_628_800_000, 1_785_715_200_000);
        assert!(result.points.is_empty());

        // 查询 8/3：只包含新事件。
        let result = trend(&ledger, 1_785_715_200_000, 1_785_801_600_000);
        assert_eq!(result.points.len(), 1);
        assert_eq!(result.points[0].requests, 1);
        assert_eq!(result.points[0].tokens.total_tokens, 15);
    }

    #[test]
    fn trend_tracks_cost_and_attention_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("codex");
        write_rollout(&home, rollout_prefix());
        let ledger = UsageLedger::open(&temp.path().join("app")).unwrap();
        save_test_catalog(&ledger);
        ledger.refresh(&home, 1_785_542_400_000).unwrap();
        // 第一天：完整字段 + 官方参考价 → 有估算、无未定价。
        ledger
            .record_activation(ActivationRecord {
                effective_at_ms: 1_785_542_400_000,
                source_kind: UsageSourceKind::Official,
                provider_id: None,
                account_id: Some("official-account".into()),
                model_provider: Some("openai".into()),
                display_name_snapshot: "工作账号".into(),
                auth_source: Some("openai_oauth".into()),
            })
            .unwrap();
        append_events(
            &home,
            &[
                token_event_full("2026-08-01T00:30:00Z", 100, 8, 108),
                token_event_partial("2026-08-03T12:00:00Z", 11, 4, 15),
            ],
        );
        ledger.refresh(&home, 1_785_758_400_000).unwrap();

        let result = trend(&ledger, 1_785_542_400_000, 1_785_801_600_000);
        assert_eq!(result.points.len(), 2);
        let estimated = result
            .points
            .iter()
            .find(|point| point.tokens.total_tokens == 108)
            .expect("完整事件的一天");
        assert!(estimated.estimated_cost_microusd > 0);
        assert_eq!(estimated.unpriced_tokens, 0);
        assert_eq!(estimated.partial_tokens, 0);
        let partial = result
            .points
            .iter()
            .find(|point| point.tokens.total_tokens == 15)
            .expect("字段不全事件的一天");
        assert_eq!(partial.partial_tokens, 15);
    }
}
