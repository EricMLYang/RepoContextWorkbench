import { useState } from "react";
import type { RepoState } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { TIERNAME, parseTags } from "../../lib/view";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";
import { RepoRoot } from "./FileTree";

/** Repo 詳情：採集到的狀態、檔案清單、標籤編輯、開 Agent 會話、移出工作台。 */
export function RepoDetail({ id }: { id: string }) {
  const store = useStore();
  const ui = useUi();
  const { close } = useDetail();
  const { lastStates, state } = useSnapshot();
  const s: Partial<RepoState> = lastStates[id] || { id };
  const [tags, setTags] = useState((state.tags[id] || []).join(", "));
  const [confirmRemove, setConfirmRemove] = useState(false);
  const rows: [string, string | number | null | undefined][] = [
    ["分支", s.branch],
    ["狀態", (s.tier && TIERNAME[s.tier]) || s.tier],
    ["未提交", s.dirty == null ? "尚未採集" : s.dirty + " 檔"],
    ["最近 commit", s.last_subject],
    ["7 天 / 30 天", (s.commits_7d ?? "—") + " / " + (s.commits_30d ?? "—")],
    ["未推送 / 落後", (s.ahead ?? "—") + " / " + (s.behind ?? "—")],
    ["Worktree", s.worktrees],
    ["路徑", s.path],
    ["備註", s.note],
  ];
  return (
    <DetailLayout
      actions={
        <>
          {confirmRemove ? (
            <>
              <button type="button" onClick={() => setConfirmRemove(false)}>
                取消
              </button>
              <AsyncButton
                className="danger"
                run={async () => {
                  await store.action("remove_repo", { id });
                  close();
                  await store.refreshState();
                  await store.rescan();
                  store.toast("已把 " + id + " 移出工作台（資料夾與檔案沒動）");
                }}
              >
                確定移出
              </AsyncButton>
            </>
          ) : (
            <button type="button" className="ghost" onClick={() => setConfirmRemove(true)}>
              移出工作台…
            </button>
          )}
          <AsyncButton
            run={async () => {
              const next = parseTags(tags),
                old = store.getSnapshot().state.tags[id] || [];
              await store.action("tag", {
                id,
                add: next.filter((t) => !old.includes(t)),
                remove: old.filter((t) => !next.includes(t)),
              });
              await store.refreshState();
              store.toast("標籤已儲存");
              close();
            }}
          >
            儲存標籤
          </AsyncButton>
          <button
            type="button"
            className="primary"
            onClick={() => ui.openSession({ repo: id, repos: [id], origin: "repo:" + id })}
          >
            開啟 Agent 會話
          </button>
        </>
      }
    >
      <dl>
        {rows.map(([label, value]) => (
          <div className="detailrow" key={label}>
            <dt>{label}</dt>
            <dd>{value ?? "—"}</dd>
          </div>
        ))}
      </dl>
      {confirmRemove ? (
        <p className="detailmeta" role="alert">
          移出後 {id} 會從所有牌組拿掉、指向它的關係也會刪除；repo 資料夾與檔案不會動，之後可以再加回來。
        </p>
      ) : null}
      <RepoRoot id={id} defaultOpen />
      <label className="field">
        標籤（以逗號分隔）
        <input value={tags} onChange={(e) => setTags(e.target.value)} />
      </label>
    </DetailLayout>
  );
}

export function repoDetailSpec(id: string) {
  return { title: id, eyebrow: "REPO DETAILS", content: <RepoDetail id={id} /> };
}
