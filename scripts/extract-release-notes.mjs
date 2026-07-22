import { readFileSync, writeFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const optionIndex = process.argv.indexOf("--version")
const rawVersion =
  optionIndex === -1
    ? process.env.GITHUB_REF_NAME
    : process.argv[optionIndex + 1]

if (!rawVersion) {
  throw new Error("请通过 --version x.y.z 或 GITHUB_REF_NAME 提供发布版本")
}

const version = rawVersion.replace(/^v/, "")
const lines = readFileSync(resolve(root, "CHANGELOG.md"), "utf8").split(/\r?\n/)
const start = lines.findIndex((line) => line.startsWith(`## [${version}]`))

if (start === -1) {
  throw new Error(`CHANGELOG.md 中找不到版本 ${version}`)
}

const nextSection = lines.findIndex(
  (line, index) => index > start && line.startsWith("## [")
)
const references = lines.findIndex(
  (line, index) => index > start && /^\[[^\]]+\]:\s/.test(line)
)
const boundaries = [nextSection, references].filter((index) => index !== -1)
const end = boundaries.length > 0 ? Math.min(...boundaries) : lines.length
const notes = lines
  .slice(start + 1, end)
  .join("\n")
  .trim()

if (!notes) {
  throw new Error(`CHANGELOG.md 中的版本 ${version} 没有发布说明`)
}

const output = resolve(root, "release-notes.md")
writeFileSync(output, `${notes}\n`, "utf8")
console.log(`已生成 ${output}`)
