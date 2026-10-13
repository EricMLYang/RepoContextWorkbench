//! 交給 agent 的任務文字（2026-10-14，Rust 才有）：挑好的檔 `pick_brief`、起草名片 `profile_draft_brief`、
//! 審查變更 `review_brief`。都只讀，組好文字給前端開會話預填。

use super::*;

/// Rust 才有（2026-10-14，痛點 2「挑幾份文件丟給 agent」）：從檔案樹挑的檔案 → 會話任務文字＋`@路徑`。
/// 每個路徑都要在它的 repo 裡（同檢視器的檢查）；重複的只算一次、照挑的順序依 repo 分段，
/// 段落標題帶名片（角色、介紹），agent 不用再問「這是哪個 repo」。
#[derive(serde::Deserialize)]
struct PickBriefArgs {
    files: Vec<PickedFile>,
}

#[derive(serde::Deserialize)]
struct PickedFile {
    id: String,
    path: String,
}

/// 一次最多交幾個檔（再多就該用組情境卡或任務包，不是逐一挑）。
const PICK_MAX: usize = 50;

pub(super) fn a_pick_brief(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let PickBriefArgs { files } = parse("pick_brief", p)?;
    if files.is_empty() {
        return Err(Error::invalid("還沒挑任何檔案"));
    }
    if files.len() > PICK_MAX {
        return Err(Error::invalid(format!(
            "一次最多 {PICK_MAX} 個檔案（挑了 {} 個）；要整組的脈絡請用組情境卡",
            files.len()
        )));
    }
    let data = registry::load(d)?;
    // repo → 挑到的絕對路徑（保留第一次出現的順序）
    let mut by_repo: Vec<(registry::Repo, Vec<String>)> = Vec::new();
    for f in &files {
        let repo = data
            .repo(&f.id)
            .ok_or_else(|| Error::invalid(format!("repo 不存在: {}", f.id)))?;
        let abs = crate::filetree::locate(&repo.path(), &f.path)?
            .to_string_lossy()
            .into_owned();
        match by_repo.iter_mut().find(|(r, _)| r.id() == f.id) {
            Some((_, list)) if list.contains(&abs) => {}
            Some((_, list)) => list.push(abs),
            None => by_repo.push((repo.clone(), vec![abs])),
        }
    }
    let mut text = String::from("先讀這幾份檔案（從工作台挑的），讀完再開始：\n");
    let mut mentions = Vec::new();
    for (repo, list) in &by_repo {
        let card = crate::profile::profile(repo);
        let mut head = format!("\n## {}", card.id);
        if let Some(role) = &card.role {
            head.push_str(&format!("（{role}）"));
        }
        if let Some(desc) = &card.description {
            head.push_str(&format!(" — {desc}"));
        }
        text.push_str(&head);
        text.push('\n');
        for abs in list {
            text.push_str(&format!("- {abs}\n"));
            mentions.push(format!("@{abs}"));
        }
    }
    let repos: Vec<String> = by_repo.iter().map(|(r, _)| r.id()).collect();
    Ok(
        json!({"ok": true, "text": text, "mentions": mentions.join("\n"),
              "repos": repos, "count": mentions.len()}),
    )
}

/// Rust 才有（F-57 P2）：請 agent 批次起草名片的任務文字。`ids` 沒給＝還沒寫介紹、沒封存的 repo（最多
/// [`DRAFT_MAX`] 個）。任務要求只讀 repo、先列草稿給人確認、確認後才用 `ctx registry describe` 寫——
/// 名片是人要背書的，agent 只起草。附上已用過的角色，讓新寫的跟既有的用詞一致。
#[derive(serde::Deserialize)]
struct DraftArgs {
    #[serde(default)]
    ids: Option<Vec<String>>,
}

/// 一次最多請 agent 起草幾張名片（多了人審不完）。
const DRAFT_MAX: usize = 10;

pub(super) fn a_profile_draft_brief(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let DraftArgs { ids } = parse("profile_draft_brief", p)?;
    let data = registry::load(d)?;
    let picked: Vec<&registry::Repo> = match ids {
        Some(ids) => ids
            .iter()
            .map(|id| {
                data.repo(id)
                    .ok_or_else(|| Error::invalid(format!("repo 不存在: {id}")))
            })
            .collect::<Result<_>>()?,
        None => data
            .repos
            .iter()
            .filter(|r| r.tier() != Some(registry::Tier::Archived) && r.description().is_none())
            .collect(),
    };
    if picked.is_empty() {
        return Err(Error::invalid("沒有要起草的 repo（都寫了介紹）"));
    }
    let picked = &picked[..picked.len().min(DRAFT_MAX)];
    let mut roles: Vec<String> = Vec::new();
    for r in &data.repos {
        if let Some(role) = r.role() {
            if !roles.contains(&role) {
                roles.push(role);
            }
        }
    }
    let mut text = String::from(
        "替這幾個 repo 起草「名片」（一句話介紹、協作角色、搜尋關鍵字）——repo 多了以後靠它找到、\
         同組的 agent 開場靠它認得彼此：\n\n",
    );
    for r in picked {
        let card = crate::profile::profile(r);
        text.push_str(&format!("- {}：{}", card.id, card.path));
        if let (Some(desc), Some(crate::profile::DescSource::Readme)) =
            (&card.description, card.description_source)
        {
            text.push_str(&format!("（README 第一句：{desc}）"));
        }
        text.push('\n');
    }
    if !roles.is_empty() {
        text.push_str(&format!(
            "\n已經用過的角色（能沿用就沿用，用詞一致才好找）：{}\n",
            roles.join("、")
        ));
    }
    text.push_str(
        "\n做法：\n\
         1. 每個 repo 讀 README／AGENTS.md、目錄結構、最近的 commit。**只讀，不改 repo 裡任何檔。**\n\
         2. 先把草稿列成表給我看（id｜角色｜介紹｜關鍵字）：介紹一句話，說這個 repo 是什麼、別的 repo 來這裡找什麼；\
         角色一個短詞；關鍵字 3～6 個。等我確認或修改。\n\
         3. 我確認後，逐一用 `ctx registry describe <id> --description \"…\" --role \"…\" --keywords \"a,b,c\"` 寫入\
         （存在 spine 的 registry，不寫進 repo）。\n",
    );
    let repos: Vec<String> = picked.iter().map(|r| r.id()).collect();
    Ok(json!({"ok": true, "text": text, "repos": repos}))
}

/// Rust 才有（2026-10-14）：請 agent 審查一個 repo 還沒 commit 的變更的任務文字——先不改檔、不 commit，
/// 摘要＋風險＋建議怎麼切 commit，結論用 log_decision 留痕。沒有變更＝錯誤。
pub(super) fn a_review_brief(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let ProfileArgs { id } = parse("review_brief", p)?;
    let repo = registry::get_repo(d, &id)?;
    let (cs, more) = crate::filetree::changes(&repo.path())?;
    if cs.is_empty() {
        return Err(Error::invalid(format!("{id} 沒有還沒 commit 的變更")));
    }
    let card = crate::profile::profile(&repo);
    let role = card.role.map(|r| format!("（{r}）")).unwrap_or_default();
    let mark = |k: crate::filetree::ChangeKind| {
        use crate::filetree::ChangeKind::*;
        match k {
            Modified => "修改",
            Added => "新增",
            Deleted => "刪除",
            Renamed => "改名",
            Untracked => "新檔（未追蹤）",
            Conflicted => "衝突",
        }
    };
    let mut text = format!(
        "審查 {id}{role} 還沒 commit 的變更（{} 個檔）。**先不要改任何檔、也不要 commit。**\n\n",
        cs.len() + more
    );
    for c in cs.iter().take(REVIEW_FILES) {
        text.push_str(&format!("- {}：{}\n", mark(c.kind), c.rel));
    }
    let rest = cs.len().saturating_sub(REVIEW_FILES) + more;
    if rest > 0 {
        text.push_str(&format!("- …另 {rest} 個（`git status` 看完整清單）\n"));
    }
    text.push_str(
        "\n請：\n\
         1. 用 `git diff HEAD -- <檔>` 看每個檔（未追蹤的直接讀）。\n\
         2. 摘要改了什麼、看得出來的話為什麼改。\n\
         3. 列出風險：可能壞掉的地方、漏掉的測試或文件、會影響哪些其他 repo。\n\
         4. 建議怎麼切 commit，每個附一句 commit 訊息。\n\
         看完用 log_decision 記下結論（依據寫檔名），等我決定要不要照做。\n",
    );
    Ok(json!({"ok": true, "id": id, "text": text, "count": cs.len() + more}))
}

/// 審查任務最多列幾個檔。
const REVIEW_FILES: usize = 40;
