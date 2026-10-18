/**
 * 卡片按鈕的出口：每個按鈕＝一個後端 action（term_create／route_pick／collide_prefill
 * 與「暫停要先填復工條件」在前端開表單，其餘直接呼叫）。
 */
import { useCallback, useState } from "react";
import type { ActionName, CardAction, InboxCard } from "../../api/types";
import { DetailLayout, useAutoFocus, useDetail } from "../../dialogs/Modal";
import { copyText } from "../../lib/clipboard";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { KINDNAME, actionLabel, actionToast, cardDetailText, primaryAction } from "../../lib/view";

export type RunAction = (c: InboxCard, a: CardAction, from: "card" | "dialog") => Promise<void>;

export function useRunAction(): RunAction {
  const store = useStore();
  const ui = useUi();
  const run: RunAction = useCallback(
    async (c, a, from) => {
      if (store.getSnapshot().busy.has(c.key)) return;
      if (a.action === "term_create") {
        ui.openSession({
          ...a.payload,
          ...(c.scope_kind === "global" ? { group: null, repos: null } : {}),
        });
        return;
      }
      if (a.action === "session_resume") {
        // F-77：接回那場會話（引擎檢查目錄在登記的 repo 底下），開好就切過去
        store.setBusy(c.key, true);
        store.setCardError(c.key, "");
        try {
          const r = await store.action("term_create", { kind: "agent", ...a.payload });
          ui.closeDetail();
          await store.refreshState();
          ui.selectTerm(r.sid);
        } catch (e) {
          store.setCardError(c.key, (e as Error).message);
        } finally {
          store.setBusy(c.key, false);
        }
        return;
      }
      if (a.action === "brief_session") {
        // 任務包（Rust 引擎 F-42）：先請引擎組好 markdown，再用它當新會話的任務
        store.setBusy(c.key, true);
        store.setCardError(c.key, "");
        try {
          const r = await store.action("todo_brief", { num: String(a.payload.num ?? "") });
          ui.closeDetail();
          ui.openSession({
            ...(c.scope_kind === "group"
              ? { group: c.scope_label, repos: null }
              : { repos: c.scope_repos }),
            task: r.markdown,
            origin: "loop:" + String(a.payload.num ?? ""),
          });
        } catch (e) {
          const msg = (e as Error).message;
          if (from === "card") store.setCardError(c.key, msg);
          else ui.setDetailError(msg);
        } finally {
          store.setBusy(c.key, false);
        }
        return;
      }
      if (a.action === "route_pick") {
        ui.openDetail({
          title: "選擇儲存位置",
          eyebrow: "分析結果",
          content: <DestinationForm card={c} run={run} />,
        });
        return;
      }
      if (a.action === "collide_prefill") {
        const text = String(a.payload.text ?? "");
        if (ui.getDraft().trim() && ui.getDraft() !== text) {
          ui.openDetail({ title: "保留現有草稿？", content: <PrefillConfirm text={text} /> });
        } else {
          ui.setDraft(text);
          ui.closeDetail();
          ui.showComposer();
        }
        return;
      }
      if (a.action === "tier" && a.payload?.tier === "paused" && !a.payload.resume_when) {
        ui.openDetail({
          title: "設定復工條件",
          content: <PauseForm card={c} action={a} run={run} />,
        });
        return;
      }
      store.setBusy(c.key, true);
      store.setCardError(c.key, "");
      ui.setDetailError("");
      try {
        const r = (await store.action(
          a.action as ActionName,
          (a.payload || {}) as never,
        )) as Record<string, unknown>;
        ui.closeDetail();
        store.toast(actionToast(a, r));
        await store.refreshState();
        if (r.rescan) await store.rescan();
      } catch (e) {
        const msg = (e as Error).message;
        if (from === "card") store.setCardError(c.key, msg);
        else ui.setDetailError(msg);
      } finally {
        store.setBusy(c.key, false);
      }
    },
    [store, ui],
  );
  return run;
}

/** 查看詳情／更多：完整內容＋全部動作（主要動作標示）。 */
export function CardDetail({ card }: { card: InboxCard }) {
  const store = useStore();
  const { busy } = useSnapshot();
  const { setError } = useDetail();
  const run = useRunAction();
  const text = cardDetailText(card);
  const primary = primaryAction(card);
  const locked = busy.has(card.key);
  return (
    <DetailLayout
      actions={(card.actions || []).map((a, i) => (
        <button
          key={i}
          type="button"
          className={a === primary ? "primary" : a.action === "ignore" ? "ghost" : ""}
          disabled={locked}
          onClick={() => void run(card, a, "dialog")}
        >
          {actionLabel(a)}
        </button>
      ))}
    >
      <p className="detailmeta">
        {`${card.scope_label || ("repo" in card ? card.repo : "") || "未分類"} · ${card.date || ""} ${card.time || ""}`}
      </p>
      <pre>{text || "選擇下方動作，繼續處理這件事。"}</pre>
      <button
        type="button"
        className="ghost small"
        onClick={async () => {
          try {
            await copyText(store.backend, card.title + "\n" + text);
            store.toast("已複製完整內容與來源");
          } catch {
            setError("請直接選取上方內容複製。");
          }
        }}
      >
        複製完整內容
      </button>
    </DetailLayout>
  );
}

export function cardDetailSpec(card: InboxCard) {
  return { title: card.title, eyebrow: KINDNAME[card.kind], content: <CardDetail card={card} /> };
}

function DestinationForm({ card, run }: { card: InboxCard; run: RunAction }) {
  const { state, busy } = useSnapshot();
  const [dest, setDest] = useState("incubator");
  const cid = "cid" in card ? card.cid : "";
  return (
    <DetailLayout
      actions={
        <button
          type="button"
          className="primary"
          disabled={busy.has(card.key)}
          onClick={() =>
            void run(
              card,
              { label: "儲存", action: "route_collision", payload: { cid, dest } },
              "dialog",
            )
          }
        >
          儲存
        </button>
      }
    >
      <label className="field">
        存入
        <select value={dest} onChange={(e) => setDest(e.target.value)}>
          <option value="incubator">孵化區</option>
          {state.groups.map((g) => (
            <option key={"g" + g.name} value={"group:" + g.name}>
              {"牌組 · " + g.name}
            </option>
          ))}
          {state.repos.map((r) => (
            <option key={"r" + r} value={"repo:" + r}>
              {"Repo · " + r}
            </option>
          ))}
        </select>
      </label>
    </DetailLayout>
  );
}

function PauseForm({ card, action, run }: { card: InboxCard; action: CardAction; run: RunAction }) {
  const { busy } = useSnapshot();
  const { setError } = useDetail();
  const ref = useAutoFocus<HTMLInputElement>();
  const [value, setValue] = useState("");
  return (
    <DetailLayout
      actions={
        <button
          type="button"
          className="primary"
          disabled={busy.has(card.key)}
          onClick={() => {
            if (!value.trim()) {
              setError("請填寫復工條件。");
              return;
            }
            void run(
              card,
              { ...action, payload: { ...action.payload, resume_when: value.trim() } },
              "dialog",
            );
          }}
        >
          暫停 repo
        </button>
      }
    >
      <label className="field">
        什麼條件下恢復工作？
        <input ref={ref} value={value} onChange={(e) => setValue(e.target.value)} />
      </label>
    </DetailLayout>
  );
}

function PrefillConfirm({ text }: { text: string }) {
  const ui = useUi();
  return (
    <DetailLayout
      actions={
        <button
          type="button"
          className="primary"
          onClick={() => {
            ui.setDraft(ui.getDraft() + "\n\n" + text);
            ui.closeDetail();
            ui.showComposer();
          }}
        >
          附加問題
        </button>
      }
    >
      <p className="dialogmessage">輸入框已有草稿。可將這個問題附加在草稿後，再一起分析。</p>
    </DetailLayout>
  );
}
