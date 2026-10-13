//! 收件匣卡片：事件 → 卡片，加上延期遞增、重看、回顧、新料、體檢標籤與主要動作。

use super::*;

// ── 收件匣卡片 ─────────────────────────────────────────────────────────

pub(super) struct CardBuilder<'a> {
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

/// 卡片的主要動作（畫面上那顆主按鈕）標 `primary: true`：有建議落點就是 `route_collision`，
/// 否則第一個不是 ignore／defer／route_pick 的。Rust 才有（2026-10-13，原本前端判斷）。
pub(super) fn mark_primary(cards: &mut [Value]) {
    for c in cards {
        let Some(acts) = c.get_mut("actions").and_then(Value::as_array_mut) else {
            continue;
        };
        let pick = acts
            .iter()
            .position(|a| a["action"] == "route_collision")
            .or_else(|| {
                acts.iter().position(|a| {
                    !matches!(
                        a["action"].as_str(),
                        Some("ignore" | "defer" | "route_pick")
                    )
                })
            });
        if let Some(o) = pick.and_then(|i| acts[i].as_object_mut()) {
            o.insert("primary".into(), Value::Bool(true));
        }
    }
}

/// Rust 才有（F-45）：延期間隔依已延次數遞增（7 → 14 → 30 天；第一次同 Python 的 7 天）。
/// 次數＝spine 裡這一項「延期 N 天」的事件數，不另存狀態。延 3 次以上卡上提醒「還要做嗎」。
/// 參考 Matuschak：「之後再說」取代刪除，間隔遞增再出現，清收件匣就沒壓力。
pub const DEFER_STEPS: [i64; 3] = [7, 14, 30];

pub(super) fn escalate_defer(cards: &mut [Value], events: &[Event]) {
    let mut times: HashMap<&str, usize> = HashMap::new();
    for ev in events.iter().filter(|e| e.etype == EventType::OpenLoop) {
        if ev.body.contains("\n延期 ") {
            if let Some(n) = ev.num() {
                *times.entry(n).or_default() += 1;
            }
        }
    }
    for c in cards.iter_mut().filter(|c| c["kind"] == "loop") {
        let n = c["num"]
            .as_str()
            .and_then(|x| times.get(x))
            .copied()
            .unwrap_or(0);
        if n == 0 {
            continue;
        }
        let days = DEFER_STEPS[n.min(DEFER_STEPS.len() - 1)];
        if let Some(acts) = c["actions"].as_array_mut() {
            for a in acts.iter_mut().filter(|a| a["action"] == "loop_defer") {
                a["label"] = json!(format!("之後再說（{days} 天）"));
                a["payload"]["days"] = json!(days);
            }
        }
        if n >= DEFER_STEPS.len() {
            if let Some(lines) = c["lines"].as_array_mut() {
                lines.push(json!(format!(
                    "已延 {n} 次——還要做嗎？不做就關掉，要做就排進下一步"
                )));
            }
        }
    }
}

/// Rust 才有（F-42）：未結卡多一個「用任務包開會話」（前端先叫 `todo_brief` 拿 markdown，再開會話）。
pub(super) fn add_brief_action(cards: &mut [Value]) {
    for c in cards.iter_mut().filter(|c| c["kind"] == "loop") {
        let num = c["num"].clone();
        if let Some(a) = c["actions"].as_array_mut() {
            a.push(act("用任務包開會話", "brief_session", json!({"num": num})));
        }
    }
}

/// Rust 才有（F-46）：重看條件到期的有效判斷成卡（同 agent 開場的「該重看」；純事件讀）。
/// 出口：開會話重看（帶好任務）／延 14 天（人看過、先維持）。
pub(super) fn reopen_cards(spine_dir: &Path, events: &[Event], scopes: &[Prov]) -> Vec<Value> {
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
pub(super) const RETRO_CARDS: usize = 5;

pub(super) fn retro_cards(spine_dir: &Path, events: &[Event], scopes: &[Prov]) -> Vec<Value> {
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
pub(super) fn fresh_cards(spine_dir: &Path, data: &RegistryData) -> Result<Vec<Value>> {
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
pub(super) fn attach_review(cards: &mut [Value], review: &Map<String, Value>) {
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
