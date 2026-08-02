# 本机 Token 用量与美元费用统计实施计划

状态：已实现，待用户手动测试
基线：Codex Tools `v0.3.4`，提交 `abc2fbf`
计价货币：仅美元（USD）
目标执行方式：严格按阶段执行；任何阶段的退出条件未满足，不得进入下一阶段。

当前落地范围：后端日志解析、增量 SQLite 账本、账号激活时间线、USD 价格规则、Tauri IPC、用量与费用页、概览摘要、账号页今日用量摘要均已完成；软件更新后创建新的统计周期，更新前 Token 不回放、不入账、不展示；中转站价格采用服务+模型快速设置并默认重算当前范围；账号切换包含 pending/confirmed/cancelled 生命周期；数据库目录和文件权限已收紧。尚未提交 Git，等待用户运行 `npm run dev` 手动验证真实 Codex 日志。

解析器维护补充：子任务 rollout 的 `session_meta.payload.source.subagent.thread_spawn` 表示文件前部可能包含父任务继承历史；继承段的 `token_count` 必须跳过，遇到 `event_msg.payload.type = task_started` 后才开始统计。解析器升级会重建当前统计周期，保留统计起点、激活历史和价格规则。无法绑定具体账号但能从日志确认 Provider 时，只保存官方/中转站来源，不猜账号；中转站账号未确认事件不参与自定义价格计算。

手动测试前的 UI 示例截图：`output/playwright/token-cost-v034-ui.png`。示例使用脱敏模拟数据，不代表真实账单。

## 1. 目标

在不 Hook Codex、不代理请求、不修改 Codex 可执行文件的前提下，统计当前电脑上 Codex 产生的 Token 用量，并在 Codex Tools 中展示：

- 今天从本机 00:00 到当前时间的累计用量；
- 官方 OAuth、Cookie/反代导入账号、第三方 Responses API 的分别用量；
- 实际使用的模型；
- 总输入、缓存读取、缓存写入、输出、推理输出和总 Token；
- 按用户配置的美元单价计算的估算费用；
- 套餐、未定价、数据不完整和未归属状态。

本功能只统计本机 Codex 会话日志。不得宣称它等同于服务商账单或账号所有设备的总用量。

## 2. 非目标

首版不得实现以下内容：

- 不注入、Hook、Patch 或修改 Codex；
- 不启动 HTTP/HTTPS 中间人代理；
- 不抓包；
- 不上传本机使用记录；
- 不自动查询或换算外汇汇率；
- 不把官方 5H/7D 限额百分比换算成 Token；
- 不自动抓取服务商价格；
- 不把估算费用显示为“实际扣费”；
- 不统计绕过 Codex 直接调用 API 的请求；
- 不在首版实现 CSV 导出、云同步、预算告警或账单对账。

## 3. 已确认的项目约束

- 前端：React、TypeScript、Vite、Tailwind CSS 4、现有 shadcn/Base UI 组件。
- 后端：Tauri 2、Rust 1.85。
- 本地数据库：项目已经依赖 bundled `rusqlite`，不得再引入另一套数据库。
- 账号与 Provider 配置继续保存在现有 `app.yaml`。
- 用量事件和价格规则保存在独立 `usage.sqlite3`，不得把大量事件塞入 `app.yaml`。
- 前端必须复用现有 `PageHeader`、`Card`、`ItemGroup`、`Table`、`Dialog`、`Sheet`、`Badge`、`Button` 和反馈工具。
- 不得为了该功能更换技术栈或重写现有页面。

## 4. 产品口径

### 4.1 “今天”

- 使用操作系统当前本地时区。
- 时间范围为本地当天 `00:00:00`（包含）到当前时刻（包含）。
- 数据库存 UTC 毫秒时间戳；查询时根据本地时区计算范围。
- UI 必须显示当前日期、时区来源和最后刷新时间。
- 跨午夜后下一次刷新必须自动进入新的一天，不允许继续累加到昨天。

### 4.2 Token 字段

日志中的原始字段必须原样保存：

- `input_tokens`
- `cached_input_tokens`
- `cache_write_input_tokens`
- `output_tokens`
- `reasoning_output_tokens`
- `total_tokens`

显示和计费规则：

- `input_tokens` 显示为“总输入”。
- `cached_input_tokens` 显示为“缓存读取”。它通常是总输入的子集。
- `cache_write_input_tokens` 显示为“缓存写入”。是否已包含在总输入中由价格规则控制。
- `reasoning_output_tokens` 是输出明细，不得在总输出之外再次加入总 Token。
- “总 Token”优先使用日志的 `total_tokens`；字段缺失时才使用安全的兼容计算，并给记录标记兼容来源。

### 4.3 费用状态

每个聚合结果必须且只能处于以下状态之一：

- `estimated`：全部用量成功匹配价格，显示 `$0.000000` 到适当精度。
- `subscription`：套餐账号，只显示“套餐用量”。
- `unpriced`：有 Token，但没有匹配价格。
- `partial`：部分事件没有标准 usage 或只有部分事件匹配价格，显示“可能偏低”。
- `unattributed`：无法可靠归属账号。
- `zero`：该时间范围内没有 Token。

所有金额旁必须出现“估算”或对应状态，不得只显示裸金额。

## 5. 总体模块设计

该功能必须实现为一个深模块 `UsageLedger`。页面和 Tauri command 只能调用其小型接口，不得自行读 JSONL、执行 SQL 或计算价格。

### 5.1 外部接口

`src-tauri/src/local_usage.rs` 暴露以下接口，除此之外的解析器、SQL 和匹配函数保持私有：

```rust
#[derive(Clone)]
pub struct UsageLedger { /* 私有字段 */ }

impl UsageLedger {
    pub fn open(app_data_root: &Path) -> anyhow::Result<Self>;

    pub fn refresh(
        &self,
        codex_home: &Path,
        now_utc_ms: i64,
    ) -> Result<UsageRefreshResult, AppError>;

    pub fn query(
        &self,
        query: UsageQuery,
    ) -> Result<UsageOverview, AppError>;

    pub fn record_activation(
        &self,
        activation: ActivationRecord,
    ) -> Result<(), AppError>;

    pub fn list_pricing_rules(
        &self,
        scope: Option<PricingScope>,
    ) -> Result<Vec<PricingRule>, AppError>;

    pub fn save_pricing_rule(
        &self,
        input: SavePricingRule,
    ) -> Result<PricingRule, AppError>;

    pub fn delete_pricing_rule(&self, id: &str) -> Result<(), AppError>;

    pub fn reprice(
        &self,
        range: UsageRange,
    ) -> Result<RepriceResult, AppError>;
}
```

接口约束：

- `refresh` 必须幂等；相同日志重复执行时新增事件数为 0。
- `query` 只读数据库，不扫描文件。
- `record_activation` 使用 UTC 毫秒时间戳。
- `save_pricing_rule` 必须校验所有价格为非负有限小数。
- `delete_pricing_rule` 只停用规则，不删除历史快照。
- `reprice` 只能修改指定时间范围内的估算结果。
- 调用者不得依赖 SQLite 表结构、文件游标或日志记录顺序。

### 5.2 内部模块

- `local_usage.rs`：`UsageLedger`、SQLite、扫描、查询、归属和事务。
- `usage_log.rs`：纯日志解析；输入字节/行，输出规范化事件，不接触数据库。
- `pricing.rs`：纯价格匹配和计算；不接触文件系统和 UI。
- `models.rs`：跨 Tauri seam 的 DTO 和枚举。

不得新增只有一两行转发逻辑的 `repository`、`service` 或 `manager` 文件。SQLite 是本地可替代依赖，测试直接使用临时 SQLite 文件，不额外定义无实际第二个 Adapter 的 trait。

## 6. SQLite 设计

数据库路径：`Store::root()/usage.sqlite3`。

打开数据库后必须执行：

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
```

所有 schema 变更使用 `PRAGMA user_version` 顺序迁移。迁移必须处于事务中。

### 6.1 `usage_events`

```sql
CREATE TABLE usage_events (
  event_id TEXT PRIMARY KEY,
  rollout_id TEXT NOT NULL,
  event_ordinal INTEGER NOT NULL,
  occurred_at_ms INTEGER NOT NULL,
  model TEXT NOT NULL,
  model_provider TEXT,
  source_kind TEXT NOT NULL,
  provider_id TEXT,
  account_id TEXT,
  input_tokens INTEGER NOT NULL CHECK (input_tokens >= 0),
  cached_input_tokens INTEGER NOT NULL CHECK (cached_input_tokens >= 0),
  cache_write_input_tokens INTEGER NOT NULL CHECK (cache_write_input_tokens >= 0),
  output_tokens INTEGER NOT NULL CHECK (output_tokens >= 0),
  reasoning_output_tokens INTEGER NOT NULL CHECK (reasoning_output_tokens >= 0),
  total_tokens INTEGER NOT NULL CHECK (total_tokens >= 0),
  usage_quality TEXT NOT NULL,
  pricing_rule_id TEXT,
  pricing_rule_version INTEGER,
  estimated_cost_microusd INTEGER,
  cost_status TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  UNIQUE (rollout_id, event_ordinal)
);
```

要求：

- `source_kind` 只允许 `official`、`provider`、`unattributed`。
- `usage_quality` 只允许 `complete`、`partial`、`compatible_fallback`。
- `estimated_cost_microusd` 使用整数微美元，`1 USD = 1_000_000 microusd`。
- 不允许用 `f64` 保存最终金额。
- `event_id` 使用稳定摘要，至少包含 `rollout_id`、`event_ordinal` 和规范化 token 字段。

索引：

```sql
CREATE INDEX usage_events_time_idx ON usage_events(occurred_at_ms);
CREATE INDEX usage_events_account_idx ON usage_events(account_id, occurred_at_ms);
CREATE INDEX usage_events_provider_idx ON usage_events(provider_id, occurred_at_ms);
CREATE INDEX usage_events_model_idx ON usage_events(model, occurred_at_ms);
```

### 6.2 `usage_cursors`

```sql
CREATE TABLE usage_cursors (
  rollout_id TEXT PRIMARY KEY,
  last_path TEXT NOT NULL,
  byte_offset INTEGER NOT NULL,
  next_event_ordinal INTEGER NOT NULL,
  last_model TEXT,
  last_model_provider TEXT,
  file_length INTEGER NOT NULL,
  file_modified_at_ms INTEGER,
  updated_at_ms INTEGER NOT NULL
);
```

游标必须按 `rollout_id`，不得只按文件路径。这样文件从 `sessions` 移入 `archived_sessions` 后不会重新计数。

### 6.3 `activation_history`

```sql
CREATE TABLE activation_history (
  id TEXT PRIMARY KEY,
  effective_at_ms INTEGER NOT NULL,
  source_kind TEXT NOT NULL,
  provider_id TEXT,
  account_id TEXT,
  model_provider TEXT,
  display_name_snapshot TEXT NOT NULL,
  auth_source TEXT,
  status TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL
);

CREATE INDEX activation_history_time_idx
  ON activation_history(effective_at_ms);
```

`status` 只允许 `pending`、`confirmed`、`cancelled`。

### 6.4 `pricing_rules`

```sql
CREATE TABLE pricing_rules (
  id TEXT PRIMARY KEY,
  version INTEGER NOT NULL,
  active INTEGER NOT NULL,
  scope_kind TEXT NOT NULL,
  provider_id TEXT,
  account_id TEXT,
  model_pattern TEXT NOT NULL,
  match_kind TEXT NOT NULL,
  billing_mode TEXT NOT NULL,
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
```

允许值：

- `scope_kind`：`account_model`、`provider_model`、`global_model`、`provider_default`。
- `match_kind`：`exact`、`prefix`。
- `billing_mode`：`token`、`subscription`、`unpriced`。

更新规则时插入新版本并停用旧版本，不原地覆盖历史版本。

### 6.5 首版不建立日汇总表

首版直接在带时间索引的 `usage_events` 上聚合。不要建立 `usage_daily`，避免事件表和日汇总形成两套可能不一致的事实。只有实际性能测试证明按日查询无法满足要求后，才能另行设计可重建缓存。

## 7. 日志发现与解析算法

### 7.1 文件发现

复用 `provider_sync::rollout_files(codex_home)`，不得另写不同的目录遍历规则。

扫描目录：

- `<CODEX_HOME>/sessions`
- `<CODEX_HOME>/archived_sessions`

只处理 `.jsonl`。必须保留现有文件大小保护思路，避免超大或恶意文件导致内存占用失控。

### 7.2 Rollout 身份

从 `session_meta.payload.id` 取得 `rollout_id`。如果文件中没有有效 ID：

- 不写入 Token 事件；
- 在 `UsageRefreshResult.warnings` 中返回文件名和原因；
- 不用文件名伪造永久 rollout ID。

### 7.3 增量读取

每个文件依次执行：

1. 读取并解析 `session_meta`，取得 `rollout_id` 和 `model_provider`。
2. 根据 `rollout_id` 查询游标。
3. 如果文件只是换路径但长度不小于游标位置，从原 offset 继续。
4. 如果文件长度小于 offset，认为文件被截断；从头重扫，依靠唯一键去重。
5. 从 offset 开始按行读取，不把整个文件一次性载入内存。
6. 最后一行没有换行或 JSON 不完整时，不推进该行的 offset。
7. 每遇到有效 `turn_context`，更新当前 `model`。
8. 每遇到 `thread_settings_applied`，更新 `thread_settings.model` 和 `thread_settings.model_provider_id`。
9. 每遇到有效 `token_count`，只读取 `info.last_token_usage`。
10. 写入事件和更新游标必须在同一 SQLite 事务中。
11. 单个文件失败不得阻断其他文件；错误进入 warnings。

游标必须保存 `last_model` 和 `last_model_provider`，否则从文件中间继续时会丢失上下文。

### 7.4 模型解析

模型优先级：

1. 最近一条有效 `turn_context.payload.model`；
2. 当前事件直接携带的模型字段（如果未来格式存在）；
3. `unknown`。

不得把 Provider 名称当作模型名称。

### 7.5 Token 解析

所有 JSON 数字必须：

- 接受非负整数；
- 缺失字段按 0，但记录 `usage_quality`；
- 拒绝负数、浮点数和超过 `i64` 的数；
- 使用饱和检查或显式溢出错误，禁止静默溢出。

`total_token_usage` 是会话累计值，严禁写入为单次事件。只允许使用 `last_token_usage`。

## 8. 账号归属算法

### 8.1 激活记录时机

所有账号切换继续使用现有 `ActivationLock`。在以下成功路径加入记录：

- `activate_openai_account`
- `activate_provider`
- `activate_official`
- 设备登录完成并立即激活的路径
- 应用启动时当前配置的对账路径

严禁在 React 页面根据点击动作记录归属；只有 Rust 确认切换成功后才能确认记录。

### 8.2 切换顺序

每次切换严格按以下顺序：

1. 持有 `ActivationLock`。
2. 在 `activation_history` 写入 `pending`，保存目标和当前 UTC 时间。
3. 执行现有 Codex 配置、凭据和 `Store` 激活流程。
4. 成功后把 pending 改为 `confirmed`。
5. 失败后把 pending 改为 `cancelled`，继续返回原切换错误。
6. 如果第 4 步失败，不回滚已经成功的账号切换；返回明确警告并在下次启动时对账修复。

启动对账：

- 获取 `Store` 当前激活目标；
- 获取最后一条 confirmed 记录；
- 两者不一致时，从“当前启动时刻”追加 confirmed 记录；
- 不得把它回填到更早的历史时间。

### 8.3 事件归属

对每个 Token 事件：

1. 找到 `effective_at_ms <= occurred_at_ms` 的最后一条 confirmed 激活记录。
2. 比对日志的 `model_provider` 与激活记录。
3. 一致时写入对应账号。
4. 日志无 Provider 时允许按时间线归属，但标记 `compatible_fallback`。
5. 两者明确冲突时设为 `unattributed`，不得猜测。
6. 事件早于最早 confirmed 记录时设为 `unattributed`。

解析器升级时只删除并重建 `usage_collection_started_at_ms` 之后的事件和游标；保留统计起点、激活历史和价格规则，确保修复日志解析后不会回放旧 Token。

Cookie/反代导入账号在归属上属于 `official`，但 `auth_source` 保留 `proxy_import`，UI 显示“Cookie / 反代”。

## 9. 美元计费算法

### 9.1 输入校验

前端传入十进制美元字符串，例如 `2.50`。Rust 必须自行解析为整数微美元，禁止前端先转 `number` 后传浮点值。

规则：

- 空字符串表示该项未配置；
- 最多 6 位小数；
- 不能为负数；
- 不能是 `NaN`、`Infinity` 或科学计数法；
- 单项价格设置合理上限并返回中文错误，例如每 1M Token 不超过 `$1,000,000`；
- 保存后返回规范化字符串。

### 9.2 匹配优先级

从高到低：

1. `account_model` + exact
2. `account_model` + prefix，最长前缀优先
3. `provider_model` + exact
4. `provider_model` + prefix，最长前缀优先
5. `global_model` + exact
6. `global_model` + prefix，最长前缀优先
7. `provider_default`
8. 无规则

同级冲突时：`effective_from_ms` 较新者优先，再按 `version` 较大者优先。必须为该排序写独立测试。

`provider_default` 的 `model_pattern` 固定保存为 `*`；此时 `match_kind` 不参与匹配。其他 scope 的模型文本去除首尾空格后不得为空。

### 9.3 费用公式

设：

- `I` = `input_tokens`
- `R` = `cached_input_tokens`
- `W` = `cache_write_input_tokens`
- `O` = `output_tokens`
- `N` = 请求次数，单个事件为 1

如果 `cache_write_included_in_input = true`：

```text
normal_input = max(I - R - W, 0)
```

否则：

```text
normal_input = max(I - R, 0)
```

费用使用整数并带余数安全计算：

```text
cost_microusd =
  normal_input * input_microusd_per_million / 1_000_000
  + R * cached_read_microusd_per_million / 1_000_000
  + W * cache_write_microusd_per_million / 1_000_000
  + O * output_microusd_per_million / 1_000_000
  + N * request_fee_microusd
```

中间乘法使用 `i128`，最终检查后转 `i64`。只有 Token 数大于 0 的计费桶才要求对应价格；例如缓存写入为 0 时可以没有缓存写入单价。任何非零计费桶缺少价格都使事件成为 `unpriced`，不得把缺失价格当 0 后显示“估算 $0”。

### 9.4 统计周期与价格

- 事件保存匹配的 `pricing_rule_id` 和 `pricing_rule_version`。
- 修改规则只产生新版本。
- 首次升级到该版本时保存 `usage_collection_started_at_ms`，并将已知 rollout 游标置于文件尾部；更新前事件不回放、不入账、不展示。
- 新发现的 rollout 也只写入统计周期起点之后的事件；所有查询和重算均再次按统计周期过滤。
- 保存中转站快速价格后默认只重算当前日期范围；不提供历史数据入口。
- 删除规则是停用；历史版本仍可用于解释过去金额。

## 10. Rust DTO

在 `models.rs` 中增加以下类型。Rust 枚举统一使用 `#[serde(rename_all = "snake_case")]`，TypeScript 使用同名字符串联合类型。

```rust
pub struct UsageQuery {
    pub start_at_ms: i64,
    pub end_at_ms: i64,
    pub group_by: UsageGroupBy,
}

pub enum UsageGroupBy { Model, Account }

pub struct TokenBreakdown {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total_tokens: u64,
}

pub struct UsageOverview {
    pub range: UsageRange,
    pub totals: UsageTotals,
    pub rows: Vec<UsageRow>,
    pub last_refreshed_at_ms: Option<i64>,
    pub warnings: Vec<UsageWarning>,
}

pub struct UsageTotals {
    pub tokens: TokenBreakdown,
    pub requests: u64,
    pub estimated_cost_microusd: u64,
    pub subscription_tokens: u64,
    pub unpriced_tokens: u64,
    pub partial_tokens: u64,
    pub unattributed_tokens: u64,
}

pub struct UsageRow {
    pub key: String,
    pub model: String,
    pub source_kind: UsageSourceKind,
    pub provider_id: Option<String>,
    pub account_id: Option<String>,
    pub source_name: String,
    pub tokens: TokenBreakdown,
    pub requests: u64,
    pub estimated_cost_microusd: Option<u64>,
    pub cost_status: CostStatus,
    pub pricing_rule_name: Option<String>,
    pub pricing_rule_version: Option<u64>,
}
```

不要把 SQLite 行结构直接序列化给前端。

## 11. Tauri commands

在 `src/lib/ipc.ts` 和 `src-tauri/src/lib.rs` 一一增加：

```text
get_usage_overview({ query }) -> UsageOverview
refresh_usage({ query }) -> UsageOverview
list_pricing_rules({ scope? }) -> PricingRule[]
save_pricing_rule({ input }) -> PricingRule
delete_pricing_rule({ id }) -> void
reprice_usage({ range }) -> RepriceResult
```

要求：

- Rust command 名称与 TypeScript `CommandMap` 完全一致。
- `refresh_usage` 的文件扫描放到 `spawn_blocking`，不得卡住 UI 线程。
- `UsageLedger` 在 Tauri builder 中只初始化一次并通过 managed state 共享。
- 同时只允许一个 refresh；重复刷新等待同一锁，不得并发扫描同一文件。
- command 只做参数转换、状态获取和错误映射，不得包含 SQL 或计费公式。

初始化顺序必须是：先创建 `Store`，再使用 `store.root()` 打开 `UsageLedger`，最后将二者分别注册为 managed state。不得再次调用 `data_root()` 猜测数据库位置。

`refresh_usage` 的实现顺序必须是：复制可移动的 `UsageLedger` 和 `codex_home` → 进入 `spawn_blocking` → 调用 `refresh` → 调用 `query` → 返回同一刷新时点的结果。

## 12. 前端信息架构

### 12.1 导航

在“账号与服务”和“历史会话”之间新增：

```text
用量与费用
查看本机 Token、模型与估算费用
```

修改：

- `src/types.ts`：`Page` 增加 `usage`。
- `src/App.tsx`：增加 lazy loader、页面描述和预加载行为。
- 图标使用现有 `lucide-react`，不引入图标库。

### 12.2 概览页

只增加轻量摘要，不放完整表格：

- 当前连接卡增加“今日本机 Token”。
- “本机状态”之前增加三张指标卡：总 Token、估算费用、未定价 Token。
- 右侧按钮“查看明细”切到 `usage` 页面。
- 概览加载失败不得影响现有当前连接和本机状态。

### 12.3 用量与费用页

新建 `src/features/usage/usage-page.tsx`。

页面从上到下：

1. PageHeader：标题、说明、日期选择、刷新按钮。
2. 汇总卡：总 Token、调用次数、估算美元、未定价 Token。
3. 分组切换：按模型、按账号。
4. 明细表格。
5. 未定价、部分统计或未归属提示。

表格列：

- 模型 / 来源
- 调用
- 总输入
- 缓存读取
- 缓存写入
- 输出
- 总 Token
- 费用

点击行打开详情 Sheet，展示：

- 来源账号或 Provider/API Key；
- 推理输出；
- 匹配的价格规则和版本；
- 费用公式；
- 数据质量和警告；
- 未归属时的原因。

日期选择首版支持：今天、昨天、最近 7 天、自定义。自定义结束日期不得早于开始日期，范围最多 366 天。

刷新调度：

- 页面第一次打开先显示数据库缓存，再触发一次 refresh；
- 应用窗口重新获得焦点时触发 refresh；
- 页面可见期间每 30 秒 refresh 一次；
- 页面卸载或窗口不可见时清除定时器；
- 同一时刻已有刷新时，不再发第二个前端请求；
- Codex Tools 关闭期间不运行监控，下次打开时增量补扫。

### 12.4 账号与服务页

官方账号行增加：

- `今日 128.4K`
- `套餐用量`、`估算 $0.86` 或 `未配置价格`
- 下拉菜单中的“价格规则”

第三方 Provider 卡片：

- 卡片头显示服务今日 Token 和估算费用；
- 每个 API Key 行显示自己的今日 Token 和估算费用；
- Provider 菜单加入“价格规则”；
- 未使用的 Key 显示 `今日 0`，不得显示空白。

原有额度刷新和本机用量刷新必须是两个不同按钮和任务状态。

### 12.5 价格规则 Dialog

默认使用中转站价格快速设置流程，字段顺序固定：

1. 中转站服务。
2. 模型。
3. 输入 USD / 1M。
4. 输出 USD / 1M。
5. 可展开的缓存读取/写入 USD / 1M。
6. 保存后重算当前范围，默认开启。

后台固定使用 provider + model 精确匹配、按 Token 估算、请求固定费为 0、立即生效；
缓存价格留空时按输入价格估算。保存后自动更新当前表格和汇总，不提供历史数据入口。
所有输入错误在字段下方显示，不能只弹 Toast。

### 12.6 响应式

- `>= 1024px`：完整表格。
- `640px - 1023px`：隐藏推理输出等次要信息，详情 Sheet 中保留。
- `< 640px`：每个模型变为卡片；金额和状态置于卡片顶部。
- 移动端价格编辑使用全屏 Sheet。
- 任何宽度不得出现横向页面滚动；表格容器内部可以滚动。

## 13. 前端格式化规则

新建 `src/features/usage/usage-format.ts`，集中实现：

- `formatTokens(0) -> "0"`
- `formatTokens(128400) -> "128.4K"`
- `formatTokens(1230000) -> "1.23M"`
- `formatMicroUsd(0) -> "$0.00"`
- 小于一美分但大于 0 时保留最多 6 位，例如 `$0.000125`
- 较大金额保留两位小数并带千分位
- 不使用浏览器默认币种符号，统一使用 `$`
- 日期和时间使用现有项目时间工具，避免页面自行拼接

格式化函数必须有单元测试。React 页面不得自己实现第二套格式化逻辑。

## 14. UI 状态

每个页面必须实现：

- 初次加载 Skeleton；
- 空数据状态；
- 刷新中，保留旧数据显示；
- 刷新成功及更新时间；
- 部分文件失败但仍有数据；
- 完全失败；
- 未配置价格；
- 套餐用量；
- 未归属；
- 数据可能偏低；
- 无权限读取 Codex Home；
- 数据库损坏或迁移失败。

禁止在刷新时清空旧表格造成闪烁。刷新按钮必须防止重复点击。

## 15. 实施阶段

### 阶段 0：基线确认

允许修改文件：无。

执行：

```powershell
git status --short --branch
git log -1 --oneline --decorate
npm run check
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

退出条件：

- HEAD 是计划采用的基线；
- 工作区原有改动已识别并保护；
- 前端和 Rust 基线检查通过。

任一基线测试失败时停止，记录原始错误，不得把原有失败误算为本功能引入。

### 阶段 1：DTO 与数据库迁移

允许修改：

- `src-tauri/src/models.rs`
- 新建 `src-tauri/src/local_usage.rs`
- `src-tauri/src/lib.rs` 仅添加 module 和 managed state

工作：

1. 添加 DTO 和枚举。
2. 实现 `UsageLedger::open`。
3. 创建 v1 schema 和索引。
4. 实现重复打开幂等迁移。
5. 添加数据库权限保护，沿用 `secure_file`/`secure_directory` 的平台策略。

测试：空目录建库、重复打开、损坏版本、迁移回滚、约束和索引存在。

退出条件：Rust 测试和 `cargo check` 通过，尚未添加日志解析或 UI。

### 阶段 2：纯日志解析

允许修改：

- 新建 `src-tauri/src/usage_log.rs`
- `src-tauri/src/local_usage.rs`
- 测试夹具目录（如果项目接受）

工作：

1. 实现逐行解析状态机。
2. 保存当前 model/provider/ordinal。
3. 只生成规范化 `last_token_usage` 事件。
4. 处理未知字段、缺失字段、超长行和部分行。

必须有测试：官方常规日志、缓存字段、模型切换、多个 token_count、累计值不重复、部分行、无 session_meta、负数、溢出、未知记录类型。

退出条件：解析器不接触 SQLite；所有行为通过其纯函数输出验证。

### 阶段 3：增量账本和去重

允许修改：

- `src-tauri/src/local_usage.rs`
- 必要时把 `provider_sync::rollout_files` 可见性改为 `pub(crate)`

工作：文件发现、游标、事务、事件写入、索引查询和警告。

必须有测试：

- 首扫新增 N 条；
- 二次扫描新增 0 条；
- 追加一条只新增 1 条；
- 会话移动到归档不重复；
- 文件截断后不重复；
- 最后一行补全后只新增一次；
- 一个坏文件不阻塞好文件；
- 模型上下文跨游标保留。

退出条件：数据库查询的 Token 合计与夹具手工合计完全一致。

### 阶段 4：账号激活时间线

允许修改：

- `src-tauri/src/lib.rs`
- `src-tauri/src/local_usage.rs`
- `src-tauri/src/models.rs`
- 相关 Rust 测试

工作：给所有成功切换路径添加 pending/confirmed/cancelled 记录和启动对账。

必须有测试：官方账号 A→B、Provider Key A→B、切换失败、崩溃遗留 pending、日志 Provider 冲突、早于首条时间线、Cookie/反代标签。

退出条件：无法可靠判断时总是 `unattributed`，绝不默认为当前账号。

### 阶段 5：美元定价

允许修改：

- 新建 `src-tauri/src/pricing.rs`
- `src-tauri/src/local_usage.rs`
- `src-tauri/src/models.rs`

工作：十进制解析、规则版本、匹配、整数费用计算、重新估算。

必须有测试：6 位小数、非法输入、优先级、最长前缀、缓存读写、请求费、套餐、缺价、`i128` 溢出保护、历史版本和指定范围重算。

退出条件：任何路径都没有浮点金额；同一输入重复计算得到完全相同的整数结果。

### 阶段 6：IPC

允许修改：

- `src-tauri/src/lib.rs`
- `src/lib/ipc.ts`
- `src/types.ts`
- IPC 测试

工作：注册六个 command、添加 TypeScript 映射、使用 `spawn_blocking`、统一中文错误。

退出条件：Rust DTO 与 TypeScript 字段逐项一致；所有 command 已注册且测试可调用。

### 阶段 7：用量页面

允许修改：

- `src/App.tsx`
- `src/types.ts`
- 新建 `src/features/usage/*`
- 必要的前端测试

工作：导航、筛选、汇总、分组、表格、详情、状态和响应式。

退出条件：桌面与 360px 宽度均可用；刷新失败仍保留旧数据；没有价格时不显示 `$0.00`。

### 阶段 8：概览与账号页面集成

允许修改：

- `src/features/dashboard/dashboard-page.tsx`
- `src/features/providers/providers-page.tsx`
- `src/features/usage/pricing-rule-dialog.tsx`
- 相关测试

工作：摘要、账号级用量、价格规则入口、额度与用量独立刷新。

退出条件：现有登录、导入、切换、删除、额度刷新行为不回归。

### 阶段 9：文档和最终验证

允许修改：

- `README.md`
- `CHANGELOG.md`
- 本计划状态

执行：

```powershell
npm run check
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

手工验收：

1. 官方 OAuth 发起请求，今日 Token 增加，费用显示套餐用量。
2. Cookie/反代账号配置价格后发起请求，账号、模型和美元估算增加。
3. 第三方 API 配置缓存价格后发起请求，四类 Token 和美元估算正确。
4. 重启 Codex Tools，数字不重复。
5. 会话归档后刷新，数字不重复。
6. 修改价格，旧费用不自动改变。
7. 明确执行重新估算后，指定范围更新。
8. 制造未知归属记录，UI 显示未归属而不是当前账号。
9. 跨过本机午夜，今天归零并保留昨天数据。
10. 断开网络后，本机日志统计仍可工作。

## 16. 禁止性规则

执行模型不得：

- 不得在没有失败测试前大规模改动解析或计费核心。
- 不得用日志文件路径作为唯一事件 ID。
- 不得累计 `total_token_usage`。
- 不得把缓存读取与全部输入重复收费。
- 不得用 JavaScript `number` 作为价格事实来源。
- 不得使用 `f64` 存储金额。
- 不得把未知账号归给当前账号。
- 不得把价格规则写入 Provider 密钥字段。
- 不得记录 API Key、Cookie 或 access token。
- 不得在 React 页面执行 SQL、扫描文件或复制计费公式。
- 不得引入新状态管理库、图表库、数据库或时间库。
- 不得无关重构现有账号、额度或会话功能。
- 不得删除或覆盖用户工作区已有改动。
- 不得在测试失败时继续下一阶段。

## 17. 完成定义

只有同时满足以下条件才能宣布完成：

- 三类账号均能显示本机今日 Token；
- 模型和所有 Token 分类可见；
- 美元价格可配置且费用只标记为估算；
- 官方套餐、未定价、部分统计、未归属状态准确；
- 重复刷新、重启和归档不重复计数；
- 账号切换后的归属正确；
- 所有新增 Rust 和前端测试通过；
- `npm run check`、`cargo test`、`cargo check` 全部通过；
- 桌面端和 360px 移动宽度完成视觉检查；
- README 说明数据来源、隐私边界和限制；
- 最终交付列出改动文件、验证结果、已知限制和未完成项。
