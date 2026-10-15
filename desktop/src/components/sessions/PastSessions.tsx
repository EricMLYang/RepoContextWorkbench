import { useEffect, useState } from "react";
import type { PastSession } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/** 在終端自己接續用的指令。 */
export function resumeCommand(s: PastSession): string {
  return `cd ${JSON.stringify(s.cwd)} && claude --resume ${s.id}`;
}

/**
 * 過去的會話（F-72）：目前範圍內 repo 的 Claude Code 互動會話，最近的在前。
 * 「接續」在原目錄開 `claude --resume <id>` 的 App 會話；「複製指令」給想在外面終端接的人。
 */
export function PastSessions() {
  const store = useStore();
  const ui = useUi();
  const { close } = useDetail();
  const [list, setList] = useState<PastSession[] | null>(null);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState("");
  const scope = JSON.stringify(store.scopePayload);
  useEffect(() => {
    // 對話框每次開都重新掛載，不必先清錯誤
    let alive = true;
    store
      .action("past_sessions", { ...(JSON.parse(scope) as object), limit: 30 })
      .then((r) => alive && setList(r.sessions))
      .catch((e: Error) => alive && setError(e.message));
    return () => {
      alive = false;
    };
  }, [store, scope]);
  return (
    <DetailLayout actions={<button onClick={close}>關閉</button>}>
      {error ? <p className="error">{"讀不到過去的會話：" + error}</p> : null}
      {list === null && !error ? <p className="muted">讀取中…</p> : null}
      {list && !list.length ? (
        <p className="muted">這個範圍還沒有 Claude Code 的互動會話。</p>
      ) : null}
      <ul className="pastlist">
        {(list || []).map((s) => (
          <li key={s.id} className="pastrow">
            <div className="pasthead">
              <span className="pasttitle" title={s.prompt || s.title}>
                {s.title}
              </span>
              {s.live ? <span className="pill">可能還開著</span> : null}
            </div>
            <div className="small muted">
              {[s.updated, s.repo, s.branch].filter(Boolean).join(" · ")}
            </div>
            {s.reply ? <div className="pastreply small muted">{s.reply}</div> : null}
            <div className="pastacts">
              <AsyncButton
                className="small"
                title={s.live ? "這場可能還在別的視窗開著，接續會變成兩個視窗同一場" : undefined}
                run={async () => {
                  const r = await store.action("term_create", {
                    kind: "agent",
                    resume: s.id,
                    cwd: s.cwd,
                    task: "接續：" + s.title,
                  });
                  await store.refreshState();
                  close();
                  ui.selectTerm(r.sid);
                }}
              >
                接續
              </AsyncButton>
              <AsyncButton
                className="ghost small"
                run={async () => {
                  await navigator.clipboard.writeText(resumeCommand(s));
                  setCopied(s.id);
                }}
              >
                {copied === s.id ? "已複製" : "複製指令"}
              </AsyncButton>
            </div>
          </li>
        ))}
      </ul>
    </DetailLayout>
  );
}

export function pastSessionsSpec() {
  return { title: "過去的會話", eyebrow: "SESSIONS", content: <PastSessions /> };
}
