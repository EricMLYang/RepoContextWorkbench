//! 工作台的畫面資料：`get_state`（輕量，純檔案讀）、`get_scan`（跑採集）、收件匣卡片、會話清冊。
//! 對應 Python `webui/state.py`（＋ `webui/server.py` 的 `/api/state` 組裝）。
//!
//! 收件匣＝一種卡（2026-09-02 UI 檢討）：出口是真按鈕→呼叫引擎原語→落 chosen 後卡消失；
//! 自己丟的碰撞＝「處理中」卡，回程原地換判定卡；自己出手的留痕（presented/chosen/monitor decision）
//! 不進收件匣。會話回報（2026-09-12 檢討 §5）：程序存活只代表程序活著，「工作是否推進」只認脊椎留痕。
//!
//! 範圍歸屬（provenance）只讀 token，不從內文猜；認不出來就老實放「未分類」。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde_json::{json, Map, Value};

use crate::pystr::splitlines;
use crate::registry::RegistryData;
use crate::spine::{self, Event, EventType};
use crate::term::TermInfo;
use crate::{brief, collect, config, notify, registry, status, summary};
use crate::{Error, Result};

/// 自己出手的留痕：不是要處理的事，不進收件匣（深看的事件表仍看得到）。
const OWN_RECORD_TYPES: [EventType; 2] = [EventType::Presented, EventType::Chosen];
/// cli＝使用者自己從 CLI 改 registry／組（ops 留痕）；agent 改的（[agent]）照常進收件匣。
const OWN_RECORD_SOURCES: [&str; 5] = ["monitor", "cli", "route", "spawn", "morning-brief"];
/// 會話回報不算的型別：presented/chosen 是工作台自己的簿記。
const REPORT_SKIP_TYPES: [EventType; 2] = [EventType::Presented, EventType::Chosen];
/// 臨時組合的 group token 前綴（`group:臨時(a,b)`）。
const ADHOC_PREFIX: &str = "臨時(";

/// 會話起手任務「整理本組現況」的文案（原本在前端 `STATUS_PROMPT`）。
pub const STATUS_PROMPT: &str =
    "先讀情境卡，用 3 行說明：你拿到哪些脈絡、這次預計產出什麼、哪些動作需要我先確認，再問我要做什麼。";

// ── 小工具 ─────────────────────────────────────────────────────────────

/// 對應 `state._ev_dict`。
pub fn ev_dict(ev: &Event) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("date".into(), json!(ev.date));
    m.insert("time".into(), json!(ev.time));
    m.insert("type".into(), json!(ev.etype.as_str()));
    m.insert("source".into(), json!(ev.source));
    m.insert("tokens".into(), json!(ev.tokens));
    m.insert("body".into(), json!(ev.body));
    m.insert("header".into(), json!(ev.header()));
    m
}

/// 對應 `state._act`：卡上的一個出口。
fn act(label: &str, action: &str, payload: Value) -> Value {
    json!({"label": label, "action": action, "payload": payload})
}

/// body 第一行；沒有 body 用 tokens 串起來（Python 各處的 `first = ... if ev.body else " ".join(ev.tokens)`）。
fn first_or_tokens(ev: &Event) -> String {
    if ev.body.is_empty() {
        ev.tokens.join(" ")
    } else {
        splitlines(&ev.body)
            .first()
            .copied()
            .unwrap_or("")
            .to_string()
    }
}

/// `group:臨時(a,b)` → `Some(["a","b"])`；不是臨時組合回 `None`。
fn adhoc_members(group: &str) -> Option<Vec<String>> {
    group
        .strip_prefix(ADHOC_PREFIX)
        .and_then(|g| g.strip_suffix(')'))
        .map(|inner| inner.split(',').map(str::to_string).collect())
}

/// 對應 `state._parse_judgement`：判定卡 body（collide 寫的五欄位）→ 有序 map；不是判定 body 回 None。
pub fn parse_judgement(body: &str) -> Option<Map<String, Value>> {
    if !body.starts_with("判定：") {
        return None;
    }
    let mut j = Map::new();
    for ln in splitlines(body) {
        if ln.contains('：') && !ln.starts_with('〔') {
            let (k, v) = ln.split_once('：').unwrap_or((ln, ""));
            j.insert(k.trim().to_string(), json!(v.trim()));
        }
    }
    Some(j)
}

fn jstr<'a>(j: &'a Map<String, Value>, k: &str) -> &'a str {
    j.get(k).and_then(Value::as_str).unwrap_or("")
}

/// 對應 `state.normalize_dest`：落點建議字串 → route dest（`repo:<id>`｜`group:<g>`｜`incubator`）。解不出回 None。
pub fn normalize_dest(text: Option<&str>) -> Option<String> {
    let t = text.unwrap_or("").trim();
    if t.is_empty() {
        return None;
    }
    if t.starts_with("incubator") {
        return Some("incubator".into());
    }
    for prefix in ["repo:", "group:"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            let ident = rest
                .split('/')
                .next()
                .unwrap_or("")
                .split(' ')
                .next()
                .unwrap_or("")
                .trim();
            return (!ident.is_empty()).then(|| format!("{prefix}{ident}"));
        }
    }
    None
}

/// 對應 `state._handled_keys`：被 chosen ref 到的事件 key（`collision:<cid>` 或 `<type>:<HH:MM>`）＝已處理。
fn handled_keys(events: &[Event]) -> HashSet<String> {
    events
        .iter()
        .filter(|e| e.etype == EventType::Chosen)
        .filter_map(|e| e.kv("ref").filter(|r| !r.is_empty()).map(str::to_string))
        .collect()
}

/// `collision_index` 的一筆。
#[derive(Debug, Clone, Default)]
pub struct CollisionRec {
    pub idea: String,
    pub group: Option<String>,
    pub opened: Option<Event>,
    pub answered: bool,
}

/// 對應 `state._collision_index`：cid → {idea, group, opened, answered}（依 cid 排序）。
pub fn collision_index(events: &[Event]) -> BTreeMap<String, CollisionRec> {
    let mut idx: BTreeMap<String, CollisionRec> = BTreeMap::new();
    for ev in events.iter().filter(|e| e.etype == EventType::Collision) {
        let Some(cid) = ev.kv("id").filter(|c| !c.is_empty()) else {
            continue;
        };
        let rec = idx.entry(cid.to_string()).or_default();
        if ev.body.starts_with("opened") {
            rec.opened = Some(ev.clone());
            rec.idea = match ev.body.split_once("輸入：") {
                Some((_, idea)) => idea.trim().to_string(),
                None => ev.body.trim().to_string(),
            };
            rec.group = ev.kv("group").map(str::to_string);
        } else {
            rec.answered = true;
        }
    }
    idx
}

/// 對應 `state._collision_scope`：opened 事件的 group token → term_create 的 group/repos payload。
fn collision_scope(group: Option<&str>) -> Map<String, Value> {
    let mut m = Map::new();
    match group.filter(|g| !g.is_empty()) {
        None => {}
        Some(g) => match adhoc_members(g) {
            Some(repos) => {
                m.insert("repos".into(), json!(repos));
            }
            None => {
                m.insert("group".into(), json!(g));
            }
        },
    }
    m
}

mod cards;
mod scope;
mod sessions;

pub use cards::*;
pub use scope::*;
pub use sessions::*;

// ── get_state ──────────────────────────────────────────────────────────

/// 畫面目前選的範圍（`get_state` 的參數；跟 `get_scan` 同語意：`repos` 優先於 `group`，都沒有＝全部）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateScope {
    pub group: Option<String>,
    pub repos: Option<Vec<String>>,
}

impl StateScope {
    pub fn new(group: Option<&str>, repos: Option<&[String]>) -> Self {
        StateScope {
            group: group.filter(|g| !g.is_empty()).map(str::to_string),
            repos: repos.map(<[String]>::to_vec),
        }
    }
    fn scoped(&self) -> bool {
        self.group.is_some() || self.repos.is_some()
    }
    /// 範圍已經不存在（組被刪或改名、臨時組合的 repo 全被移除）。Rust 才有（2026-10-13，原本前端判斷）。
    fn gone(&self, data: &RegistryData) -> bool {
        if let Some(repos) = &self.repos {
            return !repos.iter().any(|r| data.repo(r).is_some());
        }
        self.group.as_ref().is_some_and(|g| data.group(g).is_none())
    }
    /// 範圍內的 repo id（臨時組合只留登記過的；不存在的組＝空）。
    fn members(&self, data: &RegistryData) -> Vec<String> {
        if let Some(repos) = &self.repos {
            return repos
                .iter()
                .filter(|r| data.repo(r).is_some())
                .cloned()
                .collect();
        }
        if let Some(g) = &self.group {
            return data.group(g).map(|g| g.members()).unwrap_or_default();
        }
        data.repos.iter().map(|r| r.id()).collect()
    }
}

/// 關係 → 「用這條關係開工作」的任務文案（原本在前端 `relationTask()`）。
pub fn relation_task(kind: &str, a: &str, b: &str) -> String {
    match kind {
        "pm-of" => format!("檢查規劃與實作是否一致：{a} 是 {b} 的 PM。請比對 {a} 的規劃／需求文件與 {b} 的實作與驗收條件，列出不一致、缺漏與已過期的部分，每條標明來自哪個 repo 的哪份檔，再跟我確認要改哪一邊。"),
        "feeds" => format!("檢查供料是否還新：{a} 供料給 {b}。請確認 {b} 目前用的內容是不是 {a} 的最新版本，列出落差與需要重新供料的部分，標明來源檔案。"),
        "derived-from" => format!("檢查衍生落差：{a} 衍生自 {b}。請比對兩邊自分家後的差異，說明哪些該回流、哪些是刻意分開，標明來源檔案。"),
        "upstream-of" => format!("檢查上游影響：{a} 是 {b} 的上游。請看 {a} 近期變更有沒有影響 {b}，列出需要跟進的項目與依據。"),
        "sibling-topic" => format!("比對同主題內容：{a} 與 {b} 講同一件事。請找出重複、矛盾與只有一邊有的內容，標明來源檔案，再建議怎麼收斂。"),
        _ => format!("釐清 {a} 與 {b} 的關係現在是否還成立：比對兩邊近期內容，說明依據，再建議要不要調整這條關係。"),
    }
}

/// `registry.relation_kinds` 的 dict 形狀。Python 的內建詞彙只有 sibling-topic 寫了 `symmetric`，
/// 其餘沒有這個鍵；config 擴充的一律帶。這裡照抄：沒被 config 改過的內建、且非對稱的，不帶 `symmetric`。
fn rel_kinds_json(kinds: &registry::RelationKinds) -> Value {
    let builtin = registry::builtin_relation_kinds();
    let mut m = Map::new();
    for (k, v) in &kinds.0 {
        let mut o = Map::new();
        o.insert("forward".into(), json!(v.forward));
        o.insert("inverse".into(), json!(v.inverse));
        if v.symmetric || builtin.get(k) != Some(v) {
            o.insert("symmetric".into(), json!(v.symmetric));
        }
        m.insert(k.clone(), Value::Object(o));
    }
    Value::Object(m)
}

/// 對應 `state.build_state`：輕量狀態（純檔案讀）。interrupt 先於 normal（打斷要掙得，其餘累積）。
///
/// `scope`＝畫面目前選的範圍：每張卡、每筆事件多帶 `lane`，每條關係多帶 `in_scope`／`task`，
/// 頂層多帶 `scope{group, repos, members}`——前端照欄位分欄，不自己判斷範圍歸屬。
pub fn build_state(spine_dir: impl AsRef<Path>, scope: &StateScope) -> Result<Value> {
    let spine_dir = spine_dir.as_ref();
    let data = registry::load(spine_dir)?;
    let p = notify::pending(spine_dir)?;
    let events = spine::iter_events(spine_dir)?;
    let loops = spine::open_loops_of(&events);
    let scopes = event_scopes(&events, &data);
    let mut vocab = config::load(spine_dir)?.tag_suggestions();
    for t in registry::all_tags(spine_dir)? {
        if !vocab.contains(&t) {
            vocab.push(t);
        }
    }
    let scoped = scope.scoped();
    let members = scope.members(&data);

    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by(|&a, &b| {
        (&events[b].date, &events[b].time, b).cmp(&(&events[a].date, &events[a].time, a))
    });
    let recent: Vec<Value> = order
        .into_iter()
        .take(200)
        .map(|i| {
            let mut m = ev_dict(&events[i]);
            scopes[i].put(&mut m);
            m.insert(
                "lane".into(),
                json!(lane(&scopes[i], scoped, &members, false)),
            );
            Value::Object(m)
        })
        .collect();

    let mut cards = build_cards(&events, &p, &loops, &scopes);
    attach_review(&mut cards, &crate::agentapi::load_review_cache(spine_dir));
    add_brief_action(&mut cards);
    escalate_defer(&mut cards, &events);
    cards.extend(reopen_cards(spine_dir, &events, &scopes));
    cards.extend(retro_cards(spine_dir, &events, &scopes));
    cards.extend(fresh_cards(spine_dir, &data)?);
    mark_primary(&mut cards);
    for c in &mut cards {
        let l = lane(&prov_of(c), scoped, &members, c["kind"] == "interrupt");
        if let Some(o) = c.as_object_mut() {
            o.insert("lane".into(), json!(l));
        }
    }

    let relations: Vec<Value> = registry::relations(spine_dir, None)?
        .into_iter()
        .map(|r| {
            let mut v = serde_json::to_value(&r).unwrap_or(Value::Null);
            let inside = members.contains(&r.from) || members.contains(&r.to);
            if let Some(o) = v.as_object_mut() {
                o.insert("in_scope".into(), json!(inside));
                o.insert("task".into(), json!(relation_task(&r.kind, &r.from, &r.to)));
            }
            v
        })
        .collect();
    // Rust 才有（F-47）：關係建議（反芻時算好的快取；純檔案讀）——碰到目前範圍、還沒設關係的。
    // 沒有建議時不放這個鍵（舊版前端與對拍都看不到差異）。
    let related: HashSet<(String, String)> = registry::relations(spine_dir, None)?
        .into_iter()
        .map(|r| crate::relsuggest::pair(&r.from, &r.to))
        .collect();
    let suggestions: Vec<Value> = crate::relsuggest::load(spine_dir)
        .into_iter()
        .filter(|x| !related.contains(&crate::relsuggest::pair(&x.a, &x.b)))
        .filter(|x| !scoped || members.contains(&x.a) || members.contains(&x.b))
        .map(|x| {
            let (from, to) = match &x.hint {
                Some((f, t, _)) => (f.clone(), t.clone()),
                None if x.a_mentions_b >= x.b_mentions_a => (x.a.clone(), x.b.clone()),
                None => (x.b.clone(), x.a.clone()),
            };
            json!({"from": from, "to": to, "kind": x.hint.as_ref().map(|h| h.2.clone()),
                   "reason": x.reason(), "files": x.files})
        })
        .collect();

    let mut unread: Vec<Value> = p
        .interrupt
        .iter()
        .map(|e| {
            let mut m = ev_dict(e);
            m.insert("interrupt".into(), json!(true));
            Value::Object(m)
        })
        .collect();
    unread.extend(p.normal.iter().map(|e| Value::Object(ev_dict(e))));

    let mut stats = serde_json::to_value(spine::stats(spine_dir)?)?;
    if let (Some(o), Value::Object(s)) = (
        stats.as_object_mut(),
        serde_json::to_value(registry::survival(spine_dir)?)?,
    ) {
        o.extend(s);
    }

    let groups: Vec<Value> = data
        .groups
        .iter()
        .map(|g| {
            json!({"name": g.name(), "members": g.members(),
                   "goal": g.get_str("goal").unwrap_or_default(),
                   "goal_at": g.get_str("goal_at").unwrap_or_default()})
        })
        .collect();
    let mut tiers = Map::new();
    let mut tags = Map::new();
    for r in &data.repos {
        let tier = r
            .get("tier")
            .map(config::yaml_to_json)
            .unwrap_or_else(|| json!("active"));
        tiers.insert(r.id(), tier);
        let t = r
            .get("tags")
            .map(config::yaml_to_json)
            .filter(truthy)
            .unwrap_or_else(|| json!([]));
        tags.insert(r.id(), t);
    }

    let mut out = json!({
        "groups": groups,
        "repos": data.repos.iter().map(|r| r.id()).collect::<Vec<_>>(),
        "tiers": tiers,
        "tags": tags,
        "tag_vocab": vocab,
        // 關係層（2026-09-08）：前端畫 ⇄、深看關係段都從這兩個欄位來
        "relations": relations,
        "rel_kinds": rel_kinds_json(&registry::relation_kinds(spine_dir)?),
        "unread": unread,
        "loops": loops.iter().map(|e| Value::Object(ev_dict(e))).collect::<Vec<_>>(),
        "cards": cards,
        "recent": recent,
        "stats": stats,
        "scope": {"group": scope.group, "repos": scope.repos, "members": members,
                  "gone": scope.gone(&data)},
    });
    if !suggestions.is_empty() {
        out["relation_suggestions"] = Value::Array(suggestions);
    }
    // Rust 才有（F-50）：健康指標（同 `ctx health`；只讀檔）——App 活動分頁最上方
    if let Ok(h) = crate::health::health(spine_dir, 14, None) {
        out["health"] = h;
    }
    Ok(out)
}

pub(crate) use crate::pystr::truthy;

/// 對應 `GET /api/state`（`server.py`）：[`build_state`] ＋ `terms`（會話清冊＋回報）＋ `agents`（config `agents` 的鍵）。
pub fn api_state(
    spine_dir: impl AsRef<Path>,
    scope: &StateScope,
    terms: &[TermInfo],
) -> Result<Value> {
    let spine_dir = spine_dir.as_ref();
    let mut st = build_state(spine_dir, scope)?;
    let tv: Vec<Value> = terms.iter().map(term_value).collect();
    let annotated = annotate_terms(spine_dir, &tv)?;
    let agents: Vec<String> = config::load(spine_dir)?
        .agents()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    if let Some(o) = st.as_object_mut() {
        o.insert("terms".into(), json!(annotated));
        o.insert("agents".into(), json!(agents));
    }
    Ok(st)
}

// ── get_scan ───────────────────────────────────────────────────────────

/// 對應 `state.build_scan`：P3 採集＋閾值問句卡＋audit（開頁/切組/勾選/手動才跑，因為會打 git）。
/// `repos` 給定＝臨時組合（P2 免建組），優先於 group；空陣列一律拒絕（空範圍不代表全部）。
///
/// 多帶（Rust／Python 同步加）：問句卡的 provenance（`scope_kind: "repo"`）、`session_tasks`（開會話的起手任務）。
pub fn build_scan(
    spine_dir: impl AsRef<Path>,
    group: Option<&str>,
    repos: Option<&[String]>,
) -> Result<Value> {
    let spine_dir = spine_dir.as_ref();
    if repos.is_some_and(<[String]>::is_empty) {
        return Err(Error::invalid("至少選擇一個 repo；空範圍不代表全部"));
    }
    let group = group.filter(|g| !g.is_empty());
    let cfg = config::load(spine_dir)?;
    let (gname, entries) = registry::resolve_group(spine_dir, group, repos)?;
    let states = collect::collect_group(&brief::repo_specs(&entries), Some(spine_dir));
    // Rust 才有（F-46）：scan 是重的那一層（本來就跑 git），順便更新這個範圍的未結體檢——
    // 結果寫進 .state/review.json，收件匣（純檔案讀）下次重整就看得到。失敗不影響 scan。
    // 沒選範圍（全部）＝逐組跑；不靠 cwd 推主場（App 行程的目錄不是任何 repo）
    let targets: Vec<crate::agentapi::Target> = if group.is_none() && repos.is_none() {
        registry::load(spine_dir)?
            .groups
            .iter()
            .map(|g| crate::agentapi::Target {
                cwd: None,
                group: Some(g.name()),
                repos: None,
            })
            .collect()
    } else {
        vec![crate::agentapi::Target {
            cwd: None,
            group: group.map(str::to_string),
            repos: repos.map(<[String]>::to_vec),
        }]
    };
    for t in &targets {
        let _ = crate::agentapi::review_todos(spine_dir, t);
    }
    let snoozed = brief::snoozed(spine_dir, None)?;
    let cards: Vec<brief::QuestionCard> = brief::build_question_cards(&states, &cfg.thresholds())
        .into_iter()
        .filter(|c| !snoozed.contains(&(c.repo.clone(), c.qkind.as_str().to_string())))
        .collect();
    let noisy: HashSet<&str> = cards.iter().map(|c| c.repo.as_str()).collect();
    let quiet = states.len().saturating_sub(noisy.len());
    let summary = summary::build_summary(
        spine_dir,
        group,
        repos,
        Some(states.clone()),
        Some(cards.clone()),
        None,
    )?;
    let ids: Vec<String> = entries.iter().map(|e| e.id()).collect();
    let mut session_tasks = vec![
        json!({"label": "接續上次工作", "text": summary.resume_task}),
        json!({"label": "整理本組現況", "text": STATUS_PROMPT}),
    ];
    if let Some(rel) = registry::relations(spine_dir, None)?
        .into_iter()
        .find(|r| ids.contains(&r.from) && ids.contains(&r.to))
    {
        let kinds = registry::relation_kinds(spine_dir)?;
        let fwd = kinds.forward(&rel.kind);
        let fwd = if fwd.is_empty() {
            rel.kind.as_str()
        } else {
            fwd
        };
        session_tasks.push(json!({
            "label": format!("{fwd}：{} → {}", rel.from, rel.to),
            "text": relation_task(&rel.kind, &rel.from, &rel.to),
        }));
    }
    let mut question_cards: Vec<Value> = cards
        .iter()
        .map(|c| {
            let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
            if let Some(o) = v.as_object_mut() {
                Prov {
                    kind: "repo",
                    label: c.repo.clone(),
                    repos: vec![c.repo.clone()],
                }
                .put(o);
            }
            v
        })
        .collect();
    mark_primary(&mut question_cards);
    let mut audit = registry::audit::<&Path>(spine_dir, &[])?;
    audit.extend(spine::lint(spine_dir, None)?);
    Ok(json!({
        "group": gname,
        "states": states,
        // 本組工作摘要（§3）：目標／上次進度／變化／可接續的一步，每格帶來源
        "summary": summary,
        // 活動脈動（2026-09-08）：組的監控焦點＝活動頻繁度＋近期 commit 內容
        "pulse": collect::group_pulse(&states),
        "recent_commits": collect::recent_across(&states, 30),
        "question_cards": question_cards,
        "questions": cards.iter().map(brief::card_text).collect::<Vec<_>>(),
        "quiet": quiet,
        "session_tasks": session_tasks,
        // registry 稽核＋脊椎衛生迴圈併同一個紅字面
        "audit": audit,
    }))
}
