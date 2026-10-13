//! 管理層工具的實作（回人話字串；Err＝isError）。

use super::*;

// ---------------------------------------------------------------- 管理層 handlers（回字串；Err＝isError）

pub(super) fn t_registry_list(d: &Path, _a: &Args) -> Result<String> {
    let rows: Vec<String> = registry::load(d)?
        .repos
        .iter()
        .map(|r| {
            format!(
                "{}\t{}\t{}\t{}",
                r.id(),
                r.kind().unwrap_or_else(|| "mine".into()),
                r.tier_str().unwrap_or_else(|| "?".into()),
                r.path_str()
            )
        })
        .collect();
    Ok(if rows.is_empty() {
        "（registry 空）".into()
    } else {
        rows.join("\n")
    })
}

pub(super) fn t_registry_audit(d: &Path, a: &Args) -> Result<String> {
    let reds = registry::audit(d, &arg_list(a, "scan_dirs"))?;
    Ok(if reds.is_empty() {
        "audit 零紅字 OK".into()
    } else {
        reds.iter()
            .map(|r| format!("[RED] {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

pub(super) fn t_registry_scan(d: &Path, a: &Args) -> Result<String> {
    let dir = arg_s(a, "dir").ok_or_else(|| Error::invalid("'dir'"))?;
    if arg_flag(a, "apply") {
        let added = registry::scan_register(d, &dir)?;
        return Ok(if added.is_empty() {
            "無新 repo 可登記".into()
        } else {
            format!(
                "已登記：{}（上半身 type/tier/tags 待人補）",
                added.join(", ")
            )
        });
    }
    let cands = registry::scan_dir(d, &dir)?;
    Ok(if cands.is_empty() {
        "無未登記 repo".into()
    } else {
        cands
            .iter()
            .map(|(p, cid)| format!("[候選] {cid}: {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

pub(super) fn t_spine_lint(d: &Path, _a: &Args) -> Result<String> {
    let reds = spine::lint(d, None)?;
    Ok(if reds.is_empty() {
        "脊椎 lint 零紅字 OK（衛生迴圈）".into()
    } else {
        reds.iter()
            .map(|r| format!("[RED] {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

pub(super) fn t_pack_estimate(d: &Path, a: &Args) -> Result<String> {
    let (_, entries) = resolve(d, a)?;
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let mut rows = pack::estimate(&refs);
    let total: usize = rows.iter().map(|r| r.tokens).sum();
    let budget = config::load(d)?.pack_token_budget();
    rows.sort_by_key(|r| std::cmp::Reverse(r.tokens));
    let mut lines: Vec<String> = rows
        .iter()
        .map(|r| format!("{}\t{} 檔\t~{} tokens", r.id, r.files, r.tokens))
        .collect();
    lines.push(format!(
        "合計 ~{total} tokens／預算 {budget} → {}",
        if (total as i64) <= budget {
            "裝得下"
        } else {
            "超預算（挑細一點）"
        }
    ));
    Ok(lines.join("\n"))
}

pub(super) fn t_registry_relations(d: &Path, a: &Args) -> Result<String> {
    crate::text::relations(d, arg_s(a, "id").as_deref())
}

pub(super) fn t_refs_check(d: &Path, _a: &Args) -> Result<String> {
    let crossref::RefsCheck {
        broken,
        drift: dr,
        suggest: sug,
    } = crossref::check(d, None, 3)?;
    let mut out: Vec<String> = broken
        .iter()
        .map(|b| {
            format!(
                "[失效路徑] {}/{}:{} `{}` → {}",
                b.repo, b.doc, b.line, b.reference, b.fix
            )
        })
        .collect();
    // Python `f"[上游有變]{ln[1:]}"`：去掉 drift 行開頭的一個字元
    out.extend(crossref::drift_lines(&dr, 50).iter().map(|ln| {
        let mut cs = ln.chars();
        cs.next();
        format!("[上游有變]{}", cs.as_str())
    }));
    out.extend(sug.iter().map(|s| {
        format!(
            "[建議 export] {}:{} 被引用 {} 次（{}）→ {}",
            s.repo,
            s.path,
            s.cites,
            s.consumers.join(", "),
            s.command
        )
    }));
    Ok(if out.is_empty() {
        "跨 repo 參考零紅字 OK".into()
    } else {
        out.join("\n")
    })
}

pub(super) fn t_group_context(d: &Path, a: &Args) -> Result<String> {
    let (g, ids) = scope_ids(a);
    context::build_context(d, g.as_deref(), ids.as_deref(), None, None)
}

pub(super) fn t_group_list(d: &Path, _a: &Args) -> Result<String> {
    let rows: Vec<String> = registry::load(d)?
        .groups
        .iter()
        .map(|g| format!("{}\t{}", g.name(), g.members().join(",")))
        .collect();
    Ok(if rows.is_empty() {
        "（無組）".into()
    } else {
        rows.join("\n")
    })
}

pub(super) fn t_collect(d: &Path, a: &Args) -> Result<String> {
    let (_, entries) = resolve(d, a)?;
    let specs: Vec<RepoSpec> = entries.iter().map(RepoSpec::from).collect();
    Ok(collect::format_table(&collect::collect_group(
        &specs,
        Some(d),
    )))
}

pub(super) fn t_pack(d: &Path, a: &Args) -> Result<String> {
    let (_, entries) = resolve(d, a)?;
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let budget = match arg_int(a, "budget") {
        Some(b) if b != 0 => b,
        _ => config::load(d)?.pack_token_budget(),
    };
    let p = pack::pack_group(&refs, budget.max(0) as usize);
    Ok(format!(
        "<!-- 收錄 {}、未納入 {}、疑似機密擋下 {} -->\n{}",
        p.included.len(),
        p.excluded.len(),
        p.flagged.len(),
        p.text
    ))
}

pub(super) fn t_brief(d: &Path, a: &Args) -> Result<String> {
    let (g, ids) = scope_ids(a);
    Ok(brief::run(d, g.as_deref(), ids.as_deref(), None)?.1)
}

pub(super) fn t_spine_append(d: &Path, a: &Args) -> Result<String> {
    let etype = arg_s(a, "type").ok_or_else(|| Error::invalid("'type'"))?;
    let source = arg_s(a, "source").ok_or_else(|| Error::invalid("'source'"))?;
    let body = arg_s(a, "body").unwrap_or_default();
    let mut tokens = arg_list(a, "kv");
    if etype == "open-loop" {
        let due_days = config::load(d)?.thresholds().openloop_default_due_days;
        tokens = spine::default_due_tokens(&tokens, &body, due_days, None);
    }
    let ev = spine::append_event(d, &etype, &source, &tokens, &body, None)?;
    Ok(format!("已寫入：{}", ev.header()))
}

pub(super) fn t_spine_query(d: &Path, a: &Args) -> Result<String> {
    let date = if arg_flag(a, "today") {
        Some(Local::now().format("%Y-%m-%d").to_string())
    } else {
        arg_s(a, "date")
    };
    let evs =
        crate::cli::daily::query_events(d, arg_s(a, "type").as_deref(), date, arg_s(a, "group"))?;
    Ok(ev_lines(&evs, arg_flag(a, "verbose")))
}

pub(super) fn t_spine_stats(d: &Path, _a: &Args) -> Result<String> {
    let s = spine::stats(d)?;
    let sv = registry::survival(d)?;
    let rate = s.hit_rate.map(pct0).unwrap_or_else(|| "n/a".into());
    let srate = sv.survival_rate.map(pct0).unwrap_or_else(|| "n/a".into());
    Ok(format!(
        "碰撞 {} 次｜outcome 回連 {} 次｜靈感命中率 {rate}｜生出 repo {} 個｜存活 {}｜存活率 {srate}",
        s.collisions, s.collisions_with_outcome, sv.spawned, sv.alive
    ))
}

pub(super) fn t_open_loops(d: &Path, a: &Args) -> Result<String> {
    // 範圍跟簡報／情境卡同一份規則（以前這裡回全部組的未結）
    let mut evs = spine::open_loops(d)?;
    let (g, ids) = scope_ids(a);
    if g.as_deref().is_some_and(|x| !x.is_empty()) || ids.is_some() {
        let (_, entries) = resolve(d, a)?;
        let ids: Vec<String> = entries.iter().map(|e| e.id()).collect();
        evs.retain(|ev| spine::in_scope(ev, g.as_deref(), &ids));
    }
    Ok(ev_lines(&evs, true))
}

pub(super) fn t_unread(d: &Path, a: &Args) -> Result<String> {
    let p = notify::pending(d)?;
    let all: Vec<spine::Event> = p.interrupt.iter().chain(p.normal.iter()).cloned().collect();
    let mut text = format!("{}\n{}", notify::render(&p), ev_lines(&all, false));
    if arg_flag(a, "ack") {
        spine::ack_unread(d, None)?;
        text.push_str("\n（已標記全部已讀）");
    }
    Ok(text)
}

pub(super) fn t_collide_submit(d: &Path, a: &Args) -> Result<String> {
    let idea = arg_s(a, "idea").ok_or_else(|| Error::invalid("'idea'"))?;
    let (g, ids) = scope_ids(a);
    let provider = arg_s(a, "provider");
    let cid = collide::submit(d, &idea, g.as_deref(), ids.as_deref(), "agent", None)?;
    if arg_flag(a, "wait") {
        let j = collide::run_judgement(d, &cid, None, None, provider.as_deref(), None)?;
        return Ok(format!(
            "cid={cid}\n{}",
            match j {
                Some(j) => pyjson::dumps_indent(&j),
                None => "判定失敗（system-unsure）——已落脊椎浮出".into(),
            }
        ));
    }
    collide::spawn_detached(d, &cid, None, provider.as_deref())?;
    Ok(format!("cid={cid}（判定走 detached，結果看 unread）"))
}

pub(super) fn t_route(d: &Path, a: &Args) -> Result<String> {
    let text = arg_s(a, "text").ok_or_else(|| Error::invalid("'text'"))?;
    let dest = arg_s(a, "dest").ok_or_else(|| Error::invalid("'dest'"))?;
    let title = arg_s(a, "title").unwrap_or_else(|| "idea".into());
    let p = route::route(d, &text, &dest, &title, None)?;
    Ok(format!("已落：{}", p.display()))
}

pub(super) fn t_upstream_check(d: &Path, a: &Args) -> Result<String> {
    let (g, ids) = scope_ids(a);
    let (n, findings) = upstream::check(d, g.as_deref(), ids.as_deref(), None)?;
    if n == 0 {
        return Ok("無 external repo 可查（registry 的 external 需帶 upstream 欄位）".into());
    }
    Ok(if findings.is_empty() {
        format!("查了 {n} 個 external，上游無新事")
    } else {
        findings
            .iter()
            .map(|f| format!("{}: {} {} {}", f.repo, f.kind, f.id, f.title))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

pub(super) fn t_digest_run(d: &Path, a: &Args) -> Result<String> {
    let (g, ids) = scope_ids(a);
    let results = digest::run(
        d,
        g.as_deref(),
        ids.as_deref(),
        arg_s(a, "provider").as_deref(),
        None,
    )?;
    let lines: Vec<String> = results
        .iter()
        .map(|r| match r.file.as_deref().filter(|f| !f.is_empty()) {
            Some(f) => format!("{}: {} → {f}", r.repo, r.note),
            None => format!("{}: {}", r.repo, r.note),
        })
        .collect();
    Ok(if lines.is_empty() {
        "（組內無 mine repo）".into()
    } else {
        lines.join("\n")
    })
}

pub(super) fn t_spawn(d: &Path, a: &Args) -> Result<String> {
    let id = arg_s(a, "id").ok_or_else(|| Error::invalid("'id'"))?;
    let path = arg_s(a, "path").ok_or_else(|| Error::invalid("'path'"))?;
    let p = spawn::spawn(
        d,
        &id,
        &path,
        arg_s(a, "from_incubator").as_deref(),
        arg_s(a, "origin").as_deref(),
        None,
    )?;
    Ok(format!("已升格：{id} → {}", p.display()))
}
