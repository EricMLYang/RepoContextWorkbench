import type { TermInfo } from "../../api/types";
import {
  STATUS_TXT,
  sessionDot,
  sessionHeadline,
  sessionMeta,
  sessionNeedsYou,
  sessionTooltip,
  sessionWhere,
  shellOwner,
} from "../../lib/view";
import { useSnapshot, useUi, useUiState } from "../../store/context";

/**
 * 會話一列：小圓點（綠＝有留痕、琥珀＝執行中尚無回報、紅＝等你／卡住、灰＝已結束）＋標題＋狀態；
 * 在等你時多一行最後回覆（F-71：不切過去也知道它說了什麼）。
 * `inGroup`：放在牌組區塊裡時，位置只寫 repo（組名已經是區塊標題）。
 */
export function SessionRow({ s, inGroup = false }: { s: TermInfo; inGroup?: boolean }) {
  const ui = useUi();
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
                (sessionNeedsYou(s) ? STATUS_TXT[s.agent_status!.status] + " · " : "") +
                (where.repo ? where.repo : "整組")
              : sessionMeta(s)}
          </span>
          {sessionNeedsYou(s) && s.last_reply ? (
            <span className="sessionpreview">{s.last_reply}</span>
          ) : null}
        </span>
      </button>
    </div>
  );
}
