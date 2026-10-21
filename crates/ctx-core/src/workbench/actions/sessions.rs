//! 會話（PTY）相關的 action：開會話（含接續過去的會話）、殺會話、看過了、交接草稿、記結果。
//! 2026-10-16 從 `mod.rs` 拆出來（檔案超過 800 行依職責拆，見 `docs/DEVELOPMENT.md` §2）；行為不變。

use super::*;

pub(super) fn a_session_draft(d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let sid = raw(p, "sid").unwrap_or_default();
    let Some(info) = t.and_then(|t| t.info(&sid)) else {
        return Err(Error::invalid(format!("未知會話: {}", shown(p, "sid"))));
    };
    let mut out = Map::new();
    out.insert("ok".into(), json!(true));
    if let Value::Object(m) = session_draft(d, &term_value(&info))? {
        out.extend(m);
    }
    Ok(Value::Object(out))
}

/// 對單一 repo 開的會話屬於哪個牌組（只給畫面歸類，Rust 才有，2026-10-13 從前端搬來）：
/// `hint`（前端目前選的組）有這個 repo 就是它；否則只有一個組有它才算；都不成立就沒有。不是對單一 repo 開就照 `hint`。
fn home_group_for(d: &Path, hint: Option<String>, repo: Option<&str>) -> Result<Option<String>> {
    let Some(repo) = repo else {
        return Ok(hint);
    };
    let owners: Vec<String> = registry::load(d)?
        .groups
        .iter()
        .filter(|g| g.members().iter().any(|m| m == repo))
        .map(|g| g.name())
        .collect();
    Ok(match hint {
        Some(h) if owners.contains(&h) => Some(h),
        _ if owners.len() == 1 => owners.into_iter().next(),
        _ => None,
    })
}

pub(super) fn a_term_create(d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let Some(terms) = t else {
        return Err(Error::invalid("本 server 未啟用 terminal"));
    };
    let kind = raw(p, "kind").unwrap_or_else(|| "shell".into());
    let resume = s(p, "resume");
    let flags = launch_flags(p, resume.is_some())?;
    let (sid, title) = if let (true, Some(id)) = (kind == "agent", resume) {
        // F-72：接續過去的 Claude Code 會話——在它原本的目錄跑 `claude --resume <id>`（目錄要在登記的 repo 底下）
        let agent = s(p, "agent").unwrap_or_else(|| "claude".into());
        let (rid, cwd) = crate::history::resume_target(d, &id, &s(p, "cwd").unwrap_or_default())?;
        let (mut argv, at) = session::resume_argv(d, &agent, &id, s(p, "prompt").as_deref())?;
        let file =
            crate::exe::ctx_exe().and_then(|exe| crate::session_status::settings_file(d, &exe));
        if let Ok(f) = file {
            argv.insert(at, format!("--settings={}", f.display()));
        }
        argv.splice(at..at, flags.argv());
        let home = home_group_for(d, s(p, "home_group"), Some(&rid))?;
        terms.create_agent(AgentLaunch {
            agent,
            argv,
            cwd,
            env: Vec::new(),
            group: None,
            repos: Some(vec![rid.clone()]),
            repo: Some(rid),
            task: s(p, "task"),
            origin: Some(format!("resume:{id}")),
            home_group: home,
        })?
    } else if kind == "agent" {
        let group = s(p, "group");
        let repos = list(p, "repos");
        let repo = s(p, "repo");
        let task = s(p, "task");
        let agent = s(p, "agent").unwrap_or_else(|| "claude".into());
        let home = home_group_for(d, s(p, "home_group"), repo.as_deref())?;
        // 旗標不合就在寫會話料、留痕之前拒絕
        if !flags.is_empty() && !session::is_claude_code(d, &agent)? {
            return Err(Error::invalid(format!(
                "{agent} 不是 Claude Code：權限模式、模型、worktree 只有 Claude Code 支援"
            )));
        }
        if flags.worktree.is_some() && repo.is_none() {
            return Err(Error::invalid(
                "隔離的 worktree 只能對單一 repo 開（組的會話跑在 spine 目錄）",
            ));
        }
        let plan = session::build(
            d,
            SessionOpts {
                group: group.as_deref(),
                repos: repos.as_deref(),
                repo: repo.as_deref(),
                task: task.as_deref(),
                agent: Some(&agent),
                home_group: home.as_deref(),
            },
        )?;
        // 會話狀態（Rust 才有）：Claude Code 多帶一份只有三個 hook 的設定（做完／要你允許／你送出），
        // 不動使用者的 ~/.claude/settings.json；寫不出設定檔就照舊開（只是看不到「等你」）
        let mut argv = plan.cmd;
        if plan.claude_code && plan.flags_at <= argv.len() {
            argv.splice(plan.flags_at..plan.flags_at, flags.argv());
            let file =
                crate::exe::ctx_exe().and_then(|exe| crate::session_status::settings_file(d, &exe));
            if let Ok(f) = file {
                // 插在 agent 指令之後（config 可能是 ["npx", "claude"] 這種包裝）
                argv.insert(plan.flags_at, format!("--settings={}", f.display()));
            }
        }
        terms.create_agent(AgentLaunch {
            agent,
            argv,
            cwd: plan.cwd,
            env: Vec::new(),
            group,
            repos,
            repo,
            task,
            origin: s(p, "origin"),
            home_group: home,
        })?
    } else if let Some(of) = s(p, "of") {
        // 某個會話的 Terminal：開在同一個目錄，origin 記 `shell-of:<sid>`，畫面據此配對切換
        let cwd = terms
            .cwd(&of)
            .ok_or_else(|| Error::invalid(format!("未知會話: {of}")))?;
        terms.create_shell(Some(&cwd), Some(format!("shell-of:{of}")))?
    } else {
        let cwd = s(p, "cwd").map(PathBuf::from);
        terms.create_shell(cwd.as_deref(), s(p, "origin"))?
    };
    Ok(json!({"ok": true, "sid": sid, "title": title}))
}

/// F-96／F-97／F-108：開會話的選填旗標。`worktree: true`＝自動取名 `ctx-月日-時分秒`（同一分鐘開兩個也不撞）；`fork` 只對接續有效。
fn launch_flags(p: &Value, resuming: bool) -> Result<session::LaunchFlags> {
    let worktree = match p.get("worktree") {
        Some(Value::Bool(true)) => Some(format!(
            "ctx-{}",
            chrono::Local::now().format("%m%d-%H%M%S")
        )),
        Some(Value::String(w)) => Some(w.clone()),
        _ => None,
    };
    let fork = p.get("fork") == Some(&Value::Bool(true));
    if fork && !resuming {
        return Err(Error::invalid("分岔要從一場既有的會話岔出（缺 resume）"));
    }
    if worktree.is_some() && resuming {
        return Err(Error::invalid(
            "接續舊會話時不能另開 worktree（它原本在哪就回哪）",
        ));
    }
    session::LaunchFlags::new(
        s(p, "permission_mode").as_deref(),
        s(p, "model").as_deref(),
        worktree.as_deref(),
        fork,
    )
}

pub(super) fn a_term_kill(_d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let sid = raw(p, "sid").unwrap_or_default();
    if !t.is_some_and(|t| t.kill(&sid)) {
        return Err(Error::invalid(format!("未知會話: {}", shown(p, "sid"))));
    }
    Ok(ok())
}

/// 你看過這個會話了（F-80）：做完沒看→看過了。沒狀態檔或已記過＝沒事，不當錯誤（回 `{ok, changed}`）。
pub(super) fn a_session_seen(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let sid = raw(p, "sid").unwrap_or_default();
    if sid.is_empty() {
        return Err(Error::invalid("缺 sid"));
    }
    let changed = crate::session_status::mark_seen(d, &sid);
    Ok(json!({"ok": true, "changed": changed}))
}

pub(super) fn a_session_result(d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let sid = raw(p, "sid").unwrap_or_default();
    let session = t.and_then(|t| t.info(&sid));
    let text = raw(p, "text").unwrap_or_default().trim().to_string();
    let (Some(session), false) = (session, text.is_empty()) else {
        return Err(Error::invalid("請選擇會話並填寫結果"));
    };
    let origin = session.origin.clone().unwrap_or_default();
    let mut tokens: Vec<String> = if origin.starts_with("collision:") {
        vec![format!("ref:{origin}")]
    } else {
        Vec::new()
    };
    let reg = registry::load(d)?;
    let scope = session.scope.clone().unwrap_or_default();
    if reg.group(&scope).is_some() {
        tokens.push(format!("group:{scope}"));
    } else if reg.repo(&scope).is_some() {
        tokens.push(format!("repo:{scope}"));
    } else if let Some(m) = session.scope_repos.as_ref().filter(|m| !m.is_empty()) {
        tokens.push(format!("group:臨時({})", m.join(",")));
    }
    // A session note is a decision record, not evidence of an adopted idea.
    let ev = spine::append_event(
        d,
        "decision",
        "monitor",
        &tokens,
        &format!("會話結果：{}\n{text}", session.title),
        None,
    )?;
    Ok(json!({"ok": true, "date": ev.date, "time": ev.time}))
}

/// F-98（Rust 才有）：開、改或關這個會話的自動繼續。`off: true`＝關；否則開（`max` 次、`prompt` 那句，計數歸零）。
/// 只對還活著的 agent 會話（引擎查得到這個 sid 時）。
pub(super) fn a_auto_continue(d: &Path, p: &Value, t: Option<&dyn Terms>) -> Result<Value> {
    let sid = raw(p, "sid").unwrap_or_default();
    if let Some(info) = t.and_then(|t| t.info(&sid)) {
        if !info.alive || info.kind != crate::term::TermKind::Agent {
            return Err(Error::invalid("只有執行中的 agent 會話能自動繼續"));
        }
    } else if t.is_some() {
        return Err(Error::invalid(format!("未知會話: {}", shown(p, "sid"))));
    }
    if p.get("off") == Some(&Value::Bool(true)) {
        crate::auto_continue::off(d, &sid);
        return Ok(json!({"ok": true, "auto_continue": null}));
    }
    let max = match p.get("max") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| Error::invalid("max 必須是正整數"))?,
        ),
    };
    let a = crate::auto_continue::set(d, &sid, max, s(p, "prompt").as_deref())?;
    Ok(json!({"ok": true, "auto_continue": a}))
}
