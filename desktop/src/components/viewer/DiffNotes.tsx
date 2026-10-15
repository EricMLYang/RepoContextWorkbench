import { useState } from "react";
import { anchorNotes, diffLineRefs, type DiffNote } from "../../lib/view";
import { useStore, useUi } from "../../store/context";

/** 差異太長時先畫幾行。 */
const DIFF_CAP = 4000;

/** 差異的一行屬於哪種（上色用）。 */
function diffLineKind(l: string): string {
  if (l.startsWith("+++") || l.startsWith("---")) return "meta";
  if (l.startsWith("+")) return "add";
  if (l.startsWith("-")) return "del";
  if (l.startsWith("@@")) return "hunk";
  if (/^(diff |index |new file|deleted file|similarity|rename |old mode|new mode)/.test(l))
    return "meta";
  return "ctx";
}

/**
 * 差異（git diff 的文字）：加的綠、刪的紅、段落標頭藍；太長先畫前 DIFF_CAP 行。
 * F-75：滑過一行按「＋」留言，留好幾則再「貼給會話」一次送出（一次思考、一次修改，參考 Orca Annotate AI Diff）。
 * 留言由呼叫端保管（切去預覽再回來還在）；差異重讀後照內容找回原來那行。
 */
export function DiffView({
  repo,
  path,
  text,
  truncated,
  notes,
  setNotes,
}: {
  repo: string;
  path: string;
  text: string;
  truncated: boolean;
  notes: DiffNote[];
  setNotes: (n: DiffNote[]) => void;
}) {
  const store = useStore();
  const ui = useUi();
  const [all, setAll] = useState(false);
  const [editing, setEditing] = useState<{ idx: number; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  if (!text) return <p className="viewernote">這個檔沒有還沒 commit 的變更。</p>;
  const lines = text.split("\n");
  const refs = diffLineRefs(lines);
  const at = anchorNotes(notes, lines, refs);
  const lost = notes.filter((_, i) => at[i] < 0);
  const shown = all ? lines : lines.slice(0, DIFF_CAP);
  const save = () => {
    if (!editing) return;
    const note = editing.text.trim();
    const where = refs[editing.idx];
    const rest = notes.filter((_, i) => at[i] !== editing.idx);
    setNotes(note && where ? [...rest, { where, code: lines[editing.idx], note }] : rest);
    setEditing(null);
  };
  const send = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const live = notes.filter((_, i) => at[i] >= 0);
      const r = await store.action("diff_notes_brief", { id: repo, path, notes: live });
      const title = ui.pasteToSession(r.text);
      if (!title) throw new Error("目前的會話沒有連上；先在下方終端選一個 agent 會話");
      store.toast(`已把 ${r.count} 則留言貼到「${title}」（還沒送出，看一眼再按 Enter）`);
    } catch (e) {
      store.toast((e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      {truncated ? <p className="viewernote">差異太長，只顯示前 512 KB。</p> : null}
      {notes.length ? (
        <div className="diffnotesbar">
          <span className="small">留言 {notes.length - lost.length} 則</span>
          {lost.length ? (
            <span className="small muted">
              （{lost.length} 則的那一行已經不在差異裡，不會送出）
            </span>
          ) : null}
          <span className="spacer" />
          <button type="button" className="small" disabled={busy} onClick={send}>
            {busy ? "整理中…" : "貼給會話"}
          </button>
          <button type="button" className="small ghost" onClick={() => setNotes([])}>
            清掉留言
          </button>
        </div>
      ) : null}
      <pre className="diffview" aria-label="差異">
        {shown.map((l, i) => {
          const k = at.indexOf(i);
          const noted = k >= 0 ? notes[k] : null;
          return (
            <div key={i}>
              <div className={"dl " + diffLineKind(l)}>
                {refs[i] ? (
                  <button
                    type="button"
                    className="dlnote"
                    title={"在" + refs[i] + "留言"}
                    aria-label={"在" + refs[i] + "留言"}
                    onClick={() => setEditing({ idx: i, text: noted?.note ?? "" })}
                  >
                    ＋
                  </button>
                ) : null}
                {l || " "}
              </div>
              {editing?.idx === i ? (
                <div className="dlcomment editing">
                  <textarea
                    autoFocus
                    rows={3}
                    value={editing.text}
                    placeholder={refs[i] + "：想請 agent 怎麼改？（⌘Enter 存、Esc 取消）"}
                    onChange={(e) => setEditing({ idx: i, text: e.target.value })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) save();
                      if (e.key === "Escape") setEditing(null);
                    }}
                  />
                  <div className="dlcommentacts">
                    <button type="button" className="small" onClick={save}>
                      存
                    </button>
                    <button type="button" className="small ghost" onClick={() => setEditing(null)}>
                      取消
                    </button>
                  </div>
                </div>
              ) : noted ? (
                <div className="dlcomment">
                  <span className="dlcommenttext">{noted.note}</span>
                  <button
                    type="button"
                    className="small ghost"
                    onClick={() => setEditing({ idx: i, text: noted.note })}
                  >
                    改
                  </button>
                  <button
                    type="button"
                    className="small ghost"
                    aria-label="刪掉這則留言"
                    onClick={() => setNotes(notes.filter((_, j) => j !== k))}
                  >
                    ✕
                  </button>
                </div>
              ) : null}
            </div>
          );
        })}
      </pre>
      {lines.length > shown.length ? (
        <button type="button" className="small" onClick={() => setAll(true)}>
          顯示全部 {lines.length} 行
        </button>
      ) : null}
    </>
  );
}
