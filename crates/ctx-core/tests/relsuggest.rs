//! Rust 才有（F-33）：相關 repo 建議（`relsuggest::suggest` 純函數）。

use std::collections::{HashMap, HashSet};

use ctx_core::knowledge::{tokenize, Doc};
use ctx_core::relsuggest::{suggest, HUB_MIN};

fn doc(repo: &str, i: usize, text: &str) -> Doc {
    let tf = tokenize(text).into_iter().fold(HashMap::new(), |mut m, t| {
        *m.entry(t).or_insert(0.0) += 1.0;
        m
    });
    Doc {
        repo: repo.into(),
        path: format!("/{repo}/{i}.md"),
        rel: format!("{i}.md"),
        tf,
        len: 1.0,
        htf: HashMap::new(),
        hlen: 0.0,
        mtime: 0.0,
    }
}

fn names(ids: &[&str]) -> Vec<(String, Vec<String>)> {
    ids.iter()
        .map(|i| (i.to_string(), vec![i.to_lowercase()]))
        .collect()
}

#[test]
fn mentions_make_a_suggestion_with_reason() {
    let mut docs = Vec::new();
    for i in 0..4 {
        docs.push(doc("pm", i, &format!("本週 Shop_App 的進度與排程 {i}")));
    }
    docs.push(doc("shop_app", 0, "店型與檔期"));
    let s = suggest(&docs, &names(&["pm", "Shop_App"]), &[], &HashSet::new());
    assert_eq!(s.len(), 1);
    assert_eq!((s[0].a.as_str(), s[0].b.as_str()), ("Shop_App", "pm"));
    assert_eq!(s[0].b_mentions_a, 4);
    assert!(s[0].reason().contains("4 份文件互相提到"));
}

#[test]
fn few_mentions_or_existing_relation_are_not_suggested() {
    let docs = vec![doc("pm", 0, "提到 shop_app 一次"), doc("shop_app", 0, "x")];
    assert!(suggest(&docs, &names(&["pm", "shop_app"]), &[], &HashSet::new()).is_empty());
    let mut many = Vec::new();
    for i in 0..5 {
        many.push(doc("pm", i, "shop_app 進度"));
    }
    let rel: HashSet<(String, String)> = [("pm".to_string(), "shop_app".to_string())].into();
    assert!(suggest(&many, &names(&["pm", "shop_app"]), &[], &rel).is_empty());
}

#[test]
fn repo_that_mentions_many_repos_is_hinted_as_pm() {
    let projects: Vec<String> = (0..HUB_MIN).map(|i| format!("proj{i}")).collect();
    let mut docs = Vec::new();
    for (k, p) in projects.iter().enumerate() {
        for i in 0..3 {
            docs.push(doc("pm", k * 10 + i, &format!("{p} 週報")));
        }
        docs.push(doc(p, 0, "實作細節"));
    }
    let mut ids: Vec<&str> = projects.iter().map(String::as_str).collect();
    ids.push("pm");
    let s = suggest(&docs, &names(&ids), &[], &HashSet::new());
    assert_eq!(s.len(), HUB_MIN);
    for x in &s {
        let (from, _, kind) = x.hint.clone().expect("中樞要有推測");
        assert_eq!((from.as_str(), kind.as_str()), ("pm", "pm-of"));
    }
}
