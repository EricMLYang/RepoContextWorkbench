//! 組情境卡（2026-09-08 組為單位輪）——預設 agent 以「一組 repo」為工作單位的開場料。對應 `repo_context/context.py`。
//!
//! P16 會話料原本只有文件地圖（路徑＋行數）；agent 不知道這組是誰、誰管誰、最近在動什麼、
//! 有什麼未結，開場就只能問「要做什麼」。情境卡把 registry（成員、tier、關係）＋ P3 脈動
//! ＋ 脊椎（本組 loops／最近事件）壓成一屏，外加**組管家角色說明**（可由 config `session.role_prompt` 覆寫）。
//! 純函數：`build_context(spine, group|repos) -> String`；CLI `group context`、MCP `group_context` 同源。

use std::path::Path;

use chrono::{Local, NaiveDateTime};

use crate::collect::{self, RepoSpec, RepoState};
use crate::registry::{self, Direction, RelationView, Repo};
use crate::spine::{self, Event, EventType};
use crate::{config, Result};

/// 對應 `context.ROLE_PROMPT`。
pub const ROLE_PROMPT: &str = "你是這組 repo 的**組管家**（group steward）——工作單位是「這一組」，不是單一 repo。

## 開場先講三件事（各一行，講完再問我要做什麼）
1. 你拿到的脈絡：成員與關係、脈動、未結、最近事件——有缺就直接說缺。
2. 這次預計產出什麼：一句話講清楚交付物。
3. 哪些動作要我先確認：改檔、跨組、刪除、對外送出，一律先問。

## 各階段的責任（同一個你，不同時機）
- 進入工作組：整理變化、接回上次工作 → 我要看到現況、來源、可繼續的任務。
- 出現問題：比對相關 repo、辨認缺漏 → 我要看到判斷理由、還不知道什麼、建議下一步。
- 執行任務：在約定範圍內工作、遇到阻塞立刻回報 → 我要知道你在處理什麼、是否輪到我回應。
- 結束工作：整理成果與未完成事項 → 我要能檢閱成果，並知道下次從哪接回。

## 範圍、行為約定、實際權限是三件事
- **範圍**：本卡列出的成員就是這次的工作範圍。
- **行為約定**：不越組改別組的東西、要動組外 repo 先問——這是約定，靠你遵守。
- **實際權限**：本卡不是工具層的權限限制；你手上的工具碰得到的檔案不只本組。
  正因為沒有硬性阻擋，越界之前一定要先問。
- 尊重關係：PM repo 管方向、code repo 管實作。

## 工作方式
- 先讀完本情境卡再動手；需要細節再用文件地圖挑檔讀，不要整包吞。
- 講結論標明來自哪個 repo 哪份檔；跨 repo 的彙整產物落 spine 的 groups/<組>/materials/。
- 出手留痕（MCP repo-context 工具，或 `ctx <指令> --json`）：做了判斷 `log_decision`、
  發現待辦 `add_todo`、處理完 `close_todo`、等我回應或卡住 `report_status`。
  留痕是工作台唯一看得到你進度的依據——沒留痕＝沒回報。
- 隨時可用 `context_for` 重抓脈絡、`next_work` 看該做什麼、`search_knowledge` 找其他 repo 的相關文件。
- 收尾一定呼叫 `handoff`：完成了什麼、還剩什麼、下次從哪裡接回。
- 無事不報：沒有異常就一句話帶過，不要把正常狀態講成待辦。
";

/// 範圍只有一個 repo 時的角色說明（Rust 才有，2026-10-15，#73）：不講「組」與成員關係。
/// 只用在會話料（`session::build`）；CLI `group context`／MCP `group_context` 照舊用 [`ROLE_PROMPT`]（跟 Python 對拍）。
pub const ROLE_PROMPT_SOLO: &str = "你是這個 repo 的**repo 管家**——這次的工作範圍就是這一個 repo。

## 開場先講三件事（各一行，講完再問我要做什麼）
1. 你拿到的脈絡：repo 現況、脈動、未結、最近事件——有缺就直接說缺。
2. 這次預計產出什麼：一句話講清楚交付物。
3. 哪些動作要我先確認：改檔、動其他 repo、刪除、對外送出，一律先問。

## 各階段的責任（同一個你，不同時機）
- 開場：整理變化、接回上次工作 → 我要看到現況、來源、可繼續的任務。
- 出現問題：查清楚、辨認缺漏 → 我要看到判斷理由、還不知道什麼、建議下一步。
- 執行任務：在這個 repo 裡工作、遇到阻塞立刻回報 → 我要知道你在處理什麼、是否輪到我回應。
- 結束工作：整理成果與未完成事項 → 我要能檢閱成果，並知道下次從哪接回。

## 範圍
- 本卡的 repo 就是這次的工作範圍；要讀別的 repo 用 `search_knowledge`／`read_from`（會留引用），要改先問。
- 本卡不是工具層的權限限制；正因為沒有硬性阻擋，越界之前一定要先問。

## 工作方式
- 先讀完本情境卡再動手；需要細節再用文件地圖挑檔讀，不要整包吞。
- 出手留痕（MCP repo-context 工具，或 `ctx <指令> --json`）：做了判斷 `log_decision`、
  發現待辦 `add_todo`、處理完 `close_todo`、等我回應或卡住 `report_status`。
  留痕是工作台唯一看得到你進度的依據——沒留痕＝沒回報。
- 隨時可用 `context_for` 重抓脈絡、`next_work` 看該做什麼。
- 收尾一定呼叫 `handoff`：完成了什麼、還剩什麼、下次從哪裡接回。
- 無事不報：沒有異常就一句話帶過，不要把正常狀態講成待辦。
";

/// Python `f"{x:.0f}"`。
fn f0(x: f64) -> String {
    format!("{x:.0}")
}

fn member_line(entry: &Repo, state: Option<&RepoState>, rels_of: &[RelationView]) -> String {
    let rid = entry.id();
    let mut bits = vec![entry.tier_str().unwrap_or_else(|| "active".into())];
    let tags = entry.tags();
    if !tags.is_empty() {
        bits.push(tags.join("/"));
    }
    if entry.kind().as_deref() == Some("external") {
        bits.push(format!(
            "external{}",
            entry
                .upstream()
                .map(|u| format!(" ↑{u}"))
                .unwrap_or_default()
        ));
    }
    let rel_txt = rels_of
        .iter()
        .map(|r| {
            if r.direction == Direction::Out {
                format!("{}→ {}", r.label, r.peer)
            } else {
                format!("{} {}", r.label, r.peer)
            }
        })
        .collect::<Vec<_>>()
        .join("；");
    let mut act = String::new();
    match state {
        Some(s) if s.exists && s.repo_type == "mine" => {
            act = format!("近 7 天 {} commits", s.commits_7d.unwrap_or(0));
            if s.dirty > 0 {
                act.push_str(&format!("，未提交 {} 檔", s.dirty));
            }
            if let Some(c) = s.recent_commits.first() {
                act.push_str(&format!("｜最新 {}「{}」", c.date, c.subject));
            } else if let Some(d) = s.last_commit_days {
                act.push_str(&format!("｜末次 commit {} 天前", f0(d)));
            }
        }
        Some(s) if !s.exists => act = "路徑不存在".into(),
        _ => {}
    }
    let mut line = format!("- **{rid}**（{}）", bits.join("｜"));
    if !rel_txt.is_empty() {
        line.push_str(&format!("：{rel_txt}"));
    }
    if !act.is_empty() {
        line.push_str(&format!("\n  {act}"));
    }
    line
}

/// 範圍判定統一走 spine.in_scope（簡報／摘要／會話回報同一份規則）。
fn in_scope_of(ev: &Event, scoped: bool, gkey: &str, ids: &[String]) -> bool {
    if scoped {
        spine::in_scope(ev, Some(gkey), ids)
    } else {
        spine::in_scope::<&str>(ev, None, &[])
    }
}

/// 對應 `context.build_context`：組情境卡（成員與角色／脈動／未結／最近事件／你的角色）。
/// `group`、`repos` 都沒給＝全部；`states` 沒給就現場採集；`when` 沒給＝現在。
pub fn build_context(
    spine_dir: &Path,
    group: Option<&str>,
    repos: Option<&[String]>,
    states: Option<Vec<RepoState>>,
    when: Option<NaiveDateTime>,
) -> Result<String> {
    build_card(spine_dir, group, repos, states, when, false)
}

/// 會話料用的情境卡：範圍只有一個 repo 時改用 repo 的說法（標題、段名、角色），其餘同 [`build_context`]。
/// Rust 才有（2026-10-15，#73）。
pub fn build_session_card(
    spine_dir: &Path,
    group: Option<&str>,
    repos: Option<&[String]>,
    when: Option<NaiveDateTime>,
) -> Result<String> {
    build_card(spine_dir, group, repos, None, when, true)
}

fn build_card(
    spine_dir: &Path,
    group: Option<&str>,
    repos: Option<&[String]>,
    states: Option<Vec<RepoState>>,
    when: Option<NaiveDateTime>,
    solo_wording: bool,
) -> Result<String> {
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let cfg = config::load(spine_dir)?;
    let (gname, entries) = registry::resolve_group(spine_dir, group, repos)?;
    let ids: Vec<String> = entries.iter().map(Repo::id).collect();
    let scoped = group.is_some_and(|g| !g.is_empty()) || repos.is_some_and(|r| !r.is_empty());
    let gkey = group
        .filter(|g| !g.is_empty())
        .unwrap_or(&gname)
        .to_string();
    let states = match states {
        Some(s) => s,
        None => {
            let specs: Vec<RepoSpec> = entries.iter().map(RepoSpec::from).collect();
            collect::collect_group(&specs, Some(spine_dir))
        }
    };
    let solo = solo_wording && entries.len() == 1;
    let unit = if solo { "本 repo" } else { "本組" };
    let mut out = if solo {
        let mut head = vec![format!(
            "# 情境卡 {}（{}）",
            ids[0],
            now.format("%Y-%m-%d %H:%M")
        )];
        // 單一 repo 但屬於登記的牌組（例 MyProjects 只有它）：講一聲，留痕會掛在那個組
        if group.is_some_and(|g| !g.is_empty()) {
            head.push(format!("屬於牌組「{gname}」（這組目前只有它）"));
        }
        head.extend([String::new(), "## 這個 repo".to_string()]);
        head
    } else {
        vec![
            format!("# 組情境卡 {gname}（{}）", now.format("%Y-%m-%d %H:%M")),
            String::new(),
            "## 成員與角色".to_string(),
        ]
    };
    for e in &entries {
        let rels: Vec<RelationView> = registry::relations_of(spine_dir, &e.id())?
            .into_iter()
            .filter(|r| ids.contains(&r.peer))
            .collect();
        let st = states.iter().find(|s| s.id == e.id());
        out.push(member_line(e, st, &rels));
    }
    let outside: Vec<String> = registry::relation_lines(spine_dir, &ids)?
        .into_iter()
        .filter(|ln| ln.contains("組外"))
        .collect();
    if !outside.is_empty() {
        out.push(format!(
            "- {}：{}",
            if solo {
                "跟其他 repo 的關係"
            } else {
                "指向組外的關係"
            },
            outside.join("；")
        ));
    }
    out.extend([
        String::new(),
        "## 脈動".into(),
        collect::pulse_line(&collect::group_pulse(&states)),
    ]);
    let events = spine::iter_events(spine_dir)?;
    let loops: Vec<Event> = spine::open_loops_of(&events)
        .into_iter()
        .filter(|ev| in_scope_of(ev, scoped, &gkey, &ids))
        .collect();
    out.extend([String::new(), format!("## 未結（open loops，{unit}）")]);
    if loops.is_empty() {
        out.push("（無）".into());
    }
    for ev in &loops {
        let num = ev.num().unwrap_or("#?");
        let due = ev
            .kv("due")
            .filter(|d| !d.is_empty())
            .map(|d| format!("（due {d}）"))
            .unwrap_or_default();
        out.push(format!("- {num} {}{due}", ev.first_line()));
    }
    out.extend([String::new(), format!("## 最近事件（{unit}，最多 8 則）")]);
    let evs: Vec<&Event> = events
        .iter()
        .filter(|ev| ev.etype != EventType::Presented && in_scope_of(ev, scoped, &gkey, &ids))
        .collect();
    if evs.is_empty() {
        out.push("（無）".into());
    }
    for ev in evs.iter().skip(evs.len().saturating_sub(8)).rev() {
        let first = if ev.body.is_empty() {
            ev.tokens.join(" ")
        } else {
            ev.first_line().to_string()
        };
        out.push(format!(
            "- {} {} {} [{}] {first}",
            ev.date, ev.time, ev.etype, ev.source
        ));
    }
    let role = cfg
        .str("session.role_prompt")
        .filter(|s| !s.is_empty())
        .unwrap_or(if solo { ROLE_PROMPT_SOLO } else { ROLE_PROMPT });
    out.extend([
        String::new(),
        "## 你的角色".into(),
        role.trim_end().to_string(),
        String::new(),
    ]);
    Ok(out.join("\n"))
}
