import type { FitAddon } from "@xterm/addon-fit";
import type { Terminal } from "@xterm/xterm";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { TermStream } from "../../api/backend";
import type { TermInfo } from "../../api/types";
import { reportLine, sessionWhere } from "../../lib/view";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { confirmKillSpec, sessionResultSpec } from "./SessionDialogs";

interface TermRec {
  t: Terminal;
  fit: FitAddon;
  stream: TermStream;
  host: HTMLDivElement;
}

/** xterm（主程式包一半大小）等第一次開終端才載入；之後共用同一份。 */
let xtermModules: Promise<{ Terminal: typeof Terminal; FitAddon: typeof FitAddon }> | null = null;
function loadXterm() {
  xtermModules ??= Promise.all([
    import("@xterm/xterm"),
    import("@xterm/addon-fit"),
    import("@xterm/xterm/css/xterm.css"),
  ]).then(([x, f]) => ({ Terminal: x.Terminal, FitAddon: f.FitAddon }));
  return xtermModules;
}

const COLLAPSED_H = 44;
const MIN_H = 100;

function maxHeight() {
  return Math.max(MIN_H, window.innerHeight - 240);
}

/**
 * 底部終端面板：每個會話一個 xterm（切換只換顯示，不重連）；拖拉／方向鍵調高度；
 * 收合不殺程序；連線斷了保留輸出、可重新連線。
 */
export function TerminalPanel(props: {
  activeSid: string | null;
  setActiveSid: (sid: string | null) => void;
  collapsed: boolean;
  setCollapsed: (c: boolean) => void;
  /** 每次「選擇會話」遞增：觸發 attach／focus（同一個 sid 再點也要聚焦）。 */
  selectSeq: number;
}) {
  const { activeSid, setActiveSid, collapsed, setCollapsed, selectSeq } = props;
  const store = useStore();
  const ui = useUi();
  const { state } = useSnapshot();
  const terms = useMemo(() => state.terms || [], [state.terms]);
  const bodyRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<HTMLDivElement>(null);
  const recs = useRef(new Map<string, TermRec>());
  // 每個會話的連線狀態（畫面用；xterm 實體放在 recs ref）
  const [conn, setConn] = useState<Record<string, boolean>>({});
  const setConnected = useCallback(
    (sid: string, v: boolean | null) =>
      setConn((c) => {
        const next = { ...c };
        if (v === null) delete next[sid];
        else next[sid] = v;
        return next;
      }),
    [],
  );
  const [height, setHeight] = useState(260);
  const dragging = useRef(false);
  const attachGen = useRef(new Map<string, number>());

  const fitAll = useCallback(() => {
    for (const r of recs.current.values()) {
      try {
        r.fit.fit();
      } catch {
        /* 隱藏中量不到尺寸 */
      }
    }
  }, []);

  const dispose = useCallback(
    (sid: string) => {
      const r = recs.current.get(sid);
      if (!r) return;
      r.stream.close();
      r.t.dispose();
      r.host.remove();
      recs.current.delete(sid);
      setConnected(sid, null);
    },
    [setConnected],
  );

  const attach = useCallback(
    async (sid: string) => {
      // 等 xterm 載入期間又按了一次（重新連線）：只有最後一次算數
      const gen = (attachGen.current.get(sid) ?? 0) + 1;
      attachGen.current.set(sid, gen);
      const { Terminal, FitAddon } = await loadXterm();
      const body = bodyRef.current;
      if (!body || attachGen.current.get(sid) !== gen) return;
      dispose(sid);
      const host = document.createElement("div");
      host.className = "termhost on";
      body.append(host);
      const t = new Terminal({
        fontSize: 13,
        fontFamily: "Cascadia Code,Consolas,Menlo,monospace",
        theme: {
          background: "#11151b",
          foreground: "#e9edf4",
          cursor: "#a7c5ff",
          selectionBackground: "#394966",
        },
        cursorBlink: true,
        scrollback: 5000,
      });
      const fit = new FitAddon();
      t.loadAddon(fit);
      t.open(host);
      const stream = store.backend.openTerminal(sid);
      const rec: TermRec = { t, fit, stream, host };
      recs.current.set(sid, rec);
      stream.onData((b) => t.write(b));
      stream.onOpen(() => {
        try {
          fit.fit();
        } catch {
          /* ignore */
        }
        stream.resize(t.cols, t.rows);
        setConnected(sid, true);
        t.focus();
      });
      stream.onClose(() => {
        t.write("\r\n\x1b[2m[連線已中斷。可按「重新連線」，輸出仍保留。]\x1b[0m\r\n");
        setConnected(sid, false);
      });
      t.onData((d) => stream.write(d));
      t.onResize(({ cols, rows }) => stream.resize(cols, rows));
      for (const [id, r] of recs.current) r.host.classList.toggle("on", id === sid);
      setConnected(sid, false);
    },
    [store, dispose, setConnected],
  );

  // 清掉已不存在的會話
  useEffect(() => {
    const ids = new Set(terms.map((s) => s.sid));
    for (const sid of [...recs.current.keys()])
      if (!ids.has(sid)) {
        dispose(sid);
        if (activeSid === sid) setActiveSid(null);
      }
  }, [terms, activeSid, dispose, setActiveSid]);

  // 選擇會話：顯示它的 xterm，沒有就 attach
  useEffect(() => {
    if (!activeSid || !selectSeq) return;
    if (!terms.some((s) => s.sid === activeSid)) return;
    const rec = recs.current.get(activeSid);
    for (const [id, r] of recs.current) r.host.classList.toggle("on", id === activeSid);
    if (!rec) void attach(activeSid);
    else {
      try {
        rec.fit.fit();
      } catch {
        /* ignore */
      }
      rec.t.focus();
    }
    // terms 只用來確認存在；不要因輪詢重跑
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSid, selectSeq, attach]);

  // 高度＝CSS 變數 --termh（body grid 的 term 列）
  useLayoutEffect(() => {
    document.documentElement.style.setProperty(
      "--termh",
      collapsed ? COLLAPSED_H + "px" : height + "px",
    );
    fitAll();
  }, [collapsed, height, fitAll]);

  const setTermHeight = useCallback(
    (v: number) => {
      setHeight(Math.min(Math.max(v, MIN_H), maxHeight()));
      setCollapsed(false);
    },
    [setCollapsed],
  );

  useEffect(() => {
    const onResize = () => {
      if (!collapsed) setHeight((h) => Math.min(Math.max(h, MIN_H), maxHeight()));
      fitAll();
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, [collapsed, fitAll]);

  // 卸載時關掉所有連線（不殺程序）
  useEffect(() => {
    const map = recs.current;
    return () => {
      for (const sid of [...map.keys()]) dispose(sid);
    };
  }, [dispose]);

  const info: TermInfo | undefined = terms.find((s) => s.sid === activeSid);
  const attached = !!activeSid && activeSid in conn;
  const connected = !!activeSid && conn[activeSid];
  const where = info ? sessionWhere(info) : null;
  const title = info
    ? info.title +
      " · " +
      reportLine(info) +
      (attached && !connected ? " · 連線已中斷（畫面可能不是最新）" : "")
    : "尚未選擇會話";

  return (
    <section id="termpanel" aria-label="終端面板">
      <div
        id="termdrag"
        ref={dragRef}
        role="separator"
        tabIndex={0}
        aria-label="調整終端高度"
        aria-orientation="horizontal"
        aria-valuemin={MIN_H}
        aria-valuemax={typeof window !== "undefined" ? maxHeight() : 640}
        aria-valuenow={height}
        onPointerDown={(e) => {
          dragging.current = true;
          dragRef.current?.setPointerCapture?.(e.pointerId);
          e.preventDefault();
        }}
        onPointerMove={(e) => {
          if (dragging.current) setTermHeight(window.innerHeight - e.clientY - 26);
        }}
        onPointerUp={() => (dragging.current = false)}
        onPointerCancel={() => (dragging.current = false)}
        onKeyDown={(e) => {
          if (e.key === "ArrowUp" || e.key === "ArrowDown") {
            e.preventDefault();
            setTermHeight(height + (e.key === "ArrowUp" ? 24 : -24));
          }
        }}
      />
      <div id="termbar">
        <span className="termlabel">終端</span>
        {where ? (
          <span id="termwhere" className="termwhere" title={"這個會話屬於：" + where.label}>
            {where.group ? <strong>{where.group}</strong> : null}
            {where.group && where.repo ? <span aria-hidden="true"> › </span> : null}
            {where.repo ? <span>{where.repo}</span> : null}
            {!where.group && !where.repo ? <span>{where.label}</span> : null}
          </span>
        ) : null}
        <span id="termtitle" className="small muted">
          {title}
        </span>
        <span className="spacer" />
        <button
          type="button"
          id="termreconnect"
          className="ghost small"
          hidden={!attached || connected}
          onClick={() => activeSid && void attach(activeSid)}
        >
          重新連線
        </button>
        <button
          type="button"
          id="termresult"
          className="ghost small"
          disabled={!info}
          onClick={() => info && ui.openDetail(sessionResultSpec(info))}
        >
          記錄結果
        </button>
        <button
          type="button"
          id="termkill"
          className="ghost small"
          disabled={!info}
          onClick={() => info && ui.openDetail(confirmKillSpec(info))}
        >
          結束程序…
        </button>
        <button
          type="button"
          id="termtoggle"
          className="ghost small"
          aria-expanded={!collapsed}
          onClick={ui.toggleTerm}
        >
          {collapsed ? "展開" : "收合"}
        </button>
      </div>
      <div id="termbody" ref={bodyRef} hidden={collapsed}>
        <div id="termempty" className="muted" hidden={!!info}>
          從左側選擇會話，繼續你的工作。
        </div>
      </div>
    </section>
  );
}
