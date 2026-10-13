import { fileKey, type OpenFile } from "./FileViewer";

/** 主區最上方的分頁列：「工作區」＋開著的檔（中鍵或 × 關閉；有未儲存修改的標 ●）。 */
export function FileTabs(props: {
  files: OpenFile[];
  active: string | null;
  dirty?: ReadonlySet<string>;
  onSelect: (key: string | null) => void;
  onClose: (key: string) => void;
}) {
  const { files, active, dirty, onSelect, onClose } = props;
  return (
    <nav className="filetabs" aria-label="主區分頁">
      <button
        type="button"
        className={"filetab home" + (active ? "" : " on")}
        aria-current={active ? undefined : "page"}
        onClick={() => onSelect(null)}
      >
        工作區
      </button>
      {files.map((f) => {
        const k = fileKey(f);
        const name = f.path.replace(/\\/g, "/").split("/").pop() || f.path;
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
              <span className="muted small"> · {f.repo}</span>
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
    </nav>
  );
}
