import { sessionDot, sessionMeta, sessionTooltip } from "../lib/view";
import { useSnapshot, useStore, useUi } from "../store/context";
import { AsyncButton } from "./AsyncButton";
import { Deck } from "./deck/Deck";

/** 會話列：小圓點（綠＝有留痕、琥珀＝執行中尚無回報、紅＝等你／卡住、灰＝已結束）。 */
function SessionList() {
  const { state } = useSnapshot();
  const ui = useUi();
  const list = state.terms || [];
  return (
    <>
      <div id="sessions" className="sessions">
        {list.map((s) => {
          const on = ui.activeSid === s.sid;
          return (
            <div key={s.sid} className={"session" + (on ? " on" : "")}>
              <button
                type="button"
                className="ghost"
                aria-pressed={on}
                title={sessionTooltip(s)}
                onClick={() => ui.selectTerm(s.sid)}
              >
                <span className={["dot", ...sessionDot(s)].join(" ")} />
                <span className="sessioncopy">
                  <span className="sessiontitle">{s.title}</span>
                  <span className="sessionmeta">{sessionMeta(s)}</span>
                </span>
              </button>
            </div>
          );
        })}
      </div>
      <p id="sessempty" className="emptyhint" hidden={!!list.length}>
        為目前牌組開啟 Agent 會話，帶著工作脈絡開始。
      </p>
    </>
  );
}

export function Sidebar() {
  const store = useStore();
  const ui = useUi();
  const { state, audit } = useSnapshot();
  const total = state.terms?.length || 0;
  return (
    <aside id="side" aria-label="工作導覽">
      <div className="sechead">
        <h2>
          牌組 <span className="muted">／ 工作範圍</span>
        </h2>
        <button
          type="button"
          id="adhocbtn"
          className="icon ghost"
          aria-label="建立臨時組"
          title="建立臨時組"
          onClick={() => ui.openPicker()}
        >
          ＋
        </button>
      </div>
      <Deck />
      <div className="sechead sessionhead">
        <h2>
          會話 <span id="sessiontotal">{total || ""}</span>
        </h2>
        <AsyncButton
          id="newshell"
          className="ghost small"
          onError={(m) => store.toast(m)}
          run={async () => {
            const r = await store.action("term_create", { kind: "shell" });
            await store.refreshState();
            ui.selectTerm(r.sid);
          }}
        >
          ＋ Shell
        </AsyncButton>
      </div>
      <SessionList />
      <div className="sidebottom">
        <details id="auditdetails">
          <summary>
            全域登記檢查{" "}
            <span id="auditcount">
              {audit ? (audit.length ? " · " + audit.length + " 項" : " · 通過") : ""}
            </span>
          </summary>
          <div id="auditbox" className="small muted">
            {!audit ? (
              "等待首次更新"
            ) : audit.length ? (
              audit.map((t, i) => <p key={i}>{t}</p>)
            ) : (
              <p>登記檢查通過</p>
            )}
          </div>
        </details>
        <span className="small muted">本機 Repo 工作台</span>
      </div>
    </aside>
  );
}
