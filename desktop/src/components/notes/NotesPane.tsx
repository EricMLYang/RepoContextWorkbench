import { useEffect, useRef, useState } from "react";
import type { GroupNote } from "../../api/types";
import { Markdown, type MdHooks } from "../../lib/markdown";
import { continueList, indent } from "../../lib/mdedit";
import { load, save } from "../../lib/storage";
import { MSG_COPY_FAILED, copyText as writeClipboard } from "../../lib/clipboard";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

async function copyText(store: ReturnType<typeof useStore>, text: string, label: string) {
  try {
    await writeClipboard(store.backend, text);
    store.toast("已複製" + label);
  } catch {
    store.toast(MSG_COPY_FAILED);
  }
}

/** 一則筆記：時間、內文（Markdown）、複製／交給 Agent／刪除（刪除要按兩次）。 */
function NoteItem(props: {
  group: string;
  note: GroupNote;
  onRemoved: (notes: GroupNote[]) => void;
}) {
  const { group, note, onRemoved } = props;
  const store = useStore();
  const ui = useUi();
  const [confirm, setConfirm] = useState(false);
  const hooks: MdHooks = {
    onLink(href) {
      void copyText(store, href, "連結（App 內不開外部網頁）：" + href);
    },
    image: (src, alt, key) => (
      <span key={key} className="mdimgalt">
        ［圖片：{alt || src}］
      </span>
    ),
  };
  return (
    <li className="note">
      <div className="notehead">
        <time className="small muted">{note.at}</time>
        <span className="spacer" />
        <button
          type="button"
          className="ghost small"
          onClick={() => void copyText(store, note.text, "這則筆記")}
        >
          複製
        </button>
        <button
          type="button"
          className="ghost small"
          title="開一個這組的 Agent 會話，把這則當任務"
          onClick={() => ui.openSession({ group, task: note.text, origin: "note:" + group })}
        >
          交給 Agent…
        </button>
        {confirm ? (
          <>
            <AsyncButton
              className="ghost small danger"
              onError={(m) => store.toast(m)}
              run={async () => {
                const r = await store.action("note_remove", {
                  group,
                  at: note.at,
                  text: note.text,
                });
                onRemoved(r.notes);
              }}
            >
              確定刪除
            </AsyncButton>
            <button type="button" className="ghost small" onClick={() => setConfirm(false)}>
              取消
            </button>
          </>
        ) : (
          <button type="button" className="ghost small" onClick={() => setConfirm(true)}>
            刪除
          </button>
        )}
      </div>
      <div className="notebody">
        <Markdown text={note.text} hooks={hooks} />
      </div>
    </li>
  );
}

/**
 * 筆記分頁：快速記下「屬於這個組、不屬於任何 repo」的事。存在 spine 的 `groups/<組>/notes.md`
 * （普通 Markdown，人跟 agent 都能直接讀）。草稿存本機；送出失敗保留草稿並顯示原因。
 */
export function NotesPane({
  query,
  active,
  focusSeq = 0,
}: {
  query: string;
  active: boolean;
  /** 每次遞增就把游標放進輸入框（選單列「記一筆…」）。 */
  focusSeq?: number;
}) {
  const store = useStore();
  const { scope, state, stateReady } = useSnapshot();
  const group =
    scope.group && state.groups.some((g) => g.name === scope.group) ? scope.group : null;
  const [notes, setNotes] = useState<GroupNote[]>([]);
  const [path, setPath] = useState("");
  const [shownGroup, setShownGroup] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [draft, setDraftState] = useState("");
  const [draftGroup, setDraftGroup] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState("");
  const box = useRef<HTMLTextAreaElement>(null);

  // 換組：換成那組的草稿
  if (draftGroup !== group) {
    setDraftGroup(group);
    setDraftState(group ? load<string>("noteDraft:" + group, "") : "");
    setSaveError("");
  }
  const setDraft = (v: string) => {
    setDraftState(v);
    if (group) save("noteDraft:" + group, v);
  };

  // 切到這個分頁或換組就讀一次（檔案可能被人或 agent 改過）；讀失敗保留上次的
  useEffect(() => {
    if (!active || !group) return;
    let live = true;
    store
      .action("group_notes", { group })
      .then((r) => {
        if (!live) return;
        setNotes(r.notes);
        setPath(r.path);
        setShownGroup(group);
        setError("");
      })
      .catch((e: Error) => live && setError(e.message));
    return () => {
      live = false;
    };
  }, [store, active, group]);

  // 切到這個分頁、或選單列「記一筆…」，就把游標放進輸入框
  useEffect(() => {
    if (active && group) box.current?.focus();
  }, [active, group, focusSeq]);

  async function submit() {
    const text = draft.trim();
    if (!group || !text || saving) return;
    setSaving(true);
    setSaveError("");
    try {
      const r = await store.action("note_add", { group, text });
      setNotes(r.notes);
      setPath(r.path);
      setShownGroup(group);
      if ((box.current?.value ?? "").trim() === text) setDraft("");
    } catch (e) {
      setSaveError((e as Error).message + " 草稿已保留。");
    } finally {
      setSaving(false);
    }
  }

  const list = shownGroup === group ? notes : [];
  const loading = shownGroup !== group && !error;
  const shown = list.filter((n) => !query || n.text.toLocaleLowerCase().includes(query));
  return (
    <section id="tab-notes" className={"tabpane" + (active ? " on" : "")} aria-label="筆記">
      {!group ? (
        <div className="emptyline">
          {!stateReady
            ? "正在載入…"
            : "筆記記在牌組上。現在看的是「所有 repo」或臨時組——先在左側選一個牌組，或把臨時組存成牌組。"}
        </div>
      ) : (
        <>
          <div className="notecapture">
            <label htmlFor="notebox" className="sr-only">
              記一筆到 {group}
            </label>
            <textarea
              id="notebox"
              ref={box}
              rows={3}
              placeholder={"記一筆到「" + group + "」：不屬於任何 repo 的事、待確認的、臨時想到的…"}
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.nativeEvent.isComposing) return;
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                  e.preventDefault();
                  void submit();
                  return;
                }
                // 跟檔案編輯一樣：清單按 Enter 自動延續、Tab／⇧Tab 縮排
                const el = e.currentTarget;
                const a = el.selectionStart;
                const b = el.selectionEnd;
                const r =
                  e.key === "Enter" && !e.shiftKey && !e.altKey && a === b
                    ? continueList(el.value, a)
                    : e.key === "Tab" && !e.altKey && !e.metaKey && !e.ctrlKey
                      ? indent(el.value, a, b, e.shiftKey)
                      : null;
                if (!r) return;
                e.preventDefault();
                setDraft(r.text);
                requestAnimationFrame(() => box.current?.setSelectionRange(r.start, r.end));
              }}
            />
            <div className="composerfoot">
              <span className="small muted" title={path}>
                存在 spine：groups/{group}/notes.md（Markdown，agent 也讀得到）
              </span>
              <span className="spacer" />
              {path ? (
                <button
                  type="button"
                  className="ghost small"
                  onClick={() => void copyText(store, path, "筆記檔路徑：" + path)}
                >
                  複製檔案路徑
                </button>
              ) : null}
              <span className="small muted">Ctrl / ⌘ + Enter</span>
              <button
                type="button"
                className="primary"
                disabled={saving || !draft.trim()}
                onClick={() => void submit()}
              >
                {saving ? "記錄中…" : "記下"}
              </button>
            </div>
            {saveError ? (
              <p className="error" role="alert">
                {saveError}
              </p>
            ) : null}
          </div>
          {error ? (
            <p className="error" role="alert">
              {error}
            </p>
          ) : null}
          {shown.length ? (
            <ul className="notes" aria-label={group + " 的筆記"}>
              {shown.map((n, i) => (
                <NoteItem
                  key={n.at + "\u0000" + i + "\u0000" + n.text}
                  group={group}
                  note={n}
                  onRemoved={setNotes}
                />
              ))}
            </ul>
          ) : (
            <div className="emptyline">
              {loading && !list.length
                ? "讀取中…"
                : query && list.length
                  ? "沒有符合條件的筆記"
                  : "這組還沒有筆記。"}
            </div>
          )}
        </>
      )}
    </section>
  );
}
