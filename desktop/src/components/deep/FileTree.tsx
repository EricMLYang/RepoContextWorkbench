import { useEffect, useState, type MouseEvent, type ReactNode } from "react";
import type { TreeEntry } from "../../api/types";
import { useStore } from "../../store/context";

interface DirState {
  entries?: TreeEntry[];
  more?: number;
  loading?: boolean;
  error?: string;
}

/** 複製路徑：成功／失敗都用 toast 說。 */
function useCopy() {
  const store = useStore();
  return async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      store.toast("已複製" + label + "：" + text);
    } catch {
      store.toast("無法複製到剪貼簿");
    }
  };
}

function CopyButtons({ abs, rel, name }: { abs: string; rel?: string; name: string }) {
  const copy = useCopy();
  return (
    <span className="treecopy">
      <button
        type="button"
        className="ghost small"
        title={"複製絕對路徑：" + abs}
        aria-label={"複製 " + name + " 的絕對路徑"}
        onClick={() => void copy(abs, "絕對路徑")}
      >
        絕對
      </button>
      {rel ? (
        <button
          type="button"
          className="ghost small"
          title={"複製相對路徑：" + rel}
          aria-label={"複製 " + name + " 的相對路徑"}
          onClick={() => void copy(rel, "相對路徑")}
        >
          相對
        </button>
      ) : null}
    </span>
  );
}

/**
 * 一個 repo 當根的檔案樹（像 IDE 側邊欄）：展開資料夾才向引擎要下一層（action `repo_tree`），
 * 每列可複製絕對／相對路徑。讀失敗保留上次的清單並顯示錯誤。
 */
export function RepoRoot(props: {
  id: string;
  defaultOpen?: boolean;
  actions?: ReactNode;
  /** 只畫樹、不畫根目錄那一列（側邊欄用：標題已經是 repo 名）。 */
  bare?: boolean;
  /** 根目錄那一列按右鍵。 */
  onRootMenu?: (e: MouseEvent) => void;
}) {
  const { id, actions, bare = false, onRootMenu } = props;
  const defaultOpen = bare || !!props.defaultOpen;
  const store = useStore();
  const [root, setRoot] = useState("");
  const [dirs, setDirs] = useState<Record<string, DirState>>({});
  const [open, setOpen] = useState<Set<string>>(() => new Set(defaultOpen ? [""] : []));

  const load = async (dir: string) => {
    setDirs((d) => ({ ...d, [dir]: { ...d[dir], loading: true, error: "" } }));
    try {
      const r = await store.action("repo_tree", { id, dir });
      setRoot(r.root);
      setDirs((d) => ({ ...d, [dir]: { entries: r.entries, more: r.more } }));
    } catch (e) {
      setDirs((d) => ({ ...d, [dir]: { ...d[dir], loading: false, error: (e as Error).message } }));
    }
  };

  // 預設展開的根目錄：掛上時讀一次
  useEffect(() => {
    if (defaultOpen) void load("");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  const toggle = (dir: string) => {
    const next = new Set(open);
    if (next.has(dir)) next.delete(dir);
    else {
      next.add(dir);
      if (!dirs[dir]?.entries && !dirs[dir]?.loading) void load(dir);
    }
    setOpen(next);
  };

  const reload = () => {
    for (const dir of open) void load(dir);
  };

  const rows = (dir: string, depth: number): ReactNode => {
    const st = dirs[dir];
    const pad = { paddingLeft: 8 + depth * 14 + "px" };
    return (
      <>
        {st?.error ? (
          <li className="treenote error" style={pad} role="alert">
            {st.error}
          </li>
        ) : null}
        {!st?.entries && st?.loading ? (
          <li className="treenote muted" style={pad}>
            讀取中…
          </li>
        ) : null}
        {st?.entries && !st.entries.length ? (
          <li className="treenote muted" style={pad}>
            （空的）
          </li>
        ) : null}
        {st?.entries?.map((e) => (
          <li key={e.path} role="treeitem" aria-expanded={e.dir ? open.has(e.path) : undefined}>
            <div className="treerow" style={pad}>
              {e.dir ? (
                <button type="button" className="ghost treename" onClick={() => toggle(e.path)}>
                  <span className="treecaret" aria-hidden="true">
                    {open.has(e.path) ? "▾" : "▸"}
                  </span>
                  {e.name}/
                </button>
              ) : (
                <span className="treename file" title={e.path}>
                  {e.name}
                </span>
              )}
              <CopyButtons abs={e.abs} rel={e.path} name={e.name} />
            </div>
            {e.dir && open.has(e.path) ? <ul role="group">{rows(e.path, depth + 1)}</ul> : null}
          </li>
        ))}
        {st?.more ? (
          <li className="treenote muted" style={pad}>
            另 {st.more} 項沒列出
          </li>
        ) : null}
      </>
    );
  };

  const rootOpen = open.has("");
  if (bare)
    return (
      <ul role="tree" aria-label={"檔案：" + id} className="tree bare">
        {rows("", 0)}
      </ul>
    );
  return (
    <section className="reporoot" aria-label={"檔案：" + id}>
      <div className="treerow rootrow" onContextMenu={onRootMenu}>
        <button
          type="button"
          className="ghost treename"
          aria-expanded={rootOpen}
          onClick={() => toggle("")}
        >
          <span className="treecaret" aria-hidden="true">
            {rootOpen ? "▾" : "▸"}
          </span>
          <strong>{id}</strong>
          {root ? <span className="muted small rootpath">{root}</span> : null}
        </button>
        <span className="treecopy always">
          {root ? <CopyButtons abs={root} name={id} /> : null}
          <button
            type="button"
            className="ghost small"
            title="重新讀取已展開的資料夾"
            aria-label={"重新整理 " + id}
            disabled={!open.size || Object.values(dirs).some((d) => d.loading)}
            onClick={reload}
          >
            ⟳
          </button>
          {actions}
        </span>
      </div>
      {rootOpen ? (
        <ul role="tree" aria-label={id} className="tree">
          {rows("", 0)}
        </ul>
      ) : null}
    </section>
  );
}
