import { readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const read = (path) => readFileSync(resolve(root, path), "utf8")
const readJson = (path) => JSON.parse(read(path))

const semverPattern =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/

const escapeRegExp = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")

export const isValidSemver = (version) => semverPattern.test(version)

export function readCargoPackageVersion(toml) {
  const packageSection = toml.match(
    /^\[package\]\s*$([\s\S]*?)(?=^\[|(?![\s\S]))/m
  )?.[1]
  return packageSection?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1]
}

export function readCargoLockPackageVersion(lockfile, packageName) {
  const matchingPackages = lockfile
    .split(/^\[\[package\]\]\s*$/m)
    .slice(1)
    .filter((section) => {
      const name = section.match(/^name\s*=\s*"([^"]+)"\s*$/m)?.[1]
      return name === packageName && !/^source\s*=/m.test(section)
    })

  if (matchingPackages.length !== 1) return undefined
  return matchingPackages[0].match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1]
}

export function findVersionHeading(lines, version) {
  const heading = new RegExp(
    `^## \\[${escapeRegExp(version)}\\] - \\d{4}-\\d{2}-\\d{2}$`
  )
  return lines.find((line) => heading.test(line))
}

export function hasUnreleasedContent(changelog) {
  const lines = changelog.split(/\r?\n/)
  const start = lines.indexOf("## [Unreleased]")
  if (start === -1) throw new Error("CHANGELOG.md 缺少 Unreleased 部分")

  const end = lines.findIndex(
    (line, index) =>
      index > start &&
      (/^## \[[^\]]+\](?:\s|$)/.test(line) || /^\[[^\]]+\]:\s/.test(line))
  )
  return lines
    .slice(start + 1, end === -1 ? lines.length : end)
    .some((line) => line.trim() !== "")
}

export function checkVersions({ versions, expected, tag, changelog }) {
  if (!isValidSemver(expected)) {
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

  if (tag && tag !== `v${expected}`) {
    throw new Error(`发布标签 ${tag} 与项目版本 v${expected} 不一致`)
  }

  if (tag) {
    const changelogLines = changelog.split(/\r?\n/)
    if (!findVersionHeading(changelogLines, expected)) {
      throw new Error(
        `CHANGELOG.md 缺少“## [${expected}] - YYYY-MM-DD”版本标题`
      )
    }
    if (hasUnreleasedContent(changelog)) {
      throw new Error(
        "CHANGELOG.md 的 Unreleased 仍有内容，请先移动到当前发布版本"
      )
    }
  }
}

function main() {
  const packageJson = readJson("package.json")
  const packageLock = readJson("package-lock.json")
  const cargoToml = read("src-tauri/Cargo.toml")
  const cargoPackage = readCargoPackageVersion(cargoToml)

  if (!cargoPackage) {
    throw new Error("无法读取 src-tauri/Cargo.toml 的 [package] version")
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
  const expected = packageJson.version
  const versions = {
    "package.json": expected,
    "package-lock.json": packageLock.version,
    "package-lock.json (root package)": packageLock.packages?.[""]?.version,
    "src-tauri/Cargo.toml": cargoPackage,
    "src-tauri/Cargo.lock (root package)": readCargoLockPackageVersion(
      read("src-tauri/Cargo.lock"),
      packageJson.name
    ),
    "src-tauri/tauri.conf.json": readJson("src-tauri/tauri.conf.json").version,
  }

  checkVersions({
    versions,
    expected,
    tag,
    changelog: tag ? read("CHANGELOG.md") : "",
  })
  console.log(
    tag
      ? `版本检查通过：${expected}（标签 ${tag}）`
      : `版本检查通过：${expected}`
  )
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main()
}
