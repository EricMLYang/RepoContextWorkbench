//! 知識索引庫（`.state/knowledge.db`，N-51，Rust 才有）：結果要跟「在記憶體裡從頭建索引再算分」逐位元組相同，
//! 增量更新後等於從頭重建，多執行緒同時查與更新不壞，版本不符與損毀會重建，舊 JSON 會被刪。
//!
//! 對照組＝`knowledge::search(.., None, ..)`／`knowledge::build_docs(.., None)`（不給 spine 就不碰索引庫）。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ctx_core::knowledge::{self, Boosts, PathBoost};
use ctx_core::pack::RepoRef;
use ctx_core::registry::{self, AddRepo};
use ctx_core::testkit::{git, make_git_repo, spine};

struct Corpus {
    _t: tempfile::TempDir,
    sp: PathBuf,
    alpha: PathBuf,
    beta: PathBuf,
    gamma: PathBuf,
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn set_mtime(p: &Path, t: SystemTime) {
    std::fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn days_ago(d: u64) -> SystemTime {
    SystemTime::now() - Duration::from_secs(d * 86400)
}

/// 中英混合、有標題／frontmatter、有封存路徑、有秘密檔、有未追蹤與被忽略的檔、有舊檔（不吃近 30 天加分）。
fn corpus() -> Corpus {
    let (t, sp) = spine();
    let alpha = make_git_repo(t.path(), "alpha", 0, 0, true);
    let beta = make_git_repo(t.path(), "beta", 0, 0, true);
    let gamma = make_git_repo(t.path(), "gamma", 0, 0, false);
    write(
        &alpha.join("docs").join("快取設計.md"),
        "---\ntitle: 快取設計\ntags: [cache, LRU]\n---\n# 快取設計\n\n## 淘汰策略\nLRU 淘汰策略：最久沒用的先丟。\n\
         cache eviction 與 widget 快取的取捨。\n## Rust 引擎\nRust 引擎負責索引。v1.2-rc 版開始用 SQLite。\n",
    );
    write(
        &alpha.join("archive").join("舊文件.md"),
        "# 舊文件\n封存的舊文件，談快取與 LRU。\n",
    );
    write(
        &alpha.join("notes").join("會話.md"),
        "# 會話狀態\nhook 回報會話狀態：處理中、等你、做完。\nsession status hook\n",
    );
    write(
        &beta.join("notes").join("LRU.md"),
        "# LRU cache\nThe LRU cache evicts the least recently used entry.\n快取淘汰 LRU。\n",
    );
    write(
        &beta.join("keys.md"),
        "# 部署\nAKIAABCDEFGHIJKLMNOP 部署金鑰\n",
    );
    write(
        &beta.join("決策平台.md"),
        "# 大數據自動決策平台\n決策平台需要特徵倉庫與即時推論。\n",
    );
    write(&gamma.join(".gitignore"), "ignored.md\n");
    write(&gamma.join("ignored.md"), "# 被忽略\n快取 LRU\n");
    write(
        &gamma.join("untracked.md"),
        "# 未追蹤\n標題 frontmatter 都沒有的草稿，提到 widget 與中。\n",
    );
    write(
        &gamma.join("guide").join("README.md"),
        "## 指南\n計劃：已知結論是快取要用 LRU。\n中\n",
    );
    set_mtime(&beta.join("notes").join("LRU.md"), days_ago(60));
    set_mtime(&alpha.join("archive").join("舊文件.md"), days_ago(90));
    for (id, p) in [("alpha", &alpha), ("beta", &beta), ("gamma", &gamma)] {
        registry::add_repo(&sp, id, p, AddRepo::default()).unwrap();
    }
    Corpus {
        _t: t,
        sp,
        alpha,
        beta,
        gamma,
    }
}

fn refs(sp: &Path) -> Vec<RepoRef> {
    registry::load(sp)
        .unwrap()
        .repos
        .iter()
        .map(RepoRef::from)
        .collect()
}

fn boosts() -> Boosts {
    let mut b = Boosts::default();
    b.repos.insert("beta".into(), (2.0, "鄰居 repo".into()));
    b.paths.push(PathBoost {
        repo: "alpha".into(),
        prefix: "docs/".into(),
        factor: 1.5,
        reason: "export 內".into(),
    });
    b
}

/// 中文 bigram、英文、混合、單字元、沒命中、斷不出詞。
const QUERIES: [&str; 20] = [
    "快取",
    "快取設計",
    "淘汰策略",
    "LRU",
    "lru cache eviction",
    "LRU 快取",
    "widget 快取",
    "會話 狀態 hook",
    "session status",
    "決策平台 特徵倉庫",
    "部署金鑰",
    "Rust 引擎 SQLite",
    "v1.2-rc",
    "計劃 已知結論",
    "標題 frontmatter",
    "封存 舊文件",
    "中",
    "完全沒有的詞組",
    "zzzqqq xyzzy",
    "the of and",
];

/// 每個查詢 × 兩組 boosts × 兩種 limit：索引庫的結果＝對照組（PartialEq 含分數原值，再比一次序列化字串）。
fn assert_same(sp: &Path, refs: &[RepoRef], label: &str) {
    let mut hit = 0;
    for q in QUERIES {
        for b in [Boosts::default(), boosts()] {
            for limit in [3, 50] {
                let db = knowledge::search(refs, q, limit, Some(sp), &[], &b);
                let mem = knowledge::search(refs, q, limit, None, &[], &b);
                assert_eq!(db, mem, "{label}：{q}");
                assert_eq!(
                    serde_json::to_string(&db).unwrap(),
                    serde_json::to_string(&mem).unwrap()
                );
                hit += usize::from(!db.files.is_empty());
            }
        }
    }
    assert!(hit >= 4 * 10, "{label}：語料要真的命中大部分查詢（{hit}）");
    assert_eq!(
        knowledge::build_docs(refs, Some(sp)),
        knowledge::build_docs(refs, None),
        "{label}：整份 doc"
    );
    // 真的走了索引庫（不是退回記憶體、自己跟自己比）
    assert_eq!(knowledge::index_fallbacks(), 0, "{label}：不該退回記憶體");
    let c = rusqlite::Connection::open(db_path(sp)).unwrap();
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok", "{label}：庫完整");
    let n: i64 = c
        .query_row("SELECT count(*) FROM docs WHERE secret = 0", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(n > 0, "{label}：庫裡有檔");
}

/// 索引庫的內容（不含 id）：(repo, path, mtime 位元, secret, len, hlen)＋(repo, path, 欄, 詞, tf)。
type Dump = (
    BTreeSet<(String, String, u64, i64, i64, i64)>,
    BTreeSet<(String, String, String, String, i64)>,
);

fn dump(sp: &Path) -> Dump {
    let c = rusqlite::Connection::open(sp.join(".state").join("knowledge.db")).unwrap();
    let mut st = c
        .prepare("SELECT repo, path, mtime, secret, len, hlen FROM docs")
        .unwrap();
    let docs = st
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get::<_, f64>(2)?.to_bits(),
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut posts = BTreeSet::new();
    for field in ["body", "head"] {
        let mut st = c
            .prepare(&format!(
                "SELECT docs.repo, docs.path, terms.term, p.tf FROM {field} p \
                 JOIN docs ON docs.id = p.doc JOIN terms ON terms.id = p.term"
            ))
            .unwrap();
        for r in st
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    field.to_string(),
                    r.get(2)?,
                    r.get(3)?,
                ))
            })
            .unwrap()
        {
            posts.insert(r.unwrap());
        }
    }
    (docs, posts)
}

fn generation(sp: &Path) -> i64 {
    knowledge::index_generation(sp).expect("索引庫要在")
}

fn db_path(sp: &Path) -> PathBuf {
    sp.join(".state").join("knowledge.db")
}

#[test]
fn db_results_equal_in_memory_oracle() {
    let c = corpus();
    let refs = refs(&c.sp);
    assert_same(&c.sp, &refs, "冷建");
    assert_same(&c.sp, &refs, "暖（全部從索引庫）");
    // 子範圍（主場只有一兩個 repo）與排除來源檔：N、平均長度、df 都只算範圍內的
    let sub = vec![refs[1].clone(), refs[2].clone()];
    assert_same(&c.sp, &sub, "子範圍");
    let ex = vec![c.alpha.join("docs").join("快取設計.md")];
    for q in QUERIES {
        let db = knowledge::search(&refs, q, 8, Some(&c.sp), &ex, &boosts());
        let mem = knowledge::search(&refs, q, 8, None, &ex, &boosts());
        assert_eq!(db, mem, "排除來源檔：{q}");
        assert!(db.files.iter().all(|f| f.file != "docs/快取設計.md"));
    }
    // 秘密檔、被忽略的檔不在
    let r = knowledge::search(&refs, "部署金鑰", 8, Some(&c.sp), &[], &Boosts::default());
    assert!(r.files.iter().all(|f| f.file != "keys.md"));
    let r = knowledge::search(&refs, "被忽略", 8, Some(&c.sp), &[], &Boosts::default());
    assert!(r.files.iter().all(|f| f.file != "ignored.md"));
}

#[test]
fn incremental_updates_equal_full_rebuild() {
    let c = corpus();
    let refs0 = refs(&c.sp);
    assert_same(&c.sp, &refs0, "初次");
    let g0 = generation(&c.sp);
    // 沒變動：不寫
    let _ = knowledge::search(&refs0, "快取", 8, Some(&c.sp), &[], &Boosts::default());
    assert_eq!(generation(&c.sp), g0, "沒變動時不該寫索引庫");

    let fresh_dump = |c: &Corpus, refs: &[RepoRef]| {
        // 從頭重建一份（刪庫再查）拿來比
        let before = dump(&c.sp);
        let _ = std::fs::remove_file(db_path(&c.sp));
        let _ = knowledge::search(refs, "快取", 8, Some(&c.sp), &[], &Boosts::default());
        assert_eq!(dump(&c.sp), before, "增量後的索引庫＝從頭重建");
    };

    // 改一個檔（內容與 mtime 都變）
    let f = c.alpha.join("docs").join("快取設計.md");
    write(
        &f,
        "# 快取設計（改版）\n改成 ARC 淘汰策略，不再用 LRU。\nwidget widget\n",
    );
    set_mtime(&f, SystemTime::now() + Duration::from_secs(5));
    assert_same(&c.sp, &refs0, "改一個檔");
    assert_eq!(generation(&c.sp), g0 + 1, "改一個檔＝一次寫入");
    fresh_dump(&c, &refs0);

    // 加一個檔
    write(
        &c.beta.join("notes").join("新檔.md"),
        "# 新檔\nARC 淘汰策略與 LRU 的比較。\n",
    );
    assert_same(&c.sp, &refs0, "加一個檔");
    fresh_dump(&c, &refs0);

    // 刪一個檔
    std::fs::remove_file(c.gamma.join("guide").join("README.md")).unwrap();
    assert_same(&c.sp, &refs0, "刪一個檔");
    let gone = c
        .gamma
        .join("guide")
        .join("README.md")
        .to_string_lossy()
        .into_owned();
    assert!(
        dump(&c.sp).0.iter().all(|d| d.1 != gone),
        "刪掉的檔不留在庫裡"
    );
    fresh_dump(&c, &refs0);

    // 檔變成秘密檔，再變回來
    let s = c.gamma.join("untracked.md");
    write(
        &s,
        "# 未追蹤\nghp_abcdefghijklmnopqrstuvwxyz0123456789 widget\n",
    );
    set_mtime(&s, SystemTime::now() + Duration::from_secs(7));
    assert_same(&c.sp, &refs0, "變成秘密檔");
    let r = knowledge::search(&refs0, "widget", 50, Some(&c.sp), &[], &Boosts::default());
    assert!(
        r.files.iter().all(|f| f.file != "untracked.md"),
        "秘密檔不進結果"
    );
    write(&s, "# 未追蹤\nwidget 又乾淨了\n");
    set_mtime(&s, SystemTime::now() + Duration::from_secs(9));
    assert_same(&c.sp, &refs0, "變回乾淨");

    // 改名（git mv，未 commit）與改名後 commit
    git(&c.alpha, &["add", "-A"], &[]);
    git(&c.alpha, &["commit", "-q", "-m", "corpus"], &[]);
    assert_same(&c.sp, &refs0, "commit 之後（未追蹤變成追蹤）");
    git(&c.alpha, &["mv", "notes/會話.md", "notes/會話狀態.md"], &[]);
    assert_same(&c.sp, &refs0, "改名");
    git(&c.alpha, &["add", "-A"], &[]);
    git(&c.alpha, &["commit", "-q", "-m", "rename"], &[]);
    assert_same(&c.sp, &refs0, "改名後 commit");
    fresh_dump(&c, &refs0);

    // 切分支：另一個分支上有不同的檔、不同的內容
    git(&c.alpha, &["checkout", "-q", "-b", "side"], &[]);
    write(
        &c.alpha.join("side.md"),
        "# 分支上才有\nARC 淘汰策略 side branch\n",
    );
    write(
        &c.alpha.join("notes").join("會話狀態.md"),
        "# 分支版\nhook 狀態 side\n",
    );
    git(&c.alpha, &["add", "-A"], &[]);
    git(&c.alpha, &["commit", "-q", "-m", "side"], &[]);
    assert_same(&c.sp, &refs0, "分支 side");
    let main = String::from_utf8(
        std::process::Command::new("git")
            .args(["-C"])
            .arg(&c.alpha)
            .args(["rev-parse", "--abbrev-ref", "@{-1}"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    git(&c.alpha, &["checkout", "-q", main.trim()], &[]);
    assert!(!c.alpha.join("side.md").exists());
    assert_same(&c.sp, &refs0, "切回原分支");
    fresh_dump(&c, &refs0);

    // 從 registry 移除一個 repo：它的檔要清掉
    registry::remove_repo(&c.sp, "beta").unwrap();
    let refs1 = refs(&c.sp);
    assert_eq!(refs1.len(), 2);
    assert_same(&c.sp, &refs1, "移除 repo");
    assert!(
        dump(&c.sp).0.iter().all(|d| d.0 != "beta"),
        "移除的 repo 的檔不留在庫裡"
    );
    fresh_dump(&c, &refs1);
    let g = generation(&c.sp);
    let _ = knowledge::search(&refs1, "LRU", 8, Some(&c.sp), &[], &Boosts::default());
    assert_eq!(generation(&c.sp), g, "沒變動時不該寫索引庫");
}

#[test]
fn concurrent_update_and_query_do_not_fail() {
    let c = corpus();
    let refs = refs(&c.sp);
    let _ = knowledge::search(&refs, "快取", 8, Some(&c.sp), &[], &Boosts::default());
    let f = c.alpha.join("notes").join("會話.md");
    std::thread::scope(|s| {
        let writer = s.spawn(|| {
            for i in 0..15 {
                write(
                    &f,
                    &format!("# 會話狀態 {i}\nhook 回報會話狀態 第{i}版 快取\n"),
                );
                set_mtime(&f, SystemTime::now() + Duration::from_secs(10 + i));
                let r =
                    knowledge::search(&refs, "會話 快取", 8, Some(&c.sp), &[], &Boosts::default());
                assert!(r.note.is_none() && r.indexed_files.unwrap() > 0);
            }
        });
        let reader = s.spawn(|| {
            for _ in 0..30 {
                let r = knowledge::search(&refs, "LRU 快取", 8, Some(&c.sp), &[], &boosts());
                assert!(r.note.is_none() && !r.files.is_empty());
                let docs = knowledge::build_docs(&refs, Some(&c.sp));
                assert!(!docs.is_empty());
            }
        });
        writer.join().unwrap();
        reader.join().unwrap();
    });
    assert_same(&c.sp, &refs, "並行之後");
    let v: i64 = rusqlite::Connection::open(db_path(&c.sp))
        .unwrap()
        .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
        .map(|s| i64::from(s == "ok"))
        .unwrap();
    assert_eq!(v, 1, "並行之後庫完整");
}

/// 別的行程拿著寫入鎖（例：正在冷建）：查詢不等到失敗，也不寫；結果照樣對（剛算的那幾份在記憶體裡算分）。
#[test]
fn query_succeeds_while_another_writer_holds_the_lock() {
    let c = corpus();
    let refs = refs(&c.sp);
    let _ = knowledge::search(&refs, "快取", 8, Some(&c.sp), &[], &Boosts::default());
    let g = generation(&c.sp);
    let f = c.beta.join("決策平台.md");
    write(&f, "# 決策平台\n改過：決策平台不需要特徵倉庫了。\n");
    set_mtime(&f, SystemTime::now() + Duration::from_secs(5));
    let other = rusqlite::Connection::open(db_path(&c.sp)).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let db = knowledge::search(&refs, "決策平台 特徵倉庫", 8, Some(&c.sp), &[], &boosts());
    let mem = knowledge::search(&refs, "決策平台 特徵倉庫", 8, None, &[], &boosts());
    assert_eq!(db, mem);
    other.execute_batch("ROLLBACK").unwrap();
    assert_eq!(generation(&c.sp), g, "拿不到寫入鎖就不寫");
    // 鎖放掉之後下一次查詢補寫
    assert_same(&c.sp, &refs, "鎖放掉之後");
    assert_eq!(generation(&c.sp), g + 1);
}

#[test]
fn version_mismatch_rebuilds() {
    let c = corpus();
    let refs = refs(&c.sp);
    let _ = knowledge::search(&refs, "快取", 8, Some(&c.sp), &[], &Boosts::default());
    {
        let db = rusqlite::Connection::open(db_path(&c.sp)).unwrap();
        // 舊版留下的垃圾：版本號不符的庫裡多一份不存在的檔
        db.execute(
            "INSERT INTO docs(repo, path, rel, mtime, secret, len, hlen, terms) \
             VALUES('alpha', '/nope.md', 'nope.md', 1.0, 0, 1, 0, x'0000')",
            [],
        )
        .unwrap();
        db.pragma_update(None, "user_version", 999).unwrap();
    }
    assert_same(&c.sp, &refs, "版本不符");
    let db = rusqlite::Connection::open(db_path(&c.sp)).unwrap();
    let v: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, knowledge::DB_SCHEMA_VERSION);
    let junk: i64 = db
        .query_row(
            "SELECT count(*) FROM docs WHERE path = '/nope.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(junk, 0, "整個重建");
    assert_eq!(generation(&c.sp), 1, "重建後只寫過一次");
}

#[test]
fn corrupt_db_is_rebuilt() {
    let c = corpus();
    let refs = refs(&c.sp);
    // 1) 根本不是 SQLite
    std::fs::create_dir_all(c.sp.join(".state")).unwrap();
    std::fs::write(db_path(&c.sp), vec![0xAB; 8192]).unwrap();
    assert_same(&c.sp, &refs, "不是資料庫");
    assert!(knowledge::index_generation(&c.sp).is_some(), "刪掉重建");
    // 2) 表不見了（版本號還對）
    rusqlite::Connection::open(db_path(&c.sp))
        .unwrap()
        .execute_batch("DROP TABLE body;")
        .unwrap();
    assert_same(&c.sp, &refs, "表不見");
    // 3) terms 欄被寫壞（解不開）
    rusqlite::Connection::open(db_path(&c.sp))
        .unwrap()
        .execute("UPDATE docs SET terms = x'ff'", [])
        .unwrap();
    let f = c.alpha.join("docs").join("快取設計.md");
    write(&f, "# 快取設計\n改一下，逼它刪舊 posting。\n");
    set_mtime(&f, SystemTime::now() + Duration::from_secs(5));
    assert_same(&c.sp, &refs, "terms 欄壞掉");
    // 4) 檔頭還在、後面被截斷
    let bytes = std::fs::read(db_path(&c.sp)).unwrap();
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(c.sp.join(".state").join(format!("knowledge.db{suffix}")));
    }
    std::fs::write(db_path(&c.sp), &bytes[..bytes.len().min(4096 * 3)]).unwrap();
    assert_same(&c.sp, &refs, "截斷");
}

#[test]
fn legacy_json_is_removed_after_db_is_built() {
    let c = corpus();
    let refs = refs(&c.sp);
    let legacy = c.sp.join(".state").join("knowledge_index.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "{}").unwrap();
    let r = knowledge::search(&refs, "快取", 8, Some(&c.sp), &[], &Boosts::default());
    assert!(!r.files.is_empty());
    assert!(db_path(&c.sp).exists());
    assert!(!legacy.exists(), "建好 db 後刪掉舊 JSON");
}
