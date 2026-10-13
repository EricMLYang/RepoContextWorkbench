import { createContext, useContext, useSyncExternalStore, type ReactNode } from "react";
import type { GroupInfo } from "../api/types";
import type { Snapshot, WorkbenchStore } from "./store";

export const StoreContext = createContext<WorkbenchStore | null>(null);

export function useStore(): WorkbenchStore {
  const s = useContext(StoreContext);
  if (!s) throw new Error("WorkbenchStore 未提供");
  return s;
}

/** 目前畫面資料（上次成功取得的 state／scan＋連線狀態）。 */
export function useSnapshot(): Snapshot {
  const store = useStore();
  return useSyncExternalStore(store.subscribe, store.getSnapshot);
}

export type Tab = "inbox" | "notes" | "repos" | "files" | "activity" | "relations";

export interface DetailSpec {
  title: string;
  eyebrow?: string;
  /** 對話框內容（自己畫 body 與 actions，用 <DetailLayout>）。 */
  content: ReactNode;
}

/**
 * 跨元件的畫面操作（開對話框、開會話、切分頁、選終端…）。
 * 整個 App 生命週期裡同一個物件，不會變——只用操作的元件不會因為畫面狀態變動而重畫。
 */
export interface Ui {
  openDetail(spec: DetailSpec): void;
  closeDetail(): void;
  setDetailError(msg: string): void;
  openPicker(group?: GroupInfo | null): void;
  openSession(extra?: Record<string, unknown>): void;
  openOnboarding(): void;
  /** 點 repo：在右側欄打開（同一個再點＝收起）。 */
  openRepo(id: string): void;
  /** 在右側欄打開這個 repo（已開著就不動；不像 openRepo 會收起）。 */
  showRepo(id: string): void;
  closeRepo(): void;
  /** ⌘K：找 repo 或牌組。 */
  openPalette(): void;
  /** 在主區開一個檔案分頁（已開就切過去）。 */
  openFile(repo: string, path: string): void;
  /** repo 的右鍵選單（x、y 是視窗座標）。 */
  repoMenu(id: string, x: number, y: number): void;
  showComposer(): void;
  /** 當下的草稿（點按鈕時讀；要跟著畫面變就用 useUiState().draft）。 */
  getDraft(): string;
  setDraft(v: string): void;
  showTab(tab: Tab): void;
  selectTerm(sid: string): void;
  toggleTerm(): void;
  collapseTerm(): void;
  toggleGroup(key: string): void;
  renameOpenGroup(from: string, to: string): void;
  forgetOpenGroup(name: string): void;
}

/** 會變的畫面狀態；只有要跟著它重畫的元件才訂閱。 */
export interface UiState {
  /** 右側欄目前顯示的 repo（null＝收起）。 */
  panelRepo: string | null;
  /** 主區正在看的檔案（檔案樹據此標出目前這一列）；看工作區時 null。 */
  activeFile: { repo: string; path: string } | null;
  draft: string;
  termCollapsed: boolean;
  activeSid: string | null;
  /** 側欄展開中的牌組（key＝組名，"*"＝所有 repo，"__adhoc__"＝臨時組）。 */
  openGroups: ReadonlySet<string>;
}

export const UiContext = createContext<Ui | null>(null);
export const UiStateContext = createContext<UiState | null>(null);

export function useUi(): Ui {
  const u = useContext(UiContext);
  if (!u) throw new Error("Ui 未提供");
  return u;
}

export function useUiState(): UiState {
  const u = useContext(UiStateContext);
  if (!u) throw new Error("UiState 未提供");
  return u;
}
