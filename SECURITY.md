# Security Policy

## Supported versions

项目处于早期预览阶段，仅最新版本接收安全修复。

## Reporting a vulnerability

请通过 GitHub 仓库的私密漏洞报告功能提交问题。不要在公开 Issue 中附带 API Key、`auth.json`、`data/app.yaml` 或 Codex 会话数据。

## Security boundaries

- 应用不启动本地 HTTP 路由或代理。第三方模式下 Codex 直接连接所选 Responses API 地址。
- 上游 API Key 与 OpenAI Account OAuth 凭据以明文保存在 Windows 便携式 `data/app.yaml` 或 macOS 用户 Application Support 目录；激活第三方账号后，API Key 还会写入 Codex `auth.json` 的 `OPENAI_API_KEY`。请使用受保护的系统账户并限制这些文件的访问。
- WebView 只能提交新 API Key 或启动设备授权；账号读取、OAuth 轮询和保存结果均由 Rust 后端脱敏，不会把已保存密钥或 token 返回前端。
- 官方模式和第三方模式都会先解析 Codex `config.toml`，无效 TOML 会被拒绝；第三方配置在预览与应用之间还会校验 `config.toml` 和 `auth.json` 的并发变更。
- 第三方切换会更新全部 `model_provider` 与受管的 `custom` Responses 字段；官方切换会删除受管第三方字段。MCP、Skills、Hooks、沙箱、其他 Provider 和未知配置保持不变，`auth.json` 会按官方凭据或仅含 `OPENAI_API_KEY` 的第三方凭据完整重写。应用不处理 Codex 的请求体或响应体，也不会把 API Key 或 OAuth token 写入日志和前端诊断。
- 会话归属修复不创建备份，只修改已识别的 provider 字段；写入前会确认会话文件未被 Codex 并发修改，SQLite 更新使用事务提交。

## 安装包签名与系统告警

- Windows 发布包应使用可信 Authenticode 证书签名，macOS 发布包应使用 Developer ID 签名并完成公证；无签名的开发构建可能触发系统拦截或启发式检测。
- 不建议关闭实时防护或直接添加目录白名单。若出现告警，应先停止分发，核对源码、依赖锁文件、构建资源和哈希。
- 应用数据目录、旧配置、API Key 和 Codex 会话不属于 Tauri bundle 资源；Release 只嵌入 `dist/` 前端产物和声明的应用图标。
