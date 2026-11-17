//! 管理層工具：參數型別（＝inputSchema）與實作（回人話字串；Err＝isError）。
//! 操作表（`ops::ops()`）自動變成的寫入工具不在這裡——參數宣告在 `ctx_core::ops`。

use super::*;

/// 不收參數的工具。
#[derive(Deserialize, JsonSchema)]
pub struct NoParams {}

/// 只收範圍（group／repos）的工具。
#[derive(Deserialize, JsonSchema)]
pub struct ScopeOnly {
    #[serde(flatten)]
    scope: Scope,
}

pub(super) fn t_registry_list(d: &Path, _p: NoParams) -> Result<String> {
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

#[derive(Deserialize, JsonSchema)]
pub struct RegistryAudit {
    scan_dirs: Option<List>,
}
pub(super) fn t_registry_audit(d: &Path, p: RegistryAudit) -> Result<String> {
    let reds = registry::audit(d, &list(&p.scan_dirs))?;
    Ok(if reds.is_empty() {
        "audit 零紅字 OK".into()
    } else {
        reds.iter()
            .map(|r| format!("[RED] {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct RegistryScan {
    dir: Str,
    apply: Option<Flag>,
}
pub(super) fn t_registry_scan(d: &Path, p: RegistryScan) -> Result<String> {
    let dir = p.dir.0;
    if flag(&p.apply) {
        let added = registry::scan_register(d, &dir)?;
        ctx_core::home::sync_quiet(d); // F-132
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

pub(super) fn t_spine_lint(d: &Path, _p: NoParams) -> Result<String> {
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

pub(super) fn t_pack_estimate(d: &Path, p: ScopeOnly) -> Result<String> {
    let (_, entries) = p.scope.resolve(d)?;
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

#[derive(Deserialize, JsonSchema)]
pub struct RegistryRelations {
    id: Option<Str>,
}
pub(super) fn t_registry_relations(d: &Path, p: RegistryRelations) -> Result<String> {
    crate::text::relations(d, text(&p.id).as_deref())
}

pub(super) fn t_refs_check(d: &Path, _p: NoParams) -> Result<String> {
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

pub(super) fn t_group_context(d: &Path, p: ScopeOnly) -> Result<String> {
    let (g, ids) = (p.scope.group(), p.scope.repos());
    context::build_context(d, g.as_deref(), ids.as_deref(), None, None)
}

pub(super) fn t_group_list(d: &Path, _p: NoParams) -> Result<String> {
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

pub(super) fn t_collect(d: &Path, p: ScopeOnly) -> Result<String> {
    let (_, entries) = p.scope.resolve(d)?;
    let specs: Vec<RepoSpec> = entries.iter().map(RepoSpec::from).collect();
    Ok(collect::format_table(&collect::collect_group(
        &specs,
        Some(d),
    )))
}

#[derive(Deserialize, JsonSchema)]
pub struct Pack {
    #[serde(flatten)]
    scope: Scope,
    budget: Option<Int>,
}
pub(super) fn t_pack(d: &Path, p: Pack) -> Result<String> {
    let (_, entries) = p.scope.resolve(d)?;
    let refs: Vec<RepoRef> = entries.iter().map(RepoRef::from).collect();
    let budget = match int(&p.budget) {
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

pub(super) fn t_brief(d: &Path, p: ScopeOnly) -> Result<String> {
    let (g, ids) = (p.scope.group(), p.scope.repos());
    Ok(brief::run(d, g.as_deref(), ids.as_deref(), None)?.1)
}

#[derive(Deserialize, JsonSchema)]
pub struct SpineAppend {
    r#type: Str,
    source: Str,
    kv: Option<List>,
    body: Option<Str>,
}
pub(super) fn t_spine_append(d: &Path, p: SpineAppend) -> Result<String> {
    let body = text(&p.body).unwrap_or_default();
    // open-loop 沒帶 due 就沒有期限（刻意差異：不再補預設天數，見 RUST_PORTING）
    let tokens = list(&p.kv);
    let ev = spine::append_event(d, &p.r#type.0, &p.source.0, &tokens, &body, None)?;
    Ok(format!("已寫入：{}", ev.header()))
}

#[derive(Deserialize, JsonSchema)]
pub struct SpineQuery {
    r#type: Option<Str>,
    date: Option<Str>,
    today: Option<Flag>,
    group: Option<Str>,
    verbose: Option<Flag>,
}
pub(super) fn t_spine_query(d: &Path, p: SpineQuery) -> Result<String> {
    let date = if flag(&p.today) {
        Some(Local::now().format("%Y-%m-%d").to_string())
    } else {
        text(&p.date)
    };
    let evs = crate::cli::daily::query_events(d, text(&p.r#type).as_deref(), date, text(&p.group))?;
    Ok(ev_lines(&evs, flag(&p.verbose)))
}

pub(super) fn t_spine_stats(d: &Path, _p: NoParams) -> Result<String> {
    let s = spine::stats(d)?;
    let sv = registry::survival(d)?;
    let rate = s.hit_rate.map(pct0).unwrap_or_else(|| "n/a".into());
    let srate = sv.survival_rate.map(pct0).unwrap_or_else(|| "n/a".into());
    Ok(format!(
        "碰撞 {} 次｜outcome 回連 {} 次｜靈感命中率 {rate}｜生出 repo {} 個｜存活 {}｜存活率 {srate}",
        s.collisions, s.collisions_with_outcome, sv.spawned, sv.alive
    ))
}

pub(super) fn t_open_loops(d: &Path, p: ScopeOnly) -> Result<String> {
    // 範圍跟簡報／情境卡同一份規則（以前這裡回全部組的未結）
    let mut evs = spine::open_loops(d)?;
    let (g, ids) = (p.scope.group(), p.scope.repos());
    if g.as_deref().is_some_and(|x| !x.is_empty()) || ids.is_some() {
        let (_, entries) = p.scope.resolve(d)?;
        let ids: Vec<String> = entries.iter().map(|e| e.id()).collect();
        evs.retain(|ev| spine::in_scope(ev, g.as_deref(), &ids));
    }
    Ok(ev_lines(&evs, true))
}

#[derive(Deserialize, JsonSchema)]
pub struct Unread {
    ack: Option<Flag>,
}
pub(super) fn t_unread(d: &Path, p: Unread) -> Result<String> {
    let pending = notify::pending(d)?;
    let all: Vec<spine::Event> = pending
        .interrupt
        .iter()
        .chain(pending.normal.iter())
        .cloned()
        .collect();
    let mut text = format!("{}\n{}", notify::render(&pending), ev_lines(&all, false));
    if flag(&p.ack) {
        spine::ack_unread(d, None)?;
        text.push_str("\n（已標記全部已讀）");
    }
    Ok(text)
}

#[derive(Deserialize, JsonSchema)]
pub struct CollideSubmit {
    idea: Str,
    #[serde(flatten)]
    scope: Scope,
    wait: Option<Flag>,
    provider: Option<Str>,
}
pub(super) fn t_collide_submit(d: &Path, p: CollideSubmit) -> Result<String> {
    let (g, ids) = (p.scope.group(), p.scope.repos());
    let provider = text(&p.provider);
    let cid = collide::submit(d, &p.idea.0, g.as_deref(), ids.as_deref(), "agent", None)?;
    if flag(&p.wait) {
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

#[derive(Deserialize, JsonSchema)]
pub struct Route {
    text: Str,
    dest: Str,
    title: Option<Str>,
}
pub(super) fn t_route(d: &Path, p: Route) -> Result<String> {
    let title = text(&p.title).unwrap_or_else(|| "idea".into());
    let out = route::route(d, &p.text.0, &p.dest.0, &title, None)?;
    Ok(format!("已落：{}", out.display()))
}

pub(super) fn t_upstream_check(d: &Path, p: ScopeOnly) -> Result<String> {
    let (g, ids) = (p.scope.group(), p.scope.repos());
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

#[derive(Deserialize, JsonSchema)]
pub struct DigestRun {
    #[serde(flatten)]
    scope: Scope,
    provider: Option<Str>,
}
pub(super) fn t_digest_run(d: &Path, p: DigestRun) -> Result<String> {
    let (g, ids) = (p.scope.group(), p.scope.repos());
    let results = digest::run(
        d,
        g.as_deref(),
        ids.as_deref(),
        text(&p.provider).as_deref(),
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

#[derive(Deserialize, JsonSchema)]
pub struct Spawn {
    id: Str,
    path: Str,
    from_incubator: Option<Str>,
    origin: Option<Str>,
}
pub(super) fn t_spawn(d: &Path, p: Spawn) -> Result<String> {
    let id = &p.id.0;
    let out = spawn::spawn(
        d,
        id,
        &p.path.0,
        text(&p.from_incubator).as_deref(),
        text(&p.origin).as_deref(),
        None,
    )?;
    Ok(format!("已升格：{id} → {}", out.display()))
}
