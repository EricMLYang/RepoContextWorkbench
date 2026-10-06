import { useEffect, useRef, useState } from "react";
import type { TermInfo } from "../../api/types";
import { DetailLayout, useAutoFocus, useDetail } from "../../dialogs/Modal";
import { load, save } from "../../lib/storage";
import { reportLine, scopeRepos, sessionTarget, taskEntries } from "../../lib/view";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/** 開會話表單：選 Agent、寫任務；範圍在開啟時就固定。 */
export function SessionForm({ payload, task: initialTask }: { payload: Record<string, unknown>; task: string }) {
  const store = useStore();
  const ui = useUi();
  const { close } = useDetail();
  const { state, scan, scope } = useSnapshot();
  const agents = state.agents || [];
  const [agent, setAgent] = useState(() => {
    const remembered = load<string>("agent", agents[0] || "claude");
    return agents.includes(remembered) ? remembered : agents[0] || "";
  });
  const [task, setTask] = useState(initialTask);
  const taskRef = useRef<HTMLTextAreaElement>(null);
  return (
    <DetailLayout
      actions={
        <AsyncButton
          className="primary"
          disabled={!agents.length}
          run={async () => {
            save("agent", agent);
            const r = await store.action("term_create", {
              ...payload,
              kind: "agent",
              agent,
              task: task.trim(),
              origin: (payload.origin as string) || "scope",
            });
            close();
            await store.refreshState();
            ui.selectTerm(r.sid);
            store.toast("會話已開啟");
          }}
        >
          開始會話
        </AsyncButton>
      }
    >
      <p className="detailmeta">{"工作範圍：" + sessionTarget(payload)}</p>
      <p className="sumsrc">
        Agent
        開場會拿到：組情境卡（成員與關係、脈動、未結、最近事件）＋文件地圖，並被要求先說明拿到什麼、預計產出什麼、哪些動作要你確認。
      </p>
      <p className="sumsrc">
        工作範圍是行為約定，不是工具層的權限限制——Agent 手上的工具碰得到組外檔案，所以它被要求越界前先問你。
      </p>
      <label className="field">
        使用的 Agent
        <select value={agent} onChange={(e) => setAgent(e.target.value)}>
          {agents.map((a) => (
            <option key={a} value={a}>
              {a}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        這次想完成什麼？
        <textarea
          ref={taskRef}
          rows={4}
          value={task}
          placeholder="留空時，Agent 會先整理這組 repo 的現況。"
          onChange={(e) => setTask(e.target.value)}
        />
      </label>
      <div className="taskhints">
        <span className="small muted">有情境的起手：</span>
        {taskEntries(state, scan, scopeRepos(scope, state)).map(([label, text]) => (
          <button
            key={label}
            type="button"
            className="chip"
            onClick={() => {
              setTask(text);
              taskRef.current?.focus();
            }}
          >
            {label}
          </button>
        ))}
      </div>
    </DetailLayout>
  );
}

/** 記錄結果：載入交接草稿（脊椎留痕組成），人確認後存成一筆 decision。 */
export function SessionResult({ term, after }: { term: TermInfo; after?: () => void }) {
  const store = useStore();
  const { setError, close } = useDetail();
  const ref = useAutoFocus<HTMLTextAreaElement>();
  const [text, setText] = useState("");
  const [note, setNote] = useState("正在整理交接草稿…");
  const touched = useRef(false);
  useEffect(() => {
    let live = true;
    store
      .action("session_draft", { sid: term.sid })
      .then((r) => {
        if (!live) return;
        if (!touched.current) setText((cur) => (cur.trim() ? cur : r.text));
        setNote(r.note + (r.traces ? "" : "（這個會話期間沒有留痕，草稿只有骨架。）"));
      })
      .catch((e: Error) => live && setNote("無法整理草稿：" + e.message + " 可以直接自己填寫。"));
    return () => {
      live = false;
    };
  }, [store, term.sid]);
  return (
    <DetailLayout
      actions={
        <AsyncButton
          className="primary"
          run={async () => {
            if (!text.trim()) {
              setError("請填寫結果。");
              return;
            }
            await store.action("session_result", { sid: term.sid, text: text.trim() });
            close();
            store.toast("結果已儲存（記為 decision，不代表成果已被採納）");
            await store.refreshState();
            after?.();
          }}
        >
          確認並儲存
        </AsyncButton>
      }
    >
      <p className="detailmeta">
        {term.title + " · " + (term.scope || "本機 Shell") + " · " + reportLine(term)}
      </p>
      <p className="sumsrc">{note}</p>
      <label className="field">
        完成了什麼？還有哪些下一步？
        <textarea
          ref={ref}
          rows={9}
          value={text}
          onChange={(e) => {
            touched.current = true;
            setText(e.target.value);
          }}
        />
      </label>
    </DetailLayout>
  );
}

export function sessionResultSpec(term: TermInfo, after?: () => void) {
  return {
    title: "記錄會話結果",
    eyebrow: "SESSION RESULT",
    content: <SessionResult term={term} after={after} />,
  };
}

/** 結束程序前的確認：先記錄結果／保留並收合／結束程序。 */
export function ConfirmKill({ term }: { term: TermInfo }) {
  const store = useStore();
  const ui = useUi();
  const { close } = useDetail();
  return (
    <DetailLayout
      actions={
        <>
          <button
            type="button"
            onClick={() =>
              ui.openDetail(
                sessionResultSpec(term, () => ui.openDetail(confirmKillSpec(term))),
              )
            }
          >
            先記錄結果
          </button>
          <button
            type="button"
            onClick={() => {
              close();
              ui.collapseTerm();
            }}
          >
            保留並收合
          </button>
          <AsyncButton
            className="danger"
            run={async () => {
              await store.action("term_kill", { sid: term.sid });
              close();
              await store.refreshState();
              store.toast("會話程序已結束");
            }}
          >
            結束程序
          </AsyncButton>
        </>
      }
    >
      <p className="dialogmessage">
        {term.title + "\n\n這會停止程序並移除會話。若只想保留工作並收起畫面，請使用「收合」。"}
      </p>
    </DetailLayout>
  );
}

export function confirmKillSpec(term: TermInfo) {
  return { title: "結束這個會話程序？", eyebrow: "SESSION", content: <ConfirmKill term={term} /> };
}
