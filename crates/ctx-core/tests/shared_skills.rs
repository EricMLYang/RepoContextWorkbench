//! 工作台 skill（F-133，Rust 才有）：`<spine>/skills/<名稱>/` 只在納管 repo 的會話用得到——開場清單＋`skill_get`，
//! 不發成檔案；舊版發到使用者層的副本由 setup 收回。全部用 tempdir，不碰真的 `~/.claude`、`~/.codex`。

use std::path::{Path, PathBuf};

use ctx_core::agentsetup::{self, AgentCli, SetupEnv, Step};
use ctx_core::shared_skills::{self, SkipReason, MARK_FILE, MAX_FILE_BYTES, MAX_SKILL_BYTES};
use ctx_core::testkit::{spine, spine_with_repos};
use serde_json::json;

fn env_in(tmp: &Path, spine_dir: &Path) -> SetupEnv {
    SetupEnv {
        spine_dir: spine_dir.to_path_buf(),
        exe: PathBuf::from("/opt/ctx/bin/ctx"),
        claude_home: tmp.join("home/.claude"),
        codex_home: tmp.join("home/.codex"),
        user_config: tmp.join("home/.config/repo-context/config.json"),
    }
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn skill_md(desc: &str) -> String {
    format!("---\nname: x\ndescription: {desc}\n---\n# 說明\n")
}

fn repo_path(sp: &Path, id: &str) -> PathBuf {
    let data = ctx_core::registry::load(sp).unwrap();
    data.repo(id).unwrap().path()
}

fn opening_text(sp: &Path, cwd: &Path) -> String {
    let out = ctx_core::hooks::session_start(sp, &json!({"cwd": cwd.to_string_lossy()}), None);
    if out.is_empty() {
        return out;
    }
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap()
        .to_string()
}

fn reject_code(e: ctx_core::Error) -> &'static str {
    match e {
        ctx_core::Error::Rejected { code, .. } => code.as_str(),
        other => panic!("不是 Rejected：{other:?}"),
    }
}

/// 清單、SKILL.md、附檔：三層都讀得到；不在清單裡的檔（含 `..`）讀不到。
#[test]
fn index_and_read_progressive_layers() {
    let (_t, sp) = spine();
    let src = sp.join("skills/find-context");
    write(&src.join("SKILL.md"), &skill_md("找別的 repo 的資料"));
    write(&src.join("references/steps.md"), "步驟\n");
    write(&sp.join("registry.yaml.bak"), "x"); // skill 資料夾外的檔

    let idx = shared_skills::index(&sp).unwrap();
    assert_eq!(idx.skills.len(), 1);
    assert_eq!(idx.skills[0].name, "find-context");
    assert_eq!(
        idx.skills[0].description.as_deref(),
        Some("找別的 repo 的資料")
    );
    assert_eq!(idx.skills[0].files, ["SKILL.md", "references/steps.md"]);

    let d = shared_skills::read(&sp, "find-context", None).unwrap();
    assert_eq!(d.file, "SKILL.md");
    assert!(d.text.contains("# 說明"));
    assert!(d.dir.ends_with("skills/find-context"), "{}", d.dir);
    let f = shared_skills::read(&sp, "find-context", Some("./references/steps.md")).unwrap();
    assert_eq!(
        (f.file.as_str(), f.text.as_str()),
        ("references/steps.md", "步驟\n")
    );

    for bad in ["../../registry.yaml.bak", "nope.md"] {
        let e = shared_skills::read(&sp, "find-context", Some(bad)).unwrap_err();
        assert_eq!(reject_code(e), "not_found");
    }
    let e = shared_skills::read(&sp, "nope", None).unwrap_err();
    assert_eq!(reject_code(e), "not_found");
}

/// 保留名稱、不合法名稱、沒有 SKILL.md、超過上限：不進清單、列在 skipped，讀的時候講原因。
#[test]
fn reserved_bad_names_and_limits_are_skipped() {
    let (_t, sp) = spine();
    let s = sp.join("skills");
    write(&s.join("repo-context/SKILL.md"), &skill_md("冒名"));
    write(&s.join("Bad_Name/SKILL.md"), &skill_md("x"));
    write(&s.join("no-md/README.md"), "x");
    write(&s.join("big-file/SKILL.md"), &skill_md("x"));
    std::fs::write(
        s.join("big-file/blob.bin"),
        vec![b'x'; (MAX_FILE_BYTES + 1) as usize],
    )
    .unwrap();
    write(&s.join("big-total/SKILL.md"), &skill_md("x"));
    let chunk = vec![b'y'; (MAX_FILE_BYTES - 10) as usize];
    for i in 0..=(MAX_SKILL_BYTES / MAX_FILE_BYTES) {
        std::fs::write(s.join(format!("big-total/part{i}.bin")), &chunk).unwrap();
    }
    // skills/ 底下的單檔與隱藏資料夾不算 skill
    write(&s.join("README.md"), "這裡放工作台 skill");
    write(&s.join(".hidden/SKILL.md"), "x");

    let idx = shared_skills::index(&sp).unwrap();
    assert!(idx.skills.is_empty());
    let got: Vec<(&str, &SkipReason)> = idx
        .skipped
        .iter()
        .map(|k| (k.name.as_str(), &k.reason))
        .collect();
    assert_eq!(got.len(), 5, "{got:?}");
    assert_eq!(got[0], ("Bad_Name", &SkipReason::BadName));
    assert!(
        matches!(got[1], ("big-file", SkipReason::FileTooLarge { file, .. }) if file == "blob.bin")
    );
    assert!(matches!(got[2], ("big-total", SkipReason::TooLarge { .. })));
    assert_eq!(got[3], ("no-md", &SkipReason::NoSkillMd));
    assert_eq!(got[4], ("repo-context", &SkipReason::Reserved));
    let e = shared_skills::read(&sp, "no-md", None).unwrap_err();
    assert!(format!("{e:?}").contains("沒有啟用"), "{e:?}");
    assert!(shared_skills::opening(&sp).is_none());
}

/// 指到 skill 資料夾外面的 symlink 不跟（列在 ignored、讀不到）；skill 資料夾本身是 symlink 也不跟。
#[cfg(unix)]
#[test]
fn symlinks_outside_are_not_followed() {
    use std::os::unix::fs::symlink;
    let (t, sp) = spine();
    let outside = t.path().join("secret.txt");
    write(&outside, "秘密");
    let src = sp.join("skills/linky");
    write(&src.join("SKILL.md"), &skill_md("x"));
    write(&src.join("inner.md"), "裡面的");
    symlink(&outside, src.join("leak.txt")).unwrap();
    symlink(src.join("inner.md"), src.join("alias.md")).unwrap();
    let other = t.path().join("elsewhere-skill");
    write(&other.join("SKILL.md"), &skill_md("x"));
    symlink(&other, sp.join("skills/linked")).unwrap();

    let idx = shared_skills::index(&sp).unwrap();
    assert_eq!(idx.skills.len(), 1);
    let e = &idx.skills[0];
    assert_eq!(e.files, ["SKILL.md", "alias.md", "inner.md"]);
    assert_eq!(e.ignored, ["leak.txt（symlink 指到 skill 資料夾外，不跟）"]);
    assert_eq!(
        idx.skipped
            .iter()
            .map(|k| (k.name.as_str(), &k.reason))
            .collect::<Vec<_>>(),
        [("linked", &SkipReason::Symlink)]
    );
    assert!(shared_skills::read(&sp, "linky", Some("leak.txt")).is_err());
    assert_eq!(
        shared_skills::read(&sp, "linky", Some("alias.md"))
            .unwrap()
            .text,
        "裡面的"
    );
}

/// 只在納管 repo 用得到：開場清單與 skill_get 在 repo 裡有、在外面沒有；repo 移出 registry 就都沒有。
/// 整個過程任何 repo 的工作目錄都沒多出檔案。
#[test]
fn only_registered_repos_see_workbench_skills() {
    let (t, sp) = spine_with_repos();
    let ra = repo_path(&sp, "repo-a");
    let outside = t.path().join("not-registered");
    std::fs::create_dir_all(&outside).unwrap();
    let before = opening_text(&sp, &ra);
    assert!(!before.contains("工作台 skill"));

    write(
        &sp.join("skills/find-context/SKILL.md"),
        &skill_md("找別的 repo 的資料"),
    );
    // 第 1 層：納管 repo 的開場多一段（原本的內容一字不差、段落接在後面）
    let text = opening_text(&sp, &ra);
    assert!(text.starts_with(&before), "{text}");
    assert!(
        text.ends_with("\n- find-context：找別的 repo 的資料"),
        "{text}"
    );
    // 不在納管 repo：hook 什麼都不輸出
    assert_eq!(opening_text(&sp, &outside), "");

    // 第 2 層：skill_get
    let list = ctx_core::agentapi::skill_get(&sp, None, None, Some(&ra)).unwrap();
    assert_eq!(list["skills"][0]["name"], "find-context");
    let doc = ctx_core::agentapi::skill_get(&sp, Some("find-context"), None, Some(&ra.join("src")))
        .unwrap();
    assert!(doc["text"].as_str().unwrap().contains("# 說明"));
    let e =
        ctx_core::agentapi::skill_get(&sp, Some("find-context"), None, Some(&outside)).unwrap_err();
    assert_eq!(reject_code(e), "not_found");

    // repo 裡沒有任何 skill 檔
    for d in [".claude", ".agents", ".codex"] {
        assert!(!ra.join(d).exists(), "{d}");
    }
    let st = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&ra)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&st.stdout), "");

    // 移出列管：下一場開場沒有、skill_get 拒絕
    ctx_core::registry::remove_repo(&sp, "repo-a").unwrap();
    assert_eq!(opening_text(&sp, &ra), "");
    assert!(ctx_core::agentapi::skill_get(&sp, None, None, Some(&ra)).is_err());
}

/// 開場清單有上限：超過 [`shared_skills::OPENING_MAX`] 個併成一行，說明太長截短。
#[test]
fn opening_is_capped() {
    let (_t, sp) = spine();
    let n = shared_skills::OPENING_MAX + 3;
    for i in 0..n {
        write(
            &sp.join(format!("skills/s{i:02}/SKILL.md")),
            &skill_md(&"很長的說明".repeat(40)),
        );
    }
    let t = shared_skills::opening(&sp).unwrap();
    let lines: Vec<&str> = t.lines().collect();
    assert_eq!(lines.len(), 1 + shared_skills::OPENING_MAX + 1, "{t}");
    assert_eq!(
        *lines.last().unwrap(),
        "- …還有 3 個：skill_get 不帶名稱看全部"
    );
    assert!(lines[1].ends_with('…'));
    assert!(lines[1].chars().count() <= shared_skills::OPENING_DESC_CHARS + 10);
}

/// 舊版發到使用者層的副本：帶本 spine 標記的才收；使用者自己的、別的 spine 的不動。setup 有東西可收才多一步。
#[test]
fn retire_user_level_copies() {
    let (t, sp) = spine();
    let env = env_in(t.path(), &sp);
    let both = [AgentCli::Claude, AgentCli::Codex];
    assert_eq!(
        agentsetup::setup_plan(&env, &both, &[]),
        agentsetup::plan(&both)
    );

    let ours =
        json!({"spine": ctx_core::fsutil::resolve(&sp).to_string_lossy(), "hash": "x"}).to_string();
    let other = r#"{"spine": "/elsewhere/spine", "hash": ""}"#;
    let home = env.home(AgentCli::Codex);
    write(&home.join("skills/old-shared/SKILL.md"), "舊副本");
    write(&home.join("skills/old-shared").join(MARK_FILE), &ours);
    write(&home.join("skills/theirs/SKILL.md"), "別的 spine 的");
    write(&home.join("skills/theirs").join(MARK_FILE), other);
    write(&home.join("skills/private/SKILL.md"), "使用者自己寫的");

    assert_eq!(
        shared_skills::retire(&sp, home, false).unwrap(),
        ["old-shared"]
    );
    assert!(home.join("skills/old-shared").exists()); // 試跑不寫
    let steps = agentsetup::setup_plan(&env, &both, &[]);
    assert!(steps.contains(&Step::RetireSharedSkills(AgentCli::Codex)));
    assert!(!steps.contains(&Step::RetireSharedSkills(AgentCli::Claude)));
    let desc = Step::RetireSharedSkills(AgentCli::Codex).desc(&env);
    assert!(desc.ends_with("：old-shared"), "{desc}");
    let r = Step::RetireSharedSkills(AgentCli::Codex).run(&env).unwrap();
    assert!(r.ok);
    assert!(
        r.message.contains("收回使用者層的舊副本 old-shared"),
        "{}",
        r.message
    );
    assert!(!home.join("skills/old-shared").exists());
    assert!(home.join("skills/theirs/SKILL.md").exists());
    assert!(home.join("skills/private/SKILL.md").exists());
    assert_eq!(
        agentsetup::setup_plan(&env, &both, &[]),
        agentsetup::plan(&both)
    );
}

/// spine 的 AGENTS.md：有工作台 skill 才多一段，每個一行（名稱＋description 截短），仍守 60 行。
#[test]
fn agents_md_lists_shared_skills() {
    let (_t, sp) = spine();
    ctx_core::home::sync(&sp).unwrap();
    let before = std::fs::read_to_string(sp.join("AGENTS.md")).unwrap();
    assert!(!before.contains("工作台 skill"));
    // 空的 skills/：不出現
    std::fs::create_dir_all(sp.join("skills")).unwrap();
    ctx_core::home::sync(&sp).unwrap();
    assert_eq!(
        std::fs::read_to_string(sp.join("AGENTS.md")).unwrap(),
        before
    );

    write(
        &sp.join("skills/pdf-tools/SKILL.md"),
        &skill_md(&"讀寫 PDF、合併、拆頁、轉圖片".repeat(5)),
    );
    write(&sp.join("skills/plain/SKILL.md"), "# 沒有 frontmatter\n");
    write(&sp.join("skills/Bad/SKILL.md"), &skill_md("x"));
    let st = ctx_core::home::check(&sp).unwrap();
    assert_eq!(st.stale(), ["AGENTS.md"]);
    ctx_core::home::sync(&sp).unwrap();
    let text = std::fs::read_to_string(sp.join("AGENTS.md")).unwrap();
    assert!(text.contains("\n## 工作台 skill\n"), "{text}");
    let line = text
        .lines()
        .find(|l| l.starts_with("- **pdf-tools**："))
        .unwrap();
    assert!(line.ends_with('…') && line.chars().count() < 80, "{line}");
    assert!(text.lines().any(|l| l == "- **plain**"));
    assert!(!text.contains("Bad"));
    // 段落在 Repos 之後、資料來源之前
    assert!(text.find("## Repos").unwrap() < text.find("## 工作台 skill").unwrap());
    assert!(text.find("## 工作台 skill").unwrap() < text.find("## 資料來源").unwrap());
}

/// 很多 skill、很多組時仍守 60 行上限。
#[test]
fn agents_md_stays_within_line_limit_with_many_skills() {
    let (t, _sp) = spine();
    let mut yaml = String::from("repos:\n");
    for i in 0..80 {
        yaml.push_str(&format!(
            "- id: r{i}\n  path: /w/r{i}\n  type: mine\n  tier: active\n"
        ));
    }
    yaml.push_str("groups:\n");
    for i in 0..80 {
        yaml.push_str(&format!("- name: g{i}\n  members: [r{i}]\n"));
    }
    std::fs::write(t.path().join("registry.yaml"), yaml).unwrap();
    let data = ctx_core::registry::load(t.path()).unwrap();
    let skills: Vec<(String, Option<String>)> = (0..30)
        .map(|i| (format!("s{i}"), Some(format!("第 {i} 個"))))
        .collect();
    let files = ctx_core::home::render_with(&data, &Default::default(), &skills);
    let agents = &files.iter().find(|f| f.rel == "AGENTS.md").unwrap().content;
    assert!(
        agents.lines().count() <= ctx_core::home::AGENTS_MAX_LINES,
        "{} 行",
        agents.lines().count()
    );
    assert!(agents.contains("- 另 19 個：s11、"), "{agents}");
    // 沒有 skill 時跟 render 一樣
    assert_eq!(
        ctx_core::home::render_with(&data, &Default::default(), &[]),
        ctx_core::home::render(&data, &Default::default())
    );
}
