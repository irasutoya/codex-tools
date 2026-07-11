# Security Policy

## Supported versions

项目目前处于早期预览阶段，仅最新版本接收安全修复。

## Reporting a vulnerability

请通过 GitHub 仓库的私密漏洞报告功能提交安全问题，不要在公开 Issue 中附带 API Key、`auth.json`、`codex-tools.db` 或 Codex 会话数据库。

报告中请包含受影响版本、复现步骤、预期影响，以及已经脱敏的诊断信息。

## Credential storage

Codex Tools 按设计明文保存 API Key 与官方登录快照。用户应保护本机账户、应用数据库和导出的完整备份。
