import { readFileSync, writeFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath, pathToFileURL } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const escapeRegExp = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")

export function extractReleaseNotes(changelog, version) {
  const lines = changelog.split(/\r?\n/)
  const heading = new RegExp(
    `^## \\[${escapeRegExp(version)}\\] - \\d{4}-\\d{2}-\\d{2}$`
  )
  const start = lines.findIndex((line) => heading.test(line))

  if (start === -1) {
    throw new Error(`CHANGELOG.md 中找不到版本 ${version}`)
  }

  const nextSection = lines.findIndex(
    (line, index) => index > start && /^## \[[^\]]+\](?:\s|$)/.test(line)
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
  return `${notes}\n`
}

function main() {
  const optionIndex = process.argv.indexOf("--version")
  const rawVersion =
    optionIndex === -1
      ? process.env.GITHUB_REF_NAME
      : process.argv[optionIndex + 1]

  if (!rawVersion) {
    throw new Error("请通过 --version x.y.z 或 GITHUB_REF_NAME 提供发布版本")
  }

  const version = rawVersion.replace(/^v/, "")
  const changelog = readFileSync(resolve(root, "CHANGELOG.md"), "utf8")
  const output = resolve(root, "release-notes.md")
  writeFileSync(output, extractReleaseNotes(changelog, version), "utf8")
  console.log(`已生成 ${output}`)
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main()
}
