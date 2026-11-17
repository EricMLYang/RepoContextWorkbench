//! spine 靜態入口（F-132，2026-11-18，Rust 才有）：由 registry 生成 spine 根目錄的 `AGENTS.md`、`CLAUDE.md`
//! 與每組的 `groups/<組>/REPOS.md`，讓沒掛 hook／MCP 的 agent 在 spine 開場也知道有哪些 repo、在哪、動之前要知道什麼
//! （Home Repo Pattern；背景見 `docs/ideation/rounds/20261117_R42_…` §1、規劃見 `docs/TECH_FOUNDATION_PLAN.md` §5）。
//!
//! - [`render`] 是純函式：registry＋各 repo 的本機事實（[`RepoLocal`]：路徑在不在、remote）→ 檔案內容。
//! - [`check`]／[`sync`] 是薄薄的 IO：讀現況、比對、原子寫入（[`crate::fsutil::write_atomic`]）。
//! - 每個生成檔第一行是 [`MARKER`]。檔案在、但第一行不是標記＝使用者手寫的：不覆蓋，回報衝突。
//!   組被刪或改名後，舊的 `groups/<組>/REPOS.md` 帶標記才刪（資料夾裡其他東西不動；資料夾空了才收掉）。
//! - 自動重生走 [`sync_quiet`]（失敗只寫 stderr，不影響原本的操作）：`ops::call` 成功之後（CLI／工作台／MCP 的
//!   registry、組、名片、類別寫入都經過它）、工作台直接改 registry 的 action（排序、加資料夾、登記 repo）、
//!   `registry scan --apply`、`spawn`、`ctx setup` 結束時。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::registry::{self, RegistryData, Repo, RepoClass};
use crate::Result;

/// 生成檔的第一行（認檔案是不是我們寫的就看這行）。
pub const MARKER: &str = "<!-- 由 ctx home sync 生成，請勿手改；改 registry 後會重寫 -->";
/// `AGENTS.md` 的行數上限（超過就把組那幾行縮短、再不夠就併行；細節都在各組的 REPOS.md）。
pub const AGENTS_MAX_LINES: usize = 60;
/// 沒分組的 repo 放的資料夾名（`groups/_ungrouped/REPOS.md`）。
pub const UNGROUPED_DIR: &str = "_ungrouped";
const REPOS_FILE: &str = "REPOS.md";
/// AGENTS.md 組那一行的目標最多幾個字（完整的在 REPOS.md）。
const GOAL_MAX_CHARS: usize = 40;
/// AGENTS.md「共享 skill」段最多列幾個（再多就併成一行「另 N 個」）。
const SKILLS_MAX_LINES: usize = 12;
/// 共享 skill 那一行的說明最多幾個字（完整的在 SKILL.md）。
const SKILL_DESC_MAX_CHARS: usize = 60;
/// 組數不超過這個才在 AGENTS.md 每組一行附目標（Home Repo Pattern 說約 20 個就該拆；組多了每行要短）。
const FULL_MAX_GROUPS: usize = 15;
const GIT_TIMEOUT: Duration = Duration::from_secs(3);

/// 一個 repo 在本機的事實（IO 層查好；[`render`] 只讀它）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoLocal {
    /// 登記的路徑存在（是資料夾）。
    pub exists: bool,
    /// 沒 clone 時從哪 clone：registry 的 `upstream`，沒有就讀 `git remote get-url origin`；都沒有＝None。
    pub remote: Option<String>,
}

/// 一個生成檔：相對 spine 的路徑（以 `/` 分隔）＋內容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenFile {
    pub rel: String,
    pub content: String,
}

fn abs(spine_dir: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(spine_dir.to_path_buf(), |p, c| p.join(c))
}

/// 比對磁碟後，這個檔要怎麼處理（`ctx home sync --check` 回報的就是這個）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Pending {
    /// 還沒有這個檔。
    Create,
    /// 有，是生成檔，但內容過期。
    Update,
    /// 組刪了或改名了，留下的舊生成檔。
    Remove,
    /// 已是最新。
    Unchanged,
    /// 同名檔是使用者手寫的（沒有標記）：不會覆蓋。
    Conflict,
}

impl Pending {
    /// 跑 `ctx home sync` 會改到磁碟。
    pub fn is_stale(self) -> bool {
        matches!(self, Pending::Create | Pending::Update | Pending::Remove)
    }
}

/// `ctx home sync` 對一個檔做了什麼。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Written,
    Unchanged,
    Removed,
    /// 同名檔是使用者手寫的（沒有標記）：沒有覆蓋。
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileCheck {
    pub path: String,
    pub status: Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileResult {
    pub path: String,
    pub status: Outcome,
}

/// `ctx home sync --check` 的結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Checked {
    pub files: Vec<FileCheck>,
}

impl Checked {
    /// 有檔過期（跑 sync 會寫或刪）。
    pub fn stale(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter(|f| f.status.is_stale())
            .map(|f| f.path.as_str())
            .collect()
    }
    /// 被手寫檔擋住的。
    pub fn conflicts(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter(|f| f.status == Pending::Conflict)
            .map(|f| f.path.as_str())
            .collect()
    }
}

/// `ctx home sync` 的結果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Synced {
    pub files: Vec<FileResult>,
}

impl Synced {
    pub fn conflicts(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter(|f| f.status == Outcome::Conflict)
            .map(|f| f.path.as_str())
            .collect()
    }
}

// ── 純函式：registry → 內容 ─────────────────────────────────────────────

/// 一行字（名片可能是多行）：各行去頭尾空白後接起來（前一行以中文或全形標點結尾就直接接，否則補一個空白），
/// 去掉結尾句號（後面還要接「：路徑」）。
fn one_line(s: &str) -> String {
    let mut t = String::new();
    for l in s.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if t.chars().last().is_some_and(|c| c.is_ascii()) {
            t.push(' ');
        }
        t.push_str(l);
    }
    t.trim_end_matches(['。', '.']).to_string()
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// 組名能不能當 `groups/<組>/` 的資料夾名（跟建組的檢查同一條；`_ungrouped` 保留給沒分組的 repo）。
fn dir_name_ok(name: &str) -> bool {
    name != UNGROUPED_DIR && registry::check_group_name(name).is_ok_and(|n| n == name)
}

/// 括號裡那句「是什麼」：名片（有角色就放前面）；沒名片就寫類別。
fn what_of(r: &Repo) -> String {
    let label = r.class().map_or("未分類", RepoClass::label);
    match (
        r.role().map(|x| one_line(&x)),
        r.description().map(|x| one_line(&x)),
    ) {
        (Some(role), Some(d)) => format!("{role}：{d}"),
        (None, Some(d)) => d,
        (Some(role), None) => format!("{role}；{label}"),
        (None, None) => label.to_string(),
    }
}

/// REPOS.md 的一行：`- **<id>**（<是什麼>）：<路徑>。Clone from <remote> if missing. <提示>`。
fn repo_line(r: &Repo, l: Option<&RepoLocal>) -> String {
    let exists = l.is_some_and(|l| l.exists);
    let mut line = format!(
        "- **{}**（{}）：{}{}。",
        r.id(),
        what_of(r),
        r.path().display(),
        if exists { "" } else { "（路徑不存在）" }
    );
    if let Some(remote) = l.and_then(|l| l.remote.as_deref()) {
        line.push_str(&format!("Clone from {remote} if missing. "));
    }
    // 未分類照程式對待（同 class.rs）
    line.push_str(r.class().unwrap_or(RepoClass::Code).entry_hint());
    line
}

fn repos_md(
    title: &str,
    goal: Option<&str>,
    repos: &[&Repo],
    local: &BTreeMap<String, RepoLocal>,
) -> String {
    let mut out = vec![
        MARKER.to_string(),
        format!("# {title}的 repo（{} 個）", repos.len()),
        String::new(),
    ];
    if let Some(g) = goal {
        out.push(format!("目標：{}", one_line(g)));
    }
    out.push(
        "每個 repo 一行：是什麼、本機路徑、沒 clone 時從哪 clone、動它之前要知道的事。\
         要更多（從哪幾份讀起、最近判斷、最後交接）用 MCP `repo_entry`，或 `ctx repo entry <id> --json`。"
            .into(),
    );
    out.push(String::new());
    if repos.is_empty() {
        out.push("（這組還沒有 repo。）".into());
    }
    out.extend(repos.iter().map(|r| repo_line(r, local.get(&r.id()))));
    out.push(String::new());
    out.join("\n")
}

/// AGENTS.md「Repos」段的一項。
struct IndexEntry {
    label: String,
    count: usize,
    goal: Option<String>,
    file: Option<String>,
}

impl IndexEntry {
    fn full(&self) -> String {
        let goal = self
            .goal
            .as_deref()
            .map(|g| format!("：{}", clip(&one_line(g), GOAL_MAX_CHARS)))
            .unwrap_or_default();
        match &self.file {
            Some(f) => format!("- {}（{} 個 repo）{goal} → `{f}`", self.label, self.count),
            None => format!(
                "- {}（{} 個 repo）{goal}：組名不能當資料夾名，沒有生成清單檔",
                self.label, self.count
            ),
        }
    }
    fn short(&self) -> String {
        match &self.file {
            Some(f) => format!("- {}（{}）→ `{f}`", self.label, self.count),
            None => format!("- {}（{}）：沒有清單檔", self.label, self.count),
        }
    }
    fn tiny(&self) -> String {
        format!("{}（{}）", self.label, self.count)
    }
}

/// 組那幾行：組不多就一組一行（含目標）；組多了（超過 [`FULL_MAX_GROUPS`]）只留名稱、數目、檔名；
/// 行數還是放不下就幾組併一行。
fn index_lines(entries: &[IndexEntry], budget: usize) -> Vec<String> {
    let budget = budget.max(2);
    if entries.len() <= budget && entries.len() <= FULL_MAX_GROUPS {
        return entries.iter().map(IndexEntry::full).collect();
    }
    if entries.len() <= budget {
        return entries.iter().map(IndexEntry::short).collect();
    }
    // 併行：最後一行固定是說明，其餘平均分
    let rows = budget.saturating_sub(1).max(1);
    let per = entries.len().div_ceil(rows);
    let mut out: Vec<String> = entries
        .chunks(per)
        .map(|c| {
            format!(
                "- {}",
                c.iter()
                    .map(IndexEntry::tiny)
                    .collect::<Vec<_>>()
                    .join("、")
            )
        })
        .collect();
    out.push(format!(
        "- 各組的清單在 `groups/<組>/REPOS.md`；未分組的在 `groups/{UNGROUPED_DIR}/REPOS.md`。"
    ));
    out
}

/// spine 的共享 skill（F-133）：(名稱, SKILL.md frontmatter 的 description)。
pub type SharedSkill = (String, Option<String>);

/// 「共享 skill」段（沒有就空）：每個一行，太多就把後面的併成一行。
fn skills_section(skills: &[SharedSkill]) -> Vec<String> {
    if skills.is_empty() {
        return Vec::new();
    }
    let mut out = vec![
        String::new(),
        "## 共享 skill".into(),
        format!(
            "`{}/<名稱>/SKILL.md` 是跨 repo 共用的 skill，`ctx setup --claude --codex` 會發到各家 CLI；要改就改這裡的，不要改發出去的副本。",
            crate::shared_skills::SKILLS_DIR
        ),
    ];
    let line = |(name, desc): &SharedSkill| match desc {
        Some(d) => format!("- **{name}**：{}", clip(&one_line(d), SKILL_DESC_MAX_CHARS)),
        None => format!("- **{name}**"),
    };
    if skills.len() <= SKILLS_MAX_LINES {
        out.extend(skills.iter().map(line));
    } else {
        out.extend(skills[..SKILLS_MAX_LINES - 1].iter().map(line));
        let rest = &skills[SKILLS_MAX_LINES - 1..];
        out.push(format!(
            "- 另 {} 個：{}",
            rest.len(),
            rest.iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join("、")
        ));
    }
    out
}

fn agents_md(data: &RegistryData, entries: &[IndexEntry], skills: &[SharedSkill]) -> String {
    let head = vec![
        MARKER.to_string(),
        "# repo-context spine".into(),
        String::new(),
        format!(
            "這裡是 repo-context 工作台的 spine：登記了 {} 個 repo、分成 {} 個組；判斷、交接、未結記在 `spine/`（事件 md），不寫進各 repo。",
            data.repos.len(),
            data.groups.len()
        ),
        "這份檔與 `groups/*/REPOS.md` 由 registry 生成：沒掛 hook／MCP 的 agent 開場也看得到有哪些 repo、在哪、動之前要知道什麼。".into(),
        "要知道有哪些 repo，讀下面「Repos」指的檔就好，不用掃檔案系統找 `.git`。".into(),
        String::new(),
        "## 怎麼拿 context".into(),
        "- 有掛 MCP `repo-context`：`where_am_i` 看自己在哪、`context_for` 拿目標／交接／未結、`next_work` 看下一步、`search_knowledge` 問哪個 repo 有、`repo_entry` 看從哪讀起、`read_from` 讀別的 repo 的檔。".into(),
        "- 沒掛 MCP：同名功能用 `ctx <where|context|next|search|read|repo entry <id>> --json`；全部指令見 `ctx --help`。".into(),
        "- 要改某個 repo 前先讀它路徑下的 AGENTS.md；清單裡標「只讀」的是參考類，不要改。".into(),
        String::new(),
        "## Repos".into(),
    ];
    let tail = vec![
        String::new(),
        "## 資料來源".into(),
        "- `registry.yaml`（repo、組、名片、類別）與 `config.yaml`（agent 指令、排程、provider）是資料來源；這幾份生成檔不要手改。".into(),
        "- 要改用 `ctx registry …`（登記、`describe` 名片、`class` 類別）或 `ctx group …`（建組、組員、改名、目標）；改完會自動重寫，也可以跑 `ctx home sync`。".into(),
    ];
    let skills = skills_section(skills);
    let body = if entries.is_empty() {
        vec!["- 還沒有登記任何 repo：`ctx registry add <id> <路徑>`。".to_string()]
    } else {
        index_lines(
            entries,
            AGENTS_MAX_LINES - head.len() - tail.len() - skills.len(),
        )
    };
    let mut out = head;
    out.extend(body);
    out.extend(skills);
    out.extend(tail);
    out.push(String::new());
    out.join("\n")
}

/// registry＋本機事實 → 所有生成檔（AGENTS.md、CLAUDE.md、每組的 REPOS.md、有未分組才有 `_ungrouped`）。純函式。
pub fn render(data: &RegistryData, local: &BTreeMap<String, RepoLocal>) -> Vec<GenFile> {
    render_with(data, local, &[])
}

/// 同 [`render`]，AGENTS.md 多列 spine 的共享 skill（F-133；空的就跟 [`render`] 一樣）。純函式。
pub fn render_with(
    data: &RegistryData,
    local: &BTreeMap<String, RepoLocal>,
    skills: &[SharedSkill],
) -> Vec<GenFile> {
    let mut files = Vec::new();
    let mut entries = Vec::new();
    let mut grouped: BTreeSet<String> = BTreeSet::new();
    for g in &data.groups {
        let name = g.name();
        let mut seen = BTreeSet::new();
        let members: Vec<&Repo> = g
            .members()
            .iter()
            .filter_map(|id| data.repo(id))
            .filter(|r| seen.insert(r.id()))
            .collect();
        grouped.extend(members.iter().map(|r| r.id()));
        let file = dir_name_ok(&name).then(|| format!("groups/{name}/{REPOS_FILE}"));
        if let Some(rel) = &file {
            files.push(GenFile {
                rel: rel.clone(),
                content: repos_md(
                    &format!("組「{name}」"),
                    g.goal().as_deref(),
                    &members,
                    local,
                ),
            });
        }
        entries.push(IndexEntry {
            label: name,
            count: members.len(),
            goal: g.goal(),
            file,
        });
    }
    let ungrouped: Vec<&Repo> = data
        .repos
        .iter()
        .filter(|r| !grouped.contains(&r.id()))
        .collect();
    if !ungrouped.is_empty() {
        let rel = format!("groups/{UNGROUPED_DIR}/{REPOS_FILE}");
        files.push(GenFile {
            rel: rel.clone(),
            content: repos_md("未分組", None, &ungrouped, local),
        });
        entries.push(IndexEntry {
            label: "未分組".into(),
            count: ungrouped.len(),
            goal: None,
            file: Some(rel),
        });
    }
    let mut out = vec![
        GenFile {
            rel: "AGENTS.md".into(),
            content: agents_md(data, &entries, skills),
        },
        GenFile {
            rel: "CLAUDE.md".into(),
            content: format!("{MARKER}\n@AGENTS.md\n"),
        },
    ];
    out.extend(files);
    out
}

/// 文字是不是生成檔（第一行是標記）。
fn is_generated(text: &str) -> bool {
    text.trim_start_matches('\u{feff}')
        .lines()
        .next()
        .is_some_and(|l| l.trim() == MARKER)
}

fn classify(disk: &Disk, want: &str) -> Pending {
    match disk {
        Disk::Missing => Pending::Create,
        Disk::Foreign => Pending::Conflict,
        Disk::Text(t) if !is_generated(t) => Pending::Conflict,
        Disk::Text(t) if t == want => Pending::Unchanged,
        Disk::Text(_) => Pending::Update,
    }
}

// ── IO ──────────────────────────────────────────────────────────────────

/// 磁碟上的同名檔。
enum Disk {
    Missing,
    Text(String),
    /// 讀不成文字（資料夾、壞的 UTF-8、沒權限）：當成不是我們的。
    Foreign,
}

fn read_disk(p: &Path) -> Disk {
    match std::fs::read_to_string(p) {
        Ok(t) => Disk::Text(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Disk::Missing,
        Err(_) => Disk::Foreign,
    }
}

/// 路徑是生成檔（讀得到且第一行是標記）。
fn is_generated_file(p: &Path) -> bool {
    matches!(read_disk(p), Disk::Text(t) if is_generated(&t))
}

/// URL 裡的帳號密碼（`https://user:token@host/…`）拿掉，生成檔會進 spine 的 git。
fn strip_userinfo(s: &str) -> String {
    if let Some(i) = s.find("://") {
        let rest = &s[i + 3..];
        let host_end = rest.find('/').unwrap_or(rest.len());
        if let Some(at) = rest[..host_end].rfind('@') {
            return format!("{}{}", &s[..i + 3], &rest[at + 1..]);
        }
    }
    s.to_string()
}

/// registry 的 `upstream` → 可以 clone 的位址：完整 URL／`git@` 照用；`owner/repo` 補成 GitHub URL。
fn upstream_url(u: &str) -> String {
    let u = u.trim();
    if u.contains("://") || u.starts_with("git@") {
        return strip_userinfo(u);
    }
    let n = crate::upstream::normalize(u);
    if n.split('/').count() == 2 && !n.contains(' ') {
        format!("https://github.com/{n}")
    } else {
        u.to_string()
    }
}

fn remote_of(r: &Repo, exists: bool) -> Option<String> {
    if let Some(u) = r.upstream() {
        return Some(upstream_url(&u));
    }
    let p = r.path();
    // 只問 repo 本身（.git 是資料夾或 worktree 的檔）：不然會讀到外層 repo 的 origin
    if !exists || !p.join(".git").exists() {
        return None;
    }
    let (code, out) = crate::crossref::git(&p, &["remote", "get-url", "origin"], GIT_TIMEOUT);
    let out = out.trim();
    (code == 0 && !out.is_empty()).then(|| strip_userinfo(out))
}

/// 各 repo 的本機事實（每個沒有 upstream 的 repo 跑一次 git；失敗就當沒有 remote）。
/// git 各開一條執行緒同時跑：每次 registry 寫入都會走到這裡，23 個 repo 依序跑要 1 秒左右。
pub fn local_facts(data: &RegistryData) -> BTreeMap<String, RepoLocal> {
    std::thread::scope(|sc| {
        let jobs: Vec<_> = data
            .repos
            .iter()
            .map(|r| {
                sc.spawn(move || {
                    let exists = r.path().is_dir();
                    (
                        r.id(),
                        RepoLocal {
                            exists,
                            remote: remote_of(r, exists),
                        },
                    )
                })
            })
            .collect();
        jobs.into_iter().filter_map(|j| j.join().ok()).collect()
    })
}

/// 要寫的檔＋要刪的舊生成檔，各自配上磁碟現況。
fn plan(spine_dir: &Path) -> Result<Vec<(String, Option<String>, Pending)>> {
    let data = registry::load(spine_dir)?;
    let skills = crate::shared_skills::listed(spine_dir);
    let want = render_with(&data, &local_facts(&data), &skills);
    let wanted: BTreeSet<&str> = want.iter().map(|f| f.rel.as_str()).collect();
    let mut out: Vec<(String, Option<String>, Pending)> = want
        .iter()
        .map(|f| {
            let st = classify(&read_disk(&abs(spine_dir, &f.rel)), &f.content);
            (f.rel.clone(), Some(f.content.clone()), st)
        })
        .collect();
    // 組刪了／改名了：舊的生成檔（只認帶標記的）
    let mut orphans = Vec::new();
    if let Ok(rd) = std::fs::read_dir(spine_dir.join("groups")) {
        for e in rd.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            let rel = format!("groups/{name}/{REPOS_FILE}");
            if !wanted.contains(rel.as_str()) && is_generated_file(&e.path().join(REPOS_FILE)) {
                orphans.push(rel);
            }
        }
    }
    orphans.sort();
    out.extend(orphans.into_iter().map(|rel| (rel, None, Pending::Remove)));
    Ok(out)
}

/// `ctx home sync --check`：不寫，只報告每個檔的狀態。
pub fn check(spine_dir: &Path) -> Result<Checked> {
    Ok(Checked {
        files: plan(spine_dir)?
            .into_iter()
            .map(|(path, _, status)| FileCheck { path, status })
            .collect(),
    })
}

/// `ctx home sync`：寫過期的、刪舊的、不碰手寫的。不拿 spine 的 lock（生成檔只有這裡寫，內容由 registry 決定；
/// 兩個進程同時寫的是同一份內容，原子寫入不會留下半份）。
pub fn sync(spine_dir: &Path) -> Result<Synced> {
    let mut files = Vec::new();
    for (rel, content, st) in plan(spine_dir)? {
        let p = abs(spine_dir, &rel);
        let status = match (st, content) {
            (Pending::Create | Pending::Update, Some(c)) => {
                if let Some(dir) = p.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                crate::fsutil::write_atomic(&p, c.as_bytes())?;
                Outcome::Written
            }
            (Pending::Remove, _) => {
                remove_generated(&p)?;
                Outcome::Removed
            }
            (Pending::Conflict, _) => Outcome::Conflict,
            _ => Outcome::Unchanged,
        };
        files.push(FileResult { path: rel, status });
    }
    Ok(Synced { files })
}

/// 刪一個生成檔；它的資料夾空了就一起收掉（裡面還有別的東西就留著）。
fn remove_generated(p: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(p) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if let Some(dir) = p.parent() {
        let _ = std::fs::remove_dir(dir); // 不空就失敗＝留著
    }
    Ok(())
}

/// 自動重生（registry／名片寫入之後、`ctx setup` 結束時）：不是 spine 就不做；失敗只寫 stderr，不影響原本的操作。
/// 手寫檔的衝突不在這裡講（每次寫入都講太吵），`ctx health` 會提醒。
pub fn sync_quiet(spine_dir: &Path) {
    if !crate::config::is_spine(spine_dir) {
        return;
    }
    if let Err(e) = sync(spine_dir) {
        eprintln!("ctx home sync：spine 入口檔沒更新成（{e}）；之後可手動跑 `ctx home sync`");
    }
}

/// `groups/<組>/` 裡除了生成的 REPOS.md 還有沒有別的東西。組改名／刪除時用它判斷「有沒有料要搬／保留」——
/// 只有生成檔的資料夾當成沒有料（跟沒生成入口檔時的行為一樣）。
pub fn group_dir_has_user_files(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.flatten()
        .any(|e| !(e.file_name() == REPOS_FILE && is_generated_file(&e.path())))
}

/// 資料夾裡只剩生成檔時把它清掉（組改名要搬進來之前用）。
pub fn clear_generated_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() && !group_dir_has_user_files(dir) {
        remove_generated(&dir.join(REPOS_FILE))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userinfo_and_upstream() {
        assert_eq!(
            strip_userinfo("https://me:tok@github.com/a/b.git"),
            "https://github.com/a/b.git"
        );
        assert_eq!(
            strip_userinfo("git@github.com:a/b.git"),
            "git@github.com:a/b.git"
        );
        assert_eq!(upstream_url("a/b"), "https://github.com/a/b");
        assert_eq!(
            upstream_url("https://github.com/a/b"),
            "https://github.com/a/b"
        );
        assert_eq!(upstream_url("git@host:x/y.git"), "git@host:x/y.git");
    }

    #[test]
    fn marker_detection() {
        assert!(is_generated(&format!("{MARKER}\r\nx")));
        assert!(is_generated(&format!("\u{feff}{MARKER}\n")));
        assert!(!is_generated("# 我自己寫的\n"));
        assert_eq!(
            classify(&Disk::Text("# 手寫".into()), "x"),
            Pending::Conflict
        );
        assert_eq!(classify(&Disk::Missing, "x"), Pending::Create);
        let g = format!("{MARKER}\nold");
        assert_eq!(classify(&Disk::Text(g.clone()), &g), Pending::Unchanged);
        assert_eq!(classify(&Disk::Text(g), "new"), Pending::Update);
    }
}
