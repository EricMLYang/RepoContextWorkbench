import {
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  type ReactNode,
  type RefObject,
} from "react";

/**
 * 原生 <dialog> + showModal：焦點限制、Esc 關閉都交給瀏覽器；
 * 關閉後焦點回到開啟前的元素（WebKit／舊瀏覽器不一定會做，這裡自己補）；點背景關閉。
 */
export function Modal(props: {
  id: string;
  open: boolean;
  onClose: () => void;
  labelledBy: string;
  children: ReactNode;
}) {
  const { id, open, onClose, labelledBy, children } = props;
  const ref = useRef<HTMLDialogElement>(null);
  const returnTo = useRef<Element | null>(null);

  useLayoutEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) {
      returnTo.current = document.activeElement;
      d.showModal();
    } else if (!open && d.open) d.close();
  }, [open]);

  return (
    <dialog
      id={id}
      ref={ref}
      aria-labelledby={labelledBy}
      onClose={() => {
        onClose();
        const back = returnTo.current as HTMLElement | null;
        returnTo.current = null;
        const active = document.activeElement;
        if (back && back.isConnected && (!active || active === document.body || ref.current?.contains(active)))
          back.focus?.();
      }}
      onClick={(e) => {
        const d = ref.current;
        if (!d || e.target !== d) return;
        const r = d.getBoundingClientRect();
        if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom)
          d.close();
      }}
    >
      {open ? children : null}
    </dialog>
  );
}

/** 掛載後把焦點放到這個元素（對話框開啟後的起始焦點）。 */
export function useAutoFocus<T extends HTMLElement>(): RefObject<T | null> {
  const ref = useRef<T>(null);
  useEffect(() => {
    ref.current?.focus();
  }, []);
  return ref;
}

// ── 詳情對話框（共用一個，內容可替換）────────────────────────────────

export interface DetailCtl {
  error: string;
  setError(msg: string): void;
  close(): void;
}

export const DetailContext = createContext<DetailCtl>({
  error: "",
  setError: () => {},
  close: () => {},
});

export function useDetail(): DetailCtl {
  return useContext(DetailContext);
}

/** 詳情內容的版面：body → 錯誤列 → 動作列（與舊版 DOM 順序相同）。 */
export function DetailLayout({ children, actions }: { children?: ReactNode; actions?: ReactNode }) {
  const { error } = useDetail();
  return (
    <>
      <div id="detailbody">{children}</div>
      <p id="detailerror" className="error" role="alert" hidden={!error}>
        {error}
      </p>
      <div id="detailactions" className="dialogfoot">
        {actions}
      </div>
    </>
  );
}

export function DetailDialog(props: {
  spec: { title: string; eyebrow?: string; content: ReactNode; seq: number } | null;
  error: string;
  setError(msg: string): void;
  onClose(): void;
}) {
  const { spec, error, setError, onClose } = props;
  return (
    <Modal id="detaildialog" open={!!spec} onClose={onClose} labelledBy="detailtitle">
      <DetailContext.Provider value={{ error, setError, close: onClose }}>
        <div className="dialoghead">
          <div>
            <p id="detaileyebrow" className="eyebrow">
              {spec?.eyebrow || "DETAILS"}
            </p>
            <h2 id="detailtitle">{spec?.title}</h2>
          </div>
          <button
            type="button"
            id="detailclose"
            className="icon ghost"
            aria-label="關閉詳情"
            onClick={onClose}
          >
            ×
          </button>
        </div>
        <DetailBodyKey key={spec?.seq}>{spec?.content}</DetailBodyKey>
      </DetailContext.Provider>
    </Modal>
  );
}

function DetailBodyKey({ children }: { children: ReactNode }) {
  return <>{children}</>;
}
