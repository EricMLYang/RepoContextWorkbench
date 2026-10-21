import { useCallback, useState } from "react";
import type { WorktreeInfo } from "../../api/types";
import { useStore } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/** 一個 worktree 的狀態一句話。 */
export function worktreeState(w: WorktreeInfo): string {
  if (w.prunable) return "目錄已不在";
  const parts: string[] = [];
  if (w.dirty == null) parts.push("目錄已不在");
  else parts.push(w.dirty ? w.dirty + " 檔沒 commit" : "乾淨");
  if (w.unique) parts.push(w.unique + " 個 commit 只在這裡");
  if (w.locked) parts.push("鎖住");
  return parts.join(" · ");
}

/**
 * repo 掛出去的 git worktree（F-107）：agent 平行工作留下的，做完常沒人收（一個可以好幾 GB）。
 * 列分支、乾不乾淨、有沒有只在這裡的 commit；乾淨的才給「移除」（git worktree remove，分支留著）。
 * 按了才讀（每個 worktree 都要跑 git status），不在背景掃。
 */
export function RepoWorktrees({ id }: { id: string }) {
  const store = useStore();
  const [list, setList] = useState<WorktreeInfo[] | null>(null);
  const [error, setError] = useState("");
  const [armed, setArmed] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      const r = await store.action("repo_worktrees", { id });
      setList(r.worktrees);
      setError("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [store, id]);
  const extra = (list || []).filter((w) => !w.main);
  return (
    <details
      className="panelworktrees"
      onToggle={(e) => {
        if ((e.currentTarget as HTMLDetailsElement).open && !list) void load();
      }}
    >
      <summary className="small">Worktree{list ? "（" + extra.length + "）" : ""}</summary>
      {error ? <p className="error small">{"讀不到 worktree：" + error}</p> : null}
      {list && !extra.length ? <p className="muted small">沒有掛出去的 worktree。</p> : null}
      <ul className="wtlist">
        {extra.map((w) => (
          <li key={w.path} className="wtrow">
            <div className="wthead">
              <span className="wtbranch">{w.branch || "（detached " + w.head + "）"}</span>
              <span className={"small" + (w.removable ? " muted" : " warn")}>
                {worktreeState(w)}
              </span>
            </div>
            <div className="small muted wtpath" title={w.path}>
              {w.path}
            </div>
            {w.removable ? (
              armed === w.path ? (
                <span className="wtconfirm">
                  <AsyncButton
                    className="small danger"
                    onError={(m) => store.toast(m)}
                    run={async () => {
                      await store.action("worktree_remove", { id, path: w.path });
                      store.toast("已移除 worktree（分支 " + (w.branch || "—") + " 還在）");
                      setArmed(null);
                      await load();
                    }}
                  >
                    確定移除
                  </AsyncButton>
                  <button type="button" className="ghost small" onClick={() => setArmed(null)}>
                    取消
                  </button>
                </span>
              ) : (
                <button
                  type="button"
                  className="ghost small"
                  title="git worktree remove（不加 --force）：刪掉這個目錄，分支留著"
                  onClick={() => setArmed(w.path)}
                >
                  移除…
                </button>
              )
            ) : null}
          </li>
        ))}
      </ul>
      {list ? (
        <button type="button" className="ghost small" onClick={() => void load()}>
          重新讀取
        </button>
      ) : null}
    </details>
  );
}
