import { useCallback, useEffect, useState } from "react";
import type { CommitInfo } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { useStore, useUi } from "../../store/context";
import { diffLineKind } from "../viewer/DiffNotes";

/** commit 時間 → 「今天 14:05」「昨天」「3 天前」「2026-09-01」。 */
export function commitWhen(at: string, now = new Date()): string {
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((day(now) - day(d)) / 86400000);
  if (days <= 0) return "今天 " + pad(d.getHours()) + ":" + pad(d.getMinutes());
  if (days === 1) return "昨天";
  if (days < 30) return days + " 天前";
  return d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate());
}

/** 一個 commit 的差異（`git show`：說明＋統計＋patch），加的綠、刪的紅。 */
function CommitDiff({ id, commit }: { id: string; commit: CommitInfo }) {
  const store = useStore();
  const ui = useUi();
  const { close } = useDetail();
  const [r, setR] = useState<{ diff: string; truncated: boolean } | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let alive = true;
    store
      .action("commit_diff", { id, sha: commit.sha })
      .then((x) => alive && setR(x))
      .catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [store, id, commit.sha]);
  return (
    <DetailLayout
      actions={
        <>
          <button
            type="button"
            title="把這個 commit 的 sha 貼進目前的會話（不送出），例：請 agent 解釋或回頭改"
            onClick={() => {
              const t = ui.pasteToSession(commit.short + "（" + commit.subject + "）");
              store.toast(t ? "已貼到「" + t + "」" : "先在下方終端選一個執行中的會話");
            }}
          >
            貼給會話
          </button>
          <button type="button" className="primary" onClick={close}>
            關閉
          </button>
        </>
      }
    >
      <p className="detailmeta">
        {commit.short + " · " + commit.author + " · " + commitWhen(commit.at)}
      </p>
      {error ? <p className="error">{"讀不到差異：" + error}</p> : null}
      {!r && !error ? <p className="muted">讀取中…</p> : null}
      {r?.truncated ? <p className="viewernote">差異太長，只顯示前 512 KB。</p> : null}
      {r ? (
        <pre className="diffview commitdiff" aria-label="commit 差異">
          {r.diff.split("\n").map((l, i) => (
            <div key={i} className={"dl " + diffLineKind(l)}>
              {l || " "}
            </div>
          ))}
        </pre>
      ) : null}
    </DetailLayout>
  );
}

/**
 * 最近的 commit（F-109）：「變更」頁只看得到還沒 commit 的，agent 做完常常已經 commit 了。
 * 展開才讀；點一個看它的差異。
 */
export function RepoCommits({ id }: { id: string }) {
  const store = useStore();
  const ui = useUi();
  const [list, setList] = useState<CommitInfo[] | null>(null);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      const r = await store.action("repo_log", { id, limit: 20 });
      setList(r.commits);
      setError("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [store, id]);
  return (
    <details
      className="panelcommits"
      onToggle={(e) => {
        if ((e.currentTarget as HTMLDetailsElement).open) void load();
      }}
    >
      <summary className="small">最近的 commit</summary>
      {error ? <p className="error small">{"讀不到 commit：" + error}</p> : null}
      {list && !list.length ? <p className="muted small">還沒有 commit。</p> : null}
      <ul className="commitlist">
        {(list || []).map((c) => (
          <li key={c.sha}>
            <button
              type="button"
              className="ghost commitrow"
              title={c.subject + "\n" + c.author + " · " + c.at}
              onClick={() =>
                ui.openDetail({
                  title: c.subject,
                  eyebrow: "COMMIT",
                  content: <CommitDiff id={id} commit={c} />,
                })
              }
            >
              <span className="commitsubject">{c.subject}</span>
              <span className="small muted">{c.short + " · " + commitWhen(c.at)}</span>
            </button>
          </li>
        ))}
      </ul>
    </details>
  );
}
