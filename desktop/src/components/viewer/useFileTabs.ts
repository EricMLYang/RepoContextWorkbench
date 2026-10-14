import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { load, save } from "../../lib/storage";
import { fileKey, type OpenFile } from "./FileViewer";

/** 最多開幾個檔案分頁（再開就擠掉最舊的，有未儲存修改的不擠）。 */
const MAX_TABS = 12;

/**
 * 主區的檔案分頁：開哪些檔、正在看哪個（null＝工作區）、哪些有未儲存的修改。
 * 開過的分頁記在本機；有未儲存修改的分頁關閉要按兩次。
 */
export function useFileTabs(toast: (msg: string) => void) {
  const [files, setFiles] = useState<OpenFile[]>(() => load<OpenFile[]>("openFiles", []));
  const [activeFile, setActiveFile] = useState<string | null>(() =>
    load<string | null>("activeFile", null),
  );
  useEffect(() => save("openFiles", files), [files]);
  useEffect(() => save("activeFile", activeFile), [activeFile]);
  const [dirtyFiles, setDirtyFiles] = useState<ReadonlySet<string>>(() => new Set());
  const dirtyRef = useRef(dirtyFiles);
  useEffect(() => {
    dirtyRef.current = dirtyFiles;
  }, [dirtyFiles]);
  const [armedClose, setArmedClose] = useState<string | null>(null);

  const setFileDirty = useCallback((k: string, dirty: boolean) => {
    setDirtyFiles((prev) => {
      if (prev.has(k) === dirty) return prev;
      const next = new Set(prev);
      if (dirty) next.add(k);
      else next.delete(k);
      return next;
    });
  }, []);

  // opts.diff：開在「差異」模式（從變更清單點的）；已經開著就換成那個模式
  // opts.edit：剛新增的檔，打開就進編輯
  const openFile = useCallback(
    (repo: string, path: string, opts?: { diff?: boolean; edit?: boolean }) => {
      const f: OpenFile = opts?.diff
        ? { repo, path, diff: true }
        : opts?.edit
          ? { repo, path, edit: true }
          : { repo, path };
      const k = fileKey(f);
      setFiles((prev) => {
        if (prev.some((x) => fileKey(x) === k))
          return opts?.diff ? prev.map((x) => (fileKey(x) === k ? f : x)) : prev;
        const next = [...prev, f];
        while (next.length > MAX_TABS) {
          const i = next.findIndex((x) => !dirtyRef.current.has(fileKey(x)));
          if (i < 0 || i === next.length - 1) break;
          next.splice(i, 1);
        }
        return next;
      });
      setActiveFile(k);
    },
    [],
  );

  const closeFile = useCallback(
    (k: string) => {
      if (dirtyFiles.has(k) && armedClose !== k) {
        setArmedClose(k);
        const f = files.find((x) => fileKey(x) === k);
        toast("「" + (f?.path ?? "") + "」有未儲存的修改；再按一次 × 放棄修改並關閉");
        return;
      }
      setArmedClose(null);
      const i = files.findIndex((x) => fileKey(x) === k);
      const next = files.filter((x) => fileKey(x) !== k);
      setFiles(next);
      setActiveFile((cur) =>
        cur !== k ? cur : next.length ? fileKey(next[Math.min(i, next.length - 1)]) : null,
      );
    },
    [files, dirtyFiles, armedClose, toast],
  );

  /** 改名後換掉開著的分頁（含資料夾底下的檔）；分頁的檔有未儲存修改就不能改名（見 dirtyUnder）。 */
  const under = (f: OpenFile, repo: string, path: string) =>
    f.repo === repo && (f.path === path || f.path.startsWith(path + "/"));
  // 穩定的函式（給 Ui 用，Ui 整個生命週期不變）：讀 ref 拿最新的分頁
  const filesRef = useRef(files);
  useEffect(() => {
    filesRef.current = files;
  }, [files]);
  const dirtyUnder = useCallback(
    (repo: string, path: string) =>
      filesRef.current.some((f) => under(f, repo, path) && dirtyRef.current.has(fileKey(f))),
    [],
  );
  const renamePath = useCallback((repo: string, from: string, to: string) => {
    const move = (f: OpenFile): OpenFile =>
      under(f, repo, from) ? { repo, path: to + f.path.slice(from.length) } : f;
    setFiles((prev) => {
      const next = prev.map(move);
      setActiveFile((cur) => {
        const i = prev.findIndex((f) => fileKey(f) === cur);
        return i >= 0 ? fileKey(next[i]) : cur;
      });
      return next;
    });
  }, []);

  const shownFile = files.some((f) => fileKey(f) === activeFile) ? activeFile : null;
  const shownFileObj = useMemo(
    () => files.find((f) => fileKey(f) === shownFile) ?? null,
    [files, shownFile],
  );
  return {
    files,
    shownFile,
    shownFileObj,
    dirtyFiles,
    openFile,
    closeFile,
    setActiveFile,
    setFileDirty,
    dirtyUnder,
    renamePath,
  };
}
