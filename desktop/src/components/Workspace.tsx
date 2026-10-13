import { useState } from "react";
import { fullTextSpec } from "../dialogs/FullText";
import { AckConfirm } from "../dialogs/misc";
import { bannerText, scopeKindLabel, scopeLabel, scopedCards } from "../lib/view";
import { useSnapshot, useStore, useUi, type Tab } from "../store/context";
import { AsyncButton } from "./AsyncButton";
import { Composer } from "./Composer";
import { ActivityPane } from "./deep/ActivityPane";
import { FilesPane } from "./deep/FilesPane";
import { RelationsPane } from "./deep/RelationsPane";
import { ReposPane } from "./deep/ReposPane";
import { InboxPane } from "./inbox/InboxPane";
import { NotesPane } from "./notes/NotesPane";

const TABS: [Tab, string][] = [
  ["inbox", "待處理"],
  ["notes", "筆記"],
  ["repos", "Repo"],
  ["files", "檔案"],
  ["activity", "活動"],
  ["relations", "關係"],
];

/** 工作區：標題＋摘要＋開組會話、碰撞台、斷線橫幅、五分頁＋工具列。 */
export function Workspace(props: {
  tab: Tab;
  composerOpen: boolean;
  composerFocusSeq: number;
  closeComposer: () => void;
  /** 主區正在看檔案分頁時藏起來（不卸載，搜尋與篩選保留）。 */
  hidden?: boolean;
}) {
  const { tab, composerOpen, composerFocusSeq, closeComposer } = props;
  const store = useStore();
  const ui = useUi();
  const snap = useSnapshot();
  const { state, scan, scope, stateReady, stateError, scanError, scanLoading, scanAt } = snap;
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [lastTab, setLastTab] = useState(tab);
  // 換分頁清空搜尋（跟舊版 showTab 一樣）
  if (lastTab !== tab) {
    setLastTab(tab);
    setQuery("");
  }
  const q = query.trim().toLocaleLowerCase();
  const repos = store.scopeRepos;
  const local = scopedCards(state, scan);
  const todo = local.filter((c) => c.kind !== "pending").length;
  const pending = local.filter((c) => c.kind === "pending").length;
  const err = stateError || scanError;
  const noScope = !stateReady || !repos.length;

  return (
    <div id="workspace" hidden={props.hidden}>
      <div className="eyebrow">
        WORKSPACE <span id="scopekind">{scopeKindLabel(scope)}</span>
      </div>
      <div className="workspacehead">
        <div>
          <h1 id="scopename">{scopeLabel(scope)}</h1>
          <p id="worksummary" className="muted">
            {stateReady
              ? `${repos.length} 個 repo · 範圍內 ${todo} 件待處理 · ${pending} 件分析中`
              : "正在載入工作空間…"}
          </p>
        </div>
        <div className="head-actions">
          <button
            type="button"
            id="notebtn"
            disabled={!stateReady || !scope.group}
            title={
              scope.group
                ? "記一筆到這個牌組（不屬於任何 repo 的事）"
                : "筆記記在牌組上；先選一個牌組"
            }
            onClick={() => ui.showTab("notes")}
          >
            ＋ 記一筆
          </button>
          <button type="button" id="ideatoggle" disabled={noScope} onClick={ui.showComposer}>
            ＋ 新增想法
          </button>
          <button
            type="button"
            id="newagent"
            className="primary"
            disabled={noScope}
            onClick={() => ui.openSession({ origin: "group:" + scopeLabel(scope) })}
          >
            開啟組會話 <span aria-hidden="true">↗</span>
          </button>
        </div>
      </div>
      <Composer open={composerOpen} focusSeq={composerFocusSeq} onClose={closeComposer} />
      <div id="syncerror" className="errorbanner" role="alert" hidden={!err}>
        {bannerText(err)}
      </div>
      <nav id="tabs" aria-label="工作區頁面">
        {TABS.map(([key, label]) => (
          <button
            key={key}
            type="button"
            data-tab={key}
            className={tab === key ? "on" : ""}
            aria-current={tab === key ? "page" : undefined}
            onClick={() => ui.showTab(key)}
          >
            {label}
            {key === "inbox" ? (
              <>
                {" "}
                <span id="inboxcount" className="tabcount">
                  {stateReady ? todo : "—"}
                </span>
              </>
            ) : null}
          </button>
        ))}
        <span className="spacer" />
        <span id="scantime" className="small muted">
          {scanLoading ? "更新中…" : scanAt ? "更新於 " + scanAt : "尚未更新"}
        </span>
      </nav>
      <div className="toolbar">
        <label className="searchbox">
          <span aria-hidden="true">⌕</span>
          <input
            id="search"
            type="search"
            aria-label="搜尋目前頁面"
            placeholder="搜尋標題、repo 或內容"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
        <select
          id="cardfilter"
          aria-label="篩選待處理類型"
          hidden={tab !== "inbox"}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        >
          <option value="all">所有類型</option>
          <option value="question">需要判斷</option>
          <option value="collision">分析結果</option>
          <option value="loop">待跟進</option>
          <option value="fresh">相關新料</option>
          <option value="reopen">該重看</option>
          <option value="retro">判斷回顧</option>
          <option value="pending">處理中</option>
        </select>
        <span className="spacer" />
        <AsyncButton
          id="briefbtn"
          hidden={tab !== "inbox"}
          className="ghost small"
          disabled={noScope}
          onError={(m) => store.toast(m)}
          run={async () => {
            const r = await store.action("brief", store.scopePayload);
            ui.openDetail(fullTextSpec("今日簡報已儲存", r.text, r.file));
            await store.refreshState();
          }}
        >
          存成今日簡報
        </AsyncButton>
        <button
          type="button"
          id="ackbtn"
          className="ghost small"
          hidden={tab !== "inbox"}
          title="影響所有組的未讀事件；不會完成待跟進或處理中任務"
          disabled={!stateReady || !state.unread?.length}
          onClick={() =>
            ui.openDetail({
              title: "將所有組的事件標為已讀？",
              eyebrow: "ALL WORKSPACES",
              content: <AckConfirm />,
            })
          }
        >
          所有組標為已讀…
        </button>
      </div>
      <InboxPane query={q} filter={filter} active={tab === "inbox"} />
      <NotesPane query={q} active={tab === "notes"} />
      <ReposPane query={q} active={tab === "repos"} />
      <FilesPane query={q} active={tab === "files"} />
      <ActivityPane query={q} active={tab === "activity"} />
      <RelationsPane query={q} active={tab === "relations"} />
    </div>
  );
}
