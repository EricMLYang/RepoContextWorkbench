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

/**
 * 卡片上的按鈕。action 是 ACTIONS 的鍵，或前端自己處理的 route_pick／collide_prefill／brief_session／
 * session_resume（F-77：接回一場 Claude Code 會話，payload 是 term_create 的 resume／cwd／prompt）。
 */
export interface CardAction {
  label: string;
  action: ActionName | "route_pick" | "collide_prefill" | "brief_session" | "session_resume";
  payload: Record<string, unknown>;
  /** 畫面上的主按鈕（引擎標；一張卡最多一個）。 */
  primary?: boolean;
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
  CollisionCard | EventCard | PendingCard | LoopCard | FreshCard | ReopenCard | RetroCard;
export type InboxCard = StateCard | QuestionCard;

// ── 會話清冊（term.TermManager.list ＋ state.annotate_terms）────────

export type AgentStatusName = "working" | "waiting" | "blocked" | "done" | "finished" | "exited";

export interface AgentStatus {
  status: AgentStatusName | (string & {});
  /** Rust 才有（F-80）：`finished` 之後你看過那個會話了（App 呼叫 `session_seen` 記的；下一個 hook 事件就消失）。 */
  seen?: boolean;
  /** Rust 才有（N-41）：做完沒看超過 24 小時，當成看過（不亮、不算徽章）；讀端算。 */
  faded?: boolean;
  /** Rust 才有（N-41）：處理中、沒有工具在跑、超過 30 分鐘沒任何事件——幾分鐘沒動靜；讀端算。 */
  quiet_minutes?: number;
  text: string;
  at: string; // "YYYY-MM-DD HH:MM"
  scope?: string;
  group?: string | null;
  repos?: string[] | null;
  repo?: string | null;
  /** Rust 才有：Claude Code hook 自動寫的（stop／notification／prompt）；agent 經 MCP 回報的沒有這個鍵。 */
  event?: string;
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
  /** Rust 才有：所屬牌組（畫面歸類用；從組裡對單一 repo 開的會話，scope 是 repo、這裡是組）。 */
  home_group?: string;
  /** Rust 才有（F-71）：Claude Code 取的對話標題（逐字稿 ai-title）。 */
  ai_title?: string | null;
  /** Rust 才有（F-71）：這一輪最後的回覆（一行預覽；你送出下一句就清掉）。 */
  last_reply?: string | null;
  /** Rust 才有（F-78）：Claude Code 自己的會話 id 與目錄（程序結束後「接續」用）。 */
  claude_session?: string | null;
  resume_cwd?: string | null;
  /** Rust 才有（F-82）：處理中正在用的工具（PreToolUse 記的）；`stalled`＝超過 10 分鐘沒結束（可能卡住，也可能只是慢）。 */
  tool?: { label: string; minutes: number; stalled: boolean } | null;
  /** Rust 才有（F-101）：這場會話的 context 用量 %（statusLine 寫的）與模型。 */
  context_pct?: number | null;
  model?: string | null;
  /** Rust 才有（F-102）：在跑的子 agent 數。 */
  subagents?: number;
  /** Rust 才有（F-100）：停在你這邊超過一小時——提示快取多半過期，下一句會整份重算。 */
  cache_cold?: boolean;
  /** Rust 才有（F-98）：自動繼續（開著才有）。 */
  auto_continue?: AutoContinue | null;
}

/** 自動繼續的設定與計數（F-98，引擎 `auto_continue::AutoContinue`）。 */
export interface AutoContinue {
  prompt: string;
  max: number;
  count: number;
}

/** 過去的一場 Claude Code 會話（F-72，引擎 `history::PastSession`）。 */
export interface PastSession {
  id: string;
  repo: string;
  cwd: string;
  title: string;
  prompt: string | null;
  reply: string | null;
  branch: string | null;
  started: string | null;
  updated: string;
  live: boolean;
  path: string;
}

/** Claude Code 的權限模式（F-96，引擎 `session::PermissionMode`）。 */
export type PermissionMode =
  "manual" | "acceptEdits" | "plan" | "auto" | "dontAsk" | "bypassPermissions";

/** GitHub 上開著的一筆 PR 或 issue（F-112，引擎 `github::slim`）。 */
export interface GithubItem {
  number: number;
  title: string;
  url: string;
  author: string | null;
  /** ISO 8601。 */
  updated: string | null;
  /** PR 才有。 */
  branch?: string | null;
  draft?: boolean;
  /** PR 的審查結果（APPROVED／CHANGES_REQUESTED／REVIEW_REQUIRED）。 */
  review?: string | null;
  labels: string[];
}

/** 一句常用 prompt（F-106，引擎 `prompts::Prompt`；存在 spine 的 `prompts/<name>.md`）。 */
export interface SavedPrompt {
  name: string;
  text: string;
  updated: string;
}

/** 會話打得出來的一個 `/指令`（F-105，引擎 `agentcmds::AgentCommand`）。 */
export interface AgentCommand {
  name: string;
  description: string;
  kind: "skill" | "command";
  source: "user" | "project" | "plugin";
}

/** 一個 commit（F-109，引擎 `gitlog::Commit`）。 */
export interface CommitInfo {
  sha: string;
  short: string;
  subject: string;
  author: string;
  /** ISO 8601（帶時區）。 */
  at: string;
}

/** repo 掛出去的一個 git worktree（F-107，引擎 `worktrees::Worktree`）。 */
export interface WorktreeInfo {
  path: string;
  branch: string | null;
  head: string;
  /** 主 worktree（repo 本身）。 */
  main: boolean;
  /** 沒 commit 的檔數；目錄不在了＝null。 */
  dirty: number | null;
  /** 只在這裡的 commit 數（別的分支與遠端都沒有）。 */
  unique: number;
  locked: boolean;
  prunable: boolean;
  /** 乾淨、沒有獨有 commit、沒鎖：可以安全移除。 */
  removable: boolean;
}

/** 逐字稿時間軸的一框（F-94，引擎 `timeline::Frame`）。 */
export interface TranscriptFrame {
  kind: "you" | "reply" | "thinking" | "tool" | "compact";
  /** 你說的／回覆／思考的文字；工具是「做什麼」（指令、路徑…）。 */
  text: string;
  at: string | null;
  tool?: string;
  /** 工具結果預覽（結果回來了才有）。 */
  result?: string;
  error?: boolean;
}

/** App 助手的一步（引擎驗證過；error 有值＝不能執行）。 */
export interface AssistStep {
  op:
    | "switch_scope"
    | "open_tab"
    | "open_repo"
    | "create_group"
    | "set_members"
    | "rename_group"
    | "set_goal"
    | "tag"
    | "set_tier"
    | "open_session"
    | (string & {});
  args: Record<string, unknown>;
  text: string;
  why: string;
  danger: boolean;
  error?: string | null;
  /** 復原這一步要做的（引擎照規劃當下的設定算；op 多一種 remove_group）。 */
  undo?: { op: string; args: Record<string, unknown> } | null;
}

export interface AssistPlan {
  ok: true;
  text: string;
  summary: string;
  source: "rules" | "agent" | "mcp";
  steps: AssistStep[];
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
  /** 範圍已不存在（組被刪或改名、臨時組合的 repo 全被移除）。 */
  gone?: boolean;
}

/** 健康指標（Rust 引擎 F-36／F-50；同 `ctx health`）。比率是 0–1，沒有分母時是 null。 */
export interface Health {
  days: number;
  handoff: { handoffs: number; missed: number; rate: number | null };
  fresh: { found: number; used: number; dismissed: number; hit_rate: number | null };
  decisions: { with_outcome: number; awaiting_retro: number; outcome_rate: number | null };
  loops: { opened: number; closed: number; open_now: number; overdue: number };
  /** 名片覆蓋（不含封存）；舊引擎沒有這個鍵。 */
  profiles?: {
    repos: number;
    described: number;
    with_role: number;
    /** 還沒寫介紹的（最多 10 個） */
    undescribed: string[];
    undescribed_more: number;
    /** #68：寫介紹之後又有 50 個以上 commit 的（介紹可能過時）；舊引擎沒有這個鍵。 */
    stale?: { repo: string; described_at: string; commits: number }[];
  };
  /** 助手用量（F-63）；舊引擎沒有這個鍵。 */
  assist?: {
    runs: number;
    from_agent: number;
    did_something: number;
    failed: number;
    undone: number;
  };
  /** 程式化呼叫 `claude -p`（F-84；碰撞判定、ask_repo、助手、backfill——另計額度）；舊引擎沒有這個鍵。 */
  agent_calls?: {
    calls: number;
    today: number;
    avg_ms: number;
    errors: number;
    /** config `provider.programmatic`：allow｜off */
    policy: string;
  };
}

/**
 * 額度餘光（F-83；引擎排好，這裡只排版）。Claude：App 會話的 statusLine 回報（`.state/usage.json`）；
 * Codex（2026-10-28）：引擎讀 Codex 自己的會話紀錄，形狀一樣。
 */
export interface UsageWindow {
  /** 已用百分比（0–100，整數） */
  used: number;
  /** 什麼時候重置（引擎排好的本地時間字串）；不知道＝null */
  resets: string | null;
  /** 還有幾分鐘重置（引擎讀的時候算；舊資料沒有） */
  resets_in_min?: number;
}
export interface Usage {
  at: string; // "YYYY-MM-DD HH:MM"
  five_hour: UsageWindow | null;
  seven_day: UsageWindow | null;
  /** 這場會話的 context 用了幾 %；沒有＝null（只有 Claude） */
  context?: number | null;
  model?: string | null;
  /** Codex 的方案（plus、pro…） */
  plan?: string | null;
  /** 任一視窗 ≥ 80% */
  warn: boolean;
  /** 幾分鐘前更新的（引擎讀的時候算） */
  age_min?: number;
}

/** 檔案清單的一列（path＝相對 repo 根目錄，abs＝絕對路徑；都由引擎給，前端不拼路徑）。 */
export interface TreeEntry {
  name: string;
  path: string;
  abs: string;
  dir: boolean;
  /** 被 .gitignore 忽略：照列、畫灰色（跟 VS Code 一樣）。 */
  ignored?: boolean;
}

/** 一個還沒 commit 的變更（path 相對 repo 根目錄）。 */
export interface RepoChange {
  path: string;
  status: "modified" | "added" | "deleted" | "renamed" | "untracked" | "conflicted";
}

export interface GroupNote {
  /** YYYY-MM-DD HH:MM */
  at: string;
  text: string;
}

export interface GroupNotes {
  ok: true;
  group: string;
  /** 筆記檔的絕對路徑（人跟 agent 都能直接讀）。 */
  path: string;
  notes: GroupNote[];
}

/** Rust 才有：⌘K 的結果（排名在引擎）。repos 依分數排，空查詢＝registry 順序。 */
export interface PaletteResult {
  ok: true;
  query: string;
  repos: {
    repo: string;
    score: number;
    /** 為什麼中（例：名稱開頭、角色命中） */
    why: string[];
    role?: string;
    description?: string;
    /** 在哪些牌組 */
    groups: string[];
  }[];
  groups: { name: string; members: number; goal: string }[];
  /** 用檔名找到的檔（兩個字以上才找；照 payload 的範圍）。舊引擎沒有這個鍵。 */
  files?: { repo: string; path: string; name: string; abs: string }[];
}

/** Rust 才有（F-57）：repo 名片。存在 spine 的 registry，不寫進 repo。 */
export interface RepoProfile {
  ok: true;
  id: string;
  /** 絕對路徑 */
  path: string;
  /** 協作角色（一個短詞） */
  role?: string;
  description?: string;
  /** registry＝人或 agent 寫的；readme＝沒填，從 README／AGENTS.md 截的推測 */
  description_source?: "registry" | "readme";
  keywords?: string[];
  tags?: string[];
  /** 類別（2026-10-18，引擎 registry::RepoClass）；未分類＝沒有這個鍵。 */
  class?: RepoClassInfo;
  /** 類別選單的選項（引擎給，前端不對照）；舊引擎沒有這個鍵。 */
  classes?: { id: string; label: string; reliable: string }[];
}

/** repo 類別：名稱、怎樣算可靠、給 agent 的規則、名冊用的短規則。 */
export interface RepoClassInfo {
  id: string;
  label: string;
  reliable: string;
  rule: string;
  short?: string;
}

/**
 * read_file 的結果：文字、圖片（base64）、其他二進位或太大只有大小。
 * 文字的 `editable`＝能在 App 裡編輯（沒截斷、合法 UTF-8）；`version`＝存檔時帶回去判斷有沒有被別人改過。
 */
export type FileContent = { ok: true; id: string; path: string; abs: string; size: number } & (
  | { kind: "text"; text: string; truncated: boolean; editable?: boolean; version?: string }
  | { kind: "image"; mime: string; base64: string }
  | { kind: "binary" }
  | { kind: "too_large" }
);

export interface WorkbenchState {
  groups: GroupInfo[];
  repos: string[];
  tiers: Record<string, Tier>;
  tags: Record<string, string[]>;
  tag_vocab: string[];
  relations: Relation[];
  rel_kinds: Record<string, RelationKind>;
  /** 健康指標（舊版引擎沒有這個鍵）。 */
  health?: Health;
  /** Rust 才有（F-83）：Claude Code 額度餘光；沒收過 statusLine 就沒有這個鍵。 */
  usage?: Usage | null;
  /** Rust 才有（2026-10-28）：Codex 額度；沒裝、沒用過就沒有這個鍵。 */
  codex_usage?: Usage | null;
  /** Rust 才有（F-80）：活著的 agent 會話裡在等你／做完沒看各幾個（殼的徽章用；前端牌組卡自己算每組的）。 */
  attention?: { needs_you: number; finished_unseen: number; working: number };
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

/** 檔案分頁的看法（`screen` action）。 */
export type ScreenFileMode = "view" | "source" | "diff" | "edit";

/** 檔案分頁自己知道的那部分畫面（看法、看得到的行、md 停在哪個標題、有沒有沒存的修改）。 */
export interface ScreenFileDetail {
  mode: ScreenFileMode;
  lines?: [number, number];
  heading?: string;
  unsaved?: boolean;
}

/** 回報給引擎的畫面（`ctx_core::screen::ScreenInput`）。 */
export interface ScreenReport {
  view:
    | ({ kind: "file"; repo: string; path: string } & ScreenFileDetail)
    | { kind: "workspace"; tab: "inbox" | "notes" | "repos" | "files" | "activity" | "relations" };
  group?: string;
  panel?: { repo: string; tab: "files" | "changes" | "info" };
}

export interface ActionMap {
  ack: { payload: Record<string, never>; result: { ok: true } };
  /** Rust 才有（N-37）：只把目前範圍的一般未讀標為已讀。 */
  ack_scope: { payload: ScanParams; result: { ok: true; at: string } };
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
  /** 判斷回顧卡「記結果…」：人直接寫這個判斷後來怎樣（Rust 引擎；log_decision 帶 outcome_of，manual）。 */
  decision_outcome: {
    payload: { ref: string; text: string; group?: string | null; repo?: string | null };
    result: { ok: true; ref: string };
  };
  /** 該重看卡「改判斷…」：人直接寫新判斷取代原判斷（Rust 引擎；log_decision 帶 supersedes，manual）。 */
  decision_supersede: {
    payload: { ref: string; text: string; group?: string | null; repo?: string | null };
    result: { ok: true; ref: string };
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
  /** Rust 才有：repo 的檔案清單，一次一層（dir 空＝根目錄）。 */
  repo_tree: {
    payload: { id: string; dir?: string };
    result: { ok: true; id: string; root: string; dir: string; entries: TreeEntry[]; more: number };
  };
  /** Rust 才有：像 IDE 的 Add Folder to Workspace——沒登記就登記，給 group 就加進那組。 */
  group_add_folder: {
    payload: { path: string; group?: string };
    result: { ok: true; id: string; registered: boolean; group: string | null };
  };
  /** Rust 才有：側欄拖曳排序（只換順序，不記 spine）。 */
  reorder: {
    payload: { kind: "groups" | "members" | "repos"; order: string[]; group?: string };
    result: { ok: true };
  };
  /** Rust 才有：還沒 commit 的變更（git status）；不是 git repo 會失敗。 */
  repo_changes: {
    payload: { id: string };
    result: {
      ok: true;
      id: string;
      changes: RepoChange[];
      more: number;
      /** 已 commit 但還沒 push 的檔（相對路徑；沒上游分支＝空）。檔案樹標 ↑。 */
      unpushed?: string[];
    };
  };
  /** Rust 才有：範圍內每個 repo 還沒 commit 的變更（只回有變更的；git 失敗的帶 error）。 */
  scope_changes: {
    payload: ScopePayload;
    result: {
      ok: true;
      repos: { id: string; changes: RepoChange[]; more: number; error?: string }[];
    };
  };
  /** Rust 才有：請 agent 審查還沒 commit 的變更的任務文字（先不改檔、不 commit）。沒有變更會失敗。 */
  review_brief: {
    payload: { id: string };
    result: { ok: true; id: string; text: string; count: number };
  };
  /** Rust 才有：單檔差異（對上一個 commit；還沒追蹤的整份算新增）。沒有差異＝空字串。 */
  file_diff: {
    payload: { id: string; path: string };
    result: { ok: true; id: string; path: string; diff: string; truncated: boolean };
  };
  /** Rust 才有：唯讀檢視器讀一個檔。 */
  read_file: { payload: { id: string; path: string }; result: FileContent };
  write_file: {
    payload: { id: string; path: string; text: string; base: string };
    result: FileContent | { ok: false; conflict: true; id: string; path: string };
  };
  /** Rust 才有（F-63）：App 助手——一句話 → 驗證過的計畫（不執行）。 */
  assist_plan: {
    payload: { text: string } & ScanParams;
    result: AssistPlan;
  };
  /** Rust 才有（F-68）：最近用過、真的做了事的助手指令（新的在前，最多 5 句）。 */
  assist_recent: { payload: Record<string, never>; result: { ok: true; texts: string[] } };
  assist_log: {
    payload: { text: string; group?: string; done: string[]; skipped: string[]; failed?: string };
    result: { ok: true };
  };
  /** Rust 才有（F-64）：md 編輯區貼上的圖片存成新檔（png／jpg／gif／webp／bmp，≤ 8MB，不覆蓋）。 */
  save_image: {
    payload: { id: string; path: string; base64: string };
    result: { ok: true; id: string; path: string; abs: string };
  };
  /** Rust 才有（F-75）：差異上的逐行留言 → 一段給 agent 的話（空白留言略過；全空回錯）。 */
  diff_notes_brief: {
    payload: { id: string; path: string; notes: { where: string; code: string; note: string }[] };
    result: { ok: true; text: string; count: number };
  };
  /** Rust 才有（F-72）：目前範圍過去的 Claude Code 互動會話（最近的在前）。 */
  past_sessions: {
    payload: ScopePayload & { limit?: number; query?: string };
    result: { ok: true; sessions: PastSession[] };
  };
  /** Rust 才有（F-94）：一場會話的逐字稿時間軸（App 會話給 sid；過去的會話給 id＋cwd）。只讀。 */
  session_transcript: {
    payload: { sid?: string; id?: string; cwd?: string };
    result: {
      ok: true;
      id: string;
      frames: TranscriptFrame[];
      /** 檔案太大只讀了尾巴，或框數超過上限只留最後的。 */
      truncated: boolean;
      title: string | null;
      path: string;
    };
  };
  /** Rust 才有（F-103）：把一場會話整理成交接包（秘密遮掉），存到所屬組的 materials。 */
  session_digest: {
    payload: { sid?: string; id?: string; cwd?: string };
    result: {
      ok: true;
      path: string;
      text: string;
      /** 遮了幾處秘密。 */
      hidden: number;
      /** 會話目錄所屬的登記 repo（沒有就不能直接對它開新會話）。 */
      repo: string | null;
      group: string | null;
    };
  };
  /** Rust 才有（F-106）：常用 prompt，最近改的在前。只讀。 */
  prompts: {
    payload: Record<string, never>;
    result: { ok: true; dir: string; prompts: SavedPrompt[] };
  };
  /** Rust 才有（F-106）：存一句（同名覆蓋；rename_from＝改名）。 */
  prompt_save: {
    payload: { name: string; text: string; rename_from?: string };
    result: { ok: true; prompt: SavedPrompt };
  };
  prompt_remove: { payload: { name: string }; result: { ok: true } };
  /** Rust 才有（F-105）：這個會話（或目錄）打得出哪些 `/指令`。只讀。 */
  agent_commands: {
    payload: { sid?: string; cwd?: string };
    result: { ok: true; cwd: string | null; commands: AgentCommand[] };
  };
  /** Rust 才有（F-113）：貼給 agent 會話的圖存到 spine 的 .state/session_images/（不放 repo），回傳絕對路徑。 */
  session_image: { payload: { ext: string; base64: string }; result: { ok: true; abs: string } };
  /** Rust 才有（F-112）：repo 在 GitHub 上開著的 PR 與 issue（用使用者自己的 gh）。只讀。 */
  repo_github: {
    payload: { id: string };
    result: { ok: true; id: string; prs: GithubItem[]; issues: GithubItem[] };
  };
  /** Rust 才有（F-109）：repo 最近的 commit（目前分支，新的在前）。只讀。 */
  repo_log: {
    payload: { id: string; limit?: number };
    result: { ok: true; id: string; commits: CommitInfo[] };
  };
  /** Rust 才有（F-109）：一個 commit 的說明、統計與差異。只讀。 */
  commit_diff: {
    payload: { id: string; sha: string };
    result: { ok: true; id: string; sha: string; diff: string; truncated: boolean };
  };
  /** Rust 才有（F-107）：repo 掛出去的 worktree。只讀。 */
  repo_worktrees: {
    payload: { id: string };
    result: { ok: true; id: string; worktrees: WorktreeInfo[] };
  };
  /** Rust 才有（F-107）：移除一個乾淨的 worktree（不加 --force，分支留著）。 */
  worktree_remove: { payload: { id: string; path: string }; result: { ok: true } };
  /** Rust 才有（F-95）：終端裡的路徑字樣 → 登記的 repo＋相對路徑＋行欄（相對路徑照會話目錄）。只讀。 */
  locate_path: {
    payload: { text: string; sid?: string };
    result: { ok: true; repo: string; path: string; line: number | null; col: number | null };
  };
  /** Rust 才有（F-69）：repo 裡哪些 md 連到這個檔（照相對連結解析、跳過程式碼區塊；最多 50 則）。 */
  backlinks: {
    payload: { id: string; path: string };
    result: {
      ok: true;
      id: string;
      path: string;
      items: { path: string; line: number; text: string }[];
      more: number;
    };
  };
  /** Rust 才有（F-66）：改名或搬移（不覆蓋、不刪）。path 回新的相對路徑。 */
  rename_entry: {
    payload: { id: string; from: string; to: string };
    result: { ok: true; id: string; from: string; path: string; abs: string; dir: boolean };
  };
  /** Rust 才有：新增空檔或資料夾（不覆蓋；上層沒有就一路建）。path 回相對 repo 根目錄。 */
  create_entry: {
    payload: { id: string; path: string; kind: "file" | "dir" };
    result: { ok: true; id: string; path: string; abs: string; dir: boolean };
  };
  /** Rust 才有（F-57）：repo 名片（唯讀）與編輯——三欄送完整內容，引擎只寫有變的欄位。 */
  repo_profile: { payload: { id: string }; result: RepoProfile };
  palette: { payload: { query: string } & ScanParams; result: PaletteResult };
  /** Rust 才有（F-57）：請 agent 起草名片的任務（ids 沒給＝還沒寫介紹的，最多 10 個）。 */
  profile_draft_brief: {
    payload: { ids?: string[] };
    result: { ok: true; text: string; repos: string[] };
  };
  /** Rust 才有：挑好的檔案 → 會話任務文字（依 repo 分段、附名片）＋`@絕對路徑`（一行一個）。 */
  pick_brief: {
    payload: { files: { id: string; path: string }[] };
    result: { ok: true; text: string; mentions: string; repos: string[]; count: number };
  };
  describe_repo: {
    payload: { id: string; description: string; role: string; keywords: string };
    result: RepoProfile & { changed: boolean };
  };
  /** Rust 才有（2026-10-18）：改 repo 類別（空字串＝清掉）；回更新後的名片。 */
  repo_class: {
    payload: { id: string; class: string };
    result: RepoProfile & { changed: boolean };
  };
  /** Rust 才有：牌組筆記（spine 的 groups/<組>/notes.md；新的在前）。 */
  group_notes: { payload: { group: string }; result: GroupNotes };
  note_add: { payload: { group: string; text: string }; result: GroupNotes };
  note_remove: { payload: { group: string; at: string; text: string }; result: GroupNotes };
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
      /** shell 才用：開成這個會話的 Terminal（沿用它的目錄，origin＝`shell-of:<sid>`）。 */
      of?: string;
      /** 所屬牌組（只給畫面歸類）。 */
      home_group?: string;
      /** Rust 才有（F-72）：接續這場 Claude Code 會話（`claude --resume`；cwd 是它原本的目錄）。 */
      resume?: string;
      /** Rust 才有（F-77）：接續時的第一句（例：請它補交接）。 */
      prompt?: string;
      /** Rust 才有（F-96）：Claude Code 的權限模式（`--permission-mode`）。 */
      permission_mode?: PermissionMode;
      /** Rust 才有（F-96）：模型別名或全名（`--model`）。 */
      model?: string;
      /** Rust 才有（F-108）：在 Claude Code 自己建的 git worktree 跑；true＝自動取名。只能對單一 repo。 */
      worktree?: boolean | string;
      /** Rust 才有（F-97）：接續時岔出新會話（`--fork-session`），原會話不動。 */
      fork?: boolean;
    } & ScopePayload;
    result: { ok: true; sid: string; title: string };
  };
  term_kill: { payload: { sid: string }; result: { ok: true } };
  /** Rust 才有（F-80）：你看過這個會話了（做完沒看→看過了）。沒狀態檔或已記過＝`changed: false`。 */
  session_seen: { payload: { sid: string }; result: { ok: true; changed: boolean } };
  session_result: {
    payload: { sid: string; text: string };
    result: { ok: true; date: string; time: string };
  };
  /** Rust 才有（F-98）：開／改（計數歸零）或關這個會話的自動繼續。 */
  auto_continue: {
    payload: { sid: string; max?: number; prompt?: string; off?: boolean };
    result: { ok: true; auto_continue: AutoContinue | null };
  };
  /** Rust 才有（2026-10-24）：主區在看什麼（agent 被問「根據畫面」才讀）。跟上次一樣＝`changed: false`。 */
  screen: { payload: ScreenReport; result: { ok: true; changed: boolean } };
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
  "session_seen",
  "session_result",
];
