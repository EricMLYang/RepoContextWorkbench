import { useState, type ReactNode } from "react";
import { useDetail } from "../dialogs/Modal";

/**
 * 非同步按鈕：執行中鎖住；失敗把訊息放到對話框錯誤列（或 onError）。
 * 每個寫入動作都經過這裡（AGENTS.md：執行中鎖鈕、失敗顯示錯誤）。
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
  return (
    <button
      type="button"
      id={id}
      hidden={hidden}
      title={title}
      className={className}
      disabled={busy || disabled}
      onClick={async () => {
        if (busy) return;
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
    </button>
  );
}
