import { useEffect, useMemo, useState } from "react";
import type { TranscriptFrame } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { copyText } from "../../lib/clipboard";
import { useStore, useUi } from "../../store/context";

type Filter = "all" | "you" | "reply" | "tool";

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "全部" },
  { key: "you", label: "我說的" },
  { key: "reply", label: "回覆" },
  { key: "tool", label: "工具" },
];

const KIND_LABEL: Record<TranscriptFrame["kind"], string> = {
  you: "你",
  reply: "回覆",
  thinking: "思考",
  tool: "工具",
  compact: "壓縮摘要",
};

/** 逐字稿的 UTC 時間 → 本地「HH:MM」；同一天以外帶日期。 */
export function frameTime(at: string | null, now = new Date()): string {
  if (!at) return "";
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  const hm = pad(d.getHours()) + ":" + pad(d.getMinutes());
  return d.toDateString() === now.toDateString()
    ? hm
    : d.getMonth() + 1 + "/" + d.getDate() + " " + hm;
}

/** 這一框在這個篩選下要不要列（思考、壓縮摘要只在「全部」）。 */
export function frameShown(f: TranscriptFrame, filter: Filter): boolean {
  return filter === "all" || f.kind === filter;
}

/**
 * 會話逐字稿（F-94）：Claude Code 自己寫的完整紀錄，分成你說的／回覆／思考／工具（附結果）。
 * 終端捲掉的、App 重開前的都看得到。「我說的」每一句可貼回目前的會話（F-104，不按 Enter）。
 */
export function Transcript(props: { sid?: string; id?: string; cwd?: string }) {
  const { sid, id, cwd } = props;
  const store = useStore();
  const ui = useUi();
  const { close, setError } = useDetail();
  const [data, setData] = useState<{
    frames: TranscriptFrame[];
    truncated: boolean;
    path: string;
  } | null>(null);
  const [error, setLoadError] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [reload, setReload] = useState(0);
  useEffect(() => {
    let alive = true;
    store
      .action("session_transcript", sid ? { sid } : { id, cwd })
      .then((r) => {
        if (!alive) return;
        setData(r);
        setLoadError("");
      })
      .catch((e: Error) => alive && setLoadError(e.message));
    return () => {
      alive = false;
    };
  }, [store, sid, id, cwd, reload]);
  const shown = useMemo(
    () => (data ? data.frames.filter((f) => frameShown(f, filter)) : []),
    [data, filter],
  );
  const youCount = data ? data.frames.filter((f) => f.kind === "you").length : 0;
  return (
    <DetailLayout
      actions={
        <>
          <button type="button" onClick={() => setReload((n) => n + 1)}>
            重新讀取
          </button>
          <button
            type="button"
            disabled={!data}
            onClick={async () => {
              if (!data) return;
              try {
                await copyText(store.backend, data.path);
                store.toast("已複製逐字稿路徑");
              } catch {
                setError("無法存取剪貼簿");
              }
            }}
          >
            複製檔案路徑
          </button>
          <button type="button" className="primary" onClick={close}>
            關閉
          </button>
        </>
      }
    >
      <div className="txfilters" role="group" aria-label="篩選逐字稿">
        {FILTERS.map((f) => (
          <button
            key={f.key}
            type="button"
            className={"ghost small" + (filter === f.key ? " on" : "")}
            aria-pressed={filter === f.key}
            onClick={() => setFilter(f.key)}
          >
            {f.label}
            {f.key === "you" && data ? ` ${youCount}` : ""}
          </button>
        ))}
      </div>
      {error ? <p className="error">{"讀不到逐字稿：" + error}</p> : null}
      {!data && !error ? <p className="muted">讀取中…</p> : null}
      {data?.truncated ? (
        <p className="small muted">逐字稿很長，只列最後一段。完整檔案：{data.path}</p>
      ) : null}
      {data && !shown.length ? <p className="muted">這個篩選下沒有內容。</p> : null}
      <ol className="txlist" aria-label="逐字稿">
        {shown.map((f, i) => (
          <li key={i} className={"txf tx-" + f.kind + (f.error ? " txerr" : "")}>
            <div className="txhead small">
              <span className="txkind">{f.kind === "tool" ? f.tool : KIND_LABEL[f.kind]}</span>
              <span className="muted">{frameTime(f.at)}</span>
              {f.kind === "you" ? (
                <button
                  type="button"
                  className="ghost small txpaste"
                  title="貼到下方終端目前的會話（不按 Enter）"
                  onClick={() => {
                    const t = ui.pasteToSession(f.text);
                    if (t) store.toast("已貼到「" + t + "」");
                    else setError("先在下方終端選一個執行中的會話，再貼回去");
                  }}
                >
                  貼回會話
                </button>
              ) : null}
            </div>
            {f.kind === "thinking" || f.kind === "compact" ? (
              <details>
                <summary className="small muted">
                  {f.kind === "compact" ? "context 滿了，從這裡接續（展開看摘要）" : "展開思考"}
                </summary>
                <div className="txtext">{f.text}</div>
              </details>
            ) : f.kind === "tool" ? (
              <>
                <code className="txtool">{f.text || "（沒有參數）"}</code>
                {f.result ? (
                  <details>
                    <summary className="small muted">{f.error ? "錯誤結果" : "結果"}</summary>
                    <pre className="txresult">{f.result}</pre>
                  </details>
                ) : null}
              </>
            ) : (
              <div className="txtext">{f.text}</div>
            )}
          </li>
        ))}
      </ol>
    </DetailLayout>
  );
}

export function transcriptSpec(target: { sid?: string; id?: string; cwd?: string }, title: string) {
  return {
    title: "逐字稿：" + title,
    eyebrow: "TRANSCRIPT",
    content: <Transcript {...target} />,
  };
}
