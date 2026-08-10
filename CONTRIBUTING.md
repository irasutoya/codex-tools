# Contributing to Codex Tools

感谢你帮助改进 Codex Tools。提交代码前，请先搜索现有 Issue，避免重复工作。较大的
功能或会改变配置格式、安全边界、认证流程的修改，建议先创建 Issue 说明方案。

## 开发环境

- Node.js 24 和 npm 11
- Rust 1.85 或更高版本
- 当前平台所需的 Tauri 2 构建依赖

安装依赖并启动开发环境：

```shell
npm ci
npm run dev
```

shadcn/ui 组件以源码形式保存在 `src/components/ui/`；`shadcn` 包作为
`shadcn/tailwind.css` 的构建期开发依赖保留。需要管理组件时使用项目约定的
`npx shadcn@latest` 命令，并在提交前审查生成的源码和依赖变更。

## 提交前检查

```shell
npm run check
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

涉及 Windows 或 macOS 平台行为时，请尽可能在对应平台完成一次安装包构建。不要提交
`node_modules/`、`dist/`、`src-tauri/target/`、本地 `data/` 或任何签名凭据。

## Pull Request

- 一个 Pull Request 聚焦一个主题，并说明用户可见的行为变化。
- 为新增或修复的逻辑补充测试；界面改动请附截图或录屏。
- 用户可见的变化写入 `CHANGELOG.md` 的 `Unreleased` 部分。
- 不要在日志、测试夹具、截图或提交历史中包含 API Key、OAuth token、Codex
  `auth.json`、`data/app.json`、`data/credentials.json` 或会话正文。
- 保持 `package.json`、`package-lock.json`、`src-tauri/Cargo.toml` 和
  `src-tauri/tauri.conf.json` 中的版本一致。

安全问题不要提交公开 Issue，请按 [安全策略](SECURITY.md) 私密报告。
