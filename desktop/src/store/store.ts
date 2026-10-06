/**
 * 工作台的畫面狀態：上次成功取得的 state／scan、目前範圍、連線錯誤、執行中的動作。
 * 只管「拿資料、記住、通知畫面」；資料本身全由後端算好。
 *
 * 失敗時不清掉上次資料（斷線保留畫面）；掃描帶版本號，舊回應不會蓋掉新範圍。
 */
import type { Backend } from "../api/backend";
import type {
  ActionName,
  ActionPayload,
  ActionResult,
  RepoState,
  ScanResult,
  WorkbenchState,
} from "../api/types";
import { load, save } from "../lib/storage";
import { ALL_SCOPE, hhmm, isValidScope, scopePayload, scopeRepos, type Scope } from "../lib/view";

export const EMPTY_STATE: WorkbenchState = {
  groups: [],
  repos: [],
  tiers: {},
  tags: {},
  tag_vocab: [],
  relations: [],
  rel_kinds: {},
  unread: [],
  loops: [],
  cards: [],
  recent: [],
  stats: {
    collisions: 0,
    collisions_with_outcome: 0,
    hit_rate: null,
    spawned: 0,
    alive: 0,
    survival_rate: null,
  },
  scope: { group: null, repos: null, members: [] },
  terms: [],
  agents: [],
};

export const EMPTY_SCAN: ScanResult = {
  group: "",
  states: [],
  question_cards: [],
  quiet: 0,
  audit: [],
  recent_commits: [],
  summary: null,
  session_tasks: [],
};

export interface Snapshot {
  state: WorkbenchState;
  scan: ScanResult;
  scope: Scope;
  stateReady: boolean;
  scanReady: boolean;
  stateError: string;
  scanError: string;
  scanLoading: boolean;
  scanAt: string;
  lastStates: Record<string, RepoState>;
  /** 上次成功採集的登記檢查（換範圍時不清，跟舊版一樣）。null＝還沒成功過。 */
  audit: string[] | null;
  busy: ReadonlySet<string>;
  /** 卡片上就地顯示的動作錯誤（卡片 key → 訊息）。 */
  cardErrors: Readonly<Record<string, string>>;
  toast: { text: string; seq: number };
}

export const STATE_POLL_MS = 5000;
export const SCAN_POLL_MS = 120000;

export class WorkbenchStore {
  private snap: Snapshot;
  private listeners = new Set<() => void>();
  private scanVersion = 0;
  private stateFlight: Promise<boolean> | null = null;
  private timers: ReturnType<typeof setInterval>[] = [];
  private unsubscribeChanged: (() => void) | null = null;
  readonly backend: Backend;

  constructor(backend: Backend, initialScope?: Scope) {
    this.backend = backend;
    let scope = initialScope ?? load<Scope>("scope", ALL_SCOPE);
    if (!isValidScope(scope)) scope = ALL_SCOPE;
    scope = { group: scope.group ?? null, repos: scope.repos ?? null };
    this.snap = {
      state: EMPTY_STATE,
      scan: EMPTY_SCAN,
      scope,
      stateReady: false,
      scanReady: false,
      stateError: "",
      scanError: "",
      scanLoading: false,
      scanAt: "",
      lastStates: {},
      audit: null,
      busy: new Set(),
      cardErrors: {},
      toast: { text: "", seq: 0 },
    };
  }

  // ── useSyncExternalStore ────────────────────────────────────────
  subscribe = (fn: () => void) => {
    this.listeners.add(fn);
    return () => void this.listeners.delete(fn);
  };
  getSnapshot = () => this.snap;

  private set(patch: Partial<Snapshot>) {
    this.snap = { ...this.snap, ...patch };
    this.listeners.forEach((fn) => fn());
  }

  get scopeRepos(): string[] {
    return scopeRepos(this.snap.scope, this.snap.state);
  }
  get scopePayload() {
    return scopePayload(this.snap.scope, this.snap.state);
  }
  private scopeKey() {
    return JSON.stringify(this.scopePayload);
  }
  /** 給 get_state 的範圍：原樣送（引擎自己濾掉沒登記的 repo），不用還沒載入的 state 先過濾。 */
  private stateScope(scope: Scope = this.snap.scope) {
    return Array.isArray(scope.repos)
      ? { repos: scope.repos }
      : scope.group
        ? { group: scope.group }
        : {};
  }

  toast(text: string) {
    this.set({ toast: { text, seq: this.snap.toast.seq + 1 } });
  }

  setBusy(key: string, on: boolean) {
    const busy = new Set(this.snap.busy);
    if (on) busy.add(key);
    else busy.delete(key);
    this.set({ busy });
  }

  setCardError(key: string, message: string) {
    const cardErrors = { ...this.snap.cardErrors };
    if (message) cardErrors[key] = message;
    else delete cardErrors[key];
    this.set({ cardErrors });
  }

  // ── 資料 ────────────────────────────────────────────────────────

  /** 同時只跑一個；失敗保留上次資料、把錯誤放 stateError。範圍在途中換了就再抓一次（lane 是照範圍算的）。 */
  refreshState = (): Promise<boolean> => {
    if (this.stateFlight) return this.stateFlight;
    this.stateFlight = (async () => {
      try {
        const asked = JSON.stringify(this.stateScope());
        const next = await this.backend.getState(this.stateScope());
        if (asked !== JSON.stringify(this.stateScope()))
          queueMicrotask(() => void this.refreshState());
        const { scope } = this.snap;
        const gone =
          (scope.group && !next.groups.some((g) => g.name === scope.group)) ||
          (Array.isArray(scope.repos) && !scope.repos.some((r) => next.repos.includes(r)));
        this.set({ state: next, stateReady: true, stateError: "" });
        if (gone) {
          save("scope", ALL_SCOPE);
          this.scanVersion++;
          this.set({ scope: ALL_SCOPE, scanReady: false });
          queueMicrotask(() => {
            void this.refreshState();
            void this.rescan();
          });
          this.toast("原工作範圍已不存在，已返回所有 repo");
        }
        return true;
      } catch (e) {
        this.set({ stateError: (e as Error).message });
        return false;
      } finally {
        this.stateFlight = null;
      }
    })();
    return this.stateFlight;
  };

  rescan = async (): Promise<void> => {
    const version = ++this.scanVersion;
    const key = this.scopeKey();
    this.set({ scanLoading: true });
    if (!this.snap.stateReady) {
      await this.refreshState();
      if (!this.snap.stateReady) {
        this.set({ scanLoading: false });
        return;
      }
    }
    if (!this.scopeRepos.length) {
      this.set({ scanLoading: false, scanReady: true, scanAt: "" });
      return;
    }
    try {
      const result = await this.backend.getScan(this.scopePayload);
      if (version !== this.scanVersion || key !== this.scopeKey()) return;
      const lastStates = { ...this.snap.lastStates };
      for (const s of result.states) lastStates[s.id] = s;
      this.set({
        scan: result,
        scanReady: true,
        scanError: "",
        scanAt: hhmm(),
        lastStates,
        audit: result.audit,
      });
    } catch (e) {
      if (version === this.scanVersion) this.set({ scanError: (e as Error).message });
    } finally {
      if (version === this.scanVersion) this.set({ scanLoading: false });
    }
  };

  /** 換範圍：記住、清掉舊範圍的掃描結果、重新採集。 */
  setScope = (next: Scope) => {
    save("scope", next);
    this.scanVersion++;
    this.set({ scope: next, scanReady: false, scanError: "", scan: EMPTY_SCAN });
    void this.refreshState();
    void this.rescan();
  };

  /** 「更新狀態」按鈕。 */
  refreshAll = async () => {
    await this.refreshState();
    await this.rescan();
  };

  action<N extends ActionName>(name: N, payload: ActionPayload<N>): Promise<ActionResult<N>> {
    return this.backend.action(name, payload);
  }

  // ── 生命週期 ────────────────────────────────────────────────────

  /** 首次載入＋輪詢（state 5 秒、scan 2 分鐘）。視窗隱藏時不輪詢 state，回來時補一次。 */
  start() {
    void this.refreshAll();
    const hidden = () => typeof document !== "undefined" && document.visibilityState === "hidden";
    this.timers.push(
      setInterval(() => {
        if (!hidden()) void this.refreshState();
      }, STATE_POLL_MS),
      setInterval(() => {
        if (!this.snap.scanLoading && !hidden()) void this.rescan();
      }, SCAN_POLL_MS),
    );
    if (typeof document !== "undefined")
      document.addEventListener("visibilitychange", this.onVisible);
    this.unsubscribeChanged = this.backend.onChanged?.(() => void this.refreshState()) ?? null;
  }

  stop() {
    this.timers.forEach(clearInterval);
    this.timers = [];
    if (typeof document !== "undefined")
      document.removeEventListener("visibilitychange", this.onVisible);
    this.unsubscribeChanged?.();
    this.unsubscribeChanged = null;
  }

  private onVisible = () => {
    if (document.visibilityState === "visible") void this.refreshState();
  };
}
