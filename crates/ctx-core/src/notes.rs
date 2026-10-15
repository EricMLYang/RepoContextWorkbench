//! 牌組筆記（2026-10-11；Rust 才有，Python 沒有對應模組）。
//!
//! 快速記一筆「屬於這個組、不屬於任何 repo」的事。存在 spine 的 `groups/<組>/notes.md`——
//! 這個資料夾本來就跟著組走（改名一起搬、刪組保留），也會跟 spine 一起進版控。
//!
//! 檔案就是普通 Markdown，人跟 agent 都能直接讀、直接改：
//!
//! ```text
//! # <組> 筆記
//!
//! ## 2026-10-11 22:30
//! 內容（可以多行）
//! ```
//!
//! 一則＝一個 `## YYYY-MM-DD HH:MM` 標題加底下的內文，新的寫在最上面。不是脊椎事件
//! （不進收件匣、不算判斷）；要變成待辦或判斷，另外走 `add_todo`／`log_decision`。

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};

use crate::locking::with_spine_lock;
use crate::registry;
use crate::{Error, Result};

/// 檔名（在 `groups/<組>/` 底下）。
pub const FILE_NAME: &str = "notes.md";

/// 一則筆記。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// `YYYY-MM-DD HH:MM`
    pub at: String,
    pub text: String,
}

/// 這個組的筆記檔路徑（不管存不存在）。
pub fn path(spine_dir: &Path, group: &str) -> PathBuf {
    spine_dir.join("groups").join(group).join(FILE_NAME)
}

/// `## YYYY-MM-DD HH:MM` → 時間字串。
fn heading(line: &str) -> Option<&str> {
    let at = line.strip_prefix("## ")?.trim_end();
    let b = at.as_bytes();
    let shape = at.len() == 16
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            10 => *c == b' ',
            13 => *c == b':',
            _ => c.is_ascii_digit(),
        });
    shape.then_some(at)
}

/// 拆成（開頭的說明文字, 各則筆記）。
fn parse(text: &str) -> (String, Vec<Note>) {
    let mut head = String::new();
    let mut notes: Vec<(String, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        if let Some(at) = heading(line) {
            notes.push((at.to_string(), Vec::new()));
        } else if let Some((_, body)) = notes.last_mut() {
            body.push(line);
        } else {
            head.push_str(line);
            head.push('\n');
        }
    }
    let notes = notes
        .into_iter()
        .map(|(at, body)| Note {
            at,
            text: body.join("\n").trim().to_string(),
        })
        .collect();
    (head, notes)
}

fn render(head: &str, notes: &[Note]) -> String {
    let mut out = head.trim_end().to_string();
    out.push_str("\n\n");
    for n in notes {
        out.push_str(&format!("## {}\n{}\n\n", n.at, n.text));
    }
    let mut out = out.trim_end().to_string();
    out.push('\n');
    out
}

/// 內文裡剛好長得像「## 時間」的行會被當成下一則——前面加一個空格（Markdown 照樣顯示）。
fn escape(text: &str) -> String {
    text.trim()
        .lines()
        .map(|l| {
            if heading(l.trim_start()).is_some() {
                format!(" {}", l.trim_start())
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn check_group(spine_dir: &Path, group: &str) -> Result<String> {
    let group = group.trim();
    if group.is_empty() {
        return Err(Error::invalid(
            "筆記只能記在已儲存的牌組上；先把這個範圍存成牌組",
        ));
    }
    // 回登記的原名（給的可能是事件標頭裡換寫過的 PM_Group）
    match registry::load(spine_dir)?.group(group) {
        Some(g) => Ok(g.name()),
        None => Err(Error::invalid(format!("組不存在: {group}"))),
    }
}

/// 讀檔；沒有檔＝沒有筆記。
fn read(p: &Path) -> Result<String> {
    match std::fs::read_to_string(p) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e.into()),
    }
}

/// 寫到暫存檔再改名（寫到一半當掉不會留下半個檔）。
fn write(p: &Path, text: &str) -> Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::fsutil::write_atomic(p, text.as_bytes())?;
    Ok(())
}

/// 這個組的筆記（新的在前，照檔案順序）。
pub fn list(spine_dir: &Path, group: &str) -> Result<Vec<Note>> {
    let group = check_group(spine_dir, group)?;
    Ok(parse(&read(&path(spine_dir, &group))?).1)
}

/// 記一筆（放最上面）。回寫進去的那一則。
pub fn add(spine_dir: &Path, group: &str, text: &str, when: Option<NaiveDateTime>) -> Result<Note> {
    let group = check_group(spine_dir, group)?;
    // N-39：筆記也在 spine（會進 git），秘密不落地
    let text = escape(&crate::redact::redact_secrets(text).0);
    if text.is_empty() {
        return Err(Error::invalid("先寫點內容再記"));
    }
    let note = Note {
        at: when
            .unwrap_or_else(|| Local::now().naive_local())
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        text,
    };
    let p = path(spine_dir, &group);
    with_spine_lock(spine_dir, || {
        let old = read(&p)?;
        let (head, mut notes) = parse(&old);
        let head = if head.trim().is_empty() {
            format!("# {group} 筆記\n")
        } else {
            head
        };
        notes.insert(0, note.clone());
        write(&p, &render(&head, &notes))
    })?;
    Ok(note)
}

/// 刪一則：時間＋內文都要對得上（檔案可能被人或 agent 改過，對不上就不刪）。
pub fn remove(spine_dir: &Path, group: &str, at: &str, text: &str) -> Result<()> {
    let group = check_group(spine_dir, group)?;
    let p = path(spine_dir, &group);
    with_spine_lock(spine_dir, || {
        let (head, mut notes) = parse(&read(&p)?);
        let Some(i) = notes
            .iter()
            .position(|n| n.at == at && n.text == text.trim())
        else {
            return Err(Error::invalid(
                "這則筆記已經不在了（可能在別處改過），請重新整理",
            ));
        };
        notes.remove(i);
        write(&p, &render(&head, &notes))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, m: u32) -> Option<NaiveDateTime> {
        NaiveDate::from_ymd_opt(2026, 10, 11)
            .unwrap()
            .and_hms_opt(h, m, 0)
    }

    #[test]
    fn parse_render_roundtrip() {
        let text = "# g 筆記\n\n說明\n\n## 2026-10-11 22:30\n第二則\n多行\n\n## 2026-10-11 09:00\n第一則\n";
        let (head, notes) = parse(text);
        assert_eq!(head, "# g 筆記\n\n說明\n\n");
        assert_eq!(
            notes,
            vec![
                Note {
                    at: "2026-10-11 22:30".into(),
                    text: "第二則\n多行".into()
                },
                Note {
                    at: "2026-10-11 09:00".into(),
                    text: "第一則".into()
                },
            ]
        );
        assert_eq!(render(&head, &notes), text);
    }

    #[test]
    fn heading_shape() {
        assert_eq!(heading("## 2026-10-11 22:30"), Some("2026-10-11 22:30"));
        assert_eq!(heading("## 2026-10-11 22:30  "), Some("2026-10-11 22:30"));
        assert_eq!(heading("## 待辦"), None);
        assert_eq!(heading("### 2026-10-11 22:30"), None);
        assert_eq!(heading(" ## 2026-10-11 22:30"), None);
    }

    #[test]
    fn escape_lookalike_headings() {
        assert_eq!(
            escape("  a\n## 2026-01-01 00:00\nb  "),
            "a\n ## 2026-01-01 00:00\nb"
        );
    }

    #[test]
    fn add_list_remove() {
        let (_tmp, d) = crate::testkit::spine_with_repos();
        assert!(list(&d, "g1").unwrap().is_empty());
        add(&d, "g1", "第一則", at(9, 0)).unwrap();
        add(&d, "g1", " 第二則\n## 2026-01-01 00:00 ", at(22, 30)).unwrap();
        let notes = list(&d, "g1").unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].at, "2026-10-11 22:30");
        assert_eq!(notes[0].text, "第二則\n ## 2026-01-01 00:00");
        assert_eq!(notes[1].text, "第一則");
        let file = std::fs::read_to_string(path(&d, "g1")).unwrap();
        assert!(file.starts_with("# g1 筆記\n\n## 2026-10-11 22:30\n"));

        // 對不上（內文被改過）不刪
        assert!(remove(&d, "g1", "2026-10-11 09:00", "別的").is_err());
        remove(&d, "g1", "2026-10-11 09:00", "第一則").unwrap();
        assert_eq!(list(&d, "g1").unwrap().len(), 1);
    }

    #[test]
    fn rejects_unknown_or_empty() {
        let (_tmp, d) = crate::testkit::spine_with_repos();
        assert!(add(&d, "nope", "x", None).is_err());
        assert!(add(&d, "", "x", None).is_err());
        assert!(add(&d, "g1", "   ", None).is_err());
        assert!(list(&d, "nope").is_err());
    }
}
