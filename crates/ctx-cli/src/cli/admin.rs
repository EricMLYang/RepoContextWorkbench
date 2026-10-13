//! 管理：init、registry、組。對應 `repo_context/cli/admin.py`。
//! 寫入一律走操作表（`ctx_core::ops`）——參數驗證、留痕跟工作台／MCP 同一份。
//!
//! 2026-09-25 架構檢查：registry 的十幾個 action 以前共用同一組位置參數（id/path_arg/key/value），
//! `registry set <id> <key> <value>` 實際落在 id/path_arg/key 三格；現在每個 action 一個子命令。

use clap::{Args, Subcommand};
use ctx_core::ops::{self, Via};
use ctx_core::{context, crossref, fsutil, pystr, registry, summary};
use serde_json::{json, Map, Value};

use super::common::{fail, split_opt, CmdResult, Ctx, Exit, EXIT_FINDINGS, EXIT_REJECTED};

#[derive(Args, Debug)]
pub struct InitArgs {
    pub path: String,
}

/// 對應 `cmd_init`。
pub fn cmd_init(a: &InitArgs) -> CmdResult {
    let d = fsutil::expanduser(&a.path);
    ctx_core::init::init_spine(&d)?;
    println!("spine repo 就緒：{}", d.display());
    Ok(())
}

/// 對應 `_op`：跑一個操作——錯誤走 fail（--json 時是 {ok:false}），成功印人話或 JSON。
fn op(ctx: &Ctx, name: &str, args: Value) -> CmdResult {
    let d = ctx.spine_dir()?;
    match ops::call(&d, name, args, Via::Cli) {
        Ok((result, text)) => {
            ctx.emit(&result, |_| text);
            Ok(())
        }
        Err(e) if super::common::is_value_error(&e) => {
            Err(fail("invalid", e.to_string(), None, EXIT_REJECTED))
        }
        Err(e) => Err(e.into()),
    }
}

// ── registry ────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum RegistryCmd {
    /// 登記一個 repo
    Add {
        id: String,
        path: String,
        #[arg(long = "type", default_value = "mine", value_parser = ["mine", "external"])]
        kind: String,
        #[arg(long, default_value = "active")]
        tier: String,
        /// 逗號分隔
        #[arg(long)]
        tags: Option<String>,
        #[arg(long)]
        upstream: Option<String>,
    },
    /// 改 repo 的一個欄位
    Set {
        id: String,
        key: String,
        value: String,
    },
    /// 寫 repo 名片：介紹／協作角色／搜尋關鍵字（存 registry，不寫進 repo；給空字串＝清掉）
    Describe {
        id: String,
        /// 一句話介紹：這個 repo 是什麼、別的 repo 來這裡找什麼
        #[arg(long)]
        description: Option<String>,
        /// 協作角色，一個短詞（例 知識庫、規格來源、實作）
        #[arg(long)]
        role: Option<String>,
        /// 搜尋用關鍵字（逗號分隔）
        #[arg(long)]
        keywords: Option<String>,
    },
    /// 改 tier（paused 可帶 --resume-when）
    Tier {
        id: String,
        tier: String,
        /// paused 的復工條件
        #[arg(long)]
        resume_when: Option<String>,
    },
    /// 移出 registry（含所有組的 membership）
    Remove { id: String },
    /// 加／移除 tag
    Tag {
        id: String,
        /// 加（逗號分隔）
        #[arg(long)]
        add: Option<String>,
        /// 移除（逗號分隔）
        #[arg(long)]
        remove: Option<String>,
    },
    /// 列出 repo
    List,
    /// 掃目錄找未登記的 git repo
    Scan {
        dir: String,
        /// 把候選實際登記（預設只列出）
        #[arg(long)]
        apply: bool,
    },
    /// 稽核：tier 漂移、路徑失效、未登記、文件路徑失效
    Audit {
        /// 額外掃這個目錄找未登記 repo（可重複）
        #[arg(long)]
        scan: Vec<String>,
    },
    /// 設關係 A --kind--> B（例 A pm-of B＝A 是 B 的 PM）
    Relate {
        a: String,
        b: String,
        /// pm-of/feeds/derived-from/upstream-of/sibling-topic（config relations.kinds 可擴充）
        #[arg(long, required = true)]
        kind: String,
        /// 一句說明
        #[arg(long)]
        note: Option<String>,
        /// 這條關係用到 A 的哪些 export（逗號分隔）
        #[arg(long)]
        export: Option<String>,
    },
    /// 刪關係（不給 --kind＝刪 A→B 全部）
    Unrelate {
        a: String,
        b: String,
        #[arg(long)]
        kind: Option<String>,
    },
    /// 讀關係（給 id＝站在該 repo 兩向讀）
    Relations { id: Option<String> },
    /// 建議該設的關係：文件互相提到、引用紀錄、只有兩邊有的主題詞（Rust 才有；只建議，不寫）
    Suggest {
        /// 打掉一筆建議（不相關）：給兩個 repo id，之後不再建議
        #[arg(long, num_args = 2, value_names = ["A", "B"])]
        dismiss: Option<Vec<String>>,
    },
    /// 登記 export：<repo>:<名稱> → repo 內路徑
    Export {
        id: String,
        name: String,
        path: String,
        /// 一句說明
        #[arg(long)]
        note: Option<String>,
    },
    /// 刪 export
    Unexport { id: String, name: String },
    /// 列出 export
    Exports { id: Option<String> },
}

pub fn cmd_registry(ctx: &Ctx, c: &RegistryCmd) -> CmdResult {
    use RegistryCmd::*;
    match c {
        Add {
            id,
            path,
            kind,
            tier,
            tags,
            upstream,
        } => op(
            ctx,
            "registry_add",
            json!({"id": id, "path": path, "type": kind, "tier": tier,
                   "tags": tags, "upstream": upstream}),
        ),
        Describe {
            id,
            description,
            role,
            keywords,
        } => op(
            ctx,
            "registry_describe",
            json!({"id": id, "description": description, "role": role, "keywords": keywords}),
        ),
        Set { id, key, value } => op(
            ctx,
            "registry_set",
            json!({"id": id, "key": key, "value": value}),
        ),
        Tier {
            id,
            tier,
            resume_when,
        } => op(
            ctx,
            "registry_tier",
            json!({"id": id, "tier": tier, "resume_when": resume_when}),
        ),
        Remove { id } => op(ctx, "registry_remove", json!({"id": id})),
        Tag { id, add, remove } => op(
            ctx,
            "registry_tag",
            json!({"id": id, "add": add, "remove": remove}),
        ),
        List => registry_list(ctx),
        Scan { dir, apply } => registry_scan(ctx, dir, *apply),
        Audit { scan } => registry_audit(ctx, scan),
        Relate {
            a,
            b,
            kind,
            note,
            export,
        } => op(
            ctx,
            "registry_relate",
            json!({"a": a, "b": b, "kind": kind, "note": note, "exports": export}),
        ),
        Unrelate { a, b, kind } => op(
            ctx,
            "registry_unrelate",
            json!({"a": a, "b": b, "kind": kind}),
        ),
        Relations { id } => registry_relations(ctx, id.as_deref()),
        Suggest { dismiss } => match dismiss.as_deref() {
            Some([a, b]) => {
                ctx_core::relsuggest::dismiss(&ctx.spine_dir()?, a, b)?;
                ctx.emit(&json!({"ok": true, "dismissed": [a, b]}), |_| {
                    format!("已打掉：{a} ↔ {b}（之後不再建議）")
                });
                Ok(())
            }
            _ => registry_suggest(ctx),
        },
        Export {
            id,
            name,
            path,
            note,
        } => op(
            ctx,
            "registry_export",
            json!({"id": id, "name": name, "path": path, "desc": note}),
        ),
        Unexport { id, name } => op(
            ctx,
            "registry_export",
            json!({"id": id, "name": name, "remove": true}),
        ),
        Exports { id } => registry_exports(ctx, id.as_deref()),
    }
}

/// Python `r.get(k, default)` 當字串（null＝"None"）。
fn field(r: &registry::Repo, k: &str, default: &str) -> String {
    match r.get(k) {
        None => default.to_string(),
        Some(serde_yaml::Value::Null) => "None".into(),
        Some(_) => r.get_str(k).unwrap_or_default(),
    }
}

fn registry_list(ctx: &Ctx) -> CmdResult {
    let d = ctx.spine_dir()?;
    let data = registry::load(&d)?;
    let rows: Vec<Value> = data.repos.iter().map(|r| r.to_json()).collect();
    ctx.emit(&rows, |_| {
        data.repos
            .iter()
            .map(|r| {
                format!(
                    "{:<24} {:<9} {:<9} {}",
                    field(r, "id", ""),
                    field(r, "type", "mine"),
                    field(r, "tier", "?"),
                    field(r, "path", "")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    Ok(())
}

fn registry_scan(ctx: &Ctx, dir: &str, apply: bool) -> CmdResult {
    // mani「init 自動掃描＋手寫補語意」混合模式：預設只列候選，--apply 才登記
    let d = ctx.spine_dir()?;
    let invalid = |e: ctx_core::Error| -> Exit {
        if super::common::is_value_error(&e) {
            fail("invalid", e.to_string(), None, EXIT_REJECTED)
        } else {
            e.into()
        }
    };
    if apply {
        let added = registry::scan_register(&d, dir).map_err(invalid)?;
        for cid in &added {
            println!(
                "已登記：{cid}（下半身預設 mine/active；上半身 type/tier/tags 用 registry set 補）"
            );
        }
        if added.is_empty() {
            println!("無新 repo 可登記");
        }
        return Ok(());
    }
    let cands = registry::scan_dir(&d, dir).map_err(invalid)?;
    for (path, cid) in &cands {
        println!("[候選] {cid}: {}", path.display());
    }
    if cands.is_empty() {
        println!("無未登記 repo");
    } else {
        println!("共 {} 個未登記 git repo（加 --apply 登記）", cands.len());
    }
    Ok(())
}

fn registry_audit(ctx: &Ctx, scan: &[String]) -> CmdResult {
    let d = ctx.spine_dir()?;
    let mut reds = registry::audit(&d, scan)?;
    reds.extend(crossref::lint_doc_refs(&d, None)?.into_iter().map(|b| {
        format!(
            "[文件路徑失效] {}/{}:{} `{}`：{}",
            b.repo, b.doc, b.line, b.reference, b.fix
        )
    }));
    if !reds.is_empty() {
        for r in &reds {
            println!("[RED] {r}");
        }
        return Err(Exit::Code(EXIT_FINDINGS));
    }
    println!("audit 零紅字 OK");
    Ok(())
}

fn registry_suggest(ctx: &Ctx) -> CmdResult {
    let d = ctx.spine_dir()?;
    let rows = ctx_core::relsuggest::compute(&d)?;
    ctx.emit(&json!({"ok": true, "suggestions": rows}), |_| {
        if rows.is_empty() {
            return "沒有建議：找不到互相提到或引用夠多、卻還沒設關係的 repo。".to_string();
        }
        let mut out = vec![format!("關係建議 {} 筆（只建議；確認後用 registry relate 設）：", rows.len())];
        for r in &rows {
            out.push(format!("- {} ↔ {}：{}", r.a, r.b, r.reason()));
            for f in r.files.iter().take(2) {
                out.push(format!("    · {f}"));
            }
            match &r.hint {
                Some((x, y, kind)) => out.push(format!(
                    "    設定：ctx registry relate {x} {y} --kind {kind} --note \"…\""
                )),
                None => {
                    let (x, y) = if r.a_mentions_b >= r.b_mentions_a {
                        (&r.a, &r.b)
                    } else {
                        (&r.b, &r.a)
                    };
                    out.push(format!(
                        "    設定：ctx registry relate {x} {y} --kind <pm-of|feeds|derived-from|upstream-of|sibling-topic> --note \"…\""
                    ));
                }
            }
        }
        out.join("\n")
    });
    Ok(())
}

fn registry_relations(ctx: &Ctx, id: Option<&str>) -> CmdResult {
    println!("{}", crate::text::relations(&ctx.spine_dir()?, id)?);
    Ok(())
}

fn registry_exports(ctx: &Ctx, id: Option<&str>) -> CmdResult {
    let d = ctx.spine_dir()?;
    let data = registry::load(&d)?;
    let mut rows: Vec<Value> = Vec::new();
    for r in &data.repos {
        if id.is_some_and(|i| !i.is_empty() && r.id() != i) {
            continue;
        }
        let Some(serde_yaml::Value::Mapping(m)) = r.get("exports") else {
            continue;
        };
        for (k, v) in m {
            let mut o = Map::new();
            o.insert("repo".into(), json!(r.id()));
            o.insert("name".into(), json!(pystr::py_str_yaml(k)));
            if let Value::Object(vm) = ctx_core::config::yaml_to_json(v) {
                o.extend(vm);
            }
            rows.push(Value::Object(o));
        }
    }
    ctx.emit(&rows, |xs| {
        let lines: Vec<String> = xs
            .iter()
            .map(|x| {
                let s = |k: &str| x.get(k).map(pystr::py_str).unwrap_or_default();
                let desc = match x.get("desc") {
                    Some(v) if crate::truthy(v) => format!("  {}", pystr::py_str(v)),
                    _ => String::new(),
                };
                format!("{}:{:<12} → {}{desc}", s("repo"), s("name"), s("path"))
            })
            .collect();
        if lines.is_empty() {
            "（沒有 export）用 registry export <repo> <名稱> <路徑> 登記".into()
        } else {
            lines.join("\n")
        }
    });
    Ok(())
}

// ── 組 ──────────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug)]
pub enum GroupCmd {
    /// 建組
    Add {
        name: String,
        /// 逗號分隔 repo id
        members: String,
    },
    /// 列出組
    List,
    /// 加入組員（repo 需已登記）
    AddMember {
        name: String,
        /// 逗號分隔 repo id
        members: String,
    },
    /// 移出組（repo 登記保留）
    RemoveMember {
        name: String,
        /// 逗號分隔 repo id
        members: String,
    },
    /// 改名（groups/<組>/ 的料跟著搬）
    Rename { name: String, new_name: String },
    /// 刪組（repo 登記與 groups/<組>/ 的料保留）
    Remove { name: String },
    /// 一句話目標（不給 --text＝只讀）
    Goal {
        name: String,
        /// 這組要達成什麼（一句話）；空字串＝清除
        #[arg(long)]
        text: Option<String>,
    },
    /// 組情境卡（會話開場料同源；省略組名＝全部）
    Context {
        name: Option<String>,
        /// 逗號分隔 repo id（臨時組合）
        members: Option<String>,
    },
    /// 本組工作摘要（工作台首屏同一份，每格帶依據）
    Summary {
        name: Option<String>,
        /// 逗號分隔 repo id（臨時組合）
        members: Option<String>,
    },
}

/// Python `a.name or None`。
fn or_none(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|x| !x.is_empty())
}

pub fn cmd_group(ctx: &Ctx, c: &GroupCmd) -> CmdResult {
    use GroupCmd::*;
    match c {
        Add { name, members } => op(ctx, "group_add", json!({"name": name, "members": members})),
        List => {
            let d = ctx.spine_dir()?;
            let data = registry::load(&d)?;
            let rows: Vec<Value> = data.groups.iter().map(|g| g.to_json()).collect();
            ctx.emit(&rows, |_| {
                data.groups
                    .iter()
                    .map(|g| format!("{:<20} {}", g.name(), g.members().join(",")))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
            Ok(())
        }
        AddMember { name, members } => {
            op(ctx, "group_update", json!({"name": name, "add": members}))
        }
        RemoveMember { name, members } => op(
            ctx,
            "group_update",
            json!({"name": name, "remove": members}),
        ),
        Rename { name, new_name } => op(
            ctx,
            "group_update",
            json!({"name": name, "rename": new_name}),
        ),
        Remove { name } => op(ctx, "group_remove", json!({"name": name})),
        Goal { name, text } => {
            // 目標只有人能寫（機器不替你發明）；不給 --text＝只讀
            match text {
                None => {
                    let d = ctx.spine_dir()?;
                    let g = registry::get_group(&d, name).map_err(|e| {
                        if super::common::is_value_error(&e) {
                            fail("invalid", e.to_string(), None, EXIT_REJECTED)
                        } else {
                            e.into()
                        }
                    })?;
                    println!("{}", g.goal().unwrap_or_else(|| "（尚未登記目標）".into()));
                    Ok(())
                }
                Some(t) => op(ctx, "group_goal", json!({"name": name, "text": t})),
            }
        }
        Context { name, members } => {
            let d = ctx.spine_dir()?;
            let ids = split_opt(or_none(members));
            let text = context::build_context(&d, or_none(name), ids.as_deref(), None, None)?;
            println!("{text}");
            Ok(())
        }
        Summary { name, members } => {
            let d = ctx.spine_dir()?;
            let ids = split_opt(or_none(members));
            let s = summary::build_summary(&d, or_none(name), ids.as_deref(), None, None, None)?;
            ctx.emit(&s, summary::format_summary);
            Ok(())
        }
    }
}
