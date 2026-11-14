/**
 * 「回答、放行與規則」面板（F-127）：你最近的回答（每筆可撤回，按一次——撤回是安全方向）＋
 * 最近的自動放行（每筆可推翻，按兩次）＋放行規則（可降回每次都問）＋policy.yaml 的問題。
 * 不提供新增規則的表單——升級只從回答後的升級建議那一步進來；paths 先不給 UI。
 */
import { useCallback, useEffect, useState } from "react";
import type { AutoList, MyAnswers, RulesReport } from "../../api/types";
import { DetailLayout, useDetail } from "../../dialogs/Modal";
import { useStore } from "../../store/context";
import type { DetailSpec } from "../../store/context";
import { AsyncButton } from "../AsyncButton";
import { KIND_LABEL, withdrawnText } from "../../lib/view";

function OverturnButton({
  reference,
  onDone,
}: {
  reference: string;
  onDone: (msg: string) => void;
}) {
  const store = useStore();
  const [armed, setArmed] = useState(false);
  return (
    <AsyncButton
      className={armed ? "danger small" : "small"}
      run={async () => {
        if (!armed) {
          setArmed(true);
          return;
        }
        const r = await store.action("decide_overturn", { ref: reference });
        onDone(
          r.rule
            ? `已推翻：「${r.rule.repo}／${KIND_LABEL[r.rule.kind]}」降回每次都問` +
                (r.approval.handoff_ref
                  ? "，那張卡回到「要你決定」"
                  : "（agent 可能已照答案做了，要另外跟它說）")
            : r.note || "已推翻",
        );
      }}
    >
      {armed ? "再按一次推翻" : "推翻"}
    </AsyncButton>
  );
}

function AutoPanel() {
  const store = useStore();
  const { close } = useDetail();
  const [mine, setMine] = useState<MyAnswers | null>(null);
  const [auto, setAuto] = useState<AutoList | null>(null);
  const [rules, setRules] = useState<RulesReport | null>(null);
  const [error, setError] = useState("");
  const [note, setNote] = useState("");
  const fetchBoth = useCallback(
    () =>
      Promise.all([
        store.action("decide_mine", {}),
        store.action("decide_auto", {}),
        store.action("decide_rules", {}),
      ]),
    [store],
  );
  const apply = (m: MyAnswers, a: AutoList, r: RulesReport) => {
    setMine(m);
    setAuto(a);
    setRules(r);
    setError("");
  };
  // 失敗保留上次的清單、把錯誤顯示出來
  const load = useCallback(async () => {
    try {
      const [m, a, r] = await fetchBoth();
      apply(m, a, r);
    } catch (e) {
      setError((e as Error).message);
    }
  }, [fetchBoth]);
  useEffect(() => {
    let alive = true;
    fetchBoth()
      .then(([m, a, r]) => alive && apply(m, a, r))
      .catch((e) => alive && setError((e as Error).message));
    return () => {
      alive = false;
    };
  }, [fetchBoth]);
  const after = async (msg: string) => {
    setNote(msg);
    await load();
    await store.refreshState();
  };
  return (
    <DetailLayout
      actions={
        <button type="button" className="primary" onClick={close}>
          關閉
        </button>
      }
    >
      {error ? (
        <p className="error" role="alert">
          {error}
        </p>
      ) : null}
      {note ? (
        <p className="notice small" role="status">
          {note}
        </p>
      ) : null}
      <h3 className="dsect">{"你最近的回答" + (mine ? "（" + mine.since + " 之後）" : "")}</h3>
      {!mine ? (
        <p className="small muted">讀取中…</p>
      ) : !mine.items.length ? (
        <p className="small muted">這段時間你沒有回答過卡。</p>
      ) : (
        <ul className="autolist">
          {mine.items.map((a) => (
            <li key={a.ref}>
              <div>
                <strong>{a.repo}</strong>
                <span className="small muted">{"　" + a.at}</span>
              </div>
              <div>{"問：" + a.question}</div>
              <div>{"答：" + a.answer}</div>
              {a.overturned_by ? (
                <span className="small muted">已撤回</span>
              ) : (
                <AsyncButton
                  className="small"
                  title="這筆回答不算數，卡回到「要你決定」；已經送給 agent 的要另外跟它說"
                  run={async () => {
                    await store.action("decide_overturn", { ref: a.ref });
                    await after(withdrawnText(a.repo, true));
                  }}
                >
                  撤回
                </AsyncButton>
              )}
            </li>
          ))}
        </ul>
      )}
      <h3 className="dsect">{"最近的自動放行" + (auto ? "（" + auto.since + " 之後）" : "")}</h3>
      {!auto ? (
        <p className="small muted">讀取中…</p>
      ) : !auto.items.length ? (
        <p className="small muted">這段時間沒有自動放行。</p>
      ) : (
        <ul className="autolist">
          {auto.items.map((a) => (
            <li key={a.ref}>
              <div>
                <strong>{a.repo}</strong>
                <span className="small muted">{"　" + a.at + "　" + (a.rule || "")}</span>
              </div>
              <div>{"問：" + a.question}</div>
              <div>{"答：" + a.answer}</div>
              {a.overturned_by ? (
                <span className="small muted">已推翻</span>
              ) : (
                <OverturnButton reference={a.ref} onDone={(m) => void after(m)} />
              )}
            </li>
          ))}
        </ul>
      )}
      <h3 className="dsect">放行規則</h3>
      {!rules ? null : !rules.rules.length ? (
        <p className="small muted">沒有規則：每個提問都問你。升級只會從回答後的建議那一步問你。</p>
      ) : (
        <ul className="autolist">
          {rules.rules.map((r) => (
            <li key={r.repo + "/" + r.kind}>
              <div>
                <strong>{r.repo + "／" + KIND_LABEL[r.kind]}</strong>
                <span className="small muted">
                  {"　" + (r.level === "auto" ? "自動放行" : "每次都問") + "　since " + r.since}
                  {r.level === "auto" ? "　放行 " + r.approvals + " 次" : ""}
                  {r.overturned ? "　被推翻 " + r.overturned + " 次" : ""}
                </span>
              </div>
              {r.level === "auto" ? (
                <AsyncButton
                  className="small"
                  run={async () => {
                    await store.action("decide_untrust", { repo: r.repo, kind: r.kind });
                    await after(`「${r.repo}／${KIND_LABEL[r.kind]}」降回每次都問`);
                  }}
                >
                  降回每次都問
                </AsyncButton>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {rules?.problems.length ? (
        <>
          <h3 className="dsect">policy.yaml 的問題（這些條目不生效，照樣問你）</h3>
          <ul className="small">
            {rules.problems.map((p) => (
              <li key={p}>{p}</li>
            ))}
          </ul>
        </>
      ) : null}
    </DetailLayout>
  );
}

export function autoListSpec(): DetailSpec {
  return { title: "回答、放行與規則", eyebrow: "DECISIONS", content: <AutoPanel /> };
}
