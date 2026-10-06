import type { Backend } from "./backend";
import { createHttpBackend } from "./http";
import { createTauriBackend, isTauri } from "./tauri";

export type { Backend, TermStream } from "./backend";
export * from "./types";

/** 執行時選實作：在 Tauri WebView 裡（有 `__TAURI_INTERNALS__`）走 IPC，否則打 HTTP。 */
export function detectBackend(): Backend {
  return isTauri() ? createTauriBackend() : createHttpBackend();
}
