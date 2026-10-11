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

export type Tab = "inbox" | "repos" | "files" | "activity" | "relations";

export interface DetailSpec {
  title: string;
  eyebrow?: string;
  /** 對話框內容（自己畫 body 與 actions，用 <DetailLayout>）。 */
  content: ReactNode;
}

/** 跨元件的畫面操作（開對話框、開會話、切分頁、選終端…）。 */
export interface Ui {
  openDetail(spec: DetailSpec): void;
  closeDetail(): void;
  setDetailError(msg: string): void;
  openPicker(group?: GroupInfo | null): void;
  openSession(extra?: Record<string, unknown>): void;
  openOnboarding(): void;
  /** 右側欄目前顯示的 repo（null＝收起）。 */
  panelRepo: string | null;
  /** 點 repo：在右側欄打開（同一個再點＝收起）。 */
  openRepo(id: string): void;
  closeRepo(): void;
  /** repo 的右鍵選單（x、y 是視窗座標）。 */
  repoMenu(id: string, x: number, y: number): void;
  showComposer(): void;
  draft: string;
  setDraft(v: string): void;
  showTab(tab: Tab): void;
  selectTerm(sid: string): void;
  toggleTerm(): void;
  collapseTerm(): void;
  termCollapsed: boolean;
  activeSid: string | null;
  /** 側欄展開中的牌組（key＝組名，"*"＝所有 repo，"__adhoc__"＝臨時組）。 */
  openGroups: ReadonlySet<string>;
  toggleGroup(key: string): void;
  renameOpenGroup(from: string, to: string): void;
  forgetOpenGroup(name: string): void;
}

export const UiContext = createContext<Ui | null>(null);

export function useUi(): Ui {
  const u = useContext(UiContext);
  if (!u) throw new Error("Ui 未提供");
  return u;
}
