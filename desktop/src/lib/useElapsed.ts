import { useEffect, useState } from "react";

/**
 * 從 `active` 變 true 起算過了幾秒（每 `everyMs` 跳一次；active 變 false 就停、歸零）。
 * 慢動作的回饋（N-09）：按鈕、卡片、處理中都用這個，讓人知道不是卡死。
 */
export function useElapsed(active: boolean, everyMs = 1000): number {
  const [secs, setSecs] = useState(0);
  useEffect(() => {
    if (!active) return;
    const started = Date.now();
    const t = setInterval(() => setSecs(Math.floor((Date.now() - started) / 1000)), everyMs);
    return () => {
      clearInterval(t);
      setSecs(0);
    };
  }, [active, everyMs]);
  return active ? secs : 0;
}

/** 「· N 秒」／「· N 分」：超過 `after` 秒才顯示，否則空字串。 */
export function elapsedLabel(secs: number, after = 2): string {
  if (secs < after) return "";
  return secs >= 120 ? ` · ${Math.floor(secs / 60)} 分` : ` · ${secs} 秒`;
}
