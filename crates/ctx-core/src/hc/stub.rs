//! Windows：herdr-core 走 named pipe，這次沒做。[`HcBackend::start`] 一律回「尚未支援」，manager 退回行程內 PTY。
//! 型別是沒有值的 enum：其他方法永遠不會被呼叫到。

use std::path::Path;
use std::sync::Arc;

use super::{Existing, StartError};
use crate::session_status::ProgramRecorder;
use crate::term::{ExitHook, SpawnSpec, TermMeta, TermSession};
use crate::Result;

pub enum HcBackend {}

impl HcBackend {
    pub fn start(
        _spine_dir: &Path,
        _hc_path: Option<&Path>,
        _session: &str,
        _env: Vec<(String, String)>,
    ) -> std::result::Result<Arc<HcBackend>, StartError> {
        Err(StartError {
            reason: "這個平台（Windows）還不支援 herdr-core 後端".into(),
            exe: None,
        })
    }

    pub fn exe(&self) -> &Path {
        match *self {}
    }

    pub fn version(&self) -> String {
        match *self {}
    }

    pub fn spawn(
        self: &Arc<Self>,
        _sid: &str,
        _spec: &SpawnSpec,
        _meta: TermMeta,
        _agent: Option<&str>,
        _hook: ExitHook,
        _program: Option<ProgramRecorder>,
    ) -> Result<Arc<TermSession>> {
        match **self {}
    }

    pub fn existing(&self) -> Vec<Existing> {
        match *self {}
    }

    pub fn adopt(
        self: &Arc<Self>,
        _e: Existing,
        _hook: ExitHook,
        _program: Option<ProgramRecorder>,
    ) -> Option<Arc<TermSession>> {
        match **self {}
    }

    pub fn shutdown(&self) {
        match *self {}
    }
}
