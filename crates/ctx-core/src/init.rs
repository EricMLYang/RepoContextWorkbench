//! `ctx init` 的 scaffold：建 spine repo 的目錄、預設 config／registry／.gitignore，git init。
//! 對應 Python `cli/admin.cmd_init`（搬進引擎，CLI、桌面 App、測試共用）。

use std::path::Path;
use std::process::Command;

use crate::{Error, Result};

/// 對應 `cli/common.SCAFFOLD_CONFIG`。
pub const SCAFFOLD_CONFIG: &str = include_str!("assets/scaffold_config.yaml");
/// 對應 `cli/common.SCAFFOLD_GITIGNORE`。
pub const SCAFFOLD_GITIGNORE: &str = ".engine.lock\n.state/\n.agent_logs/\n";

/// 對應 `cli/admin.cmd_init`（不印訊息，由入口印）。已存在的檔不覆寫。
pub fn init_spine(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir.join("spine").join("events"))?;
    std::fs::create_dir_all(dir.join("groups"))?;
    std::fs::create_dir_all(dir.join("incubator"))?;
    for (name, content) in [
        ("config.yaml", SCAFFOLD_CONFIG),
        ("registry.yaml", "repos: []\ngroups: []\n"),
        (".gitignore", SCAFFOLD_GITIGNORE),
    ] {
        let p = dir.join(name);
        if !p.exists() {
            std::fs::write(&p, content)?;
        }
    }
    if !dir.join(".git").exists() {
        let st = Command::new("git").args(["init", "-q"]).current_dir(dir).status()?;
        if !st.success() {
            return Err(Error::External(format!("git init 失敗：{}", dir.display())));
        }
    }
    Ok(())
}
