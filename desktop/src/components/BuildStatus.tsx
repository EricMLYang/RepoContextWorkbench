import { useEffect, useState } from "react";
import type { BuildCommit, BuildInfo, BuildTest, InstallStatus } from "../api/types";
import { DetailLayout, useAutoFocus, useDetail } from "../dialogs/Modal";
import { load, save } from "../lib/storage";
import { useSnapshot, useStore, useUi } from "../store/context";
import { AsyncButton } from "./AsyncButton";

/** 上次看過「已更新」的那版 commit（這台電腦）。 */
const SEEN_KEY = "build:seen";
/** 背景更新在 build／安裝時，多久再問一次進度（等你 ⌘Q 的時候不問：在那之前不會變）。 */
const INSTALL_POLL_MS = 5000;
/** 更新失敗的提示留多久（分鐘）。 */
const FAILED_SHOW_MIN = 24 * 60;
/** 背景更新只有 macOS 有（install-app.sh）。 */
const CAN_INSTALL = !/win/i.test(navigator.platform);

/** 背景更新（F-118）現在要不要在狀態列講、講什麼；不用講＝null。 */
export function installText(st: InstallStatus | null | undefined): string | null {
  if (!st) return null;
  switch (st.phase) {
    case "building":
      return "新版 build 中…";
    case "waiting":
      return "新版好了：⌘Q 後自動換好重開";
    case "installing":
      return "新版安裝中…";
    case "failed":
      return !st.current && (st.age_min ?? 0) < FAILED_SHOW_MIN ? "更新失敗" : null;
    default:
      return null;
  }
}

/**
 * 狀態列的版本（F-114）：平常只寫「a37c4b5」；裝了新版第一次打開寫「已更新 · N 項改動」，
 * 原始碼有還沒裝的 commit 寫「N 個改動沒裝」（琥珀）——點了列出 commit，可以直接「背景 build 新版」（F-118）。
 * 重裝後 agent 的 skill 還是舊版就多一顆「更新 agent 的 skill」（ctx setup --claude 最常忘的那一步）。
 * 只在打開、視窗回到前景、背景更新 build／安裝中時問引擎；問不到就留著上次的。
 */
export function BuildStatus() {
  const store = useStore();
  const ui = useUi();
  const [info, setInfo] = useState<BuildInfo | null>(null);
  const [seen, setSeen] = useState<string | null>(() => load<string | null>(SEEN_KEY, null));
  const phase = info?.install?.phase;
  const busy = phase === "building" || phase === "installing";

  useEffect(() => {
    let live = true;
    const ask = () =>
      store
        .action("build_info", seen ? { since: seen } : {})
        .then((r) => {
          if (!live) return;
          setInfo(r);
          // 沒有「更新」可說就直接記下這版：第一次用、退回舊版、上次那版的 commit 已經找不到、原始碼不在這台
          if (r.commit && seen !== r.commit && !(r.since && r.since.count > 0)) {
            save(SEEN_KEY, r.commit);
            setSeen(r.commit);
          }
        })
        .catch(() => {});
    void ask();
    window.addEventListener("focus", ask);
    const timer = busy ? setInterval(ask, INSTALL_POLL_MS) : null;
    return () => {
      live = false;
      window.removeEventListener("focus", ask);
      if (timer) clearInterval(timer);
    };
  }, [store, seen, busy]);

  if (!info?.commit) return null;
  const commit = info.commit;
  const updated = info.since && info.since.count > 0 ? info.since : null;
  const pending = info.pending && info.pending.count > 0 ? info.pending : null;
  const install = installText(info.install);
  const markSeen = () => {
    save(SEEN_KEY, commit);
    setSeen(commit);
  };
  const startInstall = async () => {
    const r = await store.action("install_update", {});
    setInfo((cur) => (cur ? { ...cur, install: r.install } : cur));
    store.toast("開始在背景 build 新版；好了狀態列會說，⌘Q 結束 App 後自動換好重開");
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
                  tests={updated.tests}
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
      {install && info.install ? (
        <button
          type="button"
          id="buildinstall"
          className={"statusbtn" + (info.install.phase === "failed" ? " statusbad" : "")}
          title={"背景更新（" + info.install.at + " 開始）；輸出在 " + info.install.log}
          onClick={() =>
            ui.openDetail({
              title: install,
              eyebrow: "背景更新",
              content: <InstallDetail st={info.install!} />,
            })
          }
        >
          {install}
        </button>
      ) : pending ? (
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
                    "）比這個 App 新。" +
                    (CAN_INSTALL
                      ? "按「背景 build 新版」：App 開著照用，build 好了狀態列會說，你 ⌘Q 結束 App 後自動換好並重開" +
                        "（⌘Q 會結束 App 裡的會話，所以由你決定什麼時候按）。也可以自己結束 App 後在終端跑 bash scripts/install-app.sh："
                      : "先結束 App，在終端跑 bash scripts/install-app.sh，再打開 App：")
                  }
                  commits={pending.commits}
                  more={pending.count - pending.commits.length}
                  done="關閉"
                  extra={CAN_INSTALL ? startInstall : undefined}
                />
              ),
            })
          }
        >
          {pending.count + " 個改動沒裝"}
        </button>
      ) : null}
      {info.skill === "stale" ? (
        <AsyncButton
          id="skillstale"
          className="statusbtn usagewarn"
          title="重裝 App 之後，agent 讀的 repo-context skill 還是舊版（等於 ctx setup --claude 沒重跑）：按了換成這版內建的。只寫 ~/.claude/skills/repo-context/SKILL.md；開新的會話才會讀到"
          onError={(m) => store.toast(m)}
          run={async () => {
            const r = await store.action("update_skill", {});
            setInfo((cur) => (cur ? { ...cur, skill: r.skill } : cur));
            store.toast("已更新 agent 的 skill：" + r.message + "（開新的會話才會讀到）");
          }}
        >
          更新 agent 的 skill
        </AsyncButton>
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

function InstallDetail({ st }: { st: InstallStatus }) {
  const { close } = useDetail();
  const ref = useAutoFocus<HTMLButtonElement>();
  const alive = (useSnapshot().state.terms || []).filter((s) => s.alive).length;
  const text =
    st.phase === "building"
      ? "正在背景 build（通常幾分鐘）。App 照常用；build 好了這裡會變成「新版好了」。"
      : st.phase === "waiting"
        ? "新版已經 build 好。你按 ⌘Q 結束 App 後，會自動換成新版並重新打開。" +
          (alive ? "現在有 " + alive + " 個執行中會話，結束 App 會一起結束它們。" : "")
        : st.phase === "installing"
          ? "正在換成新版，好了會自動重新打開。"
          : "背景更新沒有完成：" +
            (st.detail || "（沒有說明）") +
            "。細節看下面的輸出紀錄；換之前的版本備份在 ~/.cache/repo-context/app-backups/。";
  return (
    <DetailLayout
      actions={
        <button ref={ref} type="button" className="primary" onClick={close}>
          知道了
        </button>
      }
    >
      <p className="dialogmessage">{text}</p>
      <p className="muted small">{"輸出紀錄：" + st.log + "（" + st.at + " 開始）"}</p>
    </DetailLayout>
  );
}

function CommitList(props: {
  lead: string;
  commits: BuildCommit[];
  more: number;
  /** 這段期間新增或改過的驗收列（docs/TESTING.md）。 */
  tests?: BuildTest[];
  done: string;
  onDone?: () => void;
  /** 「背景 build 新版」（F-118）；沒給就不出現。 */
  extra?: () => Promise<void>;
}) {
  const { lead, commits, more, tests, done, onDone, extra } = props;
  const { close } = useDetail();
  const ref = useAutoFocus<HTMLButtonElement>();
  return (
    <DetailLayout
      actions={
        <>
          {extra ? (
            <AsyncButton
              className="primary"
              run={async () => {
                await extra();
                close();
              }}
            >
              背景 build 新版
            </AsyncButton>
          ) : null}
          <button
            ref={ref}
            type="button"
            className={extra ? "ghost" : "primary"}
            onClick={() => {
              onDone?.();
              close();
            }}
          >
            {done}
          </button>
        </>
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
      {tests?.length ? (
        <>
          <h3 className="buildtestshead">
            {"可以驗收的（docs/TESTING.md，" + tests.length + " 項）"}
          </h3>
          <ul className="buildcommits">
            {tests.map((t) => (
              <li key={t.id}>
                <code>{t.id}</code> {t.step}
                <div className="muted small">{"預期：" + t.expect}</div>
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </DetailLayout>
  );
}
