//! 使用者畫面上正在看什麼（2026-10-24；Rust 才有）。
//!
//! App 主區換檔、換模式、捲動停下來、換分頁時呼叫 [`record`] 寫 `.state/screen.json`（只有一份，後寫的蓋掉前面的）。
//! **不自動附進會話**：使用者說「根據畫面」「我正在看的」，agent 才呼叫 [`what_user_sees`]（MCP `what_user_sees`、
//! `ctx screen`）——工具呼叫留在逐字稿裡，看得到它什麼時候看了、看到什麼。
//! 這是暫時的畫面狀態，不寫 spine 事件。

use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::{agentapi, filetree, registry, Result};

/// 時間格式（到秒；agent 要分得出「剛剛」和「半小時前」）。
const AT_FMT: &str = "%Y-%m-%dT%H:%M:%S";

/// 檔案分頁的看法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileMode {
    /// 預覽（md 畫成文件、CSV 畫成表格）或程式碼檢視
    View,
    /// 原始碼（有預覽的檔切到原文）
    Source,
    /// 差異（對 HEAD）
    Diff,
    /// 編輯中
    Edit,
}

impl FileMode {
    fn label(self) -> &'static str {
        match self {
            FileMode::View => "檢視",
            FileMode::Source => "原始碼",
            FileMode::Diff => "差異",
            FileMode::Edit => "編輯",
        }
    }
}

/// 工作區的分頁（主區沒開檔時看的）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tab {
    Inbox,
    Notes,
    Repos,
    Files,
    Activity,
    Relations,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Inbox => "收件匣",
            Tab::Notes => "筆記",
            Tab::Repos => "Repo",
            Tab::Files => "檔案",
            Tab::Activity => "活動",
            Tab::Relations => "關係",
        }
    }
}

/// 右側欄的分頁。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelTab {
    Files,
    Changes,
    Info,
}

impl PanelTab {
    fn label(self) -> &'static str {
        match self {
            PanelTab::Files => "檔案",
            PanelTab::Changes => "變更",
            PanelTab::Info => "資訊",
        }
    }
}

/// 主區顯示的東西。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    /// 開著一個檔案分頁。
    File {
        /// 登記的 repo id
        repo: String,
        /// repo 裡的相對路徑
        path: String,
        mode: FileMode,
        /// 畫面上看得到的行（有行號的看法才有）
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<[u32; 2]>,
        /// md 預覽：畫面頂端所在的標題
        #[serde(default, skip_serializing_if = "Option::is_none")]
        heading: Option<String>,
        /// 有還沒存的修改（agent 讀檔看到的是存檔前的版本）
        #[serde(default)]
        unsaved: bool,
    },
    /// 看工作區的某個分頁。
    Workspace { tab: Tab },
}

/// 右側欄開著的 repo 與分頁。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Panel {
    pub repo: String,
    pub tab: PanelTab,
}

/// App 回報的畫面（[`record`] 的參數）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenInput {
    pub view: View,
    /// 目前選的牌組（範圍）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<Panel>,
}

/// 存在 `.state/screen.json` 的樣子：回報的內容＋什麼時候。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Screen {
    #[serde(flatten)]
    pub input: ScreenInput,
    pub at: String,
}

/// `.state/screen.json`。
pub fn path(spine_dir: &Path) -> PathBuf {
    spine_dir.join(".state").join("screen.json")
}

/// 讀上次回報的畫面；沒有或壞掉＝None。
pub fn read(spine_dir: &Path) -> Option<Screen> {
    let s = std::fs::read_to_string(path(spine_dir)).ok()?;
    serde_json::from_str(&s).ok()
}

/// App 回報畫面。跟上次一樣就不重寫（回 false）——捲來捲去停在同一段不會一直寫磁碟。
pub fn record(spine_dir: &Path, input: ScreenInput) -> Result<bool> {
    if read(spine_dir).is_some_and(|s| s.input == input) {
        return Ok(false);
    }
    let p = path(spine_dir);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let s = Screen {
        input,
        at: Local::now().format(AT_FMT).to_string(),
    };
    crate::fsutil::write_atomic(&p, serde_json::to_string(&s)?.as_bytes())?;
    Ok(true)
}

/// App 關掉：畫面不在了，別讓 agent 讀到舊的。
pub fn clear(spine_dir: &Path) {
    let _ = std::fs::remove_file(path(spine_dir));
}

/// [`what_user_sees`] 的回傳。
#[derive(Debug, Clone, Serialize)]
pub struct Seen {
    /// 一句話說使用者在看什麼（直接引用給使用者看也行）
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screen: Option<Screen>,
    /// 多久以前回報的（秒）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_seconds: Option<i64>,
    /// 檔案的絕對路徑（可以直接讀）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub abs: Option<String>,
    /// 那個檔在不在你（呼叫者 cwd）的 repo
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_repo: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// agent 問「使用者畫面上在看什麼」。`cwd`＝呼叫者所在目錄（判斷是不是同一個 repo）。
pub fn what_user_sees(spine_dir: &Path, cwd: Option<&Path>) -> Result<Seen> {
    let Some(screen) = read(spine_dir) else {
        return Ok(Seen {
            summary: "App 沒有回報畫面（App 沒開，或剛開還沒切過畫面）。".into(),
            screen: None,
            age_seconds: None,
            abs: None,
            same_repo: None,
            hint: Some("請使用者直接告訴你檔名或貼上內容。".into()),
        });
    };
    let age_seconds = NaiveDateTime::parse_from_str(&screen.at, AT_FMT)
        .ok()
        .map(|t| (Local::now().naive_local() - t).num_seconds().max(0));
    let mine = agentapi::where_am_i(spine_dir, cwd)
        .ok()
        .and_then(|w| w.repo)
        .map(|r| r.id);
    let mut seen = Seen {
        summary: String::new(),
        screen: None,
        age_seconds,
        abs: None,
        same_repo: None,
        hint: None,
    };
    let mut parts = Vec::new();
    match &screen.input.view {
        View::File {
            repo,
            path,
            mode,
            lines,
            heading,
            unsaved,
        } => {
            let mut s = format!("主區開著 {repo}:{path}（{}", mode.label());
            if let Some([a, b]) = lines {
                s += &format!("，看得到第 {a}–{b} 行");
            }
            if let Some(h) = heading.as_deref().filter(|h| !h.is_empty()) {
                s += &format!("，停在「{h}」");
            }
            s += "）";
            if *unsaved {
                s += "，有還沒存的修改";
            }
            parts.push(s);
            match registry::get_repo(spine_dir, repo)
                .and_then(|r| filetree::locate(&r.path(), path))
            {
                Ok(p) => seen.abs = Some(p.to_string_lossy().into_owned()),
                Err(e) => seen.hint = Some(format!("讀不到這個檔：{e}")),
            }
            seen.same_repo = mine.as_ref().map(|m| m == repo);
            if *unsaved && seen.hint.is_none() {
                seen.hint =
                    Some("檔案還沒存：你讀到的是磁碟上的版本，跟使用者畫面上的可能不同。".into());
            }
        }
        View::Workspace { tab } => {
            parts.push(format!("主區在工作區的「{}」分頁", tab.label()));
            seen.hint = Some(
                "要看那個分頁的內容，用 context_for（收件匣、未結、最近事件）或 repo_status。"
                    .into(),
            );
        }
    }
    if let Some(g) = &screen.input.group {
        parts.push(format!("選的牌組是「{g}」"));
    }
    if let Some(p) = &screen.input.panel {
        parts.push(format!("右側欄開著 {} 的「{}」", p.repo, p.tab.label()));
    }
    let mut summary = parts.join("；") + "。";
    if let Some(a) = age_seconds.filter(|a| *a >= 600) {
        summary += &format!("（{} 分鐘前回報的，可能已經不是現在的畫面）", a / 60);
    }
    seen.summary = summary;
    seen.screen = Some(screen);
    Ok(seen)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_view(path: &str) -> ScreenInput {
        ScreenInput {
            view: View::File {
                repo: "impl".into(),
                path: path.into(),
                mode: FileMode::View,
                lines: Some([10, 40]),
                heading: None,
                unsaved: false,
            },
            group: Some("G".into()),
            panel: None,
        }
    }

    #[test]
    fn record_skips_same_screen_and_reads_back() {
        let t = tempfile::tempdir().unwrap();
        assert!(read(t.path()).is_none());
        assert!(record(t.path(), file_view("a.rs")).unwrap());
        assert!(!record(t.path(), file_view("a.rs")).unwrap());
        assert!(record(t.path(), file_view("b.rs")).unwrap());
        let s = read(t.path()).unwrap();
        assert_eq!(s.input, file_view("b.rs"));
        clear(t.path());
        assert!(read(t.path()).is_none());
    }

    #[test]
    fn json_shape_is_flat_and_tagged() {
        let v = serde_json::to_value(file_view("a.rs")).unwrap();
        assert_eq!(v["view"]["kind"], "file");
        assert_eq!(v["view"]["mode"], "view");
        assert_eq!(v["view"]["lines"], serde_json::json!([10, 40]));
        let w: ScreenInput = serde_json::from_value(
            serde_json::json!({"view": {"kind": "workspace", "tab": "inbox"}}),
        )
        .unwrap();
        assert_eq!(w.view, View::Workspace { tab: Tab::Inbox });
        assert!(serde_json::from_value::<ScreenInput>(
            serde_json::json!({"view": {"kind": "workspace", "tab": "nope"}})
        )
        .is_err());
    }
}
