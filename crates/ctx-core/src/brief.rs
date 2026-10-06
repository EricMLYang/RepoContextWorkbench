//! 早晨簡報產生器（v2 §4.5 樣貌 A，純事實版、無 LLM）。對應 Python `repo_context/brief.py`。
//!
//! 三段固定結構：需要你判斷的（≤3 條問句＋出口）／未結 open loops／沉默摘要一行。
//! v1.1：沉默摘要前多一行「脈動」（近 7/30 天 commit 數＋最新一則主旨）——資訊不是問句。
//! 超過 3 條問句＝過濾閾值錯了（照樣印，但標警語——這是免費校準資料）。
//! 產出同時寫 `groups/<g>/briefs/` 並落 presented 事件（教練規格：出手即留痕）。
//! v1.2（UX 檢討 §6.1／§6.2）：
//! - 行動文案＝實際行為：defer 一律叫「七天後再提醒」，不叫「本週排進度」（它沒有排任何進度）；
//! - 未結依所選組過濾（[`scope_loops`]），全域／未分類獨立一段——畫面宣告的組別要跟內容一致；
//! - 沉默摘要只講採集支持得起的話：「未觸發提醒條件」，不宣稱「無變化」。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde::Serialize;
use serde_json::{json, Value};

use crate::collect::{self, RepoSpec, RepoState};
use crate::config::{self, Thresholds};
use crate::registry::{self, Repo};
use crate::spine::{self, Event, EventType, Query};
use crate::Result;

/// 問句種類（機器用的 key；序列化字串同 Python 的 qkind）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QKind {
    Dirty,
    Inactive,
    Broken,
}

impl QKind {
    pub fn as_str(self) -> &'static str {
        match self {
            QKind::Dirty => "dirty",
            QKind::Inactive => "inactive",
            QKind::Broken => "broken",
        }
    }
    /// 人話（§8：qkind 是機器用的 key，不丟給人看）。
    pub fn name(self) -> &'static str {
        match self {
            QKind::Dirty => "未提交檔案放太久",
            QKind::Inactive => "長期沒有 commit",
            QKind::Broken => "採集失敗",
        }
    }
}

/// 對應 `brief.qkind_name`：認得的 qkind 給人話，不認得的原樣回。
pub fn qkind_name(qkind: &str) -> String {
    match qkind {
        "dirty" => QKind::Dirty.name().into(),
        "inactive" => QKind::Inactive.name().into(),
        "broken" => QKind::Broken.name().into(),
        other => other.into(),
    }
}

/// 卡上的一個出口（label＋引擎原語＋payload）。對應 `brief._act`。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CardAction {
    pub label: String,
    pub action: String,
    /// 自由形狀（各原語各自的參數，鍵序同 Python）。
    pub payload: Value,
}

fn act(label: &str, action: &str, payload: Value) -> CardAction {
    CardAction {
        label: label.into(),
        action: action.into(),
        payload,
    }
}

/// 問句卡（2026-09-02 UI 檢討 P0：出口是按鈕不是字）。序列化鍵同 Python dict。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QuestionCard {
    /// 固定 "question"。
    pub kind: &'static str,
    pub qkind: QKind,
    pub repo: String,
    pub key: String,
    pub title: String,
    pub actions: Vec<CardAction>,
}

/// 對應 `brief.snoozed`：defer 留痕（`chosen [monitor] repo:<id>` body `snooze:<qkind> until:<date>`）
/// → {(repo, qkind)}。`today` 預設今天（'YYYY-MM-DD'）。沒有 repo token 的以空字串表示（Python 是 None）。
pub fn snoozed(
    spine_dir: impl AsRef<Path>,
    today: Option<&str>,
) -> Result<HashSet<(String, String)>> {
    let today = today
        .map(str::to_string)
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string());
    let mut out = HashSet::new();
    for ev in spine::query(spine_dir, &Query::of_type(EventType::Chosen))? {
        // 人話寫在第一行、snooze: token 可能在任一行（舊事件在第一行，兩種都認）
        let Some(head) = crate::pystr::splitlines(&ev.body)
            .into_iter()
            .find(|ln| ln.starts_with("snooze:"))
            .map(|ln| ln.split_whitespace().collect::<Vec<_>>())
        else {
            continue;
        };
        let Some(first) = head.first() else { continue };
        let qkind = &first["snooze:".len()..];
        let until = head
            .iter()
            .find_map(|h| h.strip_prefix("until:"))
            .unwrap_or("");
        if until >= today.as_str() {
            out.insert((ev.kv("repo").unwrap_or("").to_string(), qkind.to_string()));
        }
    }
    Ok(out)
}

/// 對應 `brief.build_question_cards`：每個觸發閾值的 repo 一張卡（title＝問句、actions＝出口）。
/// 文字版（簡報 md、CLI）一律由 [`card_text`] 渲染而來，兩邊不會分岔。
pub fn build_question_cards(states: &[RepoState], th: &Thresholds) -> Vec<QuestionCard> {
    let mut cards = Vec::new();
    for s in states {
        let rid = s.id.as_str();
        let dirty_stale = s
            .dirty_days
            .is_some_and(|d| d >= th.dirty_stale_days as f64);
        let inactive = s.tier == "active"
            && s.last_commit_days
                .is_some_and(|d| d >= th.inactive_days as f64);
        if dirty_stale {
            let days = s.dirty_days.unwrap_or_default();
            cards.push(QuestionCard {
                kind: "question",
                qkind: QKind::Dirty,
                repo: rid.into(),
                key: format!("q:dirty:{rid}"),
                title: format!(
                    "{rid} 有 {} 個未提交檔案、最舊已 {days:.0} 天，本週要處理嗎？",
                    s.dirty
                ),
                actions: vec![
                    act(
                        "與 Agent 一起收尾",
                        "term_create",
                        json!({"kind": "agent", "repo": rid, "repos": [rid],
                               "task": format!("收尾 {rid} 的 {} 個未提交檔：先看 git status 與 diff，\
                                                判斷該 commit、丟棄或拆開，跟我確認後執行", s.dirty),
                               "origin": format!("q:dirty:{rid}")}),
                    ),
                    act(
                        "七天後再提醒",
                        "defer",
                        json!({"id": rid, "qkind": "dirty", "days": 7}),
                    ),
                ],
            });
        } else if inactive {
            let days = s.last_commit_days.unwrap_or_default();
            cards.push(QuestionCard {
                kind: "question",
                qkind: QKind::Inactive,
                repo: rid.into(),
                key: format!("q:inactive:{rid}"),
                title: format!(
                    "{rid} 標記為進行中，但已 {days:.0} 天沒有 commit，還算進行中嗎？"
                ),
                actions: vec![
                    // 主要動作由「要完成的工作」決定，不是維護登記狀態
                    // （2026-09-12 檢討 §3：降 tier 不該拿到最強的視覺優先權）
                    act(
                        "看這段期間的變化",
                        "term_create",
                        json!({"kind": "agent", "repo": rid, "repos": [rid],
                               "task": format!("{rid} 標記為進行中，但已 {days:.0} 天沒有 commit：\
                                                先看 git log、未合併分支與工作區，說明這段期間實際發生什麼、\
                                                現在卡在哪，再跟我確認要繼續、暫停還是改標記為休眠"),
                               "origin": format!("q:inactive:{rid}")}),
                    ),
                    act(
                        "改標記為休眠",
                        "tier",
                        json!({"id": rid, "tier": "dormant"}),
                    ),
                    act(
                        "七天後再提醒",
                        "defer",
                        json!({"id": rid, "qkind": "inactive", "days": 7}),
                    ),
                ],
            });
        }
        if !s.exists || s.note == "非 git repo" {
            let note = if s.note.is_empty() {
                "路徑不存在"
            } else {
                s.note.as_str()
            };
            cards.push(QuestionCard {
                kind: "question",
                qkind: QKind::Broken,
                repo: rid.into(),
                key: format!("q:broken:{rid}"),
                title: format!("{rid} 採集失敗（{note}），registry 要修嗎？"),
                actions: vec![
                    act(
                        "開 Shell 修登記",
                        "term_create",
                        json!({"kind": "shell", "origin": format!("q:broken:{rid}")}),
                    ),
                    act("移出 registry", "remove_repo", json!({"id": rid})),
                    act(
                        "先略過，七天後再提醒",
                        "defer",
                        json!({"id": rid, "qkind": "broken", "days": 7}),
                    ),
                ],
            });
        }
    }
    cards
}

/// 對應 `brief.card_text`：卡 → 一行文字（問句 → 〔出口〕〔出口〕），簡報 md 與 CLI 用。
pub fn card_text(card: &QuestionCard) -> String {
    let outs: String = card
        .actions
        .iter()
        .map(|a| format!("〔{}〕", a.label))
        .collect();
    if outs.is_empty() {
        card.title.clone()
    } else {
        format!("{} → {outs}", card.title)
    }
}

/// 對應 `brief.build_questions`。
pub fn build_questions(states: &[RepoState], th: &Thresholds) -> Vec<String> {
    build_question_cards(states, th)
        .iter()
        .map(card_text)
        .collect()
}

/// 對應 `brief.scope_loops`：未結依工作組分流 → (本組, 全域或未分類)。別組的事項一律不進來。
///
/// 2026-09-12 檢討 §6.2：簡報直接吃 `open_loops()`，「知識研究」的簡報會列出「產品開發」的待辦——
/// 介面宣告的組別與內容不一致。範圍判定改走 `spine::in_scope`，跟情境卡、工作摘要同一份規則；
/// 全域／未分類要保留就獨立標示，不併進本組。
pub fn scope_loops<S: AsRef<str>>(
    spine_dir: impl AsRef<Path>,
    group_name: &str,
    group: Option<&str>,
    ids: &[S],
) -> Result<(Vec<Event>, Vec<Event>)> {
    let scoped = !group_name.is_empty() && group_name != "全部";
    let gkey = group.filter(|g| !g.is_empty()).unwrap_or(group_name);
    let mut own = Vec::new();
    let mut loose = Vec::new();
    for ev in spine::open_loops(spine_dir)? {
        if !scoped || spine::in_scope(&ev, Some(gkey), ids) {
            own.push(ev);
        } else if spine::unscoped(&ev) {
            loose.push(ev);
        }
    }
    Ok((own, loose))
}

/// 只列到期與新增（其餘不吵人）。
fn loop_lines(loops: &[Event], today: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for ev in loops {
        let num = ev.num().unwrap_or("#?");
        let mut flag = String::new();
        match ev.kv("due").filter(|d| !d.is_empty()) {
            Some(due) if due <= today => {
                flag = if due == today {
                    "今天到期".into()
                } else {
                    format!("已逾期（{due}）")
                };
            }
            _ if ev.date == today => flag = "今日新增".into(),
            _ => {}
        }
        if !flag.is_empty() {
            lines.push(format!(
                "- {num}「{}」{flag} → 〔開碰撞〕〔延期〕〔關閉〕",
                ev.first_line()
            ));
        }
    }
    lines
}

/// registry 登記項 → 採集輸入（見 `impl From<&Repo> for RepoSpec`）。
pub(crate) fn repo_specs(entries: &[Repo]) -> Vec<RepoSpec> {
    entries.iter().map(RepoSpec::from).collect()
}

/// 對應 `brief.build_brief`：三段結構的簡報 markdown。`group`＝存起來的組名（None＝臨時／全部）。
pub fn build_brief(
    spine_dir: impl AsRef<Path>,
    group_name: &str,
    states: &[RepoState],
    when: Option<NaiveDateTime>,
    group: Option<&str>,
) -> Result<String> {
    let spine_dir = spine_dir.as_ref();
    let th = config::load(spine_dir)?.thresholds();
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let cards = build_question_cards(states, &th);
    let qs: Vec<String> = cards.iter().map(card_text).collect();
    let today = now.format("%Y-%m-%d").to_string();
    let ids: Vec<&str> = states.iter().map(|s| s.id.as_str()).collect();
    let (own, loose) = scope_loops(spine_dir, group_name, group, &ids)?;
    let own_lines = loop_lines(&own, &today);
    let loose_lines = loop_lines(&loose, &today);
    let flagged: HashSet<&str> = cards.iter().map(|c| c.repo.as_str()).collect();
    let quiet = states.len().saturating_sub(flagged.len());
    let mut lines = vec![
        format!("# 早晨簡報 {today}（{group_name} 組）"),
        String::new(),
    ];
    lines.push("## 需要你判斷的（每條一個問句）".into());
    if qs.is_empty() {
        lines.push("（今天沒有需要判斷的事）".into());
    } else {
        lines.extend(
            qs.iter()
                .enumerate()
                .map(|(i, q)| format!("{}. {q}", i + 1)),
        );
        if qs.len() > 3 {
            lines.push(String::new());
            lines.push(format!(
                "> ⚠ 問句 {} 條 > 3——過濾閾值該修了（config.yaml thresholds）",
                qs.len()
            ));
        }
    }
    lines.push(String::new());
    lines.push("## 未結（open loops，本組，只列到期與新增）".into());
    if own_lines.is_empty() {
        lines.push("（無到期或新增）".into());
    } else {
        lines.extend(own_lines);
    }
    if !loose_lines.is_empty() {
        lines.push(String::new());
        lines.push("## 未結（全域／未分類，不屬於本組，只列到期與新增）".into());
        lines.extend(loose_lines);
    }
    lines.push(String::new());
    lines.push("## 脈動（一行）".into());
    lines.push(collect::pulse_line(&collect::group_pulse(states)));
    // 沉默摘要只說採集撐得住的話：沒觸發閾值 ≠ 沒有變化
    lines.push(String::new());
    lines.push("## 沉默摘要（一行）".into());
    lines.push(format!("其餘 {quiet} 個 repo 本次採集未觸發提醒條件。"));
    lines.push(String::new());
    Ok(lines.join("\n"))
}

/// 對應 `brief.run`：採集 → 產簡報 → 寫 `groups/<g|adhoc>/briefs/<日期>.md` → 落 presented 事件。
/// 回傳 (簡報檔路徑, 簡報文字)。`repos` 給定（非空）＝臨時組合。
pub fn run(
    spine_dir: impl AsRef<Path>,
    group: Option<&str>,
    repos: Option<&[String]>,
    when: Option<NaiveDateTime>,
) -> Result<(PathBuf, String)> {
    let spine_dir = spine_dir.as_ref();
    let group = group.filter(|g| !g.is_empty());
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let (gname, entries) = registry::resolve_group(spine_dir, group, repos)?;
    let states = collect::collect_group(&repo_specs(&entries), None);
    let text = build_brief(spine_dir, &gname, &states, Some(now), group)?;
    let out = spine_dir
        .join("groups")
        .join(group.unwrap_or("adhoc"))
        .join("briefs")
        .join(format!("{}.md", now.format("%Y-%m-%d")));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &text)?;
    let n_q = build_question_cards(&states, &config::load(spine_dir)?.thresholds()).len();
    let ids: Vec<&str> = states.iter().map(|s| s.id.as_str()).collect();
    let (own, loose) = scope_loops(spine_dir, &gname, group, &ids)?;
    let mut body = format!("呈現了：判斷 {n_q} 條、本組未結 {} 條", own.len());
    if !loose.is_empty() {
        body.push_str(&format!("、全域／未分類 {} 條", loose.len()));
    }
    let fname = out
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    body.push_str(&format!(" → {fname}"));
    let tokens: Vec<String> = group.map(|g| format!("group:{g}")).into_iter().collect();
    spine::append_event(
        spine_dir,
        "presented",
        "morning-brief",
        &tokens,
        &body,
        Some(now),
    )?;
    Ok((out, text))
}
