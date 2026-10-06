import { DetailLayout, useDetail } from "./Modal";
import { useStore } from "../store/context";

/** 完整內容（commit、事件、簡報）：可複製。 */
export function FullText({ text, meta = "" }: { text: string; meta?: string }) {
  const store = useStore();
  const { setError, close } = useDetail();
  return (
    <DetailLayout
      actions={
        <>
          <button
            type="button"
            onClick={async () => {
              try {
                await navigator.clipboard.writeText(text);
                store.toast("已複製內容");
              } catch {
                setError("無法存取剪貼簿，可直接選取上方文字複製。");
              }
            }}
          >
            複製內容
          </button>
          <button type="button" className="primary" onClick={close}>
            關閉
          </button>
        </>
      }
    >
      <p className="detailmeta">{meta}</p>
      <pre>{text}</pre>
    </DetailLayout>
  );
}

export function fullTextSpec(title: string, text: string, meta = "") {
  return { title, content: <FullText text={text} meta={meta} /> };
}
