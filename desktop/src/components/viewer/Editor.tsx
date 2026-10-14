import { useEffect, useLayoutEffect, useRef, type ReactNode } from "react";
import { continueList, indent, insert, toggleLinePrefix, wrap, type Edit } from "../../lib/mdedit";
import { FILE_DRAG_TYPE } from "../../lib/view";

/** 編輯時的版面：只有編輯區／左右分割（左寫右看）／只看預覽。 */
export type EditLayout = "edit" | "split" | "preview";

/** 點到的是第幾個待辦方塊（照畫面順序）；不是就 -1。會擋掉瀏覽器自己的勾選（內容改了才換）。 */
export function taskIndex(e: React.MouseEvent<HTMLElement>): number {
  const box = (e.target as Element).closest?.("li.task > input[type=checkbox]");
  if (!box) return -1;
  e.preventDefault();
  return [...e.currentTarget.querySelectorAll("li.task > input[type=checkbox]")].indexOf(box);
}

/** md 工具列：按鈕 → 對選取做什麼。 */
const TOOLS: { label: string; title: string; run: (t: string, a: number, b: number) => Edit }[] = [
  { label: "B", title: "粗體（⌘B）", run: (t, a, b) => wrap(t, a, b, "**") },
  { label: "I", title: "斜體（⌘I）", run: (t, a, b) => wrap(t, a, b, "*") },
  { label: "S", title: "刪除線", run: (t, a, b) => wrap(t, a, b, "~~") },
  { label: "H", title: "標題", run: (t, a, b) => toggleLinePrefix(t, a, b, "## ") },
  { label: "•", title: "項目清單", run: (t, a, b) => toggleLinePrefix(t, a, b, "- ") },
  { label: "1.", title: "編號清單", run: (t, a, b) => toggleLinePrefix(t, a, b, "1. ") },
  { label: "☐", title: "待辦清單", run: (t, a, b) => toggleLinePrefix(t, a, b, "- [ ] ") },
  { label: "❝", title: "引言", run: (t, a, b) => toggleLinePrefix(t, a, b, "> ") },
  { label: "</>", title: "行內程式碼（⌘E）", run: (t, a, b) => wrap(t, a, b, "`", "`", "code") },
  {
    label: "{ }",
    title: "程式碼區塊",
    run: (t, a, b) => wrap(t, a, b, "\n```\n", "\n```\n", "程式碼"),
  },
  {
    label: "🔗",
    title: "連結",
    run: (t, a, b) => {
      const sel = t.slice(a, b) || "連結文字";
      const s = "[" + sel + "](網址)";
      // 選起「網址」讓人直接打
      return {
        text: t.slice(0, a) + s + t.slice(b),
        start: a + sel.length + 3,
        end: a + sel.length + 5,
      };
    },
  },
  {
    label: "▦",
    title: "表格",
    run: (t, a, b) => insert(t, a, b, "\n| 欄位 | 欄位 |\n| --- | --- |\n|  |  |\n"),
  },
];

/**
 * HackMD 式的編輯區：md 有工具列、左右分割即時預覽（捲動同步）、清單按 Enter 自動延續、Tab 縮排；
 * 其他文字檔只有編輯區。⌘S 存檔交給呼叫端。
 * 套用編輯走 `execCommand("insertText")`，系統的復原（⌘Z）才記得住；不支援就直接換文字。
 */
export function Editor(props: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  onSave: () => void;
  markdown: boolean;
  layout: EditLayout;
  readOnly?: boolean;
  /** 預覽畫什麼（md 用；拿目前的草稿）。 */
  preview?: (text: string) => ReactNode;
  /** 預覽裡點了第 i 個待辦方塊。 */
  onToggleTask?: (i: number) => void;
  /** 從檔案樹拖檔進來：拖曳資料 → 要插入的字（null＝不收）。 */
  onDropFile?: (raw: string) => string | null;
}) {
  const {
    label,
    value,
    onChange,
    onSave,
    markdown,
    layout,
    readOnly,
    preview,
    onToggleTask,
    onDropFile,
  } = props;
  const ref = useRef<HTMLTextAreaElement>(null);
  const previewRef = useRef<HTMLDivElement>(null);
  const pendingSel = useRef<[number, number] | null>(null);

  // 直接換文字時（不支援 execCommand），等 React 換上新值再把游標放回去
  useLayoutEffect(() => {
    const sel = pendingSel.current;
    const el = ref.current;
    if (!sel || !el) return;
    pendingSel.current = null;
    el.setSelectionRange(sel[0], sel[1]);
  });

  useEffect(() => {
    if (layout !== "preview") ref.current?.focus();
  }, [layout]);

  const apply = (e: Edit) => {
    const el = ref.current;
    if (!el) return;
    const old = el.value;
    // 只換有變的那一段（前後相同的不動），復原紀錄才乾淨
    let p = 0;
    while (p < old.length && p < e.text.length && old[p] === e.text[p]) p++;
    let q = 0;
    while (
      q < old.length - p &&
      q < e.text.length - p &&
      old[old.length - 1 - q] === e.text[e.text.length - 1 - q]
    )
      q++;
    el.focus();
    el.setSelectionRange(p, old.length - q);
    let done: boolean;
    try {
      done = !!document.execCommand?.("insertText", false, e.text.slice(p, e.text.length - q));
    } catch {
      done = false;
    }
    if (done && el.value === e.text) {
      el.setSelectionRange(e.start, e.end);
      onChange(el.value);
    } else {
      pendingSel.current = [e.start, e.end];
      onChange(e.text);
    }
  };

  const run = (f: (t: string, a: number, b: number) => Edit | null) => {
    const el = ref.current;
    if (!el) return false;
    const r = f(el.value, el.selectionStart, el.selectionEnd);
    if (!r) return false;
    apply(r);
    return true;
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const mod = e.metaKey || e.ctrlKey;
    const k = e.key.toLowerCase();
    if (mod && k === "s") {
      e.preventDefault();
      onSave();
      return;
    }
    if (!markdown || e.nativeEvent.isComposing) return;
    let handled = false;
    if (mod && !e.shiftKey && k === "b") handled = run((t, a, b) => wrap(t, a, b, "**"));
    else if (mod && !e.shiftKey && k === "i") handled = run((t, a, b) => wrap(t, a, b, "*"));
    else if (mod && !e.shiftKey && k === "e")
      handled = run((t, a, b) => wrap(t, a, b, "`", "`", "code"));
    else if (e.key === "Tab" && !mod && !e.altKey)
      handled = run((t, a, b) => indent(t, a, b, e.shiftKey));
    else if (e.key === "Enter" && !mod && !e.shiftKey && !e.altKey)
      handled = run((t, a, b) => (a === b ? continueList(t, a) : null));
    if (handled) e.preventDefault();
  };

  // 分割時左右捲動同步（按比例）
  const syncScroll = () => {
    const a = ref.current;
    const b = previewRef.current;
    if (!a || !b || layout !== "split") return;
    const ratio = a.scrollTop / Math.max(1, a.scrollHeight - a.clientHeight);
    b.scrollTop = ratio * (b.scrollHeight - b.clientHeight);
  };

  return (
    <div className={"editor layout-" + (markdown ? layout : "edit")}>
      {markdown && layout !== "preview" ? (
        <div className="mdtools" role="toolbar" aria-label="Markdown 格式">
          {TOOLS.map((t) => (
            <button
              key={t.title}
              type="button"
              className="ghost small"
              title={t.title}
              aria-label={t.title}
              disabled={readOnly}
              // 不搶焦點：選取留在編輯區
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => run(t.run)}
            >
              {t.label}
            </button>
          ))}
        </div>
      ) : null}
      <div className="editorpanes">
        <textarea
          ref={ref}
          className="vieweredit"
          aria-label={label}
          hidden={markdown && layout === "preview"}
          spellCheck={false}
          value={value}
          readOnly={readOnly}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={onKeyDown}
          onScroll={syncScroll}
          onDragOver={(e) => {
            if (onDropFile && e.dataTransfer.types.includes(FILE_DRAG_TYPE)) {
              e.preventDefault();
              e.dataTransfer.dropEffect = "copy";
            }
          }}
          onDrop={(e) => {
            if (!onDropFile || readOnly || !e.dataTransfer.types.includes(FILE_DRAG_TYPE)) return;
            e.preventDefault();
            const text = onDropFile(e.dataTransfer.getData(FILE_DRAG_TYPE));
            if (text) run((t, a, b) => insert(t, a, b, text));
          }}
        />
        {markdown && layout !== "edit" && preview ? (
          <div
            ref={previewRef}
            className="editorpreview"
            aria-label="即時預覽"
            onClick={(e) => {
              const i = taskIndex(e);
              if (i >= 0 && onToggleTask && !readOnly) onToggleTask(i);
            }}
          >
            {preview(value)}
          </div>
        ) : null}
      </div>
    </div>
  );
}
