import { useEffect, useState, type KeyboardEvent, type PointerEvent } from "react";
import { load, save } from "../lib/storage";

interface Spec {
  /** CSS 變數（styles.css 的 grid 用它當欄寬）。 */
  cssVar: "--sidew" | "--panelw";
  /** 存在本機偏好的鍵。 */
  key: string;
  min: number;
  max: number;
  label: string;
}

const SPECS: Record<"left" | "right", Spec> = {
  left: { cssVar: "--sidew", key: "sideWidth", min: 180, max: 520, label: "調整左側欄寬度" },
  right: { cssVar: "--panelw", key: "panelWidth", min: 240, max: 720, label: "調整右側欄寬度" },
};

/** 不超過視窗 45%，免得把中間擠沒。 */
function clamp(spec: Spec, w: number) {
  return Math.round(Math.max(spec.min, Math.min(w, spec.max, window.innerWidth * 0.45)));
}

function apply(spec: Spec, w: number | null) {
  if (w == null) document.documentElement.style.removeProperty(spec.cssVar);
  else document.documentElement.style.setProperty(spec.cssVar, w + "px");
}

function current(spec: Spec) {
  const v = getComputedStyle(document.documentElement).getPropertyValue(spec.cssVar);
  return parseFloat(v) || spec.min;
}

/** 開 App 時套用上次存的寬度（只記在這台電腦）。 */
export function useSavedWidths() {
  useEffect(() => {
    for (const spec of Object.values(SPECS)) {
      const w = load<number | null>(spec.key, null);
      if (w) apply(spec, clamp(spec, w));
    }
  }, []);
}

/**
 * 側欄邊上的拖曳把手：左側欄在它右邊、右側欄在它左邊。拖曳調寬、雙擊回預設、
 * 聚焦後方向鍵每次 16px。寬度存在本機偏好。
 */
export function Resizer({ side }: { side: "left" | "right" }) {
  const spec = SPECS[side];
  const [active, setActive] = useState(false);
  // 一掛上就有值：可聚焦的 separator 必須有 aria-valuenow（axe aria-required-attr）
  const [now, setNow] = useState<number>(() => current(spec));

  const set = (w: number) => {
    const c = clamp(spec, w);
    apply(spec, c);
    setNow(c);
    return c;
  };

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const startX = e.clientX;
    const startW = current(spec);
    let last = startW;
    setActive(true);
    document.body.classList.add("resizing");
    const onMove = (ev: globalThis.PointerEvent) => {
      const dx = ev.clientX - startX;
      last = set(side === "left" ? startW + dx : startW - dx);
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
      document.body.classList.remove("resizing");
      setActive(false);
      save(spec.key, last);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const grow = (e.key === "ArrowRight") === (side === "left");
    save(spec.key, set(current(spec) + (grow ? 16 : -16)));
  };

  return (
    <div
      className={"resizer " + side + (active ? " active" : "")}
      role="separator"
      aria-orientation="vertical"
      aria-label={spec.label}
      aria-valuemin={spec.min}
      aria-valuemax={spec.max}
      aria-valuenow={now}
      tabIndex={0}
      title="拖曳調整寬度；雙擊回預設"
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
      onDoubleClick={() => {
        apply(spec, null);
        setNow(current(spec));
        save(spec.key, null);
      }}
    />
  );
}
