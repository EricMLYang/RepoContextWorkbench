import { useState } from "react";
import type { Relation, RelationSuggestion } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { matchesSearch, relationLabel } from "../../lib/view";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { AsyncButton } from "../AsyncButton";

/** 關係分頁：碰到目前範圍的關係（跨組保留另一端）、用關係開工作、管理、新增。 */
export function RelationsPane({ query, active }: { query: string; active: boolean }) {
  const { state } = useSnapshot();
  const ui = useUi();
  // in_scope／task 由引擎給（碰到目前範圍的關係、用這條關係開工作的任務）
  const list = state.relations.filter((r) => r.in_scope && matchesSearch(r, query));
  return (
    <section
      id="tab-relations"
      className={"tabpane" + (active ? " on" : "")}
      aria-label="Repo 關係"
    >
      <div className="sectionintro">
        <div>
          <h2>Repo 關係</h2>
          <p className="small muted">
            目前範圍的 repo 如何規劃、供料與相互支援。跨組關係會保留另一端。
          </p>
        </div>
        <button
          type="button"
          id="reladd"
          onClick={() =>
            ui.openDetail({
              title: "新增 Repo 關係",
              eyebrow: "RELATIONSHIP",
              content: <RelationForm />,
            })
          }
        >
          ＋ 新增關係
        </button>
      </div>
      <Suggestions />
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
                      task: r.task,
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

/** 關係建議（引擎算好的）：理由＋證據；有推測種類的直接套用，沒有的開預填表單；不相關＝記住不再建議。 */
function Suggestions() {
  const { state } = useSnapshot();
  const list = state.relation_suggestions ?? [];
  if (!list.length) return null;
  return (
    <div id="relsuggest" className="relsuggest">
      <h3 className="small">建議設定的關係（{list.length}）</h3>
      <p className="small muted">
        從文件互相提到、引用紀錄推出來的；設了之後 agent 開場就看得到鄰居。
      </p>
      {list.map((x) => (
        <SuggestionRow key={x.from + ">" + x.to} x={x} />
      ))}
    </div>
  );
}

function SuggestionRow({ x }: { x: RelationSuggestion }) {
  const store = useStore();
  const ui = useUi();
  const { state } = useSnapshot();
  const [error, setError] = useState("");
  const label = x.kind ? (state.rel_kinds[x.kind]?.forward ?? x.kind) : "有關";
  return (
    <div className="relation suggestion">
      <div className="relationcopy">
        <div className="relationname">
          {x.from}
          <span>{label + " →"}</span>
          {x.to}
        </div>
        <p>{x.reason}</p>
        {x.files.length ? <p className="small muted">{"依據：" + x.files.join("、")}</p> : null}
        <p className="error" role="alert" hidden={!error}>
          {error}
        </p>
      </div>
      <div className="relacts">
        {x.kind ? (
          <AsyncButton
            className="small primary"
            onError={setError}
            run={async () => {
              await store.action("relate", { a: x.from, b: x.to, kind: x.kind ?? "", note: "" });
              await store.refreshState();
              store.toast("關係已建立");
            }}
          >
            套用
          </AsyncButton>
        ) : null}
        <button
          type="button"
          className="small"
          onClick={() =>
            ui.openDetail({
              title: "新增 Repo 關係",
              eyebrow: "RELATIONSHIP",
              content: <RelationForm initial={{ a: x.from, b: x.to, kind: x.kind ?? undefined }} />,
            })
          }
        >
          設定…
        </button>
        <AsyncButton
          className="ghost small"
          onError={setError}
          run={async () => {
            await store.action("relation_dismiss", { a: x.from, b: x.to });
            await store.refreshState();
          }}
        >
          不相關
        </AsyncButton>
      </div>
    </div>
  );
}

function RelationForm({ initial }: { initial?: { a: string; b: string; kind?: string } }) {
  const store = useStore();
  const { setError, close } = useDetail();
  const { state } = useSnapshot();
  const inScope = store.scopeRepos;
  // 建議的來源可能在範圍外（跨組關係）：預填時一併列進來源選單
  const sources = initial && !inScope.includes(initial.a) ? [initial.a, ...inScope] : inScope;
  const kinds = Object.keys(state.rel_kinds);
  const [a, setA] = useState(initial?.a ?? sources[0] ?? "");
  const [kind, setKind] = useState(
    initial?.kind && kinds.includes(initial.kind) ? initial.kind : (kinds[0] ?? ""),
  );
  // 預設目標避開跟來源同一個
  const [b, setB] = useState(() => {
    if (initial?.b) return initial.b;
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
