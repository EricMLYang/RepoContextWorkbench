import { useEffect, useRef, useState } from "react";
import { scopeLabel } from "../lib/view";
import { useSnapshot, useStore, useUi } from "../store/context";

/**
 * 碰撞台：草稿存本機（重新整理不丟）；成功送出才清草稿，失敗保留並顯示原因；
 * Ctrl/⌘+Enter 送出（選字中不送）。
 */
export function Composer(props: { open: boolean; focusSeq: number; onClose: () => void }) {
  const { open, focusSeq, onClose } = props;
  const store = useStore();
  const ui = useUi();
  const { scope } = useSnapshot();
  const [error, setError] = useState("");
  const [sending, setSending] = useState<"" | "sending" | "sent">("");
  const ref = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (open && focusSeq) ref.current?.focus();
  }, [open, focusSeq]);

  const repos = store.scopeRepos;

  async function submit() {
    if (sending === "sending") return;
    const value = ui.draft.trim();
    const payload = store.scopePayload;
    if (!value) {
      setError("先寫下一個想法，再開始分析。");
      return;
    }
    if (!store.scopeRepos.length) {
      setError("請先選擇至少一個 repo。");
      return;
    }
    setSending("sending");
    setError("");
    try {
      await store.action("collide", { idea: value, ...payload });
      // 送出期間又改了就不清（使用者還在寫）
      if (ref.current ? ref.current.value.trim() === value : true) ui.setDraft("");
      setSending("sent");
      store.toast("想法已送出，分析結果會回到收件匣");
      await store.refreshState();
    } catch (e) {
      setError((e as Error).message + " 草稿已保留。");
    } finally {
      setSending("");
    }
  }

  return (
    <section id="composer" hidden={!open} aria-labelledby="composertitle">
      <div className="sechead">
        <h2 id="composertitle">碰撞台 · 分析想法</h2>
        <button type="button" id="ideaclose" className="ghost small" onClick={onClose}>
          收起
        </button>
      </div>
      <label htmlFor="idea" className="sr-only">
        要分析的想法
      </label>
      <textarea
        id="idea"
        ref={ref}
        rows={3}
        placeholder="寫下想法，讓 Agent 對照這組 repo 的脈絡分析。"
        value={ui.draft}
        onChange={(e) => ui.setDraft(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey) && !e.nativeEvent.isComposing) {
            e.preventDefault();
            void submit();
          }
        }}
      />
      <div className="composerfoot">
        <span id="ideascope" className="small muted">
          {"分析範圍：" + scopeLabel(scope) + " · " + repos.length + " 個 repo"}
        </span>
        <span className="spacer" />
        <span className="small muted">Ctrl / ⌘ + Enter 送出</span>
        <button
          type="button"
          id="collidebtn"
          className="primary"
          disabled={sending === "sending"}
          onClick={() => void submit()}
        >
          {sending === "sending" ? "正在送出…" : sending === "sent" ? "已送出" : "分析想法"}
        </button>
      </div>
      <p id="ideaerror" className="error" role="alert" hidden={!error}>
        {error}
      </p>
    </section>
  );
}
