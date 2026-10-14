import { useEffect, useState } from "react";
import type { AssistPlan, AssistStep } from "../api/types";
import { useStore, useUi, type Tab } from "../store/context";
import { DetailLayout, useDetail } from "./Modal";

type Store = ReturnType<typeof useStore>;
type Ui = ReturnType<typeof useUi>;

const str = (v: unknown) => (typeof v === "string" ? v : "");
const strs = (v: unknown) => (Array.isArray(v) ? v.map(String) : []);

/**
 * 執行一步：呼叫原本的 action（照舊留痕），畫面類的（換範圍、分頁、開 repo、開會話對話框）由前端自己做。
 * 判斷與驗證都在引擎（`assist_plan`）；這裡只照步驟對應到既有的動作。
 */
async function runStep(store: Store, ui: Ui, s: AssistStep): Promise<void> {
  const a = s.args;
  switch (s.op) {
    case "switch_scope":
      if (str(a.group)) store.setScope({ group: str(a.group), repos: null });
      else store.setScope({ group: null, repos: strs(a.repos) });
      return;
    case "open_tab":
      ui.showTab(str(a.tab) as Tab);
      return;
    case "open_repo":
      ui.showRepo(str(a.id));
      return;
    case "create_group":
      await store.action("save_group", { name: str(a.name), repos: strs(a.repos) });
      return;
    case "set_members":
      await store.action("update_group", { name: str(a.group), repos: strs(a.repos) });
      return;
    case "rename_group": {
      const from = str(a.group);
      const members = store.getSnapshot().state.groups.find((g) => g.name === from)?.members;
      if (!members) throw new Error("找不到牌組「" + from + "」");
      const r = await store.action("update_group", {
        name: from,
        repos: members,
        new_name: str(a.new_name),
      });
      if (store.getSnapshot().scope.group === from) store.setScope({ group: r.name, repos: null });
      ui.renameOpenGroup(from, r.name);
      return;
    }
    case "set_goal":
      await store.action("set_goal", { group: str(a.group), text: str(a.text) });
      return;
    case "tag":
      await store.action("tag", { id: str(a.id), add: strs(a.add), remove: strs(a.remove) });
      return;
    case "set_tier":
      await store.action("tier", {
        id: str(a.id),
        tier: str(a.tier) as never,
        ...(str(a.resume_when) ? { resume_when: str(a.resume_when) } : {}),
      });
      return;
    case "remove_group":
      // 只會出現在復原（剛建的組拿掉）
      await store.action("remove_group", { name: str(a.name) });
      return;
    default:
      throw new Error("App 不認得這個步驟：" + s.op);
  }
}

/**
 * App 助手（F-63）：一句話 → 引擎給驗證過的計畫 → 人勾選、按「執行」→ 依序呼叫原本的 action。
 * 一律先確認；驗證不過的步驟不能勾；會移掉東西的標紅。任何一步失敗就停，列出做過的。
 * 「開 Agent 會話」放最後做（會換成會話對話框）。
 */
export function AssistDialog({ text }: { text: string }) {
  const store = useStore();
  const ui = useUi();
  const { setError, close } = useDetail();
  const [plan, setPlan] = useState<AssistPlan | null>(null);
  const [picked, setPicked] = useState<boolean[]>([]);
  const [running, setRunning] = useState(-1);
  const [done, setDone] = useState<number[]>([]);
  const [failed, setFailed] = useState<{ i: number; msg: string } | null>(null);
  // 執行前的範圍（復原「換範圍」用）；復原跑完沒有
  const [prevScope, setPrevScope] = useState<ReturnType<Store["getSnapshot"]>["scope"] | null>(
    null,
  );
  const [undoing, setUndoing] = useState<"" | "running" | "done">("");

  useEffect(() => {
    let live = true;
    store
      .action("assist_plan", { text, ...store.scopePayload })
      .then((p) => {
        if (!live) return;
        setPlan(p);
        setPicked(p.steps.map((s) => !s.error));
      })
      .catch((e: Error) => live && setError("助手沒辦法規劃：" + e.message));
    return () => {
      live = false;
    };
  }, [store, text, setError]);

  const runnable = plan ? plan.steps.filter((s, i) => picked[i] && !s.error) : [];
  const finished = !!plan && (done.length > 0 || !!failed) && running < 0;

  const execute = async () => {
    if (!plan || running >= 0) return;
    setError("");
    setFailed(null);
    const order = plan.steps
      .map((s, i) => ({ s, i }))
      .filter(({ s, i }) => picked[i] && !s.error)
      // 開會話對話框會換掉這個對話框，放最後
      .sort((x, y) => Number(x.s.op === "open_session") - Number(y.s.op === "open_session"));
    setPrevScope(store.getSnapshot().scope);
    const ok: number[] = [];
    let fail: { i: number; msg: string } | null = null;
    let session: AssistStep | null = null;
    for (const { s, i } of order) {
      if (s.op === "open_session") {
        session = s;
        ok.push(i);
        continue;
      }
      setRunning(i);
      try {
        await runStep(store, ui, s);
        ok.push(i);
        setDone([...ok]);
      } catch (e) {
        fail = { i, msg: (e as Error).message };
        setFailed(fail);
        break;
      }
    }
    setRunning(-1);
    const scope = store.getSnapshot().scope;
    await store
      .action("assist_log", {
        text: plan.text,
        ...(scope.group ? { group: scope.group } : {}),
        done: ok
          .filter((i) => !fail || plan.steps[i].op !== "open_session")
          .map((i) => plan.steps[i].text),
        skipped: plan.steps.filter((_, i) => !ok.includes(i) && i !== fail?.i).map((s) => s.text),
        ...(fail ? { failed: plan.steps[fail.i].text + "：" + fail.msg } : {}),
      })
      .catch(() => {});
    await store.refreshState().catch(() => {});
    if (fail) return;
    if (session) {
      const a = session.args;
      const repo = str(a.repo);
      ui.openSession({
        ...(repo ? { repo, repos: [repo] } : str(a.group) ? { group: str(a.group) } : {}),
        task: str(a.task),
        origin: "assist",
      });
      return;
    }
    store.toast("助手：完成 " + ok.length + " 步（可在對話框裡復原）");
  };

  // 做過、而且復原得了的步驟（倒著復原）
  const undoable = plan
    ? done.filter((i) => plan.steps[i].undo || plan.steps[i].op === "switch_scope").reverse()
    : [];
  const undo = async () => {
    if (!plan || undoing) return;
    setUndoing("running");
    setError("");
    const back: string[] = [];
    try {
      for (const i of undoable) {
        const s = plan.steps[i];
        if (s.op === "switch_scope") {
          if (prevScope) store.setScope(prevScope);
        } else if (s.undo) {
          await runStep(store, ui, { ...s, op: s.undo.op, args: s.undo.args });
        }
        back.push(s.text);
      }
      setUndoing("done");
      store.toast("已復原 " + back.length + " 步");
    } catch (e) {
      setUndoing("");
      setError("復原到一半失敗：" + (e as Error).message);
    } finally {
      await store
        .action("assist_log", {
          text: "復原：" + plan.text,
          done: back,
          skipped: [],
        })
        .catch(() => {});
      await store.refreshState().catch(() => {});
    }
  };

  return (
    <DetailLayout
      actions={
        finished ? (
          <>
            {undoable.length && undoing !== "done" ? (
              <button
                type="button"
                className="ghost"
                disabled={undoing === "running"}
                title="倒著把這次做過的改回去（照規劃當下的設定）"
                onClick={() => void undo()}
              >
                {undoing === "running" ? "復原中…" : "復原這次的變更"}
              </button>
            ) : null}
            <button type="button" onClick={close}>
              關閉
            </button>
          </>
        ) : (
          <>
            <button type="button" className="ghost" disabled={running >= 0} onClick={close}>
              取消
            </button>
            <button
              type="button"
              className="primary"
              disabled={!plan || !runnable.length || running >= 0}
              onClick={() => void execute()}
            >
              {running >= 0 ? "執行中…" : "執行 " + runnable.length + " 步"}
            </button>
          </>
        )
      }
    >
      <p className="assistask">「{text}」</p>
      {!plan ? (
        <p className="muted">正在規劃…（常見說法立刻好；其他要問 Agent，幾秒鐘）</p>
      ) : (
        <>
          {plan.summary ? <p>{plan.summary}</p> : null}
          {!plan.steps.length ? (
            <p className="muted">助手沒有找到能做的步驟。可以換個說法，或直接用按鈕操作。</p>
          ) : (
            <ol className="assiststeps">
              {plan.steps.map((s, i) => (
                <li
                  key={i}
                  className={
                    (s.danger ? "danger " : "") +
                    (s.error ? "bad " : "") +
                    (done.includes(i) ? "done " : "") +
                    (failed?.i === i ? "failed" : "")
                  }
                >
                  <label>
                    <input
                      type="checkbox"
                      checked={!!picked[i] && !s.error}
                      disabled={!!s.error || running >= 0 || finished}
                      onChange={(e) =>
                        setPicked((p) => p.map((v, j) => (j === i ? e.target.checked : v)))
                      }
                    />
                    <span>{s.text}</span>
                    {done.includes(i) ? <span className="small"> ✓</span> : null}
                    {running === i ? <span className="small muted"> 執行中…</span> : null}
                  </label>
                  {s.error ? (
                    <div className="small assisterr">不能執行：{s.error}</div>
                  ) : s.why ? (
                    <div className="small muted">{s.why}</div>
                  ) : null}
                  {failed?.i === i ? (
                    <div className="small assisterr" role="alert">
                      失敗：{failed.msg}（後面的步驟沒有做）
                    </div>
                  ) : null}
                </li>
              ))}
            </ol>
          )}
          <p className="small muted">
            {plan.source === "rules" ? "本機規則翻譯" : "Agent 規劃"}
            ；每一步都走原本的按鈕動作，會留痕。勾掉不想做的再執行。
          </p>
        </>
      )}
    </DetailLayout>
  );
}
