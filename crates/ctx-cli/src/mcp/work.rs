//! 工作層工具的實作：轉交 `ctx_core::agentapi`，回 JSON。

use super::*;

/// Python `_w` 轉交的參數：缺必填就照 Python TypeError 的訊息報錯。
pub(super) fn need(fname: &str, a: &Args, keys: &[&str]) -> Result<()> {
    let missing: Vec<&str> = keys
        .iter()
        .copied()
        .filter(|k| match a.get(*k) {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.is_empty(),
            _ => false,
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let quoted: Vec<String> = missing.iter().map(|k| format!("'{k}'")).collect();
    let list = match quoted.len() {
        1 => quoted[0].clone(),
        2 => format!("{} and {}", quoted[0], quoted[1]),
        n => format!("{}, and {}", quoted[..n - 1].join(", "), quoted[n - 1]),
    };
    Err(Error::External(format!(
        "{fname}() missing {} required positional argument{}: {list}",
        missing.len(),
        if missing.len() == 1 { "" } else { "s" }
    )))
}

/// `_w` 的 cwd 規則：有給就用，沒給用 server 的。
pub(super) fn cwd_of(a: &Args, here: &Path) -> PathBuf {
    arg_s(a, "cwd")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| here.to_path_buf())
}

pub(super) fn target(a: &Args, here: &Path) -> Target {
    Target {
        cwd: Some(cwd_of(a, here)),
        group: arg_s(a, "group").filter(|s| !s.is_empty()),
        repos: arg_repos(a),
    }
}

pub(super) fn nz(a: &Args, k: &str) -> Option<String> {
    arg_s(a, k).filter(|s| !s.is_empty())
}

pub(super) fn w_where(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    Ok(serde_json::to_value(agentapi::where_am_i(
        d,
        Some(&cwd_of(a, here)),
    )?)?)
}
pub(super) fn w_context(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    let card = a
        .get("card")
        .filter(|v| !v.is_null())
        .map(truthy)
        .unwrap_or(true);
    agentapi::context_for(d, &target(a, here), card)
}
pub(super) fn w_next(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    let limit = arg_int(a, "limit").unwrap_or(5).max(0) as usize;
    agentapi::next_work(d, &target(a, here), limit)
}
pub(super) fn w_log(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("log_decision", a, &["text"])?;
    let extra = agentapi::DecisionExtra {
        supersedes: nz(a, "supersedes"),
        reopen_when: nz(a, "reopen_when"),
        outcome_of: nz(a, "outcome_of"),
    };
    agentapi::log_decision_ext(
        d,
        &arg_s(a, "text").unwrap_or_default(),
        &target(a, here),
        nz(a, "ref").as_deref(),
        &extra,
        arg_flag(a, "cross_scope_ok"),
        "agent",
    )
}
pub(super) fn w_add_todo(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("add_todo", a, &["text"])?;
    agentapi::add_todo(
        d,
        &arg_s(a, "text").unwrap_or_default(),
        nz(a, "due").as_deref(),
        &target(a, here),
        arg_flag(a, "cross_scope_ok"),
        "agent",
    )
}
pub(super) fn w_close_todo(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("close_todo", a, &["id"])?;
    agentapi::close_todo(
        d,
        &arg_s(a, "id").unwrap_or_default(),
        nz(a, "note").as_deref(),
        Some(&cwd_of(a, here)),
        arg_flag(a, "cross_scope_ok"),
        "agent",
    )
}
pub(super) fn w_status(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("report_status", a, &["status"])?;
    agentapi::report_status(
        d,
        &arg_s(a, "status").unwrap_or_default(),
        &nz(a, "text").unwrap_or_default(),
        &target(a, here),
    )
}
pub(super) fn w_handoff(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("handoff", a, &["done", "next_step"])?;
    agentapi::handoff(
        d,
        &arg_s(a, "done").unwrap_or_default(),
        &arg_s(a, "next_step").unwrap_or_default(),
        &arg_list(a, "remaining"),
        &target(a, here),
        arg_flag(a, "cross_scope_ok"),
        "agent",
    )
}
pub(super) fn w_search(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("search_knowledge", a, &["query"])?;
    let mut opts = SearchOpts::default();
    if let Some(v) = a.get("everywhere").filter(|v| !v.is_null()) {
        opts.everywhere = truthy(v);
    }
    if let Some(v) = a.get("code").filter(|v| !v.is_null()) {
        opts.code = truthy(v);
    }
    if let Some(l) = arg_int(a, "limit") {
        opts.limit = l.max(0) as usize;
    }
    agentapi::search_knowledge(
        d,
        &arg_s(a, "query").unwrap_or_default(),
        &target(a, here),
        &opts,
    )
}
pub(super) fn w_read(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("read_from", a, &["ref"])?;
    agentapi::read_from(
        d,
        &arg_s(a, "ref").unwrap_or_default(),
        nz(a, "lines").as_deref(),
        Some(&cwd_of(a, here)),
    )
}
pub(super) fn w_ask(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    need("ask_repo", a, &["repo", "question"])?;
    agentapi::ask_repo(
        d,
        &arg_s(a, "repo").unwrap_or_default(),
        &arg_s(a, "question").unwrap_or_default(),
        Some(&cwd_of(a, here)),
        nz(a, "provider").as_deref(),
    )
}
pub(super) fn w_repo_status(d: &Path, a: &Args, here: &Path) -> Result<Value> {
    agentapi::repo_status(
        d,
        nz(a, "repo").as_deref(),
        Some(&cwd_of(a, here)),
        nz(a, "since").as_deref(),
    )
}
