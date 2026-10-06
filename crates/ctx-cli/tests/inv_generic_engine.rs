//! 不變式 5：引擎原始碼不得含私有專名（引擎/私有分離鐵律的可證偽版）。對應 tests/test_inv_generic_engine.py。
//! Python 掃 `repo_context/**/*.py`；Rust 版掃引擎與 CLI 的原始碼（`crates/*/src/**/*.rs`）。

use std::path::{Path, PathBuf};

const FORBIDDEN: [&str; 4] = ["Eric", "PM_Head", "AgentCodingPM", "1806011"];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

#[test]
fn test_inv_no_private_names_in_engine() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut files = Vec::new();
    for c in ["ctx-core", "ctx-cli"] {
        rs_files(&crates.join(c).join("src"), &mut files);
    }
    assert!(!files.is_empty());
    let mut hits = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        for w in FORBIDDEN {
            if text.contains(w) {
                hits.push(format!("{}: {w}", f.display()));
            }
        }
    }
    assert!(hits.is_empty(), "引擎裡出現私有專名: {hits:?}");
}
