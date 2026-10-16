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
  // 只有一個 repo 的臨時範圍＝就叫那個 repo（不講「組」，#73）
  if (Array.isArray(scope.repos)) return scope.repos.length === 1 ? scope.repos[0] : "臨時工作組";
  return scope.group || "所有 repo";
}

export function scopeKindLabel(scope: Scope): string {
  if (scope.group) return "常用牌組";
  if (Array.isArray(scope.repos)) return scope.repos.length === 1 ? "單一 repo" : "臨時範圍";
  return "跨組總覽";
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

/**
 * 收件匣的數字分類（只照引擎給的 kind 分，不新增判斷）：
 * loop＝待跟進；decide＝要你判斷（打斷、問題、分析結果、該重看、回顧）；unread＝其他未讀與新料；pending＝分析中。
 */
export function cardCounts(cards: InboxCard[]) {
  const n = { loop: 0, decide: 0, unread: 0, pending: 0 };
  for (const c of cards) {
    if (c.kind === "loop") n.loop++;
    else if (c.kind === "pending") n.pending++;
    else if (c.kind === "event" || c.kind === "fresh") n.unread++;
    else n.decide++;
  }
  return n;
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
        collide_prefill: "請 Agent 分析這個問題…",
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
  return err ? err + " 目前可能顯示上次成功取得的資料。" : "";
}

// ── 牌組卡 ──────────────────────────────────────────────────────────

/** 牌組卡的一行說明（UX-09：導覽列只放 repo 數與要檢查的；commit 統計放 groupActivity 給提示用）。 */
export function groupMeta(members: string[], lastStates: Record<string, RepoState>): string {
  const bad = members.filter(
    (r) => lastStates[r]?.note || (lastStates[r] && !lastStates[r].exists),
  ).length;
  return `${members.length} repo` + (bad ? " · " + bad + " 項需檢查" : "");
}

/** 牌組近 7 天的動靜（滑鼠提示用）。 */
export function groupActivity(members: string[], lastStates: Record<string, RepoState>): string {
  const full = members.length && members.every((r) => lastStates[r]?.commits_7d != null);
  if (!full) return "尚未完整採集";
  const count = members.reduce((a, r) => a + (lastStates[r]?.commits_7d || 0), 0);
  return "7 天 " + count + " commits";
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
  finished: "做完一輪",
  exited: "已結束",
};

/** 程序活著不等於工作在推進：回報狀態只認脊椎留痕，沒有留痕就說沒有。 */
export function reportLine(s: TermInfo): string {
  if (s.kind !== "agent") return "本機 Shell，不回報";
  if (s.agent_status)
    return (
      (s.agent_status.event ? "Claude Code「" : "Agent 回報「") +
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

/** 「某會話的 Terminal」（`term_create` 帶 `of` 開的 shell）：回它所屬的會話 sid，不是就 null。 */
export function shellOwner(s: TermInfo): string | null {
  return s.kind === "shell" && s.origin?.startsWith("shell-of:")
    ? s.origin.slice("shell-of:".length)
    : null;
}

/**
 * 目前選的會話所在的「Agent／Terminal」一對：選 agent 或它的 Terminal 都回同一對；
 * 選的是一般 shell（或配對的 agent 已不在）就回 null。
 */
export function sessionPair(
  terms: TermInfo[],
  sid: string | null,
): { agent: TermInfo; shell: TermInfo | null; onShell: boolean } | null {
  const cur = terms.find((s) => s.sid === sid);
  if (!cur) return null;
  const ownerSid = cur.kind === "agent" ? cur.sid : shellOwner(cur);
  const agent = terms.find((s) => s.sid === ownerSid && s.kind === "agent");
  if (!agent) return null;
  // 同一個 agent 可能留著已結束的舊 Terminal：優先挑還活著的、最新開的
  const shells = terms.filter((s) => shellOwner(s) === agent.sid);
  const shell = cur.kind === "shell" ? cur : (shells.findLast((s) => s.alive) ?? null);
  return { agent, shell, onShell: cur.kind === "shell" };
}

/** 階段那幾個字；處理中且知道在用哪個工具時帶工具與分鐘數（F-82），超過 10 分沒結束改說「可能卡住」。 */
export function statusText(s: TermInfo): string {
  if (!s.agent_status) return "";
  const base = STATUS_TXT[s.agent_status.status] || s.agent_status.status;
  const t = s.alive && s.agent_status.status === "working" ? s.tool : null;
  if (!t) return base;
  const mins = t.minutes >= 1 ? `（${t.minutes} 分）` : "";
  return t.stalled ? `可能卡住：${t.label}（${t.minutes} 分沒結束）` : `${base}：${t.label}${mins}`;
}

export function sessionMeta(s: TermInfo): string {
  const proc = s.alive ? "程序執行中" : "程序已結束";
  const work =
    s.kind !== "agent"
      ? "Shell"
      : s.agent_status
        ? statusText(s)
        : s.report
          ? "最近留痕 " + s.report.at.slice(11)
          : "尚無回報";
  return proc + " · " + work + " · " + sessionWhere(s).label;
}

/** 會話列的主標題：Claude Code 取的對話標題優先（多個會話同一個範圍時分得出誰是誰），沒有就用「組 › repo」。 */
export function sessionHeadline(s: TermInfo): string {
  return s.ai_title || s.title;
}

export function sessionTooltip(s: TermInfo): string {
  return (
    (s.ai_title ? s.ai_title + "\n" : "") +
    s.title +
    "\n" +
    (s.task || "") +
    "\n範圍：" +
    sessionWhere(s).label +
    "\n" +
    reportLine(s) +
    "\n依據：" +
    (s.report_basis || "—") +
    (s.last_reply ? "\n最後回覆：" + s.last_reply : "")
  );
}

/** 會話在等你（agent 回報等你／卡住，或 Claude Code 停住要你允許／回答）。做完一輪不算（F-80）。 */
export function sessionNeedsYou(s: TermInfo): boolean {
  return !!s.alive && !!s.agent_status && ["waiting", "blocked"].includes(s.agent_status.status);
}

/** 做完一輪、你還沒看（F-80：Claude Code 的 Stop 之後、你還沒切過去）。 */
export function sessionFresh(s: TermInfo): boolean {
  return !!s.alive && s.agent_status?.status === "finished" && !s.agent_status.seen;
}

/** 要你看一眼的：在等你，或做完沒看。 */
export function sessionWantsYou(s: TermInfo): boolean {
  return sessionNeedsYou(s) || sessionFresh(s);
}

/**
 * 這個範圍裡還活著的 agent 會話（UX-01：有就先「回到會話」，不默默再開一個）。
 * 牌組＝屬於這組的（見 sessionInGroup）；臨時範圍＝scope_repos 跟目前選的是同一批；所有 repo＝全部。
 * 排序：在等你的優先，再來是最近開的。
 */
export function scopeSessions(terms: TermInfo[], scope: Scope): TermInfo[] {
  const same = (a: string[], b: string[]) =>
    a.length === b.length && [...a].sort().join("\n") === [...b].sort().join("\n");
  const inScope = (t: TermInfo) =>
    scope.group
      ? sessionInGroup(t, scope.group)
      : Array.isArray(scope.repos)
        ? !!t.scope_repos && same(t.scope_repos, scope.repos)
        : true;
  return terms
    .filter((t) => t.kind === "agent" && t.alive && inScope(t))
    .sort(
      (a, b) =>
        Number(sessionWantsYou(b)) - Number(sessionWantsYou(a)) ||
        String(b.started ?? "").localeCompare(String(a.started ?? "")),
    );
}

/** 最該先處理的會話：卡住（要你允許）優先，再來是等你回覆，最後是做完沒看；同類取最早開始的。沒有＝null。 */
export function firstNeedingYou(terms: TermInfo[]): TermInfo | null {
  const rank = (t: TermInfo) =>
    t.agent_status?.status === "blocked" ? 0 : sessionNeedsYou(t) ? 1 : 2;
  return (
    terms
      .filter(sessionWantsYou)
      .sort(
        (a, b) =>
          rank(a) - rank(b) || String(a.started ?? "").localeCompare(String(b.started ?? "")),
      )[0] ?? null
  );
}

export type DotState = "dead" | "silent" | "busy" | "stalled" | "needs" | "fresh" | "seen" | "ok";

/**
 * 會話列的小圓點（F-80，抄 BAT 的「需要我」清單）：灰＝已結束、琥珀＝在跑（處理中，或還沒回報過）、
 * 紅＝等你（要你允許／回答）、亮綠＝做完沒看、暗綠＝做完看過了、綠＝有留痕。
 */
export function sessionDot(s: TermInfo): DotState[] {
  const out: DotState[] = [];
  if (!s.alive) out.push("dead");
  if (s.alive && s.kind === "agent" && !s.report && !s.agent_status) out.push("silent");
  if (s.alive && s.agent_status?.status === "working")
    out.push(s.tool?.stalled ? "stalled" : "busy");
  if (s.agent_status && ["waiting", "blocked"].includes(s.agent_status.status)) out.push("needs");
  if (s.alive && s.agent_status?.status === "finished")
    out.push(s.agent_status.seen ? "seen" : "fresh");
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
    (repos
      ? repos.length === 1
        ? repos[0]
        : "臨時工作組 · " + repos.length + " 個 repo"
      : "所有 repo")
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

/** 從檔案樹拖出來的檔（dataTransfer 的型別；內容是 `{repo, path, abs}` 的 JSON）。 */
export const FILE_DRAG_TYPE = "application/x-repo-context-file";

/**
 * 拖到終端的檔案要貼成什麼：agent 會話＝`@絕對路徑`（agent 會去讀）；shell＝加引號的路徑。
 * 後面補一個空白，不換行（不會被當成送出）。
 */
export function pathForTerminal(abs: string, kind: "agent" | "shell", windows = false): string {
  if (kind === "agent") return "@" + abs + " ";
  if (/^[\w@%+=:,./-]+$/.test(abs)) return abs + " ";
  return (windows ? '"' + abs + '"' : "'" + abs.replace(/'/g, "'\\''") + "'") + " ";
}

/** 拖曳資料（{@link FILE_DRAG_TYPE}）裡的絕對路徑；不是我們拖的或壞掉＝null。 */
export function draggedFileAbs(raw: string): string | null {
  if (!raw) return null;
  try {
    const abs = (JSON.parse(raw) as { abs?: unknown }).abs;
    return typeof abs === "string" && abs ? abs : null;
  } catch {
    return null;
  }
}

/** 差異上的一則留言（F-75）：`where` 是畫面算的位置、`code` 是那一行 diff 文字（換位置時靠它重新找回）。 */
export interface DiffNote {
  where: string;
  code: string;
  note: string;
}

/**
 * 差異每一行的位置說法（F-75）：加的「第 N 行（新增）」、刪的「原第 M 行（刪除）」、沒變的「第 N 行」；
 * 標頭與 hunk 行是 null（不能留言）。照 `@@ -a,b +c,d @@` 往下數。
 */
export function diffLineRefs(lines: string[]): (string | null)[] {
  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;
  return lines.map((l) => {
    const h = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(l);
    if (h) {
      oldNo = Number(h[1]);
      newNo = Number(h[2]);
      inHunk = true;
      return null;
    }
    if (!inHunk || l.startsWith("\\")) return null;
    if (l.startsWith("+")) return `第 ${newNo++} 行（新增）`;
    if (l.startsWith("-")) return `原第 ${oldNo++} 行（刪除）`;
    if (l.startsWith(" ") || l === "") {
      oldNo++;
      return `第 ${newNo++} 行`;
    }
    inHunk = false; // 下一個檔的 diff 標頭
    return null;
  });
}

/**
 * 留言現在掛在哪一行（差異重讀後行會移動）：先找位置與內容都一樣的，再找內容一樣、離原位置最近的；
 * 都找不到＝-1（那一行已經不在差異裡）。
 */
export function anchorNotes(notes: DiffNote[], lines: string[], refs: (string | null)[]): number[] {
  return notes.map((n) => {
    const exact = lines.findIndex((l, i) => l === n.code && refs[i] === n.where);
    if (exact >= 0) return exact;
    return lines.findIndex((l, i) => l === n.code && refs[i] !== null);
  });
}
