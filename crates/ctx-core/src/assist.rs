//! F-63 App 助手（2026-10-15；Rust 才有）：一句話 → 白名單內的步驟（計畫）→ 人確認 → 走原本的 action 執行。
//! 設計見 `docs/APP_ASSISTANT.md`。
//!
//! - [`plan`]：先試幾條本機規則（「切到 X」「X 加標籤 Y」…，不花錢、離線也能用），對不上才問 provider。
//! - [`validate`]：不管計畫是規則、agent 還是 MCP 給的，都過同一套檢查——op 在白名單、引用的 repo／組存在、
//!   參數型別對；壞的步驟標出原因、不丟掉整份計畫，也不猜（找不到就列最接近的名字）。
//! - 引擎**不執行**計畫：App 依序呼叫原本的 action（畫面類步驟如換範圍由前端自己做），每個 action 照舊留痕；
//!   執行完前端呼叫 `assist_log` 落一筆 presented 事件（指令、做了哪些、略過哪些）。

use std::path::Path;

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::agents::{self, AgentProvider};
use crate::registry::{self, RegistryData, Tier};
use crate::{config, Error, Result};

/// 一份計畫最多幾步（一句話做太多事要人拆）。
pub const MAX_STEPS: usize = 10;

/// App 的工作區分頁（`open_tab` 可用的值；跟前端 `Tab` 一致）。
const TABS: &[(&str, &str)] = &[
    ("inbox", "待處理"),
    ("notes", "筆記"),
    ("repos", "Repo"),
    ("files", "檔案"),
    ("activity", "活動"),
    ("relations", "關係"),
];

/// 白名單（op、參數、說明）——給 provider 的提示用，也是 [`validate`] 認得的全部。
const OPS: &str = r#"- switch_scope {group} 或 {repos: [id…]}：換 App 目前的工作範圍（牌組或臨時組合）
- open_tab {tab}：切工作區分頁，tab ∈ inbox|notes|repos|files|activity|relations
- open_repo {id}：在右側欄打開某個 repo
- create_group {name, repos: [id…]}：建新牌組
- set_members {group, repos: [id…]}：把牌組成員設成這一批（完整清單，不是增量）
- rename_group {group, new_name}：牌組改名
- set_goal {group, text}：設牌組目標（一句話）
- tag {id, add?: [標籤…], remove?: [標籤…]}：repo 加／拿掉標籤
- set_tier {id, tier, resume_when?}：tier ∈ active|paused|dormant|archived；paused 要給 resume_when
- open_session {repo? | group?, task}：打開「新建 Agent 會話」對話框（不會直接開）"#;

const PROMPT: &str = r#"你是「工作台助手」：把使用者的一句話翻成工作台的設定步驟。只能用下面的動作，不能發明新的；
做得到的部分照做；看不懂的、要刪除東西的、白名單做不到的，不要硬湊步驟，在「說明」講清楚缺什麼。
repo id 與牌組名稱要照【現況】裡的寫法；【現況】裡沒有的牌組或 repo 就是不存在，不要猜。
「最近沒在動」之類的判斷看【現況】裡每個 repo 的「最近 commit」天數。

【可用動作】
{ops}

【現況】
{snapshot}

【使用者的話】
{text}

只輸出一個 JSON 物件：
{"說明": "一句話講你要做什麼", "steps": [{"op": "…", "args": {…}, "理由": "一句話"}]}
"#;

/// 計畫的一步（已驗證過）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub op: String,
    /// 正規化後的參數（名字換成登記的寫法）；驗證不過時是原樣。
    pub args: Value,
    /// 人話：「VMS_PM：加標籤「規格」」。
    pub text: String,
    pub why: String,
    /// 會移掉東西（成員、封存）——畫面標紅。
    pub danger: bool,
    /// 驗證不過的原因；有值＝這步不能執行。
    pub error: Option<String>,
}

/// 一份計畫。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    pub summary: String,
    pub steps: Vec<Step>,
    /// "rules"＝本機規則；"agent"＝provider；"mcp"＝agent 自己給的。
    pub source: &'static str,
}

/// App 目前的範圍（給規則「切到」與提示用）。
#[derive(Debug, Clone, Default)]
pub struct Scope {
    pub group: Option<String>,
    pub repos: Option<Vec<String>>,
}

/// 一句話 → 計畫。規則對得上就不問 provider；provider 用 config `provider.assist`（沒設＝claude），
/// model 用 `provider.models.assist`（沒設＝haiku）。
pub fn plan(spine_dir: &Path, text: &str, scope: &Scope) -> Result<Plan> {
    let cfg = config::load(spine_dir)?;
    let name = cfg.str("provider.assist").unwrap_or("claude").to_string();
    let provider = agents::provider_by_name(&name).ok_or_else(|| {
        Error::invalid(format!(
            "未知 provider: {name}（可用：{}）",
            agents::PROVIDER_NAMES.join(", ")
        ))
    })?;
    plan_with(spine_dir, text, scope, provider.as_ref())
}

/// [`plan`] 的本體，provider 由呼叫端給（測試注入用）。
pub fn plan_with(
    spine_dir: &Path,
    text: &str,
    scope: &Scope,
    provider: &dyn AgentProvider,
) -> Result<Plan> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::invalid("請說要做什麼"));
    }
    let data = registry::load(spine_dir)?;
    if let Some(raw) = rules(&data, text) {
        return validate(&data, &raw, "rules");
    }
    let cfg = config::load(spine_dir)?;
    let model = cfg.provider_model("assist").unwrap_or("haiku").to_string();
    let prompt = PROMPT
        .replacen("{ops}", OPS, 1)
        .replacen(
            "{snapshot}",
            &snapshot(&data, scope, &activity(&data, spine_dir)),
            1,
        )
        .replacen("{text}", text, 1);
    let raw = provider.run_judgement(&prompt, spine_dir, Some(&model))?;
    validate(&data, &raw, "agent")
}

/// 每個 repo 最近一次 commit 是幾天前（現場平行採集；只在要問 provider 時跑，跟 provider 的幾秒比可以忽略）。
fn activity(data: &RegistryData, spine_dir: &Path) -> std::collections::HashMap<String, f64> {
    let specs: Vec<crate::collect::RepoSpec> = data
        .repos
        .iter()
        .map(crate::collect::RepoSpec::from)
        .collect();
    crate::collect::collect_group(&specs, Some(spine_dir))
        .into_iter()
        .filter_map(|s| s.last_commit_days.map(|d| (s.id, d)))
        .collect()
}

/// 給 provider 看的現況：牌組（成員、目標）、repo（標籤、分層、最近 commit）、目前範圍。
fn snapshot(
    data: &RegistryData,
    scope: &Scope,
    days: &std::collections::HashMap<String, f64>,
) -> String {
    let mut out = Vec::new();
    out.push(format!(
        "目前範圍：{}",
        match (&scope.group, &scope.repos) {
            (Some(g), _) => format!("牌組「{g}」"),
            (None, Some(r)) => format!("臨時組合（{}）", r.join("、")),
            _ => "所有 repo".into(),
        }
    ));
    out.push("牌組：".into());
    for g in &data.groups {
        out.push(format!(
            "- {}：{}{}",
            g.name(),
            g.members().join("、"),
            g.goal()
                .map(|t| format!("（目標：{t}）"))
                .unwrap_or_default()
        ));
    }
    out.push("repo：".into());
    for r in &data.repos {
        let tags = r.tags();
        let id = r.id();
        out.push(format!(
            "- {}{}{}{}",
            id,
            days.get(&id)
                .map(|d| format!("（最近 commit：{} 天前）", d.round()))
                .unwrap_or_default(),
            r.tier_str()
                .filter(|t| *t != "active")
                .map(|t| format!(" [{t}]"))
                .unwrap_or_default(),
            if tags.is_empty() {
                String::new()
            } else {
                format!(" 標籤：{}", tags.join("、"))
            }
        ));
    }
    out.join("\n")
}

/// 本機規則（只收最常見、不會誤會的說法）。對不上回 None。
fn rules(data: &RegistryData, text: &str) -> Option<Value> {
    let t = text.trim().trim_end_matches(['。', '.', '！', '!']);
    // 切到／切換到／換到／打開 <牌組>
    for pre in ["切換到", "切到", "換到", "打開牌組", "開牌組"] {
        if let Some(rest) = t.strip_prefix(pre) {
            let name = rest.trim().trim_matches(['「', '」', '"', '\'']);
            if let Some(g) = find_group(data, name) {
                return Some(json!({"說明": format!("切到牌組「{g}」"),
                    "steps": [{"op": "switch_scope", "args": {"group": g}}]}));
            }
        }
    }
    // <repo> 加標籤 a、b ／ <repo> 拿掉標籤 a
    for (verb, key) in [
        ("加標籤", "add"),
        ("加 tag", "add"),
        ("加tag", "add"),
        ("拿掉標籤", "remove"),
        ("移除標籤", "remove"),
        ("刪標籤", "remove"),
    ] {
        if let Some((who, tags)) = t.split_once(verb) {
            let id = find_repo(data, who.trim().trim_end_matches(['：', ':']).trim())?;
            let tags = split_list(tags);
            if tags.is_empty() {
                return None;
            }
            return Some(
                json!({"說明": format!("{id}：{}標籤", if key == "add" {"加"} else {"拿掉"}),
                "steps": [{"op": "tag", "args": {"id": id, key: tags}}]}),
            );
        }
    }
    None
}

/// 「a、b, c 和 d」→ ["a","b","c","d"]。
fn split_list(s: &str) -> Vec<String> {
    s.split(['、', ',', '，', ' ', '／', '/', '和'])
        .map(|x| x.trim().trim_matches(['「', '」', '"', '\'']))
        .filter(|x| !x.is_empty())
        .map(str::to_string)
        .collect()
}

/// 名字 → 登記的寫法：完全相同（含 PM_Group 換寫）優先，再來不分大小寫。
fn find_repo(data: &RegistryData, name: &str) -> Option<String> {
    data.repo(name).map(|r| r.id()).or_else(|| {
        let low = name.to_lowercase();
        data.repos
            .iter()
            .find(|r| r.id().to_lowercase() == low)
            .map(|r| r.id())
    })
}

fn find_group(data: &RegistryData, name: &str) -> Option<String> {
    data.group(name).map(|g| g.name()).or_else(|| {
        let low = name.to_lowercase();
        data.groups
            .iter()
            .find(|g| g.name().to_lowercase() == low)
            .map(|g| g.name())
    })
}

/// 找不到時列最接近的名字（包含或被包含，最多 3 個）。
fn near(names: impl Iterator<Item = String>, name: &str) -> String {
    let low = name.to_lowercase();
    let hits: Vec<String> = names
        .filter(|n| {
            let n = n.to_lowercase();
            !low.is_empty() && (n.contains(&low) || low.contains(&n))
        })
        .take(3)
        .collect();
    if hits.is_empty() {
        String::new()
    } else {
        format!("（最接近：{}）", hits.join("、"))
    }
}

fn str_list(v: Option<&Value>) -> Option<Vec<String>> {
    match v? {
        Value::Array(a) => a
            .iter()
            .map(|x| x.as_str().map(|s| s.trim().to_string()))
            .collect::<Option<Vec<_>>>()
            .map(|v| v.into_iter().filter(|s| !s.is_empty()).collect()),
        Value::String(s) => Some(split_list(s)),
        _ => None,
    }
}

/// provider／MCP 給的原始計畫 → 驗證過的 [`Plan`]。`raw` 要有 `steps` 陣列；`說明`／`summary` 可缺。
pub fn validate(data: &RegistryData, raw: &Value, source: &'static str) -> Result<Plan> {
    let summary = ["說明", "summary"]
        .iter()
        .find_map(|k| raw.get(*k).and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_string();
    let Some(steps) = raw.get("steps").and_then(Value::as_array) else {
        return Err(Error::invalid("計畫缺 steps"));
    };
    if steps.len() > MAX_STEPS {
        return Err(Error::invalid(format!(
            "一次最多 {MAX_STEPS} 步（給了 {}）；請拆成幾句",
            steps.len()
        )));
    }
    let steps = steps
        .iter()
        .map(|s| {
            let op = s
                .get("op")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let args = s.get("args").cloned().unwrap_or(Value::Object(Map::new()));
            let why = ["理由", "why"]
                .iter()
                .find_map(|k| s.get(*k).and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            match check(data, &op, &args) {
                Ok((args, text, danger)) => Step {
                    op,
                    args,
                    text,
                    why,
                    danger,
                    error: None,
                },
                Err(e) => Step {
                    text: if op.is_empty() {
                        "（沒有動作）".into()
                    } else {
                        op.clone()
                    },
                    op,
                    args,
                    why,
                    danger: false,
                    error: Some(e),
                },
            }
        })
        .collect();
    Ok(Plan {
        summary,
        steps,
        source,
    })
}

/// 一步的檢查：回（正規化的參數、人話、危險）或錯誤原因。
fn check(
    data: &RegistryData,
    op: &str,
    a: &Value,
) -> std::result::Result<(Value, String, bool), String> {
    let s = |k: &str| {
        a.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("")
    };
    let repo = |name: &str| {
        find_repo(data, name).ok_or_else(|| {
            format!(
                "找不到 repo「{name}」{}",
                near(data.repos.iter().map(|r| r.id()), name)
            )
        })
    };
    let group = |name: &str| {
        find_group(data, name).ok_or_else(|| {
            format!(
                "找不到牌組「{name}」{}",
                near(data.groups.iter().map(|g| g.name()), name)
            )
        })
    };
    let repos = |k: &str| -> std::result::Result<Vec<String>, String> {
        let list = str_list(a.get(k)).ok_or_else(|| format!("缺 {k}"))?;
        if list.is_empty() {
            return Err(format!("{k} 是空的"));
        }
        let mut out: Vec<String> = Vec::new();
        for n in list {
            let id = repo(&n)?;
            if !out.contains(&id) {
                out.push(id);
            }
        }
        Ok(out)
    };
    match op {
        "switch_scope" => {
            if !s("group").is_empty() {
                let g = group(s("group"))?;
                Ok((json!({"group": g}), format!("切到牌組「{g}」"), false))
            } else {
                let r = repos("repos")?;
                Ok((
                    json!({"repos": r}),
                    format!("切到臨時組合：{}", r.join("、")),
                    false,
                ))
            }
        }
        "open_tab" => {
            let t = s("tab");
            let (k, label) = TABS
                .iter()
                .find(|(k, l)| *k == t || *l == t)
                .ok_or_else(|| format!("沒有「{t}」分頁"))?;
            Ok((json!({"tab": k}), format!("打開「{label}」分頁"), false))
        }
        "open_repo" => {
            let id = repo(s("id"))?;
            Ok((json!({"id": id}), format!("在右側欄打開 {id}"), false))
        }
        "create_group" => {
            let name = s("name");
            if name.is_empty() {
                return Err("缺牌組名稱".into());
            }
            if find_group(data, name).is_some() {
                return Err(format!("已經有牌組「{name}」"));
            }
            let r = repos("repos")?;
            Ok((
                json!({"name": name, "repos": r}),
                format!("建牌組「{name}」：{}", r.join("、")),
                false,
            ))
        }
        "set_members" => {
            let g = group(s("group"))?;
            let r = repos("repos")?;
            let cur = data.group(&g).map(|x| x.members()).unwrap_or_default();
            let added: Vec<&String> = r.iter().filter(|x| !cur.contains(x)).collect();
            let removed: Vec<&String> = cur.iter().filter(|x| !r.contains(x)).collect();
            if added.is_empty() && removed.is_empty() {
                return Err(format!("「{g}」的成員已經是這一批"));
            }
            let mut parts = Vec::new();
            if !added.is_empty() {
                parts.push(format!(
                    "加入 {}",
                    added
                        .iter()
                        .map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join("、")
                ));
            }
            if !removed.is_empty() {
                parts.push(format!(
                    "移出 {}",
                    removed
                        .iter()
                        .map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join("、")
                ));
            }
            Ok((
                json!({"group": g, "repos": r}),
                format!("牌組「{g}」：{}", parts.join("；")),
                !removed.is_empty(),
            ))
        }
        "rename_group" => {
            let g = group(s("group"))?;
            let new = s("new_name");
            if new.is_empty() {
                return Err("缺新名稱".into());
            }
            if find_group(data, new).is_some() {
                return Err(format!("已經有牌組「{new}」"));
            }
            Ok((
                json!({"group": g, "new_name": new}),
                format!("牌組「{g}」改名為「{new}」"),
                false,
            ))
        }
        "set_goal" => {
            let g = group(s("group"))?;
            let text = s("text");
            if text.is_empty() {
                return Err("缺目標內容".into());
            }
            Ok((
                json!({"group": g, "text": text}),
                format!("牌組「{g}」的目標：{text}"),
                false,
            ))
        }
        "tag" => {
            let id = repo(s("id"))?;
            let add = str_list(a.get("add")).unwrap_or_default();
            let remove = str_list(a.get("remove")).unwrap_or_default();
            if add.is_empty() && remove.is_empty() {
                return Err("沒有要加或拿掉的標籤".into());
            }
            let mut parts = Vec::new();
            if !add.is_empty() {
                parts.push(format!("加標籤「{}」", add.join("」「")));
            }
            if !remove.is_empty() {
                parts.push(format!("拿掉標籤「{}」", remove.join("」「")));
            }
            Ok((
                json!({"id": id, "add": add, "remove": remove}),
                format!("{id}：{}", parts.join("、")),
                false,
            ))
        }
        "set_tier" => {
            let id = repo(s("id"))?;
            let tier: Tier = s("tier").parse().map_err(|_| {
                format!(
                    "分層要是 active／paused／dormant／archived（給了「{}」）",
                    s("tier")
                )
            })?;
            let resume = s("resume_when");
            if tier == Tier::Paused && resume.is_empty() {
                return Err("暫停要寫什麼條件下恢復（resume_when）".into());
            }
            let label = match tier {
                Tier::Active => "恢復進行中",
                Tier::Paused => "暫停",
                Tier::Dormant => "休眠",
                Tier::Archived => "封存",
            };
            let mut args = json!({"id": id, "tier": tier.as_str()});
            if !resume.is_empty() {
                args["resume_when"] = json!(resume);
            }
            Ok((
                args,
                format!(
                    "{id}：{label}{}",
                    if resume.is_empty() {
                        String::new()
                    } else {
                        format!("（恢復條件：{resume}）")
                    }
                ),
                tier == Tier::Archived,
            ))
        }
        "open_session" => {
            let task = s("task");
            let (args, target) = if !s("repo").is_empty() {
                let id = repo(s("repo"))?;
                (json!({"repo": id, "task": task}), id)
            } else if !s("group").is_empty() {
                let g = group(s("group"))?;
                (json!({"group": g, "task": task}), format!("牌組「{g}」"))
            } else {
                (json!({"task": task}), "目前範圍".into())
            };
            Ok((
                args,
                format!(
                    "打開「新建 Agent 會話」（{target}）{}",
                    if task.is_empty() {
                        String::new()
                    } else {
                        format!("：{task}")
                    }
                ),
                false,
            ))
        }
        "" => Err("沒有動作".into()),
        other => Err(format!("助手不能做「{other}」（不在白名單）")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::spine_with_repos;

    /// 測試用 provider：回固定的 JSON，記下 prompt。
    struct Fixed(Value, std::sync::Mutex<String>);
    impl AgentProvider for Fixed {
        fn name(&self) -> &'static str {
            "fixed"
        }
        fn run_judgement(&self, prompt: &str, _: &Path, _: Option<&str>) -> Result<Value> {
            *self.1.lock().unwrap() = prompt.to_string();
            Ok(self.0.clone())
        }
        fn run_text(
            &self,
            _: &str,
            _: &Path,
            _: Option<&str>,
            _: Option<&Path>,
            _: bool,
        ) -> Result<String> {
            unreachable!()
        }
    }
    fn fixed(v: Value) -> Fixed {
        Fixed(v, std::sync::Mutex::new(String::new()))
    }

    #[test]
    fn rules_handle_switch_and_tag_without_provider() {
        let (_t, d) = spine_with_repos();
        let never = fixed(json!({}));
        let p = plan_with(&d, "切到 G1", &Scope::default(), &never).unwrap();
        assert_eq!(p.source, "rules");
        assert_eq!(p.steps[0].args, json!({"group": "g1"}));
        assert_eq!(p.steps[0].text, "切到牌組「g1」");
        let p = plan_with(&d, "repo-a 加標籤 規格、前端", &Scope::default(), &never).unwrap();
        assert_eq!(
            p.steps[0].args,
            json!({"id": "repo-a", "add": ["規格", "前端"], "remove": []})
        );
        assert!(
            never.1.lock().unwrap().is_empty(),
            "規則對上就不問 provider"
        );
    }

    #[test]
    fn agent_plan_is_validated_step_by_step() {
        let (_t, d) = spine_with_repos();
        let p = fixed(json!({"說明": "整理", "steps": [
            {"op": "set_members", "args": {"group": "g1", "repos": ["repo-a"]}, "理由": "b 不用了"},
            {"op": "tag", "args": {"id": "repo-z", "add": ["x"]}},
            {"op": "remove_group", "args": {"name": "g1"}},
            {"op": "set_tier", "args": {"id": "repo-b", "tier": "paused"}},
            {"op": "open_tab", "args": {"tab": "筆記"}},
        ]}));
        let plan = plan_with(
            &d,
            "把 b 移出 g1 然後整理一下",
            &Scope {
                group: Some("g1".into()),
                repos: None,
            },
            &p,
        )
        .unwrap();
        assert_eq!(plan.source, "agent");
        assert_eq!(plan.summary, "整理");
        let prompt = p.1.lock().unwrap().clone();
        assert!(prompt.contains("目前範圍：牌組「g1」") && prompt.contains("- g1：repo-a、repo-b"));
        let s = &plan.steps;
        assert_eq!(s[0].text, "牌組「g1」：移出 repo-b");
        assert!(s[0].danger && s[0].error.is_none() && s[0].why == "b 不用了");
        assert!(s[1]
            .error
            .as_deref()
            .unwrap()
            .starts_with("找不到 repo「repo-z」"));
        assert!(s[2].error.as_deref().unwrap().contains("不在白名單"));
        assert!(s[3].error.as_deref().unwrap().contains("resume_when"));
        assert_eq!(s[4].args, json!({"tab": "notes"}));
    }

    #[test]
    fn validate_limits_and_names() {
        let (_t, d) = spine_with_repos();
        let data = registry::load(&d).unwrap();
        let many: Vec<Value> = (0..11)
            .map(|_| json!({"op": "open_tab", "args": {"tab": "inbox"}}))
            .collect();
        assert!(validate(&data, &json!({"steps": many}), "mcp").is_err());
        assert!(validate(&data, &json!({}), "mcp").is_err());
        let p = validate(
            &data,
            &json!({"steps": [
                {"op": "create_group", "args": {"name": "g1", "repos": ["repo-a"]}},
                {"op": "switch_scope", "args": {"repos": "REPO-A、repo-b"}},
                {"op": "open_repo", "args": {"id": "repo"}},
            ]}),
            "mcp",
        )
        .unwrap();
        assert!(p.steps[0].error.as_deref().unwrap().contains("已經有牌組"));
        assert_eq!(p.steps[1].args, json!({"repos": ["repo-a", "repo-b"]}));
        assert!(p.steps[2]
            .error
            .as_deref()
            .unwrap()
            .contains("最接近：repo-a、repo-b"));
    }
}
