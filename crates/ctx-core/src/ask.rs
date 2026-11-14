//! 交接的提問欄位（F-123 第 1 輪，2026-11-14，Rust 才有）：agent 停下來等人時，把「要你決定什麼」寫成結構。
//!
//! 為什麼要有：交接的「下次從這裡接」是一句自由文字，接力只能用 regex 猜它是不是在等人；人看到時也常常
//! 想不起這個 repo 的細節，答不出來。提問欄位讓 agent 把問題、選項、建議、類別、可逆與否、證據一次交代清楚，
//! 決策佇列（[`crate::decide`]）照它出卡，人 30 秒內看得懂、用編號答得完。
//!
//! 合約（寫進交接事件 body，接在「完成：／剩下：／下次從這裡接：」之後，同樣「鍵：值」一行一個）：
//! ```text
//! 要你決定：<問題>
//! 類別：<AskKind 的 snake_case>
//! 選項：<A>｜<B>｜<C>        ← 沒選項不寫這行
//! 建議：<選項文字或自由文字>    ← 沒建議不寫這行
//! 可逆：是｜否
//! 證據：<路徑>；<repo>:<路徑>  ← 沒證據不寫這行
//! ```
//! 分隔字元（選項的「｜」、證據的「；」）出現在值裡＝拒絕並說明怎麼改（不跳脫：人手改 md 時看得懂、改得動）。
//! 讀回時寬鬆：缺行、看不懂的類別、看不懂的可逆都不報錯——類別當 `other`、可逆當「否」（第 2 輪的自動放行
//! 因此永遠不會放過看不懂的提問）。

use serde::Serialize;

use crate::{Error, RejectCode, Result};

/// 選項之間的分隔字元。
pub const OPTION_SEP: char = '｜';
/// 證據之間的分隔字元。
pub const EVIDENCE_SEP: char = '；';

/// 交接 body 裡提問欄位的鍵（「鍵：值」的鍵）。
pub mod keys {
    pub const QUESTION: &str = "要你決定";
    pub const KIND: &str = "類別";
    pub const OPTIONS: &str = "選項";
    pub const RECOMMEND: &str = "建議";
    pub const REVERSIBLE: &str = "可逆";
    pub const EVIDENCE: &str = "證據";
}

/// 提問的類別（固定清單，不讓模型猜相似度：人要看得懂為什麼兩件事被歸成一類）。
/// snake_case 字串（[`AskKind::as_str`]）是合約：寫在交接事件、MCP 參數、CLI 旗標、`--json` 裡。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    /// 要不要 commit（看過 diff 再決定）
    Commit,
    /// 看過內容（文件、草稿、回覆）再放行
    ContentReview,
    /// 改設定（registry、config、環境）
    ConfigChange,
    /// 從幾個方案挑一個
    Choose,
    /// 要人用眼睛看（版面、畫面、圖）
    VisualCheck,
    /// 要人實際操作測一輪
    ManualTest,
    /// push 到遠端
    Push,
    /// 發佈（release、上架、公開）
    Publish,
    /// 刪東西
    Delete,
    /// 對外送出（寄信、發訊息、送給別人）
    SendExternal,
    /// 其他
    Other,
}

impl AskKind {
    /// 全部類別（順序＝說明與錯誤訊息裡列出的順序）。
    pub const ALL: [AskKind; 11] = [
        AskKind::Commit,
        AskKind::ContentReview,
        AskKind::ConfigChange,
        AskKind::Choose,
        AskKind::VisualCheck,
        AskKind::ManualTest,
        AskKind::Push,
        AskKind::Publish,
        AskKind::Delete,
        AskKind::SendExternal,
        AskKind::Other,
    ];

    /// 全部類別的字串（CLI 的 `value_parser`、MCP 的 enum 用）。
    pub const NAMES: [&'static str; 11] = [
        "commit",
        "content_review",
        "config_change",
        "choose",
        "visual_check",
        "manual_test",
        "push",
        "publish",
        "delete",
        "send_external",
        "other",
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AskKind::Commit => "commit",
            AskKind::ContentReview => "content_review",
            AskKind::ConfigChange => "config_change",
            AskKind::Choose => "choose",
            AskKind::VisualCheck => "visual_check",
            AskKind::ManualTest => "manual_test",
            AskKind::Push => "push",
            AskKind::Publish => "publish",
            AskKind::Delete => "delete",
            AskKind::SendExternal => "send_external",
            AskKind::Other => "other",
        }
    }

    /// snake_case 字串 → 類別；看不懂＝None。
    pub fn parse(s: &str) -> Option<AskKind> {
        let s = s.trim();
        AskKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// 人看的名稱。
    pub fn label(self) -> &'static str {
        match self {
            AskKind::Commit => "commit",
            AskKind::ContentReview => "看內容",
            AskKind::ConfigChange => "改設定",
            AskKind::Choose => "選方案",
            AskKind::VisualCheck => "目視檢查",
            AskKind::ManualTest => "手動測試",
            AskKind::Push => "push",
            AskKind::Publish => "發佈",
            AskKind::Delete => "刪除",
            AskKind::SendExternal => "對外送出",
            AskKind::Other => "其他",
        }
    }

    /// 這類提問能不能被自動放行（第 2 輪的放行規則用）。寫死在引擎，不是設定：
    /// 對外、不可收回、要人親自看或測的，永遠要人回答。
    pub fn auto_approvable(self) -> bool {
        !matches!(
            self,
            AskKind::Push
                | AskKind::Publish
                | AskKind::Delete
                | AskKind::SendExternal
                | AskKind::VisualCheck
                | AskKind::ManualTest
                | AskKind::Other
        )
    }
}

/// 一個提問（驗過、正規化過的）。`recommend` 有選項時一定是其中一個選項的文字。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ask {
    pub question: String,
    pub kind: AskKind,
    pub options: Vec<String>,
    pub recommend: Option<String>,
    pub reversible: bool,
    /// repo 內相對路徑，或 `<repo>:<路徑>`
    pub evidence: Vec<String>,
}

impl Ask {
    /// 建議是第幾個選項（1 起算）；沒建議或建議不在選項裡（人手改過）＝None。
    pub fn recommend_index(&self) -> Option<usize> {
        let r = self.recommend.as_deref()?;
        self.options.iter().position(|o| o == r).map(|i| i + 1)
    }

    /// 寫進交接 body 的幾行（接在「下次從這裡接：」之後）。
    pub fn body_lines(&self) -> Vec<String> {
        let mut out = vec![
            format!("{}：{}", keys::QUESTION, self.question),
            format!("{}：{}", keys::KIND, self.kind.as_str()),
        ];
        if !self.options.is_empty() {
            out.push(format!(
                "{}：{}",
                keys::OPTIONS,
                self.options.join(&OPTION_SEP.to_string())
            ));
        }
        if let Some(r) = &self.recommend {
            out.push(format!("{}：{r}", keys::RECOMMEND));
        }
        out.push(format!(
            "{}：{}",
            keys::REVERSIBLE,
            if self.reversible { "是" } else { "否" }
        ));
        if !self.evidence.is_empty() {
            out.push(format!(
                "{}：{}",
                keys::EVIDENCE,
                self.evidence.join(&EVIDENCE_SEP.to_string())
            ));
        }
        out
    }

    /// 從交接的「鍵：值」讀回提問（`get` 給某個鍵的值）。沒有「要你決定」或問題是空的＝None。
    /// 寬鬆：看不懂的類別＝`other`；「可逆」不是「是」（缺行、手改成別的字）＝否。
    pub fn from_fields(get: impl Fn(&str) -> Option<String>) -> Option<Ask> {
        let question = get(keys::QUESTION)
            .map(|q| q.trim().to_string())
            .filter(|q| !q.is_empty())?;
        let split = |k: &str, sep: char| -> Vec<String> {
            get(k)
                .map(|v| {
                    v.split(sep)
                        .map(|x| x.trim().to_string())
                        .filter(|x| !x.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        Some(Ask {
            question,
            kind: get(keys::KIND)
                .and_then(|k| AskKind::parse(&k))
                .unwrap_or(AskKind::Other),
            options: split(keys::OPTIONS, OPTION_SEP),
            recommend: get(keys::RECOMMEND)
                .map(|r| r.trim().to_string())
                .filter(|r| !r.is_empty()),
            reversible: get(keys::REVERSIBLE).is_some_and(|v| {
                matches!(
                    v.trim().to_lowercase().as_str(),
                    "是" | "可" | "true" | "yes"
                )
            }),
            evidence: split(keys::EVIDENCE, EVIDENCE_SEP),
        })
    }
}

/// 呼叫端給的提問（還沒驗過）。CLI 旗標與 MCP 的 `ask` 物件都先轉成這個，再 [`AskInput::validate`]。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AskInput {
    pub question: String,
    /// 省略＝`other`（永遠要人回答）
    pub kind: Option<String>,
    pub options: Vec<String>,
    /// 選項文字或 1 起算的編號
    pub recommend: Option<String>,
    /// 省略＝可逆
    pub reversible: Option<bool>,
    pub evidence: Vec<String>,
}

fn bad(msg: impl Into<String>, hint: &str) -> Error {
    Error::rejected(RejectCode::BadRequest, msg, Some(hint.to_string()))
}

/// 多行併成一行（交接一個鍵一行）。
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl AskInput {
    /// 驗證並正規化：問題不能空；類別要在清單裡；選項／證據不能含分隔字元、不能重複；
    /// 有選項時建議要是其中之一（文字或 1 起算的編號，存成選項文字）；證據要是相對路徑或 `<repo>:<路徑>`。
    /// 錯誤都是 `bad_request`，附怎麼改。
    pub fn validate(&self) -> Result<Ask> {
        let question = one_line(&self.question);
        if question.is_empty() {
            return Err(bad(
                "ask.question 不能是空的",
                "寫一句不看程式也答得出來的問題，例：預設排序改成依修改時間（A）還是保持名稱（B）？",
            ));
        }
        let kind = match self
            .kind
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            None => AskKind::Other,
            Some(k) => AskKind::parse(k).ok_or_else(|| {
                bad(
                    format!("ask.kind 看不懂：「{k}」"),
                    &format!("用這幾個之一：{}", AskKind::NAMES.join("、")),
                )
            })?,
        };
        let mut options: Vec<String> = Vec::new();
        for o in &self.options {
            if o.contains('\n') {
                return Err(bad(
                    format!("選項不能換行：「{}」", one_line(o)),
                    "每個選項寫成一句話",
                ));
            }
            let o = o.trim().to_string();
            if o.is_empty() {
                continue;
            }
            if o.contains(OPTION_SEP) {
                return Err(bad(
                    format!("選項裡不能有「{OPTION_SEP}」（交接裡用它分隔選項）：「{o}」"),
                    "改用「/」或「、」",
                ));
            }
            if options.contains(&o) {
                return Err(bad(
                    format!("選項重複：「{o}」"),
                    "每個選項要不一樣，人才能用編號回答",
                ));
            }
            options.push(o);
        }
        let recommend = match self
            .recommend
            .as_deref()
            .map(one_line)
            .filter(|r| !r.is_empty())
        {
            None => None,
            Some(r) if options.is_empty() => Some(r),
            Some(r) => Some(pick_option(&options, &r)?),
        };
        let mut evidence: Vec<String> = Vec::new();
        for e in &self.evidence {
            let e = e.trim();
            if e.is_empty() {
                continue;
            }
            if e.contains(EVIDENCE_SEP) || e.contains('\n') {
                return Err(bad(
                    format!(
                        "證據一筆一個路徑，不能有「{EVIDENCE_SEP}」或換行：「{}」",
                        one_line(e)
                    ),
                    "分成多筆給",
                ));
            }
            if e.starts_with('/') || e.starts_with('~') || std::path::Path::new(e).is_absolute() {
                return Err(bad(
                    format!("證據要寫 repo 內的相對路徑：「{e}」"),
                    "寫成 docs/plan.md，或別的 repo 的 <repo>:<路徑>",
                ));
            }
            if !evidence.iter().any(|x| x == e) {
                evidence.push(e.to_string());
            }
        }
        Ok(Ask {
            question,
            kind,
            options,
            recommend,
            reversible: self.reversible.unwrap_or(true),
            evidence,
        })
    }
}

/// 建議 → 選項文字：先比文字，再看是不是 1 起算的編號。都不是＝拒絕並列出選項。
fn pick_option(options: &[String], r: &str) -> Result<String> {
    if let Some(o) = options.iter().find(|o| o.as_str() == r) {
        return Ok(o.clone());
    }
    if let Ok(n) = r.parse::<usize>() {
        if (1..=options.len()).contains(&n) {
            return Ok(options[n - 1].clone());
        }
    }
    Err(bad(
        format!("建議「{r}」不是選項之一"),
        &format!("建議要是選項的文字或編號：{}", numbered(options).join("；")),
    ))
}

/// `1) A`、`2) B`…
pub fn numbered(options: &[String]) -> Vec<String> {
    options
        .iter()
        .enumerate()
        .map(|(i, o)| format!("{}) {o}", i + 1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(q: &str) -> AskInput {
        AskInput {
            question: q.into(),
            ..Default::default()
        }
    }

    #[test]
    fn kinds_round_trip_and_auto_rule() {
        for (k, n) in AskKind::ALL.iter().zip(AskKind::NAMES) {
            assert_eq!(k.as_str(), n);
            assert_eq!(AskKind::parse(n), Some(*k));
            assert_eq!(serde_json::to_value(k).unwrap(), serde_json::json!(n));
        }
        let auto: Vec<&str> = AskKind::ALL
            .iter()
            .filter(|k| k.auto_approvable())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            auto,
            ["commit", "content_review", "config_change", "choose"]
        );
        assert_eq!(AskKind::parse("Commit"), None);
    }

    #[test]
    fn validate_normalizes() {
        let a = AskInput {
            question: "要 commit\n這兩個檔嗎？".into(),
            kind: Some("commit".into()),
            options: vec![" 先 commit ".into(), "".into(), "不要".into()],
            recommend: Some("2".into()),
            reversible: None,
            evidence: vec!["docs/a.md".into(), "docs/a.md".into(), "other:b.md".into()],
        }
        .validate()
        .unwrap();
        assert_eq!(a.question, "要 commit 這兩個檔嗎？");
        assert_eq!(a.options, ["先 commit", "不要"]);
        assert_eq!(a.recommend.as_deref(), Some("不要"));
        assert_eq!(a.recommend_index(), Some(2));
        assert!(a.reversible);
        assert_eq!(a.evidence, ["docs/a.md", "other:b.md"]);
        // 建議也可以寫選項文字
        let b = AskInput {
            options: vec!["A".into(), "B".into()],
            recommend: Some("B".into()),
            ..input("哪個？")
        }
        .validate()
        .unwrap();
        assert_eq!(b.recommend_index(), Some(2));
        // 沒選項時建議是自由文字；沒類別＝other
        let c = AskInput {
            recommend: Some("先不動".into()),
            ..input("要改嗎？")
        }
        .validate()
        .unwrap();
        assert_eq!(c.kind, AskKind::Other);
        assert_eq!(c.recommend.as_deref(), Some("先不動"));
        assert_eq!(c.recommend_index(), None);
    }

    fn rejected_msg(r: Result<Ask>) -> (String, String) {
        match r {
            Err(Error::Rejected {
                code: RejectCode::BadRequest,
                message,
                hint,
            }) => (message, hint.unwrap_or_default()),
            other => panic!("應該是 bad_request：{other:?}"),
        }
    }

    #[test]
    fn validate_rejects_with_hints() {
        let (m, _) = rejected_msg(input("  \n ").validate());
        assert!(m.contains("question"));
        let (m, h) = rejected_msg(
            AskInput {
                kind: Some("merge".into()),
                ..input("q")
            }
            .validate(),
        );
        assert!(m.contains("merge") && h.contains("content_review"));
        let (m, h) = rejected_msg(
            AskInput {
                options: vec!["A｜B".into()],
                ..input("q")
            }
            .validate(),
        );
        assert!(m.contains("｜") && h.contains("/"));
        let (m, _) = rejected_msg(
            AskInput {
                options: vec!["A".into(), "A".into()],
                ..input("q")
            }
            .validate(),
        );
        assert!(m.contains("重複"));
        let (m, h) = rejected_msg(
            AskInput {
                options: vec!["A".into(), "B".into()],
                recommend: Some("C".into()),
                ..input("q")
            }
            .validate(),
        );
        assert!(m.contains("C"));
        assert!(h.contains("1) A") && h.contains("2) B"), "{h}");
        let (_, h) = rejected_msg(
            AskInput {
                options: vec!["A".into(), "B".into()],
                recommend: Some("3".into()),
                ..input("q")
            }
            .validate(),
        );
        assert!(h.contains("2) B"));
        let (m, _) = rejected_msg(
            AskInput {
                evidence: vec!["a.md；b.md".into()],
                ..input("q")
            }
            .validate(),
        );
        assert!(m.contains("；"));
        let (m, _) = rejected_msg(
            AskInput {
                evidence: vec!["/etc/passwd".into()],
                ..input("q")
            }
            .validate(),
        );
        assert!(m.contains("相對路徑"));
    }

    #[test]
    fn body_lines_and_back() {
        let a = AskInput {
            question: "選哪個？".into(),
            kind: Some("choose".into()),
            options: vec!["A：用快取".into(), "B".into()],
            recommend: Some("1".into()),
            reversible: Some(false),
            evidence: vec!["docs/x.md".into(), "r2:y.md".into()],
        }
        .validate()
        .unwrap();
        let lines = a.body_lines();
        assert_eq!(
            lines,
            [
                "要你決定：選哪個？",
                "類別：choose",
                "選項：A：用快取｜B",
                "建議：A：用快取",
                "可逆：否",
                "證據：docs/x.md；r2:y.md",
            ]
        );
        let get = |k: &str| {
            lines
                .iter()
                .find_map(|l| l.split_once('：').filter(|(kk, _)| *kk == k))
                .map(|(_, v)| v.to_string())
        };
        assert_eq!(Ask::from_fields(get), Some(a));
        // 最少的提問只有三行
        let m = input("好嗎？").validate().unwrap();
        assert_eq!(
            m.body_lines(),
            ["要你決定：好嗎？", "類別：other", "可逆：是"]
        );
    }

    #[test]
    fn from_fields_is_lenient() {
        let only_q = |k: &str| (k == keys::QUESTION).then(|| "要不要？".to_string());
        let a = Ask::from_fields(only_q).unwrap();
        assert_eq!(a.kind, AskKind::Other);
        assert!(!a.reversible, "缺「可逆」當不可逆");
        assert!(a.options.is_empty() && a.evidence.is_empty() && a.recommend.is_none());
        let weird = |k: &str| match k {
            keys::QUESTION => Some("q".to_string()),
            keys::KIND => Some("合併".to_string()),
            keys::REVERSIBLE => Some("大概".to_string()),
            keys::OPTIONS => Some("｜A｜｜B｜".to_string()),
            keys::RECOMMEND => Some("C".to_string()),
            _ => None,
        };
        let w = Ask::from_fields(weird).unwrap();
        assert_eq!(w.kind, AskKind::Other);
        assert!(!w.reversible);
        assert_eq!(w.options, ["A", "B"]);
        assert_eq!(w.recommend.as_deref(), Some("C"));
        assert_eq!(w.recommend_index(), None);
        assert!(
            Ask::from_fields(|k: &str| (k == keys::QUESTION).then(|| " ".to_string())).is_none()
        );
        assert!(Ask::from_fields(|_: &str| None).is_none());
    }
}
