import { useEffect, useState } from "react";
import type { FileContent } from "../../api/types";
import { Markdown, type MdHooks } from "../../lib/markdown";
import { parentDir, useRepoChanged, useWatchDirs } from "../../lib/treeWatch";
import { useStore, useUi } from "../../store/context";

export interface OpenFile {
  repo: string;
  path: string;
}

export const fileKey = (f: OpenFile) => f.repo + "\u0000" + f.path;

/** 一次最多畫幾行（再多按「顯示全部」）——上萬行的檔一次塞進 DOM 會卡。 */
const LINE_CAP = 4000;

function fmtSize(n: number) {
  if (n < 1024) return n + " B";
  if (n < 1024 * 1024) return (n / 1024).toFixed(1) + " KB";
  return (n / 1024 / 1024).toFixed(1) + " MB";
}

const isMarkdown = (p: string) => /\.(md|markdown|mdx)$/i.test(p);
/** 預設自動換行的（文章類）；程式碼預設不換行、可橫向捲。 */
const isProse = (p: string) => isMarkdown(p) || /\.(txt|rst|adoc|org|log)$/i.test(p);

/**
 * md 裡的相對連結 → repo 內路徑（`/` 開頭＝repo 根目錄）。跳出 repo 回 null。
 * 只做字串正規化；路徑合不合法、檔在不在由引擎 `read_file` 判斷。
 */
export function resolveRel(from: string, href: string): string | null {
  const clean = decodeURIComponent(href.split("#")[0].split("?")[0]);
  if (!clean) return null;
  const baseParts = from.replace(/\\/g, "/").split("/").slice(0, -1);
  const parts = clean.startsWith("/") ? [] : baseParts;
  for (const seg of clean.replace(/^\/+/, "").split("/")) {
    if (!seg || seg === ".") continue;
    if (seg === "..") {
      if (!parts.length) return null;
      parts.pop();
    } else parts.push(seg);
  }
  return parts.join("/");
}

async function copy(store: ReturnType<typeof useStore>, text: string, label: string) {
  try {
    await navigator.clipboard.writeText(text);
    store.toast("已複製" + label + "：" + text);
  } catch {
    store.toast("無法複製到剪貼簿");
  }
}

/** md 裡的相對圖片：向引擎讀（網址圖片不載入）。 */
function MdImage({ repo, path, alt }: { repo: string; path: string; alt: string }) {
  const store = useStore();
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let live = true;
    store
      .action("read_file", { id: repo, path })
      .then((r) => {
        if (!live) return;
        if (r.kind === "image") setSrc("data:" + r.mime + ";base64," + r.base64);
        else setFailed(true);
      })
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, [store, repo, path]);
  if (src) return <img className="mdimg" src={src} alt={alt} />;
  return (
    <span className="mdimgalt">
      {failed ? "［圖片讀不到：" + (alt || path) + "］" : "［圖片：" + (alt || path) + "］"}
    </span>
  );
}

/** 文字檔：行號＋等寬字；可切換自動換行。 */
function CodeView({ text, wrap }: { text: string; wrap: boolean }) {
  const [all, setAll] = useState(false);
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  const shown = all ? lines : lines.slice(0, LINE_CAP);
  const width = String(lines.length).length;
  return (
    <>
      <div
        className={"codeview" + (wrap ? " wrap" : "")}
        style={{ ["--gutter" as string]: width + 1 + "ch" }}
      >
        {shown.map((l, i) => (
          <div className="cl" key={i}>
            <span className="ln" aria-hidden="true">
              {i + 1}
            </span>
            <span className="lc">{l || " "}</span>
          </div>
        ))}
      </div>
      {!all && lines.length > LINE_CAP ? (
        <button type="button" className="small showall" onClick={() => setAll(true)}>
          顯示全部 {lines.length} 行
        </button>
      ) : null}
    </>
  );
}

/** 唯讀檢視器（一個分頁一個檔）：md 預設預覽、可切原始碼；程式碼有行號；圖片直接顯示。 */
export function FileViewer({ file, hidden }: { file: OpenFile; hidden: boolean }) {
  const store = useStore();
  const ui = useUi();
  const { repo, path } = file;
  const [content, setContent] = useState<FileContent | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [nonce, setNonce] = useState(0);
  const [source, setSource] = useState(false);
  const [wrap, setWrap] = useState(isProse(path));

  // 讀檔（換檔或按 ⟳ 重讀）；讀失敗保留上次的內容並顯示錯誤
  useEffect(() => {
    let live = true;
    store
      .action("read_file", { id: repo, path })
      .then((r) => {
        if (!live) return;
        setContent(r);
        setError("");
      })
      .catch((e: Error) => live && setError(e.message))
      .finally(() => live && setLoading(false));
    return () => {
      live = false;
    };
  }, [store, repo, path, nonce]);
  const reload = () => {
    setLoading(true);
    setError("");
    setNonce((n) => n + 1);
  };
  // 檔案在磁碟上被改了（agent 在寫、git 切分支…）就自己重讀
  useWatchDirs(store.backend, repo, [parentDir(path)]);
  useRepoChanged(store.backend, repo, (paths) => {
    if (paths.includes(path)) reload();
  });

  const hooks: MdHooks = {
    onLink(href) {
      if (href.startsWith("#")) return;
      if (/^[a-z][a-z0-9+.-]*:/i.test(href)) {
        void copy(store, href, "連結（App 內不開外部網頁）");
        return;
      }
      const target = resolveRel(path, href);
      if (target) ui.openFile(repo, target);
      else store.toast("連結指到 repo 外面：" + href);
    },
    image(src, alt, key) {
      if (/^[a-z][a-z0-9+.-]*:/i.test(src))
        return (
          <span key={key} className="mdimgalt">
            ［外部圖片：{alt || src}］
          </span>
        );
      const target = resolveRel(path, src);
      return target ? <MdImage key={key} repo={repo} path={target} alt={alt} /> : null;
    },
  };

  const md = isMarkdown(path);
  const crumbs = path.replace(/\\/g, "/").split("/");
  return (
    <section className="viewer" hidden={hidden} aria-label={"檢視：" + repo + "/" + path}>
      <div className="viewerhead">
        <div className="crumbs" title={content?.abs || path}>
          <span className="muted">{repo}</span>
          {crumbs.map((c, i) => (
            <span key={i} className={i === crumbs.length - 1 ? "crumb last" : "crumb"}>
              <span className="muted" aria-hidden="true">
                {" "}
                ›{" "}
              </span>
              {c}
            </span>
          ))}
          {content ? <span className="muted small"> · {fmtSize(content.size)}</span> : null}
        </div>
        <div className="vieweractions">
          {md && content?.kind === "text" ? (
            <button
              type="button"
              className="small ghost"
              aria-pressed={source}
              onClick={() => setSource(!source)}
            >
              {source ? "預覽" : "原始碼"}
            </button>
          ) : null}
          {content?.kind === "text" && (!md || source) ? (
            <button
              type="button"
              className="small ghost"
              aria-pressed={wrap}
              onClick={() => setWrap(!wrap)}
            >
              換行
            </button>
          ) : null}
          <button
            type="button"
            className="small ghost"
            onClick={() => void copy(store, path, "相對路徑")}
          >
            相對路徑
          </button>
          <button
            type="button"
            className="small ghost"
            disabled={!content}
            onClick={() => content && void copy(store, content.abs, "絕對路徑")}
          >
            絕對路徑
          </button>
          <button
            type="button"
            className="small ghost"
            aria-label="重新讀取"
            title="重新讀取"
            disabled={loading}
            onClick={reload}
          >
            ⟳
          </button>
        </div>
      </div>
      {error ? (
        <p className="error viewererror" role="alert">
          {error}
        </p>
      ) : null}
      <div className="viewerbody">
        {!content ? (
          loading ? (
            <p className="muted">讀取中…</p>
          ) : null
        ) : content.kind === "text" ? (
          <>
            {content.truncated ? (
              <p className="viewernote">檔案有 {fmtSize(content.size)}，只顯示前 1 MB。</p>
            ) : null}
            {md && !source ? (
              <Markdown text={content.text} hooks={hooks} />
            ) : (
              <CodeView text={content.text} wrap={wrap} />
            )}
          </>
        ) : content.kind === "image" ? (
          <div className="viewerimage">
            <img src={"data:" + content.mime + ";base64," + content.base64} alt={path} />
          </div>
        ) : (
          <p className="viewernote">
            {content.kind === "too_large"
              ? "圖片太大（" + fmtSize(content.size) + "），不在 App 裡顯示。"
              : "這是二進位檔（" + fmtSize(content.size) + "），沒有可以顯示的文字。"}
          </p>
        )}
      </div>
    </section>
  );
}
