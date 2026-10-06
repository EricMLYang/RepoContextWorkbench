//! repo-context 引擎（Rust 版）。
//!
//! 從 Python `repo_context` 逐模組搬來；資料格式（spine 事件 md、registry.yaml、config.yaml、
//! `.state/` 底下的 JSON）與 MCP 介面是合約，不隨語言改。搬遷規約見 `docs/RUST_PORTING.md`。

pub mod agentmark;
pub mod agents;
pub mod collect;
pub mod error;
pub mod fsutil;
pub mod init;
pub mod knowledge;
pub mod pack;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use error::{Error, Result};
