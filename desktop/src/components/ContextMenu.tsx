import { useEffect, useLayoutEffect, useRef, useState } from "react";

export interface MenuItem {
  label: string;
  run: () => void | Promise<void>;
  danger?: boolean;
  /** 右邊淡色提示（例：快捷鍵）。 */
  hint?: string;
  /** 同名項目要分開時給（預設用 label）。 */
  key?: string;
  /** 縮排層級（大綱用）。 */
  indent?: number;
}

/**
 * 右鍵選單（repo、檔案樹共用）。帶「…」的項目才跳視窗。
 * 方向鍵移動、Enter 執行、Esc／點外面／視窗失焦或縮放就關閉；不超出視窗。
 */
export function ContextMenu(props: {
  id?: string;
  title: string;
  items: MenuItem[];
  x: number;
  y: number;
  /** 往上開：y 是選單的下緣（貼在畫面底部的按鈕用，照實際高度放）。 */
  above?: boolean;
  onClose: () => void;
}) {
  const { id, title, items, x, y, above, onClose } = props;
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  // 不超出視窗；開啟時聚焦第一項
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      left: Math.max(4, Math.min(x, window.innerWidth - r.width - 4)),
      top: Math.max(4, Math.min(above ? y - r.height : y, window.innerHeight - r.height - 4)),
    });
    el.querySelector<HTMLButtonElement>("button")?.focus();
  }, [x, y, title, above]);

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
      id={id}
      className="ctxmenu"
      role="menu"
      aria-label={title + " 的動作"}
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
      <div className="ctxtitle">{title}</div>
      {items.map((it) => (
        <button
          key={it.key ?? it.label}
          type="button"
          role="menuitem"
          className={"ghost" + (it.danger ? " danger" : "")}
          style={it.indent ? { paddingLeft: 10 + it.indent * 14 } : undefined}
          onClick={() => {
            onClose();
            void it.run();
          }}
        >
          <span>{it.label}</span>
          {it.hint ? <span className="ctxhint">{it.hint}</span> : null}
        </button>
      ))}
    </div>
  );
}
