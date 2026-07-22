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
配置，不启动本地代理、不转发请求，也不保存聊天正文。

## 主要功能

- 保存并切换多个 OpenAI Account，使用设备码完成授权。
- 管理多个兼容 OpenAI Responses API 的 Provider 和 API 账号。
- 只更新受管的 `custom` Provider 字段，保留 MCP、Skills、Hooks、沙箱及未知配置。
- 在官方账号与第三方 API 之间切换时，同步已识别会话的 Provider 元数据。
- 直接读取 Codex JSONL/SQLite；不创建应用数据库，不保存聊天正文。
- 支持 Windows 10/11 和 macOS 11+，macOS 发布包同时支持 Apple Silicon 与 Intel。

## 下载与安装

从 [Releases](https://github.com/irasutoya/codex-tools/releases/latest)
下载适合当前系统的安装包：

| 平台    | 发布产物           | 系统要求                |
| ------- | ------------------ | ----------------------- |
| Windows | NSIS `.exe` 安装包 | Windows 10/11、WebView2 |
| macOS   | 通用架构 `.dmg`    | macOS 11 或更高版本     |

> [!IMPORTANT]
> 当前自动构建的安装包尚未配置 Authenticode 或 Apple Developer ID
> 签名。系统可能显示“未知发布者”或阻止打开。请先核对 Release 来源和
> `SHA256SUMS.txt`；不确定时请从源码构建，不要关闭系统安全防护。

Codex Tools 面向已经安装并使用 Codex 的用户。启动后应用会自动寻找 Codex
配置目录和 CLI；也可以通过 `CODEX_HOME` 和 `CODEX_BIN` 环境变量指定位置。

## 快速开始

1. 启动 Codex Tools，先在设置页确认检测到的 Codex 路径。
2. 选择“OpenAI Account”并按设备码提示登录，或添加兼容 Responses API
   的 Provider、API 地址和 API Key。
3. 测试第三方连接后，选择要使用的账号并确认配置预览。
4. 应用切换，然后重新打开 Codex 使新配置生效。

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
# Windows：输出到 src-tauri/target/release/bundle/nsis/
npm run dist:win

# macOS：输出到 src-tauri/target/release/bundle/dmg/
npm run dist:mac
```

发布流程、版本同步和标签要求见 [发布指南](docs/RELEASING.md)。参与开发前请阅读
[贡献指南](CONTRIBUTING.md)。

## 项目状态

项目仍处于早期预览阶段。升级前请阅读 [更新日志](CHANGELOG.md)，并在
[Issues](https://github.com/irasutoya/codex-tools/issues) 中报告可复现的问题。提交问题时
请移除 API Key、OAuth token、`auth.json`、`data/app.yaml` 和会话正文。

## 许可证

[MIT](LICENSE)
