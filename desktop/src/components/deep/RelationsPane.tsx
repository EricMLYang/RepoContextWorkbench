import { useState } from "react";
import type { Relation } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { matchesSearch, relationLabel, relationTask, scopeRepos } from "../../lib/view";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/** 關係分頁：碰到目前範圍的關係（跨組保留另一端）、用關係開工作、管理、新增。 */
export function RelationsPane({ query, active }: { query: string; active: boolean }) {
  const { state, scope } = useSnapshot();
  const ui = useUi();
  const members = new Set(scopeRepos(scope, state));
  const list = state.relations.filter(
    (r) => (members.has(r.from) || members.has(r.to)) && matchesSearch(r, query),
  );
  return (
    <section id="tab-relations" className={"tabpane" + (active ? " on" : "")} aria-label="Repo 關係">
      <div className="sectionintro">
        <div>
          <h2>Repo 關係</h2>
          <p className="small muted">目前範圍的 repo 如何規劃、供料與相互支援。跨組關係會保留另一端。</p>
        </div>
        <button
          type="button"
          id="reladd"
          onClick={() =>
            ui.openDetail({ title: "新增 Repo 關係", eyebrow: "RELATIONSHIP", content: <RelationForm /> })
          }
        >
          ＋ 新增關係
        </button>
      </div>
      <div id="reltable">
        {list.length ? (
          list.map((r) => (
            <div className="relation" key={r.from + ">" + r.to + ":" + r.kind}>
              <div className="relationcopy">
                <div className="relationname">
                  {r.from}
                  <span>{relationLabel(r, state.rel_kinds) + " →"}</span>
                  {r.to}
                </div>
                <p>{r.note || "尚未填寫說明"}</p>
              </div>
              <div className="relacts">
                <button
                  type="button"
                  className="small"
                  onClick={() =>
                    ui.openSession({
                      repos: [r.from, r.to],
                      task: relationTask(r),
                      origin: "relation:" + r.from + ">" + r.to,
                    })
                  }
                >
                  用這條關係開工作
                </button>
                <button
                  type="button"
                  className="ghost small"
                  onClick={() =>
                    ui.openDetail({ title: "管理關係", content: <ManageRelation r={r} /> })
                  }
                >
                  管理
                </button>
              </div>
            </div>
          ))
        ) : (
          <div className="emptystate">還沒有符合範圍的關係。新增一條，讓組內角色更清楚。</div>
        )}
      </div>
    </section>
  );
}

function RelationForm() {
  const store = useStore();
  const { setError, close } = useDetail();
  const { state, scope } = useSnapshot();
  const sources = scopeRepos(scope, state);
  const kinds = Object.keys(state.rel_kinds);
  const [a, setA] = useState(sources[0] ?? "");
  const [kind, setKind] = useState(kinds[0] ?? "");
  // 預設目標避開跟來源同一個
  const [b, setB] = useState(() => {
    const first = state.repos[0] ?? "";
    if (first === a && state.repos.length > 1) return state.repos[1];
    return first;
  });
  const [note, setNote] = useState("");
  return (
    <DetailLayout
      actions={
        <AsyncButton
          className="primary"
          run={async () => {
            if (a === b) {
              setError("來源與目標需要是不同 repo。");
              return;
            }
            await store.action("relate", { a, b, kind, note: note.trim() });
            close();
            store.toast("關係已建立");
            await store.refreshState();
          }}
        >
          新增關係
        </AsyncButton>
      }
    >
      <label className="field">
        來源 repo
        <select value={a} onChange={(e) => setA(e.target.value)}>
          {sources.map((v) => (
            <option key={v} value={v}>
              {v}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        如何連結
        <select value={kind} onChange={(e) => setKind(e.target.value)}>
          {kinds.map((v) => (
            <option key={v} value={v}>
              {state.rel_kinds[v].forward}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        目標 repo
        <select value={b} onChange={(e) => setB(e.target.value)}>
          {state.repos.map((v) => (
            <option key={v} value={v}>
              {v}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        說明（選填）
        <input value={note} onChange={(e) => setNote(e.target.value)} />
      </label>
    </DetailLayout>
  );
}

function ManageRelation({ r }: { r: Relation }) {
  const store = useStore();
  const { close } = useDetail();
  const { state } = useSnapshot();
  return (
    <DetailLayout
      actions={
        <>
          <button type="button" onClick={close}>
            取消
          </button>
          <AsyncButton
            className="danger"
            run={async () => {
              await store.action("unrelate", { a: r.from, b: r.to, kind: r.kind });
              close();
              await store.refreshState();
              store.toast("關係已移除");
            }}
          >
            移除這條關係
          </AsyncButton>
        </>
      }
    >
      <p className="dialogmessage">
        {`${r.from} ${relationLabel(r, state.rel_kinds)} → ${r.to}\n${r.note || ""}\n\n移除只會解除這條關係，repo 與檔案會保留。`}
      </p>
    </DetailLayout>
  );
}
