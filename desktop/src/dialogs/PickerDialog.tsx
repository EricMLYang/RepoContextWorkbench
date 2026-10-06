import { useEffect, useRef, useState, type FormEvent } from "react";
import type { GroupInfo } from "../api/types";
import { useSnapshot, useStore, useUi } from "../store/context";
import { DeleteGroup } from "./misc";
import { Modal } from "./Modal";

/**
 * 「選擇 repo」：組臨時範圍或存成牌組；傳入 group＝編輯既有牌組（預勾成員、可改名、刪除）。
 * 搜尋＋tag 只影響顯示；「選取搜尋結果」「清除選取」「＋相關 repo」是明確動作；零選取不能套用。
 */
export function PickerDialog(props: { open: boolean; group: GroupInfo | null; seq: number; onClose: () => void }) {
  const { open, group, seq, onClose } = props;
  return (
    <Modal id="pickdialog" open={open} onClose={onClose} labelledBy="picktitle">
      <PickerForm key={seq} group={group} onClose={onClose} />
    </Modal>
  );
}

function PickerForm({ group, onClose }: { group: GroupInfo | null; onClose: () => void }) {
  const store = useStore();
  const ui = useUi();
  const { state } = useSnapshot();
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set(group ? group.members : store.scopeRepos),
  );
  const [tags, setTags] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [name, setName] = useState(group ? group.name : "");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const searchRef = useRef<HTMLInputElement>(null);
  useEffect(() => searchRef.current?.focus(), []);

  const editing = group?.name ?? null;
  const q = query.toLocaleLowerCase();
  const matches = (r: string) =>
    r.toLocaleLowerCase().includes(q) &&
    (!tags.size || (state.tags[r] || []).some((t) => tags.has(t)));
  const used = [...new Set(Object.values(state.tags).flat())].sort();
  const visible = state.repos.filter(matches).slice().sort();

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (busy || !selected.size) return;
    const repos = [...selected],
      trimmed = name.trim();
    if (editing) {
      if (!trimmed) return setError("牌組名稱不可為空");
      setBusy(true);
      try {
        const r = await store.action("update_group", { name: editing, repos, new_name: trimmed });
        if (store.getSnapshot().scope.group === editing) store.setScope({ group: r.name, repos: null });
        if (r.name !== editing) ui.renameOpenGroup(editing, r.name);
        await store.refreshState();
        onClose();
        store.toast("已更新牌組「" + r.name + "」 · " + r.members.length + " 個 repo");
      } catch (err) {
        setError((err as Error).message);
      } finally {
        setBusy(false);
      }
      return;
    }
    setBusy(true);
    try {
      if (trimmed) {
        await store.action("save_group", { name: trimmed, repos });
        await store.refreshState();
      }
      store.setScope(trimmed ? { group: trimmed, repos: null } : { group: null, repos });
      onClose();
      store.toast(trimmed ? "已儲存牌組「" + trimmed + "」" : "已套用 " + repos.length + " 個 repo");
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setBusy(false);
    }
  };

  return (
    <form id="pickform" onSubmit={submit}>
      <div className="dialoghead">
        <div>
          <p className="eyebrow">WORKING SET</p>
          <h2 id="picktitle">{group ? "編輯牌組" : "選擇 repo"}</h2>
          <p id="picksub" className="muted">
            {group
              ? "勾選＝這組的成員；取消勾選只是移出這組，repo 登記不受影響。"
              : "組成臨時工作範圍，或儲存成常用牌組。"}
          </p>
        </div>
        <button type="button" className="icon ghost" aria-label="關閉選擇 repo" onClick={onClose}>
          ×
        </button>
      </div>
      <label className="searchbox">
        <span aria-hidden="true">⌕</span>
        <input
          id="picksearch"
          ref={searchRef}
          type="search"
          placeholder="搜尋 repo"
          aria-label="搜尋 repo"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </label>
      <div id="picktags" className="chips">
        {used.map((t) => (
          <button
            key={t}
            type="button"
            className={"chip" + (tags.has(t) ? " on" : "")}
            aria-pressed={tags.has(t)}
            onClick={() => {
              const next = new Set(tags);
              if (next.has(t)) next.delete(t);
              else next.add(t);
              setTags(next);
            }}
          >
            {t}
          </button>
        ))}
      </div>
      <div className="picktools">
        <button
          type="button"
          id="pickall"
          className="ghost small"
          onClick={() => setSelected(new Set([...selected, ...state.repos.filter(matches)]))}
        >
          選取搜尋結果
        </button>
        <button type="button" id="picknone" className="ghost small" onClick={() => setSelected(new Set())}>
          清除選取
        </button>
        <button
          type="button"
          id="pickrelated"
          className="ghost small"
          onClick={() => {
            const next = new Set(selected);
            for (const r of state.relations) {
              if (selected.has(r.from)) next.add(r.to);
              if (selected.has(r.to)) next.add(r.from);
            }
            setSelected(next);
            store.toast(
              next.size === selected.size
                ? "沒有其他相關 repo"
                : "已加入 " + (next.size - selected.size) + " 個相關 repo",
            );
          }}
        >
          ＋ 相關 repo
        </button>
        <span className="spacer" />
        <span id="pickcount" className="small">
          {"已選 " + selected.size + " 個"}
        </span>
      </div>
      <div id="picklist">
        {visible.length ? (
          visible.map((r) => (
            <label className="pickrow" key={r}>
              <input
                type="checkbox"
                aria-label={"選取 " + r}
                checked={selected.has(r)}
                onChange={(e) => {
                  const next = new Set(selected);
                  if (e.target.checked) next.add(r);
                  else next.delete(r);
                  setSelected(next);
                }}
              />
              <span className="pickcopy">
                {r}
                <span className="picktags">{(state.tags[r] || []).join(" · ")}</span>
              </span>
            </label>
          ))
        ) : (
          <p className="emptyline">沒有符合篩選的 repo；已選項目仍會保留。</p>
        )}
      </div>
      <label id="picknamelabel" className="field">
        {group ? "牌組名稱 " : "儲存牌組名稱 "}
        <span id="picknamehint" className="muted small" hidden={!!group}>
          選填，留空只套用範圍
        </span>
        <input
          id="pickname"
          placeholder="例如：產品研究"
          maxLength={100}
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </label>
      <p id="pickerror" className="error" role="alert" hidden={!error}>
        {error}
      </p>
      <div className="dialogfoot">
        <button
          type="button"
          id="pickdelete"
          className="danger"
          hidden={!group}
          disabled={busy}
          onClick={() => {
            const g = state.groups.find((x) => x.name === editing);
            if (!g) return;
            onClose();
            ui.openDetail({
              title: "刪除牌組「" + g.name + "」？",
              eyebrow: "GROUP",
              content: <DeleteGroup group={g} />,
            });
          }}
        >
          刪除牌組
        </button>
        <span className="spacer" />
        <button type="button" onClick={onClose}>
          取消
        </button>
        <button id="pickok" className="primary" type="submit" disabled={busy || !selected.size}>
          {editing ? "儲存變更" : "套用範圍"}
        </button>
      </div>
    </form>
  );
}
