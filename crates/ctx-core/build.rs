//! 把 build 時的 git commit 寫進執行檔（F-114：App 與 `ctx --version` 看得到現在跑的是哪一版）。
//! 不在 git 裡 build（例如原始碼壓縮檔）就全空，執行時當「不知道」。只在 HEAD 動了才重跑。

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("cargo 一定會給");
    let root = Path::new(&manifest).join("../..");
    let commit = git(&root, &["rev-parse", "--short=7", "HEAD"]);
    let date = git(
        &root,
        &["log", "-1", "--format=%cd", "--date=format:%Y-%m-%d %H:%M"],
    );
    let source = git(&root, &["rev-parse", "--show-toplevel"]);
    // HEAD 換分支、commit（refs 或 packed-refs 變）才重跑；worktree 的 .git 是檔案，問 git 要真的位置
    for (flag, files) in [
        ("--git-dir", &["HEAD"][..]),
        ("--git-common-dir", &["refs/heads", "packed-refs"][..]),
    ] {
        if let Some(d) = git(&root, &["rev-parse", flag]) {
            let d = if Path::new(&d).is_absolute() {
                d.into()
            } else {
                root.join(d)
            };
            for f in files {
                println!("cargo:rerun-if-changed={}", d.join(f).display());
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rustc-env=CTX_BUILD_COMMIT={}",
        commit.clone().unwrap_or_default()
    );
    println!(
        "cargo:rustc-env=CTX_BUILD_DATE={}",
        date.clone().unwrap_or_default()
    );
    println!(
        "cargo:rustc-env=CTX_SOURCE_DIR={}",
        source.unwrap_or_default()
    );
    let suffix = match (commit, date) {
        (Some(c), Some(d)) => format!(" ({c} {d})"),
        _ => String::new(),
    };
    println!("cargo:rustc-env=CTX_VERSION_SUFFIX={suffix}");
}
