//! Codex 過去的會話（2026-11-14，Rust 才有）：讀 Codex CLI 自己寫的會話紀錄，跟 Claude Code 的逐字稿
//! 列在同一份「過去的會話」裡（[`crate::history`]），一鍵 `codex resume <id>` 接續。
//!
//! 紀錄在 `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<時間>-<id>.jsonl`（預設 `~/.codex`），不像 Claude Code
//! 依專案分資料夾，所以要看每份的第一行才知道是哪個目錄的會話：
//! - 第一行 `session_meta`：`id`、`cwd`、`timestamp`、`source`（`exec`＝`codex exec` 程式化呼叫、物件＝子 agent，
//!   都不是人談過的會話，不列）、`git.branch`。這一行含整份系統指示，實測約 20 KB，所以整行讀、上限 [`META_MAX`]。
//! - 第一句話：第一則 `response_item`／`message`／`role: user` 裡不是 Codex 自己塞的那些
//!   （`<environment_context>`、`# AGENTS.md instructions…`）。
//! - 最後回覆：尾巴最後一筆 `event_msg`／`task_complete` 的 `last_agent_message`。
//! - 標題：`$CODEX_HOME/session_index.jsonl`（`id` → `thread_name`）；沒有就用第一句話。
//!
//! 只看最近 [`MAX_FILES`] 份（檔名帶時間，新的在前）：不在範圍內的只讀第一行就跳過。
//! 格式是照 Codex 0.166–0.169 的實際紀錄寫的，不是公開合約——讀不懂的檔、欄位一律略過，不報錯。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::transcript::one_line;

/// 第一行（session_meta）最多讀多少。
const META_MAX: u64 = 1024 * 1024;
/// 找第一句話最多往下讀多少（開頭是系統指示、工具清單、AGENTS.md，實測幾十 KB）。
const HEAD_MAX: u64 = 1024 * 1024;
/// 找最後回覆讀尾巴多少。
const TAIL_MAX: u64 = 256 * 1024;
/// 最多看幾份紀錄（最近的）。
pub const MAX_FILES: usize = 400;

/// 一份 Codex 會話紀錄裡，「過去的會話」要的欄位。
#[derive(Debug, Clone, PartialEq)]
pub struct CodexSession {
    pub id: String,
    pub cwd: String,
    pub title: Option<String>,
    pub prompt: String,
    pub reply: Option<String>,
    pub branch: Option<String>,
    pub started: Option<String>,
    pub mtime: SystemTime,
    pub path: PathBuf,
}

/// `$CODEX_HOME`（沒設＝`~/.codex`）。
pub fn codex_home() -> PathBuf {
    crate::agentsetup::codex_home()
}

/// `root` 底下最近的紀錄檔，新的在前，最多 [`MAX_FILES`] 份。目錄與檔名都帶時間，照名字倒著排就是時間序。
fn recent_rollouts(sessions: &Path) -> Vec<PathBuf> {
    fn sorted_desc(dir: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .collect();
        v.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        v
    }
    let mut out = Vec::new();
    for year in sorted_desc(sessions) {
        for month in sorted_desc(&year) {
            for day in sorted_desc(&month) {
                for f in sorted_desc(&day) {
                    let name = f.file_name().unwrap_or_default().to_string_lossy();
                    if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                        out.push(f.clone());
                        if out.len() >= MAX_FILES {
                            return out;
                        }
                    }
                }
            }
        }
    }
    out
}

/// 一則 user 訊息的文字是不是 Codex 自己塞的（環境說明、AGENTS.md…），不是人說的話。
fn injected(text: &str) -> bool {
    let t = text.trim_start();
    t.is_empty() || t.starts_with('<') || t.starts_with("# AGENTS.md instructions")
}

fn message_text(payload: &Value, kind: &str) -> Option<String> {
    payload["content"].as_array()?.iter().find_map(|c| {
        (c["type"] == kind)
            .then(|| c["text"].as_str())
            .flatten()
            .map(str::to_string)
    })
}

/// 第一行（session_meta）→ (id, cwd, 開始時間, 分支)；程式化呼叫與子 agent＝None。
fn read_meta(line: &str) -> Option<(String, String, Option<String>, Option<String>)> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v["type"] != "session_meta" {
        return None;
    }
    let p = &v["payload"];
    // source：`cli`／`vscode`…＝互動；`exec`＝codex exec；物件（{"subagent": …}）＝子 agent
    if p["source"].is_object() || p["source"] == "exec" {
        return None;
    }
    let id = p["id"]
        .as_str()
        .filter(|i| crate::history::valid_session_id(i))?;
    let cwd = p["cwd"].as_str().filter(|c| !c.is_empty())?;
    Some((
        id.to_string(),
        cwd.to_string(),
        p["timestamp"].as_str().map(str::to_string),
        p["git"]["branch"].as_str().map(str::to_string),
    ))
}

/// 尾巴最後一則回覆。
fn last_reply(path: &Path, len: u64) -> Option<String> {
    let mut f = std::fs::File::open(path).ok()?;
    let start = len.saturating_sub(TAIL_MAX);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.take(TAIL_MAX).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
    if start > 0 {
        lines.next(); // 第一行可能被切一半
    }
    let mut done = None;
    let mut said = None;
    for l in lines {
        let Ok(v) = serde_json::from_str::<Value>(l) else {
            continue;
        };
        let p = &v["payload"];
        if v["type"] == "event_msg" && p["type"] == "task_complete" {
            if let Some(m) = p["last_agent_message"]
                .as_str()
                .filter(|m| !m.trim().is_empty())
            {
                done = Some(m.to_string());
            }
        } else if v["type"] == "response_item" && p["type"] == "message" && p["role"] == "assistant"
        {
            if let Some(m) = message_text(p, "output_text").filter(|m| !m.trim().is_empty()) {
                said = Some(m);
            }
        }
    }
    done.or(said)
}

/// `session_index.jsonl`：id → Codex 取的標題（同一個 id 後面的蓋前面的）。
fn thread_names(home: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Ok(text) = std::fs::read_to_string(home.join("session_index.jsonl")) else {
        return out;
    };
    for l in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(l) else {
            continue;
        };
        if let (Some(id), Some(name)) = (v["id"].as_str(), v["thread_name"].as_str()) {
            if !name.trim().is_empty() {
                out.insert(id.to_string(), name.trim().to_string());
            }
        }
    }
    out
}

/// 讀一份紀錄；`wanted`＝這個 cwd 要不要（不要的只讀第一行）。不是互動會話、沒人說過話＝None。
fn read_one(path: &Path, wanted: &dyn Fn(&str) -> bool) -> Option<CodexSession> {
    let meta = std::fs::metadata(path).ok()?;
    let mut rd = BufReader::new(std::fs::File::open(path).ok()?.take(META_MAX + HEAD_MAX));
    let mut first = String::new();
    rd.read_line(&mut first).ok()?;
    let (id, cwd, started, branch) = read_meta(&first)?;
    if !wanted(&cwd) {
        return None;
    }
    let mut prompt = None;
    let mut line = String::new();
    loop {
        line.clear();
        match rd.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        // 便宜的預篩：不是 user 訊息就不解析（開頭幾行是幾十 KB 的系統指示）
        if !line.contains("\"role\":\"user\"") && !line.contains("\"role\": \"user\"") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let p = &v["payload"];
        if v["type"] == "response_item" && p["type"] == "message" && p["role"] == "user" {
            if let Some(t) = message_text(p, "input_text").filter(|t| !injected(t)) {
                prompt = Some(t);
                break;
            }
        }
    }
    Some(CodexSession {
        id,
        cwd,
        title: None,
        prompt: one_line(&prompt?, 120),
        reply: last_reply(path, meta.len()).map(|r| one_line(&r, 160)),
        branch,
        started,
        mtime: meta.modified().unwrap_or_else(|_| SystemTime::now()),
        path: path.to_path_buf(),
    })
}

/// `home`（`$CODEX_HOME`）底下、cwd 合 `wanted` 的互動會話，新的在前。沒裝 Codex、沒有紀錄＝空的。
pub fn sessions_in(home: &Path, wanted: &dyn Fn(&str) -> bool) -> Vec<CodexSession> {
    let files = recent_rollouts(&home.join("sessions"));
    if files.is_empty() {
        return Vec::new();
    }
    let names = thread_names(home);
    files
        .iter()
        .filter_map(|f| read_one(f, wanted))
        .map(|mut s| {
            s.title = names.get(&s.id).cloned();
            s
        })
        .collect()
}

/// 這個 id 的紀錄檔（檔名尾巴就是 id）。`ctx` 要讀逐字稿、確認會話還在時用。
pub fn rollout_of(home: &Path, id: &str) -> Option<PathBuf> {
    if !crate::history::valid_session_id(id) {
        return None;
    }
    let tail = format!("-{id}.jsonl");
    recent_rollouts(&home.join("sessions"))
        .into_iter()
        .find(|f| {
            f.file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(&tail))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_rollout(home: &Path, day: &str, stamp: &str, id: &str, lines: &[Value]) -> PathBuf {
        let dir = home.join("sessions").join(day);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("rollout-{stamp}-{id}.jsonl"));
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&p, text).unwrap();
        p
    }

    fn meta(id: &str, cwd: &str, source: Value) -> Value {
        json!({"type": "session_meta", "timestamp": "2026-11-14T12:27:19.843Z", "payload": {
            "id": id, "cwd": cwd, "timestamp": "2026-11-14T12:27:19.759Z", "source": source,
            "originator": "codex-tui", "base_instructions": {"text": "x".repeat(30_000)},
            "git": {"branch": "main"}}})
    }

    fn user(text: &str) -> Value {
        json!({"type": "response_item", "payload": {"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": text}]}})
    }

    fn done(text: &str) -> Value {
        json!({"type": "event_msg", "payload": {"type": "task_complete", "last_agent_message": text}})
    }

    const A: &str = "01a1da06-c73f-79b3-bb63-a14cb2e0b2b6";
    const B: &str = "01a1da14-2615-7571-a7d4-2e6da4de1c9f";
    const C: &str = "01a1da12-c4bb-73c0-b55a-7f7b063f20f5";

    #[test]
    fn reads_interactive_sessions_and_skips_exec_and_other_dirs() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        write_rollout(
            home,
            "2026/11/14",
            "2026-11-14T20-27-19",
            A,
            &[
                meta(A, "/r/app", json!("vscode")),
                user("<environment_context>\n  <cwd>/r/app</cwd>"),
                user("# AGENTS.md instructions for /r/app\n…"),
                user("你可以知道現在多少 repo 嗎\n第二行"),
                done("先查一下"),
                done("共有 **35 個 repo**"),
            ],
        );
        // codex exec（程式化呼叫）與子 agent 不列；別的目錄不列
        write_rollout(
            home,
            "2026/11/14",
            "2026-11-14T20-41-56",
            B,
            &[meta(B, "/r/app", json!("exec")), user("hi"), done("ok")],
        );
        write_rollout(
            home,
            "2026/11/13",
            "2026-11-13T09-00-00",
            C,
            &[meta(C, "/elsewhere", json!("cli")), user("hi"), done("ok")],
        );
        std::fs::write(
            home.join("session_index.jsonl"),
            format!(
                "{}\n",
                json!({"id": A, "thread_name": "查看目前 repo 數量"})
            ),
        )
        .unwrap();
        let got = sessions_in(home, &|cwd| cwd.starts_with("/r/app"));
        assert_eq!(got.len(), 1);
        let s = &got[0];
        assert_eq!(s.id, A);
        assert_eq!(s.title.as_deref(), Some("查看目前 repo 數量"));
        assert_eq!(s.prompt, "你可以知道現在多少 repo 嗎 第二行");
        assert_eq!(s.reply.as_deref(), Some("共有 **35 個 repo**"));
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(rollout_of(home, A).as_deref(), Some(s.path.as_path()));
        assert!(rollout_of(home, "nope-nope").is_none());
    }

    #[test]
    fn missing_home_and_empty_sessions_are_quietly_empty() {
        let t = tempfile::tempdir().unwrap();
        assert!(sessions_in(&t.path().join("nope"), &|_| true).is_empty());
        // 只有 meta、沒人說過話
        write_rollout(
            t.path(),
            "2026/11/14",
            "2026-11-14T10-00-00",
            A,
            &[meta(A, "/r/app", json!("cli"))],
        );
        assert!(sessions_in(t.path(), &|_| true).is_empty());
    }
}
