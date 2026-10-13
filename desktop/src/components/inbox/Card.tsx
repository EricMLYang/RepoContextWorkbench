import { memo } from "react";
import type { InboxCard } from "../../api/types";
import { Icon, type IconName } from "../../lib/icons";
import { actionLabel, cardDescription, primaryAction } from "../../lib/view";

const ICON: Partial<Record<InboxCard["kind"], IconName>> = {
  interrupt: "alert",
  question: "alert",
  collision: "check",
  loop: "clock",
  fresh: "file",
  reopen: "alert",
  retro: "chat",
};

/**
 * 收件匣卡：標題＋判定＋歸屬／時間＋一句理由；主要動作＋查看詳情＋更多。
 * 處理中卡只有轉圈沒有動作。
 */
export const Card = memo(function Card(props: {
  card: InboxCard;
  busy: boolean;
  error?: string;
  onAction: (c: InboxCard, primary: NonNullable<ReturnType<typeof primaryAction>>) => void;
  onDetail: (c: InboxCard, more: boolean) => void;
}) {
  const { card: c, busy, error, onAction, onDetail } = props;
  const primary = primaryAction(c);
  const desc = cardDescription(c);
  const verdict = "verdict" in c ? c.verdict : "";
  const due = c.kind === "loop" ? c.due : null;
  const hasMore = (c.actions || []).some((a) => a !== primary);
  return (
    <article className={"card " + c.kind} data-key={c.key}>
      <div className="cardhead">
        <span className="cardicon">
          {c.kind === "pending" ? (
            <span className="spin" />
          ) : (
            <Icon name={ICON[c.kind] || "file"} />
          )}
        </span>
        <div className="cardcopy">
          <h3 className="ttl">{c.title}</h3>
          <div className="meta">
            {verdict ? (
              <span className={"pill " + (verdict === "真增量" ? "green" : "amber")}>
                {verdict}
              </span>
            ) : null}
            <span>{c.scope_label || ("repo" in c ? c.repo : "") || "未分類"}</span>
            {c.time ? <time>{(c.date ? c.date + " " : "") + c.time}</time> : null}
            {due ? <span className="pill amber">{"到期 " + due}</span> : null}
          </div>
          {desc ? <p className="description">{desc}</p> : null}
        </div>
      </div>
      {primary || c.kind !== "pending" || hasMore ? (
        <div className="acts">
          {primary ? (
            <button
              type="button"
              className="primary"
              disabled={busy}
              onClick={() => onAction(c, primary)}
            >
              {actionLabel(primary)}
            </button>
          ) : null}
          {c.kind !== "pending" ? (
            <button
              type="button"
              className="ghost"
              disabled={busy}
              onClick={() => onDetail(c, false)}
            >
              查看詳情
            </button>
          ) : null}
          {hasMore ? (
            <button
              type="button"
              className="ghost more"
              disabled={busy}
              onClick={() => onDetail(c, true)}
            >
              更多 ···
            </button>
          ) : null}
        </div>
      ) : null}
      <p className="error carderror" role="alert" hidden={!error}>
        {error}
      </p>
    </article>
  );
});
