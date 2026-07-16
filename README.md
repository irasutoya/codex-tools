# Codex Tools

Codex Tools 是面向 macOS 11+ 与 Windows 10/11 的轻量 Tauri 2 + Rust 桌面工具。它只管理兼容 OpenAI Responses API 的 Provider，并把所选 API 地址和凭据直接写入 Codex 的 `custom` provider 配置。

## 功能

- 管理兼容 OpenAI Responses API 的上游及多个 API 账号。
- Codex 直接使用第三方 API 自己提供的模型列表，不生成或覆盖本地模型目录。
- Codex 直接连接上游，本程序不启动 HTTP 监听器、不转发请求，也不转换其他协议。
- 支持 OpenAI Account 设备码登录，并可在多个官方账号与第三方 API 之间切换。
- 切换时使用 TOML 解析器更新或删除受管 Provider 字段；MCP、Skills、Hooks、沙箱、其他 Provider 及未知配置保持不变。
- 第三方供应商切换时 Codex provider 始终为 `custom`；仅在 `openai` 与 `custom` 模式互切后，自动把全部已识别会话的 provider 元数据统一到新模式。
- 直接读取 Codex JSONL/SQLite，不创建应用 SQLite，不保存聊天正文，不生成会话备份。

## Codex 配置

应用在现有 `config.toml` 上更新以下字段，`base_url` 是所选 Provider 的真实 API 地址：

```toml
model_provider = "custom"

[model_providers.custom]
name = "<Provider 名称>"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://api.example.com/v1"

[model_providers.custom.http_headers]
X-Custom-Header = "<可选请求头>"
```

切换自定义账号时会清空并重写 `auth.json`，文件中只保留 `OPENAI_API_KEY`：

```json
{
  "OPENAI_API_KEY": "sk-..."
}
```

第三方 API 需要提供 OpenAI 兼容的模型列表与 Responses 接口。Codex 会直接向 Provider 请求模型列表和 `/responses`，本应用只在“测试连接”时检查模型列表接口是否可访问，不读取或保存返回内容。

## OpenAI Account 登录与切换

应用使用与 Codex CLI 相同的 OpenAI 设备授权端点和 Codex CLI 风格的 `User-Agent` 发起登录。登录完成后，完整凭据只在 Rust 后端处理，并保存到 `data/app.yaml`；前端只会收到账号名称、邮箱、到期时间等脱敏元数据。

两种认证模式严格互斥。`config.toml` 不会被整体清空，但 `auth.json` 每次都会按目标模式完整重写：

- 切换到官方账号：删除根配置和 Profile 中的 `model_provider` 以及 `[model_providers.custom]`，再把所选账号的完整官方凭据写入 `~/.codex/auth.json`。
- 切换到第三方 API：将根配置和已有 Profile 中的 `model_provider` 调整为 `custom`，写入 `[model_providers.custom]`，再将 `auth.json` 重写为仅含 `OPENAI_API_KEY` 的对象。

程序会先完整解析 TOML，使用结构化语法树更新字段，避免文本删改误伤其他配置。应用不管理或清理 `model`、模型目录以及其他未知字段。第三方配置在预览和应用之间会校验 `config.toml` 和 `auth.json` 是否发生变化；提交时依次清空认证、原子写入配置，再写入目标认证，避免 API 地址与错误账号凭据短暂配对。文件内容没有变化时会跳过写盘。原有 `auth.json` 内容无需可解析，因为目标文件会被完整重建。

## 数据与安全

Windows 便携式数据位于可执行文件同级 `data/`：

```text
Codex Tools.exe
data/
  app.yaml
```

第三方 API Key 和 OpenAI Account OAuth 凭据按产品约定以明文保存在平台对应的 `app.yaml`，激活第三方账号后 API Key 还会以明文写入 Codex `auth.json`。请勿上传、同步或分享这些文件。应用只读取当前的 `app.yaml`，不会扫描、导入或删除其他应用数据文件；日志和前端诊断不包含 API Key 或 OAuth token。

macOS 安装包的数据位于 `~/Library/Application Support/io.github.irasutoya.codex-tools/`，避免修改只读或已签名的 `.app`；目录与配置文件权限分别限制为当前用户可访问的 `0700` 和 `0600`。可在两个平台通过 `CODEX_TOOLS_DATA_DIR` 覆盖数据目录；macOS 图形应用找不到 Codex CLI 时，还会检查 Homebrew、用户级安装以及 Codex/ChatGPT 应用内置的 CLI，也可通过 `CODEX_BIN` 显式指定。

## 开发

需要 Node.js 24、Rust 1.85+，以及当前平台的 Tauri 构建依赖。Windows 需要 WebView2 和 NSIS；macOS 需要 Xcode Command Line Tools。

```shell
npm ci
npm run dev
```

质量检查：

```shell
npm run check
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

Windows 安装包：

```powershell
npm run dist:win
```

输出位于 `src-tauri/target/release/bundle/nsis/`。

macOS 原生安装包：

```shell
npm run dist:mac
```

输出位于 `src-tauri/target/release/bundle/dmg/`。发布流程使用 `npm run dist:mac:universal` 同时包含 Apple Silicon 与 Intel 架构；本地执行前需安装 `aarch64-apple-darwin` 和 `x86_64-apple-darwin` Rust targets。

## 许可证

[MIT](LICENSE)
