import { useEffect, useState } from "react";
import type { BuildCommit, BuildInfo } from "../api/types";
import { DetailLayout, useAutoFocus, useDetail } from "../dialogs/Modal";
import { load, save } from "../lib/storage";
import { useStore, useUi } from "../store/context";

/** 上次看過「已更新」的那版 commit（這台電腦）。 */
const SEEN_KEY = "build:seen";

/**
 * 狀態列的版本（F-114）：平常只寫「a37c4b5」；裝了新版第一次打開寫「已更新 · N 項改動」，
 * 原始碼有還沒裝的 commit 寫「N 個改動沒裝」（琥珀）——點了列出 commit。
 * 只在打開與視窗回到前景時問引擎（不輪詢）；問不到就留著上次的。
 */
export function BuildStatus() {
  const store = useStore();
  const ui = useUi();
  const [info, setInfo] = useState<BuildInfo | null>(null);
  const [seen, setSeen] = useState<string | null>(() => load<string | null>(SEEN_KEY, null));

  useEffect(() => {
    let live = true;
    const ask = () =>
      store
        .action("build_info", seen ? { since: seen } : {})
        .then((r) => {
          if (!live) return;
          setInfo(r);
          // 第一次用：記下這版就好，沒有「更新」可說
          if (!seen && r.commit) {
            save(SEEN_KEY, r.commit);
            setSeen(r.commit);
          }
        })
        .catch(() => {});
    void ask();
    window.addEventListener("focus", ask);
    return () => {
      live = false;
      window.removeEventListener("focus", ask);
    };
  }, [store, seen]);

  if (!info?.commit) return null;
  const commit = info.commit;
  const updated = info.since && info.since.count > 0 ? info.since : null;
  const pending = info.pending && info.pending.count > 0 ? info.pending : null;
  const markSeen = () => {
    save(SEEN_KEY, commit);
    setSeen(commit);
  };
  return (
    <>
      {updated ? (
        <button
          type="button"
          id="buildupdated"
          className="statusbtn"
          title={
            "這版比上次打開的 " + updated.from + " 多了 " + updated.count + " 個改動；點了看清單"
          }
          onClick={() =>
            ui.openDetail({
              title: "已更新到 " + commit,
              eyebrow: "這版多了什麼",
              content: (
                <CommitList
                  lead={
                    "上次打開的是 " +
                    updated.from +
                    "，這版（" +
                    commit +
                    (info.date ? "，" + info.date : "") +
                    "）多了 " +
                    updated.count +
                    " 個改動："
                  }
                  commits={updated.commits}
                  more={updated.count - updated.commits.length}
                  onDone={markSeen}
                  done="知道了"
                />
              ),
            })
          }
        >
          {"已更新 · " + updated.count + " 項改動"}
        </button>
      ) : null}
      {pending ? (
        <button
          type="button"
          id="buildpending"
          className="statusbtn usagewarn"
          title={
            "原始碼已經有、這個 App 還沒有的 commit：" + pending.count + " 個；點了看清單與怎麼裝"
          }
          onClick={() =>
            ui.openDetail({
              title: pending.count + " 個改動還沒裝",
              eyebrow: "現在跑的是 " + commit,
              content: (
                <CommitList
                  lead={
                    "原始碼（" +
                    (info.source ?? "") +
                    "）比這個 App 新。要用到它們，先結束 App（⌘Q；會結束 App 裡的會話），在終端跑 " +
                    "bash scripts/install-app.sh，再打開 App："
                  }
                  commits={pending.commits}
                  more={pending.count - pending.commits.length}
                  done="關閉"
                />
              ),
            })
          }
        >
          {pending.count + " 個改動沒裝"}
        </button>
      ) : null}
      <span
        id="buildver"
        className="muted"
        title={
          "現在跑的這版：" +
          commit +
          (info.date ? "（" + info.date + " 的 commit）" : "") +
          (info.source && !pending ? "；原始碼沒有更新的改動" : "")
        }
      >
        {commit}
      </span>
    </>
  );
}

function CommitList(props: {
  lead: string;
  commits: BuildCommit[];
  more: number;
  done: string;
  onDone?: () => void;
}) {
  const { lead, commits, more, done, onDone } = props;
  const { close } = useDetail();
  const ref = useAutoFocus<HTMLButtonElement>();
  return (
    <DetailLayout
      actions={
        <button
          ref={ref}
          type="button"
          className="primary"
          onClick={() => {
            onDone?.();
            close();
          }}
        >
          {done}
        </button>
      }
    >
      <p className="dialogmessage">{lead}</p>
      <ul className="buildcommits">
        {commits.map((c) => (
          <li key={c.sha}>
            <code>{c.sha}</code> <span className="muted small">{c.date}</span>
            <div>{c.subject}</div>
          </li>
        ))}
      </ul>
      {more > 0 ? <p className="muted small">{"另 " + more + " 個沒列出"}</p> : null}
    </DetailLayout>
  );
}
