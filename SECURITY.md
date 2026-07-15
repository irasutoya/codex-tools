# Security Policy

## Supported versions

项目处于早期预览阶段，仅最新版本接收安全修复。

## Reporting a vulnerability

请通过 GitHub 仓库的私密漏洞报告功能提交问题。不要在公开 Issue 中附带 API Key、`auth.json`、`data/app.yaml` 或 Codex 会话数据。

## Security boundaries

- 本地代理没有客户端认证，只允许绑定 `127.0.0.1:16384`，并拒绝非回环来源、转发代理头和 Origin 请求。
- `experimental_bearer_token` 是 Codex 兼容占位符，不是安全凭据，代理不会验证它。
- 上游 API Key 与 OpenAI Account OAuth 凭据以明文保存在便携式 `data/app.yaml`；请使用受保护的 Windows 用户账户并限制该目录访问。
- WebView 只能提交新 API Key 或启动设备授权；账号读取、OAuth 轮询和保存结果均由 Rust 后端脱敏，不会把已保存密钥或 token 返回前端。
- 官方模式会读取和写入 Codex `auth.json`；第三方模式会把它清空为 `{}`。应用不会记录请求体、响应体、API Key、OAuth token 或完整兼容 token。
- 官方模式会清空 `config.toml`；第三方模式会用最小配置替换它。此操作不会备份原配置，并会删除 MCP、Skills、Hooks、沙箱及未知字段。
- 会话迁移不创建备份，只修改已识别的 provider 字段，并用意图日志支持中断后继续。

## Windows 安全软件告警

- 发布包应使用可信 Authenticode 证书签名；无签名的本地代理和开发构建可能触发启发式检测。
- 不建议关闭实时防护或直接添加目录白名单。若出现告警，应先停止分发，核对源码、依赖锁文件、构建资源和哈希。
- `data/`、旧配置、API Key 和 Codex 会话不属于 Tauri bundle 资源；Release 只嵌入 `dist/` 前端产物和声明的应用图标。
