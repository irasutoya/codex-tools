# Codex Tools

Codex Tools 是面向 Windows 10/11 x64 的轻量 Tauri 2 + Rust 桌面工具。它通过固定的 `custom` provider 把 Codex 连接到本机 Responses 代理，并在代理内部切换第三方上游，不注入或修改 Codex 客户端。

## 功能

- 管理 OpenAI Responses、Chat Completions、Anthropic Messages 上游及多个 API 账号。
- 固定监听 `127.0.0.1:16384`；拒绝非回环、代理转发头和跨域请求。
- Responses 上游原样流式转发，Chat Completions 转换文本、推理、usage 和工具调用，Anthropic Messages 转换为 Responses 结果。
- 支持 OpenAI Account 设备码登录，并可在多个官方账号与第三方 API 之间切换。
- 官方模式只写入 Codex 原生 `auth.json` 并清空 `config.toml`；第三方模式清空 `auth.json`，再写入最小 `custom` provider 配置。
- 第三方供应商切换时 Codex provider 始终为 `custom`；仅在 `openai` 与 `custom` 模式互切后，自动把全部已识别会话的 provider 元数据统一到新模式。
- 直接读取 Codex JSONL/SQLite，不创建应用 SQLite，不保存聊天正文，不生成会话备份。

## Codex 配置

应用维护以下字段；兼容 token 由系统安全随机数生成，并在连续使用第三方模式时保持稳定：

```toml
model = "<当前模型>"
model_provider = "custom"
model_catalog_json = "<程序数据目录>/model_catalog.json"

[model_providers.custom]
name = "Custom"
base_url = "http://127.0.0.1:16384/v1"
wire_api = "responses"
experimental_bearer_token = "ct_<随机32字节Base64URL>"
request_max_retries = 4
stream_max_retries = 3
stream_idle_timeout_ms = 300000
```

两个重试参数可在“本地代理”页面设置，范围均为 `0–100`，默认值分别为 `4` 和 `3`。

当前使用第三方 Provider 时，保存当前 Provider、账号或本地代理设置，以及应用启动，都会自动同步 `config.toml`、`model_catalog.json` 和代理运行配置。

本地代理不验证 `experimental_bearer_token`；它仅作为 Codex 第三方 API 的兼容占位符。

## OpenAI Account 登录与切换

应用使用与 Codex CLI 相同的 OpenAI 设备授权端点和 Codex CLI 风格的 `User-Agent` 发起登录。登录完成后，完整凭据只在 Rust 后端处理，并保存到 `data/app.yaml`；前端只会收到账号名称、邮箱、到期时间等脱敏元数据。

两种模式严格互斥：

- 切换到官方账号：先清空 `config.toml`，再将所选账号凭据写入 `~/.codex/auth.json`。
- 切换到第三方 API：先将 `auth.json` 清空为 `{}`，再用必要字段替换 `config.toml`。

切换不会创建配置或会话备份。第三方模式会删除原 `config.toml` 中的 MCP、Skills、Hooks、沙箱及未知字段，界面会在执行前明确确认这一破坏性操作。

## 数据与安全

便携式数据位于可执行文件同级 `data/`：

```text
Codex Tools.exe
data/
  app.yaml
  model_catalog.json
```

第三方 API Key 和 OpenAI Account OAuth 凭据按产品约定以明文保存在 `data/app.yaml`，请勿上传、同步或分享该文件。旧 `data/config.yaml` 不会导入或读取。日志和前端诊断不包含 API Key、OAuth token、完整兼容 token、请求体或响应体。

## 开发

需要 Node.js 24、Rust 1.85+、Windows WebView2 和 NSIS 构建依赖。

```powershell
npm ci
npm run dev
```

质量检查：

```powershell
npm run check
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Windows 安装包：

```powershell
npm run dist:win
```

输出位于 `src-tauri/target/release/bundle/nsis/`。

## 许可证

[MIT](LICENSE)
