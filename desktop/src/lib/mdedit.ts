/**
 * Markdown 編輯器的純文字操作（HackMD 式的快捷鍵與清單延續）。只算「改完的文字＋游標」，不碰 DOM。
 * 每個函式回傳 null＝這個按鍵不處理（交給瀏覽器預設行為）。
 */

export interface Edit {
  text: string;
  /** 改完後的選取範圍。 */
  start: number;
  end: number;
}

const lineStart = (t: string, i: number) => t.lastIndexOf("\n", i - 1) + 1;
const lineEnd = (t: string, i: number) => {
  const j = t.indexOf("\n", i);
  return j < 0 ? t.length : j;
};

/** 用 before／after 包住選取；已經包著就拿掉（再按一次＝取消）。沒選字就插入 placeholder 並選起來。 */
export function wrap(
  t: string,
  start: number,
  end: number,
  before: string,
  after = before,
  placeholder = "文字",
): Edit {
  const sel = t.slice(start, end);
  if (
    t.slice(start - before.length, start) === before &&
    t.slice(end, end + after.length) === after
  ) {
    return {
      text: t.slice(0, start - before.length) + sel + t.slice(end + after.length),
      start: start - before.length,
      end: end - before.length,
    };
  }
  const inner = sel || placeholder;
  return {
    text: t.slice(0, start) + before + inner + after + t.slice(end),
    start: start + before.length,
    end: start + before.length + inner.length,
  };
}

/** 選到的每一行加上 prefix；每行都已經有就全部拿掉。 */
export function toggleLinePrefix(t: string, start: number, end: number, prefix: string): Edit {
  const a = lineStart(t, start);
  const b = lineEnd(t, end > start && t[end - 1] === "\n" ? end - 1 : end);
  const lines = t.slice(a, b).split("\n");
  const all = lines.every((l) => l.startsWith(prefix));
  const next = lines.map((l) => (all ? l.slice(prefix.length) : prefix + l));
  const block = next.join("\n");
  const delta = block.length - (b - a);
  const firstDelta = all ? -prefix.length : prefix.length;
  return {
    text: t.slice(0, a) + block + t.slice(b),
    start: Math.max(a, start + firstDelta),
    end: end + delta,
  };
}

const LIST = /^(\s*)([-*+]|(\d+)[.)])\s+(\[[ xX]\]\s+)?/;

/**
 * Enter 在清單項目裡：下一行自動帶同樣的開頭（數字遞增、待辦變成沒勾）。
 * 空的項目再按 Enter＝結束清單（把開頭拿掉）。不在清單裡回 null。
 */
export function continueList(t: string, pos: number): Edit | null {
  const a = lineStart(t, pos);
  const line = t.slice(a, lineEnd(t, pos));
  const m = LIST.exec(line);
  if (!m || pos - a < m[0].length) return null;
  if (line.trim() === m[0].trim()) {
    // 空項目：清掉開頭，留一行空白
    return { text: t.slice(0, a) + t.slice(a + line.length), start: a, end: a };
  }
  const [, indent, bullet, num, box] = m;
  const mark = num ? String(Number(num) + 1) + bullet.slice(-1) : bullet;
  const insert = "\n" + indent + mark + " " + (box ? "[ ] " : "");
  return {
    text: t.slice(0, pos) + insert + t.slice(pos),
    start: pos + insert.length,
    end: pos + insert.length,
  };
}

/** Tab／Shift+Tab：選到的行縮排或取消縮排兩格；沒選字又不在清單裡就插入兩個空白。 */
export function indent(t: string, start: number, end: number, outdent: boolean): Edit {
  const a = lineStart(t, start);
  const onList = LIST.test(t.slice(a, lineEnd(t, start)));
  if (start === end && !outdent && !onList) {
    return { text: t.slice(0, start) + "  " + t.slice(end), start: start + 2, end: start + 2 };
  }
  const b = lineEnd(t, end);
  const lines = t.slice(a, b).split("\n");
  let first = 0;
  const next = lines.map((l, i) => {
    if (!outdent) {
      if (i === 0) first = 2;
      return "  " + l;
    }
    const cut = l.startsWith("  ") ? 2 : l.startsWith(" ") || l.startsWith("\t") ? 1 : 0;
    if (i === 0) first = -cut;
    return l.slice(cut);
  });
  const block = next.join("\n");
  return {
    text: t.slice(0, a) + block + t.slice(b),
    start: Math.max(a, start + first),
    end: Math.max(a, end + block.length - (b - a)),
  };
}

/** 插入一段文字（取代選取），游標放在後面。 */
export function insert(t: string, start: number, end: number, s: string): Edit {
  return {
    text: t.slice(0, start) + s + t.slice(end),
    start: start + s.length,
    end: start + s.length,
  };
}

const FENCE_LINE = /^\s{0,3}(```+|~~~+)/;
const TASK_LINE = /^(\s*(?:>\s*)*(?:[-*+]|\d{1,9}[.)])\s+\[)( |x|X)(\]\s+)/;

/**
 * 預覽裡點了第 `index` 個待辦方塊（照文件順序，跟預覽畫出來的順序一樣）→ 把原文那一行的 [ ]／[x] 換過來。
 * 程式碼區塊與 front matter 裡的不算。找不到回 null。
 */
export function toggleTask(text: string, index: number): string | null {
  const lines = text.split("\n");
  let fence: string | null = null;
  let start = 0;
  if (lines[0]?.replace(/\r$/, "").trim() === "---") {
    const end = lines.findIndex(
      (l, j) => j > 0 && ["---", "..."].includes(l.replace(/\r$/, "").trim()),
    );
    if (end > 0) start = end + 1;
  }
  let n = 0;
  for (let i = start; i < lines.length; i++) {
    const f = FENCE_LINE.exec(lines[i]);
    if (f) {
      if (!fence) fence = f[1][0];
      else if (f[1][0] === fence) fence = null;
      continue;
    }
    if (fence) continue;
    const m = TASK_LINE.exec(lines[i]);
    if (!m) continue;
    if (n++ === index) {
      lines[i] = lines[i].replace(TASK_LINE, (_, a, mark, b) => a + (mark === " " ? "x" : " ") + b);
      return lines.join("\n");
    }
  }
  return null;
}

/** 標題 → 錨點（GitHub 的規則：小寫、拿掉標點、空白換 -；中文照留）。 */
export function slug(text: string): string {
  return text
    .trim()
    .toLowerCase()
    .replace(/[^\p{L}\p{N}\s_-]/gu, "")
    .replace(/\s/g, "-");
}

/** repo 裡從 `from` 這個檔看 `to` 的相對路徑（都用 `/`；同資料夾就是檔名）。 */
export function relPath(from: string, to: string): string {
  const a = from.replace(/\\/g, "/").split("/").slice(0, -1);
  const b = to.replace(/\\/g, "/").split("/");
  let i = 0;
  while (i < a.length && i < b.length - 1 && a[i] === b[i]) i++;
  return [...a.slice(i).map(() => ".."), ...b.slice(i)].join("/");
}

const IMAGE = /\.(png|jpe?g|gif|webp|svg|bmp|avif)$/i;

/**
 * 從檔案樹拖一個檔進 md 編輯區要插入的字：同一個 repo＝相對連結（圖片用 `![]()`）；
 * 別的 repo 連不到，就寫 `repo/路徑` 當行內碼，至少看得出指的是哪個。
 */
export function linkForDrop(
  fromRepo: string,
  fromPath: string,
  f: { repo: string; path: string },
): string {
  const name = f.path.replace(/\\/g, "/").split("/").pop() || f.path;
  if (f.repo !== fromRepo) return "`" + f.repo + "/" + f.path.replace(/\\/g, "/") + "`";
  const rel = relPath(fromPath, f.path).replace(/ /g, "%20");
  return (IMAGE.test(name) ? "!" : "") + "[" + name + "](" + rel + ")";
}
