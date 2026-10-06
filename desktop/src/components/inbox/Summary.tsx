import { Fragment, useState } from "react";
import type { Fact, WorkSummary } from "../../api/types";
import { AsyncButton } from "../AsyncButton";
import { DetailLayout, useAutoFocus, useDetail } from "../../dialogs/Modal";
import { useSnapshot, useStore, useUi } from "../../store/context";

function SumItem({ label, item, fallback }: { label: string; item: Fact | null; fallback?: string | null }) {
  return (
    <div className="sumrow">
      <div className="sumlabel">{label}</div>
      <div className="sumcopy">
        {item ? (
          <>
            <p className="sumtext">{item.text}</p>
            <p className="sumsrc">{"依據：" + item.source + (item.at ? " · " + item.at : "")}</p>
          </>
        ) : (
          <p className="sumtext missing">{fallback || "目前沒有可引用的依據。"}</p>
        )}
      </div>
    </div>
  );
}

function SumChanges({ s }: { s: WorkSummary }) {
  return (
    <div className="sumrow">
      <div className="sumlabel">變化</div>
      <div className="sumcopy">
        <p className="sumsrc">{"範圍：" + (s.changes_window || "近期")}</p>
        {s.changes?.length ? (
          s.changes.slice(0, 5).map((c, i) => (
            <Fragment key={i}>
              <p className="sumtext">{c.text}</p>
              <p className="sumsrc">{"依據：" + c.source + (c.at ? " · " + c.at : "")}</p>
            </Fragment>
          ))
        ) : (
          <p className="sumtext missing">{s.changes_gap || "本次採集沒有看到變化。"}</p>
        )}
      </div>
    </div>
  );
}

/** 本組工作摘要：目標／上次進度／變化／可接續的一步——每格帶依據，沒有依據就直說。 */
export function Summary() {
  const { state, scan, scanReady, scanError, scanAt, stateReady } = useSnapshot();
  const ui = useUi();
  const s = scan.summary;
  let body;
  if (!stateReady || !state.repos.length) {
    body = <p className="small muted">登記 repo 後，這裡會顯示這組在做什麼、上次做到哪裡。</p>;
  } else if (!s || !scanReady || scanError) {
    body = (
      <p className="small muted">
        {scanError ? "工作摘要需要一次成功的更新才顯示；目前無法確認現況。" : "正在整理本組工作摘要…"}
      </p>
    );
  } else {
    body = (
      <>
        <div className="sumhead">
          <h2>本組工作摘要</h2>
          <span className="spacer" />
          <span className="small muted">{"採集於 " + (scanAt || s.at)}</span>
        </div>
        <div className="sumbody">
          <SumItem label="目前目標" item={s.goal} fallback={s.goal_gap} />
          <SumItem label="上次進度" item={s.progress} fallback={s.progress_gap} />
          <SumChanges s={s} />
          <SumItem label="可接續的一步" item={s.next} fallback={s.next_gap} />
        </div>
        <div className="sumacts">
          <button
            type="button"
            className="primary"
            onClick={() =>
              ui.openSession({ task: s.resume_task, origin: "resume:" + (s.saved_group || s.group) })
            }
          >
            接續上次工作
          </button>
          {s.next && s.next.text.startsWith("接續 #") ? (
            <button type="button" className="ghost small" onClick={() => ui.showTab("inbox")}>
              查看這件未結
            </button>
          ) : null}
          <button
            type="button"
            className="ghost"
            onClick={() =>
              ui.openDetail({
                title: s.saved_group ? "這組在做什麼？" : "目標需要先有牌組",
                eyebrow: "GROUP GOAL",
                content: <GoalForm s={s} />,
              })
            }
          >
            {s.goal ? "修改目標" : "寫下這組的目標"}
          </button>
          <span className="small muted">
            摘要只引用登記的目標與脊椎留痕；沒有依據的欄位會直說沒有。
          </span>
        </div>
      </>
    );
  }
  return (
    <section id="summary" className="summary" aria-label="本組工作摘要">
      {body}
    </section>
  );
}

function GoalForm({ s }: { s: WorkSummary }) {
  const store = useStore();
  const { close } = useDetail();
  const saved = s.saved_group;
  const ref = useAutoFocus<HTMLInputElement>();
  const [text, setText] = useState(s.goal?.text || "");
  if (!saved)
    return (
      <DetailLayout
        actions={
          <button type="button" className="primary" onClick={close}>
            知道了
          </button>
        }
      >
        <p className="dialogmessage">
          {(s.goal_gap || "") + "\n\n用左欄的「＋」把目前範圍存成牌組，就能記下目標。"}
        </p>
      </DetailLayout>
    );
  return (
    <DetailLayout
      actions={
        <AsyncButton
          className="primary"
          run={async () => {
            await store.action("set_goal", { group: saved, text: text.trim() });
            close();
            store.toast("目標已儲存");
            await store.refreshState();
            await store.rescan();
          }}
        >
          儲存目標
        </AsyncButton>
      }
    >
      <p className="detailmeta">
        {"牌組：" + saved + "　目標只有你能寫；工作台不會用 commit 數替你推測進度。"}
      </p>
      <label className="field">
        一句話：這組現在要達成什麼？
        <input ref={ref} maxLength={300} value={text} onChange={(e) => setText(e.target.value)} />
      </label>
    </DetailLayout>
  );
}
