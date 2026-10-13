import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { GroupInfo } from "./api/types";
import { StatusBar, Toast, TopBar } from "./components/Chrome";
import { Resizer, useSavedWidths } from "./components/Resizer";
import { Sidebar } from "./components/Sidebar";
import { SessionForm } from "./components/sessions/SessionDialogs";
import { TerminalPanel } from "./components/sessions/TerminalPanel";
import { Workspace } from "./components/Workspace";
import { RepoMenu } from "./components/deep/RepoMenu";
import { RepoPanel } from "./components/deep/RepoPanel";
import { FileTabs } from "./components/viewer/FileTabs";
import { FileViewer, fileKey } from "./components/viewer/FileViewer";
import { useFileTabs } from "./components/viewer/useFileTabs";
import { Onboarding } from "./dialogs/misc";
import { DetailDialog } from "./dialogs/Modal";
import { PickerDialog } from "./dialogs/PickerDialog";
import { load, save } from "./lib/storage";
import { sessionPayload } from "./lib/view";
import { StoreContext, UiContext, type DetailSpec, type Tab, type Ui } from "./store/context";
import type { WorkbenchStore } from "./store/store";

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
  const [composerOpen, setComposerOpen] = useState(() => !!load<string>("draft", ""));
  const [composerFocusSeq, setComposerFocusSeq] = useState(0);
  const setDraft = useCallback((v: string) => {
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

  // ── 右側欄（點 repo）＋右鍵選單 ──
  const [panelRepo, setPanelRepo] = useState<string | null>(() =>
    load<string | null>("panelRepo", null),
  );
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const setPanel = useCallback((id: string | null) => {
    setPanelRepo(id);
    save("panelRepo", id);
  }, []);
  const closeMenu = useCallback(() => setMenu(null), []);
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
  } = useFileTabs(toast);

  // ── 終端 ──
  const [activeSid, setActiveSid] = useState<string | null>(null);
  const [termCollapsed, setTermCollapsed] = useState(true);
  const [selectSeq, setSelectSeq] = useState(0);

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
          title: "開啟 Agent 會話",
          eyebrow: "NEW SESSION",
          content: <SessionForm payload={payload} task={String(extra.task ?? "")} />,
        });
      },
      openOnboarding() {
        openDetail({ title: "建立第一個工作範圍", eyebrow: "FIRST RUN", content: <Onboarding /> });
      },
      panelRepo,
      openRepo: (id) => setPanel(panelRepo === id ? null : id),
      closeRepo: () => setPanel(null),
      repoMenu: (id, x, y) => setMenu({ id, x, y }),
      openFile,
      activeFile: shownFileObj,
      showComposer,
      draft,
      setDraft,
      showTab: setTab,
      selectTerm(sid) {
        setActiveSid(sid);
        setTermCollapsed(false);
        setSelectSeq((n) => n + 1);
      },
      toggleTerm: () => setTermCollapsed((c) => !c),
      collapseTerm: () => setTermCollapsed(true),
      termCollapsed,
      activeSid,
      openGroups,
      toggleGroup: (key) => updateOpen((s) => (s.has(key) ? s.delete(key) : s.add(key))),
      renameOpenGroup: (from, to) =>
        updateOpen((s) => {
          if (s.delete(from)) s.add(to);
        }),
      forgetOpenGroup: (name) => updateOpen((s) => s.delete(name)),
    }),
    [
      store,
      openDetail,
      closeDetail,
      openPicker,
      showComposer,
      draft,
      setDraft,
      panelRepo,
      setPanel,
      openFile,
      shownFileObj,
      termCollapsed,
      activeSid,
      openGroups,
      updateOpen,
    ],
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

  // Ctrl+` 收合／展開終端（有對話框開著時不動）
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "`" && !document.querySelector("dialog[open]")) {
        e.preventDefault();
        setTermCollapsed((c) => !c);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <StoreContext.Provider value={store}>
      <UiContext.Provider value={ui}>
        <TopBar
          onHome={() => {
            store.setScope({ group: null, repos: null });
            setTab("inbox");
          }}
        />
        <Sidebar />
        <Resizer side="left" />
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
            closeComposer={() => setComposerOpen(false)}
            hidden={!!shownFile}
          />
          {files.map((f) => (
            <FileViewer
              key={fileKey(f)}
              file={f}
              hidden={fileKey(f) !== shownFile}
              onDirty={(d) => setFileDirty(fileKey(f), d)}
            />
          ))}
        </main>
        <TerminalPanel
          activeSid={activeSid}
          setActiveSid={setActiveSid}
          collapsed={termCollapsed}
          setCollapsed={setTermCollapsed}
          selectSeq={selectSeq}
        />
        {panelRepo ? <RepoPanel id={panelRepo} /> : null}
        {panelRepo ? <Resizer side="right" /> : null}
        <StatusBar />
        <Toast />
        <PickerDialog
          open={picker.open}
          group={picker.group}
          seq={picker.seq}
          onClose={closePicker}
        />
        {menu ? <RepoMenu id={menu.id} x={menu.x} y={menu.y} onClose={closeMenu} /> : null}
        <DetailDialog
          spec={detail}
          error={detailError}
          setError={setDetailError}
          onClose={closeDetail}
        />
      </UiContext.Provider>
    </StoreContext.Provider>
  );
}
