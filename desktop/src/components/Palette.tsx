import { useEffect, useId, useRef, useState } from "react";
import type { PaletteResult } from "../api/types";
import { useStore, useUi } from "../store/context";

/** 打字後多久才問引擎（毫秒）——邊打邊找，不必每個字都問。 */
const DEBOUNCE_MS = 120;

type FileHit = NonNullable<PaletteResult["files"]>[number];
type Item =
  | { kind: "repo"; key: string; hit: PaletteResult["repos"][number] }
  | { kind: "group"; key: string; group: PaletteResult["groups"][number] }
  | { kind: "file"; key: string; file: FileHit };

function itemsOf(r: PaletteResult | null): Item[] {
  if (!r) return [];
  // 牌組在前：組是工作單位、數量少；repo 次之；檔案最後
  return [
    ...r.groups.map((group) => ({ kind: "group" as const, key: "g:" + group.name, group })),
    ...r.repos.map((hit) => ({ kind: "repo" as const, key: "r:" + hit.repo, hit })),
    ...(r.files || []).map((file) => ({
      kind: "file" as const,
      key: "f:" + file.repo + ":" + file.path,
      file,
    })),
  ];
}

/**
 * ⌘K：打字找 repo（名稱片段、縮寫、名片的角色／介紹／關鍵字）、牌組、檔案（目前範圍內，用檔名）。
 * 排名全在引擎（`palette` action），這裡只畫。
 * Enter＝在右側欄開 repo／切到牌組／開檔；⌘Enter（Ctrl+Enter）＝對 repo 開 Agent 會話／把檔案加入挑選；Esc 關。
 * 鍵盤操作照 WAI-ARIA combobox：焦點留在輸入框，↑↓ 移動 aria-activedescendant。
 */
export function Palette({ onClose }: { onClose: () => void }) {
  const store = useStore();
  const ui = useUi();
  const listId = useId();
  const input = useRef<HTMLInputElement>(null);
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<PaletteResult | null>(null);
  const [error, setError] = useState("");
  const [active, setActive] = useState(0);

  // 開的時候記下焦點在哪；取消（Esc、點外面）就還回去。選了東西不還——焦點交給打開的側欄或對話框
  const prevFocus = useRef<Element | null>(null);
  useEffect(() => {
    prevFocus.current = document.activeElement;
    input.current?.focus();
  }, []);
  function cancel() {
    onClose();
    (prevFocus.current as HTMLElement | null)?.focus?.();
  }

  // 問引擎；晚回來的舊查詢不蓋掉新的。失敗保留上次的清單並顯示原因。
  useEffect(() => {
    let live = true;
    const t = setTimeout(
      () => {
        store
          .action("palette", { query, ...store.scopePayload })
          .then((r) => {
            if (!live) return;
            setResult(r);
            setError("");
            setActive(0);
          })
          .catch((e: Error) => live && setError(e.message));
      },
      query ? DEBOUNCE_MS : 0,
    );
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [store, query]);

  const items = itemsOf(result);
  const cur = items[Math.min(active, items.length - 1)];

  function choose(item: Item | undefined, session: boolean) {
    if (!item) return;
    onClose();
    if (item.kind === "group") {
      store.setScope({ group: item.group.name, repos: null });
    } else if (item.kind === "file") {
      const f = item.file;
      if (session) {
        ui.togglePick({ repo: f.repo, path: f.path, name: f.name });
        store.toast("已加入挑選：" + f.repo + " / " + f.path);
      } else ui.openFile(f.repo, f.path);
    } else if (session) {
      const id = item.hit.repo;
      ui.openSession({ repo: id, repos: [id], origin: "repo:" + id });
    } else {
      ui.showRepo(item.hit.repo);
    }
  }

  return (
    <div
      id="palette"
      className="palettebackdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) cancel();
      }}
    >
      <div className="palette" role="dialog" aria-modal="true" aria-label="找 repo 或牌組">
        <input
          ref={input}
          type="text"
          role="combobox"
          aria-label="找 repo 或牌組"
          aria-expanded="true"
          aria-controls={listId}
          aria-activedescendant={cur ? listId + "-" + cur.key : undefined}
          aria-autocomplete="list"
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          // macOS WebView 的行內輸入建議（灰字＋×）會蓋住結果
          {...{ writingsuggestions: "false" }}
          placeholder="找 repo、牌組或檔案：名稱、角色、介紹、關鍵字、檔名…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === "Escape") {
              e.preventDefault();
              cancel();
            } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
              e.preventDefault();
              if (!items.length) return;
              const d = e.key === "ArrowDown" ? 1 : -1;
              setActive((a) => (Math.min(a, items.length - 1) + d + items.length) % items.length);
            } else if (e.key === "Enter") {
              e.preventDefault();
              choose(cur, e.metaKey || e.ctrlKey);
            }
          }}
        />
        {error ? (
          <p className="error small" role="alert">
            找不到資料：{error}
          </p>
        ) : null}
        <ul id={listId} role="listbox" aria-label="結果" className="palettelist">
          {items.map((it) => (
            <li
              key={it.key}
              id={listId + "-" + it.key}
              role="option"
              aria-selected={it === cur}
              className={it === cur ? "on" : undefined}
              onMouseMove={() => setActive(items.indexOf(it))}
              onClick={(e) => choose(it, e.metaKey || e.ctrlKey)}
            >
              {it.kind === "repo" ? (
                <RepoRow hit={it.hit} />
              ) : it.kind === "group" ? (
                <GroupRow group={it.group} />
              ) : (
                <FileRow file={it.file} />
              )}
            </li>
          ))}
        </ul>
        {result && !items.length ? (
          <p className="muted small paletteempty">沒有符合「{query}」的 repo、牌組或檔案。</p>
        ) : null}
        <p className="palettehint small muted">
          <span>↑↓ 選擇</span>
          <span>Enter 開 repo 側欄／切牌組／開檔</span>
          <span>⌘/Ctrl + Enter 開 Agent 會話／檔案加入挑選</span>
          <span>Esc 關閉</span>
        </p>
      </div>
    </div>
  );
}

function RepoRow({ hit }: { hit: PaletteResult["repos"][number] }) {
  return (
    <>
      <div className="paletterow">
        <span className="palettename">{hit.repo}</span>
        {hit.role ? <span className="pill">{hit.role}</span> : null}
        <span className="spacer" />
        {hit.groups.length ? <span className="small muted">{hit.groups.join("、")}</span> : null}
      </div>
      {hit.description ? <div className="palettedesc small muted">{hit.description}</div> : null}
      {hit.why.length ? <div className="palettewhy small muted">{hit.why.join("・")}</div> : null}
    </>
  );
}

function GroupRow({ group }: { group: PaletteResult["groups"][number] }) {
  return (
    <>
      <div className="paletterow">
        <span className="palettekind small muted">牌組</span>
        <span className="palettename">{group.name}</span>
        <span className="spacer" />
        <span className="small muted">{group.members} 個 repo</span>
      </div>
      {group.goal ? <div className="palettedesc small muted">目標：{group.goal}</div> : null}
    </>
  );
}

function FileRow({ file }: { file: FileHit }) {
  return (
    <>
      <div className="paletterow">
        <span className="palettekind small muted">檔案</span>
        <span className="palettename">{file.name}</span>
        <span className="spacer" />
        <span className="small muted">{file.repo}</span>
      </div>
      <div className="palettedesc small muted" title={file.abs}>
        {file.path}
      </div>
    </>
  );
}
