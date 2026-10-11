import type { ReactNode } from "react";

/**
 * 唯讀檢視器用的 Markdown → React（只產生 React 元素，不用 innerHTML——repo 裡的 md 不能在 App 裡跑腳本）。
 * 支援：標題、段落、清單（巢狀、待辦勾選）、引言、程式碼區塊、表格、分隔線、front matter；
 * 行內：`code`、**粗**、*斜*、~~刪除~~、[連結](…)、![圖](…)、<網址>（底線不當強調，免得 snake_case 變斜體）。
 * 原始 HTML 一律當文字顯示。
 */
export interface MdHooks {
  /** 點連結（相對路徑或網址），由檢視器決定開檔或複製。 */
  onLink(href: string): void;
  /** 圖片（相對路徑由檢視器去讀；網址不載入）。 */
  image(src: string, alt: string, key: string): ReactNode;
}

const FENCE = /^\s{0,3}(```+|~~~+)\s*([^`\s]*)?.*$/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const HR = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/;
const QUOTE = /^\s{0,3}>\s?(.*)$/;
const ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/;
const TABLE_SEP = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

function cells(line: string): string[] {
  let t = line.trim();
  if (t.startsWith("|")) t = t.slice(1);
  if (t.endsWith("|") && !t.endsWith("\\|")) t = t.slice(0, -1);
  return t.split(/(?<!\\)\|/).map((c) => c.trim().replace(/\\\|/g, "|"));
}

function indentOf(line: string) {
  return line.length - line.trimStart().length;
}

const INLINE =
  /(`+)([\s\S]*?[^`])\1(?!`)|!\[([^\]]*)\]\(\s*<?([^)\s>]*)>?(?:\s+"[^"]*")?\s*\)|\[([^\]]+)\]\(\s*<?([^)\s>]*)>?(?:\s+"[^"]*")?\s*\)|<(https?:\/\/[^\s>]+)>|(\*\*)(?=\S)([\s\S]*?\S)\*\*|~~(?=\S)([\s\S]*?\S)~~|(\*)(?=\S)([\s\S]*?\S)\*/;

/** 行內語法。 */
export function inline(text: string, hooks: MdHooks, key = "i"): ReactNode[] {
  const out: ReactNode[] = [];
  let rest = text;
  let n = 0;
  while (rest) {
    const m = INLINE.exec(rest);
    if (!m) {
      out.push(rest);
      break;
    }
    if (m.index > 0) out.push(rest.slice(0, m.index));
    const k = key + "-" + n++;
    if (m[1]) out.push(<code key={k}>{m[2].trim()}</code>);
    else if (m[4] !== undefined && m[0].startsWith("!")) out.push(hooks.image(m[4], m[3], k));
    else if (m[5] !== undefined) {
      const href = m[6];
      out.push(
        <a
          key={k}
          href="#"
          title={href}
          onClick={(e) => {
            e.preventDefault();
            hooks.onLink(href);
          }}
        >
          {inline(m[5], hooks, k)}
        </a>,
      );
    } else if (m[7]) {
      const href = m[7];
      out.push(
        <a
          key={k}
          href="#"
          title={href}
          onClick={(e) => {
            e.preventDefault();
            hooks.onLink(href);
          }}
        >
          {href}
        </a>,
      );
    } else if (m[8]) out.push(<strong key={k}>{inline(m[9], hooks, k)}</strong>);
    else if (m[10] !== undefined) out.push(<del key={k}>{inline(m[10], hooks, k)}</del>);
    else if (m[11]) out.push(<em key={k}>{inline(m[12], hooks, k)}</em>);
    rest = rest.slice(m.index + m[0].length);
  }
  return out;
}

/** 區塊語法（遞迴：引言、清單項目裡面再 parse 一次）。 */
function blocks(lines: string[], hooks: MdHooks, key: string): ReactNode[] {
  const out: ReactNode[] = [];
  let i = 0;
  let n = 0;
  const k = () => key + "." + n++;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    const fence = FENCE.exec(line);
    if (fence) {
      const mark = fence[1];
      const body: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith(mark)) body.push(lines[i++]);
      i++;
      out.push(
        <pre key={k()} className="mdcode" data-lang={fence[2] || undefined}>
          <code>{body.join("\n")}</code>
        </pre>,
      );
      continue;
    }
    const h = HEADING.exec(line);
    if (h) {
      const level = h[1].length;
      const content = inline(h[2], hooks, key + "h" + n);
      const Tag = ("h" + level) as "h1" | "h2" | "h3" | "h4" | "h5" | "h6";
      out.push(<Tag key={k()}>{content}</Tag>);
      i++;
      continue;
    }
    if (HR.test(line)) {
      out.push(<hr key={k()} />);
      i++;
      continue;
    }
    if (QUOTE.test(line)) {
      const inner: string[] = [];
      while (i < lines.length && lines[i].trim() && QUOTE.test(lines[i]))
        inner.push(QUOTE.exec(lines[i++])![1]);
      out.push(<blockquote key={k()}>{blocks(inner, hooks, key + "q" + n)}</blockquote>);
      continue;
    }
    const item = ITEM.exec(line);
    if (item) {
      const base = indentOf(line);
      const ordered = /\d/.test(item[2]);
      const items: string[][] = [];
      while (i < lines.length) {
        const l = lines[i];
        const m = ITEM.exec(l);
        if (m && indentOf(l) === base && /\d/.test(m[2]) === ordered) {
          items.push([m[3]]);
          i++;
        } else if (l.trim() && indentOf(l) > base && items.length) {
          items[items.length - 1].push(l.slice(Math.min(indentOf(l), base + 2)));
          i++;
        } else if (
          !l.trim() &&
          i + 1 < lines.length &&
          indentOf(lines[i + 1]) > base &&
          items.length
        ) {
          items[items.length - 1].push("");
          i++;
        } else break;
      }
      const lis = items.map((body, j) => {
        const task = /^\[( |x|X)\]\s+/.exec(body[0]);
        const first = task ? body[0].slice(task[0].length) : body[0];
        const restLines = body.slice(1);
        const simple = !restLines.some((x) => x.trim());
        return (
          <li key={j} className={task ? "task" : undefined}>
            {task ? <input type="checkbox" checked={task[1] !== " "} readOnly disabled /> : null}
            {simple
              ? inline(first, hooks, key + "l" + n + "-" + j)
              : blocks([first, ...restLines], hooks, key + "l" + n + "-" + j)}
          </li>
        );
      });
      out.push(ordered ? <ol key={k()}>{lis}</ol> : <ul key={k()}>{lis}</ul>);
      continue;
    }
    if (line.includes("|") && i + 1 < lines.length && TABLE_SEP.test(lines[i + 1])) {
      const head = cells(line);
      const rows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].trim() && lines[i].includes("|"))
        rows.push(cells(lines[i++]));
      const tk = key + "t" + n;
      out.push(
        <div key={k()} className="mdtable">
          <table>
            <thead>
              <tr>
                {head.map((c, j) => (
                  <th key={j}>{inline(c, hooks, tk + "h" + j)}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r, ri) => (
                <tr key={ri}>
                  {head.map((_, j) => (
                    <td key={j}>{inline(r[j] ?? "", hooks, tk + "r" + ri + "c" + j)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>,
      );
      continue;
    }
    // 段落：到空行或下一個區塊為止
    const para: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() &&
      !(
        para.length &&
        (FENCE.test(lines[i]) ||
          HEADING.test(lines[i]) ||
          QUOTE.test(lines[i]) ||
          HR.test(lines[i]) ||
          ITEM.test(lines[i]))
      )
    )
      para.push(lines[i++].trim());
    const pk = key + "p" + n;
    const parts: ReactNode[] = [];
    para.forEach((p, j) => {
      if (j) parts.push(<br key={pk + "br" + j} />);
      parts.push(...inline(p, hooks, pk + "-" + j));
    });
    out.push(<p key={k()}>{parts}</p>);
  }
  return out;
}

/** 整份 Markdown。開頭的 YAML front matter 用淡色程式碼區塊顯示。 */
export function Markdown({ text, hooks }: { text: string; hooks: MdHooks }) {
  let lines = text.replace(/\r\n?/g, "\n").split("\n");
  let front: string[] | null = null;
  if (lines[0]?.trim() === "---") {
    const end = lines.findIndex((l, j) => j > 0 && (l.trim() === "---" || l.trim() === "..."));
    if (end > 0) {
      front = lines.slice(1, end);
      lines = lines.slice(end + 1);
    }
  }
  return (
    <div className="markdown">
      {front ? (
        <pre className="mdfront">
          <code>{front.join("\n")}</code>
        </pre>
      ) : null}
      {blocks(lines, hooks, "m")}
    </div>
  );
}
