//! MCP 介面只增不改（N-52）。
//!
//! - `schema/mcp.tools.json`：由方法表生成的目前介面（兩層各自的 tools/list）。過期就失敗；
//!   `CTX_UPDATE_SCHEMA=1 cargo test -p ctx-cli --test mcp_schema` 重生，再看 git diff。
//! - `schema/mcp.tools.released.json`：上次發版的介面。目前的介面要接受它接受過的每一個呼叫：
//!   擋刪工具／改名、工作層移到管理層、唯讀變非唯讀、刪參數、選填變必填、新增必填、型別或 enum 收得更少。
//!   發版時把 mcp.tools.json 複製過去。真的要破壞，流程見 docs/DEVELOPMENT.md §6。

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{json, Value};

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema")
}

fn read(name: &str) -> Value {
    let p = schema_dir().join(name);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("讀不到 {}：{e}", p.display()));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{} 不是 JSON：{e}", p.display()))
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("Value 一定序列化得出來") + "\n"
}

// ---------------------------------------------------------------- 相容規則（純函式）

const LAYERS: [&str; 2] = ["work", "admin"];

fn tools<'a>(doc: &'a Value, layer: &str) -> impl Iterator<Item = &'a Value> {
    doc[layer].as_array().into_iter().flatten()
}

fn find<'a>(doc: &'a Value, layer: &str, name: &str) -> Option<&'a Value> {
    tools(doc, layer).find(|t| t["name"] == name)
}

fn names(v: &Value) -> BTreeSet<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .map(|x| match x {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .collect()
}

/// `type` 收的種類；沒寫＝什麼都收（None）。
fn kinds(s: &Value) -> Option<BTreeSet<String>> {
    match &s["type"] {
        Value::Null => None,
        Value::Array(_) => Some(names(&s["type"])),
        t => Some(names(&json!([t]))),
    }
}

/// 參數 schema：新的要收舊的收過的一切（`at`＝給人看的位置，例 `handoff.ask.kind`）。
fn input_breaks(old: &Value, new: &Value, at: &str, out: &mut Vec<String>) {
    match (kinds(old), kinds(new)) {
        (_, None) => {}
        (None, Some(n)) => out.push(format!("{at}：本來不限型別，現在只收 {n:?}")),
        (Some(o), Some(n)) => {
            if !o.is_subset(&n) {
                out.push(format!("{at}：型別 {o:?} 變成 {n:?}（收得更少）"));
            }
        }
    }
    if new.get("enum").is_some() {
        let (oe, ne) = (names(&old["enum"]), names(&new["enum"]));
        if old.get("enum").is_none() {
            out.push(format!("{at}：本來不限值，現在只收 {ne:?}"));
        } else if !oe.is_subset(&ne) {
            let gone: Vec<_> = oe.difference(&ne).collect();
            out.push(format!("{at}：不再接受的值 {gone:?}"));
        }
    }
    if old.get("items").is_some() || new.get("items").is_some() {
        input_breaks(&old["items"], &new["items"], &format!("{at}[]"), out);
    }
    let (oprops, nprops) = (&old["properties"], &new["properties"]);
    for (k, os) in oprops.as_object().into_iter().flatten() {
        match nprops.get(k) {
            None => out.push(format!("{at}.{k}：參數被刪掉（或改名）")),
            Some(ns) => input_breaks(os, ns, &format!("{at}.{k}"), out),
        }
    }
    let (oreq, nreq) = (names(&old["required"]), names(&new["required"]));
    for k in nreq.difference(&oreq) {
        if oprops.get(k).is_some() {
            out.push(format!("{at}.{k}：選填變必填"));
        } else {
            out.push(format!("{at}.{k}：新增的參數是必填"));
        }
    }
}

/// 舊介面（上次發版）→ 新介面：會弄壞照舊介面呼叫的 agent 的變更。空＝相容。
fn breaks(old: &Value, new: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for layer in LAYERS {
        for ot in tools(old, layer) {
            let name = ot["name"].as_str().unwrap_or_default();
            let nt = match find(new, layer, name) {
                Some(nt) => nt,
                None if layer == "work" && find(new, "admin", name).is_some() => {
                    out.push(format!("{name}：從工作層移到管理層（預設看不到了）"));
                    continue;
                }
                // 管理層的工具移到工作層＝更多人看得到，照常比
                None => match find(new, "work", name) {
                    Some(nt) => nt,
                    None => {
                        out.push(format!("{name}：工具不見了（刪掉或改名）"));
                        continue;
                    }
                },
            };
            let ro = |t: &Value| t["annotations"]["readOnlyHint"] == true;
            if ro(ot) && !ro(nt) {
                out.push(format!(
                    "{name}：不再標 readOnlyHint（Codex 會開始每次要核准）"
                ));
            }
            input_breaks(&ot["inputSchema"], &nt["inputSchema"], name, &mut out);
        }
    }
    out
}

// ---------------------------------------------------------------- 快照

#[test]
fn snapshot_matches_the_method_table() {
    let now = ctx_cli::mcp::interface();
    let path = schema_dir().join("mcp.tools.json");
    if std::env::var_os("CTX_UPDATE_SCHEMA").is_some() {
        std::fs::create_dir_all(schema_dir()).unwrap();
        std::fs::write(&path, pretty(&now)).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    if on_disk != pretty(&now) {
        let old: Value = serde_json::from_str(&on_disk).unwrap_or(Value::Null);
        let changed: Vec<String> = LAYERS
            .iter()
            .flat_map(|l| tools(&now, l).map(move |t| (l, t)))
            .filter(|(l, t)| find(&old, l, t["name"].as_str().unwrap_or_default()) != Some(*t))
            .map(|(_, t)| t["name"].as_str().unwrap_or_default().to_string())
            .collect();
        panic!(
            "schema/mcp.tools.json 跟方法表不一致（變了的工具：{changed:?}）。\n\
             確定是要的改動就重生：CTX_UPDATE_SCHEMA=1 cargo test -p ctx-cli --test mcp_schema，再看 git diff。"
        );
    }
}

#[test]
fn the_interface_only_grows_since_the_last_release() {
    let released = read("mcp.tools.released.json");
    let broken = breaks(&released, &ctx_cli::mcp::interface());
    assert!(
        broken.is_empty(),
        "這些變更會弄壞照上次發版介面（schema/mcp.tools.released.json）呼叫的 agent：\n  {}\n\
         改成相容的寫法（新參數做成選填、舊參數留著），或照 docs/DEVELOPMENT.md §6 走破壞性變更的流程。",
        broken.join("\n  ")
    );
}

#[test]
fn tool_names_are_unique_and_listing_is_the_table() {
    let now = ctx_cli::mcp::interface();
    let mut seen = BTreeSet::new();
    for l in LAYERS {
        for t in tools(&now, l) {
            assert!(
                seen.insert(t["name"].as_str().unwrap().to_string()),
                "重複的工具 {}",
                t["name"]
            );
            let s = &t["inputSchema"];
            assert_eq!(s["type"], "object", "{}", t["name"]);
            assert!(
                s["properties"].is_object() && s["required"].is_array(),
                "{}",
                t["name"]
            );
            // 必填的一定是宣告過的參數
            for k in names(&s["required"]) {
                assert!(
                    s["properties"].get(&k).is_some(),
                    "{}：必填 {k} 沒宣告",
                    t["name"]
                );
            }
        }
    }
}

// ---------------------------------------------------------------- 規則本身的單元測試

fn tool(name: &str, props: Value, required: Value, read_only: bool) -> Value {
    let mut t = json!({"name": name, "description": "說明",
        "inputSchema": {"type": "object", "properties": props, "required": required}});
    if read_only {
        t["annotations"] = json!({"readOnlyHint": true});
    }
    t
}

fn base() -> Value {
    json!({
        "work": [
            tool("w", json!({
                "text": {"type": "string", "description": "文字"},
                "mode": {"type": "string", "enum": ["a", "b"]},
                "tags": {"type": "array", "items": {"type": "string"}},
                "ask": {"type": "object", "properties": {"q": {"type": "string"}, "k": {"type": "string"}}, "required": ["q"]},
            }), json!(["text"]), true),
            tool("r", json!({}), json!([]), true),
        ],
        "admin": [tool("a", json!({"id": {"type": "string"}}), json!([]), false)],
    })
}

/// 在 base 上改一處：`f` 拿到可改的副本。
fn with(f: impl FnOnce(&mut Value)) -> Value {
    let mut v = base();
    f(&mut v);
    v
}

fn w_schema(v: &mut Value) -> &mut Value {
    &mut v["work"][0]["inputSchema"]
}

#[test]
fn rules_catch_each_kind_of_break() {
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "刪工具",
            with(|v| drop(v["work"].as_array_mut().unwrap().remove(1))),
            "r：工具不見了",
        ),
        (
            "改工具名",
            with(|v| v["admin"][0]["name"] = json!("a2")),
            "a：工具不見了",
        ),
        (
            "工作層移到管理層",
            with(|v| {
                let t = v["work"].as_array_mut().unwrap().remove(1);
                v["admin"].as_array_mut().unwrap().push(t);
            }),
            "r：從工作層移到管理層",
        ),
        (
            "刪參數",
            with(|v| {
                drop(
                    w_schema(v)["properties"]
                        .as_object_mut()
                        .unwrap()
                        .remove("tags"),
                )
            }),
            "w.tags：參數被刪掉",
        ),
        (
            "選填變必填",
            with(|v| w_schema(v)["required"] = json!(["text", "mode"])),
            "w.mode：選填變必填",
        ),
        (
            "新增必填參數",
            with(|v| {
                w_schema(v)["properties"]["n"] = json!({"type": "string"});
                w_schema(v)["required"] = json!(["text", "n"]);
            }),
            "w.n：新增的參數是必填",
        ),
        (
            "型別換掉",
            with(|v| w_schema(v)["properties"]["text"]["type"] = json!("integer")),
            "w.text：型別",
        ),
        (
            "enum 少一個值",
            with(|v| w_schema(v)["properties"]["mode"]["enum"] = json!(["a"])),
            "w.mode：不再接受的值",
        ),
        (
            "本來不限值、加了 enum",
            with(|v| w_schema(v)["properties"]["text"]["enum"] = json!(["x"])),
            "w.text：本來不限值",
        ),
        (
            "陣列元素型別換掉",
            with(|v| w_schema(v)["properties"]["tags"]["items"]["type"] = json!("integer")),
            "w.tags[]：型別",
        ),
        (
            "巢狀物件的選填變必填",
            with(|v| w_schema(v)["properties"]["ask"]["required"] = json!(["q", "k"])),
            "w.ask.k：選填變必填",
        ),
        (
            "巢狀物件刪欄位",
            with(|v| {
                drop(
                    w_schema(v)["properties"]["ask"]["properties"]
                        .as_object_mut()
                        .unwrap()
                        .remove("k"),
                )
            }),
            "w.ask.k：參數被刪掉",
        ),
        (
            "唯讀變非唯讀",
            with(|v| drop(v["work"][1].as_object_mut().unwrap().remove("annotations"))),
            "r：不再標 readOnlyHint",
        ),
    ];
    for (what, new, expected) in cases {
        let found = breaks(&base(), &new);
        assert!(
            found.iter().any(|f| f.contains(expected)),
            "{what}：沒擋到（要有 {expected:?}），實際：{found:?}"
        );
    }
}

#[test]
fn rules_allow_what_only_grows() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "新工具",
            with(|v| {
                v["work"]
                    .as_array_mut()
                    .unwrap()
                    .push(tool("n", json!({}), json!([]), false))
            }),
        ),
        (
            "新的選填參數",
            with(|v| w_schema(v)["properties"]["n"] = json!({"type": "boolean"})),
        ),
        (
            "enum 多一個值",
            with(|v| w_schema(v)["properties"]["mode"]["enum"] = json!(["a", "b", "c"])),
        ),
        (
            "description 改字",
            with(|v| {
                v["work"][0]["description"] = json!("新說明");
                w_schema(v)["properties"]["text"]["description"] = json!("新的參數說明");
            }),
        ),
        ("必填變選填", with(|v| w_schema(v)["required"] = json!([]))),
        (
            "型別收得更多",
            with(|v| w_schema(v)["properties"]["text"]["type"] = json!(["string", "integer"])),
        ),
        (
            "拿掉 enum 限制",
            with(|v| {
                drop(
                    w_schema(v)["properties"]["mode"]
                        .as_object_mut()
                        .unwrap()
                        .remove("enum"),
                )
            }),
        ),
        (
            "非唯讀變唯讀",
            with(|v| v["admin"][0]["annotations"] = json!({"readOnlyHint": true})),
        ),
        (
            "管理層移到工作層",
            with(|v| {
                let t = v["admin"].as_array_mut().unwrap().remove(0);
                v["work"].as_array_mut().unwrap().push(t);
            }),
        ),
        (
            "順序改變",
            with(|v| v["work"].as_array_mut().unwrap().reverse()),
        ),
    ];
    assert_eq!(breaks(&base(), &base()), Vec::<String>::new());
    for (what, new) in cases {
        assert_eq!(
            breaks(&base(), &new),
            Vec::<String>::new(),
            "{what} 應該放行"
        );
    }
}
