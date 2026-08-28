import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager"

export async function readClipboardText() {
  return (await readText()) ?? ""
}

export async function writeClipboardText(text: string) {
  await writeText(text)
}
