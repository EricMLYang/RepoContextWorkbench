//! L2 E2E：跑真 `ctx` binary，全流程＋不變式 1/3（append-only、git 單一提交者）。
//! 對應 tests/test_cli_e2e.py。

mod common;

use std::path::Path;
use std::process::Command;

use common::*;
use serde_json::Value;

fn commit_count(repo: &Path) -> i64 {
    let o = Command::new("git")
        .args(["-C", s(repo), "rev-list", "--count", "HEAD"])
        .output()
        .unwrap();
    if o.status.success() {
        String::from_utf8_lossy(&o.stdout).trim().parse().unwrap()
    } else {
        0
    }
}

#[test]
fn test_full_pipeline() {
    let t = tempfile::tempdir().unwrap();
    let tmp = t.path();
    let spine_dir = tmp.join("s");
    let ra = make_git_repo(tmp, "repo-a", 1, 0, true);
    let rb = make_git_repo(tmp, "repo-b", 2, 1, true);
    age(&rb.join("wip0.md"), 6);
    let c = ctx(tmp);
    let cs = c.clone().spine(&spine_dir);

    // init → register → group
    c.ok(&["init", s(&spine_dir)]);
    cs.ok(&["registry", "add", "repo-a", s(&ra)]);
    cs.ok(&["registry", "add", "repo-b", s(&rb)]);
    cs.ok(&["group", "add", "g1", "repo-a,repo-b"]);
    let r = cs.ok(&["registry", "list"]);
    assert!(r.stdout.contains("repo-a") && r.stdout.contains("repo-b"));

    // collect / pack / brief
    let r = cs.ok(&["collect", "--group", "g1"]);
    assert!(r.stdout.contains("repo-b"));
    let r = cs.ok(&["pack", "--group", "g1"]);
    assert!(r.stdout.contains("打包完成"));
    let r = cs.ok(&["brief", "--group", "g1"]);
    assert!(r.stdout.contains("早晨簡報") && r.stdout.contains("沉默摘要"));

    // 事件寫入：合法／壞事件 dead-letter
    cs.ok(&[
        "append",
        "--type",
        "open-loop",
        "--source",
        "manual",
        "--kv",
        "#1",
        "--body",
        "opened →「試一下」",
    ]);
    let r = cs.run(&[
        "append",
        "--type",
        "bogus",
        "--source",
        "hotkey",
        "--body",
        "想法別丟",
        "--dead-letter",
    ]);
    assert!(r.code == 1 && r.stdout.contains("dead-letter"));

    // 碰撞（mock、同步）→ outcome 回連 → stats
    let r = cs.ok(&[
        "collide",
        "submit",
        "全新的想法試撞",
        "--group",
        "g1",
        "--provider",
        "mock",
        "--wait",
    ]);
    assert!(r.stdout.contains("已丟進碰撞") && r.stdout.contains("判定"));
    let cid = format!("{}-a", today());
    cs.ok(&[
        "append",
        "--type",
        "outcome",
        "--source",
        "manual",
        "--kv",
        &format!("ref:collision:{cid}"),
        "--body",
        "長成了一份筆記",
    ]);
    let r = cs.ok(&["query", "--stats"]);
    assert!(r.stdout.contains("靈感命中率 100%"));

    // route → repo inbox
    cs.ok(&["route", "--text", "撞出來的結論", "--dest", "repo:repo-a"]);
    assert!(std::fs::read_dir(ra.join("01_inbox")).unwrap().any(|e| e
        .unwrap()
        .path()
        .extension()
        .is_some_and(|x| x == "md")));

    // 不變式 3：以上所有動作皆不產生 commit；只有 spine commit 會
    assert_eq!(commit_count(&spine_dir), 0);
    cs.ok(&["commit", "-m", "ritual: e2e"]);
    assert_eq!(commit_count(&spine_dir), 1);

    // 不變式 1：append-only——再寫一筆後，舊內容仍是新內容的前綴
    let day_file = today_file(&spine_dir);
    let before = std::fs::read_to_string(&day_file).unwrap();
    cs.ok(&[
        "append", "--type", "decision", "--source", "manual", "--body", "尾筆",
    ]);
    let after = std::fs::read_to_string(&day_file).unwrap();
    assert!(after.starts_with(&before) && after.len() > before.len());

    // 往返驗證：整條脊椎 parse 得回來（query 不炸、筆數合理）
    let r = cs.ok(&["query", "--today"]);
    assert!(r.stdout.trim().lines().count() >= 6);

    // unread → ack
    let r = cs.ok(&["unread"]);
    assert!(!r.stdout.contains("未讀 0 則"));
    cs.ok(&["unread", "--ack"]);
    let r = cs.ok(&["unread"]);
    assert!(r.stdout.contains("未讀 0 則"));
}

#[test]
fn test_audit_exit_code() {
    let t = tempfile::tempdir().unwrap();
    let spine_dir = t.path().join("s");
    let c = ctx(t.path());
    c.ok(&["init", s(&spine_dir)]);
    let cs = c.spine(&spine_dir);
    cs.ok(&["registry", "add", "ghost", s(&t.path().join("nope"))]);
    let r = cs.run(&["registry", "audit"]);
    assert!(r.code == 1 && r.stdout.contains("路徑失效"));
}

#[test]
fn test_cli_refuses_non_spine_dir() {
    let t = tempfile::tempdir().unwrap();
    let r = ctx(t.path()).spine(&t.path().join("empty")).run(&["query"]);
    assert!(r.code != 0 && r.all().contains("不是 spine repo"));
}

/// 拆機報告落地輪：registry scan（mani 課）／lint（衛生迴圈）／pack --estimate。
#[test]
fn test_scan_lint_estimate_cli() {
    let t = tempfile::tempdir().unwrap();
    let tmp = t.path();
    let spine_dir = tmp.join("s");
    make_git_repo(&tmp.join("zone"), "repo-new", 0, 0, true);
    let c = ctx(tmp);
    c.ok(&["init", s(&spine_dir)]);
    let cs = c.spine(&spine_dir);
    let zone = tmp.join("zone");
    let r = cs.ok(&["registry", "scan", s(&zone)]);
    assert!(r.stdout.contains("候選") && r.stdout.contains("repo-new"));
    cs.ok(&["registry", "scan", s(&zone), "--apply"]);
    let r = cs.ok(&["registry", "list"]);
    assert!(r.stdout.contains("repo-new"));
    let r = cs.ok(&["pack", "--estimate"]);
    assert!(r.stdout.contains("repo-new") && r.stdout.contains("合計"));
    let r = cs.ok(&["lint"]);
    assert!(r.stdout.contains("零紅字"));
    // 弄髒脊椎（斷 ref）→ lint exit 1（跟 audit 同形態）
    cs.ok(&[
        "append",
        "--type",
        "chosen",
        "--source",
        "manual",
        "--kv",
        "ref:presented:00:01",
        "--body",
        "斷鏈",
    ]);
    let r = cs.run(&["lint"]);
    assert!(r.code == 1 && r.stdout.contains("ref 斷鏈"));
}

/// 回歸：registry set <id> <key> <value> 之前因 argparse 位置參數共用而寫錯欄位。
#[test]
fn test_registry_set_field() {
    let t = tempfile::tempdir().unwrap();
    let tmp = t.path();
    let spine_dir = tmp.join("s");
    let ra = make_git_repo(tmp, "repo-a", 1, 0, true);
    let c = ctx(tmp);
    c.ok(&["init", s(&spine_dir)]);
    let cs = c.spine(&spine_dir);
    cs.ok(&["registry", "add", "repo-a", s(&ra), "--tier", "paused"]);
    let r = cs.ok(&["registry", "set", "repo-a", "resume_when", "等 Q4"]);
    assert!(r.stdout.contains("resume_when") && r.stdout.contains("等 Q4"));
    let reds = cs.run(&["registry", "audit"]);
    assert_eq!(reds.code, 0); // resume_when 補上後 audit 不再紅字
    let data: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(spine_dir.join("registry.yaml")).unwrap())
            .unwrap();
    let entry = data["repos"]
        .as_sequence()
        .unwrap()
        .iter()
        .find(|x| x["id"].as_str() == Some("repo-a"))
        .unwrap();
    assert_eq!(entry["resume_when"].as_str(), Some("等 Q4"));
    assert!(entry.get("等 Q4").is_none()); // 沒有變成錯的 key 名
}

/// P4/P5/P9/P12/P13/P16 走真 CLI：upstream 無事不報、digest 增量、spawn 出生登記、
/// timer --once 補課、notify 分類、session --print 指令組裝。
#[test]
fn test_new_primitives_pipeline() {
    let t = tempfile::tempdir().unwrap();
    let tmp = t.path();
    let spine_dir = tmp.join("s");
    let ra = make_git_repo(tmp, "repo-a", 1, 0, true);
    let c = ctx(tmp);
    c.ok(&["init", s(&spine_dir)]);
    let cs = c.spine(&spine_dir);
    cs.ok(&["registry", "add", "repo-a", s(&ra)]);
    cs.ok(&["group", "add", "g1", "repo-a"]);

    // P4：組內無 external → 講清楚，不是沉默
    let r = cs.ok(&["upstream", "--group", "g1"]);
    assert!(r.stdout.contains("無 external repo"));

    // P5：mock digest 首輪 ok、二輪水位線擋住
    let r = cs.ok(&["digest", "--group", "g1"]);
    assert!(r.stdout.contains("repo-a: ok"));
    assert!(std::fs::read_dir(spine_dir.join("groups/g1/digests"))
        .unwrap()
        .any(|e| e.unwrap().path().extension().is_some_and(|x| x == "md")));
    let r = cs.ok(&["digest", "--group", "g1"]);
    assert!(r.stdout.contains("無新 commit"));

    // P7→P9：碰撞 → 升格 → 命中率/存活率都算得出來
    cs.ok(&[
        "collide",
        "submit",
        "值得長成 repo 的想法",
        "--group",
        "g1",
        "--no-run",
    ]);
    let cid = format!("{}-a", today());
    let born = tmp.join("born");
    let r = cs.ok(&[
        "spawn",
        "born-repo",
        s(&born),
        "--origin",
        &format!("collision:{cid}"),
    ]);
    assert!(r.stdout.contains("已升格") && born.join(".git").is_dir());
    let r = cs.ok(&["query", "--stats"]);
    assert!(r.stdout.contains("靈感命中率 100%") && r.stdout.contains("存活率 100%"));

    // P12：schedule 進 config → timer --once 當日補跑一次、同日不重跑
    let cfgp = spine_dir.join("config.yaml");
    let cfg = std::fs::read_to_string(&cfgp).unwrap();
    std::fs::write(
        &cfgp,
        format!("{cfg}\nschedule:\n  - {{task: commit, at: \"00:00\"}}\n"),
    )
    .unwrap();
    let r = cs.ok(&["timer", "--once"]);
    assert!(r.stdout.contains("✔ commit@00:00@"), "{}", r.stdout);
    assert_eq!(commit_count(&spine_dir), 1); // commit 任務＝單一提交者的排程形態
    let r = cs.ok(&["timer", "--once"]);
    assert!(r.stdout.contains("無到期任務"));

    // P13：目前未讀全是一般事件 → notify 只累積不打斷
    let r = cs.ok(&["notify"]);
    assert!(!r.stdout.contains("[打斷]") && r.stdout.contains("未讀"));

    // P16：session --print——料檔＋claude 指令＋.mcp.json
    let r = cs.ok(&["session", "--group", "g1", "--task", "寫文", "--print"]);
    assert!(r.stdout.contains("料：") && r.stdout.contains("claude"));
    assert!(spine_dir.join(".mcp.json").exists());
}

/// MCP server 走真 stdio：initialize → tools/list → tools/call（含 validator 拒寫）。
#[test]
fn test_mcp_stdio_roundtrip() {
    let t = tempfile::tempdir().unwrap();
    let spine_dir = t.path().join("s");
    let c = ctx(t.path());
    c.ok(&["init", s(&spine_dir)]);
    let msgs = [
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        serde_json::json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "spine_append",
                       "arguments": {"type": "decision", "source": "agent", "body": "MCP 往返"}}}),
        serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "spine_append", "arguments": {"type": "bogus", "source": "agent"}}}),
    ];
    let input: Vec<String> = msgs.iter().map(|m| m.to_string()).collect();
    let r = c
        .spine(&spine_dir)
        .stdin(&(input.join("\n") + "\n"))
        .run(&["mcp", "--admin"]);
    let lines: Vec<Value> = r
        .stdout
        .trim()
        .lines()
        .map(|x| serde_json::from_str(x).unwrap())
        .collect();
    assert_eq!(lines.len(), 4); // notification 不回
    let by_id = |i: i64| lines.iter().find(|x| x["id"] == i).unwrap();
    assert_eq!(by_id(1)["result"]["serverInfo"]["name"], "repo_context");
    assert!(by_id(2)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "collide_submit"));
    assert_eq!(by_id(3)["result"]["isError"], false);
    assert_eq!(by_id(4)["result"]["isError"], true);
    assert!(by_id(4)["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("未知事件型別"));
    // 事件真的落了脊椎
    assert!(std::fs::read_to_string(today_file(&spine_dir))
        .unwrap()
        .contains("MCP 往返"));
}

#[test]
fn test_registry_remove() {
    let t = tempfile::tempdir().unwrap();
    let tmp = t.path();
    let spine_dir = tmp.join("s");
    let ra = make_git_repo(tmp, "repo-a", 1, 0, true);
    let c = ctx(tmp);
    c.ok(&["init", s(&spine_dir)]);
    let cs = c.spine(&spine_dir);
    cs.ok(&["registry", "add", "repo-a", s(&ra)]);
    cs.ok(&["group", "add", "g1", "repo-a"]);
    let r = cs.ok(&["registry", "remove", "repo-a"]);
    assert!(r.stdout.contains("已移除"));
    let r = cs.ok(&["registry", "list"]);
    assert!(!r.stdout.contains("repo-a"));
    let r = cs.run(&["registry", "remove", "repo-a"]);
    assert_ne!(r.code, 0); // 不存在要報錯
}

/// 2026-09-08 組為單位輪：關係設定／查詢、組情境卡、活動脈動的 CLI 全路徑。
#[test]
fn test_relations_context_pulse_cli() {
    let t = tempfile::tempdir().unwrap();
    let tmp = t.path();
    let spine_dir = tmp.join("s");
    let ra = make_git_repo(tmp, "repo-a", 1, 0, true);
    let rb = make_git_repo(tmp, "repo-b", 3, 0, true);
    let c = ctx(tmp);
    c.ok(&["init", s(&spine_dir)]);
    let cs = c.spine(&spine_dir);
    cs.ok(&["registry", "add", "repo-a", s(&ra)]);
    cs.ok(&["registry", "add", "repo-b", s(&rb)]);
    cs.ok(&["group", "add", "g1", "repo-a,repo-b"]);
    let r = cs.ok(&[
        "registry",
        "relate",
        "repo-a",
        "repo-b",
        "--kind",
        "pm-of",
        "--note",
        "a 規劃 b",
    ]);
    assert!(r.stdout.contains("pm-of"));
    let r = cs.ok(&["registry", "relations", "repo-b"]);
    assert!(r.stdout.contains("PM 是") && r.stdout.contains("repo-a"));
    let r = cs.run(&["registry", "relate", "repo-a", "repo-b", "--kind", "nope"]);
    assert!(r.code != 0 && r.all().contains("kind"));
    let r = cs.ok(&["group", "context", "g1"]);
    assert!(r.stdout.contains("# 組情境卡 g1") && r.stdout.contains("規劃（PM）→ repo-b"));
    let r = cs.ok(&["pulse", "--group", "g1"]);
    assert!(r.stdout.contains("近 7 天") && r.stdout.contains("init")); // 最近 commit 主旨列出
    cs.ok(&["registry", "unrelate", "repo-a", "repo-b"]);
    let r = cs.ok(&["registry", "relations"]);
    assert!(r.stdout.contains("無關係"));
}

/// 2026-09-12 §3：工作摘要與組目標在命令列也拿得到（跟工作台同一份資料）。
#[test]
fn test_group_summary_and_goal_cli() {
    let t = tempfile::tempdir().unwrap();
    let tmp = t.path();
    let spine_dir = tmp.join("s");
    let ra = make_git_repo(tmp, "repo-a", 1, 0, true);
    let c = ctx(tmp);
    c.ok(&["init", s(&spine_dir)]);
    let cs = c.spine(&spine_dir);
    cs.ok(&["registry", "add", "repo-a", s(&ra)]);
    cs.ok(&["group", "add", "g1", "repo-a"]);
    // 還沒登記目標／沒有進度 → 摘要直說沒有依據，不編話
    let r = cs.ok(&["group", "summary", "g1"]);
    assert!(r.stdout.contains("# 本組工作摘要 g1"));
    assert!(r.stdout.contains("尚未登記這組的目標") && r.stdout.contains("脊椎沒有本組的進度紀錄"));
    let r = cs.ok(&["group", "goal", "g1"]);
    assert!(r.stdout.contains("尚未登記目標"));
    // 寫目標＝人的動作，留痕
    cs.ok(&["group", "goal", "g1", "--text", "把兩個 repo 的說法對齊"]);
    let r = cs.ok(&["group", "summary", "g1"]);
    assert!(r.stdout.contains("把兩個 repo 的說法對齊") && r.stdout.contains("registry.yaml"));
    let r = cs.ok(&["query", "--type", "decision"]);
    assert!(r.stdout.contains("decision [cli] group:g1")); // [cli]＝自己出手，不進收件匣
    let day = std::fs::read_dir(spine_dir.join("spine").join("events"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "md"))
        .unwrap();
    assert!(std::fs::read_to_string(day)
        .unwrap()
        .contains("本組目標：把兩個 repo 的說法對齊（從 CLI）"));
}

// ── Rust 版刻意差異 ─────────────────────────────────────────────────────

/// `ctx ui`（瀏覽器模式）不搬：印說明改用 `ctx app`，exit 2。
#[test]
fn test_ui_is_retired() {
    let t = tempfile::tempdir().unwrap();
    let r = ctx(t.path()).run(&["ui"]);
    assert_eq!(r.code, 2);
    assert!(r.stderr.contains("ctx app"));
}

/// argparse 的用法錯誤＝exit 2；clap 一樣。
#[test]
fn test_usage_error_exit_2() {
    let t = tempfile::tempdir().unwrap();
    assert_eq!(ctx(t.path()).run(&["registry", "relate", "a", "b"]).code, 2);
    assert_eq!(ctx(t.path()).run(&["no-such-cmd"]).code, 2);
    assert_eq!(ctx(t.path()).run(&["--help"]).code, 0);
}
