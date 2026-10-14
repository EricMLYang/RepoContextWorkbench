import type { RecentEvent } from "../../api/types";
import { fullTextSpec } from "../../dialogs/FullText";
import { matchesSearch } from "../../lib/view";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

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
              ui.openDetail(
                fullTextSpec(title, e.body, e.date + " " + e.time + " · " + e.scope_label),
              )
            }
          />
        );
      })}
    </>
  );
}

const pct = (x: number | null) => (x === null ? "—" : Math.round(x * 100) + "%");

/** 健康指標（同 `ctx health`，引擎算好；這裡只排版）：真用一陣子後看行為有沒有改變。 */
function HealthLine() {
  const { state } = useSnapshot();
  const store = useStore();
  const ui = useUi();
  const h = state.health;
  if (!h) return null;
  const p = h.profiles;
  return (
    <div id="health" className="sectionintro">
      <div>
        <h2>{`健康（近 ${h.days} 天）`}</h2>
        <p className="small muted">
          {`交接率 ${pct(h.handoff.rate)}（沒交接 ${h.handoff.missed}）· ` +
            `推薦命中 ${pct(h.fresh.hit_rate)}（${h.fresh.used}/${h.fresh.found}）· ` +
            `判斷有結果 ${pct(h.decisions.outcome_rate)}（待回顧 ${h.decisions.awaiting_retro}）· ` +
            `未結 開 ${h.loops.opened}／關 ${h.loops.closed}，逾期 ${h.loops.overdue}`}
          {h.assist && h.assist.runs + h.assist.undone > 0
            ? ` · 助手 ${h.assist.runs} 次（有做事 ${h.assist.did_something}、失敗 ${h.assist.failed}、復原 ${h.assist.undone}）`
            : ""}
        </p>
        {p && p.repos ? (
          <p className="small muted healthprofiles">
            {`名片 ${p.described}/${p.repos} 寫了介紹`}
            {p.undescribed.length ? (
              <>
                {"；還沒寫："}
                {p.undescribed.map((id) => (
                  <button
                    key={id}
                    type="button"
                    className="linkish"
                    title={"在右側欄打開 " + id + "，寫一句介紹"}
                    onClick={() => ui.showRepo(id)}
                  >
                    {id}
                  </button>
                ))}
                {p.undescribed_more ? `，另 ${p.undescribed_more} 個` : null}
                <AsyncButton
                  className="ghost small"
                  title="開一個 Agent 會話：讀這些 repo、列草稿給你確認後才寫進 registry"
                  onError={(m) => store.toast(m)}
                  run={async () => {
                    const r = await store.action("profile_draft_brief", {});
                    ui.openSession({
                      group: null,
                      repos: r.repos,
                      task: r.text,
                      origin: "profile-draft",
                    });
                  }}
                >
                  請 Agent 起草…
                </AsyncButton>
              </>
            ) : null}
          </p>
        ) : null}
      </div>
    </div>
  );
}

/** 活動分頁：近期 commit＋脈動、範圍內事件（依事件時間）、全域與未分類（收合）、累計統計。 */
export function ActivityPane({ query, active }: { query: string; active: boolean }) {
  const { scan, state } = useSnapshot();
  const ui = useUi();
  const commits = (scan.recent_commits || []).filter((c) => matchesSearch(c, query));
  const p = scan.pulse;
  // 範圍歸屬由引擎標好（lane），這裡只做搜尋
  const local = state.recent.filter((e) => e.lane === "scope" && matchesSearch(e, query));
  const global = state.recent.filter((e) => e.lane === "global" && matchesSearch(e, query));
  const s = state.stats;
  return (
    <section id="tab-activity" className={"tabpane" + (active ? " on" : "")} aria-label="活動記錄">
      <HealthLine />
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
                ui.openDetail(
                  fullTextSpec(c.subject, c.subject, c.repo + " · " + c.hash + " · " + c.date),
                )
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
