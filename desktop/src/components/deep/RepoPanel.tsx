import { useCallback, useEffect, useState } from "react";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { RepoRoot } from "./FileTree";
import { RepoChanges } from "./RepoChanges";
import { RepoProfileCard } from "./RepoProfileCard";

/** 一行狀態：有才顯示，沒採集到寫「—」。 */
function Fact({ label, value, warn }: { label: string; value: string; warn?: boolean }) {
  return (
    <div className={"panelfact" + (warn ? " warn" : "")}>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

/**
 * 右側欄：點 repo 打開，只放邊工作邊會看的（分支、未提交、未推送／落後、最近 commit、Agent、路徑、變更、名片、檔案）。
 * 更多細節、標籤、移出工作台都在右鍵選單（或標題列「⋯」）裡，需要才跳視窗。
 */
export function RepoPanel({ id }: { id: string }) {
  const store = useStore();
  const ui = useUi();
  const { lastStates, state, stateReady } = useSnapshot();
  const s = lastStates[id];
  // 未提交：優先用「變更」段現場讀到的數字（掃描快照可能是幾分鐘前的），兩處才不會一個說乾淨、一個列出變更
  const [live, setLive] = useState<{ id: string; n: number | null }>({ id, n: null });
  const liveDirty = live.id === id ? live.n : null;
  const dirty = liveDirty ?? s?.dirty ?? 0;
  // 同一個值不換新物件（RepoChanges 每次重畫都會回報）
  const onCount = useCallback(
    (n: number | null) => setLive((cur) => (cur.id === id && cur.n === n ? cur : { id, n })),
    [id],
  );

  // repo 被移出工作台就收起
  useEffect(() => {
    if (stateReady && !state.repos.includes(id)) ui.closeRepo();
  }, [stateReady, state.repos, id, ui]);

  const copyPath = async () => {
    if (!s?.path) return;
    try {
      await navigator.clipboard.writeText(s.path);
      store.toast("已複製絕對路徑：" + s.path);
    } catch {
      store.toast("無法複製到剪貼簿");
    }
  };

  const days = s?.last_commit_days;
  return (
    <aside id="repopanel" aria-label={"Repo：" + id}>
      <div className="panelhead">
        <h2 title={id}>{id}</h2>
        <button
          type="button"
          className="icon ghost"
          aria-label={id + " 的更多動作"}
          title="更多動作（也可以在 repo 上按右鍵）"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            ui.repoMenu(id, r.left, r.bottom + 4);
          }}
        >
          ⋯
        </button>
        <button
          type="button"
          className="icon ghost"
          aria-label="收起側邊欄"
          title="收起"
          onClick={ui.closeRepo}
        >
          ×
        </button>
      </div>
      {s ? (
        <dl className="panelfacts">
          <Fact label="分支" value={s.branch || "—"} />
          <Fact
            label="未提交"
            value={(dirty ? dirty + " 檔" : "乾淨") + (liveDirty == null ? "（上次掃描）" : "")}
            warn={dirty > 0}
          />
          <Fact
            label="未推送／落後"
            value={(s.ahead ?? "—") + " ／ " + (s.behind ?? "—")}
            warn={!!s.ahead || !!s.behind}
          />
          <Fact
            label="最近 commit"
            value={
              (s.last_subject || "—") +
              (days == null ? "" : days < 1 ? "（今天）" : "（" + Math.round(days) + " 天前）")
            }
          />
          {s.agents?.length ? <Fact label="Agent" value={s.agents.join("、")} /> : null}
          {s.note ? <Fact label="注意" value={s.note} warn /> : null}
        </dl>
      ) : (
        <p className="muted small panelnote">
          還沒採集到這個 repo 的狀態（切到包含它的牌組會更新）。
        </p>
      )}
      <div className="panelactions">
        <button
          type="button"
          className="small"
          disabled={!s?.path}
          title={s?.path}
          onClick={() => void copyPath()}
        >
          複製路徑
        </button>
        <button
          type="button"
          className="small primary"
          onClick={() => ui.openSession({ repo: id, repos: [id], origin: "repo:" + id })}
        >
          開啟 Agent 會話
        </button>
      </div>
      <RepoChanges id={id} onCount={onCount} />
      <RepoProfileCard id={id} />
      <div className="panelsection">
        <RepoRoot key={id} id={id} bare />
      </div>
    </aside>
  );
}
