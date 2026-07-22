# Release guide

此清单用于维护者发布 Windows NSIS 安装包和 macOS 通用架构 DMG。

## 1. 准备版本

1. 确认 `main` 分支的 CI 全部通过。
2. 将 `CHANGELOG.md` 中准备发布的内容从 `Unreleased` 移到
   `## [x.y.z] - YYYY-MM-DD`。
3. 同步以下文件中的版本号：
   - `package.json`（运行 `npm install --package-lock-only` 同步锁文件）
   - `src-tauri/Cargo.toml`（运行 Cargo 命令同步锁文件）
   - `src-tauri/tauri.conf.json`
4. 运行 `npm run version:check -- --tag vX.Y.Z`，确认所有版本一致，且
   `Unreleased` 已清空。

## 2. 本地验证

```shell
npm ci
npm run check
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

如有对应平台环境，再运行 `npm run dist:win` 或 `npm run dist:mac` 并实际启动产物。

## 3. 创建发布

合并版本提交后，在该提交创建与版本完全一致的标签：

```shell
git tag -a vX.Y.Z -m "Codex Tools vX.Y.Z"
git push origin vX.Y.Z
```

标签会触发 Release workflow。工作流会再次校验版本，分别构建 Windows 和 macOS
安装包，生成 `SHA256SUMS.txt`，并创建草稿预发布。检查以下项目后再手动发布草稿：

- Windows `.exe` 和 macOS `.dmg` 均已上传。
- `SHA256SUMS.txt` 包含所有安装包且哈希可复算。
- Release notes 与 `CHANGELOG.md` 对应版本一致。
- 在干净机器或虚拟机中完成基本安装、启动、账号切换和卸载检查。
- 已明确说明签名/公证状态及可能出现的系统安全提示。

正式稳定后，可以在 `.github/workflows/release.yml` 中取消固定的预发布标记。
