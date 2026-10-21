import { useEffect, useRef, useState } from "react";
import type { PermissionMode, TermInfo } from "../../api/types";
import { DetailLayout, useAutoFocus, useDetail } from "../../dialogs/Modal";
import { load, save } from "../../lib/storage";
import { reportLine, sessionTarget } from "../../lib/view";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/** 權限模式的選項（F-96；空＝照 Claude Code 自己的設定）。 */
export const PERMISSION_MODES: { value: PermissionMode | ""; label: string }[] = [
  { value: "", label: "照 Claude Code 的設定" },
  { value: "manual", label: "每個工具都問我" },
  { value: "acceptEdits", label: "改檔不問，其他照問" },
  { value: "plan", label: "先提計畫，我同意才動手" },
  { value: "auto", label: "由 Claude Code 判斷要不要問" },
  { value: "dontAsk", label: "沒預先允許的一律拒絕" },
  { value: "bypassPermissions", label: "全部不問（危險）" },
];

/** 模型的常用別名（也可以打全名）。 */
const MODEL_HINTS = ["fable", "opus", "sonnet", "haiku"];

/** 這個 agent 是不是 Claude Code（config 的 key；只有它吃權限模式、模型、worktree）。 */
export function isClaudeAgent(agent: string): boolean {
  return /^claude/i.test(agent);
}

/** 開會話表單：選 Agent、寫任務；範圍在開啟時就固定。 */
export function SessionForm({
  payload,
  task: initialTask,
}: {
  payload: Record<string, unknown>;
  task: string;
}) {
  const store = useStore();
  const ui = useUi();
  const { close } = useDetail();
  const { state, scan } = useSnapshot();
  const agents = state.agents || [];
  const [agent, setAgent] = useState(() => {
    const remembered = load<string>("agent", agents[0] || "claude");
    return agents.includes(remembered) ? remembered : agents[0] || "";
  });
  const [task, setTask] = useState(initialTask);
  const [mode, setMode] = useState<PermissionMode | "">(() => {
    const m = load<string>("permission_mode", "");
    return PERMISSION_MODES.find((x) => x.value === m)?.value ?? "";
  });
  const [model, setModel] = useState(() => load("model", ""));
  const [worktree, setWorktree] = useState(false);
  const claude = isClaudeAgent(agent);
  const single = typeof payload.repo === "string" && !!payload.repo;
  const taskRef = useRef<HTMLTextAreaElement>(null);
  return (
    <DetailLayout
      actions={
        <AsyncButton
          className="primary"
          disabled={!agents.length}
          run={async () => {
            save("agent", agent);
            save("permission_mode", mode);
            save("model", model.trim());
            const r = await store.action("term_create", {
              ...payload,
              kind: "agent",
              agent,
              task: task.trim(),
              origin: (payload.origin as string) || "scope",
              ...(claude
                ? {
                    ...(mode ? { permission_mode: mode } : {}),
                    ...(model.trim() ? { model: model.trim() } : {}),
                    ...(worktree && single ? { worktree: true } : {}),
                  }
                : {}),
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
        工作範圍是行為約定，不是工具層的權限限制——Agent
        手上的工具碰得到組外檔案，所以它被要求越界前先問你。
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
      {claude ? (
        <div className="launchopts">
          <label className="field">
            權限
            <select value={mode} onChange={(e) => setMode(e.target.value as PermissionMode | "")}>
              {PERMISSION_MODES.map((m) => (
                <option key={m.value} value={m.value}>
                  {m.label}
                </option>
              ))}
            </select>
          </label>
          <label className="field">
            模型
            <input
              list="modelhints"
              value={model}
              placeholder="照 Claude Code 的設定"
              onChange={(e) => setModel(e.target.value)}
            />
            <datalist id="modelhints">
              {MODEL_HINTS.map((m) => (
                <option key={m} value={m} />
              ))}
            </datalist>
          </label>
          {single ? (
            <label className="check">
              <input
                type="checkbox"
                checked={worktree}
                onChange={(e) => setWorktree(e.target.checked)}
              />
              在隔離的 git worktree 裡做（不動你目前的工作目錄；做完要自己合併）
            </label>
          ) : null}
        </div>
      ) : null}
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
        {(scan.session_tasks || []).map(({ label, text }) => (
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
  const ui = useUi();
  // 存好之後留在對話框說記在哪、給「到活動看」（UX 檢討流程 C：不能只有一閃而過的提示）
  const [saved, setSaved] = useState<{ date: string; time: string } | null>(null);
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
  if (saved)
    return (
      <DetailLayout
        actions={
          <>
            <button
              type="button"
              onClick={() => {
                close();
                ui.showTab("activity");
              }}
            >
              到活動分頁看
            </button>
            <button type="button" className="primary" onClick={close}>
              關閉
            </button>
          </>
        }
      >
        <p className="dialogmessage" role="status">
          {"已記在活動：" +
            saved.date +
            " " +
            saved.time +
            "（記成一筆判斷 decision，下次開這組的會話時 agent 會讀到；不代表成果已被採納）"}
        </p>
      </DetailLayout>
    );
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
            const r = await store.action("session_result", { sid: term.sid, text: text.trim() });
            await store.refreshState();
            if (after) {
              close();
              store.toast("結果已儲存（記為 decision，不代表成果已被採納）");
              after();
              return;
            }
            setSaved({ date: r.date, time: r.time });
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
              ui.openDetail(sessionResultSpec(term, () => ui.openDetail(confirmKillSpec(term))))
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
