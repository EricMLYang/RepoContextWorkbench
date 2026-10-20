import { useCallback, useEffect, useState } from "react";
import { Resizer } from "../Resizer";
import { MSG_COPY_FAILED, copyText } from "../../lib/clipboard";
import { useSnapshot, useStore, useUi, type PanelTab } from "../../store/context";
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

const TABS: [PanelTab, string][] = [
  ["files", "檔案"],
  ["changes", "變更"],
  ["info", "資訊"],
];

/**
 * 右側欄：點 repo 打開，分三個頁籤一次看一種（UX-07）——檔案（樹）／變更（還沒 commit 的）／
 * 資訊（分支、未提交、未推送／落後、最近 commit、Agent、名片、開會話）。
 * 更多細節、標籤、移出工作台都在右鍵選單（或標題列「⋯」）裡，需要才跳視窗。
 * 三頁都掛著、只切換顯示：變更頁不在畫面上也照樣讀，頁籤上的數字與「未提交」才是現場的。
 */
export function RepoPanel({
  id,
  tab,
  onTab,
}: {
  id: string;
  tab: PanelTab;
  onTab: (t: PanelTab) => void;
}) {
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
      await copyText(store.backend, s.path);
      store.toast("已複製絕對路徑：" + s.path);
    } catch {
      store.toast(MSG_COPY_FAILED);
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
      <div className="paneltabs" role="tablist" aria-label={id + " 的側欄頁籤"}>
        {TABS.map(([key, label]) => (
          <button
            key={key}
            type="button"
            role="tab"
            id={"paneltab-" + key}
            aria-selected={tab === key}
            aria-controls={"panelpage-" + key}
            className={"ghost small" + (tab === key ? " on" : "")}
            onClick={() => onTab(key)}
          >
            {label}
            {key === "changes" && liveDirty ? " " + liveDirty : ""}
          </button>
        ))}
      </div>
      <div
        role="tabpanel"
        id="panelpage-files"
        aria-labelledby="paneltab-files"
        className="panelsection"
        hidden={tab !== "files"}
      >
        <RepoRoot key={id} id={id} bare />
      </div>
      <div
        role="tabpanel"
        id="panelpage-changes"
        aria-labelledby="paneltab-changes"
        hidden={tab !== "changes"}
      >
        <RepoChanges id={id} onCount={onCount} />
      </div>
      <div
        role="tabpanel"
        id="panelpage-info"
        aria-labelledby="paneltab-info"
        hidden={tab !== "info"}
      >
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
        <RepoProfileCard id={id} />
      </div>
      {/* 寬度把手放在右側欄 landmark 裡（position: fixed；axe region） */}
      <Resizer side="right" />
    </aside>
  );
}
