/** 頂列、頁尾、toast——外框元件。 */
import { useEffect, useState } from "react";
import type { Usage, UsageWindow } from "../api/types";
import { connectionText, scopeLabel } from "../lib/view";
import { useSnapshot, useStore, useUi } from "../store/context";

export function TopBar({ onHome }: { onHome: () => void }) {
  const store = useStore();
  const ui = useUi();
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
      <button
        type="button"
        className="ghost palettebtn"
        title="找 repo、牌組、檔案、會話，或輸入 > 執行指令"
        onClick={ui.openPalette}
      >
        搜尋或執行指令 <kbd>⌘K</kbd>
      </button>
      <span id="connection" role="status" className={err ? "offline" : ""}>
        {connectionText(snap)}
      </span>
      <button
        type="button"
        id="rescan"
        className="ghost"
        disabled={snap.scanLoading}
        title="重新跑 git 採集目前範圍的 repo 狀態（平常 agent 做完一輪、檔案有變都會自己更新）"
        onClick={() => void store.refreshAll()}
      >
        {snap.scanLoading ? "採集中…" : "重新採集"}
      </button>
    </header>
  );
}

/** 幾分鐘 → 「45 分」「2.5 hr」「3.2 天」（10 以上不帶小數）。 */
export function spanText(min: number): string {
  const one = (n: number) => (n < 10 ? String(Math.round(n * 10) / 10) : String(Math.round(n)));
  if (min < 60) return Math.max(0, Math.round(min)) + " 分";
  if (min < 24 * 60) return one(min / 60) + " hr";
  return one(min / (24 * 60)) + " 天";
}

/** 「2.5 hr」後面接中文要空一格，「45 分」不用。 */
const then = (span: string, word: string) => (/[a-z]$/i.test(span) ? span + " " : span) + word;

/** 超過這麼久沒更新就變淡、標「N 前」（數字可能已經不是現在的）。 */
export const USAGE_STALE_MIN = 30;

/**
 * 額度餘光一格（F-83）：`Claude 5h 62%（2.5 hr 後重置）· 週 31%（3.2 天後重置）`；舊資料沒有剩餘時間就寫時刻。
 * 很久沒更新的尾巴加「· 45 分前」。沒收過就空字串。
 */
export function usageText(label: string, u: Usage | null | undefined): string {
  if (!u) return "";
  const win = (name: string, w: UsageWindow | null) =>
    w
      ? `${name} ${w.used}%` +
        (typeof w.resets_in_min === "number"
          ? `（${then(spanText(w.resets_in_min), "後重置")}）`
          : w.resets
            ? `（${w.resets} 重置）`
            : "")
      : "";
  const parts = [win("5h", u.five_hour), win("週", u.seven_day)].filter(Boolean);
  if (!parts.length) return "";
  const stale =
    typeof u.age_min === "number" && u.age_min >= USAGE_STALE_MIN
      ? " · " + then(spanText(u.age_min), "前")
      : "";
  return label + " " + parts.join(" · ") + stale;
}

function usageClass(u: Usage): string | undefined {
  const c = [
    u.warn ? "usagewarn" : "",
    typeof u.age_min === "number" && u.age_min >= USAGE_STALE_MIN ? "usagestale" : "",
  ].filter(Boolean);
  return c.length ? c.join(" ") : undefined;
}

export function StatusBar() {
  const { scope, scanAt, state } = useSnapshot();
  const alive = (state.terms || []).filter((s) => s.alive).length;
  const usage = usageText("Claude", state.usage);
  const codex = usageText("Codex", state.codex_usage);
  return (
    <footer id="status">
      <span id="scanscope">{"範圍：" + scopeLabel(scope)}</span>
      <span id="scanwhen">{scanAt ? "上次成功更新 " + scanAt : ""}</span>
      <span className="spacer" />
      {usage ? (
        <span
          id="usage"
          className={usageClass(state.usage!)}
          title={
            "Claude 訂閱額度：整個帳號的用量（claude.ai、其他終端用掉的也算），只在 App 開的 Claude Code 會話跑完一輪時更新；" +
            "最後更新 " +
            state.usage!.at +
            (state.usage!.model ? "（模型 " + state.usage!.model + "）" : "") +
            "。App 背景的 claude -p（碰撞判定、ask_repo、助手）用訂閱登入時也吃這份額度，次數見活動分頁。"
          }
        >
          {usage}
        </span>
      ) : null}
      {codex ? (
        <span
          id="codexusage"
          className={usageClass(state.codex_usage!)}
          title={
            "Codex 額度" +
            (state.codex_usage!.plan ? "（" + state.codex_usage!.plan + "）" : "") +
            "：讀這台機器上 Codex 的會話紀錄，任何 Codex 會話跑一輪都會更新；最後更新 " +
            state.codex_usage!.at +
            "。"
          }
        >
          {codex}
        </span>
      ) : null}
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
