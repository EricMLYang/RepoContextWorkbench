/**
 * 在一塊畫好的內容裡找字（檢視器的 ⌘F；系統 WebView 沒有內建的尋找列）。
 * 只做畫面：走文字節點算出 Range，用 CSS Custom Highlight API 標起來（不改 DOM，React 不會打架）；
 * 不支援 Highlight 的環境（舊 WebView、測試）只回數量、照樣能捲到那一筆。
 */

/** 換區塊（一行程式碼、一段、一格）時插入的分隔：搜尋不會跨過它。 */
const SEP = "\u0000";
const BLOCK = "div,p,li,h1,h2,h3,h4,h5,h6,td,th,pre,blockquote";

/**
 * 找出 root 底下所有符合 query 的位置（不分大小寫）。同一個區塊裡的文字節點串起來找
 * （上色把一個字詞切成好幾段也找得到），Range 可以跨節點；不同區塊之間不會連起來。
 */
export function findRanges(root: Element, query: string): Range[] {
  const q = query.toLocaleLowerCase();
  if (!q) return [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) =>
      // 行號（aria-hidden）不算
      n.parentElement?.closest("[aria-hidden='true']")
        ? NodeFilter.FILTER_REJECT
        : NodeFilter.FILTER_ACCEPT,
  });
  // 串起來的文字，以及每個節點在裡面從哪裡開始
  let all = "";
  const nodes: { node: Text; start: number }[] = [];
  let lastBlock: Element | null | undefined;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const block = n.parentElement?.closest(BLOCK) ?? null;
    if (lastBlock !== undefined && block !== lastBlock) all += SEP;
    lastBlock = block;
    nodes.push({ node: n as Text, start: all.length });
    all += (n.nodeValue || "").toLocaleLowerCase();
  }
  // 位置 → (節點, 節點內的位移)：二分找最後一個 start ≤ pos 的節點（大檔、常見字也不會慢）；
  // end＝true 時落在節點的開頭就改用前一個節點的尾端（Range 才不會多包一段）
  const locate = (pos: number, end: boolean) => {
    let lo = 0;
    let hi = nodes.length - 1;
    let k = -1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (nodes[mid].start <= pos) {
        k = mid;
        lo = mid + 1;
      } else hi = mid - 1;
    }
    if (k < 0) return null;
    if (end && k > 0 && nodes[k].start === pos) k--;
    const { node, start } = nodes[k];
    const off = pos - start;
    return off <= (node.nodeValue || "").length ? { node, off } : null;
  };
  const out: Range[] = [];
  let i = all.indexOf(q);
  while (i >= 0) {
    const a = locate(i, false);
    const b = locate(i + q.length, true);
    if (a && b) {
      const r = document.createRange();
      r.setStart(a.node, a.off);
      r.setEnd(b.node, b.off);
      out.push(r);
    }
    i = all.indexOf(q, i + q.length);
  }
  return out;
}

type HighlightCtor = new (...ranges: Range[]) => unknown;
interface HighlightRegistry {
  set(name: string, h: unknown): void;
  delete(name: string): void;
}

function registry(): { reg: HighlightRegistry; Ctor: HighlightCtor } | null {
  const css = globalThis.CSS as unknown as { highlights?: HighlightRegistry } | undefined;
  const Ctor = (globalThis as unknown as { Highlight?: HighlightCtor }).Highlight;
  return css?.highlights && Ctor ? { reg: css.highlights, Ctor } : null;
}

/** 標出全部符合的位置，目前這筆另外標；捲到目前這筆。 */
export function showMatches(ranges: Range[], current: number) {
  const r = registry();
  if (r) {
    r.reg.set("find", new r.Ctor(...ranges));
    if (ranges[current]) r.reg.set("find-current", new r.Ctor(ranges[current]));
    else r.reg.delete("find-current");
  }
  const el = ranges[current]?.startContainer.parentElement;
  el?.scrollIntoView?.({ block: "center" });
}

export function clearMatches() {
  const r = registry();
  r?.reg.delete("find");
  r?.reg.delete("find-current");
}
