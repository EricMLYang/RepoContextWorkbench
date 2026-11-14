import { useCallback, useState } from "react";
import type { CardAction, InboxCard } from "../../api/types";
import { Icon } from "../../lib/icons";
import { cardSections, emptyState, type CardSection } from "../../lib/view";
import { useSnapshot, useUi } from "../../store/context";
import { Card } from "./Card";
import { cardDetailSpec, useRunAction } from "./cardActions";
import { tidyTodosSpec } from "../todos/TidyTodos";
import { Summary } from "./Summary";
import { DecidePane } from "./DecidePane";
import { load, save } from "../../lib/storage";

/** 每段一次畫幾張卡（N-25）。 */
export const PAGE = 50;

function Section({
  section,
  busy,
  errors,
  onAction,
  onDetail,
}: {
  section: CardSection;
  busy: ReadonlySet<string>;
  errors: Readonly<Record<string, string>>;
  onAction: (c: InboxCard, a: CardAction) => void;
  onDetail: (c: InboxCard) => void;
}) {
  const title = section.title + " · " + section.cards.length;
  // 一段先畫 PAGE 張（spine 長大後一段可能上千則，全畫進 DOM 會卡住 WebView）；其餘按鈕展開
  const [limit, setLimit] = useState(PAGE);
  const shown = section.cards.slice(0, limit);
  const rest = section.cards.length - shown.length;
  const inner = (
    <>
      <p className="lane-note" hidden={!section.note}>
        {section.note || ""}
      </p>
      <div className="sectioncards">
        {shown.map((c) => (
          <Card
            key={c.key}
            card={c}
            busy={busy.has(c.key)}
            error={errors[c.key]}
            onAction={onAction}
            onDetail={onDetail}
          />
        ))}
      </div>
      {rest > 0 ? (
        <button
          type="button"
          className="ghost small showmore"
          onClick={() => setLimit(limit + PAGE)}
        >
          {`再顯示 ${Math.min(rest, PAGE)} 則（還有 ${rest} 則）`}
        </button>
      ) : null}
    </>
  );
  return section.collapsed ? (
    <details className="cardsection">
      <summary className="secttl">
        <h2>{title}</h2>
      </summary>
      {inner}
    </details>
  ) : (
    <section className="cardsection">
      <h2 className="secttl">{title}</h2>
      {inner}
    </section>
  );
}

/** 待處理分頁：工作摘要＋分段卡片＋空狀態＋靜默行。 */
export function InboxPane({
  query,
  filter,
  active,
}: {
  query: string;
  filter: string;
  active: boolean;
}) {
  const snap = useSnapshot();
  const ui = useUi();
  const run = useRunAction();
  const { state, scan, stateReady, stateError, scanReady, scanError, busy, cardErrors } = snap;
  const sections = cardSections(state, scan, query, filter);
  const empty = emptyState({
    stateReady,
    stateError,
    repoCount: state.repos.length,
    scanReady,
    scanError,
    filtered: !!query || filter !== "all",
  });

  const onAction = useCallback((c: InboxCard, a: CardAction) => void run(c, a, "card"), [run]);
  const onDetail = useCallback((c: InboxCard) => ui.openDetail(cardDetailSpec(c)), [ui]);
  // F-127：有要你決定的卡時，其他類型收在「其他（N）」底下（預設摺起來、記住展開與否）；沒有就照舊
  const deciding = !!snap.decide?.cards.length;
  const showEmpty = !deciding && !sections.some((s) => !["global", "unclassified"].includes(s.key));
  const [otherOpen, setOtherOpen] = useState(() => load<boolean>("inbox:otherOpen", false));
  const otherN = sections.reduce((n, s) => n + s.cards.length, 0);
  const list = sections.map((s) => (
    <Section
      key={s.key}
      section={s}
      busy={busy}
      errors={cardErrors}
      onAction={onAction}
      onDetail={onDetail}
    />
  ));
  return (
    <section id="tab-inbox" className={"tabpane" + (active ? " on" : "")} aria-label="收件匣">
      <DecidePane active={active} />
      <Summary />
      <div id="cards">
        {deciding && otherN ? (
          <details
            className="othercards"
            open={otherOpen}
            onToggle={(e) => {
              const open = (e.currentTarget as HTMLDetailsElement).open;
              setOtherOpen(open);
              save("inbox:otherOpen", open);
            }}
          >
            <summary className="secttl">
              <h2>{"其他（" + otherN + "）"}</h2>
            </summary>
            {list}
          </details>
        ) : (
          list
        )}
      </div>
      <div id="allclear" className="emptystate" hidden={!showEmpty}>
        <Icon name="check" />
        <h2>{empty.title}</h2>
        <p>{empty.text}</p>
        {empty.onboard ? (
          <button type="button" className="primary" onClick={ui.openOnboarding}>
            選擇資料夾，掃描 repo
          </button>
        ) : null}
      </div>
      <p id="quietline" className="small muted">
        {scanReady && !scanError
          ? `本次檢查 ${scan.states.length} 個 repo · ${scan.quiet} 個未觸發提醒條件`
          : ""}
      </p>
      {stateReady && state.repos.length ? (
        <button
          type="button"
          id="tidybtn"
          className="linkish"
          title="把目前範圍的未結分成可能已做完／擱置太久／等你決定／還在進行，勾了一次關掉"
          onClick={() => ui.openDetail(tidyTodosSpec())}
        >
          清理未結…
        </button>
      ) : null}
    </section>
  );
}
