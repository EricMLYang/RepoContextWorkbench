//! CLI 共用：spine 定位、錯誤出口、--json 輸出。對應 `repo_context/cli/common.py`。
//!
//! exit code 約定（agent 靠它分支，不必讀中文）：
//!   0 成功｜1 稽核／lint 有紅字或一般失敗｜2 用法錯誤或 agent 請求被拒（--json 時 stdout 有 {ok:false,...}）
//!   3 找不到 spine（先 `ctx init` 或 `ctx setup --spine <路徑>`）

use std::path::PathBuf;

use ctx_core::{config, fsutil, Error};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::pyjson;

pub const EXIT_FINDINGS: i32 = 1;
pub const EXIT_REJECTED: i32 = 2;
pub const EXIT_NO_SPINE: i32 = 3;

/// 子命令處理函式的結束方式。`Ok(())`＝exit 0。
#[derive(Debug)]
pub enum Exit {
    /// 對應 `_fail`：--json 時 stdout 印 {ok:false,error,message,hint?} 並以 code 結束；
    /// 否則 stderr 印「錯誤：…」並 exit 1（Python `sys.exit(str)` 一律是 1）。
    Fail {
        error: String,
        message: String,
        hint: Option<String>,
        code: i32,
    },
    /// 對應 `sys.exit(n)`（訊息已經印過）。
    Code(i32),
    /// 對應 `sys.exit("訊息")`：stderr 印訊息，exit 1。
    Msg(String),
    /// 沒接住的引擎錯誤（Python 會是 traceback＋exit 1）。
    Core(Error),
}

impl From<Error> for Exit {
    fn from(e: Error) -> Self {
        Exit::Core(e)
    }
}

impl From<std::io::Error> for Exit {
    fn from(e: std::io::Error) -> Self {
        Exit::Core(Error::Io(e))
    }
}

pub type CmdResult = Result<(), Exit>;

/// 全域選項（`--spine`、`--json`）。
#[derive(Debug, Clone, Default)]
pub struct Ctx {
    pub spine: Option<String>,
    pub json: bool,
}

/// 對應 `_fail`。
pub fn fail(error: &str, message: impl Into<String>, hint: Option<String>, code: i32) -> Exit {
    Exit::Fail {
        error: error.into(),
        message: message.into(),
        hint: hint.filter(|h| !h.is_empty()),
        code,
    }
}

/// `_fail(..., code=EXIT_REJECTED)` 的簡寫。
pub fn reject(error: &str, message: impl Into<String>) -> Exit {
    fail(error, message, None, EXIT_REJECTED)
}

impl Ctx {
    /// 對應 `_spine_dir`：--spine ＞ $REPOENGINE_SPINE ＞ 使用者設定檔（ctx setup 寫的）＞ cwd。
    pub fn spine_dir(&self) -> Result<PathBuf, Exit> {
        if let Some(explicit) = self.spine.as_deref() {
            if !explicit.is_empty() && !config::is_spine(fsutil::expanduser(explicit)) {
                return Err(fail(
                    "no_spine",
                    format!("{explicit} 不是 spine repo（找不到 config.yaml；先跑 init）"),
                    None,
                    EXIT_NO_SPINE,
                ));
            }
        }
        config::find_spine(self.spine.as_deref()).ok_or_else(|| {
            fail(
                "no_spine",
                "找不到 spine repo：用 --spine 指定、設 REPOENGINE_SPINE，\
                 或跑一次 `ctx setup --spine <路徑>` 記住它",
                None,
                EXIT_NO_SPINE,
            )
        })
    }

    /// 對應 `_emit`：--json 印 obj（indent=1）；否則印 human(obj) 回傳的文字（空字串不印）。
    pub fn emit<T: Serialize + ?Sized>(&self, obj: &T, human: impl FnOnce(&T) -> String) {
        if self.json {
            println!("{}", pyjson::dumps_indent(obj));
        } else {
            let text = human(obj);
            if !text.is_empty() {
                println!("{text}");
            }
        }
    }

    /// 只有 JSON 形態的 `_emit(args, obj, None)`。
    pub fn emit_json<T: Serialize + ?Sized>(&self, obj: &T) {
        println!("{}", pyjson::dumps_indent(obj));
    }
}

/// 把一個 [`Exit`] 落地：印該印的、回傳 exit code。
pub fn finish(ctx: &Ctx, r: CmdResult) -> i32 {
    match r {
        Ok(()) => 0,
        Err(Exit::Code(c)) => c,
        Err(Exit::Msg(m)) => {
            eprintln!("{m}");
            1
        }
        Err(Exit::Fail {
            error,
            message,
            hint,
            code,
        }) => {
            if ctx.json {
                let mut out = Map::new();
                out.insert("ok".into(), Value::Bool(false));
                out.insert("error".into(), Value::String(error));
                out.insert("message".into(), Value::String(message));
                if let Some(h) = hint {
                    out.insert("hint".into(), Value::String(h));
                }
                println!("{}", pyjson::dumps(&Value::Object(out)));
                code
            } else {
                match hint {
                    Some(h) => eprintln!("錯誤：{message}\n提示：{h}"),
                    None => eprintln!("錯誤：{message}"),
                }
                1
            }
        }
        Err(Exit::Core(e)) => {
            // Python 沒接住的例外＝traceback＋exit 1；這裡印最後一行就好
            eprintln!("錯誤：{e}");
            1
        }
    }
}

/// 逗號字串 → id 清單（Python 各處 `repos` 參數收逗號字串）。
pub fn split_opt(s: Option<&str>) -> Option<Vec<String>> {
    s.filter(|x| !x.is_empty())
        .map(ctx_core::registry::split_ids)
}

/// Python `f"{x:.0%}"`。
pub fn pct0(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// 引擎錯誤的訊息字串（Python `str(e)`）。
pub fn msg_of(e: &Error) -> String {
    e.to_string()
}

/// Python `except ValueError` 能接到的錯誤種類（Invalid／Validation／Rejected）。
pub fn is_value_error(e: &Error) -> bool {
    matches!(
        e,
        Error::Invalid(_) | Error::Validation(_) | Error::Rejected { .. }
    )
}
