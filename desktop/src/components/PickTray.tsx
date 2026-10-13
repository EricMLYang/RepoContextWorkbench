import { useState } from "react";
import { useStore, useUi, useUiState } from "../store/context";

/**
 * 挑好的檔案（痛點 2：挑幾份文件一次交給 agent）。在檔案樹 ⌘／Ctrl＋點、或右鍵「加入挑選」。
 * 任務文字與 `@路徑` 由引擎組（`pick_brief`：檢查每個檔都還在 repo 裡、依 repo 分段並附名片）；這裡只畫與轉交。
 */
export function PickTray() {
  const store = useStore();
  const ui = useUi();
  const { picks } = useUiState();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [open, setOpen] = useState(false);

  async function brief() {
    return store.action("pick_brief", {
      files: picks.map((p) => ({ id: p.repo, path: p.path })),
    });
  }

  async function run(f: () => Promise<void>) {
    if (busy) return;
    setBusy(true);
    setError("");
    try {
      await f();
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  }

  const toAgent = () =>
    run(async () => {
      const r = await brief();
      const one = r.repos.length === 1 ? r.repos[0] : null;
      ui.openSession({
        ...(one ? { repo: one, repos: [one] } : { group: null, repos: r.repos }),
        task: r.text,
        origin: "pick:" + r.count,
      });
    });

  const copyMentions = () =>
    run(async () => {
      const r = await brief();
      try {
        await navigator.clipboard.writeText(r.mentions);
      } catch {
        throw new Error("無法複製到剪貼簿");
      }
      store.toast("已複製 " + r.count + " 個 @路徑（貼到 agent 的對話框）");
    });

  return (
    <section id="picktray" aria-label="挑好的檔案">
      <div className="pickbar">
        <button
          type="button"
          className="ghost small"
          aria-expanded={open}
          onClick={() => setOpen((o) => !o)}
        >
          已挑 {picks.length} 個檔案 {open ? "▾" : "▸"}
        </button>
        <span className="spacer" />
        <button type="button" className="small" disabled={busy} onClick={() => void copyMentions()}>
          複製 @路徑
        </button>
        <button
          type="button"
          className="small primary"
          disabled={busy}
          onClick={() => void toAgent()}
        >
          交給 Agent…
        </button>
        <button type="button" className="ghost small" disabled={busy} onClick={ui.clearPicks}>
          清空
        </button>
      </div>
      {error ? (
        <p className="error small" role="alert">
          {error}
        </p>
      ) : null}
      {open ? (
        <ul className="picklist">
          {picks.map((p) => (
            <li key={p.repo + ":" + p.path}>
              <span className="small muted">{p.repo}</span>
              <span className="pickpath" title={p.path}>
                {p.path}
              </span>
              <button
                type="button"
                className="icon ghost"
                aria-label={"不挑 " + p.repo + " 的 " + p.path}
                onClick={() => ui.togglePick(p)}
              >
                ×
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
