//! 範圍歸屬：事件／卡片屬於哪個 repo 或組（`Prov`），以及在畫面上落哪一欄（`lane`）。

use super::*;

// ── 範圍歸屬 ───────────────────────────────────────────────────────────

/// 範圍歸屬（provenance）：`scope_kind`（repo｜group｜global｜unclassified）、`scope_label`、`scope_repos`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prov {
    pub kind: &'static str,
    pub label: String,
    pub repos: Vec<String>,
}

impl Prov {
    pub(super) fn unclassified() -> Self {
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
    pub(super) fn put(&self, m: &mut Map<String, Value>) {
        m.insert("scope_kind".into(), json!(self.kind));
        m.insert("scope_label".into(), json!(self.label));
        m.insert("scope_repos".into(), json!(self.repos));
    }
}

pub(super) struct ScopeResolver<'a> {
    events: &'a [Event],
    /// 組名（原名與換寫過的寫法都當 key）→ (原名, 成員)
    groups: HashMap<String, (String, Vec<String>)>,
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
            // 標頭裡的組名可能換寫過（PM_Group）：標籤用登記的原名
            let known = self.groups.get(group);
            let members = adhoc_members(group)
                .unwrap_or_else(|| known.map(|k| k.1.clone()).unwrap_or_default());
            Prov {
                kind: "group",
                label: known.map_or_else(|| group.to_string(), |k| k.0.clone()),
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
    // 事件標頭裡的組名可能是換寫過的（「PM_Group」，見 spine::token_value）：兩種寫法都認
    let groups: HashMap<String, (String, Vec<String>)> = reg
        .groups
        .iter()
        .flat_map(|g| {
            let (name, members) = (g.name(), g.members());
            [
                (
                    crate::spine::token_value(&name),
                    (name.clone(), members.clone()),
                ),
                (name.clone(), (name, members)),
            ]
        })
        .collect();
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

/// 卡片／事件屬於畫面的哪一欄：`scope`＝目前範圍內、`global`＝全域與未分類（或範圍外的打斷）、`other`＝不顯示。
pub(super) fn lane(p: &Prov, scoped: bool, members: &[String], interrupt: bool) -> &'static str {
    let matches = !scoped || p.repos.iter().any(|r| members.contains(r));
    if p.unscoped() || (interrupt && !matches) {
        "global"
    } else if matches {
        "scope"
    } else {
        "other"
    }
}

pub(super) fn prov_of(v: &Value) -> Prov {
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
