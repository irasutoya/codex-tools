import { readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const read = (path) => readFileSync(resolve(root, path), "utf8")
const readJson = (path) => JSON.parse(read(path))

const packageJson = readJson("package.json")
const packageLock = readJson("package-lock.json")
const tauriConfig = readJson("src-tauri/tauri.conf.json")
const cargoPackage = read("src-tauri/Cargo.toml").match(
  /\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m
)

if (!cargoPackage) {
  throw new Error("无法读取 src-tauri/Cargo.toml 的 [package] version")
}

const versions = {
  "package.json": packageJson.version,
  "package-lock.json": packageLock.version,
  "package-lock.json (root package)": packageLock.packages?.[""]?.version,
  "src-tauri/Cargo.toml": cargoPackage[1],
  "src-tauri/tauri.conf.json": tauriConfig.version,
}

const expected = packageJson.version
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(expected)) {
  throw new Error(`package.json 中的版本不是有效的 SemVer：${expected}`)
}

const mismatches = Object.entries(versions).filter(
  ([, version]) => version !== expected
)
if (mismatches.length > 0) {
  const details = mismatches
    .map(([file, version]) => `- ${file}: ${version ?? "<missing>"}`)
    .join("\n")
  throw new Error(`版本不一致，期望 ${expected}：\n${details}`)
}

const optionIndex = process.argv.indexOf("--tag")
if (optionIndex !== -1 && !process.argv[optionIndex + 1]) {
  throw new Error("--tag 后必须提供标签")
}

const tag =
  optionIndex === -1
    ? process.env.GITHUB_REF_TYPE === "tag"
      ? process.env.GITHUB_REF_NAME
      : undefined
    : process.argv[optionIndex + 1]

if (tag && tag !== `v${expected}`) {
  throw new Error(`发布标签 ${tag} 与项目版本 v${expected} 不一致`)
}

if (tag) {
  const changelogLines = read("CHANGELOG.md").split(/\r?\n/)
  const versionHeading = changelogLines.find((line) =>
    line.startsWith(`## [${expected}]`)
  )
  const expectedHeading = new RegExp(
    `^## \\[${expected.replaceAll(".", "\\.")}\\] - \\d{4}-\\d{2}-\\d{2}$`
  )

  if (!versionHeading || !expectedHeading.test(versionHeading)) {
    throw new Error(`CHANGELOG.md 缺少“## [${expected}] - YYYY-MM-DD”版本标题`)
  }

  const unreleasedStart = changelogLines.indexOf("## [Unreleased]")
  if (unreleasedStart === -1) {
    throw new Error("CHANGELOG.md 缺少 Unreleased 部分")
  }

  const nextVersion = changelogLines.findIndex(
    (line, index) => index > unreleasedStart && line.startsWith("## [")
  )
  const unreleasedLines = changelogLines.slice(
    unreleasedStart + 1,
    nextVersion === -1 ? changelogLines.length : nextVersion
  )

  if (unreleasedLines.some((line) => line.trimStart().startsWith("- "))) {
    throw new Error(
      "CHANGELOG.md 的 Unreleased 仍有内容，请先移动到当前发布版本"
    )
  }
}

console.log(
  tag ? `版本检查通过：${expected}（标签 ${tag}）` : `版本检查通过：${expected}`
)
