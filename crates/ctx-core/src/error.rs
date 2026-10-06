//! 引擎統一錯誤。入口（CLI／MCP／桌面）各自把它轉成自己的形狀。

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 資料不合文法（spine 標頭、kv token…）——對應 Python `spine.ValidationError`。
    #[error("{0}")]
    Validation(String),
    /// 參數或狀態不合法——對應 Python 各處的 `ValueError`。
    #[error("{0}")]
    Invalid(String),
    /// 寫到主場以外被拒——對應 agentapi 的 cross_scope。
    #[error("{message}")]
    Rejected { code: String, message: String, hint: Option<String> },
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
    pub fn rejected(code: impl Into<String>, msg: impl Into<String>, hint: Option<String>) -> Self {
        Error::Rejected { code: code.into(), message: msg.into(), hint }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
