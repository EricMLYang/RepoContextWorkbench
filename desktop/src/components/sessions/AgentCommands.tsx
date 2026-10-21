import { useEffect, useMemo, useState } from "react";
import type { AgentCommand } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { useStore, useUi } from "../../store/context";

const SOURCE_LABEL: Record<AgentCommand["source"], string> = {
  project: "這個 repo",
  user: "我的",
  plugin: "plugin",
};

/** 依名稱與說明篩（不分大小寫，空白分開的每個字都要有）。 */
export function filterCommands(list: AgentCommand[], query: string): AgentCommand[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return list.filter((c) => {
    const hay = (c.name + " " + c.description).toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

/**
 * 可用的 `/指令`（F-105）：這個會話打得出來的 skill、自訂指令、plugin 指令。
 * 點一個把 `/名字 ` 貼進會話（不送出，後面可以接著打參數）。
 */
export function AgentCommands({ sid }: { sid: string }) {
  const store = useStore();
  const ui = useUi();
  const { close, setError } = useDetail();
  const [list, setList] = useState<AgentCommand[] | null>(null);
  const [error, setLoadError] = useState("");
  const [query, setQuery] = useState("");
  useEffect(() => {
    let alive = true;
    store
      .action("agent_commands", { sid })
      .then((r) => alive && setList(r.commands))
      .catch((e: Error) => alive && setLoadError(e.message));
    return () => {
      alive = false;
    };
  }, [store, sid]);
  const shown = useMemo(() => filterCommands(list || [], query), [list, query]);
  return (
    <DetailLayout
      actions={
        <button type="button" className="primary" onClick={close}>
          關閉
        </button>
      }
    >
      <input
        className="pastsearch"
        type="search"
        autoFocus
        value={query}
        placeholder="找指令：名稱或說明…"
        aria-label="找指令"
        onChange={(e) => setQuery(e.target.value)}
      />
      {error ? <p className="error">{"讀不到指令：" + error}</p> : null}
      {!list && !error ? <p className="muted">讀取中…</p> : null}
      {list && !shown.length ? (
        <p className="muted">
          {query ? "沒有符合的指令。" : "這裡沒有找到任何 skill 或自訂指令。"}
        </p>
      ) : null}
      <ul className="cmdlist">
        {shown.map((c) => (
          <li key={c.name}>
            <button
              type="button"
              className="ghost cmdrow"
              title="貼進會話（不送出）"
              onClick={() => {
                const t = ui.pasteToSession("/" + c.name + " ");
                if (!t) {
                  setError("先在下方終端選一個執行中的會話");
                  return;
                }
                store.toast("已貼到「" + t + "」：/" + c.name);
                close();
              }}
            >
              <span className="cmdname">/{c.name}</span>
              <span className="pill small">
                {SOURCE_LABEL[c.source]}
                {c.kind === "skill" ? " · skill" : ""}
              </span>
              <span className="small muted cmddesc">{c.description}</span>
            </button>
          </li>
        ))}
      </ul>
    </DetailLayout>
  );
}

export function agentCommandsSpec(sid: string) {
  return { title: "插入 /指令", eyebrow: "COMMANDS", content: <AgentCommands sid={sid} /> };
}
