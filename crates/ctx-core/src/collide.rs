//! P7 撞 collide——兩段式（v2 檢討④）。對應 Python `repo_context/collide.py`。
//!
//! 段一 [`submit`]：想法原文即刻落脊椎（collision opened，判定掛了想法也不丟），
//! 段二 [`run_judgement`]：pack 組文件層 → provider 判定（五欄位）→ 結果落脊椎（unread 浮出）。
//! `--wait` 同步跑段二；`--detach` 由 [`spawn_detached`] 起獨立進程（不掛呼叫者進程樹）；
//! 判定失敗 → 落 system-unsure 事件（准打斷類，不准沉默）。
//! P7 不准跑在殼的 thread 裡——引擎這邊以獨立進程支持這條。

use std::ffi::OsString;
use std::path::Path;

use chrono::{Local, NaiveDateTime};

use crate::agents::{self, AgentProvider, Judgement};
use crate::pack::{self, RepoRef};
use crate::registry;
use crate::spine::{self, EventType};
use crate::{config, Error, Result};

/// 對應 `collide.PROMPT_TEMPLATE`（`{idea}`、`{packed}` 由 [`prompt`] 代入）。
pub const PROMPT_TEMPLATE: &str = r#"你是「碰撞判定器」。判斷【想法】相對【組內容】是：已知（組內已有）、衝突（與組內既有結論矛盾）、還是真增量（新的、值得落地）。

只輸出一個 JSON 物件，欄位固定：
{"判定": "已知|衝突|真增量", "理由": "一句話", "證據": "哪個 repo 哪份文件的哪段（已知/衝突必填實引；真增量寫『組內無相關段』並點名最接近的文件）", "落點建議": "repo:<id>/01_inbox | group:<g>/materials | incubator", "下一步": "一個具體動作"}

【想法】
{idea}

【組內容（文件層選料，未納入清單見 pack 標頭）】
{packed}
"#;

fn prompt(idea: &str, packed: &str) -> String {
    // 先換 {idea} 再換 {packed}：pack 內容裡出現的大括號字樣不會被再代換
    let (head, tail) = PROMPT_TEMPLATE
        .split_once("{packed}")
        .unwrap_or((PROMPT_TEMPLATE, ""));
    format!("{}{packed}{tail}", head.replacen("{idea}", idea, 1))
}

/// 臨時組合的 group token 前綴（`group:臨時(a,b)`）。
const ADHOC_PREFIX: &str = "臨時(";

/// 對應 `collide._next_collision_id`：當天第一個沒用過的 `<日期>-a`…`-z`，用完改 `-z<n>`。
pub fn next_collision_id(spine_dir: impl AsRef<Path>, date_str: &str) -> Result<String> {
    let used: std::collections::HashSet<String> = spine::iter_events(spine_dir)?
        .iter()
        .filter(|ev| ev.etype == EventType::Collision)
        .filter_map(|ev| ev.kv("id"))
        .filter(|cid| !cid.is_empty() && cid.starts_with(date_str))
        .map(str::to_string)
        .collect();
    for c in 'a'..='z' {
        let cid = format!("{date_str}-{c}");
        if !used.contains(&cid) {
            return Ok(cid);
        }
    }
    Ok(format!("{date_str}-z{}", used.len()))
}

/// 對應 `collide.submit`（段一）：想法落脊椎。回傳 collision id。`source` Python 預設 "hotkey"。
/// `repos` 給定（非空）＝臨時組合（P2 免建組）：token 落 `group:臨時(a,b)`（§3.2 文法範例），
/// detached 進程重讀事件即可解回選料範圍。
pub fn submit(
    spine_dir: impl AsRef<Path>,
    idea: &str,
    group: Option<&str>,
    repos: Option<&[String]>,
    source: &str,
    when: Option<NaiveDateTime>,
) -> Result<String> {
    let spine_dir = spine_dir.as_ref();
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let cid = next_collision_id(spine_dir, &now.format("%Y-%m-%d").to_string())?;
    let mut group = group.filter(|g| !g.is_empty()).map(str::to_string);
    if let Some(ids) = repos.filter(|r| !r.is_empty()) {
        group = Some(format!("{ADHOC_PREFIX}{})", ids.join(",")));
    }
    let mut tokens = vec![format!("id:{cid}")];
    if let Some(g) = &group {
        tokens.push(format!("group:{g}"));
    }
    spine::append_event(
        spine_dir,
        "collision",
        source,
        &tokens,
        &format!("opened\n輸入：{idea}"),
        Some(now),
    )?;
    Ok(cid)
}

/// 對應 `collide.run_judgement`（段二）：判定並落脊椎。回傳判定；provider 沒把握（system-unsure）＝`Ok(None)`
/// （已落事件浮出）。`provider` 缺＝config `provider.default`。找不到 opened 事件＝`Error::Invalid`。
pub fn run_judgement(
    spine_dir: impl AsRef<Path>,
    cid: &str,
    group: Option<&str>,
    repos: Option<&[String]>,
    provider: Option<&str>,
    when: Option<NaiveDateTime>,
) -> Result<Option<Judgement>> {
    let spine_dir = spine_dir.as_ref();
    let cfg = config::load(spine_dir)?;
    let name = provider
        .filter(|p| !p.is_empty())
        .unwrap_or(cfg.provider_default())
        .to_string();
    let p = agents::provider_by_name(&name).ok_or_else(|| {
        Error::invalid(format!(
            "未知 provider: {name}（可用：{}）",
            agents::PROVIDER_NAMES.join(", ")
        ))
    })?;
    run_judgement_with(spine_dir, cid, group, repos, p.as_ref(), when)
}

/// `run_judgement` 的本體，provider 由呼叫端給（測試注入用）。model 讀 config `provider.models.collide`。
pub fn run_judgement_with(
    spine_dir: impl AsRef<Path>,
    cid: &str,
    group: Option<&str>,
    repos: Option<&[String]>,
    provider: &dyn AgentProvider,
    when: Option<NaiveDateTime>,
) -> Result<Option<Judgement>> {
    let spine_dir = spine_dir.as_ref();
    let now = when.unwrap_or_else(|| Local::now().naive_local());
    let cfg = config::load(spine_dir)?;
    let mut group = group.filter(|g| !g.is_empty()).map(str::to_string);
    let mut idea = None;
    for ev in spine::iter_events(spine_dir)? {
        if ev.etype == EventType::Collision
            && ev.kv("id") == Some(cid)
            && ev.body.starts_with("opened")
        {
            let text = ev
                .body
                .split_once("輸入：")
                .map_or(ev.body.as_str(), |x| x.1);
            idea = Some(text.trim().to_string());
            if group.is_none() {
                group = ev.kv("group").filter(|g| !g.is_empty()).map(str::to_string);
            }
        }
    }
    let Some(idea) = idea else {
        return Err(Error::invalid(format!(
            "找不到 collision opened 事件: {cid}"
        )));
    };
    let mut repos: Option<Vec<String>> = repos.map(<[String]>::to_vec);
    if repos.is_none() {
        // 臨時組合：從事件 token 解回 repo 清單
        if let Some(inner) = group
            .as_deref()
            .and_then(|g| g.strip_prefix(ADHOC_PREFIX))
            .and_then(|g| g.strip_suffix(')'))
        {
            repos = Some(registry::split_ids(inner));
            group = None;
        }
    }
    let (_, entries) = registry::resolve_group(spine_dir, group.as_deref(), repos.as_deref())?;
    let budget = usize::try_from(cfg.pack_token_budget()).unwrap_or(0);
    let packed = pack::pack_group(
        &entries.iter().map(RepoRef::from).collect::<Vec<_>>(),
        budget,
    );
    let tokens = [format!("id:{cid}"), format!("ref:collision:{cid}")];
    let j = match agents::judge_with(
        provider,
        &prompt(&idea, &packed.text),
        spine_dir,
        cfg.provider_model("collide"),
    ) {
        Ok(j) => j,
        Err(Error::External(e)) => {
            spine::append_event(
                spine_dir,
                "collision",
                "engine",
                &tokens,
                &format!("判定失敗（system-unsure，准打斷）：{e}"),
                Some(now),
            )?;
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    let body = format!(
        "判定：{}\n理由：{}\n證據：{}\n落點建議：{}\n下一步：{}\n\
         〔照建議落〕〔改落點〕〔升格 incubator〕〔深撞：開 agent 視窗〕〔丟棄並記錄〕",
        j.verdict, j.reason, j.evidence, j.placement, j.next_step
    );
    spine::append_event(spine_dir, "collision", "engine", &tokens, &body, Some(now))?;
    Ok(Some(j))
}

/// detached 進程的參數（不含執行檔本身）：`--spine <dir> collide run <cid> [--group g] [--provider p]`。
/// 跟 Python `python -m repo_context --spine … collide run …` 的子命令與旗標相同，只是執行檔換成 ctx。
pub fn detached_args(
    spine_dir: impl AsRef<Path>,
    cid: &str,
    group: Option<&str>,
    provider: Option<&str>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "--spine".into(),
        spine_dir.as_ref().as_os_str().to_owned(),
        "collide".into(),
        "run".into(),
        cid.into(),
    ];
    if let Some(g) = group.filter(|g| !g.is_empty()) {
        args.push("--group".into());
        args.push(g.into());
    }
    if let Some(p) = provider.filter(|p| !p.is_empty()) {
        args.push("--provider".into());
        args.push(p.into());
    }
    args
}

/// 對應 `collide.spawn_detached`：段二丟獨立進程（見 [`crate::exe::spawn_detached`]），
/// 執行檔＝[`crate::exe::ctx_exe`]（Rust 的 ctx；Python 是 `sys.executable -m repo_context`）。
/// 設了 `REPOCTX_NO_BACKGROUND` 就不起進程、回 `Ok(false)`（Python 版 collide 沒檢查這個；
/// Rust 版刻意尊重，跟 ruminate 的背景進程同一條規則）。起了回 `Ok(true)`。
pub fn spawn_detached(
    spine_dir: impl AsRef<Path>,
    cid: &str,
    group: Option<&str>,
    provider: Option<&str>,
) -> Result<bool> {
    if crate::exe::background_disabled() {
        return Ok(false);
    }
    spawn_detached_with(&crate::exe::ctx_exe()?, spine_dir, cid, group, provider)
}

/// 同 [`spawn_detached`]，執行檔由呼叫端給。不檢查 `REPOCTX_NO_BACKGROUND`（呼叫端自己決定）。
pub fn spawn_detached_with(
    exe: &Path,
    spine_dir: impl AsRef<Path>,
    cid: &str,
    group: Option<&str>,
    provider: Option<&str>,
) -> Result<bool> {
    crate::exe::spawn_detached(exe, detached_args(spine_dir, cid, group, provider))?;
    Ok(true)
}
