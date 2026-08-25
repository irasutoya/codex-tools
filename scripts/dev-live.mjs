import { spawn } from "node:child_process"
import { mkdirSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"

if (process.platform !== "win32") {
  throw new Error("dev:live 仅支持 Windows。\n")
}

const localAppData = process.env.LOCALAPPDATA
if (!localAppData) {
  throw new Error("未设置 LOCALAPPDATA，无法确定只读基底目录。\n")
}

const scriptDirectory = dirname(fileURLToPath(import.meta.url))
const repositoryRoot = resolve(scriptDirectory, "..")
const overlayRoot = resolve(
  repositoryRoot,
  "build",
  "live-data-overlay-20260825"
)
const codexHome = resolve(overlayRoot, "codex-home")
const configuredReadRoot = process.env.CODEX_TOOLS_READ_DATA_DIR
const readRoot = configuredReadRoot
  ? resolve(configuredReadRoot)
  : resolve(localAppData, "Codex Tools", "data")

// 已存在的覆盖层属于当前用户，绝不清空、复制或覆盖其中的数据。
mkdirSync(overlayRoot, { recursive: true })
mkdirSync(codexHome, { recursive: true })

const environment = {
  ...process.env,
  CODEX_HOME: codexHome,
  CODEX_TOOLS_DATA_DIR: overlayRoot,
  CODEX_TOOLS_READ_DATA_DIR: readRoot,
}
delete environment.CODEX_TOOLS_DEV_DEMO

const child = spawn("npm.cmd", ["run", "dev"], {
  cwd: repositoryRoot,
  env: environment,
  stdio: "inherit",
})

child.once("error", (error) => {
  console.error(`无法启动开发版：${error.message}`)
  process.exitCode = 1
})
child.once("exit", (code) => {
  process.exitCode = code ?? 1
})
