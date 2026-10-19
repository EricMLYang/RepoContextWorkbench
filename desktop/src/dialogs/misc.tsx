import { useState } from "react";
import { imeKey } from "../lib/ime";
import type { GroupInfo } from "../api/types";
import { AsyncButton } from "../components/AsyncButton";
import { useSnapshot, useStore, useUi } from "../store/context";
import { DetailLayout, useAutoFocus, useDetail } from "./Modal";

/** 「所有組標為已讀…」確認框：顯示影響則數。 */
export function AckConfirm() {
  const store = useStore();
  const { close } = useDetail();
  const { state } = useSnapshot();
  const count = state.unread?.length || 0;
  return (
    <DetailLayout
      actions={
        <>
          <button type="button" onClick={close}>
            取消
          </button>
          <AsyncButton
            className="primary"
            run={async () => {
              await store.action("ack", {});
              close();
              await store.refreshState();
              store.toast("所有組的事件已標為已讀");
            }}
          >
            所有組標為已讀
          </AsyncButton>
        </>
      }
    >
      <p className="dialogmessage">
        {`這會標記所有組目前的 ${count} 則未讀事件，包含其他牌組的提醒。\n\n處理中任務與待跟進事項會保留。標記已讀不代表完成處理。`}
      </p>
    </DetailLayout>
  );
}

/** 刪除牌組確認（從編輯牌組進來；取消回到編輯）。 */
export function DeleteGroup({ group }: { group: GroupInfo }) {
  const store = useStore();
  const ui = useUi();
  const { close } = useDetail();
  const name = group.name;
  return (
    <DetailLayout
      actions={
        <>
          <button
            type="button"
            onClick={() => {
              close();
              ui.openPicker(group);
            }}
          >
            取消
          </button>
          <AsyncButton
            className="danger"
            run={async () => {
              await store.action("remove_group", { name });
              if (store.getSnapshot().scope.group === name)
                store.setScope({ group: null, repos: null });
              ui.forgetOpenGroup(name);
              close();
              await store.refreshState();
              store.toast("已刪除牌組「" + name + "」");
            }}
          >
            刪除牌組
          </AsyncButton>
        </>
      }
    >
      <p className="dialogmessage">
        {`只會刪除這個牌組的定義。\n\n裡面的 ${group.members.length} 個 repo 仍保留登記；這組過去的簡報與存料（groups/${name}/）也會留著。`}
      </p>
    </DetailLayout>
  );
}

/**
 * 首次使用：輸入資料夾 → 預覽候選 → 勾選 → 命名第一組 → 登記。
 * `mode="add"`（檔案分頁「掃描資料夾…」）：之後隨時加 repo；勾選的逐一加進目前牌組（沒牌組就只登記）。
 */
export function Onboarding({ mode = "first" }: { mode?: "first" | "add" }) {
  const store = useStore();
  const adding = mode === "add";
  const { setError, close } = useDetail();
  const dirRef = useAutoFocus<HTMLInputElement>();
  const [dir, setDir] = useState("");
  const [found, setFound] = useState<{ path: string; id: string }[] | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [name, setName] = useState("");
  const [previewing, setPreviewing] = useState(false);
  const [creating, setCreating] = useState(false);

  const preview = async () => {
    if (previewing) return;
    setPreviewing(true);
    setError("");
    try {
      const r = await store.action("scan_dir", { dir: dir.trim() });
      setFound(r.candidates);
      setPicked(new Set(r.candidates.map((c) => c.path)));
    } catch (e) {
      setFound([]);
      setPicked(new Set());
      setError((e as Error).message);
    } finally {
      setPreviewing(false);
    }
  };

  const create = async () => {
    if (creating || !picked.size || !found) return;
    setCreating(true);
    setError("");
    try {
      const repos = found.filter((c) => picked.has(c.path));
      if (adding) {
        const { scope } = store.getSnapshot();
        const ids: string[] = [];
        for (const c of repos) {
          const r = await store.action(
            "group_add_folder",
            scope.group ? { path: c.path, group: scope.group } : { path: c.path },
          );
          ids.push(r.id);
        }
        close();
        await store.refreshState();
        if (!scope.group && Array.isArray(scope.repos)) {
          store.setScope({ group: null, repos: [...new Set([...scope.repos, ...ids])] });
        } else await store.rescan();
        store.toast(
          "已加入 " +
            ids.length +
            " 個 repo" +
            (scope.group ? "到牌組「" + scope.group + "」" : ""),
        );
        return;
      }
      const r = await store.action("register_repos", { repos, group: name.trim() });
      close();
      await store.refreshState();
      if (r.group) store.setScope({ group: r.group, repos: null });
      else await store.rescan();
      store.toast(
        "已登記 " +
          r.added.length +
          " 個 repo" +
          (r.group ? "，並建立牌組「" + r.group + "」" : ""),
      );
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setCreating(false);
    }
  };

  return (
    <DetailLayout
      actions={
        <>
          {store.backend.pickFolder ? (
            <button
              type="button"
              onClick={async () => {
                const p = await store.backend.pickFolder?.();
                if (p) setDir(p);
              }}
            >
              選擇資料夾…
            </button>
          ) : null}
          <button type="button" disabled={previewing} onClick={() => void preview()}>
            預覽這個資料夾
          </button>
          <button
            type="button"
            className="primary"
            disabled={creating || !picked.size}
            onClick={() => void create()}
          >
            {adding ? "加入選取的 repo" : "登記並建立這一組"}
          </button>
        </>
      }
    >
      <p className="detailmeta">
        工作台只看這個資料夾本身和下一層有沒有 .git，掃描不會改動任何 repo。
      </p>
      <label className="field">
        放 repo 的資料夾
        <input
          ref={dirRef}
          value={dir}
          placeholder="例如 /Users/you/Coding"
          onChange={(e) => setDir(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !imeKey(e)) {
              e.preventDefault();
              void preview();
            }
          }}
        />
      </label>
      <div className="onboardlist">
        {found === null ? null : found.length ? (
          found.map((c) => (
            <label className="pickrow" key={c.path}>
              <input
                type="checkbox"
                aria-label={"登記 " + c.id}
                checked={picked.has(c.path)}
                onChange={(e) => {
                  const next = new Set(picked);
                  if (e.target.checked) next.add(c.path);
                  else next.delete(c.path);
                  setPicked(next);
                }}
              />
              <span className="pickcopy">
                {c.id}
                <span className="picktags">{c.path}</span>
              </span>
            </label>
          ))
        ) : (
          <p className="emptyline">這個資料夾（和下一層）沒有尚未登記的 git repo。</p>
        )}
      </div>
      <label className="field" hidden={adding || !found?.length}>
        第一組的名稱
        <span className="muted small"> 選填，留空只登記 repo</span>
        <input
          placeholder="例如：產品開發"
          maxLength={100}
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </label>
    </DetailLayout>
  );
}
