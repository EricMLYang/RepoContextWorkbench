import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";

const THRESHOLD = 4;

function same(a: readonly string[], b: readonly string[]) {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

/**
 * 直向清單的拖曳排序（用 pointer 事件自己做，不用 HTML5 drag & drop——Tauri 的檔案拖放會攔它）。
 * - 整列按住拖：移動超過 4px 才算拖，沒拖就照常是點擊。
 * - 鍵盤：Alt+↑／Alt+↓ 移一格。
 * - 放開就 `commit(新順序)`；存好之前先照新順序畫，失敗退回原順序（錯誤由 commit 自己顯示）。
 */
export function useSortable(items: string[], commit: (next: string[]) => Promise<void>) {
  const [override, setOverride] = useState<string[] | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  const els = useRef(new Map<string, HTMLElement>());
  const suppressClick = useRef(false);

  // 清單內容變了（後端存好回來、或別處改了）就不再用畫面暫存的順序；拖曳中不動
  const itemsKey = items.join("\u0000");
  const [seenKey, setSeenKey] = useState(itemsKey);
  if (seenKey !== itemsKey) {
    setSeenKey(itemsKey);
    if (!dragging) setOverride(null);
  }
  const order =
    override && override.length === items.length && override.every((x) => items.includes(x))
      ? override
      : items;

  const save = (next: string[]) => {
    if (same(next, items)) {
      setOverride(null);
      return;
    }
    setOverride(next);
    commit(next).catch(() => setOverride(null));
  };

  const ref = (id: string) => (el: HTMLElement | null) => {
    if (el) els.current.set(id, el);
    else els.current.delete(id);
  };

  const onPointerDown = (id: string) => (e: PointerEvent) => {
    if (e.button !== 0 || items.length < 2) return;
    const startY = e.clientY;
    let current = order;
    let moved = false;
    const onMove = (ev: globalThis.PointerEvent) => {
      if (!moved && Math.abs(ev.clientY - startY) < THRESHOLD) return;
      if (!moved) {
        moved = true;
        setDragging(id);
      }
      const rest = current.filter((x) => x !== id);
      let at = 0;
      for (const other of rest) {
        const r = els.current.get(other)?.getBoundingClientRect();
        if (r && ev.clientY > r.top + r.height / 2) at += 1;
      }
      const next = [...rest.slice(0, at), id, ...rest.slice(at)];
      if (!same(next, current)) {
        current = next;
        setOverride(next);
      }
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
      if (!moved) return;
      setDragging(null);
      suppressClick.current = true;
      setTimeout(() => (suppressClick.current = false), 0);
      save(current);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
  };

  const onKeyDown = (id: string) => (e: KeyboardEvent) => {
    if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    const i = order.indexOf(id);
    const j = e.key === "ArrowUp" ? i - 1 : i + 1;
    if (i < 0 || j < 0 || j >= order.length) return;
    const next = order.slice();
    [next[i], next[j]] = [next[j], next[i]];
    save(next);
  };

  /** 給可拖的那一列：ref、pointer、鍵盤、拖完不觸發點擊。 */
  const rowProps = (id: string) => ({
    ref: ref(id),
    onPointerDown: onPointerDown(id),
    onKeyDown: onKeyDown(id),
    onClickCapture: (e: { preventDefault(): void; stopPropagation(): void }) => {
      if (suppressClick.current) {
        e.preventDefault();
        e.stopPropagation();
      }
    },
    "data-dragging": dragging === id ? "true" : undefined,
  });

  return { order, dragging, rowProps };
}
