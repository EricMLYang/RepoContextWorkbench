import type {
  ActionName,
  ActionPayload,
  ActionResult,
  ScanParams,
  ScanResult,
  WorkbenchState,
} from "./types";

/**
 * 會話（PTY）的一條連線。關掉連線＝只是不再看，不殺程序（殺程序走 action `term_kill`）。
 * 先註冊 handler 再等資料：實作保證 replay（程序至今的輸出）在第一個 onData 送到。
 */
export interface TermStream {
  /** PTY 輸出（原始位元組，可能切在 UTF-8 中間；交給 xterm 自己接）。 */
  onData(cb: (bytes: Uint8Array) => void): void;
  /** 連上了（之後 write/resize 才有作用）。 */
  onOpen(cb: () => void): void;
  /** 連線斷了（網路、server 關、或程序已移除）。之後要看需重新 openTerminal。 */
  onClose(cb: () => void): void;
  /** 鍵盤輸入（xterm onData 給的字串）。 */
  write(data: string): void;
  resize(cols: number, rows: number): void;
  close(): void;
  readonly connected: boolean;
}

/** 前端唯一看得到的後端。HTTP（Python 版、對拍）與 Tauri（Rust 版）各一個實作。 */
export interface Backend {
  readonly kind: "http" | "tauri";
  /** 輕量狀態（純檔案讀，供 5 秒輪詢）。範圍＝畫面目前選的（引擎據此算卡片／事件／關係的 lane）。 */
  getState(scope?: ScanParams): Promise<WorkbenchState>;
  /** 跑採集（會打 git）。只在開頁／切範圍／手動更新／每 2 分鐘時呼叫。 */
  getScan(params: ScanParams): Promise<ScanResult>;
  /** 卡片按鈕與各表單的寫入動作。失敗一律 reject Error（message 是給人看的中文）。 */
  action<N extends ActionName>(name: N, payload: ActionPayload<N>): Promise<ActionResult<N>>;
  openTerminal(sid: string): TermStream;
  /**
   * 選配（Tauri 才有）：檔案樹展開中的資料夾（`{repo id: [相對路徑…]}`，`""`＝根）。
   * 殼只監看這些，有變動用 onRepoChanged 通知。空物件＝不監看。
   */
  watchTree?(repos: Record<string, string[]>): Promise<void>;
  /**
   * 選配：展開中資料夾裡的檔案有變（新增、刪除、改名、改內容）；回傳取消訂閱。
   * `rescan`＝OS 漏了事件（agent 一次改太多檔）：不知道哪裡變了，`paths` 是所有展開中的資料夾，全部重讀。
   */
  onRepoChanged?(cb: (id: string, paths: string[], rescan: boolean) => void): () => void;
  /** 選配：原生「選擇資料夾」對話框（Tauri 才有）；取消回 null。 */
  pickFolder?(): Promise<string | null>;
  /**
   * 選配：後端主動通知「資料變了」（Tauri 事件 `workbench://changed`）。
   * 有這個就不用靠 5 秒輪詢才看到變化；回傳取消訂閱。
   */
  onChanged?(cb: () => void): () => void;
  /** 選配（Tauri 才有）：選單列「記一筆…」——前端切到筆記分頁、游標放進輸入框。回傳取消訂閱。 */
  onCapture?(cb: () => void): () => void;
  /** 選配（Tauri 才有）：選單列「等你的會話」被點——前端切到第一個在等你的會話。回傳取消訂閱。 */
  onNextWaiting?(cb: () => void): () => void;
}

/** 給人看的連線錯誤訊息（兩個實作共用，與 Python 版前端逐字一致）。 */
export const MSG_UNREACHABLE = "無法連接工作台。請確認本機服務仍在執行，再重試。";
export const MSG_BAD_RESPONSE = "工作台回應不完整，請稍後重試。";
export const MSG_FAILED = "操作未完成，請重試。";
