//! 常用 prompt（2026-10-22，ideation F-106；Rust 才有，參考 BAT 的 snippet manager）。
//!
//! 好用的那幾句（「照計畫做下一步，做完一步就停」「先列風險再動手」…）每次都重打，或散在各處。
//! 這裡存在 spine 的 `prompts/<名稱>.md`：一句一個檔、人可以直接打開改、跟 spine 一起有版本；
//! 不放 `.state`（那是可以丟掉的快取）。App 從會話選單挑一句貼進會話。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{Error, Result};

/// 一句常用 prompt。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Prompt {
    /// 檔名（不含 `.md`），也是顯示的名稱。
    pub name: String,
    pub text: String,
    /// 最後修改（本地 `YYYY-MM-DD HH:MM`）。
    pub updated: String,
}

/// 名稱最多幾個字元；內容最多多少位元組。
const NAME_MAX: usize = 60;
const TEXT_MAX: usize = 20_000;

/// `<spine>/prompts`。
pub fn dir(spine_dir: &Path) -> PathBuf {
    spine_dir.join("prompts")
}

/// 名稱能不能當檔名：不空、不以 `.` 開頭、沒有路徑或檔名不允許的字元（中文可以）。
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= NAME_MAX
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| "/\\:*?\"<>|".contains(c) || c.is_control())
}

/// 全部常用 prompt，最近改的在前。
pub fn list(spine_dir: &Path) -> Vec<Prompt> {
    let Ok(rd) = std::fs::read_dir(dir(spine_dir)) else {
        return Vec::new();
    };
    let mut out: Vec<(std::time::SystemTime, Prompt)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "md") {
                return None;
            }
            let name = p.file_stem()?.to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&p).ok()?;
            let modified = e.metadata().ok()?.modified().ok()?;
            let updated = chrono::DateTime::<chrono::Local>::from(modified)
                .format("%Y-%m-%d %H:%M")
                .to_string();
            Some((
                modified,
                Prompt {
                    name,
                    text: text.trim_end().to_string(),
                    updated,
                },
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    out.into_iter().map(|(_, p)| p).collect()
}

/// 存一句（同名就覆蓋）。`rename_from` 有給而且跟 `name` 不同＝改名（舊檔刪掉）。
pub fn save(spine_dir: &Path, name: &str, text: &str, rename_from: Option<&str>) -> Result<Prompt> {
    let name = name.trim();
    if !valid_name(name) {
        return Err(Error::invalid(format!(
            "名稱不能空白、不能以 . 開頭、不能有 / \\ : * ? \" < > |，最多 {NAME_MAX} 字"
        )));
    }
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::invalid("內容是空的"));
    }
    if text.len() > TEXT_MAX {
        return Err(Error::invalid("內容太長了（最多 20KB）"));
    }
    let d = dir(spine_dir);
    std::fs::create_dir_all(&d)?;
    let path = d.join(format!("{name}.md"));
    crate::fsutil::write_atomic(&path, format!("{text}\n").as_bytes())?;
    if let Some(old) = rename_from
        .map(str::trim)
        .filter(|o| *o != name && valid_name(o))
    {
        let _ = std::fs::remove_file(d.join(format!("{old}.md")));
    }
    Ok(list(spine_dir)
        .into_iter()
        .find(|p| p.name == name)
        .unwrap_or(Prompt {
            name: name.to_string(),
            text: text.to_string(),
            updated: String::new(),
        }))
}

/// 刪一句。沒有這句＝錯誤（畫面可能過時了）。
pub fn remove(spine_dir: &Path, name: &str) -> Result<()> {
    if !valid_name(name) {
        return Err(Error::invalid(format!("名稱不對：{name}")));
    }
    let p = dir(spine_dir).join(format!("{name}.md"));
    if !p.is_file() {
        return Err(Error::invalid(format!("沒有這句：{name}")));
    }
    std::fs::remove_file(p)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_rename_remove() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        assert!(list(d).is_empty());
        save(d, "下一步", "  照計畫做下一步，\n做完一步就停  ", None).unwrap();
        let got = list(d);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].text, "照計畫做下一步，\n做完一步就停");
        assert_eq!(
            std::fs::read_to_string(d.join("prompts/下一步.md")).unwrap(),
            "照計畫做下一步，\n做完一步就停\n"
        );
        save(d, "做下一步", "照計畫做下一步", Some("下一步")).unwrap();
        let names: Vec<String> = list(d).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["做下一步"]);
        for bad in ["", ".x", "a/b", "a:b", &"長".repeat(61)] {
            assert!(save(d, bad, "x", None).is_err(), "{bad}");
        }
        assert!(save(d, "空的", "   ", None).is_err());
        assert!(remove(d, "沒有").is_err());
        assert!(remove(d, "../registry").is_err());
        remove(d, "做下一步").unwrap();
        assert!(list(d).is_empty());
    }
}
