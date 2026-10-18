import { MSG_COPY_FAILED, copyText } from "../../lib/clipboard";
import { useStore, useUi } from "../../store/context";
import { ContextMenu, type MenuItem } from "../ContextMenu";
import { repoDetailSpec } from "./RepoDetail";
import { canRemoveFromScope, removeFromScope } from "./repoActions";

/**
 * repo 的右鍵選單：在側邊欄開啟、更多細節…、開會話、複製路徑、從這組移除、移出工作台…。
 * 帶「…」的才跳視窗。方向鍵移動、Enter 執行、Esc／點外面關閉。
 */
export function RepoMenu(props: { id: string; x: number; y: number; onClose: () => void }) {
  const { id, x, y, onClose } = props;
  const store = useStore();
  const ui = useUi();

  const path = store.getSnapshot().lastStates[id]?.path;
  const items: MenuItem[] = [
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
          await copyText(store.backend, path);
          store.toast("已複製絕對路徑：" + path);
        } catch {
          store.toast(MSG_COPY_FAILED);
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

  return <ContextMenu id="repomenu" title={id} items={items} x={x} y={y} onClose={onClose} />;
}
