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

// ── 範圍歸屬 ───────────────────────────────────────────────────────────

/// 範圍歸屬（provenance）：`scope_kind`（repo｜group｜global｜unclassified）、`scope_label`、`scope_repos`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prov {
    pub kind: &'static str,
    pub label: String,
    pub repos: Vec<String>,
}

impl Prov {
    fn unclassified() -> Self {
        Prov {
            kind: "unclassified",
            label: "未分類".into(),
            repos: Vec::new(),
        }
    }
    fn global() -> Self {
        Prov {
            kind: "global",
            label: "所有 repo".into(),
            repos: Vec::new(),
        }
    }
    /// 既非某組也非某 repo（全域或未分類）。
    pub fn unscoped(&self) -> bool {
        self.kind == "global" || self.kind == "unclassified"
    }
    fn put(&self, m: &mut Map<String, Value>) {
        m.insert("scope_kind".into(), json!(self.kind));
        m.insert("scope_label".into(), json!(self.label));
        m.insert("scope_repos".into(), json!(self.repos));
    }
}

struct ScopeResolver<'a> {
    events: &'a [Event],
    groups: HashMap<String, Vec<String>>,
    openings: HashMap<String, usize>,
    refs: HashMap<String, Vec<usize>>,
    cache: Vec<Option<Prov>>,
}

impl ScopeResolver<'_> {
    fn resolve(&mut self, i: usize, seen: &HashSet<usize>) -> Prov {
        if let Some(p) = &self.cache[i] {
            return p.clone();
        }
        if seen.contains(&i) {
            return Prov::unclassified();
        }
        let mut seen = seen.clone();
        seen.insert(i);
        let e = &self.events[i];
        let result = if let Some(rid) = e.kv("repo").filter(|r| !r.is_empty()) {
            Prov {
                kind: "repo",
                label: rid.to_string(),
                repos: vec![rid.to_string()],
            }
        } else if let Some(group) = e.kv("group").filter(|g| !g.is_empty()) {
            let members = adhoc_members(group)
                .unwrap_or_else(|| self.groups.get(group).cloned().unwrap_or_default());
            Prov {
                kind: "group",
                label: group.to_string(),
                repos: members,
            }
        } else {
            let rf = e.kv("ref").unwrap_or("").to_string();
            let mut cid = if e.etype == EventType::Collision {
                e.kv("id").map(str::to_string)
            } else {
                None
            };
            if let Some(c) = rf.strip_prefix("collision:") {
                cid = Some(c.to_string());
            }
            let mut parent = cid
                .filter(|c| !c.is_empty())
                .and_then(|c| self.openings.get(&c).copied());
            if parent == Some(i) {
                Prov::global()
            } else {
                let candidates: Vec<usize> = self
                    .refs
                    .get(&rf)
                    .map(|v| {
                        v.iter()
                            .copied()
                            .filter(|&x| x != i && self.events[x].date == e.date)
                            .collect()
                    })
                    .unwrap_or_default();
                if parent.is_none() && candidates.len() == 1 {
                    parent = Some(candidates[0]);
                }
                match parent {
                    Some(p) => self.resolve(p, &seen),
                    None => Prov::unclassified(),
                }
            }
        };
        self.cache[i] = Some(result.clone());
        result
    }
}

/// 對應 `state.event_scopes`：每個事件的範圍歸屬（與 `events` 同序）。只讀 token，不從內文推。
///
/// 碰撞回程繼承 opened 的範圍；決定（chosen）繼承它 ref 到的事件；認不出來的放「未分類」。
pub fn event_scopes(events: &[Event], reg: &RegistryData) -> Vec<Prov> {
    let groups: HashMap<String, Vec<String>> =
        reg.groups.iter().map(|g| (g.name(), g.members())).collect();
    let mut openings = HashMap::new();
    let mut refs: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, e) in events.iter().enumerate() {
        if e.etype == EventType::Collision && e.body.starts_with("opened") {
            if let Some(id) = e.kv("id") {
                openings.insert(id.to_string(), i);
            }
        }
        refs.entry(format!("{}:{}", e.etype, e.time))
            .or_default()
            .push(i);
    }
    let mut r = ScopeResolver {
        events,
        groups,
        openings,
        refs,
        cache: vec![None; events.len()],
    };
    (0..events.len())
        .map(|i| r.resolve(i, &HashSet::new()))
        .collect()
}

// ── 收件匣卡片 ─────────────────────────────────────────────────────────

struct CardBuilder<'a> {
    handled: HashSet<String>,
    cidx: BTreeMap<String, CollisionRec>,
    seen_cids: HashSet<String>,
    by_record: HashMap<(String, String, String), &'a Prov>,
}

impl CardBuilder<'_> {
    fn provenance(&self, ev: &Event, m: &mut Map<String, Value>) {
        match self
            .by_record
            .get(&(ev.date.clone(), ev.header(), ev.body.clone()))
        {
            Some(p) => p.put(m),
            None => Prov::unclassified().put(m),
        }
    }

    fn event_card(&mut self, ev: &Event, interrupt: bool) -> Option<Value> {
        let cid = if ev.etype == EventType::Collision {
            ev.kv("id").filter(|c| !c.is_empty()).map(str::to_string)
        } else {
            None
        };
        if OWN_RECORD_TYPES.contains(&ev.etype) || OWN_RECORD_SOURCES.contains(&ev.source.as_str())
        {
            return None;
        }
        if cid.is_some() && ev.body.starts_with("opened") {
            return None; // 自己的提問由 pending 卡呈現
        }
        if ev.etype == EventType::OpenLoop {
            return None; // loop 由 loop 卡呈現（不重複成一則未讀）
        }
        let j = cid.as_ref().and_then(|_| parse_judgement(&ev.body));
        let key = match (&cid, &j) {
            (Some(c), Some(_)) => format!("collision:{c}"),
            _ => format!("{}:{}", ev.etype, ev.time),
        };
        if self.handled.contains(&key) {
            return None;
        }
        if let (Some(cid), Some(j)) = (&cid, &j) {
            if self.seen_cids.contains(cid) {
                return None;
            }
            self.seen_cids.insert(cid.clone());
            let info = self.cidx.get(cid).cloned().unwrap_or_default();
            let dest = normalize_dest(j.get("落點建議").and_then(Value::as_str));
            let scope = collision_scope(info.group.as_deref());
            let task = format!(
                "深撞碰撞 {cid}：想法「{}」；判定 {}，理由：{}。請對照組內文件驗證判定並提出落地步驟",
                info.idea,
                jstr(j, "判定"),
                jstr(j, "理由")
            );
            let mut actions = Vec::new();
            if let Some(d) = &dest {
                actions.push(act(
                    &format!("照建議落 {d}"),
                    "route_collision",
                    json!({"cid": cid, "dest": d}),
                ));
            }
            actions.push(act("改落點…", "route_pick", json!({"cid": cid})));
            if dest.as_deref() != Some("incubator") {
                actions.push(act(
                    "升格 incubator",
                    "route_collision",
                    json!({"cid": cid, "dest": "incubator"}),
                ));
            }
            let mut tp = Map::new();
            tp.insert("kind".into(), json!("agent"));
            tp.insert("task".into(), json!(task));
            tp.insert("origin".into(), json!(format!("collision:{cid}")));
            tp.extend(scope);
            actions.push(act("與 Agent 深入討論", "term_create", Value::Object(tp)));
            actions.push(act(
                "丟棄並記錄",
                "ignore",
                json!({"type": ev.etype.as_str(), "time": ev.time, "header": ev.header(), "cid": cid}),
            ));
            let mut m = Map::new();
            m.insert("key".into(), json!(key));
            m.insert("kind".into(), json!("collision"));
            m.insert("cid".into(), json!(cid));
            self.provenance(ev, &mut m);
            m.insert("time".into(), json!(ev.time));
            m.insert("date".into(), json!(ev.date));
            let title = if info.idea.is_empty() {
                format!("碰撞 {cid}")
            } else {
                info.idea.clone()
            };
            m.insert("title".into(), json!(title));
            m.insert("verdict".into(), json!(jstr(j, "判定")));
            m.insert("judgement".into(), Value::Object(j.clone()));
            let lines: Vec<String> = j
                .iter()
                .filter(|(k, _)| k.as_str() != "判定")
                .map(|(k, v)| format!("{k}：{}", v.as_str().unwrap_or("")))
                .collect();
            m.insert("lines".into(), json!(lines));
            m.insert("actions".into(), json!(actions));
            return Some(Value::Object(m));
        }
        let first = splitlines(&ev.body);
        let title = match first.first() {
            Some(t) => t.to_string(),
            None => ev.tokens.join(" "),
        };
        let lines: Vec<&str> = first.iter().skip(1).take(5).copied().collect();
        let mut actions = vec![act(
            "忽略並記錄",
            "ignore",
            json!({"type": ev.etype.as_str(), "time": ev.time, "header": ev.header()}),
        )];
        if interrupt {
            if let Some(c) = &cid {
                actions.insert(0, act("重跑判定", "collide_rerun", json!({"cid": c})));
            }
        }
        let mut m = Map::new();
        m.insert("key".into(), json!(key));
        m.insert(
            "kind".into(),
            json!(if interrupt { "interrupt" } else { "event" }),
        );
        self.provenance(ev, &mut m);
        m.insert("time".into(), json!(ev.time));
        m.insert("date".into(), json!(ev.date));
        m.insert("etype".into(), json!(ev.etype.as_str()));
        m.insert("source".into(), json!(ev.source));
        m.insert("title".into(), json!(title));
        m.insert("lines".into(), json!(lines));
        m.insert("actions".into(), json!(actions));
        Some(Value::Object(m))
    }
}

/// 對應 `state.build_cards`：收件匣＝一種卡。順序：interrupt → 處理中 → 碰撞回程 → 其他未讀 → open loops。
/// `scopes` 與 `events` 同序（[`event_scopes`]）。
pub fn build_cards(
    events: &[Event],
    pending: &notify::Pending,
    loops: &[Event],
    scopes: &[Prov],
) -> Vec<Value> {
    let mut by_record = HashMap::new();
    for (e, p) in events.iter().zip(scopes) {
        by_record.insert((e.date.clone(), e.header(), e.body.clone()), p);
    }
    let mut b = CardBuilder {
        handled: handled_keys(events),
        cidx: collision_index(events),
        seen_cids: HashSet::new(),
        by_record,
    };
    let mut cards: Vec<Value> = pending
        .interrupt
        .iter()
        .filter_map(|ev| b.event_card(ev, true))
        .collect();
    // 處理中：opened 未回程（不看 last_seen——沒回來就一直是處理中）
    let pend: Vec<(String, CollisionRec)> = b
        .cidx
        .iter()
        .filter(|(cid, rec)| {
            !rec.answered
                && rec.opened.is_some()
                && !b.handled.contains(&format!("collision:{cid}"))
        })
        .map(|(c, r)| (c.clone(), r.clone()))
        .collect();
    for (cid, rec) in pend {
        let Some(ev) = rec.opened.as_ref() else {
            continue;
        };
        let mut m = Map::new();
        m.insert("key".into(), json!(format!("pending:{cid}")));
        m.insert("kind".into(), json!("pending"));
        m.insert("cid".into(), json!(cid));
        b.provenance(ev, &mut m);
        m.insert("time".into(), json!(ev.time));
        m.insert("date".into(), json!(ev.date));
        m.insert("title".into(), json!(rec.idea));
        m.insert(
            "lines".into(),
            json!([format!(
                "送出於 {} {}｜範圍 {}",
                ev.date,
                ev.time,
                rec.group
                    .as_deref()
                    .filter(|g| !g.is_empty())
                    .unwrap_or("全部")
            )]),
        );
        m.insert("actions".into(), json!([]));
        cards.push(Value::Object(m));
    }
    let mut normal: Vec<Value> = pending
        .normal
        .iter()
        .filter_map(|ev| b.event_card(ev, false))
        .collect();
    normal.sort_by_key(|c| if c["kind"] == "collision" { 0 } else { 1 });
    cards.extend(normal);
    for ev in loops {
        let num = ev.num().unwrap_or("#?").to_string();
        let first = splitlines(&ev.body);
        let mut title: String = first
            .first()
            .map(|s| s.to_string())
            .unwrap_or_else(|| num.clone());
        for pre in ["opened →", "opened"] {
            if let Some(rest) = title.strip_prefix(pre) {
                title = rest.trim().to_string();
            }
        }
        let title = title.trim_matches(|c| c == '「' || c == '」').to_string();
        let mut m = Map::new();
        m.insert("key".into(), json!(format!("loop:{num}")));
        m.insert("kind".into(), json!("loop"));
        m.insert("num".into(), json!(num));
        b.provenance(ev, &mut m);
        m.insert("due".into(), json!(ev.kv("due")));
        m.insert("time".into(), json!(ev.time));
        m.insert("date".into(), json!(ev.date));
        m.insert("title".into(), json!(title));
        m.insert("lines".into(), json!([]));
        m.insert(
            "actions".into(),
            json!([
                act("開碰撞", "collide_prefill", json!({"text": title})),
                act("延 7 天", "loop_defer", json!({"num": num, "days": 7})),
                act("關閉", "close_loop", json!({"num": num})),
            ]),
        );
        cards.push(Value::Object(m));
    }
    cards
}

// ── 會話 ───────────────────────────────────────────────────────────────

fn tget<'a>(term: &'a Value, k: &str) -> Option<&'a str> {
    term.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// 對應 `state._session_scope`：會話範圍 → (group, ids)。認不出來就回 (None, None)＝不限範圍。
fn session_scope(term: &Value, data: &RegistryData) -> (Option<String>, Option<Vec<String>>) {
    if let Some(scope) = term.get("scope").and_then(Value::as_str) {
        if let Some(g) = data.group(scope) {
            return (Some(scope.to_string()), Some(g.members()));
        }
        if data.repo(scope).is_some() {
            return (None, Some(vec![scope.to_string()]));
        }
    }
    let members: Vec<String> = term
        .get("scope_repos")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    (None, (!members.is_empty()).then_some(members))
}

fn in_scope(ev: &Event, group: Option<&str>, ids: Option<&[String]>) -> bool {
    spine::in_scope(ev, group, ids.unwrap_or(&[]))
}

/// 對應 `state.session_report`：會話開始後、範圍內的最後一筆脊椎留痕；沒有就 None。
///
/// 回傳帶 source／type／時間——使用者要看得出這筆是 agent 寫的還是工作台寫的，
/// 才判斷得了「現在輪到誰」。不做任何由程序狀態推進度的推論。
pub fn session_report(term: &Value, events: &[Event], data: &RegistryData) -> Option<Value> {
    let started = tget(term, "started")?;
    let (group, ids) = session_scope(term, data);
    let mut best: Option<&Event> = None;
    for ev in events {
        if REPORT_SKIP_TYPES.contains(&ev.etype) || ev.stamp().as_str() < started {
            continue;
        }
        if !in_scope(ev, group.as_deref(), ids.as_deref()) {
            continue;
        }
        if best.is_none_or(|b| (ev.date.as_str(), ev.time.as_str()) >= (&b.date, &b.time)) {
            best = Some(ev);
        }
    }
    let best = best?;
    Some(
        json!({"text": first_or_tokens(best), "type": best.etype.as_str(),
                "source": best.source, "at": best.stamp()}),
    )
}

/// 對應 `state.annotate_terms`：替會話清冊補上回報狀態（清冊本身由 TermManager 給，這裡只加脊椎那一半）。
/// `terms` 是 `TermManager::list()` 序列化後的物件（或測試給的部分欄位）；未知欄位原樣保留。
pub fn annotate_terms(spine_dir: impl AsRef<Path>, terms: &[Value]) -> Result<Vec<Value>> {
    let spine_dir = spine_dir.as_ref();
    let events = spine::iter_events(spine_dir)?;
    let data = registry::load(spine_dir)?;
    Ok(annotate_terms_with(spine_dir, terms, &events, &data))
}

fn annotate_terms_with(
    spine_dir: &Path,
    terms: &[Value],
    events: &[Event],
    data: &RegistryData,
) -> Vec<Value> {
    terms
        .iter()
        .map(|t| {
            let mut rec = t.as_object().cloned().unwrap_or_default();
            if t.get("kind").and_then(Value::as_str) != Some("agent") {
                rec.insert("report".into(), Value::Null);
                rec.insert("report_basis".into(), json!("shell 會話不回報"));
            } else {
                let (group, ids) = session_scope(t, data);
                rec.insert(
                    "report".into(),
                    session_report(t, events, data).unwrap_or(Value::Null),
                );
                // agent 自己回報的階段（report_status）——只認會話開始後的，舊的不算
                let started = tget(t, "started").unwrap_or("~");
                let st = status::get_status(spine_dir, group.as_deref(), ids.as_deref())
                    .filter(|s| s.as_object().is_some_and(|o| !o.is_empty()))
                    .filter(|s| s.get("at").and_then(Value::as_str).unwrap_or("") >= started);
                rec.insert("agent_status".into(), st.unwrap_or(Value::Null));
                // 依據要說得出口：範圍認得出來就講範圍，認不出來就說沒再過濾
                let basis = match (&group, &ids) {
                    (Some(g), _) => format!("會話開始後、範圍內（{g}）的脊椎留痕"),
                    (None, Some(ids)) => {
                        format!("會話開始後、範圍內（{}）的脊椎留痕", ids.join("、"))
                    }
                    (None, None) => {
                        "會話開始後的脊椎留痕（此會話範圍是全部 repo，未再依組過濾）".into()
                    }
                };
                rec.insert("report_basis".into(), json!(basis));
            }
            Value::Object(rec)
        })
        .collect()
}

/// [`TermInfo`] → 給 [`annotate_terms`]／[`session_draft`] 的物件（欄位同 Python `TermManager.list()`）。
pub fn term_value(info: &TermInfo) -> Value {
    serde_json::to_value(info).unwrap_or(Value::Null)
}

/// 對應 `state.session_draft`：交接草稿＝會話期間的脊椎留痕彙整。是草稿不是成果，尚未寫入任何事件。
///
/// 2026-09-12 檢討 §5：「記錄結果」原本是空白表單，使用者得自己翻終端整理。
/// 這裡只把已經存在的留痕排好給人確認——沒有留痕就直說沒有，不編造成果。
pub fn session_draft(spine_dir: impl AsRef<Path>, term: &Value) -> Result<Value> {
    let spine_dir = spine_dir.as_ref();
    let events = spine::iter_events(spine_dir)?;
    let data = registry::load(spine_dir)?;
    let started = tget(term, "started");
    let (group, ids) = session_scope(term, &data);
    let traces: Vec<&Event> = events
        .iter()
        .filter(|ev| {
            !REPORT_SKIP_TYPES.contains(&ev.etype)
                && started.is_none_or(|s| ev.stamp().as_str() >= s)
                && in_scope(ev, group.as_deref(), ids.as_deref())
        })
        .collect();
    let loops: Vec<Event> = spine::open_loops_of(&events)
        .into_iter()
        .filter(|ev| in_scope(ev, group.as_deref(), ids.as_deref()))
        .collect();
    let title = tget(term, "title")
        .or_else(|| tget(term, "sid"))
        .unwrap_or("None");
    let mut lines = vec![
        format!(
            "會話：{title}（範圍 {}{}）",
            tget(term, "scope").unwrap_or("本機 Shell"),
            started.map(|s| format!("，開始於 {s}")).unwrap_or_default()
        ),
        String::new(),
        "完成了什麼（以下是會話期間的脊椎留痕，請確認後再存）：".into(),
    ];
    if traces.is_empty() {
        lines.push("- （會話期間沒有留痕，請自己補；下次可請 agent 收尾時呼叫 handoff）".into());
    } else {
        for ev in &traces[traces.len().saturating_sub(8)..] {
            lines.push(format!(
                "- {} {} {} [{}] {}",
                ev.date,
                ev.time,
                ev.etype,
                ev.source,
                first_or_tokens(ev)
            ));
        }
    }
    lines.push(String::new());
    lines.push("還沒結束的事：".into());
    if loops.is_empty() {
        lines.push("- （本範圍目前沒有未結事項）".into());
    } else {
        for ev in loops.iter().take(6) {
            let num = ev.num().unwrap_or("#?");
            let first = if ev.body.is_empty() {
                ""
            } else {
                splitlines(&ev.body).first().copied().unwrap_or("")
            };
            let due = ev
                .kv("due")
                .filter(|d| !d.is_empty())
                .map(|d| format!("（due {d}）"))
                .unwrap_or_default();
            lines.push(format!("- {num}「{first}」{due}"));
        }
    }
    lines.extend(["".into(), "下次從哪裡接回：".into(), "- （請補上）".into()]);
    Ok(json!({
        "text": lines.join("\n"),
        "traces": traces.len(),
        "loops": loops.len(),
        "note": "草稿由脊椎留痕組成，尚未儲存；存檔會落一筆 decision，不代表成果已被採納。",
    }))
}

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

/// 卡片／事件屬於畫面的哪一欄：`scope`＝目前範圍內、`global`＝全域與未分類（或範圍外的打斷）、`other`＝不顯示。
fn lane(p: &Prov, scoped: bool, members: &[String], interrupt: bool) -> &'static str {
    let matches = !scoped || p.repos.iter().any(|r| members.contains(r));
    if p.unscoped() || (interrupt && !matches) {
        "global"
    } else if matches {
        "scope"
    } else {
        "other"
    }
}

fn prov_of(v: &Value) -> Prov {
    Prov {
        kind: match v["scope_kind"].as_str() {
            Some("repo") => "repo",
            Some("group") => "group",
            Some("global") => "global",
            _ => "unclassified",
        },
        label: v["scope_label"].as_str().unwrap_or("").to_string(),
        repos: v["scope_repos"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
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

/// Rust 才有（F-42）：未結卡多一個「用任務包開會話」（前端先叫 `todo_brief` 拿 markdown，再開會話）。
fn add_brief_action(cards: &mut [Value]) {
    for c in cards.iter_mut().filter(|c| c["kind"] == "loop") {
        let num = c["num"].clone();
        if let Some(a) = c["actions"].as_array_mut() {
            a.push(act("用任務包開會話", "brief_session", json!({"num": num})));
        }
    }
}

/// Rust 才有（F-46）：重看條件到期的有效判斷成卡（同 agent 開場的「該重看」；純事件讀）。
/// 出口：開會話重看（帶好任務）／延 14 天（人看過、先維持）。
fn reopen_cards(spine_dir: &Path, events: &[Event], scopes: &[Prov]) -> Vec<Value> {
    let superseded = crate::agentapi::superseded_map(events);
    let snoozes = crate::agentapi::reopen_snoozes(spine_dir);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut out = Vec::new();
    for (ev, prov) in events.iter().zip(scopes) {
        if ev.etype != EventType::Decision {
            continue;
        }
        let r = crate::agentapi::event_ref(ev);
        if superseded.contains_key(&r) {
            continue;
        }
        let Some(cond) = crate::agentapi::reopen_when(ev) else {
            continue;
        };
        if !crate::agentapi::reopen_is_due(&cond, &r, &today, &snoozes) {
            continue;
        }
        let text = first_or_tokens(ev);
        let mut session = collision_scope(ev.kv("group"));
        if session.is_empty() {
            if let Some(repo) = ev.kv("repo").filter(|x| !x.is_empty()) {
                session.insert("repos".into(), json!([repo]));
            }
        }
        session.insert("kind".into(), json!("agent"));
        session.insert(
            "task".into(),
            json!(format!(
                "重看判斷 {r}：{text}\n當初說的重看條件：{cond}\n先用 search_knowledge 看這段期間相關的判斷與變化，\
                 跟我討論後：要改就 log_decision 並帶 supersedes {r}；維持原判斷就說明理由。"
            )),
        );
        session.insert("origin".into(), json!(format!("reopen:{r}")));
        let mut m = Map::new();
        m.insert("key".into(), json!(format!("reopen:{r}")));
        m.insert("kind".into(), json!("reopen"));
        m.insert("ref".into(), json!(r));
        m.insert("title".into(), json!(format!("該重看：{text}")));
        m.insert("date".into(), json!(ev.date));
        m.insert("time".into(), json!(ev.time));
        m.insert("lines".into(), json!([format!("當初說的重看條件：{cond}")]));
        prov.put(&mut m);
        m.insert(
            "actions".into(),
            json!([
                act("開會話重看", "term_create", Value::Object(session)),
                act("延 14 天", "reopen_defer", json!({"ref": r, "days": 14})),
            ]),
        );
        out.push(Value::Object(m));
    }
    out
}

/// Rust 才有（F-23）：判斷回顧——滿 14 天、還沒記結果的判斷，最舊的幾筆成卡。
/// 出口：開會話回顧（任務帶好 outcome_of 做法）／延 30 天。
const RETRO_CARDS: usize = 5;

fn retro_cards(spine_dir: &Path, events: &[Event], scopes: &[Prov]) -> Vec<Value> {
    let prov: HashMap<String, &Prov> = events
        .iter()
        .zip(scopes)
        .filter(|(e, _)| e.etype == EventType::Decision)
        .map(|(e, p)| (crate::agentapi::event_ref(e), p))
        .collect();
    let now = chrono::Local::now().naive_local();
    crate::agentapi::retro_candidates(spine_dir, events, None, now)
        .into_iter()
        .take(RETRO_CARDS)
        .map(|c| {
            let r = c["ref"].as_str().unwrap_or("").to_string();
            let text = c["text"].as_str().unwrap_or("").to_string();
            let (date, time) = c["at"]
                .as_str()
                .unwrap_or("")
                .split_once(' ')
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .unwrap_or_default();
            let mut session = collision_scope(c["group"].as_str());
            if session.is_empty() {
                if let Some(repo) = c["repo"].as_str() {
                    session.insert("repos".into(), json!([repo]));
                }
            }
            session.insert("kind".into(), json!("agent"));
            session.insert(
                "task".into(),
                json!(format!(
                    "回顧判斷 {r}（{} 天前）：{text}\n先看這段期間的 commit、判斷與交接，跟我確認它後來怎樣：\
                     做得好不好、有沒有後悔、學到什麼。確認後用 log_decision 帶 outcome_of={r} 記一筆結果。",
                    c["days"]
                )),
            );
            session.insert("origin".into(), json!(format!("retro:{r}")));
            let mut m = Map::new();
            m.insert("key".into(), json!(format!("retro:{r}")));
            m.insert("kind".into(), json!("retro"));
            m.insert("ref".into(), json!(r));
            m.insert("title".into(), json!(format!("判斷回顧：{text}")));
            m.insert("date".into(), json!(date));
            m.insert("time".into(), json!(time));
            m.insert(
                "lines".into(),
                json!([format!("{} 天前的判斷，還沒記結果——後來怎樣？", c["days"])]),
            );
            match prov.get(&r) {
                Some(p) => p.put(&mut m),
                None => Prov::unclassified().put(&mut m),
            }
            m.insert(
                "actions".into(),
                json!([
                    act("開會話回顧", "term_create", Value::Object(session)),
                    act("延 30 天", "reopen_defer", json!({"ref": r, "days": 30})),
                ]),
            );
            Value::Object(m)
        })
        .collect()
}

/// Rust 才有（F-48）：反芻推薦成卡（排在最後；只放目前規則、近幾天的＝跟 agent 開場看到的同一批）。
/// 純檔案讀（`.state/ruminate.json`）。出口：拿來試（開帶出處的待辦）／不相關。
fn fresh_cards(spine_dir: &Path, data: &RegistryData) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for g in &data.groups {
        let name = g.name();
        for l in crate::ruminate::push_links(spine_dir, &name, None)? {
            let id = l.id.clone().unwrap_or_default();
            let (date, time) = l.found.split_once(' ').unwrap_or((l.found.as_str(), ""));
            let mut lines = vec![json!(format!("共同詞：{}", l.terms.join("、")))];
            if let Some(sn) = l.snippets.as_ref().and_then(|v| v.first()) {
                lines.push(json!(format!(
                    "L{}：{}",
                    sn.line,
                    crate::pystr::take_chars(&sn.text, 100)
                )));
            }
            let mut m = Map::new();
            m.insert("key".into(), json!(format!("fresh:{id}")));
            m.insert("kind".into(), json!("fresh"));
            m.insert("id".into(), json!(id));
            m.insert("title".into(), json!(format!("新料：{}", l.reference)));
            m.insert("date".into(), json!(date));
            m.insert("time".into(), json!(time));
            m.insert("lines".into(), Value::Array(lines));
            Prov {
                kind: "group",
                label: name.clone(),
                repos: g.members(),
            }
            .put(&mut m);
            m.insert(
                "actions".into(),
                json!([
                    act("拿來試", "fresh_try", json!({"id": id})),
                    act("不相關", "fresh_dismiss", json!({"id": id})),
                ]),
            );
            out.push(Value::Object(m));
        }
    }
    Ok(out)
}

/// Rust 才有（F-46）：未結卡帶上 agent 看到的同一份體檢結果（`.state/review.json`，由開場／`ctx todo review` 算好）。
/// 只有「可能已處理」「擱置」才加 `verdict`（卡片上的標籤）；有證據的列進 `lines`（描述＋詳情）。
fn attach_review(cards: &mut [Value], review: &Map<String, Value>) {
    for c in cards.iter_mut() {
        if c["kind"] != "loop" {
            continue;
        }
        let Some(r) = c["num"].as_str().and_then(|n| review.get(n)) else {
            continue;
        };
        let label = match r["verdict"].as_str() {
            Some("maybe_done") => Some("可能已處理"),
            Some("stale") => Some("擱置沒動靜"),
            _ => None,
        };
        // 進行中的不加註（人看收件匣時那只是雜訊；agent 要細節用 ctx todo review）
        if label.is_none() {
            continue;
        }
        let mut lines: Vec<Value> = Vec::new();
        {
            lines.push(json!(format!(
                "體檢（{}）：{}",
                r["at"].as_str().unwrap_or(""),
                r["why"].as_str().unwrap_or("")
            )));
        }
        for e in r["evidence"].as_array().into_iter().flatten().take(3) {
            lines.push(json!(format!(
                "證據 {} {}（蓋住 {}%）：{}",
                e["at"].as_str().unwrap_or(""),
                e["ref"].as_str().unwrap_or(""),
                (e["cover"].as_f64().unwrap_or(0.0) * 100.0).round(),
                crate::pystr::take_chars(e["text"].as_str().unwrap_or(""), 80)
            )));
        }
        if let Some(o) = c.as_object_mut() {
            if let Some(l) = label {
                o.insert("verdict".into(), json!(l));
            }
            if !lines.is_empty() {
                o.insert("lines".into(), Value::Array(lines));
            }
        }
    }
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
    cards.extend(reopen_cards(spine_dir, &events, &scopes));
    cards.extend(retro_cards(spine_dir, &events, &scopes));
    cards.extend(fresh_cards(spine_dir, &data)?);
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
        "scope": {"group": scope.group, "repos": scope.repos, "members": members},
    });
    if !suggestions.is_empty() {
        out["relation_suggestions"] = Value::Array(suggestions);
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
    let question_cards: Vec<Value> = cards
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
