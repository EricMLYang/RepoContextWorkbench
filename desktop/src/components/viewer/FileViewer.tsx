import { useEffect, useRef, useState } from "react";
import type { FileContent } from "../../api/types";
import { Markdown, type MdHooks } from "../../lib/markdown";
import { parentDir, useRepoChanged, useWatchDirs } from "../../lib/treeWatch";
import { useStore, useUi } from "../../store/context";
import { load as loadPref, save as savePref } from "../../lib/storage";
import { ContextMenu, type MenuItem } from "../ContextMenu";
import { Editor, type EditLayout } from "./Editor";
import { clearMatches, findRanges, showMatches } from "../../lib/findText";

export interface OpenFile {
  repo: string;
  path: string;
  /** 開在「差異」模式（從變更清單點的）。 */
  diff?: boolean;
  /** 一打開就進編輯（剛新增的檔）。 */
  edit?: boolean;
}

export const fileKey = (f: OpenFile) => f.repo + "\u0000" + f.path;

/** 一次最多畫幾行（再多按「顯示全部」）——上萬行的檔一次塞進 DOM 會卡。 */
const LINE_CAP = 4000;

function fmtSize(n: number) {
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
  return (n / 1024 / 1024).toFixed(1) + " MB";
}

const isMarkdown = (p: string) => /\.(md|markdown|mdx)$/i.test(p);
/** 預設自動換行的（文章類）；程式碼預設不換行、可橫向捲。 */
const isProse = (p: string) => isMarkdown(p) || /\.(txt|rst|adoc|org|log)$/i.test(p);

/**
 * md 裡的相對連結 → repo 內路徑（`/` 開頭＝repo 根目錄）。跳出 repo 回 null。
 * 只做字串正規化；路徑合不合法、檔在不在由引擎 `read_file` 判斷。
 */
export function resolveRel(from: string, href: string): string | null {
  const clean = decodeURIComponent(href.split("#")[0].split("?")[0]);
  if (!clean) return null;
  const baseParts = from.replace(/\\/g, "/").split("/").slice(0, -1);
  const parts = clean.startsWith("/") ? [] : baseParts;
  for (const seg of clean.replace(/^\/+/, "").split("/")) {
    if (!seg || seg === ".") continue;
    if (seg === "..") {
      if (!parts.length) return null;
      parts.pop();
    } else parts.push(seg);
  }
  return parts.join("/");
}

async function copy(store: ReturnType<typeof useStore>, text: string, label: string) {
  try {
    await navigator.clipboard.writeText(text);
    store.toast("已複製" + label + "：" + text);
  } catch {
    store.toast("無法複製到剪貼簿");
  }
}

/** md 裡的相對圖片：向引擎讀（網址圖片不載入）。 */
function MdImage({ repo, path, alt }: { repo: string; path: string; alt: string }) {
  const store = useStore();
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    store
      .action("read_file", { id: repo, path })
      .then((r) => {
        if (!live) return;
        if (r.kind === "image") setSrc("data:" + r.mime + ";base64," + r.base64);
        else setFailed(true);
      })
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, [store, repo, path]);
  if (src) return <img className="mdimg" src={src} alt={alt} />;
  return (
    <span className="mdimgalt">
      {failed ? "［圖片讀不到：" + (alt || path) + "］" : "［圖片：" + (alt || path) + "］"}
    </span>
  );
}

/** 文字檔：行號＋等寬字；可切換自動換行。 */
function CodeView({ text, wrap }: { text: string; wrap: boolean }) {
  const [all, setAll] = useState(false);
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  const shown = all ? lines : lines.slice(0, LINE_CAP);
  const width = String(lines.length).length;
  return (
    <>
      <div
        className={"codeview" + (wrap ? " wrap" : "")}
        style={{ ["--gutter" as string]: width + 1 + "ch" }}
      >
        {shown.map((l, i) => (
          <div className="cl" key={i}>
            <span className="ln" aria-hidden="true">
              {i + 1}
            </span>
            <span className="lc">{l || " "}</span>
          </div>
        ))}
      </div>
      {!all && lines.length > LINE_CAP ? (
        <button type="button" className="small showall" onClick={() => setAll(true)}>
          顯示全部 {lines.length} 行
        </button>
      ) : null}
    </>
  );
}

/**
 * 檔案裡找字（⌘F）：Enter 下一筆、⇧Enter 上一筆、Esc 關。找的是畫面上看到的內容（預覽、程式碼、差異、
 * 分割編輯的右邊預覽）；內容一變（agent 改檔、打字）就重找。
 */
function FindBar(props: {
  root: () => Element | null;
  /** 內容變了就重找（長度固定的陣列，逐項當 effect 的相依）。 */
  watch: unknown[];
  onClose: () => void;
}) {
  const { root, watch, onClose } = props;
  const [q, setQ] = useState("");
  const [cur, setCur] = useState(0);
  const [count, setCount] = useState(0);
  const ranges = useRef<Range[]>([]);
  useEffect(() => {
    const el = root();
    ranges.current = el && q ? findRanges(el, q) : [];
    setCount(ranges.current.length);
    const i = Math.min(cur, Math.max(0, ranges.current.length - 1));
    showMatches(ranges.current, i);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [q, cur, ...watch]);
  useEffect(() => clearMatches, []);
  const step = (d: number) => {
    if (!count) return;
    setCur((c) => (c + d + count) % count);
  };
  return (
    <div className="findbar" role="search">
      <input
        autoFocus
        aria-label="在檔案中尋找"
        placeholder="在檔案中尋找"
        value={q}
        spellCheck={false}
        onChange={(e) => {
          setQ(e.target.value);
          setCur(0);
        }}
        onKeyDown={(e) => {
          if (e.nativeEvent.isComposing) return;
          if (e.key === "Enter") {
            e.preventDefault();
            step(e.shiftKey ? -1 : 1);
          } else if (e.key === "Escape") {
            e.preventDefault();
            onClose();
          }
        }}
      />
      <span className="small muted findcount" aria-live="polite">
        {q ? (count ? Math.min(cur, count - 1) + 1 + " / " + count : "找不到") : ""}
      </span>
      <button
        type="button"
        className="small ghost"
        aria-label="上一筆"
        disabled={!count}
        onClick={() => step(-1)}
      >
        ↑
      </button>
      <button
        type="button"
        className="small ghost"
        aria-label="下一筆"
        disabled={!count}
        onClick={() => step(1)}
      >
        ↓
      </button>
      <button type="button" className="small ghost" aria-label="關閉尋找" onClick={onClose}>
        ×
      </button>
    </div>
  );
}

/** textarea 只會給 LF；比對「有沒有改」也用 LF（存檔時引擎會還原原檔的 CRLF／BOM）。 */
const lf = (t: string) => t.replace(/\r\n/g, "\n").replace(/^\uFEFF/, "");

/**
 * 檢視器（一個分頁一個檔）：md 預設預覽、可切原始碼；程式碼有行號；圖片直接顯示。
 * 文字檔可以簡單編輯（textarea，⌘S 存）。agent 改檔很快，所以：
 * - 沒改東西時，磁碟一變就重讀（編輯中也跟著換成新內容）；
 * - 有未存的修改時不重讀（不洗掉使用者打的字），改成提示「磁碟上的檔變了」，讓人選載入或覆蓋；
 * - 存檔帶讀檔時的 version，引擎發現被改過就不寫（conflict）。
 */
/** 差異的一行屬於哪種（上色用）。 */
function diffLineKind(l: string): string {
  if (l.startsWith("+++") || l.startsWith("---")) return "meta";
  if (l.startsWith("+")) return "add";
  if (l.startsWith("-")) return "del";
  if (l.startsWith("@@")) return "hunk";
  if (/^(diff |index |new file|deleted file|similarity|rename |old mode|new mode)/.test(l))
    return "meta";
  return "ctx";
}

/** 差異（git diff 的文字）：加的綠、刪的紅、段落標頭藍；太長先畫前 LINE_CAP 行。 */
function DiffView({ text, truncated }: { text: string; truncated: boolean }) {
  const [all, setAll] = useState(false);
  if (!text) return <p className="viewernote">這個檔沒有還沒 commit 的變更。</p>;
  const lines = text.split("\n");
  const shown = all ? lines : lines.slice(0, LINE_CAP);
  return (
    <>
      {truncated ? <p className="viewernote">差異太長，只顯示前 512 KB。</p> : null}
      <pre className="diffview" aria-label="差異">
        {shown.map((l, i) => (
          <div key={i} className={"dl " + diffLineKind(l)}>
            {l || " "}
          </div>
        ))}
      </pre>
      {lines.length > shown.length ? (
        <button type="button" className="small" onClick={() => setAll(true)}>
          顯示全部 {lines.length} 行
        </button>
      ) : null}
    </>
  );
}

export function FileViewer({
  file,
  hidden,
  onDirty,
  picked = false,
}: {
  file: OpenFile;
  hidden: boolean;
  onDirty?: (dirty: boolean) => void;
  /** 這個檔已經挑起來了（App 給；不訂閱整個 UiState，免得打字時每個分頁都重畫）。 */
  picked?: boolean;
}) {
  const store = useStore();
  const ui = useUi();
  const { repo, path } = file;
  const [content, setContent] = useState<FileContent | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [nonce, setNonce] = useState(0);
  // 檢視模式：view＝md 預覽／程式碼；source＝md 原始碼；diff＝跟 HEAD 的差異
  const [mode, setMode] = useState<"view" | "source" | "diff">(file.diff ? "diff" : "view");
  const showDiff = mode === "diff";
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const moreRef = useRef<HTMLButtonElement>(null);
  // md 大綱：從畫好的預覽讀標題（不另外解析），點了捲過去
  const [outline, setOutline] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const outlineRef = useRef<HTMLButtonElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const [finding, setFinding] = useState(false);
  // ⌘F（Ctrl+F）：這個分頁看得到時打開尋找列；焦點在終端裡不搶
  useEffect(() => {
    if (hidden) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "f" || e.shiftKey || e.altKey)
        return;
      if ((e.target as Element | null)?.closest?.("#termpanel, .xterm")) return;
      e.preventDefault();
      setFinding(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [hidden]);
  const openOutline = () => {
    const hs = [
      ...(bodyRef.current?.querySelectorAll<HTMLElement>(
        ".markdown > h1, .markdown > h2, .markdown > h3, .markdown > h4",
      ) ?? []),
    ];
    const r = outlineRef.current?.getBoundingClientRect();
    setOutline({
      x: r ? r.left : 0,
      y: r ? r.bottom + 4 : 0,
      items: hs.length
        ? hs.map((h, i) => ({
            key: String(i),
            label: h.textContent || "",
            indent: Number(h.tagName[1]) - 1,
            run: () => h.scrollIntoView({ block: "start", behavior: "smooth" }),
          }))
        : [{ label: "（這份文件沒有標題）", run: () => {} }],
    });
  };
  const [wrap, setWrap] = useState(isProse(path));
  // 編輯：draft＝null 表示沒在編輯；editBase＝這份草稿是從哪個內容開始改的（LF）
  const [draft, setDraft] = useState<string | null>(null);
  const [editBase, setEditBase] = useState("");
  const [saving, setSaving] = useState(false);
  // HackMD 式：md 預設左寫右看；自動儲存（停手 1.5 秒存一次，衝突就停下來讓人選）
  const [layout, setLayoutState] = useState<EditLayout>(() =>
    loadPref<EditLayout>("editLayout", "split"),
  );
  const setLayout = (l: EditLayout) => {
    savePref("editLayout", l);
    setLayoutState(l);
  };
  const [autosave, setAutosaveState] = useState(() => loadPref<boolean>("autosave", true));
  const setAutosave = (on: boolean) => {
    savePref("autosave", on);
    setAutosaveState(on);
  };
  const [diskChanged, setDiskChanged] = useState(false);
  const [armDiscard, setArmDiscard] = useState(false);
  // 差異模式：看這個檔跟上一個 commit 差在哪（agent 改了什麼）
  const [diff, setDiff] = useState<{ text: string; truncated: boolean } | null>(null);
  const [diffError, setDiffError] = useState("");
  // 從變更清單再點一次（已開著的分頁）：切到差異
  const [askedDiff, setAskedDiff] = useState(file.diff);
  if (askedDiff !== file.diff) {
    setAskedDiff(file.diff);
    if (file.diff) setMode("diff");
  }
  const dirty = draft !== null && draft !== editBase;
  // 給非同步回呼（讀檔回來、磁碟事件）看最新值
  const dirtyRef = useRef(dirty);
  const contentRef = useRef(content);
  const onDirtyRef = useRef(onDirty);
  useEffect(() => {
    dirtyRef.current = dirty;
    contentRef.current = content;
    onDirtyRef.current = onDirty;
  });
  useEffect(() => {
    onDirtyRef.current?.(dirty);
  }, [dirty]);
  useEffect(() => () => onDirtyRef.current?.(false), []);

  /** 換上新讀到的內容；編輯中但沒改東西就讓草稿跟著換。 */
  const accept = (r: FileContent) => {
    setContent(r);
    if (r.kind === "text" && !dirtyRef.current) {
      setEditBase(lf(r.text));
      setDraft((d) => (d === null ? null : lf(r.text)));
    }
  };

  // 讀檔（換檔或按 ⟳ 重讀）；讀失敗保留上次的內容並顯示錯誤
  useEffect(() => {
    let live = true;
    store
      .action("read_file", { id: repo, path })
      .then((r) => {
        if (!live) return;
        accept(r);
        setError("");
      })
      .catch((e: Error) => live && setError(e.message))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [store, repo, path, nonce]);
  const reload = () => {
    setLoading(true);
    setError("");
    setNonce((n) => n + 1);
  };
  // 差異：切到差異模式、或檔案重讀（磁碟變了）時重算；失敗保留上次的並顯示錯誤
  useEffect(() => {
    if (!showDiff) return;
    let live = true;
    store
      .action("file_diff", { id: repo, path })
      .then((r) => {
        if (!live) return;
        setDiff({ text: r.diff ?? "", truncated: !!r.truncated });
        setDiffError("");
      })
      .catch((e: Error) => live && setDiffError(e.message));
    return () => {
      live = false;
    };
  }, [store, repo, path, nonce, showDiff]);
  // 檔案在磁碟上被改了（agent 在寫、git 切分支…）就自己重讀；有未存的修改時只提示
  useWatchDirs(store.backend, repo, [parentDir(path)]);
  useRepoChanged(store.backend, repo, (paths, rescan) => {
    if (!rescan && !paths.includes(path)) return;
    if (!dirtyRef.current) {
      reload();
      return;
    }
    // 可能只是自己剛存檔的回音：版本一樣就不吵
    store
      .action("read_file", { id: repo, path })
      .then((r) => {
        const cur = contentRef.current;
        const v = (c: FileContent | null) => (c?.kind === "text" ? c.version : undefined);
        if (v(r) !== v(cur)) setDiskChanged(true);
      })
      .catch(() => setDiskChanged(true));
  });

  const startEdit = () => {
    if (content?.kind !== "text") return;
    setEditBase(lf(content.text));
    setDraft(lf(content.text));
    setArmDiscard(false);
  };
  const stopEdit = () => {
    if (dirty && !armDiscard) {
      setArmDiscard(true);
      return;
    }
    setDraft(null);
    setArmDiscard(false);
    setDiskChanged(false);
  };
  /** 存檔；`overwrite`＝磁碟上已經變了也照樣用我的版本蓋過去。 */
  const save = async (overwrite = false, quiet = false) => {
    if (draft === null || saving || content?.kind !== "text") return;
    const sent = draft;
    setSaving(true);
    setError("");
    try {
      let base = content.version ?? "";
      if (overwrite) {
        const cur = await store.action("read_file", { id: repo, path });
        if (cur.kind !== "text" || !cur.editable)
          throw new Error("磁碟上的檔已經不能編輯了（變成二進位或太大）");
        base = cur.version ?? "";
      }
      const r = await store.action("write_file", { id: repo, path, text: sent, base });
      if (!r.ok) {
        setDiskChanged(true);
        return;
      }
      setContent(r);
      setEditBase(sent);
      setDiskChanged(false);
      setArmDiscard(false);
      if (!quiet) store.toast("已儲存 " + path);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  };
  // 剛新增的檔：讀到內容就直接進編輯（只做一次）
  const autoEdit = useRef(!!file.edit);
  useEffect(() => {
    if (autoEdit.current && content?.kind === "text" && content.editable && draft === null) {
      autoEdit.current = false;
      startEdit();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [content]);
  // 自動儲存：有改、沒在存、磁碟沒被別人改過，停手 1.5 秒就存
  const saveRef = useRef(save);
  useEffect(() => {
    saveRef.current = save;
  });
  useEffect(() => {
    if (!autosave || !dirty || saving || diskChanged) return;
    const t = setTimeout(() => void saveRef.current(false, true), 1500);
    return () => clearTimeout(t);
  }, [autosave, dirty, saving, diskChanged, draft]);

  const loadDisk = () => {
    setDraft(null);
    setDiskChanged(false);
    setArmDiscard(false);
    reload();
  };

  const hooks: MdHooks = {
    onLink(href) {
      if (href.startsWith("#")) return;
      if (/^[a-z][a-z0-9+.-]*:/i.test(href)) {
        void copy(store, href, "連結（App 內不開外部網頁）");
        return;
      }
      const target = resolveRel(path, href);
      if (target) ui.openFile(repo, target);
      else store.toast("連結指到 repo 外面：" + href);
    },
    image(src, alt, key) {
      if (/^[a-z][a-z0-9+.-]*:/i.test(src))
        return (
          <span key={key} className="mdimgalt">
            ［外部圖片：{alt || src}］
          </span>
        );
      const target = resolveRel(path, src);
      return target ? <MdImage key={key} repo={repo} path={target} alt={alt} /> : null;
    },
  };

  const md = isMarkdown(path);
  const editing = draft !== null;
  const crumbs = path.replace(/\\/g, "/").split("/");
  // 低頻動作收進「⋯」：換行（看程式碼／原始碼時）、複製路徑、重新讀取（有未存修改時不給）
  const moreItems: MenuItem[] = [];
  if (!editing && content?.kind === "text" && (!md || mode === "source"))
    moreItems.push({ label: wrap ? "關閉自動換行" : "自動換行", run: () => setWrap(!wrap) });
  moreItems.push({ label: "複製相對路徑", run: () => copy(store, path, "相對路徑") });
  if (content)
    moreItems.push({ label: "複製絕對路徑", run: () => copy(store, content.abs, "絕對路徑") });
  if (!dirty && !loading) moreItems.push({ label: "重新讀取", run: reload });
  return (
    <section className="viewer" hidden={hidden} aria-label={"檢視：" + repo + "/" + path}>
      <div className="viewerhead">
        <div className="crumbs" title={content?.abs || path}>
          <span className="muted">{repo}</span>
          {crumbs.map((c, i) => (
            <span key={i} className={i === crumbs.length - 1 ? "crumb last" : "crumb"}>
              <span className="muted" aria-hidden="true">
                {" "}
                ›{" "}
              </span>
              {c}
            </span>
          ))}
          {content ? <span className="muted small"> · {fmtSize(content.size)}</span> : null}
          {dirty ? <span className="dirtymark"> · 未儲存</span> : null}
        </div>
        <div className="vieweractions">
          {!editing ? (
            // 刪掉的檔（讀不到）也要能看差異，所以沒有內容時一樣給
            <div className="segmented" role="group" aria-label="檢視模式">
              <button
                type="button"
                className="small ghost"
                aria-pressed={mode === "view"}
                onClick={() => setMode("view")}
              >
                {md && content?.kind === "text" ? "預覽" : "內容"}
              </button>
              {md && content?.kind === "text" ? (
                <button
                  type="button"
                  className="small ghost"
                  aria-pressed={mode === "source"}
                  onClick={() => setMode("source")}
                >
                  原始碼
                </button>
              ) : null}
              <button
                type="button"
                className="small ghost"
                aria-pressed={mode === "diff"}
                title="跟上一個 commit（HEAD）比，看 agent 改了什麼"
                onClick={() => setMode(mode === "diff" ? "view" : "diff")}
              >
                差異
              </button>
            </div>
          ) : null}
          {editing && md ? (
            <div className="segmented" role="group" aria-label="編輯版面">
              {(
                [
                  ["edit", "編輯"],
                  ["split", "分割"],
                  ["preview", "預覽"],
                ] as [EditLayout, string][]
              ).map(([k, l]) => (
                <button
                  key={k}
                  type="button"
                  className="small ghost"
                  aria-pressed={layout === k}
                  onClick={() => setLayout(k)}
                >
                  {l}
                </button>
              ))}
            </div>
          ) : null}
          {editing ? (
            <>
              <label
                className="small autosave"
                title="停手 1.5 秒自動存；磁碟上的檔被別人改過就停下來讓你選"
              >
                <input
                  type="checkbox"
                  checked={autosave}
                  onChange={(e) => setAutosave(e.target.checked)}
                />
                自動儲存
              </label>
              <button
                type="button"
                className="small primary"
                disabled={!dirty || saving}
                title="⌘S"
                onClick={() => void save(false)}
              >
                {saving ? "儲存中…" : "儲存"}
              </button>
              <button
                type="button"
                className={"small ghost" + (armDiscard ? " danger" : "")}
                disabled={saving}
                onClick={stopEdit}
              >
                {!dirty ? "完成" : armDiscard ? "再按一次放棄修改" : "放棄修改"}
              </button>
            </>
          ) : content?.kind === "text" ? (
            <button
              type="button"
              className="small"
              disabled={!content.editable}
              title={
                content.editable
                  ? "直接改這個檔（⌘S 存）"
                  : "檔案超過 1 MB 或不是 UTF-8，不能在 App 裡編輯"
              }
              onClick={startEdit}
            >
              編輯
            </button>
          ) : null}
          <button
            type="button"
            className="small ghost"
            aria-pressed={picked}
            title="放進主區下緣的挑選列，之後跟其他檔一起交給 Agent（不會自動送出）"
            onClick={() => ui.togglePick({ repo, path, name: crumbs[crumbs.length - 1] })}
          >
            {picked ? "已加入脈絡 ✓" : "加入脈絡"}
          </button>
          {md && !editing && mode === "view" && content?.kind === "text" ? (
            <button
              ref={outlineRef}
              type="button"
              className="small ghost"
              aria-haspopup="menu"
              aria-expanded={!!outline}
              title="跳到某個標題"
              onClick={() => (outline ? setOutline(null) : openOutline())}
            >
              大綱
            </button>
          ) : null}
          {outline ? (
            <ContextMenu
              id="outlinemenu"
              title="大綱"
              items={outline.items}
              x={outline.x}
              y={outline.y}
              onClose={() => setOutline(null)}
            />
          ) : null}
          <button
            ref={moreRef}
            type="button"
            className="small ghost"
            aria-label="更多檔案動作"
            aria-haspopup="menu"
            aria-expanded={!!menu}
            onClick={() => {
              const r = moreRef.current?.getBoundingClientRect();
              setMenu(menu ? null : { x: r ? r.right - 180 : 0, y: r ? r.bottom + 4 : 0 });
            }}
          >
            ⋯
          </button>
          {menu ? (
            <ContextMenu
              id="filemenu"
              title={crumbs[crumbs.length - 1]}
              items={moreItems}
              x={menu.x}
              y={menu.y}
              onClose={() => setMenu(null)}
            />
          ) : null}
        </div>
      </div>
      {finding && !hidden ? (
        <FindBar
          root={() =>
            bodyRef.current?.querySelector(".editorpreview") ??
            (draft !== null ? null : bodyRef.current)
          }
          watch={[content, draft, mode, diff, layout]}
          onClose={() => setFinding(false)}
        />
      ) : null}
      {diskChanged ? (
        <div className="viewerconflict" role="alert">
          <span>磁碟上的檔在你編輯時被改了（可能是 agent）。</span>
          <button type="button" className="small" disabled={saving} onClick={loadDisk}>
            載入磁碟版本（放棄我的修改）
          </button>
          <button
            type="button"
            className="small danger"
            disabled={saving}
            onClick={() => void save(true)}
          >
            用我的版本覆蓋
          </button>
        </div>
      ) : null}
      {showDiff && !editing ? null : error ? (
        <p className="error viewererror" role="alert">
          {error}{" "}
          {!dirty ? (
            <button type="button" className="small" disabled={loading} onClick={reload}>
              重新讀取
            </button>
          ) : null}
        </p>
      ) : null}
      <div className="viewerbody" ref={bodyRef}>
        {showDiff && !editing ? (
          diffError ? (
            <p className="error viewererror" role="alert">
              {diffError}
            </p>
          ) : diff ? (
            <DiffView text={diff.text} truncated={diff.truncated} />
          ) : (
            <p className="muted">讀取差異中…</p>
          )
        ) : !content ? (
          loading ? (
            <p className="muted">讀取中…</p>
          ) : null
        ) : content.kind === "text" ? (
          editing ? (
            <Editor
              label={"編輯 " + path}
              value={draft}
              readOnly={saving}
              markdown={md}
              layout={layout}
              onChange={(v) => {
                setDraft(v);
                setArmDiscard(false);
              }}
              onSave={() => {
                if (dirty) void save(false);
              }}
              preview={(text) => <Markdown text={text} hooks={hooks} />}
            />
          ) : (
            <>
              {content.truncated ? (
                <p className="viewernote">檔案有 {fmtSize(content.size)}，只顯示前 1 MB。</p>
              ) : null}
              {md && mode === "view" ? (
                <div
                  className="mdpreview"
                  title={content.editable ? "雙擊進入編輯" : undefined}
                  onDoubleClick={() => {
                    if (content.editable && !window.getSelection()?.toString()) startEdit();
                  }}
                >
                  <Markdown text={content.text} hooks={hooks} />
                </div>
              ) : (
                <CodeView text={content.text} wrap={wrap} />
              )}
            </>
          )
        ) : content.kind === "image" ? (
          <div className="viewerimage">
            <img src={"data:" + content.mime + ";base64," + content.base64} alt={path} />
          </div>
        ) : (
          <p className="viewernote">
            {content.kind === "too_large"
              ? "圖片太大（" + fmtSize(content.size) + "），不在 App 裡顯示。"
              : "這是二進位檔（" + fmtSize(content.size) + "），沒有可以顯示的文字。"}
          </p>
        )}
      </div>
    </section>
  );
}
