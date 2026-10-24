import type { ScreenFileDetail, ScreenReport } from "../api/types";
import type { PanelTab, Tab } from "./context";
import type { WorkbenchStore } from "./store";

/** 停多久才回報（捲動、切換停下來才送一次）。 */
const DELAY = 500;

interface Base {
  file: { repo: string; path: string } | null;
  tab: Tab;
  panel: { repo: string; tab: PanelTab } | null;
}

/**
 * 把主區在看什麼回報給引擎（`screen` action；agent 被問「根據畫面」才讀，見 `ctx_core::screen`）。
 * App 給大範圍（開哪個檔、工作區分頁、右側欄），正在看的檔案分頁給量細節的函式（看法、看得到的行）——
 * 停下來才量、才送；跟上次一樣不送。回報失敗不打擾使用者（下次變動再送）。
 */
export class ScreenReporter {
  private base: Base | null = null;
  private detail: { repo: string; path: string; measure: () => ScreenFileDetail } | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private last = "";

  constructor(private store: WorkbenchStore) {}

  setBase(base: Base) {
    this.base = base;
    this.schedule();
  }

  /** 正在看的檔案分頁登記怎麼量自己；分頁收起時給 null。 */
  setDetail(repo: string, path: string, measure: (() => ScreenFileDetail) | null) {
    if (measure) this.detail = { repo, path, measure };
    else if (this.detail?.repo === repo && this.detail.path === path) this.detail = null;
    this.schedule();
  }

  /** 有東西變了（捲動、範圍）：等停下來再量。 */
  schedule = () => {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.send(), DELAY);
  };

  stop() {
    clearTimeout(this.timer);
  }

  private send() {
    const b = this.base;
    if (!b) return;
    const d = this.detail;
    const view: ScreenReport["view"] = b.file
      ? {
          kind: "file",
          ...b.file,
          ...(d && d.repo === b.file.repo && d.path === b.file.path
            ? d.measure()
            : { mode: "view" }),
        }
      : { kind: "workspace", tab: b.tab };
    const group = this.store.getSnapshot().scope.group;
    const r: ScreenReport = {
      view,
      ...(group ? { group } : {}),
      ...(b.panel ? { panel: b.panel } : {}),
    };
    const key = JSON.stringify(r);
    if (key === this.last) return;
    this.last = key;
    void this.store.action("screen", r).catch(() => {
      this.last = "";
    });
  }
}
