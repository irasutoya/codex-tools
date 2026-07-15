import { invoke } from "@tauri-apps/api/core"

export async function call<T>(
  command: string,
  args?: Record<string, unknown>
): Promise<T> {
  try {
    return await invoke<T>(command, args)
  } catch (error) {
    if (error instanceof Error) throw error
    throw new Error(typeof error === "string" ? error : String(error), {
      cause: error,
    })
  }
}
