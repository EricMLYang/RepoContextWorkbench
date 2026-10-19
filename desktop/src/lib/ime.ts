/**
 * 中文／日文輸入法選字按 Enter 的誤判。
 *
 * 所有輸入框本來就檢查 `isComposing`，但 WebKit（Safari、Tauri 的 WKWebView）選字按 Enter 時會先送
 * `compositionend`、再補送一個 `isComposing=false` 的 Enter keydown——跟真的按 Enter 一模一樣，
 * 編輯器就把它當換行（清單延續／保留縮排）或送出處理。Chrome／Firefox 的 Enter keydown 在
 * compositionend 之前、且 isComposing=true，沒這問題。
 *
 * 做法跟 ProseMirror 一樣：記住最近一次 compositionend 的時間，之後 500ms 內的第一個 Enter 視為
 * 「確認選字」略過（只略過一次；再按一次 Enter 就是真的換行）。只在 WebKit 生效，
 * 免得 Chrome 下快速連按 Enter 被吃掉一次。
 *
 * 用法：keydown 開頭 `if (imeKey(e)) return;`（取代 `if (e.nativeEvent.isComposing) return;`）。
 */

const WINDOW_MS = 500;
let endedAt = -1e9;

/** 跟 ProseMirror 的 browser.safari 同義：WebKit 但不是 Chrome 系。測試可改。 */
export const imeGuard = {
  webkit:
    typeof navigator !== "undefined" &&
    /AppleWebKit/.test(navigator.userAgent) &&
    !/Chrome|Chromium|Edg\//.test(navigator.userAgent),
};

if (typeof window !== "undefined") {
  window.addEventListener(
    "compositionend",
    (e) => {
      endedAt = e.timeStamp;
    },
    true,
  );
}

type KeyLike = { key: string; timeStamp: number; nativeEvent?: { isComposing?: boolean } };

/** 這個 keydown 是輸入法在用的（組字中、或 WebKit 選字確認的那個 Enter）就回 true，呼叫端直接 return。 */
export function imeKey(e: KeyLike): boolean {
  const native = e.nativeEvent ?? (e as { isComposing?: boolean });
  if (native.isComposing) return true;
  if (imeGuard.webkit && e.key === "Enter" && Math.abs(e.timeStamp - endedAt) < WINDOW_MS) {
    endedAt = -1e9;
    return true;
  }
  return false;
}
