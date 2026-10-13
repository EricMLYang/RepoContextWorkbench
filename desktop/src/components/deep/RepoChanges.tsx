import { useEffect, useState } from "react";
import type { RepoChange } from "../../api/types";
import { useSnapshot, useStore, useUi, useUiState } from "../../store/context";

/** 一次最多列幾個（其餘寫數量；全部看 git 或終端）。 */
const SHOWN = 30;

const MARK: Record<RepoChange["status"], [string, string]> = {
  modified: ["M", "修改"],
  added: ["A", "新增（已暫存）"],
  deleted: ["D", "刪除"],
  renamed: ["R", "改名"],
  untracked: ["U", "新檔（還沒追蹤）"],
  conflicted: ["!", "合併衝突"],
};

/**
 * 右側欄「變更」：還沒 commit 的檔（agent 剛改了什麼）。點了在主區開「差異」。
 * state 一變就重讀（agent 做完一輪會寫會話狀態 → state 更新），另有 ⟳；讀失敗保留上次的並顯示原因。
 * 不是 git repo 就整段不顯示。
 */
export function RepoChanges({ id }: { id: string }) {
  const store = useStore();
  const ui = useUi();
  const view = useUiState();
  const { state, lastStates } = useSnapshot();
  const [list, setList] = useState<{ id: string; changes: RepoChange[]; more: number } | null>(
    null,
  );
  const [error, setError] = useState("");
  const [notGit, setNotGit] = useState(false);
  const [nonce, setNonce] = useState(0);
  const dirty = lastStates[id]?.dirty;

  useEffect(() => {
    let live = true;
    store
      .action("repo_changes", { id })
      .then((r) => {
        if (!live) return;
        setList({ id, changes: r.changes ?? [], more: r.more ?? 0 });
        setError("");
        setNotGit(false);
      })
      .catch((e: Error) => {
        if (!live) return;
        if (e.message.includes("不是 git repo")) setNotGit(true);
        else setError(e.message);
      });
    return () => {
      live = false;
    };
  }, [store, id, state, dirty, nonce]);

  if (notGit) return null;
  const shown = list?.id === id ? list : null;
  const active = view.activeFile?.repo === id ? view.activeFile.path : null;
  return (
    <section className="panelsection changes" aria-label={id + " 還沒 commit 的變更"}>
      <div className="profilehead">
        <p className="eyebrow">
          變更{shown ? "（" + (shown.changes.length + shown.more) + "）" : ""}
        </p>
        <span className="spacer" />
        <button
          type="button"
          className="icon ghost"
          aria-label={"重新讀取 " + id + " 的變更"}
          title="重新讀取"
          onClick={() => setNonce((n) => n + 1)}
        >
          ⟳
        </button>
      </div>
      {error ? (
        <p className="error small" role="alert">
          讀不到變更：{error}
        </p>
      ) : null}
      {!shown ? (
        error ? null : (
          <p className="muted small panelnote">正在讀變更…</p>
        )
      ) : !shown.changes.length ? (
        <p className="muted small panelnote">沒有還沒 commit 的變更。</p>
      ) : (
        <ul className="changelist">
          {shown.changes.slice(0, SHOWN).map((c) => (
            <li key={c.path}>
              <button
                type="button"
                className={"ghost" + (c.path === active ? " on" : "")}
                title={MARK[c.status][1] + "：" + c.path + "（點了看差異）"}
                onClick={() => ui.openFile(id, c.path, { diff: true })}
              >
                <span className={"chmark " + c.status} aria-label={MARK[c.status][1]}>
                  {MARK[c.status][0]}
                </span>
                <span className="chpath">{c.path}</span>
              </button>
            </li>
          ))}
          {shown.changes.length + shown.more > SHOWN ? (
            <li className="muted small">…另 {shown.changes.length + shown.more - SHOWN} 個</li>
          ) : null}
        </ul>
      )}
    </section>
  );
}
