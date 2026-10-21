import type { FitAddon } from "@xterm/addon-fit";
import type { Terminal } from "@xterm/xterm";
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type DragEvent,
  type MutableRefObject,
} from "react";
import type { TermStream } from "../../api/backend";
import type { TermInfo } from "../../api/types";
import { AsyncButton } from "../AsyncButton";
import { ContextMenu } from "../ContextMenu";
import {
  draggedFileAbs,
  FILE_DRAG_TYPE,
  pathForTerminal,
  reportLine,
  sessionPair,
  sessionWhere,
} from "../../lib/view";
import { findInLines, findPathHits, lineCells, type TermHit } from "../../lib/termLinks";
import { useSnapshot, useStore, useUi } from "../../store/context";
import { autoContinueSpec, confirmKillSpec, sessionResultSpec } from "./SessionDialogs";
import { transcriptSpec } from "./Transcript";

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
  /** 這裡填入「貼到目前會話」：成功回會話標題；沒有連上的會話回 null。 */
  pasteRef?: MutableRefObject<((text: string) => string | null) | null>;
}) {
  const { activeSid, setActiveSid, collapsed, setCollapsed, selectSeq, pasteRef } = props;
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
  // F-111：終端內搜尋（掃 xterm 的緩衝，選起找到的那段並捲過去）
  const [finding, setFinding] = useState(false);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<TermHit[]>([]);
  const [hitAt, setHitAt] = useState(-1);
  const cellsRef = useRef<number[][]>([]);
  const findInputRef = useRef<HTMLInputElement>(null);
  const openFindRef = useRef<() => void>(() => {});
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
      // 先標「還沒連上」再掛監聽：串流若同步就開好，onOpen 的「已連上」不會被蓋掉
      setConnected(sid, false);
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
      // F-95：輸出裡的檔案路徑（`src/a.rs:12`）畫底線，點了在檢視器開、捲到那一行；
      // 是不是登記的 repo 裡真有的檔交給引擎判斷（相對路徑照這個會話的目錄）
      t.registerLinkProvider({
        provideLinks(y, done) {
          const line = t.buffer.active.getLine(y - 1);
          if (!line) return done(undefined);
          const { text, cellOf } = lineCells(line);
          const hits = findPathHits(text);
          done(
            hits.length
              ? hits.map((h) => ({
                  text: h.text,
                  range: {
                    start: { x: cellOf[h.start] + 1, y },
                    end: { x: cellOf[h.end - 1] + 1, y },
                  },
                  decorations: { underline: true, pointerCursor: true },
                  activate: () => {
                    store
                      .action("locate_path", { text: h.text, sid })
                      .then((r) =>
                        ui.openFile(r.repo, r.path, r.line ? { line: r.line } : undefined),
                      )
                      .catch((e: unknown) =>
                        store.toast(e instanceof Error ? e.message : String(e)),
                      );
                  },
                }))
              : undefined,
          );
        },
      });
      // F-111：⌘F（Windows／Linux：Ctrl+Shift+F）在終端裡找；這個鍵不送進程序
      t.attachCustomKeyEventHandler((e) => {
        const mac = /mac/i.test(navigator.platform);
        if (
          e.type === "keydown" &&
          e.key.toLowerCase() === "f" &&
          (mac ? e.metaKey && !e.ctrlKey : e.ctrlKey && e.shiftKey)
        ) {
          openFindRef.current();
          return false;
        }
        return true;
      });
      t.onData((d) => stream.write(d));
      t.onResize(({ cols, rows }) => stream.resize(cols, rows));
      for (const [id, r] of recs.current) r.host.classList.toggle("on", id === sid);
    },
    [store, ui, dispose, setConnected],
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

  const openShellFor = useCallback(
    async (agentSid: string) => {
      const r = await store.action("term_create", { kind: "shell", of: agentSid });
      await store.refreshState();
      ui.selectTerm(r.sid);
    },
    [store, ui],
  );

  // Ctrl+Shift+` 在 Agent 和它的 Terminal 之間切換（Terminal 還沒開就開一個）
  // agent 會話和它的 Terminal 共用這塊版面：標題列切換，位置標籤跟著 agent
  const pair = sessionPair(terms, activeSid);
  const pairRef = useRef(pair);
  useEffect(() => {
    pairRef.current = pair;
  });
  const opening = useRef(false);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!(e.ctrlKey && e.shiftKey && (e.key === "`" || e.key === "~"))) return;
      const p = pairRef.current;
      if (!p || document.querySelector("dialog[open]")) return;
      e.preventDefault();
      if (p.onShell) ui.selectTerm(p.agent.sid);
      else if (p.shell?.alive) ui.selectTerm(p.shell.sid);
      else if (!opening.current) {
        opening.current = true;
        openShellFor(p.agent.sid)
          .catch((err: unknown) => store.toast(err instanceof Error ? err.message : String(err)))
          .finally(() => (opening.current = false));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ui, store, openShellFor]);

  const info: TermInfo | undefined = terms.find((s) => s.sid === activeSid);

  const activeTerm = () => (activeSid ? recs.current.get(activeSid)?.t : undefined);
  /** 選起第 i 個結果、捲到畫面中間。 */
  const showHit = (list: TermHit[], i: number) => {
    const t = activeTerm();
    const h = list[i];
    if (!t || !h) return;
    const cells = cellsRef.current[h.y] || [];
    const x0 = cells[h.start] ?? h.start;
    const x1 = cells[h.end - 1] ?? h.end - 1;
    const wide = t.buffer.active.getLine(h.y)?.getCell(x1)?.getWidth() ?? 1;
    t.select(x0, h.y, x1 + Math.max(1, wide) - x0);
    t.scrollToLine(Math.max(0, h.y - Math.floor(t.rows / 2)));
    setHitAt(i);
  };
  const runFind = (q: string) => {
    setQuery(q);
    const t = activeTerm();
    if (!t) return;
    const buf = t.buffer.active;
    const texts: string[] = [];
    const cells: number[][] = [];
    for (let y = 0; y < buf.length; y++) {
      const line = buf.getLine(y);
      const lc = line ? lineCells(line) : { text: "", cellOf: [] };
      texts.push(lc.text);
      cells.push(lc.cellOf);
    }
    cellsRef.current = cells;
    const found = findInLines(texts, q);
    setHits(found);
    if (!found.length) {
      t.clearSelection();
      setHitAt(-1);
      return;
    }
    // 從最後面（最新的輸出）開始找
    showHit(found, found.length - 1);
  };
  const stepFind = (dir: 1 | -1) => {
    if (!hits.length) return;
    showHit(hits, (hitAt + dir + hits.length) % hits.length);
  };
  const closeFind = () => {
    setFinding(false);
    activeTerm()?.clearSelection();
    activeTerm()?.focus();
  };
  useEffect(() => {
    openFindRef.current = () => {
      setFinding(true);
      setCollapsed(false);
      setTimeout(() => findInputRef.current?.select(), 0);
    };
  });

  // F-97：帶著同樣的前文岔出一場新會話（新的 Claude Code 會話 id），這一場照常跑
  const forkSession = (t: TermInfo) => {
    if (!t.claude_session || !t.resume_cwd) return;
    store
      .action("term_create", {
        kind: "agent",
        resume: t.claude_session,
        cwd: t.resume_cwd,
        fork: true,
        task: "分岔：" + (t.ai_title || t.title),
      })
      .then(async (r) => {
        await store.refreshState();
        ui.selectTerm(r.sid);
        store.toast("已岔出新會話");
      })
      .catch((e: unknown) => store.toast(e instanceof Error ? e.message : String(e)));
  };
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const moreRef = useRef<HTMLButtonElement>(null);

  // 貼到目前會話：走 xterm 的 paste（尊重 bracketed paste，換行不會被當成送出），不按 Enter
  const pasteActive = (text: string): string | null => {
    const rec = activeSid ? recs.current.get(activeSid) : undefined;
    if (!rec || !activeSid || !conn[activeSid] || !info?.alive) return null;
    setCollapsed(false);
    rec.t.paste(text);
    rec.t.focus();
    return info.title;
  };
  useEffect(() => {
    if (!pasteRef) return;
    pasteRef.current = pasteActive;
    return () => {
      pasteRef.current = null;
    };
  });

  // 從檔案樹拖檔進來：agent 會話貼 @路徑、shell 貼加引號的路徑（不按 Enter）
  const [dropping, setDropping] = useState(false);
  const dragProps = {
    onDragOver: (e: DragEvent) => {
      if (!e.dataTransfer.types.includes(FILE_DRAG_TYPE)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      setDropping(true);
    },
    onDragLeave: () => setDropping(false),
    onDrop: (e: DragEvent) => {
      setDropping(false);
      const abs = draggedFileAbs(e.dataTransfer.getData(FILE_DRAG_TYPE));
      if (!abs) return;
      e.preventDefault();
      const kind = info?.kind ?? "shell";
      const windows = /win/i.test(navigator.platform);
      if (!pasteActive(pathForTerminal(abs, kind, windows)))
        store.toast("先在下方終端選一個執行中的會話，再把檔案拖進來");
    },
  };

  const attached = !!activeSid && activeSid in conn;
  const connected = !!activeSid && conn[activeSid];
  const where = pair ? sessionWhere(pair.agent) : info ? sessionWhere(info) : null;
  const title = info
    ? info.title +
      " · " +
      reportLine(info) +
      (attached && !connected ? " · 連線已中斷（畫面可能不是最新）" : "")
    : "尚未選擇會話";

  return (
    <section
      id="termpanel"
      aria-label="終端面板"
      className={dropping ? "dropping" : undefined}
      {...dragProps}
    >
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
        {pair ? (
          <span className="termswitch" role="group" aria-label="切換 Agent／Terminal">
            <button
              type="button"
              className={"ghost small" + (pair.onShell ? "" : " on")}
              aria-pressed={!pair.onShell}
              title="回到 Agent 對話（Ctrl+Shift+`）"
              onClick={() => ui.selectTerm(pair.agent.sid)}
            >
              Agent
            </button>
            {pair.shell?.alive ? (
              <button
                type="button"
                className={"ghost small" + (pair.onShell ? " on" : "")}
                aria-pressed={pair.onShell}
                title="切到這個會話目錄下的 Terminal（Ctrl+Shift+`）"
                onClick={() => pair.shell && ui.selectTerm(pair.shell.sid)}
              >
                Terminal
              </button>
            ) : (
              <AsyncButton
                className="ghost small"
                title="在這個會話的目錄開一個 Terminal（Ctrl+Shift+`）"
                onError={(m) => store.toast(m)}
                run={() => openShellFor(pair.agent.sid)}
              >
                開啟終端
              </AsyncButton>
            )}
          </span>
        ) : (
          <span className="termlabel">終端</span>
        )}
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
        {finding ? (
          <form
            className="termfind"
            role="search"
            onSubmit={(e) => {
              e.preventDefault();
              if (hits.length) stepFind(-1);
              else runFind(query);
            }}
          >
            <input
              ref={findInputRef}
              type="search"
              value={query}
              placeholder="在終端裡找"
              aria-label="在終端裡找"
              onChange={(e) => runFind(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  e.preventDefault();
                  closeFind();
                } else if (e.key === "Enter" && e.shiftKey) {
                  e.preventDefault();
                  stepFind(1);
                }
              }}
            />
            <span className="small muted" aria-live="polite">
              {query ? (hits.length ? hitAt + 1 + "/" + hits.length : "找不到") : ""}
            </span>
            <button
              type="button"
              className="ghost small"
              aria-label="上一個"
              title="上一個（Enter）"
              disabled={!hits.length}
              onClick={() => stepFind(-1)}
            >
              ↑
            </button>
            <button
              type="button"
              className="ghost small"
              aria-label="下一個"
              title="下一個（Shift+Enter）"
              disabled={!hits.length}
              onClick={() => stepFind(1)}
            >
              ↓
            </button>
            <button type="button" className="ghost small" aria-label="關閉尋找" onClick={closeFind}>
              ×
            </button>
          </form>
        ) : null}
        <button
          type="button"
          id="termreconnect"
          className="ghost small"
          hidden={!attached || connected}
          onClick={() => activeSid && void attach(activeSid)}
        >
          重新連線
        </button>
        {where?.repo ? (
          <button
            type="button"
            className="ghost small"
            title={"在右側欄看 " + where.repo + " 還沒 commit 的變更（agent 改了什麼）"}
            onClick={() => where.repo && ui.showRepo(where.repo, "changes")}
          >
            查看變更
          </button>
        ) : null}
        <button
          ref={moreRef}
          type="button"
          id="termmore"
          className="ghost small"
          aria-label="更多會話動作"
          aria-haspopup="menu"
          aria-expanded={!!menu}
          disabled={!info}
          onClick={() => {
            const r = moreRef.current?.getBoundingClientRect();
            setMenu(menu ? null : { x: r ? r.right - 200 : 0, y: r ? r.top - 4 : 0 });
          }}
        >
          ⋯
        </button>
        {menu && info ? (
          <ContextMenu
            id="termmenu"
            title={info.title || "會話"}
            items={[
              ...(info.kind === "agent"
                ? [
                    {
                      label: "逐字稿（完整對話與工具）",
                      run: () =>
                        ui.openDetail(
                          transcriptSpec({ sid: info.sid }, info.ai_title || info.title),
                        ),
                    },
                  ]
                : []),
              ...(info.claude_session && info.resume_cwd
                ? [{ label: "岔出新會話（這場不動）", run: () => forkSession(info) }]
                : []),
              ...(info.kind === "agent" && info.alive
                ? [
                    {
                      label: info.auto_continue
                        ? `自動繼續（${info.auto_continue.count}/${info.auto_continue.max}）…`
                        : "自動繼續…",
                      run: () => ui.openDetail(autoContinueSpec(info)),
                    },
                  ]
                : []),
              ...(attached
                ? [{ label: "在終端裡找（⌘F）", run: () => openFindRef.current() }]
                : []),
              // 只打開紀錄表單；不會結束程序、也不會 commit
              { label: "記錄本次工作結果…", run: () => ui.openDetail(sessionResultSpec(info)) },
              {
                label: "停止此會話程序…",
                danger: true,
                run: () => ui.openDetail(confirmKillSpec(info)),
              },
            ]}
            x={menu.x}
            y={menu.y}
            above
            onClose={() => setMenu(null)}
          />
        ) : null}
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
