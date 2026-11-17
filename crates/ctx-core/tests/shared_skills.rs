//! 共享 skill（F-133，Rust 才有）：`<spine>/skills/<名稱>/` → 各家 CLI 的 `skills/<名稱>/`。
//! 全部用 tempdir 與 [`SetupEnv`] 的假 home，不碰真的 `~/.claude`、`~/.codex`。

use std::path::{Path, PathBuf};

use ctx_core::agentsetup::{self, AgentCli, SetupEnv, Step};
use ctx_core::shared_skills::{
    self, Conflict, Outcome, Report, SkipReason, MARK_FILE, MAX_FILE_BYTES, MAX_SKILL_BYTES,
};
use ctx_core::testkit::spine;

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

fn outcomes(r: &[Report]) -> Vec<(&str, &Outcome)> {
    r.iter().map(|x| (x.name.as_str(), &x.outcome)).collect()
}

fn run(env: &SetupEnv, cli: AgentCli) -> Vec<Report> {
    shared_skills::sync(&env.spine_dir, env.home(cli), true).unwrap()
}

/// 發送、更新、來源刪檔、來源刪 skill、冪等、dry-run 不寫——兩家各跑一次。
#[test]
fn publish_update_delete_and_idempotent_for_both_clis() {
    for cli in AgentCli::ALL {
        let (t, sp) = spine();
        let env = env_in(t.path(), &sp);
        let src = sp.join("skills/pdf-tools");
        write(&src.join("SKILL.md"), &skill_md("處理 PDF"));
        write(&src.join("scripts/run.sh"), "echo hi\n");
        write(&src.join("ref/a.md"), "a\n");
        let dest = env.home(cli).join("skills/pdf-tools");

        // dry-run：算得出計畫、不寫
        let plan = shared_skills::sync(&sp, env.home(cli), false).unwrap();
        assert_eq!(outcomes(&plan), [("pdf-tools", &Outcome::Installed)]);
        assert!(!dest.exists());
        assert!(shared_skills::has_work(&sp, env.home(cli)));

        // 發送
        let r = run(&env, cli);
        assert_eq!(outcomes(&r), [("pdf-tools", &Outcome::Installed)]);
        assert_eq!(
            std::fs::read_to_string(dest.join("scripts/run.sh")).unwrap(),
            "echo hi\n"
        );
        assert!(dest.join("SKILL.md").is_file() && dest.join("ref/a.md").is_file());
        let mark = std::fs::read_to_string(dest.join(MARK_FILE)).unwrap();
        let mark: serde_json::Value = serde_json::from_str(&mark).unwrap();
        assert_eq!(
            mark["spine"].as_str().unwrap(),
            ctx_core::fsutil::resolve(&sp).to_string_lossy()
        );
        assert_eq!(mark["hash"].as_str().unwrap().len(), 16);
        // 內建 skill 不受影響、也沒被當成要收的
        assert!(!env.home(cli).join("skills/repo-context").exists());

        // 冪等
        assert_eq!(
            outcomes(&run(&env, cli)),
            [("pdf-tools", &Outcome::Unchanged)]
        );
        assert!(shared_skills::pending(&sp, env.home(cli))
            .unwrap()
            .is_empty());

        // 改一個檔 → 更新
        write(&src.join("scripts/run.sh"), "echo bye\n");
        let p = shared_skills::pending(&sp, env.home(cli)).unwrap();
        assert_eq!(p.stale, ["pdf-tools"]);
        assert_eq!(
            outcomes(&run(&env, cli)),
            [("pdf-tools", &Outcome::Updated)]
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("scripts/run.sh")).unwrap(),
            "echo bye\n"
        );
        let mark2 = std::fs::read_to_string(dest.join(MARK_FILE)).unwrap();
        assert_ne!(
            serde_json::from_str::<serde_json::Value>(&mark2).unwrap()["hash"],
            mark["hash"]
        );

        // 有人改了發出去的副本 → 下次換回來源的樣子
        write(&dest.join("SKILL.md"), "被改了");
        assert_eq!(
            outcomes(&run(&env, cli)),
            [("pdf-tools", &Outcome::Updated)]
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            skill_md("處理 PDF")
        );

        // 來源刪檔 → 目的地跟著刪（空資料夾一起收）
        std::fs::remove_dir_all(src.join("ref")).unwrap();
        assert_eq!(
            outcomes(&run(&env, cli)),
            [("pdf-tools", &Outcome::Updated)]
        );
        assert!(!dest.join("ref").exists());
        assert!(dest.join("scripts/run.sh").exists());

        // 來源刪 skill → 目的地移除；dry-run 先看得到
        std::fs::remove_dir_all(&src).unwrap();
        let plan = shared_skills::sync(&sp, env.home(cli), false).unwrap();
        assert_eq!(outcomes(&plan), [("pdf-tools", &Outcome::Removed)]);
        assert!(dest.exists());
        assert_eq!(
            shared_skills::pending(&sp, env.home(cli)).unwrap().stale,
            ["pdf-tools"]
        );
        assert_eq!(
            outcomes(&run(&env, cli)),
            [("pdf-tools", &Outcome::Removed)]
        );
        assert!(!dest.exists());
        assert!(run(&env, cli).is_empty());
        assert!(!shared_skills::has_work(&sp, env.home(cli)));
        // 另一家沒被動到
        let other = AgentCli::ALL.into_iter().find(|c| *c != cli).unwrap();
        assert!(!env.home(other).exists());
    }
}

/// 無標記的同名資料夾＝使用者的，不覆蓋；別的 spine 發的不動（也不當成已刪而收掉）。
#[test]
fn user_folders_and_other_spines_are_left_alone() {
    for cli in AgentCli::ALL {
        let (t, sp) = spine();
        let env = env_in(t.path(), &sp);
        write(&sp.join("skills/mine/SKILL.md"), &skill_md("我的"));
        write(&sp.join("skills/shared/SKILL.md"), &skill_md("共用"));
        let home = env.home(cli);
        write(&home.join("skills/mine/SKILL.md"), "使用者自己寫的");
        // 別的 spine 發的：一個同名（衝突）、一個 spine 裡沒有（不收）
        let other = r#"{"spine": "/elsewhere/spine", "hash": ""}"#;
        write(&home.join("skills/shared/SKILL.md"), "別的 spine 的");
        write(&home.join("skills/shared").join(MARK_FILE), other);
        write(&home.join("skills/theirs/SKILL.md"), "別的 spine 的");
        write(&home.join("skills/theirs").join(MARK_FILE), other);
        // 使用者自己的另一個 skill（沒標記、spine 裡沒有）也不收
        write(&home.join("skills/private/SKILL.md"), "x");

        let r = run(&env, cli);
        assert_eq!(
            outcomes(&r),
            [
                ("mine", &Outcome::Conflict(Conflict::UserOwned)),
                (
                    "shared",
                    &Outcome::Conflict(Conflict::OtherSpine("/elsewhere/spine".into()))
                ),
            ]
        );
        assert_eq!(
            std::fs::read_to_string(home.join("skills/mine/SKILL.md")).unwrap(),
            "使用者自己寫的"
        );
        assert!(!home.join("skills/mine").join(MARK_FILE).exists());
        assert_eq!(
            std::fs::read_to_string(home.join("skills/shared/SKILL.md")).unwrap(),
            "別的 spine 的"
        );
        assert!(home.join("skills/theirs/SKILL.md").exists());
        assert!(home.join("skills/private/SKILL.md").exists());
        let p = shared_skills::pending(&sp, home).unwrap();
        assert_eq!(p.conflict, ["mine", "shared"]);
        let line = shared_skills::summary(&r);
        assert!(
            line.contains("衝突 mine（同名資料夾不是 ctx 發的，不覆蓋）"),
            "{line}"
        );
    }
}

/// 保留名稱、不合法名稱、沒有 SKILL.md、超過上限：略過並回報；目的地什麼都不寫。
#[test]
fn reserved_bad_names_and_limits_are_skipped() {
    for cli in AgentCli::ALL {
        let (t, sp) = spine();
        let env = env_in(t.path(), &sp);
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
        write(&s.join("README.md"), "這裡放共享 skill");
        write(&s.join(".hidden/SKILL.md"), "x");

        let r = run(&env, cli);
        let got = outcomes(&r);
        assert_eq!(got.len(), 5, "{got:?}");
        assert_eq!(got[0], ("Bad_Name", &Outcome::Skipped(SkipReason::BadName)));
        assert!(matches!(
            got[1],
            ("big-file", Outcome::Skipped(SkipReason::FileTooLarge { file, .. })) if file == "blob.bin"
        ));
        assert!(matches!(
            got[2],
            ("big-total", Outcome::Skipped(SkipReason::TooLarge { .. }))
        ));
        assert_eq!(got[3], ("no-md", &Outcome::Skipped(SkipReason::NoSkillMd)));
        assert_eq!(
            got[4],
            ("repo-context", &Outcome::Skipped(SkipReason::Reserved))
        );
        assert!(!env.home(cli).join("skills").exists());
        let line = shared_skills::summary(&r);
        assert!(
            line.starts_with("略過 Bad_Name（資料夾名只能用小寫英數與 -）"),
            "{line}"
        );
        assert!(
            line.contains("repo-context（repo-context 是內建 skill 的名字）"),
            "{line}"
        );

        // 內建 skill 裝好之後，spine 的同名資料夾也不會蓋過它
        agentsetup::install_skill(env.home(cli)).unwrap();
        run(&env, cli);
        assert_eq!(agentsetup::skill_status(env.home(cli)), "current");
    }
}

/// 指到 skill 資料夾外面的 symlink 不跟（裡面的照發成一般檔）；skill 資料夾本身是 symlink 也不跟。
#[cfg(unix)]
#[test]
fn symlinks_outside_are_not_followed() {
    use std::os::unix::fs::symlink;
    for cli in AgentCli::ALL {
        let (t, sp) = spine();
        let env = env_in(t.path(), &sp);
        let src = sp.join("skills/linky");
        write(&src.join("SKILL.md"), &skill_md("x"));
        write(&src.join("inner.md"), "inner\n");
        let secret = t.path().join("secret.txt");
        std::fs::write(&secret, "不該發出去").unwrap();
        symlink(&secret, src.join("leak.txt")).unwrap();
        symlink(t.path(), src.join("outdir")).unwrap();
        symlink(src.join("inner.md"), src.join("alias.md")).unwrap();
        symlink(t.path().join("nope"), src.join("broken")).unwrap();
        let elsewhere = t.path().join("elsewhere-skill");
        write(&elsewhere.join("SKILL.md"), &skill_md("x"));
        symlink(&elsewhere, sp.join("skills/linked-dir")).unwrap();

        let r = run(&env, cli);
        assert_eq!(
            outcomes(&r),
            [
                ("linked-dir", &Outcome::Skipped(SkipReason::Symlink)),
                ("linky", &Outcome::Installed),
            ]
        );
        let dest = env.home(cli).join("skills/linky");
        assert!(!dest.join("leak.txt").exists());
        assert!(!dest.join("outdir").exists());
        assert!(!dest.join("broken").exists());
        assert!(!env.home(cli).join("skills/linked-dir").exists());
        let alias = dest.join("alias.md");
        assert!(!std::fs::symlink_metadata(&alias).unwrap().is_symlink());
        assert_eq!(std::fs::read_to_string(alias).unwrap(), "inner\n");
        assert_eq!(r[1].ignored.len(), 3, "{:?}", r[1].ignored);
        let line = shared_skills::summary(&r);
        assert!(
            line.contains("linky/leak.txt（symlink 指到 skill 資料夾外，不跟）"),
            "{line}"
        );
        // 冪等（symlink 不會讓每次都算成更新）
        assert_eq!(run(&env, cli)[1].outcome, Outcome::Unchanged);
    }
}

/// setup 的計畫：沒有 `skills/`（或是空的）時跟以前一模一樣；有才在內建 skill 之後接一步，desc 列得出計畫。
#[test]
fn setup_plan_adds_step_only_when_there_are_shared_skills() {
    let (t, sp) = spine();
    let env = env_in(t.path(), &sp);
    let both = [AgentCli::Claude, AgentCli::Codex];
    assert_eq!(
        agentsetup::setup_plan(&env, &both, &[]),
        agentsetup::plan(&both)
    );
    std::fs::create_dir_all(sp.join("skills")).unwrap();
    assert_eq!(
        agentsetup::setup_plan(&env, &both, &[]),
        agentsetup::plan(&both)
    );
    // 這次沒點名、之前掛過的那家：內建 skill 換新（照舊）
    assert_eq!(
        agentsetup::setup_plan(&env, &[AgentCli::Claude], &both),
        [
            Step::SaveSpine,
            Step::Mcp(AgentCli::Claude),
            Step::Hooks(AgentCli::Claude),
            Step::Skill(AgentCli::Claude),
            Step::Skill(AgentCli::Codex),
        ]
    );

    write(&sp.join("skills/pdf-tools/SKILL.md"), &skill_md("處理 PDF"));
    let steps = agentsetup::setup_plan(&env, &[AgentCli::Claude], &both);
    assert_eq!(
        steps[3..],
        [
            Step::Skill(AgentCli::Claude),
            Step::SharedSkills(AgentCli::Claude),
            Step::Skill(AgentCli::Codex),
            Step::SharedSkills(AgentCli::Codex),
        ]
    );
    let desc = Step::SharedSkills(AgentCli::Codex).desc(&env);
    assert!(
        desc.starts_with("發 spine/skills/ 的共享 skill 到 "),
        "{desc}"
    );
    assert!(desc.ends_with(".codex/skills：新增 pdf-tools"), "{desc}");
    assert!(!env.codex_home.exists()); // desc 不寫
    let r = Step::SharedSkills(AgentCli::Codex).run(&env).unwrap();
    assert!(r.ok);
    assert_eq!(r.message, "共享 skill：新增 pdf-tools");
    assert!(env.codex_home.join("skills/pdf-tools/SKILL.md").exists());
    assert_eq!(
        Step::SharedSkills(AgentCli::Codex)
            .run(&env)
            .unwrap()
            .message,
        "共享 skill：已是最新 pdf-tools"
    );
    // 刪光之後，目的地還有本 spine 發的 → 這步還在（要收掉）
    std::fs::remove_dir_all(sp.join("skills/pdf-tools")).unwrap();
    let steps = agentsetup::setup_plan(&env, &both, &[]);
    assert!(steps.contains(&Step::SharedSkills(AgentCli::Codex)));
    assert!(!steps.contains(&Step::SharedSkills(AgentCli::Claude)));
    Step::SharedSkills(AgentCli::Codex).run(&env).unwrap();
    assert_eq!(
        agentsetup::setup_plan(&env, &both, &[]),
        agentsetup::plan(&both)
    );
}

/// spine 的 AGENTS.md：有共享 skill 才多一段，每個一行（名稱＋description 截短），仍守 60 行。
#[test]
fn agents_md_lists_shared_skills() {
    let (_t, sp) = spine();
    ctx_core::home::sync(&sp).unwrap();
    let before = std::fs::read_to_string(sp.join("AGENTS.md")).unwrap();
    assert!(!before.contains("共享 skill"));
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
    assert!(text.contains("\n## 共享 skill\n"), "{text}");
    let line = text
        .lines()
        .find(|l| l.starts_with("- **pdf-tools**："))
        .unwrap();
    assert!(line.ends_with('…') && line.chars().count() < 80, "{line}");
    assert!(text.lines().any(|l| l == "- **plain**"));
    assert!(!text.contains("Bad"));
    // 段落在 Repos 之後、資料來源之前
    assert!(text.find("## Repos").unwrap() < text.find("## 共享 skill").unwrap());
    assert!(text.find("## 共享 skill").unwrap() < text.find("## 資料來源").unwrap());
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
