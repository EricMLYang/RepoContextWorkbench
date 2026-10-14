import { useEffect, useRef, useState } from "react";
import { ContextMenu } from "../ContextMenu";
import { fileKey, type OpenFile } from "./FileViewer";

const parts = (p: string) => p.replace(/\\/g, "/").split("/").filter(Boolean);

/**
 * 分頁標籤（UX-08）：預設只顯示檔名；同名的才補最短能分辨的上層目錄，路徑也一樣才補 repo。
 * 回傳 [檔名, 補充說明]（沒有補充＝""）。
 */
export function tabLabels(files: OpenFile[]): [string, string][] {
  return files.map((f) => {
    const ps = parts(f.path);
    const name = ps[ps.length - 1] || f.path;
    const twins = files.filter((o) => o !== f && (parts(o.path).pop() || o.path) === name);
    if (!twins.length) return [name, ""];
    // 從最近的上層目錄往上加，直到跟每個同名檔都不一樣
    for (let depth = 1; depth < ps.length; depth++) {
      const tail = ps.slice(-1 - depth, -1).join("/");
      const clash = twins.some(
        (o) =>
          parts(o.path)
            .slice(-1 - depth, -1)
            .join("/") === tail,
      );
      if (!clash) return [name, tail];
    }
    // 路徑一模一樣（不同 repo）：補 repo
    const dir = ps.slice(0, -1).join("/");
    return [name, dir ? f.repo + " › " + dir : f.repo];
  });
}

/** 主區最上方的分頁列：「工作區」＋開著的檔（中鍵或 × 關閉；有未儲存修改的標 ●）；▾ 列出全部開著的檔。 */
export function FileTabs(props: {
  files: OpenFile[];
  active: string | null;
  dirty?: ReadonlySet<string>;
  onSelect: (key: string | null) => void;
  onClose: (key: string) => void;
}) {
  const { files, active, dirty, onSelect, onClose } = props;
  const navRef = useRef<HTMLElement>(null);
  const listRef = useRef<HTMLButtonElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const labels = tabLabels(files);

  // 選中的分頁捲進看得到的地方（分頁很多時）
  useEffect(() => {
    navRef.current
      ?.querySelector(".filetab.on")
      ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [active, files.length]);

  return (
    <nav className="filetabs" aria-label="主區分頁" ref={navRef}>
      <button
        type="button"
        className={"filetab home" + (active ? "" : " on")}
        aria-current={active ? undefined : "page"}
        onClick={() => onSelect(null)}
      >
        工作區
      </button>
      {files.map((f, i) => {
        const k = fileKey(f);
        const [name, hint] = labels[i];
        return (
          <span
            key={k}
            className={"filetab" + (k === active ? " on" : "")}
            onAuxClick={(e) => {
              if (e.button === 1) onClose(k);
            }}
          >
            <button
              type="button"
              className="tabname"
              title={f.repo + " / " + f.path}
              aria-current={k === active ? "page" : undefined}
              onClick={() => onSelect(k)}
            >
              {name}
              {hint ? <span className="muted small"> · {hint}</span> : null}
              {dirty?.has(k) ? (
                <span className="dirtymark" aria-label="未儲存">
                  {" "}
                  ●
                </span>
              ) : null}
            </button>
            <button
              type="button"
              className="tabclose"
              aria-label={"關閉 " + name}
              title="關閉"
              onClick={() => onClose(k)}
            >
              ×
            </button>
          </span>
        );
      })}
      {files.length > 1 ? (
        <button
          ref={listRef}
          type="button"
          className="ghost small tablistbtn"
          aria-label="開啟的檔案清單"
          aria-haspopup="menu"
          aria-expanded={!!menu}
          title="開啟的檔案清單"
          onClick={() => {
            const r = listRef.current?.getBoundingClientRect();
            setMenu(menu ? null : { x: r ? r.right - 260 : 0, y: r ? r.bottom + 4 : 0 });
          }}
        >
          ▾ {files.length}
        </button>
      ) : null}
      {menu ? (
        <ContextMenu
          id="tablistmenu"
          title="開啟的檔案"
          items={files.map((f) => ({
            label: f.repo + " / " + f.path + (dirty?.has(fileKey(f)) ? " ●" : ""),
            run: () => onSelect(fileKey(f)),
          }))}
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
        />
      ) : null}
    </nav>
  );
}
