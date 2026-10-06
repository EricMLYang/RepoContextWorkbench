//! 不變式 4：跨行程 lock——多進程同時寫脊椎＋registry，事後全部完好。對應 tests/test_concurrency.py。
//!
//! 用真實子行程（不是 thread）模擬 P16 多視窗 agent 併發寫入的常態：父測試重新執行本測試二進位，
//! 只跑 `child_writer`／`child_tagger`（靠環境變數 `CTX_TEST_CHILD` 進子行程模式；平常跑這兩個是空測試）。
//! 環境變數只設在子行程的 Command 上，不動本行程的全域環境。

use std::path::Path;
use std::process::{Command, Output};

use ctx_core::registry;
use ctx_core::spine::{self, EventType, Query};
use ctx_core::testkit::spine_with_repos;

const CHILD: &str = "CTX_TEST_CHILD";

fn run_children(test_name: &str, spine_dir: &Path) -> Vec<Output> {
    let exe = std::env::current_exe().unwrap();
    let handles: Vec<_> = (1..=4)
        .map(|who| {
            let exe = exe.clone();
            let spine_dir = spine_dir.to_path_buf();
            let test_name = test_name.to_string();
            std::thread::spawn(move || {
                Command::new(exe)
                    .args(["--exact", &test_name, "--test-threads=1", "--nocapture"])
                    .env(CHILD, format!("{}|{}", spine_dir.display(), who))
                    .output()
                    .unwrap()
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

fn child_args() -> Option<(String, u32)> {
    let v = std::env::var(CHILD).ok()?;
    let (d, who) = v.rsplit_once('|')?;
    Some((d.to_string(), who.parse().ok()?))
}

/// 子行程：8 筆事件＋改一次 registry（對應 Python WRITER 腳本）。
#[test]
fn child_writer() {
    let Some((d, who)) = child_args() else { return };
    for i in 0..8 {
        spine::append_event(
            &d,
            "decision",
            "manual",
            &[format!("#{}", who * 100 + i)],
            &format!("writer {who} event {i}"),
            None,
        )
        .unwrap();
    }
    let tags = serde_yaml::Value::Sequence(vec![format!("w{who}").into()]);
    registry::set_field(&d, "repo-a", "tags", tags).unwrap();
    println!("done {who}");
}

/// 子行程：10 次 tag_repo（對應 Python TAGGER 腳本）。
#[test]
fn child_tagger() {
    let Some((d, who)) = child_args() else { return };
    let none: &[&str] = &[];
    for i in 0..10 {
        registry::tag_repo(&d, "repo-a", &[format!("w{who}-{i}")], none).unwrap();
    }
}

fn assert_ok(results: &[Output]) {
    for r in results {
        assert!(
            r.status.success(),
            "{}{}",
            String::from_utf8_lossy(&r.stdout),
            String::from_utf8_lossy(&r.stderr)
        );
    }
}

#[test]
fn test_inv_concurrent_writers() {
    let (_t, d) = spine_with_repos();
    assert_ok(&run_children("child_writer", &d));
    // 脊椎：4 進程 × 8 筆全數寫入，且全部 parse 得回來（沒有撕裂行）
    let evs = spine::query(&d, &Query::of_type(EventType::Decision)).unwrap();
    assert_eq!(evs.len(), 32);
    let nums: std::collections::HashSet<&String> = evs
        .iter()
        .flat_map(|e| e.tokens.iter())
        .filter(|t| t.starts_with('#'))
        .collect();
    assert_eq!(nums.len(), 32);
    // registry：仍是合法 YAML、結構完整（就地更新沒被撕裂）
    let data: serde_yaml::Value =
        serde_yaml::from_str(&std::fs::read_to_string(d.join("registry.yaml")).unwrap()).unwrap();
    let ids: std::collections::BTreeSet<&str> = data["repos"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["repo-a", "repo-b"].into_iter().collect());
    assert!(registry::get_repo(&d, "repo-a").unwrap().tags()[0].starts_with('w'));
}

/// 不變式 4 補強（2026-09-25 架構檢查實測：2 進程各寫 40 筆只剩 41）：
/// registry 讀–改–寫要在同一把 lock 裡，否則後寫的蓋掉先寫的——YAML 沒撕裂但資料掉了。
#[test]
fn test_inv_registry_no_lost_updates() {
    let (_t, d) = spine_with_repos();
    assert_ok(&run_children("child_tagger", &d));
    let tags = registry::get_repo(&d, "repo-a").unwrap().tags();
    assert_eq!(tags.iter().filter(|t| t.starts_with('w')).count(), 40);
}

/// 寫到一半炸掉 → 原檔完好（先寫暫存檔再 rename），讀的人不會讀到半份 YAML。
/// Python 版 monkeypatch `yaml.safe_dump`；這裡讓暫存檔位置被一個資料夾佔住，寫暫存檔就失敗。
#[test]
fn test_registry_write_is_atomic() {
    let (_t, d) = spine_with_repos();
    let before = std::fs::read_to_string(d.join("registry.yaml")).unwrap();
    std::fs::create_dir(d.join(format!(".registry.yaml.{}.tmp", std::process::id()))).unwrap();
    let none: &[&str] = &[];
    assert!(registry::tag_repo(&d, "repo-a", &["x"], none).is_err());
    assert_eq!(
        std::fs::read_to_string(d.join("registry.yaml")).unwrap(),
        before
    );
}

#[test]
fn test_registry_transaction_rolls_back_on_error() {
    let (_t, d) = spine_with_repos();
    let r: ctx_core::Result<()> = registry::transaction(&d, |data| {
        data.repos.clear();
        Err(ctx_core::Error::invalid("中途失敗"))
    });
    assert!(r.is_err());
    assert_eq!(registry::load(&d).unwrap().repos.len(), 2);
}
