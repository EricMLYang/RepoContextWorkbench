/** 頂列、頁尾、toast——外框元件。 */
import { useEffect, useState } from "react";
import { connectionText, scopeLabel } from "../lib/view";
import { useSnapshot, useStore } from "../store/context";

export function TopBar({ onHome }: { onHome: () => void }) {
  const store = useStore();
  const snap = useSnapshot();
  const err = snap.stateError || snap.scanError;
  return (
    <header id="top">
      <a
        className="brand"
        href="#"
        aria-label="Repo 工作台首頁"
        onClick={(e) => {
          e.preventDefault();
          onHome();
        }}
      >
        <span className="brandmark">
          r<span>›</span>
        </span>{" "}
        repo<span className="brandtail">工作台</span>
      </a>
      <span className="top-divider" />
      <span className="muted">個人工作空間</span>
      <span className="spacer" />
      <span id="connection" role="status" className={err ? "offline" : ""}>
        {connectionText(snap)}
      </span>
      <button
        type="button"
        id="rescan"
        className="ghost"
        disabled={snap.scanLoading}
        onClick={() => void store.refreshAll()}
      >
        更新狀態
      </button>
    </header>
  );
}

export function StatusBar() {
  const { scope, scanAt, state } = useSnapshot();
  const alive = (state.terms || []).filter((s) => s.alive).length;
  return (
    <footer id="status">
      <span id="scanscope">{"範圍：" + scopeLabel(scope)}</span>
      <span id="scanwhen">{scanAt ? "上次成功更新 " + scanAt : ""}</span>
      <span className="spacer" />
      <span id="sesscount">{alive + " 個執行中會話"}</span>
      <span>Ctrl + ` 終端</span>
    </footer>
  );
}

export function Toast() {
  const { toast } = useSnapshot();
  // 每則 toast 顯示 3.8 秒；新的一則重新計時
  const [expired, setExpired] = useState(0);
  useEffect(() => {
    if (!toast.seq) return;
    const t = setTimeout(() => setExpired(toast.seq), 3800);
    return () => clearTimeout(t);
  }, [toast.seq]);
  const visible = toast.seq > 0 && expired !== toast.seq;
  return (
    <div id="toast" role="status" aria-live="polite" style={{ opacity: visible ? 1 : 0 }}>
      {toast.text}
    </div>
  );
}
