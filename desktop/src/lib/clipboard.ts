import type { Backend } from "../api/backend";

export const MSG_COPY_FAILED = "無法複製到剪貼簿";

/**
 * 寫系統剪貼簿（檔案樹、repo、筆記、卡片…所有「複製」都走這裡）。
 *
 * Tauri 下走殼層命令 `copy_text`，不用 WebView 的 `navigator.clipboard`：macOS 的 WKWebView 只在
 * WebView 是目前焦點時才放行那個 API，焦點剛從終端、選單列或別的視窗回來時會直接拒絕，
 * 症狀就是偶爾「無法複製到剪貼簿」、過一下又好了。瀏覽器／測試沒有殼層就退回 Web API。
 * 失敗 reject `Error(MSG_COPY_FAILED)`，由呼叫端決定用 toast 還是表單錯誤顯示。
 */
export async function copyText(backend: Pick<Backend, "copyText">, text: string): Promise<void> {
  if (backend.copyText) {
    try {
      await backend.copyText(text);
      return;
    } catch {
      // 殼層失敗（極少）再給 Web API 一次機會
    }
  }
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    throw new Error(MSG_COPY_FAILED);
  }
}
