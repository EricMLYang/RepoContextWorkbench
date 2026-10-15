//! 過去的 agent 會話（F-72）：範圍內 repo 的 Claude Code 逐字稿、排除 `claude -p`、接續目標要在登記的 repo 底下。

use std::path::Path;

use ctx_core::history::{past_sessions, project_dir_name, resume_target};
use ctx_core::registry;
use ctx_core::testkit::spine_with_repos;

fn transcript(root: &Path, cwd: &Path, id: &str, entry: &str, prompt: &str) {
    let dir = root.join(project_dir_name(cwd));
    std::fs::create_dir_all(&dir).unwrap();
    let rows = [
        serde_json::json!({"type":"user","entrypoint":entry,"cwd":cwd,"timestamp":"2026-10-16T01:00:00Z",
                           "gitBranch":"main","message":{"content":prompt}}),
        serde_json::json!({"type":"ai-title","aiTitle":format!("標題 {id}")}),
        serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"做好了"}]}}),
    ];
    let text: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
    std::fs::write(dir.join(format!("{id}.jsonl")), text.join("\n") + "\n").unwrap();
}

#[test]
fn lists_interactive_sessions_of_the_scope_newest_first() {
    let (t, d) = spine_with_repos();
    let root = t.path().join("claude-projects");
    let ra = registry::get_repo(&d, "repo-a").unwrap().path();
    let rb = registry::get_repo(&d, "repo-b").unwrap().path();
    transcript(&root, &ra, "aaaaaaaa-0001", "cli", "修登入");
    std::thread::sleep(std::time::Duration::from_millis(20));
    transcript(
        &root,
        &ra.join("src"),
        "aaaaaaaa-0002",
        "cli",
        "子目錄裡開的",
    );
    transcript(&root, &ra, "aaaaaaaa-0003", "sdk-cli", "助手的 -p 呼叫");
    transcript(&root, &rb, "bbbbbbbb-0001", "cli", "repo-b 的");
    // 資料夾名撞到前綴、其實不在 repo 底下
    let other = ra.with_file_name("repo-a-old");
    transcript(&root, &other, "cccccccc-0001", "cli", "別的資料夾");

    let only_a = vec!["repo-a".to_string()];
    let got = past_sessions(&d, None, Some(&only_a), 10, Some(&root)).unwrap();
    let ids: Vec<&str> = got.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["aaaaaaaa-0002", "aaaaaaaa-0001"]);
    assert_eq!(got[1].title, "標題 aaaaaaaa-0001");
    assert_eq!(got[1].prompt.as_deref(), Some("修登入"));
    assert_eq!(got[1].reply.as_deref(), Some("做好了"));
    assert_eq!(got[1].repo, "repo-a");
    assert!(got[1].live);

    let group = past_sessions(&d, Some("g1"), None, 10, Some(&root)).unwrap();
    assert_eq!(group.len(), 3);
    assert_eq!(
        past_sessions(&d, Some("g1"), None, 1, Some(&root))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn resume_target_must_be_a_real_dir_inside_a_registered_repo() {
    let (t, d) = spine_with_repos();
    let ra = registry::get_repo(&d, "repo-a").unwrap().path();
    let (rid, cwd) = resume_target(&d, "aaaaaaaa-0001", ra.to_str().unwrap()).unwrap();
    assert_eq!((rid.as_str(), cwd.as_path()), ("repo-a", ra.as_path()));
    assert!(resume_target(&d, "bad id; rm", ra.to_str().unwrap()).is_err());
    assert!(resume_target(&d, "aaaaaaaa-0001", t.path().to_str().unwrap()).is_err());
    assert!(resume_target(&d, "aaaaaaaa-0001", "/no/such/dir").is_err());
}
