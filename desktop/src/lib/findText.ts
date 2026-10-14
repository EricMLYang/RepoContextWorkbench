/**
 * 在一塊畫好的內容裡找字（檢視器的 ⌘F；系統 WebView 沒有內建的尋找列）。
 * 只做畫面：走文字節點算出 Range，用 CSS Custom Highlight API 標起來（不改 DOM，React 不會打架）；
 * 不支援 Highlight 的環境（舊 WebView、測試）只回數量、照樣能捲到那一筆。
 */

/** 找出 root 底下所有符合 query 的位置（不分大小寫）。 */
export function findRanges(root: Element, query: string): Range[] {
  const q = query.toLocaleLowerCase();
  if (!q) return [];
  const out: Range[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
    acceptNode: (n) =>
      // 行號（aria-hidden）不算
      n.parentElement?.closest("[aria-hidden='true']")
        ? NodeFilter.FILTER_REJECT
        : NodeFilter.FILTER_ACCEPT,
  });
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = (n.nodeValue || "").toLocaleLowerCase();
    let i = text.indexOf(q);
    while (i >= 0) {
      const r = document.createRange();
      r.setStart(n, i);
      r.setEnd(n, i + q.length);
      out.push(r);
      i = text.indexOf(q, i + q.length);
    }
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
