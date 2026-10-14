//! agents 模組的直接測試。Python 沒有獨立的 test_agents.py——這些行為在 Python 是經 collide 測的
//! （test_pack_route_collide.py 的 collide_* 系列）；collide 還沒搬，先在 provider 層把同樣的行為釘住。
//! 名稱不與 Python 一對一，等 collide 搬完，Python 的同名測試會在 tests/pack_route_collide 對應處補上。

use std::path::Path;

use ctx_core::agents::{self, AgentProvider, ClaudeProvider, MockProvider, Verdict};
use ctx_core::Error;
use serde_json::{json, Value};

fn log_count(spine: &Path) -> usize {
    std::fs::read_dir(spine.join(".agent_logs"))
        .map(|d| d.count())
        .unwrap_or(0)
}

/// 對應 test_collide_inner_validation_retry：第 1 次壞、第 2 次好 → 成功。
#[test]
fn mock_inner_validation_retry() {
    let tmp = tempfile::tempdir().unwrap();
    let m = MockProvider::new(1);
    let j = agents::judge_with(&m, "重試路徑測試", tmp.path(), None).unwrap();
    assert_eq!(j.verdict, Verdict::Novel);
    assert_eq!(m.calls(), 2);
}

/// 對應 test_collide_system_unsure_surfaces：兩次都壞 → External（AgentUnsure），不准沉默。
#[test]
fn mock_twice_bad_is_unsure() {
    let tmp = tempfile::tempdir().unwrap();
    let m = MockProvider::new(5);
    let e = agents::judge_with(&m, "必敗測試", tmp.path(), None).unwrap_err();
    assert!(
        matches!(&e, Error::External(msg) if msg.starts_with("輸出驗證兩次皆失敗: ")),
        "{e}"
    );
    assert_eq!(m.calls(), 2);
}

/// 對應 test_agent_raw_log_written：每次 agent 呼叫 raw request/response 必落檔。
#[test]
fn mock_raw_log_written() {
    let tmp = tempfile::tempdir().unwrap();
    agents::judge_with(&MockProvider::new(0), "raw log 測試", tmp.path(), None).unwrap();
    assert!(
        log_count(tmp.path()) > 0,
        "每次 agent 呼叫 raw request/response 必落檔"
    );
    let f = std::fs::read_dir(tmp.path().join(".agent_logs"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let v: Value = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
    assert_eq!(v["provider"], "mock");
    assert_eq!(v["call"], 1);
}

/// 對應 test_mock_verdict_follows_idea_not_template：prompt 模板含三個判定詞，mock 只准看【想法】段。
#[test]
fn mock_verdict_follows_idea_not_template() {
    let tmp = tempfile::tempdir().unwrap();
    let tpl = |idea: &str| {
        format!("判斷是 已知／衝突／真增量\n【想法】\n{idea}\n【組內容】\n已知結論 衝突")
    };
    let j = agents::judge("mock", &tpl("一個完全嶄新的方向"), tmp.path(), None).unwrap();
    assert_eq!(j.verdict, Verdict::Novel);
    let j2 = agents::judge("mock", &tpl("這與既有結論衝突吧"), tmp.path(), None).unwrap();
    assert_eq!(j2.verdict, Verdict::Conflict);
    let j3 = agents::judge("mock", &tpl("快取（已知 的變形）"), tmp.path(), None).unwrap();
    assert_eq!(j3.verdict, Verdict::Known);
}

#[test]
fn judgement_serializes_with_python_keys() {
    let tmp = tempfile::tempdir().unwrap();
    let j = agents::judge("mock", "x", tmp.path(), None).unwrap();
    let v = serde_json::to_value(&j).unwrap();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, agents::JUDGE_KEYS.to_vec());
    assert_eq!(v["判定"], "真增量");
}

#[test]
fn validate_judgement_rejects_bad_shapes() {
    let ok = json!({"判定": "已知", "理由": "r", "證據": "e", "落點建議": "p", "下一步": "n"});
    assert!(agents::validate_judgement(&ok).is_ok());
    let missing = json!({"判定": "已知", "理由": " ", "證據": "e"});
    let e = agents::validate_judgement(&missing)
        .unwrap_err()
        .to_string();
    assert_eq!(e, "缺欄位: 理由, 落點建議, 下一步");
    let bad = json!({"判定": "也許", "理由": "r", "證據": "e", "落點建議": "p", "下一步": "n"});
    let e = agents::validate_judgement(&bad).unwrap_err().to_string();
    assert_eq!(e, "判定值不合法: '也許'（需 已知|衝突|真增量）");
}

#[test]
fn extract_json_takes_outermost_braces() {
    let v = agents::extract_json("前言 {\"a\": {\"b\": 1}} 後記").unwrap();
    assert_eq!(v, json!({"a": {"b": 1}}));
    assert!(agents::extract_json("沒有物件").is_err());
}

/// 對應 test_crossref.test_ask_repo_read_only_and_cites 的前兩行（build_cmd 部分；其餘屬 agentapi）。
#[test]
fn claude_build_cmd_read_only() {
    let cmd = ClaudeProvider.build_cmd("q", None, true);
    assert!(cmd.iter().any(|c| c == "--allowedTools"));
    let i = cmd.iter().position(|c| c == "--disallowedTools").unwrap();
    assert!(cmd[i..].iter().any(|c| c == "Edit"));
    assert_eq!(
        ClaudeProvider.build_cmd("q", Some("opus"), false),
        vec![
            "claude",
            "-p",
            "q",
            "--output-format",
            "json",
            "--setting-sources",
            "project,local",
            "--strict-mcp-config",
            "--model",
            "opus"
        ]
    );
}

/// 一律回空字串的 provider——測 run_text 的空輸出重試。
struct Blank(std::sync::atomic::AtomicUsize);
impl AgentProvider for Blank {
    fn name(&self) -> &'static str {
        "blank"
    }
    fn run_judgement(&self, _: &str, _: &Path, _: Option<&str>) -> ctx_core::Result<Value> {
        Err(Error::External("外部失敗不重試".into()))
    }
    fn run_text(
        &self,
        _: &str,
        _: &Path,
        _: Option<&str>,
        _: Option<&Path>,
        _: bool,
    ) -> ctx_core::Result<String> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok("  ".into())
    }
}

#[test]
fn run_text_empty_retries_then_unsure_and_external_not_retried() {
    let tmp = tempfile::tempdir().unwrap();
    let b = Blank(Default::default());
    let e = agents::run_text_with(&b, "p", tmp.path(), None, None, false).unwrap_err();
    assert!(
        matches!(&e, Error::External(m) if m == "文字輸出兩次皆失敗: 空輸出"),
        "{e}"
    );
    assert_eq!(b.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    // provider 自己丟 External（AgentUnsure）→ 不重試、原樣往上
    let e = agents::judge_with(&b, "p", tmp.path(), None).unwrap_err();
    assert!(matches!(&e, Error::External(m) if m == "外部失敗不重試"));
    // 未知 provider
    assert!(matches!(
        agents::run_text("nope", "p", tmp.path(), None, None, false),
        Err(Error::External(_))
    ));
    // mock 文字輸出
    let out = agents::run_text("mock", "a\nb", tmp.path(), None, None, false).unwrap();
    assert_eq!(out, "mock 摘要：輸入 2 行——重點請看 diff。");
}
