import type { RecentEvent } from "../../api/types";
import { fullTextSpec } from "../../dialogs/FullText";
import { isUnscoped, matchesScope, matchesSearch, scopeRepos } from "../../lib/view";
import { useSnapshot, useUi } from "../../store/context";

function RecordRow(props: { title: string; meta: string; time: string; onClick: () => void }) {
  return (
    <div className="record">
      <time className="recordtime">{props.time}</time>
      <div className="recordcopy">
        <button type="button" className="recordtitle ghost" onClick={props.onClick}>
          {props.title}
        </button>
        <div className="recordmeta">{props.meta}</div>
      </div>
    </div>
  );
}

function EventList({ list }: { list: RecentEvent[] }) {
  const ui = useUi();
  if (!list.length) return <p className="emptyline">沒有符合條件的事件</p>;
  return (
    <>
      {list.map((e, i) => {
        const title = e.body.split("\n")[0] || e.type;
        return (
          <RecordRow
            key={e.date + e.header + i}
            title={title}
            meta={`${e.scope_label} · ${e.type} · ${e.source}`}
            time={e.date + " " + e.time}
            onClick={() =>
              ui.openDetail(fullTextSpec(title, e.body, e.date + " " + e.time + " · " + e.scope_label))
            }
          />
        );
      })}
    </>
  );
}

/** 活動分頁：近期 commit＋脈動、範圍內事件（依事件時間）、全域與未分類（收合）、累計統計。 */
export function ActivityPane({ query, active }: { query: string; active: boolean }) {
  const { scan, state, scope } = useSnapshot();
  const ui = useUi();
  const commits = (scan.recent_commits || []).filter((c) => matchesSearch(c, query));
  const p = scan.pulse;
  const members = scopeRepos(scope, state);
  const local = state.recent.filter(
    (e) => matchesScope(e, scope, members) && !isUnscoped(e) && matchesSearch(e, query),
  );
  const global = state.recent.filter((e) => isUnscoped(e) && matchesSearch(e, query));
  const s = state.stats;
  return (
    <section id="tab-activity" className={"tabpane" + (active ? " on" : "")} aria-label="活動記錄">
      <div className="sectionintro">
        <div>
          <h2>近期 commit</h2>
          <p id="pulse" className="small muted">
            {p
              ? `近 7 天 ${p.commits_7d} commits · 30 天 ${p.commits_30d} commits${p.most_active ? " · 最活躍 " + p.most_active : ""}`
              : "等待採集"}
          </p>
        </div>
      </div>
      <div id="recentcommits">
        {commits.length ? (
          commits.map((c, i) => (
            <RecordRow
              key={c.repo + c.hash + i}
              title={c.subject}
              meta={c.repo + " · " + c.hash}
              time={c.date}
              onClick={() =>
                ui.openDetail(fullTextSpec(c.subject, c.subject, c.repo + " · " + c.hash + " · " + c.date))
              }
            />
          ))
        ) : (
          <p className="emptyline">目前範圍沒有符合條件的 commit</p>
        )}
      </div>
      <div className="sectionintro">
        <div>
          <h2>事件與處理記錄</h2>
          <p className="small muted">最近 200 筆事件中屬於目前範圍的記錄，依事件時間排序。</p>
        </div>
      </div>
      <div id="recent">
        <EventList list={local} />
      </div>
      <details id="globalactivity" hidden={!global.length}>
        <summary>
          全域與未分類記錄 <span id="globalactivitycount">{global.length}</span>
        </summary>
        <div id="recentglobal">
          <EventList list={global} />
        </div>
      </details>
      <p id="statsraw" className="small muted">
        {s.collisions
          ? `所有組累計：${s.collisions} 次分析 · ${s.collisions_with_outcome} 次成果回連${s.spawned ? " · " + s.spawned + " 個新 repo" : ""}`
          : ""}
      </p>
    </section>
  );
}
