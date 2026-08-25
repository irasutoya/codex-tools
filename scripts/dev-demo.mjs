import { spawn } from "node:child_process"
import { existsSync, readFileSync } from "node:fs"
import { createRequire } from "node:module"
import { fileURLToPath } from "node:url"
import path from "node:path"

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const dataDir = path.join(root, "build", "dev-data")
const require = createRequire(path.join(root, "package.json"))

export function demoEnvironment(base = process.env) {
  return {
    ...base,
    CODEX_TOOLS_DATA_DIR: dataDir,
    CODEX_TOOLS_DEV_DEMO: "1",
    CODEX_HOME: path.join(dataDir, "codex-home"),
  }
}

export function resolveTauriEntry() {
  const packagePath = require.resolve("@tauri-apps/cli/package.json")
  const packageJson = JSON.parse(readFileSync(packagePath, "utf8"))
  const bin =
    typeof packageJson.bin === "string" ? packageJson.bin : packageJson.bin?.tauri
  if (typeof bin !== "string" || !bin) {
    throw new Error("当前项目的 @tauri-apps/cli 未声明 tauri bin 入口。")
  }
  const entry = path.resolve(path.dirname(packagePath), bin)
  if (!existsSync(entry)) {
    throw new Error(`当前项目的 Tauri CLI 入口不存在：${entry}`)
  }
  return entry
}

if (process.argv.includes("--print-config")) {
  console.log(
    JSON.stringify({
      dataDir,
      codexHome: path.join(dataDir, "codex-home"),
      demoFlag: "1",
    })
  )
} else if (process.argv.includes("--check-entry")) {
  console.log(
    JSON.stringify({
      node: process.execPath,
      tauriEntry: resolveTauriEntry(),
    })
  )
} else {
  const child = spawn(process.execPath, [resolveTauriEntry(), "dev"], {
    cwd: root,
    env: demoEnvironment(),
    stdio: "inherit",
  })

  child.on("error", (error) => {
    console.error(`无法启动 Tauri 开发模式：${error.message}`)
    process.exitCode = 1
  })
  child.on("exit", (code, signal) => {
    if (signal) {
      process.exitCode = 1
    } else {
      process.exitCode = code ?? 1
    }
  })
}
