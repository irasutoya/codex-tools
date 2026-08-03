# Changelog

本项目遵循 [Semantic Versioning](https://semver.org/)。

## [Unreleased]

### Added

- 新增“用量与费用”页面：从本机 Codex rollout 日志增量统计官方账号、Cookie/反代账号和第三方 API 的 Token、模型、缓存字段及 USD 估算费用；更新后建立新统计周期，旧 Token 不回放、不计入当前周期；支持详情抽屉和移动端布局。
- 概览与账号页增加今日 Token/费用摘要，并保留官方套餐用量、未定价、部分数据和未归属状态，不把估算冒充实际账单。
- 官方 OpenAI 来源现在可按日志自动识别，并内置 GPT-5.6 Sol、Terra、Luna 的官方参考价；界面明确标记“官方参考价 / 非实际账单”，中转站仍使用自定义价格规则。
- 中转站价格设置改为快速流程：按服务和模型输入输入/输出单价，保存后默认自动重算当前范围；缓存价格留空时按输入单价估算。
- 修复会话中途切换 Provider 后 Token 仍被归为官方账号的问题：解析 `thread_settings_applied`，冲突时标记未归属，并在解析器升级时重建当前统计周期。
- 修复子任务日志复制父任务历史导致 Token 重复统计的问题：识别 `source.subagent.thread_spawn`，只统计 `task_started` 之后的真实事件；来源已知但账号未确认时不再套用中转站价格。

### Changed

- 自动刷新现在只在 Codex Tools 处于前台活动窗口且页面已打开时运行；最小化、切换到其他软件或窗口失去焦点后会暂停，恢复前台时按到期状态补一次。
- OpenAI 额度刷新改为按额度变化、无消耗、已用完、限流和失败状态自适应调度，避免在额度长期不变或已经用完时持续高频请求。
- 每次进入“用量与费用”都会立即扫描本机用量；进入概览或账号页时会立即校准可查询的 OpenAI 额度，并继续使用并发去重和状态退避策略。

### Fixed

- 用量与费用跨过本地零点后会重新计算“今天/昨天/最近 7 天”的范围，不再继续显示昨天的金额。
- 额度接口返回非成功状态时不再误判为成功；失败会保留最后一次有效额度，登录失效或不支持查询的账号会停止自动重试。

## [0.3.4] - 2026-07-31

### Changed

- macOS 安装说明按 Apple Silicon 与 Intel 分开，并补充未公证应用的一次性安全确认方法。

### Fixed

- macOS `.app` 现在由 Tauri 对整个应用包执行 ad-hoc 签名，不再只保留 Mach-O 链接器签名，避免 Gatekeeper 将有效下载误判为“应用已损坏”。
- CI 和 Release 在上传 DMG 前强制运行 `codesign --verify --deep --strict`；签名结构无效时发布流程会直接失败。

## [0.3.3] - 2026-07-31

### Changed

- 移除侧栏折叠入口和快捷操作，窗口最小尺寸固定为默认的 1180×760；界面只按完整桌面布局组织，不再为窄窗口压缩核心信息。
- 统一四个页面的内容宽度、间距、卡片边界、侧栏选中态和语义色对比度，并将主题入口收进页面标题区。
- 发布产物按架构拆分：macOS 分别构建 arm64 与 x64 DMG，Windows 仅构建 x64 NSIS 安装包；CI 同步覆盖这三个明确的平台目标。

### Fixed

- 移除左上角折叠按钮及竖向分隔线；当前账号不再显示低对比度的禁用按钮，改为明确的高对比状态标签。
- 额度状态重新按 shadcn Item 组合排版：单周期占满可用宽度，双周期等宽展示，避免只有一个周期时留下大块空白。
- 根布局改为使用 WebView 的实际可视高度，默认窗口下侧栏页脚、分页栏和最后一张卡片不再被 macOS 标题栏挤出画面。

## [0.3.2] - 2026-07-31

### Changed

- 按 shadcn `base-nova` 规范重构应用壳层与全部四个页面，改用 inset 侧栏、紧凑顶部栏、统一页面标题和一致的区块层级。
- OpenAI 账号改为单层列表，保留切换操作并将刷新、删除收进上下文菜单；第三方 API、额度信息和活动状态采用相同的信息密度与反馈方式。
- 概览、历史会话和配置页统一卡片、空状态、加载状态及响应式布局，减少嵌套容器和无效留白。

### Fixed

- 账号与 API Key 的操作区在窄窗口下会完整换行；会话表格可横向滚动，配置卡片不再被同列内容强制拉高。
- Cookie 导入弹窗在 390 像素宽窗口和超长凭据下仍保持固定输入高度、内部滚动与可访问的底部操作区。

## [0.3.1] - 2026-07-31

### Changed

- 按项目的 shadcn `base-nova` 规范统一表单禁用态、按钮图标、组件组合和窄窗口换行行为；诊断信息与额度时间格式改为复用计算结果。
- 全部页面统一操作结果、错误原因和后续建议的表达方式，账号页将“反代号”调整为“Cookie 账号”，并明确数据来源、写入范围和实际行为。
- 批量额度查询最多同时发送 4 个请求，避免大量账号同时刷新时占满网络连接。

### Fixed

- Cookie JSON 输入区改为固定高度并限制最大输入长度，长内容在输入框内换行滚动，不再撑出应用窗口。
- 所有弹窗、文本域和错误详情增加尺寸上限，异常长的名称、路径、响应或错误信息不会再撑破界面。
- 服务、API 地址、API Key、账号、应用数据、Codex 配置和会话文件增加分层大小限制；超出安全范围时停止处理并显示明确原因。
- Codex 配置读取失败不再显示为“可以读取”；会话扫描警告会聚合显示，超大会话行和文件会安全跳过。

## [0.3.0] - 2026-07-31

### Added

- Codex 账号页新增独立的反代号登录入口，支持导入 `at-...`、`accessToken`、`refresh_token` 或账号 JSON，原始 JSON 不会落盘。
- 正常网页登录账号与反代号统一从 OpenAI 官方 `wham/usage` 查询并缓存 5H/7D 额度，支持单个、批量和 Dashboard 刷新。
- 仅含 accessToken 的反代号使用 Codex 官方支持的 `personal_access_token` 登录格式；第三方 Responses API 继续只管理 API Key。

## [0.2.0] - 2026-07-24

### Added

- OpenAI 设备码登录、Token 刷新和第三方 Provider 连通性测试现在遵循 Windows 与 macOS 系统代理；显式设置的 `HTTP_PROXY`、`HTTPS_PROXY` 或 `ALL_PROXY` 环境变量仍然优先。
- 系统代理例外列表会同步应用于后端网络请求，局域网、本机地址和用户配置的直连域名不会被错误转发。

### Changed

- OAuth 与 Provider 测试复用统一的网络客户端配置，并共享连接超时、连接池和 TCP keepalive 策略，减少重复配置及网络行为差异。
- Windows 系统代理支持按 HTTP/HTTPS 分协议配置，macOS 支持从 System Configuration 读取代理端点与例外列表。

## [0.1.0] - 2026-07-22

### Added

- Provider、账号、会话管理和 shadcn/ui 桌面界面。
- 贡献指南和维护者发布清单。
- OpenAI Account 设备码登录、多账号保存与切换；登录请求使用 Codex CLI 风格的 `User-Agent`，OAuth 凭据不会返回 WebView。
- macOS 11+ 运行与打包支持，包括系统浏览器登录、平台化 Codex CLI 发现、Application Support 数据目录、通用架构 DMG 和 macOS CI。

### Changed

- 重写面向发布的 README，补充下载安装、首次使用、数据安全、系统签名告警和开发说明。
- 发布工作流现在校验项目版本与标签、从对应 Changelog 版本提取 Release notes，并为安装包生成 SHA-256 校验文件。
- CI 和发布任务增加并发控制与超时限制，项目元数据补充运行时版本、主页和问题追踪入口。
- 将只在构建阶段提供 Tailwind 样式和组件工具的 shadcn 包从运行时依赖移至开发依赖。
- 桌面运行时全面迁移到 Tauri 2 + Rust，React/shadcn 只负责按页懒加载的界面。
- 应用数据统一为平台数据目录中的 `app.yaml`，旧 `data/config.yaml` 不再读取，也不创建应用 SQLite。
- 删除本地路由、协议转换与请求日志，只保留 OpenAI Responses Provider 管理；API 地址和请求头直接写入 Codex `config.toml`，API Key 写入 `auth.json`。
- 会话归属修复收窄到 rollout 首条 metadata 与已识别 Codex SQLite schema。
- 删除第三方模型解锁和本地模型目录生成，模型列表改由 Codex 直接从 Provider API 获取。
- 重写 Provider 切换：第三方模式写入 `custom` Responses 字段；官方模式删除受管第三方字段；MCP、Skills、Hooks、沙箱、其他 Provider 和未知配置保持不变，`auth.json` 按完整官方凭据或仅含 `OPENAI_API_KEY` 的第三方凭据清空重写。
- 上游密钥和自定义请求头改为 Rust 后端只写、前端脱敏；会话扫描增加大小及并发边界。
- 删除应用配置版本迁移、废弃协议兼容分支、旧模型字段清理和未使用 IPC；相同配置不再重复写盘，账号切换前不再扫描全部会话。

[Unreleased]: https://github.com/irasutoya/codex-tools/compare/v0.3.4...HEAD
[0.3.4]: https://github.com/irasutoya/codex-tools/compare/v0.3.3...v0.3.4
[0.3.3]: https://github.com/irasutoya/codex-tools/compare/v0.3.2...v0.3.3
[0.3.2]: https://github.com/irasutoya/codex-tools/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/irasutoya/codex-tools/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/irasutoya/codex-tools/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/irasutoya/codex-tools/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/irasutoya/codex-tools/releases/tag/v0.1.0
