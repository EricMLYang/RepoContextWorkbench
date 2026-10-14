import { useEffect, useMemo, useRef, useState } from "react";
import { langOf, tokenizeLine, type HlState, type Lang } from "../../lib/highlight";
import type { FileContent } from "../../api/types";
import { Markdown, type MdHooks } from "../../lib/markdown";
import { parentDir, useRepoChanged, useWatchDirs } from "../../lib/treeWatch";
import { useStore, useUi } from "../../store/context";
import { load as loadPref, save as savePref } from "../../lib/storage";
import { ContextMenu, type MenuItem } from "../ContextMenu";
import { Editor, taskIndex, type EditLayout } from "./Editor";
import { lineOffset, linkForDrop, mdHeadings, relPath, slug, toggleTask } from "../../lib/mdedit";
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

/** 文字檔：行號＋等寬字；可切換自動換行；認得的語言輕量上色。 */
function CodeView({ text, wrap, lang }: { text: string; wrap: boolean; lang: Lang | null }) {
  const [all, setAll] = useState(false);
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  const shown = all ? lines : lines.slice(0, LINE_CAP);
  const toks = useMemo(() => {
    if (!lang) return null;
    const st: HlState = { block: null };
    return shown.map((l) => tokenizeLine(l, lang, st));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, all, lang]);
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
            <span className="lc">
              {toks
                ? toks[i].length
                  ? toks[i].map((t, j) =>
                      t.cls ? (
                        <span key={j} className={"tk-" + t.cls}>
                          {t.text}
                        </span>
                      ) : (
                        t.text
                      ),
                    )
                  : " "
                : l || " "}
            </span>
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

/**
 * md 預覽下面的「連到這份的文件」（F-69，抄 Clearly 的 backlinks）：引擎照相對連結找，點了開那份。
 * 換檔或重讀才重找；讀不到就不顯示（不擋看文件）。
 */
function Backlinks({ repo, path, nonce }: { repo: string; path: string; nonce: number }) {
  const store = useStore();
  const ui = useUi();
  const [r, setR] = useState<{
    items: { path: string; line: number; text: string }[];
    more: number;
  } | null>(null);
  useEffect(() => {
    let live = true;
    store
      .action("backlinks", { id: repo, path })
      // 缺欄位（舊版殼、錯誤回應）就不顯示，不讓整個檢視器炸掉
      .then(
        (x) => live && setR(Array.isArray(x.items) ? { items: x.items, more: x.more ?? 0 } : null),
      )
      .catch(() => live && setR(null));
    return () => {
      live = false;
    };
  }, [store, repo, path, nonce]);
  if (!r) return null;
  return (
    <section className="backlinks" aria-label="連到這份的文件">
      <p className="eyebrow">
        連到這份的文件{r.items.length ? "（" + (r.items.length + r.more) + "）" : ""}
      </p>
      {r.items.length ? (
        <ul>
          {r.items.map((b) => (
            <li key={b.path + ":" + b.line}>
              <button
                type="button"
                className="ghost"
                title={b.path + " 第 " + b.line + " 行"}
                onClick={() => ui.openFile(repo, b.path)}
              >
                <span className="blpath">{b.path}</span>
                <span className="small muted">{b.text}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="small muted">這個 repo 裡還沒有其他 md 連到這份。</p>
      )}
      {r.more ? <p className="small muted">另有 {r.more} 則沒列出。</p> : null}
    </section>
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
  // 這個分頁的快捷鍵（每次重畫換成最新的，監聽只掛一次）：
  // ⌘F 尋找；HackMD 式 ⌘⌥E 編輯、⌘⌥B 分割、⌘⌥V 預覽（md）
  const keysRef = useRef<(e: KeyboardEvent) => void>(() => {});
  useEffect(() => {
    if (hidden) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.shiftKey) return;
      if ((e.target as Element | null)?.closest?.("#termpanel, .xterm")) return;
      keysRef.current(e);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [hidden]);
  const [jump, setJump] = useState<{ line: number; pos: number; seq: number } | undefined>();
  const openOutline = () => {
    // 編輯中：照草稿的標題，點了把游標跳到那一行
    if (draft !== null) {
      const hs = mdHeadings(draft);
      const r = outlineRef.current?.getBoundingClientRect();
      setOutline({
        x: r ? r.left : 0,
        y: r ? r.bottom + 4 : 0,
        items: hs.length
          ? hs.map((h, i) => ({
              key: String(i),
              label: h.text,
              indent: h.level - 1,
              run: () =>
                setJump((j) => ({
                  line: h.line,
                  pos: lineOffset(draft, h.line),
                  seq: (j?.seq ?? 0) + 1,
                })),
            }))
          : [{ label: "（這份文件沒有標題）", run: () => {} }],
      });
      return;
    }
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
  // 最近一次送去存的內容（LF）：磁碟事件可能比存檔的回應先到，內容一樣就是自己的回音
  const sentRef = useRef<string | null>(null);
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
        if (r.kind === "text" && sentRef.current !== null && lf(r.text) === sentRef.current) return;
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
    sentRef.current = sent;
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
  // HackMD 式偏好：打開 md 就直接進編輯（版面照上次選的，預設分割；預設關，⋯ 裡切換）
  const [mdOpenEdit, setMdOpenEditState] = useState(() => loadPref<boolean>("mdOpenEdit", false));
  const setMdOpenEdit = (on: boolean) => {
    savePref("mdOpenEdit", on);
    setMdOpenEditState(on);
  };
  const autoEdit = useRef(!!file.edit || (isMarkdown(path) && mdOpenEdit && !file.diff));
  // 偏好被別處改了（助手、別的分頁）：開著的分頁跟著換；null＝回到預設
  useEffect(() => {
    const on = (e: Event) => {
      const { key, value } = (e as CustomEvent<{ key: string; value: unknown }>).detail;
      if (key === "autosave") setAutosaveState(value === null ? true : !!value);
      else if (key === "mdOpenEdit") setMdOpenEditState(!!value);
      else if (key === "editLayout")
        setLayoutState(value === null ? "split" : (value as EditLayout));
    };
    window.addEventListener("workbench:pref", on);
    return () => window.removeEventListener("workbench:pref", on);
  }, []);
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

  // 預覽裡勾待辦（HackMD 式）：直接改檔存回去，帶讀檔時的 version；被別人改過就重讀，不蓋掉
  const toggleTaskOnDisk = async (i: number) => {
    if (content?.kind !== "text" || !content.editable || saving) return;
    const next = toggleTask(lf(content.text), i);
    if (next === null) return;
    setSaving(true);
    try {
      const r = await store.action("write_file", {
        id: repo,
        path,
        text: next,
        base: content.version ?? "",
      });
      if (!r.ok) {
        store.toast("檔案剛被改過，已重新讀取；再勾一次");
        reload();
        return;
      }
      setContent(r);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setSaving(false);
    }
  };

  const onShortcut = (e: KeyboardEvent) => {
    // ⌥ 會把字母換成特殊字元（macOS），用 code 判斷
    const k = e.altKey ? e.code.replace(/^Key/, "").toLowerCase() : e.key.toLowerCase();
    if (!e.altKey && k === "f") {
      e.preventDefault();
      setFinding(true);
      return;
    }
    if (!e.altKey || !md || content?.kind !== "text") return;
    // Windows／Linux 的 Ctrl+Alt＝AltGr（例：德文鍵盤 AltGr+E 打 €）：在輸入框裡不能搶，只認 macOS 的 ⌘⌥
    const typing = (e.target as Element | null)?.closest?.("textarea, input");
    if (typing && !e.metaKey) return;
    const want = ({ e: "edit", b: "split", v: "preview" } as Record<string, EditLayout>)[k];
    if (!want) return;
    e.preventDefault();
    if (want === "preview" && draft === null) {
      setMode("view");
      return;
    }
    if (draft === null) {
      if (!content.editable) return;
      startEdit();
    }
    setLayout(want);
  };

  useEffect(() => {
    keysRef.current = onShortcut;
  });

  const loadDisk = () => {
    setDraft(null);
    setDiskChanged(false);
    setArmDiscard(false);
    reload();
  };

  const hooks: MdHooks = {
    // 可編輯的檔，預覽裡的待辦方塊可以直接點
    tasks: content?.kind === "text" && !!content.editable,
    onLink(href) {
      if (href.startsWith("#")) {
        // 文件內的錨點：捲到同名的標題
        const want = decodeURIComponent(href.slice(1)).toLowerCase();
        const h = [
          ...(bodyRef.current?.querySelectorAll<HTMLElement>(
            ".markdown h1, .markdown h2, .markdown h3, .markdown h4, .markdown h5, .markdown h6",
          ) ?? []),
        ].find((x) => slug(x.textContent || "") === want);
        if (h) h.scrollIntoView({ block: "start", behavior: "smooth" });
        else store.toast("找不到這個標題：" + href);
        return;
      }
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
  if (md)
    moreItems.push({
      label: mdOpenEdit ? "打開 md 時直接分割編輯：開（按了關掉）" : "打開 md 時直接分割編輯",
      run: () => {
        setMdOpenEdit(!mdOpenEdit);
        store.toast(mdOpenEdit ? "之後打開 md 先看預覽" : "之後打開 md 直接左寫右看");
      },
    });
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
          {md && content?.kind === "text" && (editing || mode === "view") ? (
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
              jump={jump}
              onDropFile={
                md
                  ? (raw) => {
                      try {
                        const f = JSON.parse(raw) as { repo?: unknown; path?: unknown };
                        if (typeof f.repo !== "string" || typeof f.path !== "string") return null;
                        return linkForDrop(repo, path, { repo: f.repo, path: f.path });
                      } catch {
                        return null;
                      }
                    }
                  : undefined
              }
              onPasteImage={
                md
                  ? async (mime, b64) => {
                      // 存到這份 md 旁邊的 assets/，檔名＝md 主檔名＋時間
                      const ext = mime.split("/")[1]?.replace("jpeg", "jpg") || "png";
                      const dir = parentDir(path.replace(/\\/g, "/"));
                      const stem = (crumbs[crumbs.length - 1] || "image").replace(/\.[^.]+$/, "");
                      const stamp = new Date()
                        .toISOString()
                        .replace(/[-:]/g, "")
                        .replace("T", "-")
                        .slice(0, 15);
                      const target =
                        (dir ? dir + "/" : "") + "assets/" + stem + "-" + stamp + "." + ext;
                      try {
                        const r = await store.action("save_image", {
                          id: repo,
                          path: target,
                          base64: b64,
                        });
                        store.toast("圖片存到 " + r.path);
                        const name = r.path.replace(/\\/g, "/").split("/").pop() || "image";
                        return "![" + name + "](" + relPath(path, r.path) + ")";
                      } catch (e) {
                        store.toast("圖片存不了：" + (e as Error).message);
                        return null;
                      }
                    }
                  : undefined
              }
              onToggleTask={(i) => {
                const next = draft === null ? null : toggleTask(draft, i);
                if (next !== null) setDraft(next);
              }}
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
                  onClick={(e) => {
                    const i = taskIndex(e);
                    if (i >= 0) void toggleTaskOnDisk(i);
                  }}
                  onDoubleClick={(e) => {
                    if ((e.target as Element).closest?.("input")) return;
                    if (content.editable && !window.getSelection()?.toString()) startEdit();
                  }}
                >
                  <Markdown text={content.text} hooks={hooks} />
                  <Backlinks repo={repo} path={path} nonce={nonce} />
                </div>
              ) : (
                <CodeView text={content.text} wrap={wrap} lang={md ? null : langOf(path)} />
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
