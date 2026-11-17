//! 會話逐字稿時間軸（2026-10-21，ideation F-94；Rust 才有，抄 BAT `claude-cli-frames.mjs` 的分類）。
//!
//! 終端的捲動緩衝只有 5,000 行，而且 App 重開就沒了；Claude Code 自己寫的逐字稿
//! （`~/.claude/projects/<編碼後的 cwd>/<id>.jsonl`）才是完整紀錄。這裡把它分成五種框：
//! 你說的（`you`）、它的回覆（`reply`）、思考（`thinking`）、工具（`tool`，配上結果）、
//! 壓縮摘要（`compact`，context 滿了之後接續的地方）。harness 自己塞的列（背景任務通知、斜線指令、中斷標記）不列。
//!
//! 只讀不寫。一份逐字稿可以到幾十 MB：只讀最後 [`READ_BYTES`]、最多回 [`MAX_FRAMES`] 框，文字各自截短；
//! 有截掉就標 `truncated`。

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::config::AgentKind;
use crate::convo::{self, Item, Items, Role, Scanner, Start, ToolAction};
use crate::transcript::one_line;

/// 最多讀檔尾多少位元組。
const READ_BYTES: u64 = 8 * 1024 * 1024;
/// 最多回幾框（取最後的）。
const MAX_FRAMES: usize = 800;
/// 一框文字最多幾個字元。
const TEXT_MAX: usize = 6_000;
/// 工具結果預覽最多幾個字元。
const RESULT_MAX: usize = 800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FrameKind {
    You,
    Reply,
    Thinking,
    Tool,
    Compact,
}

/// 時間軸上的一框。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Frame {
    pub kind: FrameKind,
    /// 你說的／回覆／思考的文字；工具是「做什麼」（指令、路徑…）。
    pub text: String,
    /// 逐字稿的時間（UTC ISO）。
    pub at: Option<String>,
    /// 工具名（只有 `tool`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// 工具結果預覽（只有 `tool`，結果回來了才有）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// 工具結果是錯誤。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub error: bool,
    /// 工具做的是哪一類事（交接包用；不輸出，畫面看 `tool`）。
    #[serde(skip)]
    pub action: ToolAction,
}

/// 整份時間軸。
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Timeline {
    pub frames: Vec<Frame>,
    /// 檔案太大只讀了尾巴，或框數超過上限只留最後的。
    pub truncated: bool,
    /// Claude Code 取的標題（最後一筆 `ai-title`）。
    pub title: Option<String>,
    pub path: String,
    /// 哪一家的逐字稿：`claude`｜`codex`（F-134）。
    pub agent: &'static str,
}

impl Frame {
    fn new(kind: FrameKind, text: &str, at: Option<String>) -> Self {
        Frame {
            kind,
            text: crate::pystr::clip(text.trim(), TEXT_MAX),
            at,
            tool: None,
            result: None,
            error: false,
            action: ToolAction::Other,
        }
    }
}

/// 事件流 → 框（工具結果配回它的工具框）。回傳 (框, 標題)。
pub fn frames_from(items: impl Iterator<Item = Item>) -> (Vec<Frame>, Option<String>) {
    let (msgs, title) = convo::pair(items);
    let frames = msgs
        .into_iter()
        .map(|p| {
            let m = p.message;
            let kind = match m.role {
                Role::User => FrameKind::You,
                Role::Assistant => FrameKind::Reply,
                Role::Thinking => FrameKind::Thinking,
                Role::Tool => FrameKind::Tool,
                Role::Compact => FrameKind::Compact,
            };
            let mut f = Frame::new(kind, &m.text, m.at);
            if let Some(t) = m.tool {
                f.tool = Some(t.name);
                f.action = t.action;
                f.result = p.result.map(|r| crate::pystr::clip(r.trim(), RESULT_MAX));
                f.error = p.error;
            }
            f
        })
        .collect();
    (frames, title)
}

/// Claude Code 逐字稿各列 → 框（F-94 原本的入口；2026-11-18 起經過 [`convo::claude::row_items`]）。
pub fn frames_of(rows: &[Value]) -> (Vec<Frame>, Option<String>) {
    let mut items = Vec::new();
    for o in rows {
        convo::claude::row_items(o, &mut items);
    }
    frames_from(items.into_iter())
}

/// 讀一份 Claude Code 逐字稿的時間軸。檔案不在或讀不了＝None。
pub fn read(path: &Path) -> Option<Timeline> {
    read_with(
        &convo::claude::ClaudeScanner::new(crate::backfill::projects_root()),
        path,
    )
}

/// 用那一家的掃描器讀時間軸（F-134：Codex 也有）。只讀檔尾 [`READ_BYTES`]，一行一行串流。
pub fn read_with(scanner: &dyn Scanner, path: &Path) -> Option<Timeline> {
    let mut items = Items::open(scanner, path, Start::Tail(READ_BYTES)).ok()?;
    let mut all = Vec::new();
    for it in items.by_ref() {
        all.push(it);
    }
    let cut = items.lines.cut;
    let (mut frames, title) = frames_from(all.into_iter());
    // Codex 的標題不在逐字稿裡（`session_index.jsonl`）
    let title = title.or_else(|| scanner.probe(path).and_then(|s| s.title));
    let over = frames.len() > MAX_FRAMES;
    if over {
        frames.drain(..frames.len() - MAX_FRAMES);
    }
    Some(Timeline {
        frames,
        truncated: cut || over,
        title,
        path: path.display().to_string(),
        agent: scanner.kind().name(),
    })
}

/// 依會話 id 找 Claude Code 逐字稿：先看 cwd 對應的專案資料夾，找不到再掃全部專案資料夾（cwd 是子目錄或搬過時）。
/// `root` 預設 [`crate::backfill::projects_root`]。id 不合格＝None（不拼進路徑）。
pub fn locate(id: &str, cwd: Option<&str>, root: Option<&Path>) -> Option<PathBuf> {
    let root = root.map_or_else(crate::backfill::projects_root, Path::to_path_buf);
    convo::claude::ClaudeScanner::new(root).locate(id, cwd)
}

/// 依會話 id 找到並讀出時間軸（F-134：兩家都找；`agent` 給了只找那一家，否則 Claude Code 先）。
pub fn find(id: &str, cwd: Option<&str>, agent: Option<AgentKind>) -> crate::Result<Timeline> {
    find_in(&convo::Sources::default_homes(), id, cwd, agent)
}

/// 同 [`find`]，掃描器由呼叫端給（測試用暫存目錄）。
pub fn find_in(
    src: &convo::Sources,
    id: &str,
    cwd: Option<&str>,
    agent: Option<AgentKind>,
) -> crate::Result<Timeline> {
    let (kind, path) = src
        .locate(id, cwd, agent)
        .ok_or_else(|| crate::Error::invalid(format!("找不到這場會話的逐字稿：{id}")))?;
    let sc = src
        .get(kind)
        .ok_or_else(|| crate::Error::invalid(format!("找不到這場會話的逐字稿：{id}")))?;
    read_with(sc, &path)
        .ok_or_else(|| crate::Error::invalid(format!("讀不了逐字稿：{}", path.display())))
}

/// 交接包（F-103，抄 BAT `transcript-transfer`）：把一場會話濃縮成新會話讀得完的一頁——
/// 你說過的話、動過的檔、跑過的指令、最後的回覆。思考與工具結果不帶（太長、新會話自己會再看）。
/// 秘密（金鑰、token）先遮掉；回傳 (Markdown, 遮了幾處)。
pub fn digest(tl: &Timeline, id: &str, cwd: Option<&str>, now: &str) -> (String, usize) {
    let mut out = Vec::new();
    out.push(format!(
        "# 交接包：{}",
        tl.title.as_deref().unwrap_or("（沒有標題的會話）")
    ));
    out.push(String::new());
    out.push(format!(
        "- 來源：{} 會話 `{id}`{}",
        AgentKind::parse(tl.agent)
            .unwrap_or(AgentKind::ClaudeCode)
            .label(),
        cwd.map(|c| format!("（{c}）")).unwrap_or_default()
    ));
    out.push(format!("- 整理：{now}（ctx 自動整理，秘密已遮蔽）"));
    if tl.truncated {
        out.push("- 注意：這場會話很長，只整理了最後一段".to_string());
    }
    out.push(format!("- 完整逐字稿：{}", tl.path));
    out.push(String::new());
    out.push("## 你說過的話（依序）".to_string());
    let said: Vec<&Frame> = tl
        .frames
        .iter()
        .filter(|f| f.kind == FrameKind::You)
        .collect();
    if said.is_empty() {
        out.push("（沒有）".to_string());
    }
    for (i, f) in said.iter().enumerate() {
        out.push(format!("{}. {}", i + 1, one_line(&f.text, 400)));
    }
    // 動過的檔：Edit／Write／NotebookEdit 的路徑，依第一次出現排序、記次數
    let mut files: Vec<(String, usize)> = Vec::new();
    let mut cmds: Vec<String> = Vec::new();
    for f in tl.frames.iter().filter(|f| f.kind == FrameKind::Tool) {
        match f.action {
            ToolAction::Edit if !f.text.is_empty() => {
                match files.iter_mut().find(|(p, _)| *p == f.text) {
                    Some((_, n)) => *n += 1,
                    None => files.push((f.text.clone(), 1)),
                }
            }
            ToolAction::Shell if !f.text.is_empty() => cmds.push(format!(
                "- `{}`{}",
                one_line(&f.text, 160),
                if f.error { " ✗" } else { "" }
            )),
            _ => {}
        }
    }
    out.push(String::new());
    out.push("## 動過的檔".to_string());
    if files.is_empty() {
        out.push("（沒有）".to_string());
    }
    for (p, n) in &files {
        out.push(if *n > 1 {
            format!("- {p}（{n} 次）")
        } else {
            format!("- {p}")
        });
    }
    out.push(String::new());
    out.push("## 跑過的指令（最後 15 個，✗＝失敗）".to_string());
    if cmds.is_empty() {
        out.push("（沒有）".to_string());
    }
    let skip = cmds.len().saturating_sub(15);
    out.extend(cmds.into_iter().skip(skip));
    out.push(String::new());
    out.push("## 最後的回覆".to_string());
    let replies: Vec<&Frame> = tl
        .frames
        .iter()
        .filter(|f| f.kind == FrameKind::Reply)
        .collect();
    match replies.last() {
        Some(f) => out.push(crate::pystr::clip(&f.text, 3_000)),
        None => out.push("（沒有）".to_string()),
    }
    out.push(String::new());
    crate::redact::redact_secrets(&out.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &[&str]) -> Vec<Value> {
        lines
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn frames_classify_you_reply_thinking_tool_and_pair_results() {
        let (f, title) = frames_of(&rows(&[
            r#"{"type":"user","message":{"content":"幫我跑測試"},"timestamp":"2026-10-21T01:00:00Z"}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"<local-command-caveat>x</local-command-caveat>"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"先看看"},{"type":"text","text":"好"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test","description":"跑測試"}}]},"timestamp":"2026-10-21T01:00:05Z"}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"test result: ok","is_error":false}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/r/a.rs"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"沒有這個檔"}],"is_error":true}]}}"#,
            r#"{"type":"ai-title","aiTitle":"跑測試"}"#,
            r#"{"type":"user","isCompactSummary":true,"message":{"content":"This session is being continued from a previous conversation."}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"看這張圖"},{"type":"image"}]}}"#,
        ]));
        let kinds: Vec<FrameKind> = f.iter().map(|x| x.kind).collect();
        assert_eq!(
            kinds,
            [
                FrameKind::You,
                FrameKind::Thinking,
                FrameKind::Reply,
                FrameKind::Tool,
                FrameKind::Tool,
                FrameKind::Compact,
                FrameKind::You,
            ]
        );
        assert_eq!(f[0].text, "幫我跑測試");
        assert_eq!(f[0].at.as_deref(), Some("2026-10-21T01:00:00Z"));
        assert_eq!(f[3].tool.as_deref(), Some("Bash"));
        assert_eq!(f[3].text, "cargo test");
        assert_eq!(f[3].result.as_deref(), Some("test result: ok"));
        assert!(!f[3].error);
        assert_eq!(f[4].result.as_deref(), Some("沒有這個檔"));
        assert!(f[4].error);
        assert_eq!(f[6].text, "看這張圖");
        assert_eq!(title.as_deref(), Some("跑測試"));
    }

    #[test]
    fn digest_lists_prompts_files_commands_and_last_reply_with_secrets_masked() {
        let (frames, title) = frames_of(&rows(&[
            r#"{"type":"user","message":{"content":"修登入，token 是 ghp_abcdefghijklmnopqrstuvwxyz0123456789"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"a","name":"Edit","input":{"file_path":"/r/login.rs"}},{"type":"tool_use","id":"b","name":"Edit","input":{"file_path":"/r/login.rs"}},{"type":"tool_use","id":"c","name":"Bash","input":{"command":"cargo test"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"c","content":"fail","is_error":true}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"秘密的想法"},{"type":"text","text":"改好了"}]}}"#,
            r#"{"type":"ai-title","aiTitle":"修登入"}"#,
        ]));
        let tl = Timeline {
            frames,
            truncated: false,
            title,
            path: "/p.jsonl".into(),
            agent: "claude",
        };
        let (md, hidden) = digest(&tl, "abc", Some("/r"), "2026-10-22 00:10");
        assert!(md.starts_with("# 交接包：修登入"), "{md}");
        assert!(md.contains("1. 修登入，token 是"), "{md}");
        assert!(!md.contains("ghp_abcdefghij"), "{md}");
        assert!(hidden >= 1);
        assert!(md.contains("- /r/login.rs（2 次）"), "{md}");
        assert!(md.contains("- `cargo test` ✗"), "{md}");
        assert!(md.contains("## 最後的回覆\n改好了"), "{md}");
        assert!(!md.contains("秘密的想法"));
    }

    #[test]
    fn read_keeps_the_last_frames_and_locate_finds_the_file() {
        let t = tempfile::tempdir().unwrap();
        let proj = t
            .path()
            .join(crate::history::project_dir_name(Path::new("/r/app")));
        std::fs::create_dir_all(&proj).unwrap();
        let id = "02908d1e-7843-436e-9629-55eaa75fdb85";
        let mut body = String::new();
        for i in 0..(MAX_FRAMES + 5) {
            body.push_str(&format!(
                "{{\"type\":\"user\",\"message\":{{\"content\":\"第 {i} 句\"}}}}\n"
            ));
        }
        std::fs::write(proj.join(format!("{id}.jsonl")), body).unwrap();
        let p = locate(id, Some("/r/app"), Some(t.path())).unwrap();
        // cwd 對不上（子目錄、搬過）就掃全部專案資料夾
        assert_eq!(
            locate(id, Some("/elsewhere"), Some(t.path())),
            Some(p.clone())
        );
        assert_eq!(locate("../etc/passwd", None, Some(t.path())), None);
        assert_eq!(locate("ffffffff-0000", None, Some(t.path())), None);
        let tl = read(&p).unwrap();
        assert_eq!(tl.frames.len(), MAX_FRAMES);
        assert!(tl.truncated);
        assert_eq!(
            tl.frames.last().unwrap().text,
            format!("第 {} 句", MAX_FRAMES + 4)
        );
        assert_eq!(read(&t.path().join("none.jsonl")), None);
    }

    /// F-134 之前的時間軸分類（原樣留著當規格）：新路徑（經過 `convo::claude`）對同一份逐字稿要輸出一樣。
    mod legacy {
        use super::super::*;
        use crate::transcript::{is_compact_summary, is_harness_noise};
        use std::collections::HashMap;

        /// 工具輸入裡人看得懂的那個欄位（跟會話狀態的工具標籤同一組鍵）。
        fn tool_detail(input: Option<&Value>) -> String {
            const KEYS: [&str; 9] = [
                "command",
                "file_path",
                "path",
                "notebook_path",
                "url",
                "pattern",
                "description",
                "prompt",
                "query",
            ];
            let Some(obj) = input.and_then(Value::as_object) else {
                return String::new();
            };
            KEYS.iter()
                .find_map(|k| obj.get(*k).and_then(Value::as_str))
                .map(|v| one_line(v, 300))
                .unwrap_or_default()
        }

        /// 工具結果的文字：字串，或區塊陣列裡的文字（圖片等略過）。
        fn result_text(content: Option<&Value>) -> String {
            match content {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(blocks)) => blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            }
        }

        /// 把逐字稿各列分成框。工具結果配回它的工具框（`tool_use_id`）。
        pub fn frames_of(rows: &[Value]) -> (Vec<Frame>, Option<String>) {
            let mut out: Vec<Frame> = Vec::new();
            let mut tool_at: HashMap<String, usize> = HashMap::new();
            let mut title = None;
            for o in rows {
                let at = o
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let content = o.get("message").and_then(|m| m.get("content"));
                match o.get("type").and_then(Value::as_str) {
                    Some("ai-title") => {
                        if let Some(t) = o.get("aiTitle").and_then(Value::as_str) {
                            title = Some(t.to_string());
                        }
                    }
                    Some("user") => match content {
                        Some(Value::String(s)) => {
                            let t = s.trim();
                            if t.is_empty() {
                                continue;
                            }
                            if is_compact_summary(o, t) {
                                out.push(Frame::new(FrameKind::Compact, t, at));
                            } else if !is_harness_noise(o, t) {
                                out.push(Frame::new(FrameKind::You, t, at));
                            }
                        }
                        Some(Value::Array(blocks)) => {
                            let mut said = Vec::new();
                            for b in blocks {
                                match b.get("type").and_then(Value::as_str) {
                                    Some("tool_result") => {
                                        let id = b.get("tool_use_id").and_then(Value::as_str);
                                        if let Some(&i) = id.and_then(|id| tool_at.get(id)) {
                                            let text = result_text(b.get("content"));
                                            out[i].result =
                                                Some(crate::pystr::clip(text.trim(), RESULT_MAX));
                                            out[i].error =
                                                b.get("is_error") == Some(&Value::Bool(true));
                                        }
                                    }
                                    Some("text") => {
                                        if let Some(t) = b.get("text").and_then(Value::as_str) {
                                            said.push(t.trim());
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            let text = said.join("\n");
                            if !text.is_empty() && !is_harness_noise(o, &text) {
                                out.push(Frame::new(FrameKind::You, &text, at));
                            }
                        }
                        _ => {}
                    },
                    Some("assistant") => {
                        let Some(blocks) = content.and_then(Value::as_array) else {
                            continue;
                        };
                        for b in blocks {
                            match b.get("type").and_then(Value::as_str) {
                                Some("text") => {
                                    let t = b.get("text").and_then(Value::as_str).unwrap_or("");
                                    if !t.trim().is_empty() {
                                        out.push(Frame::new(FrameKind::Reply, t, at.clone()));
                                    }
                                }
                                Some("thinking") => {
                                    let t = b.get("thinking").and_then(Value::as_str).unwrap_or("");
                                    if !t.trim().is_empty() {
                                        out.push(Frame::new(FrameKind::Thinking, t, at.clone()));
                                    }
                                }
                                Some("tool_use") => {
                                    let name =
                                        b.get("name").and_then(Value::as_str).unwrap_or("工具");
                                    let mut f = Frame::new(
                                        FrameKind::Tool,
                                        &tool_detail(b.get("input")),
                                        at.clone(),
                                    );
                                    f.tool = Some(name.to_string());
                                    if let Some(id) = b.get("id").and_then(Value::as_str) {
                                        tool_at.insert(id.to_string(), out.len());
                                    }
                                    out.push(f);
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
            (out, title)
        }
    }

    #[test]
    fn new_scanner_path_matches_the_legacy_frames_exactly() {
        let fixture = rows(&[
            r#"{"type":"user","message":{"content":"幫我跑測試"},"timestamp":"2026-10-21T01:00:00Z"}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"<local-command-caveat>x</local-command-caveat>"}}"#,
            r#"{"type":"user","promptSource":"system","origin":{"kind":"task-notification"},"message":{"content":"背景任務完成"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"先看看"},{"type":"text","text":"好"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test","description":"跑測試"}}]},"timestamp":"2026-10-21T01:00:05Z"}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"test result: ok","is_error":false}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/r/a.rs"}},{"type":"tool_use","id":"t3","name":"WebFetch","input":{"url":"https://example.com","prompt":"x"}},{"type":"tool_use","id":"t4","name":"Task","input":{}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"沒有這個檔"},{"type":"image"}],"is_error":true},{"type":"tool_result","tool_use_id":"nope","content":"孤兒"}]}}"#,
            r#"{"type":"ai-title","aiTitle":"跑測試"}"#,
            r#"{"type":"user","isCompactSummary":true,"message":{"content":"This session is being continued from a previous conversation."}}"#,
            r#"{"type":"user","message":{"content":[{"type":"text","text":"看這張圖"},{"type":"image"}]}}"#,
            r#"{"type":"user","message":{"content":"   "}}"#,
            r#"{"type":"user","message":{"content":"[Request interrupted by user]"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"   "},{"type":"text","text":"很長的回覆"}]}}"#,
            r#"{"type":"ai-title","aiTitle":"最後的標題"}"#,
            r#"{"type":"system","subtype":"x"}"#,
        ]);
        let (old, old_title) = legacy::frames_of(&fixture);
        let (new, new_title) = frames_of(&fixture);
        assert_eq!(old_title, new_title);
        assert_eq!(
            serde_json::to_value(&old).unwrap(),
            serde_json::to_value(&new).unwrap()
        );
        assert_eq!(new.len(), 10);
    }
}
