//! 這個目錄的 Claude Code 會話打得出哪些 `/指令`（2026-10-22，ideation F-105；Rust 才有，參考 BAT Skills & Agents 面板）。
//!
//! skill 與自訂指令散在好幾個地方：`~/.claude/skills`、`~/.claude/commands`、專案的 `.claude/skills`／`commands`、
//! 裝好的 plugin。人常常不記得有哪些、叫什麼；這裡只讀每個檔的 front matter（name、description），列出來給人點一下插進會話。
//! 只讀、不執行。

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

/// 一個打得出來的 `/指令`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentCommand {
    /// 打的名字（不含 `/`）；plugin 的是 `plugin:名字`。
    pub name: String,
    pub description: String,
    /// `skill` 或 `command`。
    pub kind: &'static str,
    /// `user`（~/.claude）、`project`（這個 repo）、`plugin`。
    pub source: &'static str,
}

/// 每份檔最多讀多少位元組（front matter 在最前面）。
const HEAD_BYTES: usize = 8 * 1024;
/// 說明最多幾個字元。
const DESC_MAX: usize = 200;

fn head(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = vec![0u8; HEAD_BYTES];
    let n = std::fs::File::open(path).ok()?.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// front matter 裡的單行鍵值（`name: x`、`description: "y"`）；沒有 front matter＝None。
fn front(text: &str, key: &str) -> Option<String> {
    let body = text.strip_prefix("---")?;
    let end = body.find("\n---")?;
    body[..end].lines().find_map(|ln| {
        let v = ln.strip_prefix(key)?.strip_prefix(':')?.trim();
        let v = v.trim_matches(|c| c == '"' || c == '\'');
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// 沒有 front matter 的說明：第一個不是標題符號的非空行。
fn first_line(text: &str) -> String {
    let body = match text.strip_prefix("---") {
        Some(b) => b.find("\n---").map_or(b, |e| &b[e + 4..]),
        None => text,
    };
    body.lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

fn clip(s: &str) -> String {
    crate::transcript::one_line(s, DESC_MAX)
}

/// `<dir>/<name>/SKILL.md`（往下最多 `depth` 層，synced 的 skill 多包一層）。
fn skills_in(
    dir: &Path,
    depth: usize,
    prefix: &str,
    source: &'static str,
    out: &mut Vec<AgentCommand>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut subdirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    subdirs.sort();
    for d in subdirs {
        let skill = d.join("SKILL.md");
        if let Some(text) = skill.is_file().then(|| head(&skill)).flatten() {
            let fallback = d
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let name = front(&text, "name").unwrap_or(fallback);
            out.push(AgentCommand {
                name: format!("{prefix}{name}"),
                description: clip(
                    &front(&text, "description").unwrap_or_else(|| first_line(&text)),
                ),
                kind: "skill",
                source,
            });
        } else if depth > 0 {
            skills_in(&d, depth - 1, prefix, source, out);
        }
    }
}

/// `<dir>/*.md`（子資料夾＝命名空間：`frontend/lint.md` → `frontend:lint`）。
fn commands_in(dir: &Path, prefix: &str, source: &'static str, out: &mut Vec<AgentCommand>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        let stem = p
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if p.is_dir() {
            commands_in(&p, &format!("{prefix}{stem}:"), source, out);
        } else if p.extension().is_some_and(|x| x == "md") {
            let text = head(&p).unwrap_or_default();
            out.push(AgentCommand {
                name: format!("{prefix}{stem}"),
                description: clip(
                    &front(&text, "description").unwrap_or_else(|| first_line(&text)),
                ),
                kind: "command",
                source,
            });
        }
    }
}

/// 裝好、而且這個目錄用得到的 plugin（使用者範圍，或專案範圍且專案是這個目錄或它的上層）→ (名字, 安裝位置)。
fn plugins(claude_home: &Path, cwd: Option<&Path>) -> Vec<(String, PathBuf)> {
    let Some(v) =
        std::fs::read_to_string(claude_home.join("plugins").join("installed_plugins.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        return Vec::new();
    };
    let Some(map) = v.get("plugins").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, installs) in map {
        let name = key.split('@').next().unwrap_or(key).to_string();
        let applies = |i: &Value| match i.get("scope").and_then(Value::as_str) {
            Some("user") | None => true,
            _ => i
                .get("projectPath")
                .and_then(Value::as_str)
                .is_some_and(|pp| cwd.is_some_and(|c| c.starts_with(pp))),
        };
        if let Some(path) = installs
            .as_array()
            .into_iter()
            .flatten()
            .find(|i| applies(i))
            .and_then(|i| i.get("installPath").and_then(Value::as_str))
        {
            out.push((name, PathBuf::from(path)));
        }
    }
    out.sort();
    out
}

/// 這個目錄的會話打得出來的 `/指令`：專案的在前，再來使用者的、plugin 的；同名只留第一個（專案蓋過使用者）。
/// `claude_home` 預設 `~/.claude`（`CLAUDE_CONFIG_DIR`）。
pub fn list(cwd: Option<&Path>, claude_home: Option<&Path>) -> Vec<AgentCommand> {
    let home = claude_home.map_or_else(
        || {
            crate::backfill::projects_root()
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default()
        },
        Path::to_path_buf,
    );
    let mut out = Vec::new();
    if let Some(c) = cwd {
        skills_in(
            &c.join(".claude").join("skills"),
            1,
            "",
            "project",
            &mut out,
        );
        commands_in(&c.join(".claude").join("commands"), "", "project", &mut out);
    }
    skills_in(&home.join("skills"), 2, "", "user", &mut out);
    commands_in(&home.join("commands"), "", "user", &mut out);
    for (name, path) in plugins(&home, cwd) {
        let prefix = format!("{name}:");
        skills_in(&path.join("skills"), 1, &prefix, "plugin", &mut out);
        commands_in(&path.join("commands"), &prefix, "plugin", &mut out);
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|c| seen.insert(c.name.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, text: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn lists_project_user_and_plugin_skills_and_commands() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let repo = t.path().join("repo");
        write(
            &repo.join(".claude/skills/pm-ideation/SKILL.md"),
            "---\nname: pm-ideation\ndescription: \"構想盤點\"\n---\n# x",
        );
        write(
            &repo.join(".claude/commands/frontend/lint.md"),
            "# 跑前端 lint\n",
        );
        write(
            &home.join("skills/repo-context/SKILL.md"),
            "---\nname: repo-context\ndescription: 跨 repo 的脈絡\n---\n",
        );
        // 跟專案同名：專案的蓋過
        write(
            &home.join("skills/pm-ideation/SKILL.md"),
            "---\nname: pm-ideation\ndescription: 舊的\n---\n",
        );
        write(
            &home.join("skills/synced/abc/morning/SKILL.md"),
            "---\nname: morning\n---\n早上的簡報\n",
        );
        let pl = t.path().join("plugins/code-review");
        write(
            &pl.join("commands/code-review.md"),
            "---\ndescription: 審查 PR\n---\n",
        );
        let other = t.path().join("plugins/other");
        write(&other.join("commands/x.md"), "x");
        write(
            &home.join("plugins/installed_plugins.json"),
            &serde_json::json!({"version": 2, "plugins": {
                "code-review@official": [{"scope": "user", "installPath": pl}],
                "other@official": [{"scope": "project", "installPath": other, "projectPath": "/elsewhere"}],
            }})
            .to_string(),
        );
        let got = list(Some(&repo), Some(&home));
        let names: Vec<(&str, &str, &str)> = got
            .iter()
            .map(|c| (c.name.as_str(), c.kind, c.source))
            .collect();
        assert_eq!(
            names,
            [
                ("pm-ideation", "skill", "project"),
                ("frontend:lint", "command", "project"),
                ("repo-context", "skill", "user"),
                ("morning", "skill", "user"),
                ("code-review:code-review", "command", "plugin"),
            ]
        );
        assert_eq!(got[0].description, "構想盤點");
        assert_eq!(got[1].description, "跑前端 lint");
        assert_eq!(got[3].description, "早上的簡報");
        assert_eq!(got[4].description, "審查 PR");
        assert!(list(None, Some(&t.path().join("none"))).is_empty());
    }
}
