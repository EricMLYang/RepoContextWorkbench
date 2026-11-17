//! CLI 介面只增不改（N-52 第 2 批）：agent 會用的 `--json` 輸出形狀、全部子指令的旗標。
//!
//! - `schema/cli.json.shape.json`：在固定的 fixture 上跑 agent 會用的指令（[`SCRIPT`]），把 `--json` 輸出化成
//!   「鍵路徑 → 型別」（陣列取所有元素的聯集，`[]` 表示元素）。只看型別不看值，時間、路徑、編號不影響。
//! - `schema/cli.flags.json`：clap 定義生成的每個子指令的旗標（長短名、別名、必填、收不收值、可選值）。
//! - 各自的 `*.released.json`：上次發版的。相容規則——輸出：鍵不能消失、不能多出新的型別（可以多鍵）；
//!   旗標：子指令與旗標不能消失或改名、選填不能變必填、不能新增必填、收值與否不能變、可選值不能變少。
//!
//! 過期就失敗；`CTX_UPDATE_SCHEMA=1 cargo test -p ctx-cli --test cli_schema` 重生。發版與破壞性變更的流程見
//! docs/DEVELOPMENT.md §6。

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use clap::CommandFactory;
use common::*;
use serde_json::{json, Map, Value};

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema")
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("Value 一定序列化得出來") + "\n"
}

/// 快照檔跟現在生成的比；`CTX_UPDATE_SCHEMA` 有設就重寫。
fn check_snapshot(file: &str, now: &Value) {
    let path = schema_dir().join(file);
    if std::env::var_os("CTX_UPDATE_SCHEMA").is_some() {
        std::fs::create_dir_all(schema_dir()).unwrap();
        std::fs::write(&path, pretty(now)).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&path).unwrap_or_default();
    if on_disk != pretty(now) {
        let old: Value = serde_json::from_str(&on_disk).unwrap_or(Value::Null);
        let changed: Vec<&String> = now
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, v)| old.get(k.as_str()) != Some(v))
            .map(|(k, _)| k)
            .collect();
        panic!(
            "schema/{file} 過期了（變了的：{changed:?}）。\n\
             確定是要的改動就重生：CTX_UPDATE_SCHEMA=1 cargo test -p ctx-cli --test cli_schema，再看 git diff。"
        );
    }
}

fn released(file: &str) -> Value {
    let p = schema_dir().join(file);
    let s = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("讀不到 {}：{e}", p.display()));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("{} 不是 JSON：{e}", p.display()))
}

// ================================================================ --json 輸出形狀

/// 型別名：整數與小數不分（同一個欄位可能剛好是整數值）。
fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// JSON → 「鍵路徑 → 型別集合」。陣列元素的路徑是 `<路徑>[]`，所有元素聯集。
fn shape(v: &Value, at: &str, out: &mut BTreeMap<String, BTreeSet<String>>) {
    out.entry(at.to_string())
        .or_default()
        .insert(kind(v).to_string());
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                shape(x, &format!("{at}.{k}"), out);
            }
        }
        Value::Array(xs) => {
            for x in xs {
                shape(x, &format!("{at}[]"), out);
            }
        }
        _ => {}
    }
}

fn shape_value(v: &Value) -> Value {
    let mut m = BTreeMap::new();
    shape(v, "", &mut m);
    let obj: Map<String, Value> = m
        .into_iter()
        .map(|(k, ts)| {
            let k = if k.is_empty() {
                "$".to_string()
            } else {
                format!("${k}")
            };
            (k, json!(ts.into_iter().collect::<Vec<_>>()))
        })
        .collect();
    Value::Object(obj)
}

/// 輸出形狀的相容規則（純函式）：舊的每個鍵路徑都要還在，而且不能多出舊的沒有的型別。
/// `old`／`new`＝{指令: {路徑: [型別…]}}。
fn shape_breaks(old: &Value, new: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for (cmd, opaths) in old.as_object().into_iter().flatten() {
        let Some(npaths) = new.get(cmd) else {
            out.push(format!("{cmd}：不再有這個指令的輸出"));
            continue;
        };
        for (path, okinds) in opaths.as_object().into_iter().flatten() {
            let Some(nkinds) = npaths.get(path) else {
                out.push(format!("{cmd} {path}：鍵不見了"));
                continue;
            };
            let set = |v: &Value| -> BTreeSet<String> {
                v.as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            };
            let (o, n) = (set(okinds), set(nkinds));
            let extra: Vec<&String> = n.difference(&o).collect();
            if !extra.is_empty() {
                out.push(format!(
                    "{cmd} {path}：型別 {o:?} 變成 {n:?}（多了 {extra:?}）"
                ));
            }
        }
    }
    out
}

/// agent 會用的指令（SKILL.md、`ctx instructions` 提到的）。依序跑在同一個 spine 上：先寫後讀，
/// 讀的指令才有東西可列。標籤＝快照裡的鍵。
///
/// 沒放進來的（輸出不是 JSON 或不穩定）：`instructions`（markdown）、`unread`（人話）、
/// `sessions --digest`／`--grep`（要真的 Claude／Codex 逐字稿）、`ask`（要 provider；mock 的回答是假資料）、
/// `screen`（要 App 在跑才有畫面，否則只有一句說明）。
const SCRIPT: &[(&str, &[&str])] = &[
    ("where", &["where"]),
    ("context", &["context"]),
    ("log", &["log", "快取改用 LRU", "--ref", "decision:x"]),
    ("todo add", &["todo", "add", "驗證快取"]),
    (
        "todo add --due",
        &["todo", "add", "第二件", "--due", "2099-01-01"],
    ),
    ("todo close", &["todo", "close", "#1", "--note", "做完了"]),
    ("todo list", &["todo", "list"]),
    ("todo review", &["todo", "review"]),
    ("todo tidy", &["todo", "tidy"]),
    ("todo brief", &["todo", "brief", "#2"]),
    ("status", &["status", "waiting", "等使用者確認"]),
    (
        "handoff",
        &[
            "handoff",
            "--done",
            "改了快取",
            "--next",
            "補測試",
            "--remaining",
            "文件",
            "--ask",
            "要 A 還是 B？",
            "--option",
            "A",
            "--option",
            "B",
            "--recommend",
            "A",
        ],
    ),
    ("next", &["next"]),
    ("context (after)", &["context"]),
    ("decide list", &["decide", "list"]),
    (
        "decide request",
        &[
            "decide",
            "request",
            "--ask",
            "要發版嗎？",
            "--kind",
            "publish",
        ],
    ),
    ("search", &["search", "widget 快取"]),
    ("read", &["read", "notes:notes/plan.md"]),
    ("resolve", &["resolve", "notes"]),
    ("repo", &["repo", "impl"]),
    ("msg send", &["msg", "send", "notes", "介面改了"]),
    ("msg list", &["msg", "list"]),
    ("relay plan", &["relay", "plan"]),
    (
        "relay report",
        &["relay", "report", "--since", "2000-01-01 00:00"],
    ),
    ("sessions", &["sessions"]),
    ("worktrees", &["worktrees", "impl"]),
    ("loops", &["loops"]),
    ("query", &["query", "--today"]),
    ("registry list", &["registry", "list"]),
    ("group list", &["group", "list"]),
    ("refs check", &["refs", "check"]),
];

/// fixture：spine＋兩個 git repo（impl、notes，都有 notes/plan.md）同在組「產品」；cwd＝impl。
fn fixture(t: &Path) -> (PathBuf, PathBuf) {
    let sp = t.join("spine");
    let c = ctx(t);
    c.ok(&["init", s(&sp)]);
    let impl_ = make_git_repo(t, "impl", 1, 0, true);
    let notes = make_git_repo(t, "notes", 3, 0, true);
    let cs = c.spine(&sp);
    cs.ok(&["registry", "add", "impl", s(&impl_)]);
    cs.ok(&["registry", "add", "notes", s(&notes)]);
    cs.ok(&["group", "add", "產品", "impl,notes"]);
    (sp, impl_)
}

fn json_shapes() -> Value {
    let t = tempfile::tempdir().unwrap();
    let (sp, impl_) = fixture(t.path());
    let c = ctx(t.path()).cwd(&impl_).env("REPOENGINE_SPINE", &sp);
    let mut all = Map::new();
    for (label, args) in SCRIPT {
        let mut argv: Vec<&str> = args.to_vec();
        argv.push("--json");
        let o = c.ok(&argv);
        all.insert(label.to_string(), shape_value(&o.json()));
    }
    Value::Object(all)
}

#[test]
fn json_output_shapes() {
    let now = json_shapes();
    check_snapshot("cli.json.shape.json", &now);
    let broken = shape_breaks(&released("cli.json.shape.released.json"), &now);
    assert!(
        broken.is_empty(),
        "這些 --json 輸出變更會弄壞照上次發版讀欄位的 agent：\n  {}\n\
         改成相容的寫法（加新鍵、舊鍵留著），或照 docs/DEVELOPMENT.md §6 走破壞性變更的流程。",
        broken.join("\n  ")
    );
}

#[test]
fn json_shape_rules() {
    let old = json!({"c": {"$": ["object"], "$.a": ["string"], "$.b": ["null", "string"], "$.xs": ["array"], "$.xs[]": ["number"]}});
    assert_eq!(shape_breaks(&old, &old), Vec::<String>::new());
    // 可以：多鍵、型別變少（原本可能是 null，現在一定有值）
    let grown = json!({"c": {"$": ["object"], "$.a": ["string"], "$.b": ["string"], "$.xs": ["array"], "$.xs[]": ["number"], "$.new": ["boolean"]}, "d": {"$": ["array"]}});
    assert_eq!(shape_breaks(&old, &grown), Vec::<String>::new());
    // 不行：鍵不見、型別換掉、多出 null、指令沒了
    let cases = [
        (
            json!({"c": {"$": ["object"], "$.b": ["null", "string"], "$.xs": ["array"], "$.xs[]": ["number"]}}),
            "c $.a：鍵不見了",
        ),
        (
            json!({"c": {"$": ["object"], "$.a": ["number"], "$.b": ["string"], "$.xs": ["array"], "$.xs[]": ["number"]}}),
            "c $.a：型別",
        ),
        (
            json!({"c": {"$": ["object"], "$.a": ["null", "string"], "$.b": ["string"], "$.xs": ["array"], "$.xs[]": ["number"]}}),
            "c $.a：型別",
        ),
        (
            json!({"c": {"$": ["object"], "$.a": ["string"], "$.b": ["string"], "$.xs": ["array"]}}),
            "c $.xs[]：鍵不見了",
        ),
        (json!({}), "c：不再有"),
    ];
    for (new, expected) in cases {
        let found = shape_breaks(&old, &new);
        assert!(
            found.iter().any(|f| f.contains(expected)),
            "沒擋到 {expected:?}：{found:?}"
        );
    }
    // 形狀化本身：陣列取元素聯集、值不影響
    let a = shape_value(&json!({"t": "2026-01-01", "xs": [{"k": 1}, {"k": null, "j": "x"}]}));
    let b = shape_value(&json!({"t": "1999-12-31", "xs": [{"k": 2.5, "j": "y"}, {"k": null}]}));
    assert_eq!(a, b);
    assert_eq!(a["$.xs[].k"], json!(["null", "number"]));
}

// ================================================================ 旗標表面

/// clap 定義 → {"ctx sub sub": {旗標 id: {long, short, aliases, positional, required, takes_value, values}}}。
fn flags() -> Value {
    fn walk(cmd: &clap::Command, path: &str, out: &mut Map<String, Value>) {
        let mut args = Map::new();
        for a in cmd.get_arguments() {
            let id = a.get_id().as_str();
            if id == "help" || id == "version" {
                continue;
            }
            let mut aliases: Vec<String> = a
                .get_all_aliases()
                .unwrap_or_default()
                .into_iter()
                .map(String::from)
                .collect();
            aliases.sort();
            let values: Vec<String> = a
                .get_possible_values()
                .iter()
                .map(|v| v.get_name().to_string())
                .collect();
            args.insert(
                id.to_string(),
                json!({
                    "long": a.get_long(),
                    "short": a.get_short().map(String::from),
                    "aliases": aliases,
                    "positional": a.get_index(),
                    "required": a.is_required_set(),
                    "takes_value": a.get_action().takes_values(),
                    "values": values,
                }),
            );
        }
        let mut aliases: Vec<String> = cmd.get_all_aliases().map(String::from).collect();
        aliases.sort();
        out.insert(path.to_string(), json!({"aliases": aliases, "args": args}));
        for sub in cmd.get_subcommands() {
            if sub.get_name() == "help" {
                continue;
            }
            walk(sub, &format!("{path} {}", sub.get_name()), out);
        }
    }
    let mut out = Map::new();
    walk(&ctx_cli::cli::Cli::command(), "ctx", &mut out);
    Value::Object(out)
}

/// 旗標的相容規則（純函式）：照舊版能下的指令，新版都要照樣能下。
fn flag_breaks(old: &Value, new: &Value) -> Vec<String> {
    let strs = |v: &Value| -> BTreeSet<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(String::from))
            .collect()
    };
    let mut out = Vec::new();
    for (cmd, oc) in old.as_object().into_iter().flatten() {
        let Some(nc) = new.get(cmd) else {
            out.push(format!("{cmd}：子指令不見了（刪掉或改名）"));
            continue;
        };
        for a in strs(&oc["aliases"]).difference(&strs(&nc["aliases"])) {
            out.push(format!("{cmd}：別名 {a} 不見了"));
        }
        let (oargs, nargs) = (&oc["args"], &nc["args"]);
        for (id, oa) in oargs.as_object().into_iter().flatten() {
            let Some(na) = nargs.get(id) else {
                out.push(format!("{cmd} {id}：旗標不見了（刪掉或改名）"));
                continue;
            };
            for k in ["long", "short", "positional"] {
                if oa[k] != na[k] {
                    out.push(format!("{cmd} {id}：{k} 從 {} 變成 {}", oa[k], na[k]));
                }
            }
            for a in strs(&oa["aliases"]).difference(&strs(&na["aliases"])) {
                out.push(format!("{cmd} {id}：別名 {a} 不見了"));
            }
            if oa["required"] != true && na["required"] == true {
                out.push(format!("{cmd} {id}：選填變必填"));
            }
            if oa["takes_value"] != na["takes_value"] {
                out.push(format!(
                    "{cmd} {id}：收不收值變了（{} → {}）",
                    oa["takes_value"], na["takes_value"]
                ));
            }
            let (ov, nv) = (strs(&oa["values"]), strs(&na["values"]));
            if !nv.is_empty() {
                let gone: Vec<&String> = ov.difference(&nv).collect();
                if ov.is_empty() {
                    out.push(format!("{cmd} {id}：本來不限值，現在只收 {nv:?}"));
                } else if !gone.is_empty() {
                    out.push(format!("{cmd} {id}：不再接受的值 {gone:?}"));
                }
            }
        }
        for (id, na) in nargs.as_object().into_iter().flatten() {
            if oargs.get(id).is_none() && na["required"] == true {
                out.push(format!("{cmd} {id}：新增的旗標是必填"));
            }
        }
    }
    out
}

#[test]
fn flag_surface() {
    let now = flags();
    check_snapshot("cli.flags.json", &now);
    let broken = flag_breaks(&released("cli.flags.released.json"), &now);
    assert!(
        broken.is_empty(),
        "這些 CLI 旗標變更會弄壞照上次發版下指令的 agent／腳本：\n  {}\n\
         改成相容的寫法（舊旗標留成別名、新旗標做成選填），或照 docs/DEVELOPMENT.md §6 走破壞性變更的流程。",
        broken.join("\n  ")
    );
}

#[test]
fn flag_rules() {
    let arg = |long: &str, required: bool, takes: bool, values: Value| {
        json!({"long": long, "short": null, "aliases": [], "positional": null,
               "required": required, "takes_value": takes, "values": values})
    };
    let base = json!({
        "ctx": {"aliases": [], "args": {}},
        "ctx x": {"aliases": ["xx"], "args": {"a": arg("a", false, true, json!([])), "k": arg("kind", false, true, json!(["p", "q"])), "f": arg("f", false, false, json!([]))}},
    });
    assert_eq!(flag_breaks(&base, &base), Vec::<String>::new());
    let with = |f: &dyn Fn(&mut Value)| {
        let mut v = base.clone();
        f(&mut v);
        v
    };
    // 可以：新子指令、新的選填旗標、可選值變多、必填變選填
    for (what, new) in [
        (
            "新子指令",
            with(&|v| v["ctx y"] = json!({"aliases": [], "args": {}})),
        ),
        (
            "新選填旗標",
            with(&|v| v["ctx x"]["args"]["n"] = arg("n", false, true, json!([]))),
        ),
        (
            "可選值變多",
            with(&|v| v["ctx x"]["args"]["k"]["values"] = json!(["p", "q", "r"])),
        ),
        (
            "新別名",
            with(&|v| v["ctx x"]["args"]["a"]["aliases"] = json!(["aa"])),
        ),
    ] {
        assert_eq!(
            flag_breaks(&base, &new),
            Vec::<String>::new(),
            "{what} 應該放行"
        );
    }
    for (new, expected) in [
        (
            with(&|v| drop(v.as_object_mut().unwrap().remove("ctx x"))),
            "ctx x：子指令不見了",
        ),
        (
            with(&|v| v["ctx x"]["aliases"] = json!([])),
            "別名 xx 不見了",
        ),
        (
            with(&|v| drop(v["ctx x"]["args"].as_object_mut().unwrap().remove("a"))),
            "ctx x a：旗標不見了",
        ),
        (
            with(&|v| v["ctx x"]["args"]["a"]["long"] = json!("b")),
            "ctx x a：long",
        ),
        (
            with(&|v| v["ctx x"]["args"]["a"]["required"] = json!(true)),
            "ctx x a：選填變必填",
        ),
        (
            with(&|v| v["ctx x"]["args"]["n"] = arg("n", true, true, json!([]))),
            "ctx x n：新增的旗標是必填",
        ),
        (
            with(&|v| v["ctx x"]["args"]["f"]["takes_value"] = json!(true)),
            "ctx x f：收不收值變了",
        ),
        (
            with(&|v| v["ctx x"]["args"]["k"]["values"] = json!(["p"])),
            "ctx x k：不再接受的值",
        ),
        (
            with(&|v| v["ctx x"]["args"]["a"]["values"] = json!(["only"])),
            "ctx x a：本來不限值",
        ),
    ] {
        let found = flag_breaks(&base, &new);
        assert!(
            found.iter().any(|f| f.contains(expected)),
            "沒擋到 {expected:?}：{found:?}"
        );
    }
}
