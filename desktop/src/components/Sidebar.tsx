import { sessionInGroup, shellOwner } from "../lib/view";
import { useSnapshot, useStore, useUi } from "../store/context";
import { AsyncButton } from "./AsyncButton";
import { Deck } from "./deck/Deck";
import { SessionRow } from "./sessions/SessionRow";

/** 不屬於任何牌組的會話（Shell、臨時組、全部）；屬於牌組的放在牌組區塊裡。 */
function SessionList() {
  const { state } = useSnapshot();
  const groups = state.groups.map((g) => g.name);
  const all = state.terms || [];
  // 某個 agent 的 Terminal 從 agent 那列的終端標題切過去，這裡不另列（agent 不在了才出來）
  const list = all.filter(
    (s) => !groups.some((g) => sessionInGroup(s, g)) && !all.some((a) => a.sid === shellOwner(s)),
  );
  return (
    <>
      <div id="sessions" className="sessions">
        {list.map((s) => (
          <SessionRow key={s.sid} s={s} />
        ))}
      </div>
      <p id="sessempty" className="emptyhint" hidden={!!list.length}>
        {all.length
          ? "牌組的會話放在各牌組區塊裡；這裡是 Shell 和臨時範圍的。"
          : "為目前牌組開啟 Agent 會話，帶著工作脈絡開始。"}
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
