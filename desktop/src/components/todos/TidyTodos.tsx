import { useCallback, useEffect, useState } from "react";
import type { TidyBucket, TidyItem, TidyResult } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/** 各類的標題與說明（分類本身由引擎算，這裡只排版）。 */
const BUCKETS: { key: TidyBucket; title: string; hint: string }[] = [
  { key: "done", title: "可能已做完", hint: "之後的 commit／判斷看起來已經做掉，附一筆證據" },
  { key: "shelved", title: "擱置太久", hint: "逾期超過兩週還沒結，或一直沒人提到" },
  { key: "ask", title: "等你決定", hint: "不是要做的事，是在等你回答的問題" },
  { key: "live", title: "還在進行", hint: "還沒到期，或最近有動靜" },
];

/** 預設勾起來的：引擎建議可以關的（可能已做完＋擱置太久）。 */
const preselect = (r: TidyResult) => new Set(r.suggest_close);

/**
 * 清理未結（2026-11-10）：目前範圍的未結分四類，勾了一次關掉；也可以交給 Agent 整理後再問你。
 * 未結幾乎都是 agent 開的、期限是自動加的，沒有一個地方能掃一眼整批清——這裡就是那個地方。
 */
export function TidyTodos() {
  const store = useStore();
  const ui = useUi();
  const { close, setError } = useDetail();
  const [data, setData] = useState<TidyResult | null>(null);
  const [failed, setFailed] = useState(false);
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [note, setNote] = useState("");
  const [result, setResult] = useState("");
  const scope = JSON.stringify(store.scopePayload);

  const fetchTidy = useCallback(
    () => store.action("todo_tidy", JSON.parse(scope) as object),
    [store, scope],
  );
  const loadFailed = useCallback(
    (e: unknown) => {
      // 失敗時保留上次的清單（斷線不清畫面）
      setFailed(true);
      setError("讀不到未結清單：" + (e as Error).message);
    },
    [setError],
  );
  useEffect(() => {
    let alive = true;
    fetchTidy()
      .then((r) => {
        if (!alive) return;
        setData(r);
        setFailed(false);
        setPicked(preselect(r));
      })
      .catch((e) => alive && loadFailed(e));
    return () => {
      alive = false;
    };
  }, [fetchTidy, loadFailed]);
  /** 關完之後重讀：還勾著的只留還開著的。 */
  const reload = async () => {
    try {
      const r = await fetchTidy();
      setData(r);
      setFailed(false);
      const open = new Set(r.items.map((it) => it.id));
      setPicked((cur) => new Set([...cur].filter((id) => open.has(id))));
    } catch (e) {
      loadFailed(e);
    }
  };

  const toggle = (id: string) =>
    setPicked((cur) => {
      const next = new Set(cur);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const setBucket = (items: TidyItem[], on: boolean) =>
    setPicked((cur) => {
      const next = new Set(cur);
      for (const it of items) {
        if (on) next.add(it.id);
        else next.delete(it.id);
      }
      return next;
    });

  const closePicked = async () => {
    const nums = [...picked];
    const r = await store.action("close_loops", { nums, note: note.trim() || undefined });
    setResult(
      `已關閉 ${r.closed.length} 項` +
        (r.failed.length
          ? `；${r.failed.length} 項沒關成：${r.failed.map((f) => f.id + " " + f.error).join("；")}`
          : ""),
    );
    setNote("");
    await reload();
    void store.refreshState();
  };

  const c = data?.counts;
  return (
    <DetailLayout
      actions={
        <>
          <AsyncButton
            className="ghost"
            title="開一個 Agent 會話：對照 repo 現況逐項判斷，列出建議關掉與留下的，你同意後才關"
            run={async () => {
              const r = await store.action("todo_tidy_brief", JSON.parse(scope) as object);
              close();
              ui.openSession({ task: r.text, origin: "todo-tidy" });
            }}
          >
            請 Agent 整理…
          </AsyncButton>
          <button type="button" onClick={close}>
            關閉
          </button>
          <AsyncButton className="primary" disabled={!picked.size} run={closePicked}>
            {`關掉勾選的 ${picked.size} 項`}
          </AsyncButton>
        </>
      }
    >
      {data === null && !failed ? <p className="muted">讀取中…</p> : null}
      {c ? (
        <p className="small muted" id="tidycounts">
          {`${data.scope.label}：共 ${c.total} 項——可能已做完 ${c.done}、擱置太久 ${c.shelved}、等你決定 ${c.ask}、還在進行 ${c.live}。` +
            "關掉只是在 spine 記一筆「已結」，原本的內容都還查得到。"}
        </p>
      ) : null}
      {result ? (
        <p className="small" role="status">
          {result}
        </p>
      ) : null}
      {data && !data.items.length ? <p className="muted">這個範圍沒有未結事項。</p> : null}
      <div className="tidylist">
        {BUCKETS.map((b) => {
          const items = (data?.items || []).filter((it) => it.bucket === b.key);
          if (!items.length) return null;
          const all = items.every((it) => picked.has(it.id));
          return (
            <details key={b.key} className="tidybucket" open={b.key !== "live"}>
              <summary>
                <span className="tidytitle">{`${b.title} · ${items.length}`}</span>
                <span className="small muted">{b.hint}</span>
                <button
                  type="button"
                  className="ghost small"
                  onClick={(e) => {
                    e.preventDefault();
                    setBucket(items, !all);
                  }}
                >
                  {all ? "全不選" : "全選"}
                </button>
              </summary>
              <ul>
                {items.map((it) => (
                  <li key={it.id}>
                    <label className="tidyrow">
                      <input
                        type="checkbox"
                        checked={picked.has(it.id)}
                        onChange={() => toggle(it.id)}
                      />
                      <span className="tidycopy">
                        <span className="tidytext" title={it.text}>
                          <span className="muted">{it.id}</span> {it.text}
                        </span>
                        <span className="small muted">
                          {[
                            it.due ? "due " + it.due + (it.overdue ? "（已逾期）" : "") : "",
                            `開了 ${it.age_days} 天`,
                            it.evidence[0]
                              ? "證據：" + it.evidence[0].ref + " " + it.evidence[0].text
                              : it.why,
                          ]
                            .filter(Boolean)
                            .join(" · ")}
                        </span>
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
            </details>
          );
        })}
      </div>
      {data?.items.length ? (
        <input
          className="tidynote"
          type="text"
          value={note}
          placeholder="怎麼結的（選填，整批共用；沒填＝從工作台清理）"
          aria-label="怎麼結的"
          onChange={(e) => setNote(e.target.value)}
        />
      ) : null}
    </DetailLayout>
  );
}

export function tidyTodosSpec() {
  return { title: "清理未結", eyebrow: "OPEN LOOPS", content: <TidyTodos /> };
}
