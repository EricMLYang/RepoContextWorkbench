/**
 * 終端輸出裡看起來像檔案路徑的字（F-95）：`src/a.rs:12:5`、`./x.md`、`/abs/path.ts(3,1)`、`README.md:4`。
 * 這裡只找「長得像」的位置給 xterm 畫底線；點下去才交給引擎（`locate_path`）判斷是不是登記的 repo 裡真的有的檔。
 */
// 分隔符號認 `/` 與 `\`（Windows／VDI：`src\lib\a.ts:12`、`C:\repo\a.rs`）
const CANDIDATE =
  /(?:[A-Za-z]:[\\/]|~[\\/]|\.{1,2}[\\/]|[\\/])?(?:[\w.@+-]+[\\/])+[\w.@+-]+(?::\d+(?::\d+)?|\(\d+(?:,\s?\d+)?\))?|[\w@+-][\w.@+-]*\.[A-Za-z][A-Za-z0-9]{0,7}(?::\d+(?::\d+)?|\(\d+(?:,\s?\d+)?\))/g;

export interface PathHit {
  /** 字串裡的起點（含）與終點（不含）。 */
  start: number;
  end: number;
  text: string;
}

/** 一行字裡的路徑候選。網址（`https://…`）、沒有副檔名也不是 `/`／`./`／`~/` 開頭的（`and/or`）不算。 */
export function findPathHits(line: string): PathHit[] {
  const out: PathHit[] = [];
  for (const m of line.matchAll(CANDIDATE)) {
    const start = m.index ?? 0;
    let text = m[0];
    // 句尾的「.」不是路徑的一部分
    text = text.replace(/\.+$/, "");
    if (!text) continue;
    const before = line.slice(Math.max(0, start - 3), start);
    if (/:\/?\/?$/.test(before) && /[a-z]$/i.test(line.slice(0, start).replace(/:\/?\/?$/, "")))
      continue; // 網址的後半
    if (text.startsWith("//")) continue;
    const bare = text.replace(/(?::\d+(?::\d+)?|\(\d+(?:,\s?\d+)?\))$/, "");
    const last = bare.split(/[\\/]/).pop() ?? "";
    const rooted = /^(?:[\\/]|\.{1,2}[\\/]|~[\\/]|[A-Za-z]:[\\/])/.test(bare);
    const hasExt = /\.[A-Za-z][A-Za-z0-9]{0,7}$/.test(last);
    if (!rooted && !hasExt) continue;
    if (bare === "/" || /^\.+$/.test(bare)) continue;
    out.push({ start, end: start + text.length, text });
  }
  return out;
}

/** xterm 一個格子（只用到的部分）。 */
interface CellLike {
  getChars(): string;
  getWidth(): number;
}

/**
 * xterm 一行的字＋「字串第 i 個字元在第幾格」：全形字佔兩格，第二格是空的（寬 0），直接 translateToString 會對不上格子。
 */
export function lineCells(line: { length: number; getCell(x: number): CellLike | undefined }): {
  text: string;
  cellOf: number[];
} {
  let text = "";
  const cellOf: number[] = [];
  for (let x = 0; x < line.length; x++) {
    const c = line.getCell(x);
    if (!c || c.getWidth() === 0) continue;
    const ch = c.getChars() || " ";
    for (let i = 0; i < ch.length; i++) cellOf.push(x);
    text += ch;
  }
  return { text, cellOf };
}

export interface TermHit {
  /** 第幾行（buffer 的 0 起算）。 */
  y: number;
  /** 字串裡的起點（含）與終點（不含），配 [lineCells] 換成格子。 */
  start: number;
  end: number;
}

/** 終端內搜尋（F-111）：每行找出所有出現的位置（不分大小寫），由上到下。 */
export function findInLines(lines: string[], query: string): TermHit[] {
  const q = query.toLowerCase();
  if (!q) return [];
  const out: TermHit[] = [];
  lines.forEach((line, y) => {
    const l = line.toLowerCase();
    let i = l.indexOf(q);
    while (i >= 0) {
      out.push({ y, start: i, end: i + q.length });
      i = l.indexOf(q, i + q.length);
    }
  });
  return out;
}
