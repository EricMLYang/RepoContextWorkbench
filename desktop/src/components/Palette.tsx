import { useEffect, useId, useRef, useState } from "react";
import type { PaletteResult, TermInfo } from "../api/types";
import { STATUS_TXT, sessionNeedsYou, sessionWhere } from "../lib/view";
import { useSnapshot, useStore, useUi } from "../store/context";

/** 打字後多久才問引擎（毫秒）——邊打邊找，不必每個字都問。 */
const DEBOUNCE_MS = 120;

type FileHit = NonNullable<PaletteResult["files"]>[number];
/** 一次最多列幾個會話。 */
const SESSIONS_SHOWN = 6;

/** ⌘K 的指令（畫面動作，前端定義）。 */
interface Command {
  id: string;
  label: string;
  /** 找得到這個指令的別名（例「note」「shell」） */
  keys: string;
  run: () => void | Promise<void>;
}

type Item =
  | { kind: "command"; key: string; cmd: Command }
  | { kind: "session"; key: string; term: TermInfo }
  | { kind: "repo"; key: string; hit: PaletteResult["repos"][number] }
  | { kind: "group"; key: string; group: PaletteResult["groups"][number] }
  | { kind: "file"; key: string; file: FileHit };

/**
 * 執行中的 agent 會話（畫面已經有的資料，只做文字過濾）：空查詢＝全部、在等你的在前；
 * 有查詢＝標題、任務、所在（組 › repo）包含查詢的。
 */
function sessionsOf(terms: TermInfo[], query: string): Item[] {
  const q = query.trim().toLocaleLowerCase();
  const live = terms.filter((t) => t.alive && t.kind === "agent");
  const hit = q
    ? live.filter((t) =>
        [t.title, t.task || "", sessionWhere(t).label].some((x) =>
          x.toLocaleLowerCase().includes(q),
        ),
      )
    : [...live].sort((a, b) => Number(sessionNeedsYou(b)) - Number(sessionNeedsYou(a)));
  return hit
    .slice(0, SESSIONS_SHOWN)
    .map((term) => ({ kind: "session" as const, key: "s:" + term.sid, term }));
}

/**
 * 指令：查詢以「>」開頭＝列出全部（像 VS Code）；否則打兩個字以上、名稱或別名包含查詢的才列（免得擠掉 repo）。
 */
function commandsOf(cmds: Command[], query: string): Item[] {
  const q = query.trim().toLocaleLowerCase();
  const all = q.startsWith(">");
  const qq = all ? q.slice(1).trim() : q;
  // 沒有「>」時至少打兩個字才比對（一個字母會中一堆別名）
  if (!all && [...qq].length < 2) return [];
  return cmds
    .filter((c) => !qq || (c.label + " " + c.keys).toLocaleLowerCase().includes(qq))
    .map((cmd) => ({ kind: "command" as const, key: "c:" + cmd.id, cmd }));
}

function itemsOf(r: PaletteResult | null, sessions: Item[], commands: Item[]): Item[] {
  if (!r) return [...commands, ...sessions];
  // 指令（打了才出現）、會話（切過去最常用）；牌組是工作單位、數量少；repo 次之；檔案最後
  return [
    ...commands,
    ...sessions,
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
 * ⌘K：打字找執行中的 agent 會話（切過去）、repo（名稱片段、縮寫、名片的角色／介紹／關鍵字）、牌組、
 * 檔案（目前範圍內，用檔名）。
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

  const { state } = useSnapshot();
  const commands: Command[] = [
    {
      id: "note",
      label: "寫組筆記（目前牌組）",
      keys: "note 筆記 capture",
      run: () => ui.showTab("notes"),
    },
    {
      id: "idea",
      label: "請 Agent 分析想法…",
      keys: "idea collide 想法",
      run: () => ui.showComposer(),
    },
    {
      id: "session",
      label: "開這個範圍的 Agent 會話",
      keys: "agent session claude 會話",
      run: () => ui.openSession({ origin: "palette" }),
    },
    {
      id: "shell",
      label: "開一個 Shell",
      keys: "shell terminal 終端",
      run: async () => {
        const r = await store.action("term_create", { kind: "shell" });
        await store.refreshState();
        ui.selectTerm(r.sid);
      },
    },
    {
      id: "refresh",
      label: "更新狀態（重新採集）",
      keys: "refresh rescan 更新",
      run: () => store.refreshAll(),
    },
    ...(
      [
        ["inbox", "待處理"],
        ["notes", "筆記"],
        ["repos", "Repo"],
        ["files", "檔案"],
        ["activity", "活動"],
        ["relations", "關係"],
      ] as const
    ).map(([tab, name]) => ({
      id: "tab:" + tab,
      label: "切到「" + name + "」分頁",
      keys: "tab 分頁 " + tab,
      run: () => ui.showTab(tab),
    })),
  ];
  // 「>」開頭＝只看指令（不列會話、repo、檔案）
  const commandMode = query.trim().startsWith(">");
  const items = commandMode
    ? commandsOf(commands, query)
    : itemsOf(result, sessionsOf(state.terms || [], query), commandsOf(commands, query));
  const cur = items[Math.min(active, items.length - 1)];

  function choose(item: Item | undefined, session: boolean) {
    if (!item) return;
    onClose();
    if (item.kind === "command") {
      void Promise.resolve(item.cmd.run()).catch((e: unknown) =>
        store.toast(e instanceof Error ? e.message : String(e)),
      );
    } else if (item.kind === "session") {
      ui.selectTerm(item.term.sid);
    } else if (item.kind === "group") {
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
      <div className="palette" role="dialog" aria-modal="true" aria-label="搜尋或執行指令">
        <input
          ref={input}
          type="text"
          role="combobox"
          aria-label="搜尋或執行指令"
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
          placeholder="找會話、repo、牌組或檔案：名稱、角色、介紹、關鍵字、檔名…"
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
              {it.kind === "command" ? (
                <div className="paletterow">
                  <span className="palettekind small muted">指令</span>
                  <span className="palettename">{it.cmd.label}</span>
                </div>
              ) : it.kind === "session" ? (
                <SessionItem term={it.term} />
              ) : it.kind === "repo" ? (
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
          <p className="muted small paletteempty">沒有符合「{query}」的會話、repo、牌組或檔案。</p>
        ) : null}
        <p className="palettehint small muted">
          <span>↑↓ 選擇</span>
          <span>Enter 切到會話／開 repo 側欄／切牌組／開檔</span>
          <span>⌘/Ctrl + Enter 開 Agent 會話／檔案加入挑選</span>
          <span>「&gt;」只看指令</span>
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

function SessionItem({ term }: { term: TermInfo }) {
  const needs = sessionNeedsYou(term);
  return (
    <>
      <div className="paletterow">
        <span className="palettekind small muted">會話</span>
        <span className="palettename">{term.title}</span>
        {needs ? (
          <span className="pill palettewait">{STATUS_TXT[term.agent_status!.status]}</span>
        ) : null}
        <span className="spacer" />
        <span className="small muted">{sessionWhere(term).label}</span>
      </div>
      {needs && term.agent_status?.text ? (
        <div className="palettedesc small muted">{term.agent_status.text}</div>
      ) : term.task ? (
        <div className="palettedesc small muted">任務：{term.task}</div>
      ) : null}
    </>
  );
}
