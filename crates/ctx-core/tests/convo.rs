//! F-134：兩家的掃描器產出同一種會話與訊息；逐字稿、交接包、回填、全文搜尋都只面對這一層。
//! 逐字稿 fixture 都是手寫的（照 Codex 0.169／Claude Code 實檔的格式，內容是編的）。

use std::io::Write;
use std::path::{Path, PathBuf};

use ctx_core::config::AgentKind;
use ctx_core::convo::codex::CodexScanner;
use ctx_core::convo::{self, index, Item, Role, Scanner, Sources, Start, ToolAction};
use ctx_core::registry;
use ctx_core::testkit::spine_with_repos;
use serde_json::{json, Value};

const CX: &str = "01a1da06-c73f-79b3-bb63-a14cb2e0b2b6";
const CX_EXEC: &str = "01a1da14-2615-7571-a7d4-2e6da4de1c9f";
const CL: &str = "02908d1e-7843-436e-9629-55eaa75fdb85";
/// 假的 token（不是真的憑證）：索引與片段裡都不能出現原文。
const FAKE_TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";

fn write_lines(p: &Path, rows: &[Value]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let text: String = rows.iter().map(|r| format!("{r}\n")).collect();
    std::fs::write(p, text).unwrap();
}

fn append_lines(p: &Path, rows: &[Value]) {
    let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
    for r in rows {
        writeln!(f, "{r}").unwrap();
    }
}

fn meta(id: &str, cwd: &Path, source: Value) -> Value {
    json!({"timestamp": "2026-11-14T12:27:19.843Z", "type": "session_meta", "payload": {
        "id": id, "session_id": id, "cwd": cwd, "timestamp": "2026-11-14T12:27:19.759Z",
        "source": source, "originator": "codex-tui", "cli_version": "0.169.0",
        "base_instructions": {"text": "x".repeat(20_000)}, "git": {"branch": "main"}}})
}

fn ri(payload: Value) -> Value {
    json!({"timestamp": "2026-11-14T12:28:00.000Z", "type": "response_item", "payload": payload})
}

fn user(text: &str) -> Value {
    ri(
        json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}),
    )
}

/// 一份 Codex 0.169 格式的 rollout（工具多半包在 `exec` 的 JS 裡）。
fn codex_rows(cwd: &Path) -> Vec<Value> {
    vec![
        meta(CX, cwd, json!("vscode")),
        json!({"timestamp": "2026-11-14T12:27:20Z", "type": "world_state", "payload": {"full": true}}),
        ri(
            json!({"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "系統指示"}]}),
        ),
        user("<environment_context>\n  <cwd>/r</cwd>\n</environment_context>"),
        user("# AGENTS.md instructions for /r\n…"),
        user(&format!(
            "幫我把索引改成增量，順便看 incremental 的做法；token 是 {FAKE_TOKEN}"
        )),
        // 同一句話在舊版事件裡又出現一次：不能重複
        json!({"type": "event_msg", "payload": {"type": "user_message", "message": "幫我把索引改成增量"}}),
        ri(json!({"type": "reasoning", "summary": [], "encrypted_content": "gAAAA"})),
        ri(
            json!({"type": "custom_tool_call", "name": "exec", "call_id": "c1",
            "input": "text(await tools.exec_command({cmd:\"cargo test -q\",max_output_tokens:2000}));"}),
        ),
        ri(json!({"type": "custom_tool_call_output", "call_id": "c1",
            "output": [{"type": "input_text", "text": "Script completed\nOutput:\nok"}]})),
        ri(
            json!({"type": "custom_tool_call", "name": "exec", "call_id": "c2",
            "input": "text(await tools.apply_patch(\"*** Begin Patch\\n*** Update File: /r/src/index.rs\\n@@\\n-a\\n+b\\n*** End Patch\"));"}),
        ),
        // 舊版格式：function_call shell＋JSON 包起來的輸出
        ri(
            json!({"type": "function_call", "name": "shell", "call_id": "c3",
            "arguments": "{\"command\":[\"bash\",\"-lc\",\"git status\"]}"}),
        ),
        ri(json!({"type": "function_call_output", "call_id": "c3",
            "output": "{\"output\":\"fatal: not a git repo\",\"metadata\":{\"exit_code\":128}}"})),
        ri(
            json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "改好了，增量索引的測試都過"}]}),
        ),
        json!({"type": "event_msg", "payload": {"type": "agent_message", "message": "改好了"}}),
        json!({"type": "event_msg", "payload": {"type": "task_complete", "last_agent_message": "改好了，增量索引的測試都過"}}),
        json!({"timestamp": "2026-11-14T12:40:00Z", "type": "compacted", "payload": {"message": "前情提要：索引改增量"}}),
    ]
}

fn codex_home_with(home: &Path, cwd: &Path) -> PathBuf {
    let p = home
        .join("sessions/2026/11/14")
        .join(format!("rollout-2026-11-14T20-27-19-{CX}.jsonl"));
    write_lines(&p, &codex_rows(cwd));
    // codex exec（程式化呼叫）不列、不進索引
    write_lines(
        &home
            .join("sessions/2026/11/14")
            .join(format!("rollout-2026-11-14T20-41-56-{CX_EXEC}.jsonl")),
        &[
            meta(CX_EXEC, cwd, json!("exec")),
            user("exec 裡的增量 secret"),
        ],
    );
    std::fs::write(
        home.join("session_index.jsonl"),
        format!("{}\n", json!({"id": CX, "thread_name": "索引改增量"})),
    )
    .unwrap();
    p
}

fn claude_rows(cwd: &Path) -> Vec<Value> {
    vec![
        json!({"type":"user","entrypoint":"cli","cwd":cwd,"timestamp":"2026-11-15T01:00:00Z","gitBranch":"dev",
               "message":{"content":"登入頁的 session cookie 怎麼處理"}}),
        json!({"type":"ai-title","aiTitle":"處理登入 cookie"}),
        json!({"type":"assistant","timestamp":"2026-11-15T01:00:05Z","message":{"content":[
            {"type":"thinking","thinking":"想一下"},
            {"type":"text","text":"我改用 HttpOnly cookie"},
            {"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/r/login.rs"}},
            {"type":"tool_use","id":"t2","name":"Bash","input":{"command":"cargo test"}}]}}),
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"ok","is_error":false}]}}),
    ]
}

fn claude_root_with(root: &Path, cwd: &Path) -> PathBuf {
    let p = root
        .join(convo::claude::project_dir_name(cwd))
        .join(format!("{CL}.jsonl"));
    write_lines(&p, &claude_rows(cwd));
    p
}

#[test]
fn codex_scanner_reads_messages_in_order_and_skips_injected_and_duplicates() {
    let t = tempfile::tempdir().unwrap();
    let cwd = t.path().join("repo");
    let p = codex_home_with(&t.path().join("codex"), &cwd);
    let sc = CodexScanner::new(t.path().join("codex"));
    let items: Vec<Item> = convo::Items::open(&sc, &p, Start::Head).unwrap().collect();
    let shape: Vec<(Role, String, Option<ToolAction>)> = items
        .iter()
        .filter_map(|i| match i {
            Item::Message(m) => Some((m.role, m.text.clone(), m.tool.as_ref().map(|t| t.action))),
            _ => None,
        })
        .collect();
    assert_eq!(
        shape,
        [
            (
                Role::User,
                format!("幫我把索引改成增量，順便看 incremental 的做法；token 是 {FAKE_TOKEN}"),
                None
            ),
            (Role::Tool, "cargo test -q".into(), Some(ToolAction::Shell)),
            (Role::Tool, "/r/src/index.rs".into(), Some(ToolAction::Edit)),
            (Role::Tool, "git status".into(), Some(ToolAction::Shell)),
            (Role::Assistant, "改好了，增量索引的測試都過".into(), None),
            (Role::Compact, "前情提要：索引改增量".into(), None),
        ]
    );
    let names: Vec<&str> = items
        .iter()
        .filter_map(|i| match i {
            Item::Message(m) => m.tool.as_ref().map(|t| t.name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["exec_command", "apply_patch", "shell"]);
    // 工具結果配回呼叫；非零結束碼＝錯誤
    let (paired, _) = convo::pair(items.into_iter());
    let git = paired
        .iter()
        .find(|p| p.message.text == "git status")
        .unwrap();
    assert_eq!(git.result.as_deref(), Some("fatal: not a git repo"));
    assert!(git.error);
    let cargo = paired
        .iter()
        .find(|p| p.message.text == "cargo test -q")
        .unwrap();
    assert!(cargo.result.as_deref().unwrap().contains("ok"));
    assert!(!cargo.error);

    // 會話清單：exec 不列；標題來自 session_index
    let got = sc.sessions(std::slice::from_ref(&cwd));
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, CX);
    assert_eq!(got[0].agent, AgentKind::Codex);
    assert_eq!(got[0].title.as_deref(), Some("索引改增量"));
    assert_eq!(got[0].branch.as_deref(), Some("main"));
    assert_eq!(sc.locate(CX, None).as_deref(), Some(p.as_path()));
}

#[test]
fn transcripts_and_digests_work_for_both_agents() {
    let t = tempfile::tempdir().unwrap();
    let cwd = t.path().join("repo");
    let home = t.path().join("codex");
    let root = t.path().join("claude");
    codex_home_with(&home, &cwd);
    claude_root_with(&root, &cwd);
    let src = Sources::new(Some(&root), Some(&home));

    let tl = ctx_core::timeline::find_in(&src, CX, None, None).unwrap();
    assert_eq!(tl.agent, "codex");
    assert_eq!(tl.title.as_deref(), Some("索引改增量"));
    let kinds: Vec<String> = tl
        .frames
        .iter()
        .map(|f| {
            serde_json::to_value(f.kind)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(kinds, ["you", "tool", "tool", "tool", "reply", "compact"]);
    assert_eq!(tl.frames[1].tool.as_deref(), Some("exec_command"));
    let (md, hidden) = ctx_core::timeline::digest(&tl, CX, Some("/r"), "2026-11-18 10:00");
    assert!(md.contains("- 來源：Codex 會話"), "{md}");
    assert!(md.contains("- /r/src/index.rs"), "{md}");
    assert!(md.contains("- `cargo test -q`"), "{md}");
    assert!(md.contains("- `git status` ✗"), "{md}");
    assert!(
        md.contains("## 最後的回覆\n改好了，增量索引的測試都過"),
        "{md}"
    );
    assert!(!md.contains(FAKE_TOKEN), "{md}");
    assert!(hidden >= 1);

    // 同一個 id 沒指定哪一家：Claude Code 先找；Claude Code 的交接包跟以前一樣
    let tl = ctx_core::timeline::find_in(&src, CL, None, None).unwrap();
    assert_eq!(tl.agent, "claude");
    let (md, _) = ctx_core::timeline::digest(&tl, CL, None, "2026-11-18 10:00");
    assert!(md.contains("- 來源：Claude Code 會話"), "{md}");
    assert!(md.contains("- /r/login.rs"), "{md}");
    assert!(md.contains("- `cargo test`"), "{md}");
    // 指定錯的那一家＝找不到
    assert!(ctx_core::timeline::find_in(&src, CL, None, Some(AgentKind::Codex)).is_err());
}

#[test]
fn past_sessions_list_both_agents_through_the_scanners() {
    let (t, d) = spine_with_repos();
    let ra = registry::get_repo(&d, "repo-a").unwrap().path();
    let home = t.path().join("codex");
    let root = t.path().join("claude");
    codex_home_with(&home, &ra);
    claude_root_with(&root, &ra);
    let got =
        ctx_core::history::past_sessions_in(&d, Some("g1"), None, 10, &root, Some(&home)).unwrap();
    let mut agents: Vec<&str> = got.iter().map(|s| s.agent).collect();
    agents.sort();
    assert_eq!(agents, ["claude", "codex"]);
    let cx = got.iter().find(|s| s.agent == "codex").unwrap();
    assert_eq!(cx.title, "索引改增量");
    assert_eq!(cx.repo, "repo-a");
    assert!(
        cx.resume.ends_with(&format!("codex resume {CX}")),
        "{}",
        cx.resume
    );
    let cl = got.iter().find(|s| s.agent == "claude").unwrap();
    assert_eq!(cl.title, "處理登入 cookie");
    assert_eq!(cl.reply.as_deref(), Some("我改用 HttpOnly cookie"));
}

#[test]
fn backfill_scan_and_extract_cover_codex_sessions() {
    let (t, d) = spine_with_repos();
    let ra = registry::get_repo(&d, "repo-a").unwrap().path();
    let home = t.path().join("codex");
    let root = t.path().join("claude");
    let p = codex_home_with(&home, &ra);
    claude_root_with(&root, &ra);
    // 30 分鐘內有寫＝live；調回 5 小時前
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(5 * 3600);
    for f in [
        &p,
        &root
            .join(convo::claude::project_dir_name(&ra))
            .join(format!("{CL}.jsonl")),
    ] {
        std::fs::File::options()
            .write(true)
            .open(f)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    let src = Sources::new(Some(&root), Some(&home));
    let recs = ctx_core::backfill::scan_in(&d, &src, None).unwrap();
    assert_eq!(recs.len(), 2, "{recs:?}");
    let cx = recs.iter().find(|r| r.sid == CX).unwrap();
    assert_eq!(cx.agent, AgentKind::Codex);
    assert_eq!(cx.repo.as_deref(), Some("repo-a"));
    assert_eq!(cx.user_turns, 1);
    assert_eq!(cx.title.as_deref(), Some("索引改增量"));
    assert_eq!(cx.status, ctx_core::backfill::SessionStatus::Pending);
    // JSON：Codex 的多一欄 agent；Claude Code 的不變
    let js = serde_json::to_value(&recs).unwrap();
    let cl = js
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["sid"] == CL)
        .unwrap();
    assert!(cl.get("agent").is_none(), "{cl}");
    let cxj = js
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["sid"] == CX)
        .unwrap();
    assert_eq!(cxj["agent"], "codex");

    let lines = ctx_core::backfill::compact_codex(&p).unwrap();
    assert_eq!(lines.len(), 5, "{lines:?}");
    assert!(
        lines[0].starts_with("[t1] 【使用者】幫我把索引改成增量"),
        "{lines:?}"
    );
    assert_eq!(lines[1], "〔工具 exec_command〕cargo test -q");
    assert_eq!(lines[4], "【agent】改好了，增量索引的測試都過");

    // 抽取＋彙整（假的模型）：Codex 會話的候選標 agent，收下的事件註明是 Codex 會話
    let call =
        |prompt: &str| -> ctx_core::Result<String> {
            if prompt.contains("逐場整理") {
                assert!(prompt.contains("（Codex）"), "{prompt}");
                Ok(
                    json!({"decisions": [{"text": "索引改增量", "basis": "全量太慢",
                "sid": &CX[..8], "date": "2026-11-14"}], "open_todos": [], "handoff": null,
                "goal_hints": []})
                    .to_string(),
                )
            } else {
                Ok(json!({"summary": "改索引", "decisions": [], "todos": [], "done": [], "next": "",
                "goal_hints": []})
            .to_string())
            }
        };
    let rep = ctx_core::backfill::run(
        &d,
        &call,
        &ctx_core::backfill::RunOpts {
            root: Some(root.clone()),
            codex: Some(home.clone()),
            jobs: 1,
            ..Default::default()
        },
        &|_: &str| {},
    )
    .unwrap();
    assert_eq!(rep.extracted, 2, "{rep:?}");
    let saved: Value = serde_json::from_str(
        &std::fs::read_to_string(d.join(format!(".state/backfill/sessions/{CX}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["agent"], "codex");
    let cands = ctx_core::backfill::candidates(&d, Some("repo-a")).unwrap();
    assert_eq!(cands[0]["decisions"][0]["agent"], "codex");
    ctx_core::backfill::accept(&d, "repo-a", &["d1"]).unwrap();
    let evs = ctx_core::spine::iter_events(&d).unwrap();
    let ev = evs.iter().rev().find(|e| e.source == "backfill").unwrap();
    assert!(
        ev.body
            .contains(&format!("（回填自 2026-11-14 Codex 會話 {}）", &CX[..8])),
        "{}",
        ev.body
    );
}

fn db_texts(d: &Path) -> Vec<String> {
    let c = rusqlite_open(d);
    let mut out = Vec::new();
    for sql in [
        "SELECT text FROM messages_fts",
        "SELECT coalesce(title,'') || coalesce(prompt,'') || coalesce(reply,'') FROM sessions",
    ] {
        let mut q = c.prepare(sql).unwrap();
        out.extend(
            q.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .map(Result::unwrap),
        );
    }
    out
}

fn rusqlite_open(d: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(index::db_path(d)).unwrap()
}

#[test]
fn session_search_finds_chinese_and_english_respects_scope_and_masks_secrets() {
    let (t, d) = spine_with_repos();
    let ra = registry::get_repo(&d, "repo-a").unwrap().path();
    let home = t.path().join("codex");
    let root = t.path().join("claude");
    codex_home_with(&home, &ra);
    let cl = claude_root_with(&root, &ra);
    let src = Sources::new(Some(&root), Some(&home));
    let all = |q: &str| index::search(&d, &src, Some("g1"), None, q, 10).unwrap();

    // 兩個字的中文詞（trigram 查不到，走 LIKE）、三個字以上、英文不分大小寫、多詞
    let f = all("索引");
    assert_eq!(f.hits.len(), 1, "{f:?}");
    assert_eq!(f.hits[0].agent, "codex");
    assert_eq!(f.hits[0].id, CX);
    assert_eq!(f.hits[0].title, "索引改增量");
    assert!(f.hits[0].snippet.contains("索引"), "{}", f.hits[0].snippet);
    assert!(f.hits[0].resume.ends_with(&format!("codex resume {CX}")));
    assert_eq!(
        f.index.by_agent,
        [("claude".to_string(), 1), ("codex".to_string(), 1)]
    );
    assert_eq!(all("增量索引").hits.len(), 1);
    assert_eq!(all("INCREMENTAL").hits[0].id, CX);
    assert_eq!(all("cookie").hits[0].agent, "claude");
    assert_eq!(all("登入").hits[0].id, CL);
    assert_eq!(all("cookie HttpOnly").hits[0].role, "assistant");
    assert!(all("cookie 不存在的詞").hits.is_empty());
    // 工具摘要也搜得到（名稱＋一行）
    assert_eq!(all("cargo test -q").hits[0].role, "tool");
    // codex exec 的不收
    assert!(all("secret").hits.is_empty());
    // 範圍：只看 repo-b＝沒有
    let only_b = vec!["repo-b".to_string()];
    assert!(index::search(&d, &src, None, Some(&only_b), "索引", 10)
        .unwrap()
        .hits
        .is_empty());

    // 秘密：庫裡與片段都沒有原文
    let f = all("token");
    assert!(
        !f.hits[0].snippet.contains(FAKE_TOKEN),
        "{}",
        f.hits[0].snippet
    );
    assert!(
        f.hits[0].snippet.contains("[REDACTED TOKEN]"),
        "{}",
        f.hits[0].snippet
    );
    assert!(all("ghp_abcdefghij").hits.is_empty());
    assert!(db_texts(&d).iter().all(|x| !x.contains(FAKE_TOKEN)));

    // 沒變動＝不寫
    let g = index::generation(&d).unwrap();
    let f = all("索引");
    assert_eq!((f.index.read, f.index.appended), (0, 0));
    assert_eq!(index::generation(&d), Some(g));

    // 增量：附加幾行＝只讀新增的部分，舊訊息不重複
    let before = f.index.messages;
    append_lines(
        &cl,
        &[
            json!({"type":"user","timestamp":"2026-11-15T02:00:00Z","message":{"content":"再加上 CSRF 防護"}}),
            json!({"type":"assistant","message":{"content":[{"type":"text","text":"CSRF token 加好了"}]}}),
        ],
    );
    let f = all("CSRF");
    assert_eq!((f.index.read, f.index.appended), (0, 1), "{:?}", f.index);
    assert_eq!(f.index.added, 2);
    assert_eq!(f.index.messages, before + 2);
    assert_eq!(f.hits[0].hits, 2);
    assert_eq!(all("cookie").hits[0].hits, 2, "舊訊息不能重複");
    // 還沒寫完的最後一行（沒換行）先不讀，寫完再讀
    {
        let mut fh = std::fs::OpenOptions::new().append(true).open(&cl).unwrap();
        write!(
            fh,
            "{}",
            json!({"type":"user","message":{"content":"半行的 XYZZY"}})
        )
        .unwrap();
    }
    assert!(all("XYZZY").hits.is_empty());
    {
        let mut fh = std::fs::OpenOptions::new().append(true).open(&cl).unwrap();
        writeln!(fh).unwrap();
    }
    assert_eq!(all("XYZZY").hits.len(), 1);
    assert_eq!(all("cookie").hits[0].hits, 2);
}

#[test]
fn session_search_rebuilds_a_broken_db_and_follows_registry_changes() {
    let (t, d) = spine_with_repos();
    let ra = registry::get_repo(&d, "repo-a").unwrap().path();
    let root = t.path().join("claude");
    claude_root_with(&root, &ra);
    let src = Sources::new(Some(&root), None);
    assert_eq!(
        index::search(&d, &src, None, None, "cookie", 5)
            .unwrap()
            .hits
            .len(),
        1
    );
    // 損毀：整個檔換成垃圾 → 刪掉重建
    let db = index::db_path(&d);
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    std::fs::write(
        &db,
        b"this is not a sqlite database at all, just junk bytes....",
    )
    .unwrap();
    let f = index::search(&d, &src, None, None, "cookie", 5).unwrap();
    assert_eq!(f.hits.len(), 1);
    assert_eq!(f.index.read, 1);
    // 表被刪掉也一樣
    rusqlite_open(&d).execute_batch("DROP TABLE meta;").unwrap();
    assert_eq!(
        index::search(&d, &src, None, None, "cookie", 5)
            .unwrap()
            .hits
            .len(),
        1
    );
    // 空查詢＝錯
    assert!(index::search(&d, &src, None, None, "   ", 5).is_err());
    // repo 移出登記：整場拿掉
    registry::remove_repo(&d, "repo-a").unwrap();
    let f = index::search(&d, &src, None, None, "cookie", 5).unwrap();
    assert!(f.hits.is_empty());
    assert_eq!(f.index.sessions, 0);
}

#[test]
fn reading_a_big_transcript_streams_line_by_line() {
    let t = tempfile::tempdir().unwrap();
    let cwd = t.path().join("repo");
    let root = t.path().join("claude");
    let p = root
        .join(convo::claude::project_dir_name(&cwd))
        .join(format!("{CL}.jsonl"));
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let mut f = std::io::BufWriter::new(std::fs::File::create(&p).unwrap());
    let filler = "很長的工具輸出 ".repeat(150); // 每行約 3 KB
    let mut n = 0u64;
    while n < 32 * 1024 * 1024 {
        let row = json!({"type":"user","cwd":cwd,"message":{"content":[
            {"type":"tool_result","tool_use_id":"t","content":filler}]}})
        .to_string();
        writeln!(f, "{row}").unwrap();
        n += row.len() as u64 + 1;
    }
    writeln!(
        f,
        "{}",
        json!({"type":"assistant","message":{"content":[{"type":"text","text":"最後一句"}]}})
    )
    .unwrap();
    drop(f);
    let sc = convo::claude::ClaudeScanner::new(root);
    let mut items = convo::Items::open(&sc, &p, Start::Head).unwrap();
    let mut results = 0usize;
    let mut last = None;
    for it in items.by_ref() {
        match it {
            Item::ToolResult { .. } => results += 1,
            Item::Message(m) => last = Some(m.text),
            Item::Title(_) => {}
        }
    }
    assert!(results > 5_000, "{results}");
    assert_eq!(last.as_deref(), Some("最後一句"));
    // 一次只握一行（≈3 KB），不是整份 32 MB
    assert!(
        items.lines.peak_line() < 16 * 1024,
        "{}",
        items.lines.peak_line()
    );
    assert!(items.lines.pos() > 32 * 1024 * 1024);
}
