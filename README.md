# Codex Tools

<p align="center">
  <img src="public/codex-tools.svg" alt="Codex Tools" width="96" height="96" />
</p>

<p align="center">
  在 Windows 和 macOS 上管理 Codex 的 OpenAI 账号与兼容 Responses API 的第三方服务。
</p>

<p align="center">
  <a href="https://github.com/irasutoya/codex-tools/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/irasutoya/codex-tools/actions/workflows/ci.yml/badge.svg" /></a>
  <a href="https://github.com/irasutoya/codex-tools/releases/latest"><img alt="GitHub Release" src="https://img.shields.io/github/v/release/irasutoya/codex-tools?display_name=tag&sort=semver" /></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-blue.svg" /></a>
</p>

Codex Tools 是一个轻量的 Tauri 2 + Rust 桌面应用。它直接更新 Codex
配置，不保存聊天正文；仅当接入只提供 Chat Completions API 的服务时，才在本机
`127.0.0.1:27777` 启动一个转换代理，把 Responses API 请求翻译成 Chat
Completions 请求转发，其余场景请求仍由 Codex 直连。

## 主要功能

- 保存并切换多个 OpenAI Account，使用设备码完成授权。
- OpenAI 登录、Token 刷新和服务测试自动遵循 Windows/macOS 系统代理，并保留环境变量代理优先级。
- 管理多个兼容 OpenAI Responses API 或 Chat Completions API 的服务，一个服务对应一个 API Key；保存/编辑服务或**切换服务**时自动静默获取服务端 `/models` 接口返回的可用模型（只保存接口实际返回的模型 id），并用 models.dev（`models.json`）**匹配**补充展示名、上下文窗口与简介——兼容纯模型 id（`deepseek-v4-flash`）、厂商前缀（`deepseek/deepseek-v4-flash`、`deepseek:deepseek-v4-flash`）与大小写差异；「账号与服务」页可对每个服务手动「同步模型」，前台停留时也会每 10 分钟自动同步一次。
- 只提供 Chat Completions API 的服务（如 DeepSeek、Moonshot、GLM、Qwen 等）可在编辑服务时选择“Chat Completions 转换”：本应用在本机 `127.0.0.1:27777` 启动转换代理，把 Codex 的 Responses API 请求自动翻译成 Chat 请求转发，再把上游流式/非流式响应翻译回 Responses 格式，支持多轮工具调用、图片输入与思考模式；代理会记住上一轮 `reasoning_content` 并在下一轮请求中回传（DeepSeek 等 thinking 模式的硬性要求）；遇到不支持 `response_format` 结构化输出的上游时自动降级为提示词方式重试；代理固定端口、只监听本机，Codex 侧使用固定 Key，真实的服务商 Key 只保存在本应用、由代理注入上游请求。
- 只更新受管的 `custom` Provider 字段，保留 MCP、Skills、Hooks、沙箱及未知配置。
- 通过 Chrome DevTools Protocol (CDP) 解锁 Codex 桌面应用的模型列表：模型目录只包含**当前激活服务商**实际存在的模型（id 与服务 `/models` 接口返回完全一致的可用模型），不注入任何内置模型；生成 `model_catalog_json` 模型目录（`model-catalogs/codex-tools.json`）让 CLI 与应用返回自定义模型；从概览页「打开 Codex（自动解锁）」或设置页重启时以调试模式启动并注入脚本，把选择器白名单补齐为被订阅等级隐藏的当前服务商模型；提示词保持通用、不绑定具体模型版本；注入只作用于内存，不修改安装文件。
- 在官方账号与第三方 API 之间切换时，同步已识别会话的 Provider 元数据。
- 直接读取 Codex JSONL/SQLite；本机用量索引保存在独立的 `usage.sqlite3`，不保存聊天正文。
- 支持 Windows 10/11 和 macOS 11+（Apple Silicon）。

## 下载与安装

从 [Releases](https://github.com/irasutoya/codex-tools/releases/latest)
下载适合当前系统的安装包：

| 平台                | 发布产物                | 系统要求                |
| ------------------- | ----------------------- | ----------------------- |
| Windows x64         | `_x64-setup.exe` 安装包 | Windows 10/11、WebView2 |
| macOS Apple Silicon | `_aarch64.dmg` 磁盘映像 | macOS 11 或更高版本     |

> [!IMPORTANT]
> 当前自动构建的安装包尚未配置 Authenticode 或 Apple Developer ID
> 签名。系统可能显示“未知发布者”或阻止打开。请先核对 Release 来源和
> `SHA256SUMS.txt`；不确定时请从源码构建，不要关闭系统安全防护。

macOS 用户将应用拖入“应用程序”后，如果首次启动被 Gatekeeper 阻止，请打开
“系统设置 → 隐私与安全性”，确认来源后选择“仍要打开”。如果已经核对
`SHA256SUMS.txt`，但系统仍将应用误报为“已损坏”，可以只移除 Codex Tools 的
下载隔离标记，然后重新打开：

```shell
xattr -dr com.apple.quarantine "/Applications/Codex Tools.app"
```

此命令只处理指定应用，不会关闭 macOS 的全局安全检查。

Codex Tools 面向已经安装并使用 Codex 的用户。启动后应用会自动寻找 Codex
配置目录和 CLI；也可以通过 `CODEX_HOME` 和 `CODEX_BIN` 环境变量指定位置。

## 快速开始

1. 启动 Codex Tools，先在设置页确认检测到的 Codex 路径；未识别到时可点击“手动选择…”指定 Codex 桌面应用（macOS 选 `.app`，Windows 选可执行文件）。
2. 选择“OpenAI Account”并按设备码提示登录，或添加兼容 Responses API
   的 Provider、API 地址和 API Key。
3. 测试第三方连接后，选择要使用的账号并确认配置预览。
4. 应用切换，然后从概览页点击“打开 Codex（自动解锁）”——会以调试模式启动并自动注入模型解锁脚本；已运行的实例会先退出重启，已解锁的实例直接刷新目录不重启。

切换前建议退出正在运行的 Codex 实例。应用会检测预览后发生的并发修改，但仍应
避免同时用其他工具编辑 `config.toml` 或 `auth.json`。

## 配置行为

第三方账号激活后，应用会把所选服务写入 Codex 配置：

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

对应的 `auth.json` 会被完整重建，只保留当前第三方凭据：

```json
{
  "OPENAI_API_KEY": "sk-..."
}
```

切换回官方账号时，应用会删除根配置和现有 Profile 中受管的 `custom`
Provider 字段，并写入所选 OpenAI Account 的完整凭据。配置通过 TOML
语法树更新；其他 Provider、MCP、Skills、Hooks、沙箱及未知字段保持不变。

第三方服务必须提供兼容 OpenAI 的模型列表与 Responses API。Codex 会直接连接
该服务；Codex Tools 只在“测试连接”时检查模型列表接口，不读取或保存响应内容。

## 数据与安全

### 本机 Token 用量与美元估算

“用量与费用”页面只读取当前电脑上的 Codex 会话 JSONL 日志，不 Hook、注入、抓包或代理
Codex 请求，也不会上传用量数据。统计范围是本机被 Codex 写入的会话；其他设备、直接绕过
Codex 调用 API 的请求，以及服务商后台账单不会出现在这里。

页面中的美元金额是根据 OpenAI 内置官方参考价或你手动填写的 API 服务 USD 价格规则计算的估算值，
不等于 OpenAI 或 API 服务的实际扣费。官方套餐实际账单无法只从本机 Token 日志推导，界面会明确标记
“官方参考价 / 非实际账单”。用量事件、账号归属快照和价格规则保存在应用数据目录的
`usage.sqlite3`，与账号配置分离。升级后首次刷新会建立新的统计周期，并把已知 rollout 游标置于文件尾部；
升级前的 Token 不回放、不入账、不展示。API 服务价格按服务和模型快速设置，保存后默认只重算当前日期范围；
页面不提供历史数据入口。

凭据会按产品约定以明文保存在本机。请勿上传、同步或分享以下目录和文件：

| 平台    | Codex Tools 数据位置                                                     |
| ------- | ------------------------------------------------------------------------ |
| Windows | 安装目录或可执行文件同级的 `data/app.yaml`                               |
| macOS   | `~/Library/Application Support/io.github.irasutoya.codex-tools/app.yaml` |

激活第三方账号后，API Key 还会写入 Codex 的 `auth.json`。Windows 便携目录结构如下：

```text
Codex Tools.exe
data/
  app.yaml
```

可以通过 `CODEX_TOOLS_DATA_DIR` 覆盖应用数据目录。macOS 默认将目录和配置文件
权限分别限制为当前用户可访问的 `0700` 和 `0600`。日志和前端诊断不会包含 API
Key 或 OAuth token。完整边界和漏洞报告方式见 [安全策略](SECURITY.md)。

## 本地开发

需要 Node.js 24、Rust 1.85+ 以及当前平台的
[Tauri 2 构建依赖](https://v2.tauri.app/start/prerequisites/)。Windows 构建还需要
WebView2 和 NSIS，macOS 构建需要 Xcode Command Line Tools。

```shell
git clone https://github.com/irasutoya/codex-tools.git
cd codex-tools
npm ci
npm run dev
```

提交前运行完整检查：

```shell
npm run check
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

本地打包：

```shell
# Windows x64：输出到 src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/
npm run dist:win

# macOS Apple Silicon：输出到 src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/
npm run dist:mac:arm64
```

发布流程、版本同步和标签要求见 [发布指南](docs/RELEASING.md)。参与开发前请阅读
[贡献指南](CONTRIBUTING.md)。

## 项目状态

项目仍处于早期预览阶段。升级前请阅读 [更新日志](CHANGELOG.md)，并在
[Issues](https://github.com/irasutoya/codex-tools/issues) 中报告可复现的问题。提交问题时
请移除 API Key、OAuth token、`auth.json`、`data/app.yaml` 和会话正文。

## 许可证

[MIT](LICENSE)
