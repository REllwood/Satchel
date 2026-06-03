import { invoke } from "@tauri-apps/api/core";

/** True when running inside the Tauri webview (vs. a plain browser dev preview). */
export const isTauri =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * Central place for all Rust command calls. Keeping IPC in one module keeps the
 * UI free of stringly-typed `invoke` calls and gives us one seam to mock/test.
 */
export async function coreVersion(): Promise<string> {
  if (!isTauri) return "dev";
  return invoke<string>("core_version");
}
