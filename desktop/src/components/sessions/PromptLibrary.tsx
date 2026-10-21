import { useCallback, useEffect, useMemo, useState } from "react";
import type { SavedPrompt } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/** 依名稱與內容篩（不分大小寫，每個字都要有）。 */
export function filterPrompts(list: SavedPrompt[], query: string): SavedPrompt[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  return list.filter((p) => words.every((w) => (p.name + " " + p.text).toLowerCase().includes(w)));
}

/**
 * 常用 prompt（F-106）：存在 spine 的 `prompts/*.md`（人可以直接改、跟 spine 一起有版本）。
 * 挑一句貼進目前的會話（不送出）；也能新增、改、刪。`draft` 有給就直接打開新增表單（從逐字稿「存成常用」來的）。
 */
export function PromptLibrary({ draft }: { draft?: string }) {
  const store = useStore();
  const ui = useUi();
  const { close, setError } = useDetail();
  const [list, setList] = useState<SavedPrompt[] | null>(null);
  const [dir, setDir] = useState("");
  const [loadError, setLoadError] = useState("");
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<{
    from: string | null;
    name: string;
    text: string;
  } | null>(draft ? { from: null, name: "", text: draft } : null);
  const [armed, setArmed] = useState<string | null>(null);
  const load = useCallback(async () => {
    try {
      const r = await store.action("prompts", {});
      setList(r.prompts);
      setDir(r.dir);
      setLoadError("");
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : String(e));
    }
  }, [store]);
  useEffect(() => {
    let alive = true;
    store
      .action("prompts", {})
      .then((r) => {
        if (!alive) return;
        setList(r.prompts);
        setDir(r.dir);
      })
      .catch((e: Error) => alive && setLoadError(e.message));
    return () => {
      alive = false;
    };
  }, [store]);
  const shown = useMemo(() => filterPrompts(list || [], query), [list, query]);

  if (editing)
    return (
      <DetailLayout
        actions={
          <>
            <button type="button" onClick={() => setEditing(null)}>
              取消
            </button>
            <AsyncButton
              className="primary"
              disabled={!editing.name.trim() || !editing.text.trim()}
              run={async () => {
                await store.action("prompt_save", {
                  name: editing.name.trim(),
                  text: editing.text,
                  ...(editing.from ? { rename_from: editing.from } : {}),
                });
                store.toast("已存：" + editing.name.trim());
                setEditing(null);
                await load();
              }}
            >
              存
            </AsyncButton>
          </>
        }
      >
        <label className="field">
          名稱（也是檔名）
          <input
            autoFocus
            value={editing.name}
            placeholder="例：做下一步"
            onChange={(e) => setEditing({ ...editing, name: e.target.value })}
          />
        </label>
        <label className="field">
          內容
          <textarea
            rows={6}
            value={editing.text}
            onChange={(e) => setEditing({ ...editing, text: e.target.value })}
          />
        </label>
      </DetailLayout>
    );

  return (
    <DetailLayout
      actions={
        <>
          <button type="button" onClick={() => setEditing({ from: null, name: "", text: "" })}>
            新增…
          </button>
          <button type="button" className="primary" onClick={close}>
            關閉
          </button>
        </>
      }
    >
      <input
        className="pastsearch"
        type="search"
        autoFocus
        value={query}
        placeholder="找一句：名稱或內容…"
        aria-label="找常用 prompt"
        onChange={(e) => setQuery(e.target.value)}
      />
      {loadError ? <p className="error">{"讀不到常用 prompt：" + loadError}</p> : null}
      {list && !list.length ? (
        <p className="muted">
          還沒有常用 prompt。按「新增…」，或在逐字稿「我說的」那句按「存成常用」。存在 {dir}
        </p>
      ) : null}
      <ul className="promptlist">
        {shown.map((p) => (
          <li key={p.name} className="promptrow">
            <div className="prompthead">
              <strong>{p.name}</strong>
              <span className="small muted">{p.updated}</span>
              <span className="spacer" />
              <button
                type="button"
                className="small"
                title="貼進目前的會話（不送出）"
                onClick={() => {
                  const t = ui.pasteToSession(p.text);
                  if (!t) {
                    setError("先在下方終端選一個執行中的會話");
                    return;
                  }
                  store.toast("已貼到「" + t + "」");
                  close();
                }}
              >
                貼進會話
              </button>
              <button
                type="button"
                className="ghost small"
                onClick={() => setEditing({ from: p.name, name: p.name, text: p.text })}
              >
                改
              </button>
              {armed === p.name ? (
                <AsyncButton
                  className="small danger"
                  run={async () => {
                    await store.action("prompt_remove", { name: p.name });
                    setArmed(null);
                    await load();
                  }}
                >
                  確定刪除
                </AsyncButton>
              ) : (
                <button type="button" className="ghost small" onClick={() => setArmed(p.name)}>
                  刪除…
                </button>
              )}
            </div>
            <div className="small muted prompttext">{p.text}</div>
          </li>
        ))}
      </ul>
    </DetailLayout>
  );
}

export function promptLibrarySpec(draft?: string) {
  return {
    title: draft ? "存成常用 prompt" : "常用 prompt",
    eyebrow: "PROMPTS",
    content: <PromptLibrary draft={draft} />,
  };
}
