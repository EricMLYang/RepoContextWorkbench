//! MCP 工具參數的型別（N-52）：每個工具一個 `#[derive(Deserialize, JsonSchema)]` struct，
//! `inputSchema` 由它生成（[`input_schema`]），`tools/call` 的 arguments 反序列化成它（[`parse`]）。
//!
//! 欄位用這裡的寬鬆型別，而不是 `String`／`bool`：agent 常送錯型別（`limit: "3"`、`text: 123`），
//! 以前手寫的取值函式照 Python 的規則照收（`str()`、真值），這裡照舊——反序列化永遠不因型別失敗，
//! 驗值的事留給引擎（它的錯誤訊息才講得出「status 需為 working／…」）。
//!
//! - 必填＝欄位型別不是 `Option`（或 `#[schemars(required)]`）；schema 的 `required` 由此而來，
//!   缺值的檢查也照它（[`parse`] 先查，訊息跟以前一樣）。
//! - 參數說明寫在欄位的 doc comment（單行）或 `#[schemars(description = ...)]`（要接長字串時）。
//! - enum：`#[schemars(extend("enum" = 常數))]`，常數用引擎的那份（`agentapi::STATUSES`…）。

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use ctx_core::agentapi::Target;
use ctx_core::{pystr, registry, Error, Result};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use crate::truthy;

// ---------------------------------------------------------------- 寬鬆的欄位型別

/// 宣告一個「收任何 JSON、自己轉」的欄位型別：schema 是固定的一小段。
fn fixed(schema: Value) -> Schema {
    Schema::try_from(schema).expect("寫死的 schema 一定是物件")
}

/// 字串：非字串照 Python `str()` 轉（agent 送 `3` 也收成 `"3"`）。
#[derive(Debug, Clone, Default)]
pub struct Str(pub String);

impl<'de> Deserialize<'de> for Str {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(Str(pystr::py_str(&Value::deserialize(d)?)))
    }
}
impl JsonSchema for Str {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "Str".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "string"})
    }
}

/// 真假：Python 真值（`"yes"`、`1` 都算真；`""`、`0` 算假）。
#[derive(Debug, Clone, Copy, Default)]
pub struct Flag(pub bool);

impl<'de> Deserialize<'de> for Flag {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(Flag(truthy(&Value::deserialize(d)?)))
    }
}
impl JsonSchema for Flag {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "Flag".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "boolean"})
    }
}

/// 整數：數字（小數取整）或數字字串；其他值當沒給。
#[derive(Debug, Clone, Copy, Default)]
pub struct Int(pub Option<i64>);

impl<'de> Deserialize<'de> for Int {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(Int(match Value::deserialize(d)? {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }))
    }
}
impl JsonSchema for Int {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "Int".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "integer"})
    }
}

/// 字串清單：陣列逐項轉字串；單一值算一項。
#[derive(Debug, Clone, Default)]
pub struct List(pub Vec<String>);

impl<'de> Deserialize<'de> for List {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(List(match Value::deserialize(d)? {
            Value::Array(xs) => xs.iter().map(pystr::py_str).collect(),
            Value::Null => Vec::new(),
            v => vec![pystr::py_str(&v)],
        }))
    }
}
impl JsonSchema for List {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "List".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "array", "items": {"type": "string"}})
    }
}

/// `repos`：逗號字串或陣列；空字串／空陣列＝沒給。schema 照舊宣告成字串。
#[derive(Debug, Clone, Default)]
pub struct Repos(pub Option<Vec<String>>);

impl<'de> Deserialize<'de> for Repos {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(Repos(match Value::deserialize(d)? {
            Value::Null => None,
            Value::Array(xs) if xs.is_empty() => None,
            Value::Array(xs) => Some(xs.iter().map(pystr::py_str).collect()),
            Value::String(s) if s.is_empty() => None,
            v => Some(registry::split_ids(&pystr::py_str(&v))),
        }))
    }
}
impl JsonSchema for Repos {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "Repos".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "string"})
    }
}

/// `workbench_apply` 的 steps：原樣交給引擎驗（`[{op, args}]`）。
#[derive(Debug, Clone, Default)]
pub struct Steps(pub Value);

impl<'de> Deserialize<'de> for Steps {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(Steps(Value::deserialize(d)?))
    }
}
impl JsonSchema for Steps {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> Cow<'static, str> {
        "Steps".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        fixed(serde_json::json!({"type": "array", "items": {"type": "object"}}))
    }
}

// ---------------------------------------------------------------- 取值小工具

/// 選填字串（null／缺＝None）。
pub fn text(v: &Option<Str>) -> Option<String> {
    v.as_ref().map(|s| s.0.clone())
}
/// 選填字串，空字串也當沒給。
pub fn nz(v: &Option<Str>) -> Option<String> {
    text(v).filter(|s| !s.is_empty())
}
/// 選填真假（缺＝false）。
pub fn flag(v: &Option<Flag>) -> bool {
    v.is_some_and(|f| f.0)
}
/// 選填整數（缺或認不得＝None）。
pub fn int(v: &Option<Int>) -> Option<i64> {
    v.and_then(|i| i.0)
}
/// 選填清單（缺＝空）。
pub fn list(v: &Option<List>) -> Vec<String> {
    v.as_ref().map(|l| l.0.clone()).unwrap_or_default()
}

// ---------------------------------------------------------------- 共用的參數組（用 #[serde(flatten)] 攤進工具參數）

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct Cwd {
    /// 用哪個目錄推主場；省略＝server 啟動目錄（通常就是你的專案）
    pub cwd: Option<Str>,
}

impl Cwd {
    /// `_w` 的 cwd 規則：有給就用，沒給用 server 的。
    pub fn dir(&self, here: &Path) -> PathBuf {
        nz(&self.cwd)
            .map(PathBuf::from)
            .unwrap_or_else(|| here.to_path_buf())
    }
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct Scope {
    /// 組名；省略＝依 cwd 推出的主場
    pub group: Option<Str>,
    /// 逗號分隔 repo id（臨時組合，優先於 group）
    pub repos: Option<Repos>,
}

impl Scope {
    pub fn group(&self) -> Option<String> {
        text(&self.group)
    }
    pub fn repos(&self) -> Option<Vec<String>> {
        self.repos.as_ref().and_then(|r| r.0.clone())
    }
    /// 管理層用：解出組名與成員。
    pub fn resolve(&self, d: &Path) -> Result<(String, Vec<registry::Repo>)> {
        registry::resolve_group(d, self.group().as_deref(), self.repos().as_deref())
    }
    /// 工作層的範圍：cwd＋組＋臨時組合。
    pub fn target(&self, cwd: &Cwd, here: &Path) -> Target {
        Target {
            cwd: Some(cwd.dir(here)),
            group: nz(&self.group),
            repos: self.repos(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct Cross {
    /// 寫入主場以外的範圍。只有在使用者明確同意後才設 true
    pub cross_scope_ok: Option<Flag>,
}

impl Cross {
    pub fn ok(&self) -> bool {
        flag(&self.cross_scope_ok)
    }
}

// ---------------------------------------------------------------- schema 與解析

/// 參數型別 → MCP 的 `inputSchema`。schemars 生成後整理成這個 server 一向的形狀（`tools/list` 跟 Python
/// 對拍、也是 agent 看得到的介面，不能隨 schemars 版本漂移）：
/// - 不要 `$schema`、`title`、頂層 `description`（工具說明另外給）；
/// - `Option` 帶進來的 `"null"` 拿掉（null＝沒給，本來就收）；
/// - 鍵的順序固定：type、items、properties、required、description、enum；
/// - 頂層一定有 `properties` 與 `required`（空的也寫）。
pub fn input_schema<P: JsonSchema>() -> Value {
    let gen = schemars::generate::SchemaSettings::draft2020_12()
        .with(|s| {
            s.meta_schema = None;
            s.inline_subschemas = true;
        })
        .into_generator();
    let mut v = tidy(gen.into_root_schema_for::<P>().to_value());
    if let Value::Object(m) = &mut v {
        m.remove("description");
        let props = m
            .remove("properties")
            .unwrap_or_else(|| Value::Object(Map::new()));
        let req = m
            .remove("required")
            .unwrap_or_else(|| Value::Array(Vec::new()));
        m.insert("properties".into(), props);
        m.insert("required".into(), req);
    }
    v
}

const KEY_ORDER: [&str; 6] = [
    "type",
    "items",
    "properties",
    "required",
    "description",
    "enum",
];

fn tidy(v: Value) -> Value {
    let Value::Object(mut m) = v else { return v };
    m.remove("$schema");
    m.remove("title");
    if let Some(Value::Array(ts)) = m.get_mut("type") {
        ts.retain(|t| t != "null");
        if ts.len() == 1 {
            let t = ts.remove(0);
            m.insert("type".into(), t);
        }
    }
    if let Some(Value::Object(props)) = m.remove("properties") {
        let props = props.into_iter().map(|(k, p)| (k, tidy(p))).collect();
        m.insert("properties".into(), Value::Object(props));
    }
    if let Some(items) = m.remove("items") {
        m.insert("items".into(), tidy(items));
    }
    let mut out = Map::new();
    for k in KEY_ORDER {
        if let Some(x) = m.remove(k) {
            out.insert(k.into(), x);
        }
    }
    out.extend(m);
    Value::Object(out)
}

/// 缺必填參數時怎麼報錯——兩層的訊息本來就不同，照舊（agent 靠它們自我修正）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// 工作層：照 Python TypeError——`handoff() missing 1 required positional argument: 'done'`；
    /// 缺值、null、空字串都算缺，一次列出全部。
    Positional,
    /// 管理層：照 Python KeyError——只報第一個缺的 `'dir'`；空字串不算缺。
    Key,
}

fn is_missing(v: Option<&Value>, how: Missing) -> bool {
    match v {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => how == Missing::Positional && s.is_empty(),
        _ => false,
    }
}

/// 檢查必填參數（`required` 由 schema 來，順序同 schema）。
pub fn check_required(
    tool: &str,
    a: &Map<String, Value>,
    required: &[String],
    how: Missing,
) -> Result<()> {
    let missing: Vec<&str> = required
        .iter()
        .map(String::as_str)
        .filter(|k| is_missing(a.get(*k), how))
        .collect();
    let Some(first) = missing.first() else {
        return Ok(());
    };
    if how == Missing::Key {
        return Err(Error::invalid(format!("'{first}'")));
    }
    let quoted: Vec<String> = missing.iter().map(|k| format!("'{k}'")).collect();
    let list = match quoted.len() {
        1 => quoted[0].clone(),
        2 => format!("{} and {}", quoted[0], quoted[1]),
        n => format!("{}, and {}", quoted[..n - 1].join(", "), quoted[n - 1]),
    };
    Err(Error::External(format!(
        "{tool}() missing {} required positional argument{}: {list}",
        missing.len(),
        if missing.len() == 1 { "" } else { "s" }
    )))
}

/// `required` 清單（schema 裡的順序）。
pub fn required_of(schema: &Value) -> Vec<String> {
    schema["required"]
        .as_array()
        .map(|xs| {
            xs.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// arguments → 參數型別：先查必填（訊息同以前），再反序列化。欄位型別都收任何 JSON，
/// 反序列化失敗只可能是參數型別本身寫錯——照實回給 agent，不會 panic。
pub fn parse<P: for<'de> Deserialize<'de>>(
    tool: &str,
    a: &Map<String, Value>,
    required: &[String],
    how: Missing,
) -> Result<P> {
    check_required(tool, a, required, how)?;
    serde_json::from_value(Value::Object(a.clone()))
        .map_err(|e| Error::External(format!("{tool} 的參數讀不懂：{e}")))
}
