/**
 * 收件匣最上面的「要你決定」（F-127）：一次聚焦一張卡，其餘是下面的短列。
 * 資料全由引擎算好（`decide_queue`）；這裡只畫、收鍵盤、按了才呼叫 action。
 *
 * 鍵盤（焦點不在輸入框、沒有對話框時）：1–9 標記選項、Enter 送出標記的、Esc 取消標記、↑↓ 或 J／K 換卡。
 * 滑鼠點選項＝直接送（按鈕本身就是明確的選擇）；數字鍵容易按錯，所以先標記再 Enter。
 * 點錯了：「已記下」卡上的「撤回」（引擎留痕、那筆不算數、卡回來）；卡離開畫面後在「回答、放行與規則」面板撤回。
 * 做了收不回來的提問：沒有預設的主要按鈕（建議只標「建議」），避免順手按下去。
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { DecideCard, Decided } from "../../api/types";
import { KIND_LABEL, decideAge as ageText, withdrawnText } from "../../lib/view";
import { imeKey } from "../../lib/ime";
import { useSnapshot, useStore, useUi, type Ui } from "../../store/context";
import type { WorkbenchStore } from "../../store/store";
import { AsyncButton } from "../AsyncButton";
import { pasteWhenReady } from "../sessions/SessionRow";
import { autoListSpec } from "./DecideRules";

const cardKey = (c: DecideCard) => c.repo + "\u0000" + c.handoff_ref;

/** 焦點在能打字的地方、或有對話框開著：鍵盤留給它們。 */
function keysBelongElsewhere(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (document.querySelector("dialog[open]")) return true;
  if (!el || !el.tagName) return false;
  const tag = el.tagName.toLowerCase();
  return tag === "input" || tag === "textarea" || tag === "select" || el.isContentEditable;
}

/** 回答後的「下一步」：依送達方式送到 agent（一律要人按一下）。 */
async function deliver(ui: Ui, store: WorkbenchStore, d: Decided): Promise<string> {
  const r = d.relay;
  if (r.delivery.kind === "existing_session") {
    ui.selectTerm(r.delivery.sid);
    const title = await pasteWhenReady(ui, r.task);
    if (!title) throw new Error("會話還沒接上，再按一次");
    return `已貼到「${title}」（還沒送出，看一眼再按 Enter）`;
  }
  if (r.delivery.kind === "resume") {
    const res = await store.action("term_create", {
      kind: "agent",
      ...(r.delivery.agent ? { agent: r.delivery.agent } : {}),
      resume: r.delivery.claude_session_id,
      cwd: r.delivery.cwd,
      prompt: r.task,
      task: "接續：" + d.question,
    });
    await store.refreshState();
    ui.selectTerm(res.sid);
    return "已接回原會話";
  }
  ui.openSession({ repo: d.repo, task: r.task, origin: "decide:" + d.repo });
  return "";
}

function deliverLabel(d: Decided): string {
  switch (d.relay.delivery.kind) {
    case "existing_session":
      return "貼到開著的會話";
    case "resume":
      return "接回原會話接著做";
    default:
      return "開新會話接著做…";
  }
}

/** 回答之後：一句已記下＋怎麼送＋撤回＋（有的話）升級建議。 */
function AnsweredStep({
  d,
  onDone,
  onWithdrawn,
}: {
  d: Decided;
  onDone: () => void;
  onWithdrawn: () => Promise<void>;
}) {
  const ui = useUi();
  const store = useStore();
  const [error, setError] = useState("");
  const [armed, setArmed] = useState(false);
  const [trusted, setTrusted] = useState(false);
  const hint = d.suggestion;
  return (
    <article className="decidecard answered" aria-label={"已回答：" + d.repo}>
      <p className="decidedone">
        {"已記下：" + d.repo + "「" + d.question + "」→ " + d.answer}
        {d.option ? `（第 ${d.option} 個選項）` : ""}
      </p>
      {d.advice_differs ? (
        <p className="small muted">
          {(d.advice_differs.from.kind === "steward"
            ? "總管"
            : d.advice_differs.from.repo + " 的 agent") +
            "建議的是「" +
            d.advice_differs.text +
            "」，你選的是「" +
            d.answer +
            "」"}
        </p>
      ) : null}
      {d.downgraded ? (
        <p className="small notice">
          {"這類原本自動放行，這次跟建議不同：" +
            d.downgraded.repo +
            "／" +
            KIND_LABEL[d.downgraded.kind] +
            " 已降回每次都問。"}
        </p>
      ) : null}
      <div className="decideacts">
        <AsyncButton
          className="primary"
          onError={setError}
          run={async () => {
            setError("");
            const msg = await deliver(ui, store, d);
            if (msg) store.toast(msg);
            onDone();
          }}
        >
          {deliverLabel(d)}
        </AsyncButton>
        <button type="button" onClick={onDone}>
          之後再送
        </button>
        <span className="spacer" />
        <AsyncButton
          className="small"
          title="點錯了：這筆回答不算數（不算連續照建議），卡回到上面重新回答"
          onError={setError}
          run={async () => {
            setError("");
            await store.action("decide_overturn", { ref: d.ref });
            await onWithdrawn();
          }}
        >
          撤回
        </AsyncButton>
      </div>
      {hint && !trusted ? (
        <div className="decidetrust">
          <p>
            {`這類已連續 ${hint.streak} 次照建議：以後「${hint.repo}」的「${KIND_LABEL[hint.kind]}」提問自動放行？`}
          </p>
          <p className="small muted">
            自動＝agent
            問這類、有建議、做了收得回來時，直接照建議接著做並留痕；push、刪除、對外送出永遠不會自動。
            在「回答、放行與規則」面板隨時降回，推翻一次也會自動降回。
          </p>
          <AsyncButton
            className={armed ? "danger" : ""}
            onError={setError}
            run={async () => {
              if (!armed) {
                setArmed(true);
                return;
              }
              await store.action("decide_trust", { repo: hint.repo, kind: hint.kind });
              setTrusted(true);
              store.toast(`「${hint.repo}／${KIND_LABEL[hint.kind]}」以後照建議自動放行`);
              await store.refreshState();
            }}
          >
            {armed ? "再按一次，設為自動" : "設為自動"}
          </AsyncButton>
        </div>
      ) : null}
      {trusted ? (
        <p className="small muted">已設為自動；在「回答、放行與規則」面板可以降回。</p>
      ) : null}
      <p className="error" role="alert" hidden={!error}>
        {error}
      </p>
    </article>
  );
}

/** 聚焦的那張卡。 */
function FocusCard({
  c,
  marked,
  onAnswered,
}: {
  c: DecideCard;
  marked: number | null;
  onAnswered: (d: Decided) => void;
}) {
  const ui = useUi();
  const store = useStore();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [freeOpen, setFreeOpen] = useState(!c.options.length);
  const [text, setText] = useState("");
  const send = useCallback(
    async (reply: string) => {
      if (busy || !reply.trim()) return;
      setBusy(true);
      setError("");
      try {
        const d = await store.action("decide_answer", { repo: c.repo, reply: reply.trim() });
        onAnswered(d);
        void store.refreshState();
      } catch (e) {
        // 送不出去（會話正在處理中…）：卡片留著，說明為什麼
        setError((e as Error).message);
      } finally {
        setBusy(false);
      }
    },
    [busy, c.repo, onAnswered, store],
  );
  // Enter 送出標記的選項（由上層的鍵盤處理叫進來）
  const sendRef = useRef(send);
  useEffect(() => {
    sendRef.current = send;
  }, [send]);
  useEffect(() => {
    const on = (e: Event) => {
      const n = (e as CustomEvent<number>).detail;
      void sendRef.current(String(n));
    };
    window.addEventListener("decide:send", on);
    return () => window.removeEventListener("decide:send", on);
  }, []);
  const who = c.about || [c.role, c.class].filter(Boolean).join("・");
  const irreversible = c.reversible === false;
  // 收不回來的：沒有預設的主要按鈕，建議只用標籤標出
  const primaryOf = (recommended: boolean) => recommended && !irreversible;
  return (
    <article
      className="decidecard"
      aria-label={"要你決定：" + c.repo}
      aria-busy={busy || undefined}
    >
      <div className="decidewho">
        <button type="button" className="linkish decidrepo" onClick={() => ui.showRepo(c.repo)}>
          {c.repo}
        </button>
        {c.group ? <span className="muted small">{"〔" + c.group + "〕"}</span> : null}
        {who ? <span className="decideabout">{who}</span> : null}
        {!c.about ? (
          <button
            type="button"
            className="linkish small faint"
            title="寫一句這個 repo 是什麼：之後卡片、名冊、搜尋都用它"
            onClick={() => ui.showRepo(c.repo, "info")}
          >
            補一句介紹
          </button>
        ) : null}
      </div>
      <h3 className="decideq">{c.question}</h3>
      {irreversible ? (
        <p className="decidewarn" role="note">
          <strong>做了收不回來</strong>
          <span>沒有預設選項：看清楚再按，按了就送出。</span>
        </p>
      ) : null}
      {c.options.length ? (
        <div className="decideopts" role="group" aria-label="選項">
          {c.options.map((o) => (
            <button
              key={o.n}
              type="button"
              className={
                (primaryOf(o.recommended) ? "primary" : "") + (marked === o.n ? " marked" : "")
              }
              aria-pressed={marked === o.n}
              disabled={busy}
              title={`按 ${o.n} 標記、Enter 送出；點一下直接送出`}
              onClick={() => void send(String(o.n))}
            >
              <span className="optkey" aria-hidden="true">
                {o.n}
              </span>
              {o.text}
              {o.recommended ? <span className="opttag">建議</span> : null}
            </button>
          ))}
        </div>
      ) : null}
      {marked !== null ? (
        <p className="small muted" role="status">
          {`已標記第 ${marked} 個：Enter 送出、Esc 取消`}
        </p>
      ) : null}
      {c.options.length && !freeOpen ? (
        <button type="button" className="linkish small" onClick={() => setFreeOpen(true)}>
          用一句話回答…
        </button>
      ) : null}
      {freeOpen ? (
        <form
          className="decidefree"
          onSubmit={(e) => {
            e.preventDefault();
            void send(text);
          }}
        >
          <input
            value={text}
            disabled={busy}
            aria-label={"回答「" + c.question + "」"}
            placeholder={c.options.length ? "一句話回答（不照選項）" : "一句話回答，Enter 送出"}
            autoFocus={!!c.options.length}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => {
              // 輸入法選字的 Enter 不送
              if (imeKey(e)) {
                if (e.key === "Enter") e.preventDefault();
                return;
              }
              if (e.key === "Escape" && c.options.length) setFreeOpen(false);
            }}
          />
          <button
            type="submit"
            className={c.options.length ? "" : "primary"}
            disabled={busy || !text.trim()}
          >
            送出
          </button>
        </form>
      ) : null}
      <p className="decidemeta">
        {c.kind ? <span>{KIND_LABEL[c.kind]}</span> : <span>舊交接（沒有結構化提問）</span>}
        {/* 收不回來的已在問題下方用紅帶標出，這裡不重複 */}
        {c.reversible === true ? <span>可逆</span> : null}
        <span>{"放了 " + ageText(c)}</span>
        <span>{"交接 " + c.handoff_at}</span>
        {c.notes.length ? <span>{c.notes.join("；")}</span> : null}
      </p>
      {c.evidence.length ? (
        <p className="decideev">
          <span className="muted small">證據：</span>
          {c.evidence.map((e) =>
            e.exists && e.repo && e.rel ? (
              <button
                key={e.ref}
                type="button"
                className="linkish small"
                onClick={() => ui.openFile(e.repo as string, e.rel as string)}
              >
                {e.ref}
              </button>
            ) : (
              <span key={e.ref} className="small faint" title="這個檔不存在">
                {e.ref + "（不存在）"}
              </span>
            ),
          )}
        </p>
      ) : null}
      {c.advice ? (
        <p className="decideadvice small">
          {(c.advice.from.kind === "steward" ? "總管" : c.advice.from.repo + " 的 agent") +
            "建議：" +
            (c.advice.option ? c.advice.option + ") " : "") +
            c.advice.text +
            (c.advice.why ? "（" + c.advice.why + "）" : "")}
        </p>
      ) : null}
      <details className="decidefull">
        <summary className="small">看交接全文</summary>
        <dl className="small">
          {c.done ? (
            <>
              <dt>完成</dt>
              <dd>{c.done}</dd>
            </>
          ) : null}
          {c.remaining ? (
            <>
              <dt>剩下</dt>
              <dd>{c.remaining}</dd>
            </>
          ) : null}
          <dt>下次從這裡接</dt>
          <dd>{c.next_step || "（沒寫）"}</dd>
        </dl>
      </details>
      <p className="error" role="alert" hidden={!error}>
        {error}
      </p>
    </article>
  );
}

/** 「要你決定」區：沒有卡（也沒有剛回答完的）就整區不出現。 */
export function DecidePane({ active }: { active: boolean }) {
  const ui = useUi();
  const store = useStore();
  const { decide, decideError, state } = useSnapshot();
  const cards = useMemo(() => decide?.cards ?? [], [decide]);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const [marked, setMarked] = useState<number | null>(null);
  const [answered, setAnswered] = useState<Decided | null>(null);
  const focusIdx = Math.max(
    0,
    cards.findIndex((c) => cardKey(c) === focusKey),
  );
  const focus = cards[focusIdx] ?? null;
  const move = useCallback(
    (delta: number) => {
      if (!cards.length) return;
      const i = (focusIdx + delta + cards.length) % cards.length;
      setFocusKey(cardKey(cards[i]));
      setMarked(null);
    },
    [cards, focusIdx],
  );
  useEffect(() => {
    if (!active) return;
    const on = (e: KeyboardEvent) => {
      if (keysBelongElsewhere(e.target) || e.metaKey || e.ctrlKey || e.altKey) return;
      if (imeKey(e)) return;
      if (answered) return;
      if (!focus) return;
      if (/^[1-9]$/.test(e.key)) {
        const n = Number(e.key);
        if (n <= focus.options.length) {
          setMarked(n);
          e.preventDefault();
        }
      } else if (e.key === "Enter" && marked !== null) {
        // 焦點在按鈕上時 Enter 是按那個按鈕，不攔
        if ((e.target as HTMLElement)?.tagName?.toLowerCase() === "button") return;
        e.preventDefault();
        window.dispatchEvent(new CustomEvent("decide:send", { detail: marked }));
        setMarked(null);
      } else if (e.key === "Escape" && marked !== null) {
        setMarked(null);
      } else if (e.key === "ArrowDown" || e.key === "j" || e.key === "J") {
        e.preventDefault();
        move(1);
      } else if (e.key === "ArrowUp" || e.key === "k" || e.key === "K") {
        e.preventDefault();
        move(-1);
      }
    };
    window.addEventListener("keydown", on);
    return () => window.removeEventListener("keydown", on);
  }, [active, answered, focus, marked, move]);
  const autoN = state.decide?.auto_recent ?? 0;
  if (!cards.length && !answered) {
    // 無事不報；但抓不到資料要講（保留上次的卡時不會到這裡）
    return decideError && !decide ? (
      <p className="error small" role="alert">
        {"要你決定的卡讀不到：" + decideError}
      </p>
    ) : null;
  }
  const rest = cards.filter((c) => !focus || cardKey(c) !== cardKey(focus));
  return (
    <section id="decide" className="decide" aria-label="要你決定">
      <div className="decidehead">
        <h2>{"要你決定 · " + cards.length}</h2>
        <span className="small muted">1–9 標記、Enter 送出、↑↓ 換卡</span>
        <span className="spacer" />
        <button
          type="button"
          className="linkish small"
          title="你最近的回答（可撤回）、最近的自動放行（可推翻）、放行規則"
          onClick={() => ui.openDetail(autoListSpec())}
        >
          {autoN > 0 ? "回答與放行 · 自動 " + autoN : "回答與放行"}
        </button>
      </div>
      {decideError ? (
        <p className="error small" role="alert">
          {"更新失敗，顯示的是上次的卡：" + decideError}
        </p>
      ) : null}
      {answered ? (
        <AnsweredStep
          d={answered}
          onDone={() => {
            setAnswered(null);
            setMarked(null);
          }}
          onWithdrawn={async () => {
            const back = answered.repo + "\u0000" + (answered.handoff_ref ?? "");
            store.toast(withdrawnText(answered.repo, false));
            setAnswered(null);
            setMarked(null);
            setFocusKey(back);
            await store.refreshDecide();
            void store.refreshState();
          }}
        />
      ) : focus ? (
        <FocusCard
          key={cardKey(focus)}
          c={focus}
          marked={marked}
          onAnswered={(d) => {
            setAnswered(d);
            setMarked(null);
            // 處理完自動聚焦下一張
            const next = rest[0];
            setFocusKey(next ? cardKey(next) : null);
          }}
        />
      ) : null}
      {rest.length ? (
        <ul className="decidelist" aria-label="其他要你決定的">
          {(answered
            ? cards.filter(
                (c) => !(c.repo === answered.repo && c.handoff_ref === answered.handoff_ref),
              )
            : rest
          ).map((c) => (
            <li key={cardKey(c)}>
              <button
                type="button"
                className="ghost"
                onClick={() => {
                  setAnswered(null);
                  setFocusKey(cardKey(c));
                  setMarked(null);
                }}
              >
                <span className="dlrepo">{c.repo}</span>
                <span className="dlq">{c.question}</span>
                <span className="dlage">{ageText(c)}</span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}
