//! repo-context 引擎（Rust 版）。
//!
//! 從 Python `repo_context` 逐模組搬來；資料格式（spine 事件 md、registry.yaml、config.yaml、
//! `.state/` 底下的 JSON）與 MCP 介面是合約，不隨語言改。搬遷規約見 `docs/RUST_PORTING.md`。

pub mod agentapi;
pub mod agentmark;
pub mod agents;
pub mod agentsetup;
pub mod backfill;
pub mod brief;
pub mod collect;
pub mod collide;
pub mod config;
pub mod context;
pub mod crossref;
pub mod digest;
pub mod error;
pub mod exe;
pub mod filetree;
pub mod fsutil;
pub mod health;
pub mod hooks;
pub mod init;
pub mod knowledge;
pub mod locking;
pub mod notes;
pub mod notify;
pub mod ops;
pub mod pack;
pub mod pyjson;
pub mod pystr;
pub mod registry;
pub mod relsuggest;
pub mod repocard;
pub mod route;
pub mod ruminate;
pub mod session;
pub mod spawn;
pub mod spine;
pub mod status;
pub mod summary;
pub mod term;
pub mod timer;
pub mod upstream;
pub mod workbench;
pub mod yamlout;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use error::{Error, RejectCode, Result};
