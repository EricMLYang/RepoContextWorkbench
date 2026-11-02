import type { WorkbenchStore } from "../store/store";

/** 「在 Finder 中顯示」（Windows：檔案總管）——選單文字跟著平台。 */
export const REVEAL_LABEL = /win/i.test(navigator.platform)
  ? "在檔案總管中顯示"
  : "在 Finder 中顯示";
export const OPEN_EXTERNAL_LABEL = "用預設 App 開啟";

/**
 * 在這個資料夾開一個 Shell（VS Code 的 Open in Integrated Terminal；R38）：開好就切過去。失敗用 toast 說。
 */
export async function openShellAt(
  store: WorkbenchStore,
  selectTerm: (sid: string) => void,
  cwd: string,
  origin: string,
): Promise<void> {
  try {
    const r = await store.action("term_create", { kind: "shell", cwd, origin });
    await store.refreshState();
    selectTerm(r.sid);
  } catch (e) {
    store.toast((e as Error).message);
  }
}

/**
 * 交給系統（F-115）：在 Finder 中顯示（`path` 空字串＝repo 根目錄）或用預設 App 開啟。
 * 引擎擋掉資料夾、會直接執行的檔與跳出 repo 的路徑；失敗用 toast 說原因。
 */
export async function openInSystem(
  store: WorkbenchStore,
  id: string,
  path: string,
  how: "reveal" | "open",
): Promise<void> {
  try {
    await store.action("open_entry", { id, path, how });
  } catch (e) {
    store.toast((e as Error).message);
  }
}
