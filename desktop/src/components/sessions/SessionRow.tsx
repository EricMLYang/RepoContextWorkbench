import type { TermInfo } from "../../api/types";
import {
  STATUS_TXT,
  sessionDot,
  sessionHeadline,
  sessionMeta,
  sessionTooltip,
  sessionWantsYou,
  sessionWhere,
  shellOwner,
} from "../../lib/view";
import { useSnapshot, useStore, useUi, useUiState } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/**
 * 會話一列：小圓點（琥珀＝在跑、紅＝等你／卡住、亮綠＝做完沒看、暗綠＝做完看過、灰＝已結束；見 `sessionDot`）＋標題＋狀態；
 * 在等你或做完沒看時多一行最後回覆（F-71／F-80：不切過去也知道它說了什麼）。
 * `inGroup`：放在牌組區塊裡時，位置只寫 repo（組名已經是區塊標題）。
 */
export function SessionRow({ s, inGroup = false }: { s: TermInfo; inGroup?: boolean }) {
  const ui = useUi();
  const store = useStore();
  const view = useUiState();
  const { state } = useSnapshot();
  // 正在看它的 Terminal 也算選著這列
  const active = state.terms?.find((t) => t.sid === view.activeSid);
  const on = view.activeSid === s.sid || (!!active && shellOwner(active) === s.sid);
  const where = sessionWhere(s);
  return (
    <div className={"session" + (on ? " on" : "") + (inGroup ? " ingroup" : "")}>
      <button
        type="button"
        className="ghost"
        aria-pressed={on}
        title={sessionTooltip(s)}
        onClick={() => ui.selectTerm(s.sid)}
      >
        <span className={["dot", ...sessionDot(s)].join(" ")} />
        <span className="sessioncopy">
          <span className="sessiontitle">{sessionHeadline(s)}</span>
          <span className="sessionmeta">
            {inGroup
              ? (s.alive ? "" : "已結束 · ") +
                (sessionWantsYou(s) ? STATUS_TXT[s.agent_status!.status] + " · " : "") +
                (where.repo ? where.repo : "整組")
              : sessionMeta(s)}
          </span>
          {sessionWantsYou(s) && s.last_reply ? (
            <span className="sessionpreview">{s.last_reply}</span>
          ) : null}
        </span>
      </button>
      {!s.alive && s.claude_session && s.resume_cwd ? (
        // F-78：程序結束了但 Claude Code 的對話還在——在原目錄接回去（像 Orca 的 Restart）
        <AsyncButton
          className="ghost small sessionresume"
          title="在原目錄執行 claude --resume，接回這段對話"
          onError={(m) => store.toast(m)}
          run={async () => {
            const r = await store.action("term_create", {
              kind: "agent",
              resume: s.claude_session!,
              cwd: s.resume_cwd!,
              task: "接續：" + sessionHeadline(s),
            });
            await store.refreshState();
            ui.selectTerm(r.sid);
          }}
        >
          接續
        </AsyncButton>
      ) : null}
    </div>
  );
}
