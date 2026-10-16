//! config.yaml 載入與預設值（v2 §3.4 閾值 schema）。公私切分線：私有內容全在這。
//! 對應 Python `repo_context/config.py`。
//!
//! ## 存取 API（其他模組照這個用）
//!
//! `load(spine_dir)` 回 [`Config`]：內部是「DEFAULTS 深合併 config.yaml」後的一棵
//! `serde_json::Value`（保鍵序、保留不認得的鍵），所以 Python 的 `cfg["a"]["b"]` 在這裡兩種寫法：
//!
//! - 點路徑通用存取：`cfg.get("session.role_prompt")`、`cfg.str("provider.ask")`、
//!   `cfg.i64("thresholds.dirty_stale_days")`、`cfg.bool("mcp.admin")`、`cfg.str_list("interrupt")`。
//!   路徑不存在或型別不符＝`None`（等同 Python 的 `.get(...) or {}` 鏈）。
//! - 常用段的 typed 存取：[`Config::thresholds`]、[`Config::provider_default`]、
//!   [`Config::provider_model`]、[`Config::pack_token_budget`]、[`Config::agents`]、
//!   [`Config::schedule`]、[`Config::ruminate`]、[`Config::tag_suggestions`]。
//!   型別不符時退回 DEFAULTS 的值（不讓手寫錯一個欄位就整個引擎掛掉）。
//!
//! 使用者層設定（`~/.config/repo-context/config.json`）：agent 從任何 repo 開起來都要找得到 spine，
//! 不能每個指令都帶 `--spine`、也不能靠 cwd。解析順序：`--spine` ＞ `$REPOENGINE_SPINE` ＞
//! 使用者設定檔 ＞ cwd。`ctx setup` 寫這個檔。測試請用 `*_in`／`find_spine_with` 版本傳顯式路徑，
//! 不要改全域環境變數（Rust 測試同進程平行跑）。

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::Result;

/// 對應 `config.DEFAULTS`。
pub fn defaults() -> Value {
    json!({
        "thresholds": {
            "dirty_stale_days": 4,
            "inactive_days": 21,
            "upstream": {"kinds": ["release"], "prerelease": false},
            "openloop_default_due_days": 14,
        },
        "interrupt": ["irreversible-confirm", "costly-anomaly", "system-unsure"],
        "provider": {
            "default": "mock",
            "models": {"digest": "haiku", "collide": "strong"},
        },
        // P16 會話可開的 agent CLI（terminal 只負責 spawn，帶料 prompt 走 positional）。
        // mcp: "mcp-config"＝支援 --mcp-config 掛本引擎（v1 只保證 Claude Code）；
        //      "cwd"＝靠 cwd 設定檔自動掛載；"none"＝該家 MCP 設定自理，引擎不代管。
        // prompt_flag: prompt 前置旗標（如某家要 ["-i"]），預設空＝直接接在指令後。
        "agents": {
            "claude": {"cmd": ["claude"], "mcp": "mcp-config", "prompt_flag": []},
        },
        "pack": {"token_budget": 60000},
        // tag 詞彙建議（顯示在 tag 編輯器裡供一鍵點選；registry 既有 tag 會自動併入）
        "tags": {"suggestions": []},
        // P12：排程住殼內（S6）。每項 {task: brief|digest|upstream|ruminate|commit, at: "HH:MM", group: <組名|null>}
        "schedule": [],
        // 反芻（ctx fresh）：window_days＝第一次回看幾天；min_shared＝至少幾個共同少見詞才推；
        // max_per_group＝每組每輪最多推幾筆。命中率（ctx fresh --stats）低就把 min_shared 調高。
        "ruminate": {"window_days": 14, "min_shared": 3, "max_per_group": 3},
    })
}

/// 對應 `config._merge`：dict 對 dict 遞迴合併，其餘以 over 為準。
pub fn merge(base: &Value, over: &Value) -> Value {
    let mut out = base.clone();
    if let (Some(o), Value::Object(over)) = (out.as_object_mut(), over) {
        for (k, v) in over {
            let merged = match (o.get(k), v) {
                (Some(b @ Value::Object(_)), Value::Object(_)) => merge(b, v),
                _ => v.clone(),
            };
            o.insert(k.clone(), merged);
        }
    }
    out
}

/// YAML 值轉 JSON 值（非字串鍵轉成字串；NaN／Inf 變 null）。
pub fn yaml_to_json(v: &serde_yaml::Value) -> Value {
    use serde_yaml::Value as Y;
    match v {
        Y::Null => Value::Null,
        Y::Bool(b) => Value::Bool(*b),
        Y::Number(n) => {
            if let Some(i) = n.as_i64() {
                json!(i)
            } else if let Some(u) = n.as_u64() {
                json!(u)
            } else {
                n.as_f64()
                    .and_then(serde_json::Number::from_f64)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            }
        }
        Y::String(s) => Value::String(s.clone()),
        Y::Sequence(seq) => Value::Array(seq.iter().map(yaml_to_json).collect()),
        Y::Mapping(m) => {
            let mut o = Map::new();
            for (k, v) in m {
                let key = match k {
                    Y::String(s) => s.clone(),
                    other => crate::pystr::py_str_yaml(other),
                };
                o.insert(key, yaml_to_json(v));
            }
            Value::Object(o)
        }
        Y::Tagged(t) => yaml_to_json(&t.value),
    }
}

/// JSON 值轉 YAML 值（registry 寫入用）。
pub fn json_to_yaml(v: &Value) -> serde_yaml::Value {
    use serde_yaml::Value as Y;
    match v {
        Value::Null => Y::Null,
        Value::Bool(b) => Y::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Y::Number(i.into())
            } else if let Some(u) = n.as_u64() {
                Y::Number(u.into())
            } else {
                Y::Number(n.as_f64().unwrap_or(0.0).into())
            }
        }
        Value::String(s) => Y::String(s.clone()),
        Value::Array(a) => Y::Sequence(a.iter().map(json_to_yaml).collect()),
        Value::Object(o) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, v) in o {
                m.insert(Y::String(k.clone()), json_to_yaml(v));
            }
            Y::Mapping(m)
        }
    }
}

/// 合併後的 config（見模組註解）。
#[derive(Debug, Clone)]
pub struct Config {
    root: Value,
}

/// `thresholds` 段。
#[derive(Debug, Clone, PartialEq)]
pub struct Thresholds {
    pub dirty_stale_days: i64,
    pub inactive_days: i64,
    pub openloop_default_due_days: i64,
    pub upstream_kinds: Vec<String>,
    pub upstream_prerelease: bool,
}

/// 引擎自己叫 `claude -p` 的政策（config `provider.programmatic`，F-84）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Programmatic {
    Allow,
    Off,
}

impl Programmatic {
    /// config 裡的寫法，也是 `health` 回的字串。
    pub fn as_str(self) -> &'static str {
        match self {
            Programmatic::Allow => "allow",
            Programmatic::Off => "off",
        }
    }
}

/// `agents.<名>` 一項（P16 會話 agent 註冊表）。
#[derive(Debug, Clone, PartialEq)]
pub struct AgentSpec {
    pub cmd: Vec<String>,
    /// `mcp-config` | `cwd` | `none`
    pub mcp: String,
    pub prompt_flag: Vec<String>,
    /// 整項原始值（保留不認得的鍵）。
    pub raw: Value,
}

/// `schedule` 一項。
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleEntry {
    pub task: String,
    pub at: String,
    pub group: Option<String>,
}

/// `ruminate` 段。
#[derive(Debug, Clone, PartialEq)]
pub struct Ruminate {
    pub window_days: i64,
    pub min_shared: i64,
    pub max_per_group: i64,
}

impl Config {
    /// 由 config.yaml 解析後的值建（會先與 DEFAULTS 深合併）。
    pub fn from_value(over: &Value) -> Self {
        Config {
            root: merge(&defaults(), over),
        }
    }

    /// 整棵合併後的值。
    pub fn value(&self) -> &Value {
        &self.root
    }

    /// 點路徑存取：`get("thresholds.upstream.kinds")`。
    pub fn get(&self, dotted: &str) -> Option<&Value> {
        let mut cur = &self.root;
        for part in dotted.split('.') {
            cur = cur.as_object()?.get(part)?;
        }
        Some(cur)
    }

    pub fn str(&self, dotted: &str) -> Option<&str> {
        self.get(dotted)?.as_str()
    }

    pub fn i64(&self, dotted: &str) -> Option<i64> {
        let v = self.get(dotted)?;
        v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
    }

    pub fn f64(&self, dotted: &str) -> Option<f64> {
        self.get(dotted)?.as_f64()
    }

    pub fn bool(&self, dotted: &str) -> Option<bool> {
        self.get(dotted)?.as_bool()
    }

    /// 字串陣列（非字串元素以 Python `str()` 轉）；不存在＝空。
    pub fn str_list(&self, dotted: &str) -> Vec<String> {
        match self.get(dotted) {
            Some(Value::Array(a)) => a.iter().map(crate::pystr::py_str).collect(),
            _ => Vec::new(),
        }
    }

    fn i64_or_default(&self, dotted: &str) -> i64 {
        self.i64(dotted)
            .or_else(|| Config::from_value(&Value::Null).i64(dotted))
            .unwrap_or_default()
    }

    /// 對應 `cfg["thresholds"]`。
    pub fn thresholds(&self) -> Thresholds {
        Thresholds {
            dirty_stale_days: self.i64_or_default("thresholds.dirty_stale_days"),
            inactive_days: self.i64_or_default("thresholds.inactive_days"),
            openloop_default_due_days: self.i64_or_default("thresholds.openloop_default_due_days"),
            upstream_kinds: self.str_list("thresholds.upstream.kinds"),
            upstream_prerelease: self.bool("thresholds.upstream.prerelease").unwrap_or(false),
        }
    }

    /// `provider.programmatic`（F-84，Rust 才有）：引擎能不能自己叫 `claude -p`（另計額度）。
    /// 預設 allow；`"off"` 或 YAML 裸 `off`（會變成 false）＝不准。
    pub fn programmatic(&self) -> Programmatic {
        match self.get("provider.programmatic") {
            Some(Value::Bool(false)) => Programmatic::Off,
            Some(Value::String(s)) if s.trim().eq_ignore_ascii_case("off") => Programmatic::Off,
            _ => Programmatic::Allow,
        }
    }

    /// 對應 `cfg["provider"]["default"]`。
    pub fn provider_default(&self) -> &str {
        self.str("provider.default").unwrap_or("mock")
    }

    /// 對應 `cfg["provider"]["models"].get(task)`（task：digest／collide…）。
    pub fn provider_model(&self, task: &str) -> Option<&str> {
        self.get("provider.models")?
            .as_object()?
            .get(task)?
            .as_str()
    }

    /// 對應 `cfg["pack"]["token_budget"]`。
    pub fn pack_token_budget(&self) -> i64 {
        self.i64_or_default("pack.token_budget")
    }

    /// 對應 `cfg["tags"]["suggestions"]`。
    pub fn tag_suggestions(&self) -> Vec<String> {
        self.str_list("tags.suggestions")
    }

    /// 對應 `cfg["agents"]`：依設定檔順序（claude 內建在最前）。
    pub fn agents(&self) -> Vec<(String, AgentSpec)> {
        let Some(Value::Object(o)) = self.get("agents") else {
            return Vec::new();
        };
        o.iter()
            .filter(|(_, v)| v.is_object())
            .map(|(k, v)| {
                let list = |key: &str| match v.get(key) {
                    Some(Value::Array(a)) => a.iter().map(crate::pystr::py_str).collect(),
                    _ => Vec::new(),
                };
                let spec = AgentSpec {
                    cmd: list("cmd"),
                    mcp: v
                        .get("mcp")
                        .and_then(Value::as_str)
                        .unwrap_or("none")
                        .to_string(),
                    prompt_flag: list("prompt_flag"),
                    raw: v.clone(),
                };
                (k.clone(), spec)
            })
            .collect()
    }

    /// 對應 `cfg.get("schedule") or []`；不是 dict 的項略過。
    pub fn schedule(&self) -> Vec<ScheduleEntry> {
        let Some(Value::Array(a)) = self.get("schedule") else {
            return Vec::new();
        };
        a.iter()
            .filter_map(|e| {
                let o = e.as_object()?;
                let s = |k: &str| o.get(k).filter(|v| !v.is_null()).map(crate::pystr::py_str);
                Some(ScheduleEntry {
                    task: s("task").unwrap_or_else(|| "None".into()),
                    at: s("at").unwrap_or_default(),
                    group: s("group"),
                })
            })
            .collect()
    }

    /// 對應 `cfg["ruminate"]`。
    pub fn ruminate(&self) -> Ruminate {
        Ruminate {
            window_days: self.i64_or_default("ruminate.window_days"),
            min_shared: self.i64_or_default("ruminate.min_shared"),
            max_per_group: self.i64_or_default("ruminate.max_per_group"),
        }
    }
}

/// 對應 `config.load`：讀 `<spine>/config.yaml`（沒有＝全預設）與 DEFAULTS 深合併。
pub fn load(spine_dir: impl AsRef<Path>) -> Result<Config> {
    let p = spine_dir.as_ref().join("config.yaml");
    let over = if p.exists() {
        let text = std::fs::read_to_string(&p)?;
        let y: serde_yaml::Value = serde_yaml::from_str(&text)?;
        yaml_to_json(&y)
    } else {
        Value::Null
    };
    Ok(Config::from_value(&over))
}

// ---- 使用者層設定（2026-09-25 Agent 友善輪）----

/// 對應 `config.user_config_path`：`$XDG_CONFIG_HOME`（或 `~/.config`）`/repo-context/config.json`。
pub fn user_config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::fsutil::expanduser("~/.config"));
    user_config_path_in(&base)
}

/// 同 [`user_config_path`]，但 config 根目錄由呼叫端給（測試用）。
pub fn user_config_path_in(config_home: &Path) -> PathBuf {
    config_home.join("repo-context").join("config.json")
}

/// 對應 `config.load_user_config`：讀不到或壞掉＝空物件。
pub fn load_user_config() -> Value {
    load_user_config_from(&user_config_path())
}

/// 同 [`load_user_config`]，讀指定檔。
pub fn load_user_config_from(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .unwrap_or_else(|| json!({}))
}

/// 對應 `config.save_user_config`：`json.dumps(ensure_ascii=False, indent=1)`。回傳寫入的路徑。
pub fn save_user_config(data: &Value) -> Result<PathBuf> {
    save_user_config_to(&user_config_path(), data)
}

/// 同 [`save_user_config`]，寫指定檔。
pub fn save_user_config_to(path: &Path, data: &Value) -> Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    serde::Serialize::serialize(data, &mut ser)?;
    std::fs::write(path, buf)?;
    Ok(path.to_path_buf())
}

/// 對應 `config.is_spine`：有 config.yaml 或 registry.yaml 就算。
pub fn is_spine(d: impl AsRef<Path>) -> bool {
    let d = d.as_ref();
    d.join("config.yaml").exists() || d.join("registry.yaml").exists()
}

/// 對應 `config.find_spine`：回 spine 路徑或 `None`（呼叫端決定要報錯還是靜默——hook 要靜默）。
pub fn find_spine(explicit: Option<&str>) -> Option<PathBuf> {
    let env = std::env::var("REPOENGINE_SPINE").ok();
    let user = load_user_config();
    find_spine_with(explicit, env.as_deref(), &user)
}

/// 同 [`find_spine`]，環境變數與使用者設定由呼叫端給（測試用；cwd 仍用行程的）。
pub fn find_spine_with(
    explicit: Option<&str>,
    env_spine: Option<&str>,
    user_cfg: &Value,
) -> Option<PathBuf> {
    let from_user = user_cfg.get("spine").and_then(Value::as_str);
    for cand in [explicit, env_spine, from_user, Some(".")]
        .into_iter()
        .flatten()
    {
        if cand.is_empty() {
            continue;
        }
        let p = crate::fsutil::expanduser(cand);
        if is_spine(&p) {
            return Some(crate::fsutil::resolve(&p));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_merge_keeps_defaults_and_unknown_keys() {
        let c = Config::from_value(&json!({
            "thresholds": {"dirty_stale_days": 9},
            "agents": {"codex": {"cmd": ["codex"], "mcp": "none"}},
            "session": {"role_prompt": "x"},
        }));
        let t = c.thresholds();
        assert_eq!(t.dirty_stale_days, 9);
        assert_eq!(t.inactive_days, 21);
        assert_eq!(t.upstream_kinds, vec!["release"]);
        let names: Vec<_> = c.agents().into_iter().map(|(k, _)| k).collect();
        assert_eq!(names, vec!["claude", "codex"]);
        assert_eq!(c.str("session.role_prompt"), Some("x"));
        assert_eq!(c.pack_token_budget(), 60000);
        assert_eq!(c.provider_model("collide"), Some("strong"));
    }

    #[test]
    fn user_config_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let p = user_config_path_in(tmp.path());
        assert_eq!(load_user_config_from(&p), json!({}));
        save_user_config_to(&p, &json!({"spine": "/x/中"})).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "{\n \"spine\": \"/x/中\"\n}"
        );
        assert_eq!(load_user_config_from(&p)["spine"], "/x/中");
    }
}
