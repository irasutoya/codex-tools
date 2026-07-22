# Changelog

本项目遵循 [Semantic Versioning](https://semver.org/)。

## [Unreleased]

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

[Unreleased]: https://github.com/irasutoya/codex-tools/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/irasutoya/codex-tools/releases/tag/v0.1.0
