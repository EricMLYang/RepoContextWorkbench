import { useEffect, useState, type ReactNode } from "react";
import { useDetail } from "../dialogs/Modal";

/** 超過這個秒數還沒回來，按鈕旁顯示已過秒數（N-09：慢動作要讓人知道還在跑）。 */
export const SLOW_AFTER_S = 2;

/**
 * 非同步按鈕：執行中鎖住；失敗把訊息放到對話框錯誤列（或 onError）。
 * 每個寫入動作都經過這裡（AGENTS.md：執行中鎖鈕、失敗顯示錯誤）。
 * 跑超過 SLOW_AFTER_S 秒就在文字後面顯示「· N 秒」，使用者才知道不是卡死。
 */
export function AsyncButton(props: {
  run: () => Promise<unknown>;
  children: ReactNode;
  className?: string;
  disabled?: boolean;
  onError?: (msg: string) => void;
  id?: string;
  hidden?: boolean;
  title?: string;
}) {
  const { run, children, className, disabled, onError, id, hidden, title } = props;
  const detail = useDetail();
  const [busy, setBusy] = useState(false);
  const [secs, setSecs] = useState(0);
  useEffect(() => {
    if (!busy) return;
    const t = setInterval(() => setSecs((n) => n + 1), 1000);
    return () => clearInterval(t);
  }, [busy]);
  return (
    <button
      type="button"
      id={id}
      hidden={hidden}
      title={title}
      className={className}
      disabled={busy || disabled}
      aria-busy={busy || undefined}
      onClick={async () => {
        if (busy) return;
        setSecs(0);
        setBusy(true);
        try {
          await run();
        } catch (e) {
          (onError ?? detail.setError)((e as Error).message);
        } finally {
          setBusy(false);
        }
      }}
    >
      {children}
      {busy && secs >= SLOW_AFTER_S ? (
        <span className="asyncsecs" aria-live="polite">
          {" · " + secs + " 秒"}
        </span>
      ) : null}
    </button>
  );
}
