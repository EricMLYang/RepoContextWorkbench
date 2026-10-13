//! 引擎統一錯誤。入口（CLI／MCP／桌面）各自把它轉成自己的形狀。

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 資料不合文法（spine 標頭、kv token…）——對應 Python `spine.ValidationError`。
    #[error("{0}")]
    Validation(String),
    /// 參數或狀態不合法——對應 Python 各處的 `ValueError`。
    #[error("{0}")]
    Invalid(String),
    /// 給 agent 的結構化拒絕（MCP 回 `{ok:false, error: code, message, hint}`）——對應 agentapi 的 AgentError。
    #[error("{message}")]
    Rejected {
        code: RejectCode,
        message: String,
        hint: Option<String>,
    },
    #[error("lock timeout: {0}")]
    LockTimeout(String),
    /// 外部指令（git／gh／claude）失敗或沒把握——對應 Python `AgentUnsure` 等。
    #[error("{0}")]
    External(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Yaml(#[from] serde_yaml::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn invalid(msg: impl Into<String>) -> Self {
        Error::Invalid(msg.into())
    }
    pub fn rejected(code: RejectCode, msg: impl Into<String>, hint: Option<String>) -> Self {
        Error::Rejected {
            code,
            message: msg.into(),
            hint,
        }
    }
}

/// 拒絕的種類：agent 依此決定怎麼補救。字串形式（[`RejectCode::as_str`]）是 MCP 合約，不能改。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectCode {
    /// 參數不合法。
    BadRequest,
    /// 寫到主場以外（要先問使用者，再帶 cross_scope_ok）。
    CrossScope,
    /// 引用的東西不存在。
    NotFound,
    /// 內容像憑證，不給讀。
    Secret,
    /// 事件不合文法。
    InvalidEvent,
    /// 外部動作失敗（例：讀檔、git）。
    Failed,
}

impl RejectCode {
    pub fn as_str(self) -> &'static str {
        match self {
            RejectCode::BadRequest => "bad_request",
            RejectCode::CrossScope => "cross_scope",
            RejectCode::NotFound => "not_found",
            RejectCode::Secret => "secret",
            RejectCode::InvalidEvent => "invalid_event",
            RejectCode::Failed => "failed",
        }
    }
}

impl std::fmt::Display for RejectCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
