import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";
import type { TreeEntry } from "../../api/types";
import { load as loadPref, save as savePref } from "../../lib/storage";
import { parentDir, useRepoChanged, useWatchDirs } from "../../lib/treeWatch";
import { FILE_DRAG_TYPE } from "../../lib/view";
import { samePick, useStore, useUi, useUiState, type Pick } from "../../store/context";
import { ContextMenu, type MenuItem } from "../ContextMenu";

interface DirState {
  entries?: TreeEntry[];
  more?: number;
  loading?: boolean;
  error?: string;
}

/** 一層縮排幾 px（跟 VS Code 預設差不多）。 */
const INDENT = 12;
const BASE = 6;

/** 檔名 → 小圖示（Seti 風格的彩色字樣；資料夾只用箭頭，跟 VS Code 預設一樣）。 */
const ICONS: [RegExp, string, string][] = [
  [/\.(tsx?|mts|cts)$/i, "TS", "#519aba"],
  [/\.(jsx?|mjs|cjs)$/i, "JS", "#cbcb41"],
  [/\.rs$/i, "RS", "#e37933"],
  [/\.py$/i, "PY", "#5a9fd4"],
  [/\.go$/i, "GO", "#519aba"],
  [/\.(java|kt|kts)$/i, "J", "#cc3e44"],
  [/\.(c|h|cc|cpp|hpp)$/i, "C", "#a074c4"],
  [/\.cs$/i, "C#", "#8dc149"],
  [/\.(md|mdx|markdown)$/i, "M↓", "#519aba"],
  [/\.json5?$/i, "{}", "#cbcb41"],
  [/\.(ya?ml)$/i, "Y", "#a074c4"],
  [/\.toml$/i, "T", "#9aa4b2"],
  [/\.(css|scss|less)$/i, "#", "#519aba"],
  [/\.html?$/i, "<>", "#e37933"],
  [/\.(sh|zsh|bash|fish|ps1|bat|cmd)$/i, "$", "#8dc149"],
  [/\.(png|jpe?g|gif|webp|svg|ico|bmp|avif)$/i, "▣", "#a074c4"],
  [/\.(lock|sum)$/i, "◆", "#7f8a99"],
  [/^\.git(ignore|attributes|modules)$/i, "G", "#e37933"],
  [/^(dockerfile|.*\.dockerfile)$/i, "D", "#519aba"],
  [/\.(txt|log|csv|tsv)$/i, "≡", "#9aa4b2"],
];

function FileIcon({ name }: { name: string }) {
  const hit = ICONS.find(([re]) => re.test(name));
  return (
    <span className="ticon" aria-hidden="true" style={hit ? { color: hit[2] } : undefined}>
      {hit ? hit[1] : "≡"}
    </span>
  );
}

function Chevron({ open }: { open: boolean }) {
  return (
    <svg className={"tchev" + (open ? " open" : "")} viewBox="0 0 16 16" aria-hidden="true">
      <path d="M6 4l4 4-4 4" fill="none" stroke="currentColor" strokeWidth="1.5" />
    </svg>
  );
}

/** 複製路徑：成功／失敗都用 toast 說。 */
function useCopy() {
  const store = useStore();
  return async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      store.toast("已複製" + label + "：" + text);
    } catch {
      store.toast("無法複製到剪貼簿");
    }
  };
}

/** 畫面上的一列（攤平成一維，跟 VS Code 一樣用 aria-level 表示層級）。 */
type Row =
  | { kind: "entry"; e: TreeEntry; depth: number }
  | { kind: "note"; key: string; depth: number; text: string; error?: boolean }
  | { kind: "new"; depth: number };

/** 正在新增的檔案／資料夾（在 dir 底下第一列出現輸入框）。 */
interface Creating {
  dir: string;
  kind: "file" | "dir";
}

/** 樹裡的路徑接上新名字（引擎兩種分隔符都收；顯示用 /）。 */
const joinRel = (dir: string, name: string) =>
  (dir ? dir.replace(/\\/g, "/") + "/" : "") + name.trim().replace(/^\/+/, "");

/** 改名的輸入框（取代那一列的檔名）：預設選起主檔名；Enter 改、Esc 取消；失敗留著說原因。 */
function RenameInput(props: {
  name: string;
  onSubmit: (name: string) => Promise<void>;
  onCancel: () => void;
}) {
  const { name, onSubmit, onCancel } = props;
  const [value, setValue] = useState(name);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  // 按了 Esc：之後的失焦（有些 WebView 拿掉輸入框時會補一個 blur）不能再送出
  const cancelled = useRef(false);
  const submit = async () => {
    if (cancelled.current) return;
    const v = value.trim();
    if (busy) return;
    if (!v || v === name) return onCancel();
    setBusy(true);
    setError("");
    try {
      await onSubmit(v);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <input
        autoFocus
        className="tnewinput"
        aria-label={"新名稱：" + name}
        value={value}
        disabled={busy}
        spellCheck={false}
        onFocus={(e) => {
          const dot = name.lastIndexOf(".");
          e.currentTarget.setSelectionRange(0, dot > 0 ? dot : name.length);
        }}
        onClick={(e) => e.stopPropagation()}
        onChange={(e) => {
          setValue(e.target.value);
          setError("");
        }}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter" && !e.nativeEvent.isComposing) {
            e.preventDefault();
            void submit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            cancelled.current = true;
            onCancel();
          }
        }}
        onBlur={() => {
          if (!busy && !error) void submit();
        }}
      />
      {error ? (
        <span className="tnewerror" role="alert">
          {error}
        </span>
      ) : null}
    </>
  );
}

/** 新增檔案／資料夾的輸入列：Enter 建立、Esc 取消；空白失焦也取消。失敗留在輸入框顯示原因。 */
function NewEntryRow(props: {
  creating: Creating;
  pad: { paddingLeft: number };
  guides: ReactNode;
  onSubmit: (name: string) => Promise<void>;
  onCancel: () => void;
}) {
  const { creating, pad, guides, onSubmit, onCancel } = props;
  const [name, setName] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const submit = async () => {
    if (busy) return;
    if (!name.trim()) return onCancel();
    setBusy(true);
    setError("");
    try {
      await onSubmit(name);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="trow tnew" style={pad}>
      {guides}
      <span className="tchevspace" />
      {creating.kind === "dir" ? (
        <span className="ticon" aria-hidden="true">
          ▸
        </span>
      ) : (
        <FileIcon name={name || "x.md"} />
      )}
      <input
        autoFocus
        className="tnewinput"
        aria-label={creating.kind === "dir" ? "新資料夾名稱" : "新檔名"}
        placeholder={creating.kind === "dir" ? "資料夾名稱" : "檔名，例如 notes.md"}
        value={name}
        disabled={busy}
        spellCheck={false}
        onChange={(e) => {
          setName(e.target.value);
          setError("");
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.nativeEvent.isComposing) {
            e.preventDefault();
            void submit();
          } else if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
        onBlur={() => {
          if (!name.trim() && !busy) onCancel();
        }}
      />
      {error ? (
        <span className="tnewerror" role="alert">
          {error}
        </span>
      ) : null}
    </div>
  );
}

/**
 * 一個 repo 當根的檔案樹（VS Code 檔案總管風格）：展開資料夾才向引擎要下一層（action `repo_tree`）；
 * 磁碟上有變動（新增／刪除／改名）會自己更新（Tauri 殼監看展開中的資料夾）。
 * 右鍵：複製路徑／相對路徑；鍵盤：↑↓ 移動、→ 展開、← 收合或回上層、Enter 開啟、⌥⌘C／⇧⌥⌘C 複製。
 * 展開狀態記在這台電腦。讀失敗保留上次的清單並顯示錯誤。
 */
export function RepoRoot(props: {
  id: string;
  defaultOpen?: boolean;
  actions?: ReactNode;
  /** 只畫樹＋精簡標題列（右側欄用：上面已經有 repo 名）。 */
  bare?: boolean;
  /** 根目錄那一列按右鍵。 */
  onRootMenu?: (e: MouseEvent) => void;
  /** 看得到才監看磁碟變動（分頁藏起來時給 false）。預設 true。 */
  watching?: boolean;
}) {
  const { id, actions, bare = false, onRootMenu, watching = true } = props;
  const store = useStore();
  const ui = useUi();
  const view = useUiState();
  const copy = useCopy();
  const prefKey = "tree:" + (bare ? "panel:" : "files:") + id;
  const [root, setRoot] = useState("");
  const [dirs, setDirs] = useState<Record<string, DirState>>({});
  const [open, setOpenState] = useState<Set<string>>(
    () =>
      new Set(loadPref<string[] | null>(prefKey, null) ?? (bare || props.defaultOpen ? [""] : [])),
  );
  const [focusPath, setFocusPath] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; e: TreeEntry } | null>(null);
  const [creating, setCreating] = useState<Creating | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const treeRef = useRef<HTMLDivElement>(null);

  const setOpen = (next: Set<string>) => {
    setOpenState(next);
    savePref(prefKey, [...next]);
  };

  // agent 改檔很快：同一層可能同時有好幾次重讀在路上，晚送出的才算數（先回來的舊清單不能蓋掉新的）
  const seq = useRef(new Map<string, number>());
  const load = async (dir: string) => {
    const n = (seq.current.get(dir) ?? 0) + 1;
    seq.current.set(dir, n);
    const latest = () => seq.current.get(dir) === n;
    setDirs((d) => ({ ...d, [dir]: { ...d[dir], loading: true, error: "" } }));
    try {
      const r = await store.action("repo_tree", { id, dir });
      if (!latest()) return;
      setRoot(r.root);
      setDirs((d) => ({ ...d, [dir]: { entries: r.entries, more: r.more } }));
    } catch (e) {
      if (!latest()) return;
      setDirs((d) => ({ ...d, [dir]: { ...d[dir], loading: false, error: (e as Error).message } }));
    }
  };

  // 掛上時把（記住的）展開中資料夾讀一次
  useEffect(() => {
    for (const dir of open) void load(dir);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  // 磁碟變動：只重讀受影響、而且讀過的資料夾（不閃「讀取中」，舊清單留著直到新的回來）
  useWatchDirs(store.backend, id, watching ? [...open] : []);
  useRepoChanged(store.backend, id, (paths) => {
    const hit = new Set<string>();
    for (const p of paths) {
      const parent = parentDir(p);
      if (open.has(parent) && dirs[parent]) hit.add(parent);
      if (open.has(p) && dirs[p]) hit.add(p);
    }
    for (const dir of hit) void load(dir);
  });

  const toggle = (dir: string, want?: boolean) => {
    const isOpen = open.has(dir);
    if (want === isOpen) return;
    const next = new Set(open);
    if (isOpen) next.delete(dir);
    else {
      next.add(dir);
      if (!dirs[dir]?.entries && !dirs[dir]?.loading) void load(dir);
    }
    setOpen(next);
  };

  /** 開始在 dir 底下新增（先展開它）。 */
  const startCreate = (dir: string, kind: "file" | "dir") => {
    const next = new Set(open);
    next.add("");
    next.add(dir);
    if (next.size !== open.size) setOpen(next);
    if (!dirs[dir]?.entries && !dirs[dir]?.loading) void load(dir);
    setCreating({ dir, kind });
  };
  /** 工具列的新增：加在目前選的資料夾（選的是檔就加在它旁邊），沒選就加在根目錄。 */
  const createTarget = () => {
    const e = entries.find((x) => x.path === focusPath);
    return e ? (e.dir ? e.path : parentDir(e.path)) : "";
  };
  const create = async (c: Creating, name: string) => {
    const r = await store.action("create_entry", { id, path: joinRel(c.dir, name), kind: c.kind });
    setCreating(null);
    await load(c.dir);
    setFocusPath(r.path);
    if (r.dir) store.toast("已新增資料夾 " + r.path);
    else ui.openFile(id, r.path, { edit: true });
  };

  /** 改名：有開著沒存的分頁就先擋（不然草稿會跟舊路徑一起丟掉）。 */
  const rename = async (e: TreeEntry, name: string) => {
    if (ui.dirtyUnder(id, e.path)) throw new Error("有分頁還沒存，先存檔再改名");
    const parent = parentDir(e.path);
    const r = await store.action("rename_entry", { id, from: e.path, to: joinRel(parent, name) });
    setRenaming(null);
    // 展開狀態跟著換到新路徑
    const moved = [...open].map((p) =>
      p === e.path || p.startsWith(e.path + "/") ? r.path + p.slice(e.path.length) : p,
    );
    setOpen(new Set(moved));
    await load(parent);
    ui.renamePath(id, e.path, r.path);
    setFocusPath(r.path);
    store.toast("已改名為 " + r.path);
  };

  const reload = () => {
    for (const dir of open) void load(dir);
  };
  const collapseAll = () => setOpen(new Set(open.has("") ? [""] : []));

  // 攤平成畫面上的列
  const rows: Row[] = [];
  const walk = (dir: string, depth: number) => {
    const st = dirs[dir];
    if (st?.error)
      rows.push({ kind: "note", key: dir + ":err", depth, text: st.error, error: true });
    if (!st?.entries && st?.loading)
      rows.push({ kind: "note", key: dir + ":ld", depth, text: "讀取中…" });
    if (creating?.dir === dir) rows.push({ kind: "new", depth });
    if (st?.entries && !st.entries.length && creating?.dir !== dir)
      rows.push({ kind: "note", key: dir + ":empty", depth, text: "（空的）" });
    for (const e of st?.entries || []) {
      rows.push({ kind: "entry", e, depth });
      if (e.dir && open.has(e.path)) walk(e.path, depth + 1);
    }
    if (st?.more)
      rows.push({ kind: "note", key: dir + ":more", depth, text: "另 " + st.more + " 項沒列出" });
  };
  const rootOpen = open.has("");
  if (rootOpen) walk("", 0);
  const entries = rows.flatMap((r) => (r.kind === "entry" ? [r.e] : []));
  const tabStop = entries.some((e) => e.path === focusPath)
    ? focusPath
    : (entries[0]?.path ?? null);

  const activate = (e: TreeEntry) => {
    setFocusPath(e.path);
    if (e.dir) toggle(e.path);
    else ui.openFile(id, e.path);
  };

  const focusRow = (path: string) => {
    setFocusPath(path);
    const el = treeRef.current?.querySelector<HTMLElement>(`[data-path="${CSS.escape(path)}"]`);
    el?.focus();
  };

  const onKey = (ev: KeyboardEvent, e: TreeEntry) => {
    const i = entries.findIndex((x) => x.path === e.path);
    const go = (j: number) => {
      const t = entries[Math.max(0, Math.min(entries.length - 1, j))];
      if (t) focusRow(t.path);
    };
    if (ev.code === "KeyC" && ev.altKey && (ev.metaKey || ev.ctrlKey)) {
      ev.preventDefault();
      if (ev.shiftKey) void copy(e.path, "相對路徑");
      else void copy(e.abs, "路徑");
      return;
    }
    switch (ev.key) {
      case "F2":
        setRenaming(e.path);
        break;
      case "ArrowDown":
        go(i + 1);
        break;
      case "ArrowUp":
        go(i - 1);
        break;
      case "Home":
        go(0);
        break;
      case "End":
        go(entries.length - 1);
        break;
      case "ArrowRight":
        if (!e.dir) return;
        if (!open.has(e.path)) toggle(e.path, true);
        else if (entries[i + 1] && parentDir(entries[i + 1].path) === e.path) go(i + 1);
        break;
      case "ArrowLeft":
        if (e.dir && open.has(e.path)) toggle(e.path, false);
        else if (parentDir(e.path)) focusRow(parentDir(e.path));
        break;
      case "Enter":
      case " ":
        activate(e);
        break;
      default:
        return;
    }
    ev.preventDefault();
  };

  const pickOf = (e: TreeEntry): Pick => ({ repo: id, path: e.path, name: e.name });
  const isPicked = (e: TreeEntry) => view.picks.some((p) => samePick(p, pickOf(e)));
  const menuItems = (e: TreeEntry): MenuItem[] => [
    e.dir
      ? { label: open.has(e.path) ? "收合" : "展開", run: () => toggle(e.path) }
      : { label: "開啟", run: () => ui.openFile(id, e.path) },
    {
      label: isPicked(e) ? "移出脈絡" : "加入脈絡（交給 Agent）",
      hint: "⌘＋點",
      run: () => ui.togglePick(pickOf(e)),
    },
    {
      label: "新增檔案…",
      run: () => startCreate(e.dir ? e.path : parentDir(e.path), "file"),
    },
    { label: "重新命名…", hint: "F2", run: () => setRenaming(e.path) },
    {
      label: "新增資料夾…",
      run: () => startCreate(e.dir ? e.path : parentDir(e.path), "dir"),
    },
    { label: "複製路徑", hint: "⌥⌘C", run: () => copy(e.abs, "路徑") },
    { label: "複製相對路徑", hint: "⇧⌥⌘C", run: () => copy(e.path, "相對路徑") },
  ];

  const active = view.activeFile?.repo === id ? view.activeFile.path : null;
  const tree = (
    <div
      ref={treeRef}
      role="tree"
      aria-label={"檔案：" + id}
      className={"tree" + (bare ? " bare" : "")}
    >
      {rows.map((r) => {
        const guides = Array.from({ length: r.depth }, (_, k) => (
          <span key={k} className="tguide" style={{ left: BASE + k * INDENT + 7 }} />
        ));
        const pad = { paddingLeft: BASE + r.depth * INDENT };
        if (r.kind === "new")
          return creating ? (
            <NewEntryRow
              key={"new:" + creating.dir + ":" + creating.kind}
              creating={creating}
              pad={pad}
              guides={guides}
              onSubmit={(name) => create(creating, name)}
              onCancel={() => setCreating(null)}
            />
          ) : null;
        if (r.kind === "note")
          return (
            <div
              key={r.key}
              className={"trow tnote" + (r.error ? " error" : "")}
              style={pad}
              role={r.error ? "alert" : undefined}
            >
              {guides}
              <span className="tchevspace" />
              {r.text}
            </div>
          );
        const e = r.e;
        const isOpen = e.dir && open.has(e.path);
        const picked = isPicked(e);
        return (
          <div
            key={e.path}
            role="treeitem"
            aria-level={r.depth + 1}
            aria-expanded={e.dir ? isOpen : undefined}
            aria-selected={!e.dir && e.path === active}
            aria-label={picked ? e.name + "（已加入脈絡）" : e.name}
            data-path={e.path}
            tabIndex={e.path === tabStop ? 0 : -1}
            className={
              "trow" +
              (e.dir ? " dir" : " file") +
              (!e.dir && e.path === active ? " selected" : "") +
              (picked ? " picked" : "")
            }
            style={pad}
            title={e.path}
            onClick={(ev) => {
              // ⌘／Ctrl＋點＝挑起來（多選要交給 agent 的檔案），不開檔
              if (ev.metaKey || ev.ctrlKey) {
                setFocusPath(e.path);
                ui.togglePick(pickOf(e));
              } else activate(e);
            }}
            draggable
            onDragStart={(ev) => {
              // 拖到下方終端＝貼路徑（TerminalPanel 收）
              ev.dataTransfer.setData(
                FILE_DRAG_TYPE,
                JSON.stringify({ repo: id, path: e.path, abs: e.abs }),
              );
              ev.dataTransfer.setData("text/plain", e.abs);
              ev.dataTransfer.effectAllowed = "copy";
            }}
            onFocus={() => setFocusPath(e.path)}
            onKeyDown={(ev) => onKey(ev, e)}
            onContextMenu={(ev) => {
              ev.preventDefault();
              setFocusPath(e.path);
              setMenu({ x: ev.clientX, y: ev.clientY, e });
            }}
          >
            {guides}
            {e.dir ? <Chevron open={!!isOpen} /> : <span className="tchevspace" />}
            {e.dir ? null : <FileIcon name={e.name} />}
            {renaming === e.path ? (
              <RenameInput
                name={e.name}
                onSubmit={(name) => rename(e, name)}
                onCancel={() => setRenaming(null)}
              />
            ) : (
              <span className="tlabel">{e.name}</span>
            )}
            {picked ? (
              <span className="tpicked" aria-hidden="true">
                ✓
              </span>
            ) : null}
          </div>
        );
      })}
      {menu ? (
        <ContextMenu
          title={menu.e.path}
          items={menuItems(menu.e)}
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
        />
      ) : null}
    </div>
  );

  const busy = Object.values(dirs).some((d) => d.loading);
  const tools = (
    <span className="ttools">
      <button
        type="button"
        className="icon ghost"
        title="新增檔案（加在選取的資料夾）"
        aria-label={"新增檔案到 " + id}
        onClick={() => startCreate(createTarget(), "file")}
      >
        ＋
      </button>
      <button
        type="button"
        className="icon ghost"
        title="新增資料夾（加在選取的資料夾）"
        aria-label={"新增資料夾到 " + id}
        onClick={() => startCreate(createTarget(), "dir")}
      >
        ⊞
      </button>
      <button
        type="button"
        className="icon ghost"
        title="重新整理"
        aria-label={"重新整理 " + id}
        disabled={!open.size || busy}
        onClick={reload}
      >
        ⟳
      </button>
      <button
        type="button"
        className="icon ghost"
        title="全部收合"
        aria-label={"全部收合 " + id}
        disabled={open.size <= (rootOpen ? 1 : 0)}
        onClick={collapseAll}
      >
        ⊟
      </button>
      {actions}
    </span>
  );

  if (bare)
    return (
      <section className="explorer bare" aria-label={"檔案：" + id}>
        <div className="explorerhead">
          <span className="eyebrow">檔案</span>
          {tools}
        </div>
        {tree}
      </section>
    );
  return (
    <section className="reporoot explorer" aria-label={"檔案：" + id}>
      <div className="explorerhead rootrow" onContextMenu={onRootMenu}>
        <button
          type="button"
          className="ghost troot"
          aria-expanded={rootOpen}
          title={root || id}
          onClick={() => toggle("")}
        >
          <Chevron open={rootOpen} />
          <span className="trootname">{id}</span>
        </button>
        {root ? (
          <button
            type="button"
            className="icon ghost"
            title={"複製路徑：" + root}
            aria-label={"複製 " + id + " 的路徑"}
            onClick={() => void copy(root, "路徑")}
          >
            ⧉
          </button>
        ) : null}
        {tools}
      </div>
      {rootOpen ? tree : null}
    </section>
  );
}
