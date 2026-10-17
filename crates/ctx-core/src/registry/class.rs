//! repo 類別（Rust 才有，2026-10-18）：依「怎樣才算可靠」分六類，決定工作台與 agent 怎麼對待一個 repo。
//!
//! 第一版是**軟區分**：只標示＋告訴 agent 這類的規則（開場注入、名冊、名片），不加硬性限制。
//! 唯一改變行為的是「參考」：比照舊的 `type: external`，不採集 git 狀態、不進程式碼搜尋。
//! 之後要替某一類加共同限制（App 會話權限、遮蔽、反芻不外推…），在 [`RepoClass`] 加一個方法、
//! 各處照類別問它就好——規則只寫在這裡。設計見 `docs/REPO_CLASSES.md`。
//!
//! 存在 registry 的 `class` 欄（不給＝未分類，照程式對待）；`type: external` 沒寫 class 時當成參考。

use std::fmt;
use std::str::FromStr;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepoClass {
    /// 你在開發的程式（含資料管線、爬蟲）：測試通過才算數。
    Code,
    /// 領域 Agent：prompt／skill／流程是產品本體，eval 或範例跑過才算數。
    Agent,
    /// 數據探索：notebook、查詢、分析，重跑得出同樣數字、每個數字有出處才算數。
    Data,
    /// 文件・PM：決策與進度，事實正確、決策有記錄才算數。
    Docs,
    /// 知識：筆記、書摘、卡片，有出處即可，可以大膽整理。
    Knowledge,
    /// 參考：別人的專案、clone 來讀，只讀不改。
    Reference,
}

impl RepoClass {
    /// 宣告順序＝畫面上的選項順序。
    pub const ALL: [RepoClass; 6] = [
        RepoClass::Code,
        RepoClass::Agent,
        RepoClass::Data,
        RepoClass::Docs,
        RepoClass::Knowledge,
        RepoClass::Reference,
    ];

    /// registry 裡存的字。
    pub fn as_str(self) -> &'static str {
        match self {
            RepoClass::Code => "code",
            RepoClass::Agent => "agent",
            RepoClass::Data => "data",
            RepoClass::Docs => "docs",
            RepoClass::Knowledge => "knowledge",
            RepoClass::Reference => "reference",
        }
    }

    /// 畫面與注入用的名稱。
    pub fn label(self) -> &'static str {
        match self {
            RepoClass::Code => "程式",
            RepoClass::Agent => "領域 Agent",
            RepoClass::Data => "數據探索",
            RepoClass::Docs => "文件・PM",
            RepoClass::Knowledge => "知識",
            RepoClass::Reference => "參考",
        }
    }

    /// 怎樣才算可靠（分類的主軸；名片卡與說明用）。
    pub fn reliable(self) -> &'static str {
        match self {
            RepoClass::Code => "測試通過",
            RepoClass::Agent => "eval／範例跑過、prompt 改動有理由",
            RepoClass::Data => "重跑得出同樣數字、每個數字有出處",
            RepoClass::Docs => "事實正確、決策有記錄",
            RepoClass::Knowledge => "有出處，可以自由整理",
            RepoClass::Reference => "不改，只讀",
        }
    }

    /// 給在這個 repo 工作的 agent 的規則（開場注入「你在這」那一行）。
    pub fn rule(self) -> &'static str {
        match self {
            RepoClass::Code => "改完跑測試，照這個 repo 的 AGENTS.md 規矩 commit",
            RepoClass::Agent => {
                "prompt／skill／流程文件是產品本體：改措辭要說理由、保留領域術語；改了 prompt 就跑這個 repo 的 eval 或範例，結果用 log_decision 記下（改了什麼、為什麼、效果）；工具程式碼照程式規矩"
            }
            RepoClass::Data => {
                "每個數字都附出處（查詢、表、資料日期），不確定就說不確定、不要補；大資料先抽樣、不整份讀；資料列不要貼進 log_decision／handoff；發現用 log_decision 記結論＋來源"
            }
            RepoClass::Docs => {
                "寫的是事實與決策：不確定就問、不要補細節；不用跑測試；可能含真實工作資訊，不要貼到外部"
            }
            RepoClass::Knowledge => "可以重組、合併、改寫筆記；引用外部來源要附出處",
            RepoClass::Reference => {
                "別人的專案，只讀：不要改、不要 commit；引用時寫明是哪個專案的哪個檔"
            }
        }
    }

    /// 給讀別的 repo 的 agent 的短規則（名冊一列的尾巴；程式類不加）。
    pub fn short_rule(self) -> Option<&'static str> {
        match self {
            RepoClass::Code => None,
            RepoClass::Agent => Some("改 prompt 要說理由"),
            RepoClass::Data => Some("數字要附出處"),
            RepoClass::Docs => Some("事實別補、別外傳"),
            RepoClass::Knowledge => Some("可自由整理"),
            RepoClass::Reference => Some("只讀"),
        }
    }

    /// 要不要採集 git 狀態（沒 commit、多久沒動、ahead／behind）。參考 repo 不是你的工作，不採集。
    pub fn collects_git(self) -> bool {
        self != RepoClass::Reference
    }

    /// 要不要進程式碼搜尋（`git grep`）。參考 repo 預設不搜（大 repo 拖慢、結果被它佔掉）；md 知識索引照常。
    pub fn searches_code(self) -> bool {
        self != RepoClass::Reference
    }
}

/// 類別連同說明（給名片、名冊、App；前端不自己對照名稱與規則）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ClassInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub reliable: &'static str,
    pub rule: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short: Option<&'static str>,
}

impl From<RepoClass> for ClassInfo {
    fn from(c: RepoClass) -> Self {
        ClassInfo {
            id: c.as_str(),
            label: c.label(),
            reliable: c.reliable(),
            rule: c.rule(),
            short: c.short_rule(),
        }
    }
}

impl fmt::Display for RepoClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RepoClass {
    type Err = Error;

    /// 認 registry 的字（大小寫不拘）與中文名稱。
    fn from_str(s: &str) -> Result<Self> {
        let t = s.trim();
        RepoClass::ALL
            .into_iter()
            .find(|c| c.as_str().eq_ignore_ascii_case(t) || c.label() == t)
            .ok_or_else(|| {
                Error::invalid(format!(
                    "類別需為 {}（空字串＝清掉）",
                    RepoClass::ALL.map(RepoClass::as_str).join("|")
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ids_and_labels() {
        for c in RepoClass::ALL {
            assert_eq!(c.as_str().parse::<RepoClass>().unwrap(), c);
            assert_eq!(c.label().parse::<RepoClass>().unwrap(), c);
        }
        assert_eq!(
            "Reference".parse::<RepoClass>().unwrap(),
            RepoClass::Reference
        );
        assert!("pm".parse::<RepoClass>().is_err());
    }

    #[test]
    fn only_reference_changes_behavior() {
        for c in RepoClass::ALL {
            let r = c == RepoClass::Reference;
            assert_eq!(c.collects_git(), !r);
            assert_eq!(c.searches_code(), !r);
        }
    }
}
