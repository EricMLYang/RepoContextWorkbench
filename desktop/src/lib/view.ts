/**
 * 畫面用的純函式：只做「怎麼顯示」——照 lane 分段、搜尋、按鈕文案、主次動作、狀態文字。
 * 業務判斷（卡片有哪些動作、範圍歸屬與 lane、摘要內容、任務文案）全由後端給；這裡不新增任何判斷。
 * 2026-10-06：範圍篩選（卡片／事件／關係）、relationTask、taskEntries 已移進引擎（workbench::state）。
 */
import type {
  CardAction,
  CardKind,
  InboxCard,
  Relation,
  RelationKind,
  RepoState,
  ScanParams,
  ScanResult,
  TermInfo,
  WorkbenchState,
} from "../api/types";

export interface Scope {
  group: string | null;
  repos: string[] | null;
}

export const ALL_SCOPE: Scope = { group: null, repos: null };

export const TIERNAME: Record<string, string> = {
  active: "進行中",
  paused: "已暫停",
  dormant: "休眠",
  archived: "已封存",
};

export const KINDNAME: Record<CardKind, string> = {
  interrupt: "需要立即處理",
  question: "需要你判斷",
  collision: "分析結果",
  event: "其他未讀",
  loop: "待跟進",
  pending: "處理中",
  fresh: "相關新料",
  reopen: "該重看的判斷",
  retro: "判斷回顧",
};

export function isValidScope(s: unknown): s is Scope {
  if (!s || typeof s !== "object") return false;
  const o = s as Scope;
  return o.repos === null || o.repos === undefined || Array.isArray(o.repos);
}

export function scopeRepos(
  scope: Scope,
  state: Pick<WorkbenchState, "repos" | "groups">,
): string[] {
  if (Array.isArray(scope.repos)) return scope.repos.filter((r) => state.repos.includes(r));
  if (scope.group) return state.groups.find((g) => g.name === scope.group)?.members.slice() || [];
  return state.repos.slice();
}

/** 給後端的範圍 payload（collide／brief／term_create）。 */
export function scopePayload(
  scope: Scope,
  state: Pick<WorkbenchState, "repos" | "groups">,
): ScanParams {
  if (Array.isArray(scope.repos)) return { repos: scopeRepos(scope, state) };
  return scope.group ? { group: scope.group } : {};
}

export function scopeLabel(scope: Scope): string {
  return Array.isArray(scope.repos) ? "臨時工作組" : scope.group || "所有 repo";
}

export function scopeKindLabel(scope: Scope): string {
  return scope.group ? "常用牌組" : Array.isArray(scope.repos) ? "臨時範圍" : "跨組總覽";
}

/** 搜尋只比對畫面上看得到的文字：字串與數字的值（不含鍵名、不含動作的 payload）。 */
function visibleText(v: unknown, out: string[]): string[] {
  if (typeof v === "string" || typeof v === "number") out.push(String(v));
  else if (Array.isArray(v)) v.forEach((x) => visibleText(x, out));
  else if (v && typeof v === "object")
    for (const [k, x] of Object.entries(v)) if (k !== "payload") visibleText(x, out);
  return out;
}

export function matchesSearch(item: unknown, query: string): boolean {
  return !query || visibleText(item, []).join("\n").toLocaleLowerCase().includes(query);
}

/** 範圍內的卡：掃描的問句卡（本來就是這個範圍採集的）＋ state 卡中引擎標 lane＝scope 的。 */
export function scopedCards(
  state: Pick<WorkbenchState, "cards">,
  scan: Pick<ScanResult, "question_cards">,
): InboxCard[] {
  return (scan.question_cards as InboxCard[]).concat(state.cards.filter((c) => c.lane === "scope"));
}

export interface CardSection {
  key: string;
  title: string;
  note?: string;
  cards: InboxCard[];
  collapsed?: boolean;
}

/** 收件匣分段：全域警示 → 各類型 → 全域與未分類（收合）。歸屬照引擎給的 lane。 */
export function cardSections(
  state: Pick<WorkbenchState, "cards">,
  scan: Pick<ScanResult, "question_cards">,
  query: string,
  filter: string,
): CardSection[] {
  const local = scopedCards(state, scan);
  const extras = state.cards.filter((c) => c.lane === "global");
  const accept = (c: InboxCard) =>
    matchesSearch(c, query) && (filter === "all" || c.kind === filter);
  const sections: CardSection[] = [];
  const globalWarnings = extras.filter((c) => c.kind === "interrupt" && accept(c));
  if (globalWarnings.length)
    sections.push({
      key: "global",
      title: "全域警示",
      note: "跨組顯示，操作只影響卡片標示的來源。",
      cards: globalWarnings,
    });
  for (const kind of [
    "interrupt",
    "question",
    "reopen",
    "collision",
    "loop",
    "fresh",
    "retro",
    "event",
    "pending",
  ] as const) {
    const cards = local.filter((c) => c.kind === kind && accept(c));
    if (cards.length) sections.push({ key: kind, title: KINDNAME[kind], cards });
  }
  const unclassified = extras.filter((c) => c.kind !== "interrupt" && accept(c));
  if (unclassified.length)
    sections.push({
      key: "unclassified",
      title: "全域與未分類",
      note: "這些事件未限定於目前牌組。",
      cards: unclassified,
      collapsed: true,
    });
  return sections;
}

export function actionLabel(a: CardAction): string {
  if (a.action === "route_collision") {
    const dest = String(a.payload.dest ?? "");
    return dest === "incubator" ? "移入孵化區" : "存入 " + dest.replace(/^(repo|group):/, "");
  }
  return (
    (
      {
        route_pick: "選擇儲存位置",
        ignore: "忽略並記錄",
        collide_prefill: "分析這個問題",
        close_loop: "完成跟進",
        collide_rerun: "重新分析",
      } as Record<string, string>
    )[a.action] || a.label
  );
}

/** 主要動作：引擎標的 `primary`（規則在 `state.rs` 的 `mark_primary`）。 */
export function primaryAction(c: InboxCard): CardAction | undefined {
  return (c.actions || []).find((a) => a.primary);
}

export function cardDescription(c: InboxCard): string | undefined {
  if (c.kind === "pending") return undefined;
  const j = "judgement" in c ? c.judgement : undefined;
  return j?.["理由"] || c.lines?.[0];
}

export function cardDetailText(c: InboxCard): string {
  return "judgement" in c && c.judgement
    ? Object.entries(c.judgement)
        .map(([k, v]) => k + "：" + v)
        .join("\n\n")
    : (c.lines || []).join("\n");
}

/** 成功 toast：「〈動作〉：完成 · 檔名／下次提醒／日期」。 */
export function actionToast(a: CardAction, r: Record<string, unknown>): string {
  const file = typeof r.file === "string" ? r.file : "";
  return (
    actionLabel(a) +
    "：完成" +
    (file
      ? " · " + file.split(/[\\/]/).pop()
      : r.until
        ? " · 下次提醒 " + r.until
        : r.due
          ? " · " + r.due
          : "")
  );
}

// ── 空狀態（四種＋篩選無結果）────────────────────────────────────────

export interface EmptyState {
  title: string;
  text: string;
  onboard: boolean;
}

export function emptyState(v: {
  stateReady: boolean;
  stateError: string;
  repoCount: number;
  scanReady: boolean;
  scanError: string;
  filtered: boolean;
}): EmptyState {
  if (!v.stateReady)
    return v.stateError
      ? { title: "無法載入工作空間", text: "請確認本機服務，再按更新狀態。", onboard: false }
      : { title: "正在載入工作空間", text: "正在取得 repo 與事件。", onboard: false };
  if (!v.repoCount)
    return {
      title: "建立你的第一個工作範圍",
      text: "選一個放 repo 的資料夾，工作台會掃出裡面的 git repo，讓你直接建第一組；也可以用 registry add／registry scan 從命令列登記。",
      onboard: true,
    };
  if (!v.scanReady || v.scanError)
    return v.scanError
      ? {
          title: "無法確認目前狀態",
          text: "更新成功後，才能確認是否有待處理事項。",
          onboard: false,
        }
      : { title: "正在檢查這組 repo", text: "收件匣會在採集完成後更新。", onboard: false };
  if (v.filtered)
    return { title: "沒有符合條件的事項", text: "試著清除搜尋或選擇其他類型。", onboard: false };
  return {
    title: "目前沒有待處理事項",
    text: "新的分析結果與需要判斷的變化會出現在這裡。",
    onboard: false,
  };
}

// ── 連線狀態 ────────────────────────────────────────────────────────

export function connectionText(v: {
  stateError: string;
  scanError: string;
  scanLoading: boolean;
  stateReady: boolean;
}): string {
  if (v.stateError || v.scanError) return "連線或更新異常";
  if (v.scanLoading) return "正在更新…";
  return v.stateReady ? "已連接 · 本機" : "正在連接…";
}

export function bannerText(err: string): string {
  return err ? err + " 目前可能顯示上次成功取得的資料。可按「更新狀態」重試。" : "";
}

// ── 牌組卡 ──────────────────────────────────────────────────────────

export function groupMeta(members: string[], lastStates: Record<string, RepoState>): string {
  const count = members.reduce((a, r) => a + (lastStates[r]?.commits_7d || 0), 0);
  const bad = members.filter(
    (r) => lastStates[r]?.note || (lastStates[r] && !lastStates[r].exists),
  ).length;
  const full = members.length && members.every((r) => lastStates[r]?.commits_7d != null);
  return (
    `${members.length} repo${full ? " · 7 天 " + count + " commits" : " · 尚未完整採集"}` +
    (bad ? " · " + bad + " 項需檢查" : "")
  );
}

// ── Repo 表 ─────────────────────────────────────────────────────────

export const CORE_COLUMNS: [keyof RepoState, string][] = [
  ["id", "Repo"],
  ["tier", "狀態"],
  ["branch", "分支"],
  ["dirty", "未提交"],
  ["commits_7d", "7 天活動"],
  ["agents", "Agent"],
];
export const MORE_COLUMNS: [keyof RepoState, string][] = [
  ["dirty_days", "未提交天數"],
  ["last_commit_days", "距末次 commit"],
  ["ahead", "未推送"],
  ["behind", "落後"],
  ["worktrees", "Worktree"],
];

export function sortRepos(
  rows: RepoState[],
  column: keyof RepoState,
  direction: 1 | -1,
): RepoState[] {
  return rows.slice().sort((a, b) => {
    const x = a[column],
      y = b[column];
    return (
      (typeof x === "number" && typeof y === "number"
        ? x - y
        : String(x ?? "").localeCompare(String(y ?? ""))) * direction
    );
  });
}

export function repoCell(s: RepoState, key: keyof RepoState): string {
  if (key === "tier") return s.note ? "需檢查" : TIERNAME[s.tier] || s.tier;
  if (key === "agents") return (s.agents || []).join("、") || "—";
  const v = s[key];
  if (v == null) return "—";
  if (typeof v === "number")
    return (
      String(Math.round(v)) + (key === "dirty" ? " 檔" : key === "commits_7d" ? " commits" : "")
    );
  return String(v);
}

// ── 關係 ────────────────────────────────────────────────────────────

export function relationLabel(r: Relation, kinds: Record<string, RelationKind>): string {
  return kinds[r.kind]?.forward || r.kind;
}

// ── 會話 ────────────────────────────────────────────────────────────

export const STATUS_TXT: Record<string, string> = {
  working: "處理中",
  waiting: "等你回應",
  blocked: "卡住了",
  done: "已交接",
};

/** 程序活著不等於工作在推進：回報狀態只認脊椎留痕，沒有留痕就說沒有。 */
export function reportLine(s: TermInfo): string {
  if (s.kind !== "agent") return "本機 Shell，不回報";
  if (s.agent_status)
    return (
      "Agent 回報「" +
      (STATUS_TXT[s.agent_status.status] || s.agent_status.status) +
      "」" +
      s.agent_status.at.slice(11) +
      (s.agent_status.text ? "：" + s.agent_status.text : "")
    );
  if (s.report)
    return (
      "最近留痕 " +
      s.report.at +
      "（" +
      s.report.type +
      " · 來自 " +
      s.report.source +
      "）：" +
      s.report.text
    );
  return "尚無回報——這個會話期間，範圍內還沒有任何脊椎留痕";
}

/**
 * 會話在哪：所屬牌組（home_group）＋對單一 repo 開的那個 repo。
 * label：「組 › repo」「組」「repo」「臨時（3 個 repo）」「本機 Shell」。
 */
export function sessionWhere(s: TermInfo): {
  group: string | null;
  repo: string | null;
  label: string;
} {
  const group = s.home_group ?? null;
  const repo =
    s.scope && s.scope_repos?.length === 1 && s.scope_repos[0] === s.scope ? s.scope : null;
  const label =
    group && repo
      ? group + " › " + repo
      : group ||
        repo ||
        (s.scope === "臨時" && s.scope_repos?.length
          ? "臨時（" + s.scope_repos.length + " 個 repo）"
          : s.scope || "本機 Shell");
  return { group, repo, label };
}

/** 會話屬於這個牌組嗎（舊會話沒有 home_group：看 scope 是不是組名）。 */
export function sessionInGroup(s: TermInfo, group: string): boolean {
  return s.home_group ? s.home_group === group : s.scope === group;
}

export function sessionMeta(s: TermInfo): string {
  const proc = s.alive ? "程序執行中" : "程序已結束";
  const work =
    s.kind !== "agent"
      ? "Shell"
      : s.agent_status
        ? STATUS_TXT[s.agent_status.status] || s.agent_status.status
        : s.report
          ? "最近留痕 " + s.report.at.slice(11)
          : "尚無回報";
  return proc + " · " + work + " · " + sessionWhere(s).label;
}

export function sessionTooltip(s: TermInfo): string {
  return (
    s.title +
    "\n" +
    (s.task || "") +
    "\n範圍：" +
    sessionWhere(s).label +
    "\n" +
    reportLine(s) +
    "\n依據：" +
    (s.report_basis || "—")
  );
}

export type DotState = "dead" | "silent" | "needs" | "ok";

export function sessionDot(s: TermInfo): DotState[] {
  const out: DotState[] = [];
  if (!s.alive) out.push("dead");
  if (s.alive && s.kind === "agent" && !s.report && !s.agent_status) out.push("silent");
  if (s.agent_status && ["waiting", "blocked"].includes(s.agent_status.status)) out.push("needs");
  return out;
}

/** 開會話表單的 payload：範圍＋額外欄位（repo 優先、group 蓋過 repos）。 */
export function sessionPayload(
  base: ScanParams,
  extra: Record<string, unknown>,
): Record<string, unknown> {
  const payload: Record<string, unknown> = { ...base, ...extra };
  if (extra.group) delete payload.repos;
  if (extra.repo) {
    delete payload.group;
    payload.repos = [extra.repo];
  }
  return payload;
}

export function sessionTarget(payload: Record<string, unknown>): string {
  const repos = payload.repos as string[] | null | undefined;
  const home = payload.home_group as string | undefined;
  return (
    (payload.repo ? (home ? home + " › " : "") + (payload.repo as string) : "") ||
    (payload.group as string) ||
    (repos ? "臨時工作組 · " + repos.length + " 個 repo" : "所有 repo")
  );
}

export function hhmm(d = new Date()): string {
  return d.toLocaleTimeString("zh-TW", { hour12: false, hour: "2-digit", minute: "2-digit" });
}

export function parseTags(input: string): string[] {
  return [
    ...new Set(
      input
        .split(/[,，]/)
        .map((x) => x.trim())
        .filter(Boolean),
    ),
  ];
}
