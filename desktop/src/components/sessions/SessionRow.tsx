import { useState } from "react";
import type { TermInfo } from "../../api/types";
import {
  sessionAwaitsTui,
  sessionDot,
  sessionFailed,
  sessionFresh,
  sessionGauges,
  sessionHeadline,
  sessionMeta,
  sessionNeedsYou,
  sessionTooltip,
  sessionWantsYou,
  sessionWhere,
  shellOwner,
  statusText,
} from "../../lib/view";
import { useSnapshot, useStore, useUi, useUiState, type Ui } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/** 切到會話後終端要一拍才接上：最多等這麼多次、每次這麼久，貼得進去就回會話標題。 */
const PASTE_TRIES = 12;
const PASTE_WAIT_MS = 150;

/** 貼進目前會話（bracketed paste，不送出）；終端還沒接上就等一下再試。回會話標題；放棄＝null。 */
export async function pasteWhenReady(ui: Ui, text: string): Promise<string | null> {
  for (let i = 0; i < PASTE_TRIES; i++) {
    const title = ui.pasteToSession(text);
    if (title) return title;
    await new Promise((r) => setTimeout(r, PASTE_WAIT_MS));
  }
  return null;
}

/**
 * F-85：在等你的會話直接回一句——切到那個會話、把話貼進輸入框（不按 Enter，看一眼再送）。
 * 要你允許工具的那種不會替你按 y（不注入按鍵）。
 */
function ReplyBox({ s, onDone }: { s: TermInfo; onDone: () => void }) {
  const ui = useUi();
  const store = useStore();
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const send = async () => {
    const t = text.trim();
    if (!t || busy) return;
    setBusy(true);
    ui.selectTerm(s.sid);
    const title = await pasteWhenReady(ui, t);
    setBusy(false);
    if (!title) {
      store.toast("會話還沒接上，再按一次 Enter");
      return;
    }
    store.toast(`已貼到「${title}」（還沒送出，看一眼再按 Enter）`);
    setText("");
    onDone();
  };
  return (
    <form
      className="sessionreplyform"
      onSubmit={(e) => {
        e.preventDefault();
        void send();
      }}
    >
      <input
        autoFocus
        value={text}
        disabled={busy}
        placeholder="回答…（Enter 貼進會話，不送出）"
        title="Enter 貼進會話的輸入框（不送出）；Esc 收起"
        aria-label={"回答「" + sessionHeadline(s) + "」"}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onDone();
        }}
      />
    </form>
  );
}

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
  const [replying, setReplying] = useState(false);
  // Claude Code 停在權限／提問提示：打字回答沒用，要切過去在終端按；其他等你的才給回答框
  const awaitsTui = sessionAwaitsTui(s) && s.alive;
  const canReply = sessionNeedsYou(s) && s.kind === "agent" && !awaitsTui;
  // 等你時多一行它說了什麼；停在權限提示而沒有回覆預覽時，拿 hook 記的提示（例：要你允許 Bash）
  const gauges = sessionGauges(s);
  const preview = sessionWantsYou(s)
    ? s.last_reply || (awaitsTui ? s.agent_status?.text : null) || null
    : null;
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
                (sessionWantsYou(s) || s.tool ? statusText(s) + " · " : "") +
                (where.repo ? where.repo : "整組")
              : sessionMeta(s)}
          </span>
          {preview ? <span className="sessionpreview">{preview}</span> : null}
          {gauges.length ? (
            <span className="sessiongauges">
              {gauges.map((g) => (
                <span key={g.text} className={"gauge" + (g.warn ? " warn" : "")} title={g.title}>
                  {g.text}
                </span>
              ))}
            </span>
          ) : null}
        </span>
      </button>
      {sessionFailed(s) ? (
        // F-99：額度、過載中斷——切過去把「繼續」貼進輸入框（不送出，額度還沒回來就先別按 Enter）
        <AsyncButton
          className="ghost small sessionreply"
          title="切到這個會話，把「繼續」貼進輸入框（不送出）"
          onError={(m) => store.toast(m)}
          run={async () => {
            ui.selectTerm(s.sid);
            const title = await pasteWhenReady(ui, "繼續");
            store.toast(
              title ? `已貼到「${title}」（看額度回來了再按 Enter）` : "會話還沒接上，再按一次",
            );
          }}
        >
          繼續
        </AsyncButton>
      ) : null}
      {awaitsTui ? (
        <button
          type="button"
          className="ghost small sessionreply"
          title="切到這個會話，在終端允許或回答（App 不會替你按 y）"
          onClick={() => ui.selectTerm(s.sid)}
        >
          去處理
        </button>
      ) : null}
      {sessionFresh(s) && where.repo ? (
        // 做完一輪、還沒看：人下一步幾乎都是「看它改了什麼」——直接開那個 repo 的變更頁（不切會話、不算看過）
        <button
          type="button"
          className="ghost small sessionreply"
          title={"打開 " + where.repo + " 的變更頁：還沒 commit 的檔，點了看差異"}
          onClick={() => ui.showRepo(where.repo!, "changes")}
        >
          看變更
        </button>
      ) : null}
      {canReply ? (
        <button
          type="button"
          className="ghost small sessionreply"
          aria-pressed={replying}
          title="回它一句：切到這個會話、貼進輸入框（不送出）"
          onClick={() => setReplying((v) => !v)}
        >
          回答…
        </button>
      ) : null}
      {canReply && replying ? <ReplyBox s={s} onDone={() => setReplying(false)} /> : null}
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
