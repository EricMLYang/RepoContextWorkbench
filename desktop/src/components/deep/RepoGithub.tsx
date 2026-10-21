import { useCallback, useState } from "react";
import type { GithubItem } from "../../api/types";
import { openUrl } from "../../lib/clipboard";
import { useStore } from "../../store/context";
import { commitWhen } from "./RepoCommits";

const REVIEW: Record<string, string> = {
  APPROVED: "已核准",
  CHANGES_REQUESTED: "要求修改",
  REVIEW_REQUIRED: "待審查",
};

/** 一筆的副標：作者、分支、草稿、審查、標籤、多久前。 */
export function githubMeta(x: GithubItem): string {
  return [
    x.author,
    x.branch,
    x.draft ? "草稿" : null,
    x.review ? (REVIEW[x.review] ?? x.review) : null,
    ...x.labels,
    x.updated ? commitWhen(x.updated) : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * GitHub 上開著的 PR 與 issue（F-112）：用使用者自己的 gh（已登入的帳號），展開才讀（要連網）。
 * 點一筆用系統瀏覽器開。
 */
export function RepoGithub({ id }: { id: string }) {
  const store = useStore();
  const [data, setData] = useState<{ prs: GithubItem[]; issues: GithubItem[] } | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const load = useCallback(async () => {
    setBusy(true);
    try {
      const r = await store.action("repo_github", { id });
      setData(r);
      setError("");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }, [store, id]);
  const section = (label: string, list: GithubItem[]) => (
    <>
      <p className="eyebrow">
        {label}（{list.length}）
      </p>
      {!list.length ? <p className="muted small">沒有開著的。</p> : null}
      <ul className="ghlist">
        {list.map((x) => (
          <li key={x.number}>
            <button
              type="button"
              className="ghost ghrow"
              title={"用瀏覽器開 " + x.url}
              onClick={() =>
                openUrl(store.backend, x.url).catch((e: Error) => store.toast(e.message))
              }
            >
              <span className="ghtitle">
                #{x.number} {x.title}
              </span>
              <span className="small muted">{githubMeta(x)}</span>
            </button>
          </li>
        ))}
      </ul>
    </>
  );
  return (
    <details
      className="panelgithub"
      onToggle={(e) => {
        if ((e.currentTarget as HTMLDetailsElement).open && !data) void load();
      }}
    >
      <summary className="small">
        GitHub{data ? "（PR " + data.prs.length + "、issue " + data.issues.length + "）" : ""}
      </summary>
      {busy && !data ? <p className="muted small">讀取中…（要連網）</p> : null}
      {error ? <p className="error small">{error}</p> : null}
      {data ? (
        <>
          {section("PR", data.prs)}
          {section("Issue", data.issues)}
          <button type="button" className="ghost small" disabled={busy} onClick={() => void load()}>
            {busy ? "讀取中…" : "重新讀取"}
          </button>
        </>
      ) : null}
    </details>
  );
}
