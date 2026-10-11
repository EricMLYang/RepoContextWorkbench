import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useStore, useUi } from "../../store/context";
import { repoDetailSpec } from "./RepoDetail";
import { canRemoveFromScope, removeFromScope } from "./repoActions";

interface Item {
  label: string;
  run: () => void | Promise<void>;
  danger?: boolean;
}

/**
 * repo 的右鍵選單：在側邊欄開啟、更多細節…、開會話、複製路徑、從這組移除、移出工作台…。
 * 帶「…」的才跳視窗。方向鍵移動、Enter 執行、Esc／點外面關閉。
 */
export function RepoMenu(props: { id: string; x: number; y: number; onClose: () => void }) {
  const { id, x, y, onClose } = props;
  const store = useStore();
  const ui = useUi();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  const path = store.getSnapshot().lastStates[id]?.path;
  const items: Item[] = [
    { label: "在側邊欄開啟", run: () => ui.openRepo(id) },
    { label: "更多細節…", run: () => ui.openDetail(repoDetailSpec(id)) },
    {
      label: "開啟 Agent 會話…",
      run: () => ui.openSession({ repo: id, repos: [id], origin: "repo:" + id }),
    },
  ];
  if (path)
    items.push({
      label: "複製絕對路徑",
      run: async () => {
        try {
          await navigator.clipboard.writeText(path);
          store.toast("已複製絕對路徑：" + path);
        } catch {
          store.toast("無法複製到剪貼簿");
        }
      },
    });
  if (canRemoveFromScope(store, id))
    items.push({
      label: "從這組移除",
      run: () => removeFromScope(store, id).catch((e: Error) => store.toast(e.message)),
    });
  items.push({
    label: "移出工作台…",
    danger: true,
    run: () => ui.openDetail(repoDetailSpec(id, { confirmRemove: true })),
  });

  // 不超出視窗；開啟時聚焦第一項
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - r.width - 4)),
      top: Math.max(4, Math.min(y, window.innerHeight - r.height - 4)),
    });
    el.querySelector<HTMLButtonElement>("button")?.focus();
  }, [x, y, id]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onClose);
    window.addEventListener("resize", onClose);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("resize", onClose);
    };
  }, [onClose]);

  const move = (delta: number) => {
    const btns = [...(ref.current?.querySelectorAll<HTMLButtonElement>("button") || [])];
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    btns[(i + delta + btns.length) % btns.length]?.focus();
  };

  return (
    <div
      ref={ref}
      id="repomenu"
      className="ctxmenu"
      role="menu"
      aria-label={id + " 的動作"}
      style={{ left: pos.left, top: pos.top }}
      onKeyDown={(e) => {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          move(1);
        } else if (e.key === "ArrowUp") {
          e.preventDefault();
          move(-1);
        }
      }}
    >
      <div className="ctxtitle">{id}</div>
      {items.map((it) => (
        <button
          key={it.label}
          type="button"
          role="menuitem"
          className={"ghost" + (it.danger ? " danger" : "")}
          onClick={() => {
            onClose();
            void it.run();
          }}
        >
          {it.label}
        </button>
      ))}
    </div>
  );
}
