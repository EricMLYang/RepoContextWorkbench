//! 前端型別的契約（N-52 第 3 批）：`get_state`／`get_scan` 回的大多是手組的 `serde_json::Value`，
//! 沒有型別可以生成 TypeScript；改用契約測試——這裡在固定的 testkit spine 上跑引擎，把輸出存成
//! `desktop/src/test/fixtures/engine-contract.json`，vitest（`src/test/contract.test.ts`）讀它，
//! 驗 `src/api/types.ts` 宣告為必有的欄位都在、型別對。
//!
//! 這支測試在 fixture 的形狀（頂層鍵與型別）跟引擎現在的輸出不同時失敗：
//! `CTX_UPDATE_SCHEMA=1 cargo test -p ctx-core --test ipc_contract` 重生，再跑 `npm test` 看前端型別跟不跟得上。
//! （同一支 binary 只有這個測試：改 `CODEX_HOME` 不會影響別的測試。）

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ctx_core::agentapi::{self, Target};
use ctx_core::term::{TermInfo, TermKind};
use ctx_core::testkit::spine_with_repos;
use ctx_core::workbench::{api_state, build_scan, handle_action, StateScope};
use serde_json::{json, Map, Value};

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../desktop/src/test/fixtures/engine-contract.json")
}

/// 字串裡的暫存路徑換成 `<tmp>`（fixture 進 git，不要帶機器上的路徑）。
fn scrub(v: Value, tmp: &str) -> Value {
    match v {
        Value::String(s) => Value::String(s.replace(tmp, "<tmp>")),
        Value::Array(xs) => Value::Array(xs.into_iter().map(|x| scrub(x, tmp)).collect()),
        Value::Object(m) => Value::Object(m.into_iter().map(|(k, x)| (k, scrub(x, tmp))).collect()),
        other => other,
    }
}

fn engine_outputs() -> Value {
    let (t, d) = spine_with_repos();
    // 機器上的 Codex 額度不能混進來（api_state 會讀 ~/.codex）
    std::env::set_var("CODEX_HOME", t.path().join("no-codex"));
    let g1 = Target {
        cwd: None,
        group: Some("g1".into()),
        repos: None,
    };
    agentapi::add_todo(&d, "補測試", Some("2099-01-01"), &g1, false, "agent").unwrap();
    agentapi::log_decision(&d, "快取改用 LRU", &g1, None, false, "agent").unwrap();
    agentapi::handoff_with_ask(
        &d,
        "改了快取",
        "補測試",
        &[] as &[String],
        None,
        &g1,
        false,
        "agent",
    )
    .unwrap();
    handle_action(
        &d,
        "relate",
        &json!({"a": "repo-a", "b": "repo-b", "kind": "pm-of", "note": "n"}),
        None,
    )
    .unwrap();
    let term = TermInfo {
        sid: "t1".into(),
        title: "Agent 任務".into(),
        alive: true,
        kind: TermKind::Agent,
        origin: Some("group:g1".into()),
        scope: Some("g1".into()),
        scope_repos: None,
        started: String::new(),
        task: None,
        home_group: None,
    };
    let state = api_state(&d, &StateScope::new(Some("g1"), None), &[term]).unwrap();
    let scan = build_scan(&d, Some("g1"), None).unwrap();
    let tmp = t.path().to_string_lossy().into_owned();
    // macOS 的暫存目錄有時以 /private 開頭出現
    let out = json!({"get_state": state, "get_scan": scan});
    let out = scrub(scrub(out, &format!("/private{tmp}")), &tmp);
    drop(d);
    out
}

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

/// 每個命令的頂層鍵 → 型別（fixture 過期與否只看這個；值天天變）。
fn top_shape(v: &Value) -> BTreeMap<String, BTreeMap<String, &'static str>> {
    v.as_object()
        .into_iter()
        .flatten()
        .map(|(cmd, out)| {
            let keys = out
                .as_object()
                .map(|m: &Map<String, Value>| m.iter().map(|(k, x)| (k.clone(), kind(x))).collect())
                .unwrap_or_default();
            (cmd.clone(), keys)
        })
        .collect()
}

fn read(p: &Path) -> Value {
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

#[test]
fn frontend_fixture_matches_engine_output() {
    let now = engine_outputs();
    let p = fixture_path();
    if std::env::var_os("CTX_UPDATE_SCHEMA").is_some() {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        return;
    }
    let (want, got) = (top_shape(&now), top_shape(&read(&p)));
    assert!(
        want == got,
        "desktop/src/test/fixtures/engine-contract.json 跟引擎現在的輸出形狀不同：\n  引擎：{want:?}\n  fixture：{got:?}\n\
         重生：CTX_UPDATE_SCHEMA=1 cargo test -p ctx-core --test ipc_contract，再跑 desktop 的 npm test 看 types.ts 跟不跟得上。"
    );
}
