import type { WorkbenchStore } from "../../store/store";

/** 加一個資料夾到目前範圍（像 IDE 的 Add Folder to Workspace）：沒登記的先登記。 */
export async function addFolder(store: WorkbenchStore, path: string) {
  const { scope } = store.getSnapshot();
  const r = await store.action(
    "group_add_folder",
    scope.group ? { path, group: scope.group } : { path },
  );
  await store.refreshState();
  if (!scope.group && Array.isArray(scope.repos) && !scope.repos.includes(r.id)) {
    store.setScope({ group: null, repos: [...scope.repos, r.id] });
  } else {
    await store.rescan();
  }
  store.toast(
    (r.registered ? "已登記 " : "已加入 ") + r.id + (r.group ? "，在牌組「" + r.group + "」" : ""),
  );
}

/** 目前範圍是牌組或臨時組、且 repo 在裡面＝可以「從這組移除」（全部 repo 的範圍沒有「這組」）。 */
export function canRemoveFromScope(store: WorkbenchStore, id: string): boolean {
  const { scope } = store.getSnapshot();
  return (!!scope.group || Array.isArray(scope.repos)) && store.scopeRepos.includes(id);
}

/** 從目前牌組／臨時組移除（登記保留，檔案不動）。 */
export async function removeFromScope(store: WorkbenchStore, id: string) {
  const { scope, state } = store.getSnapshot();
  const group = scope.group ? state.groups.find((g) => g.name === scope.group) : undefined;
  if (group) {
    await store.action("update_group", {
      name: group.name,
      repos: group.members.filter((m) => m !== id),
    });
    await store.refreshState();
    store.toast("已從「" + group.name + "」移除 " + id + "（登記保留，檔案不動）");
  } else if (Array.isArray(scope.repos)) {
    store.setScope({ group: null, repos: scope.repos.filter((r) => r !== id) });
  }
}
