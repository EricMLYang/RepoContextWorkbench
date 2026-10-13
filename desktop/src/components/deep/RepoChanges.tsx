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
        <ChangeList id={id} changes={shown.changes} more={shown.more} active={active} />
      )}
    </section>
  );
}

/** 一個 repo 的變更清單（右側欄與檔案分頁共用）：點了在主區開差異。 */
export function ChangeList(props: {
  id: string;
  changes: RepoChange[];
  more: number;
  active: string | null;
}) {
  const { id, changes, more, active } = props;
  const ui = useUi();
  return (
    <ul className="changelist">
      {changes.slice(0, SHOWN).map((c) => (
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
      {changes.length + more > SHOWN ? (
        <li className="muted small">…另 {changes.length + more - SHOWN} 個</li>
      ) : null}
    </ul>
  );
}

/**
 * 檔案分頁上方「這組的變更」：範圍內每個有變更的 repo 一段（多個 agent 同時在不同 repo 改東西時，一眼看出誰動了什麼）。
 * 引擎一次平行查完（`scope_changes`）；分頁看得到時、state 一變就重讀；沒有任何變更就整段不顯示。
 * 讀失敗：讀成功過才顯示原因（保留上次的清單），從沒成功過就不顯示。
 */
export function ScopeChanges({ active }: { active: boolean }) {
  const store = useStore();
  const view = useUiState();
  const { state } = useSnapshot();
  const [rows, setRows] = useState<
    { id: string; changes: RepoChange[]; more: number; error?: string }[] | null
  >(null);
  const [error, setError] = useState("");
  const [nonce, setNonce] = useState(0);
  const key = JSON.stringify(store.scopePayload);

  useEffect(() => {
    if (!active) return;
    let live = true;
    store
      .action("scope_changes", JSON.parse(key) as Record<string, unknown>)
      .then((r) => {
        if (!live) return;
        setRows(r.repos ?? []);
        setError("");
      })
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [store, key, active, state, nonce]);

  // 輔助的總覽：從沒讀成功過（例如舊引擎沒有這個 action）就安靜不顯示；讀過之後失敗才保留舊的並顯示原因
  if (rows === null || (!error && !rows.length)) return null;
  const total = (rows || []).reduce((n, r) => n + r.changes.length + r.more, 0);
  return (
    <section className="scopechanges" aria-label="這組還沒 commit 的變更">
      <div className="profilehead">
        <h3>
          這組的變更（{rows?.length ?? 0} 個 repo、{total} 個檔）
        </h3>
        <span className="spacer" />
        <button
          type="button"
          className="icon ghost"
          aria-label="重新讀取這組的變更"
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
      {(rows || []).map((r) => (
        <div key={r.id} className="scopechangerepo">
          <p className="eyebrow">
            {r.id}（{r.changes.length + r.more}）
          </p>
          {r.error ? (
            <p className="error small">{r.error}</p>
          ) : (
            <ChangeList
              id={r.id}
              changes={r.changes}
              more={r.more}
              active={view.activeFile?.repo === r.id ? view.activeFile.path : null}
            />
          )}
        </div>
      ))}
    </section>
  );
}
