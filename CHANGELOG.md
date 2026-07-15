# Changelog

本项目遵循 [Semantic Versioning](https://semver.org/)。

## [Unreleased]

### Changed

- 桌面运行时全面迁移到 Tauri 2 + Rust，React/shadcn 只负责按页懒加载的界面。
- 应用数据统一为程序同级 `data/app.yaml` 和 `data/model_catalog.json`，旧 `data/config.yaml` 保留但不再读取，也不创建应用 SQLite。
- 本地路由改为固定监听器和可热替换上游目标。
- 会话迁移收窄到 rollout 首条 metadata 与已识别 Codex SQLite schema。
- 模型目录同步优先保留完整 Codex 原生 Agent/工具定义。
- Codex 官方账号与第三方 API 改为严格互斥：官方模式清空 `config.toml` 并写入 `auth.json`，第三方模式清空 `auth.json` 并写入最小 `custom` 配置。
- 上游密钥和自定义请求头改为 Rust 后端只写、前端脱敏，代理响应和会话扫描增加大小及并发边界。

### Added

- OpenAI Account 设备码登录、多账号保存与切换；登录请求使用 Codex CLI 风格的 `User-Agent`，OAuth 凭据不会返回 WebView。

## [0.1.0] - 2026-07-11

### Added

- Provider、账号、本地路由、会话管理和 shadcn/ui 桌面界面。

[Unreleased]: https://github.com/irasutoya/codex-tools/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/irasutoya/codex-tools/releases/tag/v0.1.0
