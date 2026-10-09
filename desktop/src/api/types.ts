/**
 * 工作台資料形狀——照 Python 版 `repo_context/webui/state.py`、`actions.py` 實際產出的欄位。
 * Rust 端（`ctx_core::workbench::{state,actions}`）序列化出來的 JSON 必須跟這裡一致（見 desktop/IPC.md）。
 * 欄位名一律沿用 Python（snake_case），前端不改名。
 */

// ── 共用 ────────────────────────────────────────────────────────────

export type Tier = "active" | "paused" | "dormant" | "archived" | (string & {});

/** 範圍歸屬（state.event_scopes）。 */
export type ScopeKind = "repo" | "group" | "global" | "unclassified";

export interface Provenance {
  scope_kind: ScopeKind;
  scope_label: string;
  scope_repos: string[];
}

/**
 * 卡片／事件在目前範圍（get_state 的 group/repos）下屬於哪一欄——引擎算好（state.build_state），前端只照著分欄：
 * scope＝範圍內、global＝全域與未分類（或範圍外的打斷）、other＝不顯示。
 */
export type Lane = "scope" | "global" | "other";

/** 脊椎事件（state._ev_dict）。 */
export interface SpineEvent {
  date: string; // YYYY-MM-DD
  time: string; // HH:MM
  type: string;
  source: string;
  tokens: string[];
  body: string;
  header: string;
}

// ── 卡片動作（每個按鈕＝一個 action 名稱＋payload）──────────────────

/** 卡片上的按鈕。action 是 ACTIONS 的鍵，或前端自己處理的 route_pick／collide_prefill／brief_session。 */
export interface CardAction {
  label: string;
  action: ActionName | "route_pick" | "collide_prefill" | "brief_session";
  payload: Record<string, unknown>;
}

// ── 收件匣卡片（state.build_cards ＋ brief.build_question_cards）────

export type CardKind =
  | "interrupt"
  | "question"
  | "collision"
  | "event"
  | "loop"
  | "pending"
  | "fresh"
  | "reopen"
  | "retro";

interface CardBase {
  key: string;
  kind: CardKind;
  title: string;
  lines?: string[];
  actions: CardAction[];
  date?: string;
  time?: string;
}

/** 碰撞判定回程。 */
export interface CollisionCard extends CardBase, Provenance {
  kind: "collision";
  lane: Lane;
  cid: string;
  verdict: string;
  judgement: Record<string, string>;
}

/** 打斷／其他未讀。 */
export interface EventCard extends CardBase, Provenance {
  kind: "interrupt" | "event";
  lane: Lane;
  etype: string;
  source: string;
}

/** 處理中的碰撞（opened 未回程）。actions 恆為 []。 */
export interface PendingCard extends CardBase, Provenance {
  kind: "pending";
  lane: Lane;
  cid: string;
}

/** 待跟進（open loop）。 */
export interface LoopCard extends CardBase, Provenance {
  kind: "loop";
  lane: Lane;
  num: string;
  due: string | null;
}

/** 閾值問句卡（/api/scan 的 question_cards；provenance 以 repo 為範圍，由引擎補）。 */
export interface QuestionCard extends CardBase, Provenance {
  kind: "question";
  qkind: "dirty" | "inactive" | "broken" | (string & {});
  repo: string;
}

/** 反芻推薦（Rust 引擎 F-48）：別組新進的知識跟本組手上工作相關；出口＝拿來試／不相關。 */
export interface FreshCard extends CardBase, Provenance {
  kind: "fresh";
  lane: Lane;
  id: string;
}

/** 該重看的判斷（Rust 引擎 F-46）：重看條件的日期到了；出口＝開會話重看／延 14 天。 */
export interface ReopenCard extends CardBase, Provenance {
  kind: "reopen";
  lane: Lane;
  ref: string;
}

/** 判斷回顧（Rust 引擎 F-23）：滿 14 天、還沒記結果的判斷；出口＝開會話回顧／延 30 天。 */
export interface RetroCard extends CardBase, Provenance {
  kind: "retro";
  lane: Lane;
  ref: string;
}

export type StateCard =
  | CollisionCard
  | EventCard
  | PendingCard
  | LoopCard
  | FreshCard
  | ReopenCard
  | RetroCard;
export type InboxCard = StateCard | QuestionCard;

// ── 會話清冊（term.TermManager.list ＋ state.annotate_terms）────────

export type AgentStatusName = "working" | "waiting" | "blocked" | "done";

export interface AgentStatus {
  status: AgentStatusName | (string & {});
  text: string;
  at: string; // "YYYY-MM-DD HH:MM"
  scope?: string;
  group?: string | null;
  repos?: string[] | null;
  repo?: string | null;
}

export interface SessionReport {
  text: string;
  type: string;
  source: string;
  at: string; // "YYYY-MM-DD HH:MM"
}

export interface TermInfo {
  sid: string;
  title: string;
  alive: boolean;
  kind: "agent" | "shell";
  origin: string | null;
  scope: string | null;
  scope_repos: string[] | null;
  started: string | null;
  task: string | null;
  report: SessionReport | null;
  report_basis: string;
  /** 只有 agent 會話有這個鍵。 */
  agent_status?: AgentStatus | null;
}

// ── /api/state ──────────────────────────────────────────────────────

export interface GroupInfo {
  name: string;
  members: string[];
  goal: string;
  goal_at: string;
}

export interface Relation {
  from: string;
  to: string;
  kind: string;
  note?: string;
  exports?: string[];
  /** 碰到目前範圍（from 或 to 在範圍內）。 */
  in_scope: boolean;
  /** 「用這條關係開工作」的任務文案。 */
  task: string;
}

/** 關係建議（Rust 引擎 F-47；反芻時算好）：from→to 是建議的方向，kind 有推測才有。 */
export interface RelationSuggestion {
  from: string;
  to: string;
  kind: string | null;
  reason: string;
  files: string[];
}

export interface RelationKind {
  forward: string;
  inverse: string;
  /** Python 內建詞彙只有對稱的才帶這個鍵（config 擴充的一律帶）。 */
  symmetric?: boolean;
}

export interface Stats {
  collisions: number;
  collisions_with_outcome: number;
  hit_rate: number | null;
  spawned: number;
  alive: number;
  survival_rate: number | null;
}

export type RecentEvent = SpineEvent & Provenance & { lane: Lane };

/** get_state 收到的範圍（原樣回傳）＋範圍內的 repo。 */
export interface StateScope {
  group: string | null;
  repos: string[] | null;
  members: string[];
}

export interface WorkbenchState {
  groups: GroupInfo[];
  repos: string[];
  tiers: Record<string, Tier>;
  tags: Record<string, string[]>;
  tag_vocab: string[];
  relations: Relation[];
  rel_kinds: Record<string, RelationKind>;
  /** 碰到目前範圍、還沒設的關係建議；沒有建議時引擎不給這個鍵。 */
  relation_suggestions?: RelationSuggestion[];
  unread: (SpineEvent & { interrupt?: boolean })[];
  loops: SpineEvent[];
  cards: StateCard[];
  recent: RecentEvent[];
  stats: Stats;
  /** 這份 state 是照哪個範圍算 lane／in_scope 的。 */
  scope: StateScope;
  /** server 補的：會話清冊。 */
  terms: TermInfo[];
  /** server 補的：config.yaml 的 agents 鍵。 */
  agents: string[];
}

// ── /api/scan ───────────────────────────────────────────────────────

export interface Commit {
  date: string;
  hash: string;
  subject: string;
}

/** collect.collect_repo */
export interface RepoState {
  id: string;
  type: string;
  tier: Tier;
  path: string;
  exists: boolean;
  dirty: number;
  dirty_days: number | null;
  last_commit_days: number | null;
  ahead: number | null;
  behind: number | null;
  branch: string | null;
  last_subject: string;
  worktrees: number;
  commits_7d: number | null;
  commits_30d: number | null;
  recent_commits: Commit[];
  agents: string[];
  note: string;
}

export interface Pulse {
  repos: number;
  commits_7d: number;
  commits_30d: number;
  per_repo_7d: Record<string, number>;
  active_7d: string[];
  quiet_30d: string[];
  most_active: string | null;
  latest: (Commit & { repo: string }) | null;
}

/** summary._fact */
export interface Fact {
  text: string;
  source: string;
  at: string | null;
}

/** summary.build_summary */
export interface WorkSummary {
  group: string;
  saved_group: string | null;
  repos: string[];
  at: string;
  goal: Fact | null;
  goal_gap: string | null;
  progress: Fact | null;
  progress_gap: string | null;
  since: string | null;
  changes: Fact[];
  changes_window: string;
  next: Fact | null;
  next_gap: string | null;
  changes_gap: string;
  gaps: string[];
  resume_task: string;
}

export interface ScanResult {
  group: string;
  states: RepoState[];
  summary: WorkSummary | null;
  pulse?: Pulse;
  recent_commits: (Commit & { repo: string })[];
  question_cards: QuestionCard[];
  questions?: string[];
  quiet: number;
  /** 開會話的起手任務（接續上次工作／整理本組現況／組內第一條關係）。 */
  session_tasks: SessionTask[];
  audit: string[];
}

export interface SessionTask {
  label: string;
  text: string;
}

/** /api/scan 的參數：repos 優先於 group；兩者都沒有＝全部。repos 不可為空陣列。 */
export interface ScanParams {
  group?: string;
  repos?: string[];
}

// ── 動作（POST /api/<name>；Tauri: run_action {name, payload}）────────

/** 會被 term_create／collide 等帶上的範圍。 */
export interface ScopePayload {
  group?: string | null;
  repos?: string[] | null;
}

export interface ActionMap {
  ack: { payload: Record<string, never>; result: { ok: true } };
  relate: {
    payload: { a: string; b: string; kind: string; note?: string };
    result: { ok: true; relation: Relation };
  };
  unrelate: {
    payload: { a: string; b: string; kind?: string };
    result: { ok: true } & Record<string, unknown>;
  };
  /** 反芻推薦「拿來試」：在該組開帶出處的待辦（Rust 引擎 F-26）。 */
  fresh_try: {
    payload: { id: string };
    result: { ok: true; todo: string; group: string; ref: string; text: string };
  };
  /** 「該重看」延後（Rust 引擎 F-46）。 */
  reopen_defer: {
    payload: { ref: string; days: number };
    result: { ok: true; until: string };
  };
  /** 未結事項的任務包 markdown（Rust 引擎 F-42；前端 brief_session 用它開會話）。 */
  todo_brief: {
    payload: { num: string };
    result: { ok: true; id: string; markdown: string } & Record<string, unknown>;
  };
  /** 反芻推薦「不相關」（Rust 引擎 F-48）。 */
  fresh_dismiss: {
    payload: { id: string };
    result: { ok: true; dismissed: string };
  };
  /** 關係建議「不相關」：記住，之後不再建議（Rust 引擎 F-47）。 */
  relation_dismiss: {
    payload: { a: string; b: string };
    result: { ok: true; dismissed: [string, string] };
  };
  collide: {
    payload: { idea: string; wait?: boolean } & ScopePayload;
    result: { ok: true; cid: string; judgement?: Record<string, string> };
  };
  collide_rerun: { payload: { cid: string }; result: { ok: true; cid: string } };
  ignore: {
    payload: { type: string; time: string; header?: string; cid?: string };
    result: { ok: true };
  };
  close_loop: { payload: { num: string }; result: { ok: true } };
  loop_defer: { payload: { num: string; days?: number }; result: { ok: true; due: string } };
  defer: {
    payload: { id: string; qkind?: string; days?: number; text?: string };
    result: { ok: true; until: string; days: number };
  };
  tier: {
    payload: { id: string; tier: Tier; resume_when?: string };
    result: { ok: true; id: string; tier: Tier };
  };
  remove_repo: { payload: { id: string }; result: { ok: true } & Record<string, unknown> };
  route_collision: {
    payload: { cid: string; dest: string };
    result: { ok: true; cid: string; dest: string; file: string };
  };
  tag: {
    payload: { id: string; add?: string[]; remove?: string[] };
    result: { ok: true } & Record<string, unknown>;
  };
  save_group: { payload: { name: string; repos: string[] }; result: { ok: true; name: string } };
  update_group: {
    payload: { name: string; repos: string[]; new_name?: string };
    result: { ok: true; name: string; members: string[] };
  };
  remove_group: { payload: { name: string }; result: { ok: true } & Record<string, unknown> };
  set_goal: {
    payload: { group: string; text: string };
    result: { ok: true } & Record<string, unknown>;
  };
  session_draft: {
    payload: { sid: string };
    result: { ok: true; text: string; traces: number; loops: number; note: string };
  };
  scan_dir: {
    payload: { dir: string };
    result: { ok: true; dir: string; candidates: { path: string; id: string }[] };
  };
  register_repos: {
    payload: { repos: { path: string; id: string }[]; group?: string };
    result: { ok: true; added: string[]; group: string | null };
  };
  brief: { payload: ScopePayload; result: { ok: true; text: string; file: string } };
  term_create: {
    payload: {
      kind: "agent" | "shell";
      agent?: string;
      task?: string;
      origin?: string;
      repo?: string;
      cwd?: string;
    } & ScopePayload;
    result: { ok: true; sid: string; title: string };
  };
  term_kill: { payload: { sid: string }; result: { ok: true } };
  session_result: {
    payload: { sid: string; text: string };
    result: { ok: true; date: string; time: string };
  };
}

export type ActionName = keyof ActionMap;
export type ActionPayload<N extends ActionName> = ActionMap[N]["payload"];
export type ActionResult<N extends ActionName> = ActionMap[N]["result"];

/** 全部 action 名稱（與 webui/actions.py 的 ACTIONS 一一對應）。 */
export const ACTION_NAMES: ActionName[] = [
  "ack",
  "relate",
  "unrelate",
  "collide",
  "collide_rerun",
  "ignore",
  "close_loop",
  "loop_defer",
  "defer",
  "tier",
  "remove_repo",
  "route_collision",
  "tag",
  "save_group",
  "update_group",
  "remove_group",
  "set_goal",
  "session_draft",
  "scan_dir",
  "register_repos",
  "brief",
  "term_create",
  "term_kill",
  "session_result",
];
