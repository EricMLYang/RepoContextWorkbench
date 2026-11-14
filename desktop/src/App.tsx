import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { GroupInfo } from "./api/types";
import { StatusBar, Toast, TopBar } from "./components/Chrome";
import { useSavedWidths } from "./components/Resizer";
import { Sidebar } from "./components/Sidebar";
import { SessionForm } from "./components/sessions/SessionDialogs";
import { TerminalPanel } from "./components/sessions/TerminalPanel";
import { Workspace } from "./components/Workspace";
import { RepoMenu } from "./components/deep/RepoMenu";
import { RepoPanel } from "./components/deep/RepoPanel";
import { Palette } from "./components/Palette";
import { PickTray } from "./components/PickTray";
import { FileTabs } from "./components/viewer/FileTabs";
import { FileViewer, fileKey } from "./components/viewer/FileViewer";
import { useFileTabs } from "./components/viewer/useFileTabs";
import { ScreenReporter } from "./store/screen";
import { Onboarding } from "./dialogs/misc";
import { DetailDialog } from "./dialogs/Modal";
import { PickerDialog } from "./dialogs/PickerDialog";
import { load, save } from "./lib/storage";
import { firstNeedingYou, scopeMemoryKey, sessionFresh, sessionPayload } from "./lib/view";
import {
  StoreContext,
  UiContext,
  UiStateContext,
  samePick,
  type DetailSpec,
  type PanelTab,
  type Pick,
  type Tab,
  type Ui,
  type UiState,
} from "./store/context";
import type { WorkbenchStore } from "./store/store";

/** 一個範圍上次的畫面（存本機，鍵見 `scopeMemoryKey`）。 */
interface ScopeView {
  files: { repo: string; path: string }[];
  active: string | null;
  panelRepo: string | null;
  tab: Tab;
  activeSid: string | null;
}
const TABS: Tab[] = ["inbox", "notes", "repos", "files", "activity", "relations"];

/**
 * 工作台。一級物件：牌（側欄）／收件匣（工作區）／會話（側欄下段＋終端面板）／深看（分頁）。
 * 資料全由 store 從後端取得；這裡只管畫面狀態（開哪個對話框、哪個分頁、哪個會話）。
 */
export function App({ store, autoStart = true }: { store: WorkbenchStore; autoStart?: boolean }) {
  // ── 詳情對話框 ──
  const [detail, setDetail] = useState<(DetailSpec & { seq: number }) | null>(null);
  const [detailError, setDetailError] = useState("");
  const seq = useRef(0);
  const openDetail = useCallback((spec: DetailSpec) => {
    seq.current += 1;
    setDetail({ ...spec, seq: seq.current });
    setDetailError("");
  }, []);
  const closeDetail = useCallback(() => setDetail(null), []);

  // ── 選擇 repo ──
  const [picker, setPicker] = useState<{ open: boolean; group: GroupInfo | null; seq: number }>({
    open: false,
    group: null,
    seq: 0,
  });
  const openPicker = useCallback(
    (group: GroupInfo | null = null) => setPicker((p) => ({ open: true, group, seq: p.seq + 1 })),
    [],
  );
  const closePicker = useCallback(() => setPicker((p) => ({ ...p, open: false })), []);

  // ── 碰撞台草稿（存本機，重新整理不丟）──
  const [draft, setDraftState] = useState(() => load<string>("draft", ""));
  const draftRef = useRef(draft);
  const [composerOpen, setComposerOpen] = useState(() => !!draft);
  const [composerFocusSeq, setComposerFocusSeq] = useState(0);
  const [notesFocusSeq, setNotesFocusSeq] = useState(0);
  const setDraft = useCallback((v: string) => {
    draftRef.current = v;
    setDraftState(v);
    save("draft", v);
  }, []);
  const showComposer = useCallback(() => {
    setComposerOpen(true);
    setComposerFocusSeq((n) => n + 1);
  }, []);

  // ── 分頁 ──
  const [tab, setTab] = useState<Tab>("inbox");

  // ── 牌組展開 ──
  const [openGroups, setOpenGroups] = useState<Set<string>>(
    () => new Set(load<string[]>("openGroups", [])),
  );
  const updateOpen = useCallback((fn: (s: Set<string>) => void) => {
    setOpenGroups((prev) => {
      const next = new Set(prev);
      fn(next);
      save("openGroups", [...next]);
      return next;
    });
  }, []);

  // 側欄寬度（拖曳把手調的，存在本機偏好）
  useSavedWidths();

  // ── 挑檔案給 agent（記在這台電腦；檔案不見了交出去時引擎會擋）──
  const [picks, setPicks] = useState<Pick[]>(() => {
    const v = load<unknown>("picks", []);
    return Array.isArray(v)
      ? v.filter(
          (x): x is Pick =>
            !!x &&
            typeof x.repo === "string" &&
            typeof x.path === "string" &&
            typeof x.name === "string",
        )
      : [];
  });
  useEffect(() => save("picks", picks), [picks]);
  const updatePicks = useCallback((f: (cur: Pick[]) => Pick[]) => setPicks(f), []);

  // ── 貼到目前會話（TerminalPanel 填入）、⌘K ──
  const pasteRef = useRef<((text: string) => string | null) | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  // ⌘⇧F 打開時直接是「%」搜內容（跟 VS Code 一樣）
  const [paletteInit, setPaletteInit] = useState("");
  const closePalette = useCallback(() => setPaletteOpen(false), []);

  // ── 右側欄（點 repo）＋右鍵選單 ──
  const [panelRepo, setPanelRepo] = useState<string | null>(() =>
    load<string | null>("panelRepo", null),
  );
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const setPanel = useCallback((fn: (cur: string | null) => string | null) => {
    setPanelRepo((cur) => {
      const next = fn(cur);
      save("panelRepo", next);
      return next;
    });
  }, []);
  const closeMenu = useCallback(() => setMenu(null), []);
  const [panelTab, setPanelTab] = useState<PanelTab>(() => load<PanelTab>("panelTab", "files"));
  const choosePanelTab = useCallback((t: PanelTab) => {
    save("panelTab", t);
    setPanelTab(t);
  }, []);
  // 挑選列疊在主區下緣：主區底下留空，最後幾列不會被蓋住
  const hasPicks = picks.length > 0;
  useEffect(() => {
    document.body.classList.toggle("withpicks", hasPicks);
    return () => document.body.classList.remove("withpicks");
  }, [hasPicks]);

  // 版面多一欄：body 是 grid（styles.css 的 body.withpanel）
  useEffect(() => {
    document.body.classList.toggle("withpanel", !!panelRepo);
    return () => document.body.classList.remove("withpanel");
  }, [panelRepo]);

  // ── 檔案分頁（主區）：null＝看工作區 ──
  const toast = useCallback((msg: string) => store.toast(msg), [store]);
  const {
    files,
    shownFile,
    shownFileObj,
    dirtyFiles,
    openFile,
    closeFile,
    setActiveFile,
    setFileDirty,
    dirtyUnder,
    renamePath,
    closeUnder,
    getTabs,
    swapTabs,
  } = useFileTabs(toast);

  // ── 回報畫面（agent 被問「根據畫面」才讀；停下來才送）──
  const [screen] = useState(() => new ScreenReporter(store));
  useEffect(() => {
    screen.setBase({
      file: shownFileObj ? { repo: shownFileObj.repo, path: shownFileObj.path } : null,
      tab,
      panel: panelRepo ? { repo: panelRepo, tab: panelTab } : null,
    });
  }, [screen, shownFileObj, tab, panelRepo, panelTab]);
  // 換牌組也要回報（範圍在 store 裡，不在這裡的 state）
  useEffect(() => store.subscribe(screen.schedule), [store, screen]);
  useEffect(() => () => screen.stop(), [screen]);

  // ── 終端 ──
  const [activeSid, setActiveSid] = useState<string | null>(null);
  const [termCollapsed, setTermCollapsed] = useState(true);
  const [selectSeq, setSelectSeq] = useState(0);

  // ── 每個牌組記住自己看到哪（2026-11-10，照 VS Code 的 workspace）：開著的檔案分頁、正在看的檔、右側欄的 repo、
  // 工作區分頁、目前的會話。換牌組時存舊的、還原新的；沒去過的牌組檔案分頁從空的開始（右側欄、工作區分頁、會話照舊）。
  const viewRef = useRef({ panelRepo, tab, activeSid });
  useEffect(() => {
    viewRef.current = { panelRepo, tab, activeSid };
  }, [panelRepo, tab, activeSid]);
  useEffect(
    () =>
      store.onScopeChange((prev, next) => {
        const from = scopeMemoryKey(prev);
        const to = scopeMemoryKey(next);
        if (from === to) return;
        save("view:" + from, { ...getTabs(), ...viewRef.current });
        const v = load<Partial<ScopeView> | null>("view:" + to, null);
        const saved = Array.isArray(v?.files)
          ? v.files.filter((f) => !!f && typeof f.repo === "string" && typeof f.path === "string")
          : [];
        swapTabs(saved, typeof v?.active === "string" ? v.active : null);
        // 右側欄：去過的牌組還原當時的（含「關著」）；沒去過的不動
        if (v) setPanel(() => (typeof v.panelRepo === "string" ? v.panelRepo : null));
        if (v?.tab && TABS.includes(v.tab)) setTab(v.tab);
        // 會話只在它還活著時切過去（收合狀態不動，不搶畫面）
        const sid = v?.activeSid;
        if (sid && (store.getSnapshot().state.terms || []).some((t) => t.sid === sid && t.alive))
          setActiveSid(sid);
      }),
    [store, getTabs, swapTabs, setPanel],
  );

  // 操作只依賴穩定的 setter，整個生命週期同一個物件；會變的狀態另外放 UiStateContext
  const ui: Ui = useMemo(
    () => ({
      openDetail,
      closeDetail,
      setDetailError,
      openPicker,
      openSession(extra = {}) {
        const payload = sessionPayload(store.scopePayload, extra);
        // 對單一 repo 開：把目前選的組當提示，屬於哪個牌組由引擎判斷（term_create 的 home_group_for）
        const { scope } = store.getSnapshot();
        if (typeof payload.repo === "string" && !payload.group && scope.group)
          payload.home_group = scope.group;
        openDetail({
          title: extra.steward ? "開總管會話" : "開啟 Agent 會話",
          eyebrow: extra.steward ? "STEWARD" : "NEW SESSION",
          content: <SessionForm payload={payload} task={String(extra.task ?? "")} />,
        });
      },
      openOnboarding() {
        openDetail({ title: "建立第一個工作範圍", eyebrow: "FIRST RUN", content: <Onboarding /> });
      },
      openRepo: (id) => setPanel((cur) => (cur === id ? null : id)),
      showRepo: (id, tab) => {
        if (tab) choosePanelTab(tab);
        setPanel(() => id);
      },
      openPalette: () => {
        setPaletteInit("");
        setPaletteOpen(true);
      },
      togglePick: (pick) =>
        updatePicks((cur) =>
          cur.some((x) => samePick(x, pick))
            ? cur.filter((x) => !samePick(x, pick))
            : [...cur, pick],
        ),
      clearPicks: () => updatePicks(() => []),
      pasteToSession: (text) => pasteRef.current?.(text) ?? null,
      closeRepo: () => setPanel(() => null),
      repoMenu: (id, x, y) => setMenu({ id, x, y }),
      openFile,
      dirtyUnder,
      renamePath,
      forgetPath: (repo, path) => {
        closeUnder(repo, path);
        updatePicks((cur) =>
          cur.filter(
            (p) => !(p.repo === repo && (p.path === path || p.path.startsWith(path + "/"))),
          ),
        );
      },
      showComposer: () => {
        setActiveFile(null);
        showComposer();
      },
      getDraft: () => draftRef.current,
      setDraft,
      // 切分頁／開碰撞台＝要看工作區：正在看檔案分頁的話先回到工作區（⌘K 指令、選單列都會走這裡）
      showTab: (t) => {
        setActiveFile(null);
        setTab(t);
      },
      selectTerm(sid) {
        setActiveSid(sid);
        setTermCollapsed(false);
        setSelectSeq((n) => n + 1);
      },
      toggleTerm: () => setTermCollapsed((c) => !c),
      collapseTerm: () => setTermCollapsed(true),
      toggleGroup: (key) => updateOpen((s) => (s.has(key) ? s.delete(key) : s.add(key))),
      renameOpenGroup: (from, to) =>
        updateOpen((s) => {
          if (s.delete(from)) s.add(to);
        }),
      forgetOpenGroup: (name) => updateOpen((s) => s.delete(name)),
      reportFileView: (repo, path, measure) => screen.setDetail(repo, path, measure),
      screenChanged: screen.schedule,
    }),
    [
      store,
      openDetail,
      closeDetail,
      openPicker,
      showComposer,
      setDraft,
      setPanel,
      choosePanelTab,
      openFile,
      dirtyUnder,
      renamePath,
      closeUnder,
      setActiveFile,
      updateOpen,
      updatePicks,
      screen,
    ],
  );
  const uiState: UiState = useMemo(
    () => ({
      panelRepo,
      activeFile: shownFileObj,
      draft,
      termCollapsed,
      activeSid,
      openGroups,
      picks,
    }),
    [panelRepo, shownFileObj, draft, termCollapsed, activeSid, openGroups, picks],
  );

  // 首次載入＋輪詢；#deep 直接開 Repo 分頁、#adhoc 開選擇 repo
  useEffect(() => {
    if (!autoStart) return;
    store.start();
    let live = true;
    void store.refreshState().then(() => {
      if (!live) return;
      if (location.hash === "#deep") setTab("repos");
      if (location.hash === "#adhoc") openPicker();
    });
    return () => {
      live = false;
      store.stop();
    };
  }, [store, autoStart, openPicker]);

  // 選單列「記一筆…」：回到工作區的筆記分頁、游標放進輸入框（沒選牌組時筆記分頁會說要先選）
  useEffect(
    () =>
      store.backend.onCapture?.(() => {
        setActiveFile(null);
        setTab("notes");
        setNotesFocusSeq((n) => n + 1);
      }),
    [store, setActiveFile],
  );

  // 選單列「等你的會話」：切到最該先處理的那個（卡住的優先）
  useEffect(
    () =>
      store.backend.onNextWaiting?.(() => {
        const t = firstNeedingYou(store.getSnapshot().state.terms || []);
        if (t) ui.selectTerm(t.sid);
        else store.toast("現在沒有會話在等你");
      }),
    [store, ui],
  );

  // F-80：正在看的會話做完一輪＝看過了：告訴引擎記 seen（亮綠退成暗綠、徽章數字減一）。
  // 切到它時查一次，之後它的狀態每變一次再查（下一輪做完又會變回沒看）
  useEffect(() => {
    if (!activeSid) return;
    const check = () => {
      const t = (store.getSnapshot().state.terms || []).find((x) => x.sid === activeSid);
      if (t && sessionFresh(t))
        void store.action("session_seen", { sid: activeSid }).catch(() => undefined);
    };
    check();
    return store.subscribe(check);
  }, [store, activeSid]);

  // Ctrl+` 收合／展開終端（有對話框開著時不動）；⌘K（Ctrl+K）找 repo——
  // 焦點在終端裡時 Ctrl+K 留給 shell（刪到行尾），⌘K 照常
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (document.querySelector("dialog[open]")) return;
      if (e.ctrlKey && !e.shiftKey && e.key === "`") {
        e.preventDefault();
        setTermCollapsed((c) => !c);
      } else if (
        (e.metaKey || e.ctrlKey) &&
        !e.shiftKey &&
        !e.altKey &&
        e.key.toLowerCase() === "k" &&
        !(e.ctrlKey && !e.metaKey && (e.target as Element | null)?.closest?.(".xterm"))
      ) {
        e.preventDefault();
        setPaletteInit("");
        setPaletteOpen((o) => !o);
      } else if (
        // ⌘1～⌘9 切到第 n 個牌組（側欄的順序）、⌘0 回所有 repo；Windows 是 Ctrl，焦點在終端時留給終端
        (e.metaKey || e.ctrlKey) &&
        !e.shiftKey &&
        !e.altKey &&
        /^[0-9]$/.test(e.key) &&
        !(e.ctrlKey && !e.metaKey && (e.target as Element | null)?.closest?.(".xterm"))
      ) {
        const groups = store.getSnapshot().state.groups;
        const g = e.key === "0" ? null : groups[Number(e.key) - 1];
        if (g === undefined) return;
        e.preventDefault();
        store.setScope({ group: g ? g.name : null, repos: null });
      } else if (
        // ⌘⇧F（Windows：Ctrl+Shift+F，終端裡留給終端搜尋）＝搜檔案內容
        (e.metaKey || e.ctrlKey) &&
        e.shiftKey &&
        !e.altKey &&
        e.key.toLowerCase() === "f" &&
        !(e.ctrlKey && !e.metaKey && (e.target as Element | null)?.closest?.(".xterm"))
      ) {
        e.preventDefault();
        setPaletteInit("%");
        setPaletteOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [store]);

  return (
    <StoreContext.Provider value={store}>
      <UiContext.Provider value={ui}>
        <UiStateContext.Provider value={uiState}>
          <TopBar
            onHome={() => {
              store.setScope({ group: null, repos: null });
              setTab("inbox");
            }}
          />
          <Sidebar />
          <main id="main" className={files.length ? "withtabs" : undefined}>
            {files.length ? (
              <FileTabs
                files={files}
                active={shownFile}
                dirty={dirtyFiles}
                onSelect={setActiveFile}
                onClose={closeFile}
              />
            ) : null}
            <Workspace
              tab={tab}
              composerOpen={composerOpen}
              composerFocusSeq={composerFocusSeq}
              notesFocusSeq={notesFocusSeq}
              closeComposer={() => setComposerOpen(false)}
              hidden={!!shownFile}
            />
            {files.map((f) => (
              <FileViewer
                key={fileKey(f)}
                file={f}
                hidden={fileKey(f) !== shownFile}
                onDirty={(d) => setFileDirty(fileKey(f), d)}
                picked={picks.some((p) => p.repo === f.repo && p.path === f.path)}
              />
            ))}
          </main>
          <TerminalPanel
            activeSid={activeSid}
            setActiveSid={setActiveSid}
            collapsed={termCollapsed}
            setCollapsed={setTermCollapsed}
            selectSeq={selectSeq}
            pasteRef={pasteRef}
          />
          {panelRepo ? <RepoPanel id={panelRepo} tab={panelTab} onTab={choosePanelTab} /> : null}
          <StatusBar />
          <Toast />
          <PickerDialog
            open={picker.open}
            group={picker.group}
            seq={picker.seq}
            onClose={closePicker}
          />
          {paletteOpen ? (
            <Palette key={paletteInit} initial={paletteInit} onClose={closePalette} />
          ) : null}
          {picks.length ? <PickTray /> : null}
          {menu ? <RepoMenu id={menu.id} x={menu.x} y={menu.y} onClose={closeMenu} /> : null}
          <DetailDialog
            spec={detail}
            error={detailError}
            setError={setDetailError}
            onClose={closeDetail}
          />
        </UiStateContext.Provider>
      </UiContext.Provider>
    </StoreContext.Provider>
  );
}
