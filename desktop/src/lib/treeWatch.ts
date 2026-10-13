import { useEffect, useRef, useState } from "react";
import type { Backend } from "../api/backend";

/**
 * 檔案樹跟著磁碟變（Tauri 殼的 `watch_tree`／`repo://changed`）。
 * 每棵樹、每個檢視器把「我現在看著哪些資料夾」登記上來；合併後一次告訴殼。
 * 沒有東西登記＝殼不監看任何 repo（閒置不耗資源）。HTTP 後端沒有這功能，就全部不做。
 */
interface Registry {
  owners: Map<symbol, { id: string; dirs: string[] }>;
  timer: ReturnType<typeof setTimeout> | null;
  sent: string;
}

const registries = new WeakMap<Backend, Registry>();

function registry(b: Backend): Registry {
  let r = registries.get(b);
  if (!r) {
    r = { owners: new Map(), timer: null, sent: "{}" };
    registries.set(b, r);
  }
  return r;
}

/** 合併所有登記（同 repo 取聯集），跟上次送的一樣就不送。 */
function sync(b: Backend) {
  const r = registry(b);
  if (r.timer) clearTimeout(r.timer);
  r.timer = setTimeout(() => {
    r.timer = null;
    const union: Record<string, Set<string>> = {};
    for (const { id, dirs } of r.owners.values()) {
      const s = (union[id] ??= new Set());
      dirs.forEach((d) => s.add(d));
    }
    const plain: Record<string, string[]> = {};
    for (const id of Object.keys(union).sort()) plain[id] = [...union[id]].sort();
    const key = JSON.stringify(plain);
    if (key === r.sent) return;
    r.sent = key;
    b.watchTree?.(plain).catch(() => {
      r.sent = ""; // 下次再試
    });
  }, 60);
}

/** 登記「我看著 repo `id` 的這些資料夾」（`""`＝根）；卸載時取消。 */
export function useWatchDirs(backend: Backend, id: string, dirs: string[]) {
  const [token] = useState(() => Symbol(id));
  // JSON：只有根目錄（[""]）也要跟空清單分得開
  const key = JSON.stringify([...dirs].sort());
  useEffect(() => {
    if (!backend.watchTree) return;
    const r = registry(backend);
    r.owners.set(token, { id, dirs: JSON.parse(key) as string[] });
    sync(backend);
    return () => {
      r.owners.delete(token);
      sync(backend);
    };
    // key 已涵蓋 dirs 的內容
  }, [backend, id, key, token]);
}

/** repo `id` 展開中的資料夾裡有東西變了（相對路徑們；`rescan`＝不知道哪裡變了，全部重讀）。 */
export function useRepoChanged(
  backend: Backend,
  id: string,
  cb: (paths: string[], rescan: boolean) => void,
) {
  const ref = useRef(cb);
  useEffect(() => {
    ref.current = cb;
  });
  useEffect(() => {
    if (!backend.onRepoChanged) return;
    return backend.onRepoChanged((rid, paths, rescan) => {
      if (rid === id) ref.current(paths, !!rescan);
    });
  }, [backend, id]);
}

/** 相對路徑的上一層（`a/b.txt` → `a`，`b.txt` → `""`）；兩種分隔符都認。 */
export function parentDir(p: string) {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i < 0 ? "" : p.slice(0, i);
}
