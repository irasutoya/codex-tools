# Codex Tools

Codex Tools 是面向 Windows 10/11 x64 的单体 Codex Provider、账号和会话管理工具。它不会使用 CDP、不会向 Codex 注入脚本，也不会修改 Codex 安装目录。

> 当前版本为 `0.1.0` 早期预览版。Provider 切换和数据库修复会在写入前创建备份，但仍建议先自行备份 `%USERPROFILE%\.codex`。

## 功能

- 保存多个 Responses 或 Chat Completions 兼容 Provider。
- 每个 Provider 管理多个 API 账号并一键切换。
- 保存和切换 Codex 官方 OAuth 登录快照。
- 在应用进程内运行仅监听 loopback 的 Responses → Chat Completions 适配代理。
- 切换账号时同步已识别的 Codex 会话 Provider，统一历史可见性。
- 扫描默认 SQLite、`sqlite_home`、`CODEX_SQLITE_HOME` 和 rollout JSONL。
- 修复前备份 SQLite、WAL/SHM、rollout 和全局状态，并支持失败回滚。
- 搜索、Markdown 导出和永久删除已识别会话。

## 安全说明

按项目设计，API Key 和官方登录快照以明文保存在本机 `codex-tools.db`。完整应用备份也可能包含明文凭据：

- 不要将数据库、备份或诊断附件上传到公开位置。
- Provider 测试不会在界面或日志中返回完整上游响应。
- 未识别的 Codex SQLite schema 只读扫描，不猜测字段并写入。
- 只迁移 Codex Tools 明确认识的历史 Provider ID，不改写用户自定义 Provider。

## 开发环境

- Node.js 22 LTS（Node.js 24 也可用于当前依赖）
- npm 10+
- Rust 1.85+
- Visual Studio 2022 Build Tools
  - Desktop development with C++
  - MSVC v143 x64/x86 build tools
  - Windows 10/11 SDK
- Microsoft Edge WebView2 Runtime

```powershell
npm ci
npm run tauri:dev
```

## 质量检查

```powershell
npm run check
cd src-tauri
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## 构建安装包

```powershell
npm ci
npm run tauri:build
```

NSIS 安装包输出到 `src-tauri\target\release\bundle\nsis`。应用采用 current-user 安装，不需要管理员权限。

## 项目结构

```text
src/
  components/ui/          shadcn/ui 源码组件
  features/providers/     Provider 与账号领域逻辑
  lib/                    前端基础设施与 Tauri IPC
  types.ts                前后端 IPC 数据契约
src-tauri/src/
  codex.rs                Codex 配置、扫描与会话操作
  protocol_proxy.rs       loopback 协议适配代理
  provider_sync.rs        会话统一、备份与回滚
  storage.rs              codex-tools.db 与迁移
  models.rs               Rust IPC 数据类型和稳定错误
```

## 发布

- 推送普通提交会运行 Windows CI。
- 推送形如 `v0.1.0` 的 tag 会构建 NSIS 安装包并创建 GitHub Release。
- 发布前需同步 `package.json`、`src-tauri/Cargo.toml` 与 `src-tauri/tauri.conf.json` 的版本号。

## 致谢

项目在行为设计上参考了 [CodexPlusPlus](https://github.com/BigPizzaV3/CodexPlusPlus) 和 [cc-switch](https://github.com/farion1231/cc-switch)，实现代码独立编写。

## 许可证

[MIT](LICENSE)
