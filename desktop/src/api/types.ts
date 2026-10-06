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

/** 卡片上的按鈕。action 是 ACTIONS 的鍵，或前端自己處理的 route_pick／collide_prefill。 */
export interface CardAction {
  label: string;
  action: ActionName | "route_pick" | "collide_prefill";
  payload: Record<string, unknown>;
}

// ── 收件匣卡片（state.build_cards ＋ brief.build_question_cards）────

export type CardKind = "interrupt" | "question" | "collision" | "event" | "loop" | "pending";

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
  cid: string;
  verdict: string;
  judgement: Record<string, string>;
}

/** 打斷／其他未讀。 */
export interface EventCard extends CardBase, Provenance {
  kind: "interrupt" | "event";
  etype: string;
  source: string;
}

/** 處理中的碰撞（opened 未回程）。actions 恆為 []。 */
export interface PendingCard extends CardBase, Provenance {
  kind: "pending";
  cid: string;
}

/** 待跟進（open loop）。 */
export interface LoopCard extends CardBase, Provenance {
  kind: "loop";
  num: string;
  due: string | null;
}

/** 閾值問句卡（/api/scan 的 question_cards；沒有 provenance，前端以 repo 補）。 */
export interface QuestionCard extends CardBase, Partial<Provenance> {
  kind: "question";
  qkind: "dirty" | "inactive" | "broken" | (string & {});
  repo: string;
}

export type StateCard = CollisionCard | EventCard | PendingCard | LoopCard;
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
}

export interface RelationKind {
  forward: string;
  inverse: string;
  symmetric: boolean;
}

export interface Stats {
  collisions: number;
  collisions_with_outcome: number;
  hit_rate: number | null;
  spawned: number;
  alive: number;
  survival_rate: number | null;
}

export type RecentEvent = SpineEvent & Provenance;

export interface WorkbenchState {
  groups: GroupInfo[];
  repos: string[];
  tiers: Record<string, Tier>;
  tags: Record<string, string[]>;
  tag_vocab: string[];
  relations: Relation[];
  rel_kinds: Record<string, RelationKind>;
  unread: (SpineEvent & { interrupt?: boolean })[];
  loops: SpineEvent[];
  cards: StateCard[];
  recent: RecentEvent[];
  stats: Stats;
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
  audit: string[];
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
