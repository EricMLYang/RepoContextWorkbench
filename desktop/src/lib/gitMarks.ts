import type { RepoChange } from "../api/types";

/** 檔案樹上的 git 記號（跟 VS Code 一樣）：還沒 commit 的六種狀態，加上「已 commit 還沒 push」。 */
export type GitMark = RepoChange["status"] | "unpushed";

/** 資料夾取最重的：衝突 > 刪除 > 修改 > 改名 > 已暫存 > 新檔 > 沒 push。 */
const WEIGHT: Record<GitMark, number> = {
  conflicted: 7,
  deleted: 6,
  modified: 5,
  renamed: 4,
  added: 3,
  untracked: 2,
  unpushed: 1,
};

/** 檔案列右邊的字母（資料夾一律畫點）。 */
export const MARK_LETTER: Record<GitMark, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "U",
  conflicted: "!",
  unpushed: "↑",
};

export const MARK_LABEL: Record<GitMark, string> = {
  modified: "已修改",
  added: "已暫存",
  deleted: "已刪除",
  renamed: "已改名",
  untracked: "新檔，還沒追蹤",
  conflicted: "合併衝突",
  unpushed: "還沒 push",
};

/**
 * 路徑（相對 repo 根）→ 記號。檔案照自己的狀態；資料夾照裡面最重的（含還沒 push 的）。
 * 同一個檔既在沒 push 的 commit 裡、又有新的改動，算還沒 commit 的那種（後處理的蓋掉）。
 */
export function gitMarks(changes: RepoChange[], unpushed: string[]): Map<string, GitMark> {
  const marks = new Map<string, GitMark>();
  const mark = (path: string, m: GitMark) => {
    marks.set(path, m);
    for (let i = path.lastIndexOf("/"); i > 0; i = path.lastIndexOf("/")) {
      path = path.slice(0, i);
      const cur = marks.get(path);
      if (!cur || WEIGHT[m] > WEIGHT[cur]) marks.set(path, m);
    }
  };
  for (const p of unpushed) mark(p, "unpushed");
  for (const c of changes) mark(c.path, c.status);
  return marks;
}
