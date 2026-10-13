//! repo 名片的看與改（F-57，Rust 才有）：`repo_profile`、`describe_repo`。

use super::*;

/// Rust 才有（F-57）：一個 repo 的名片（介紹、角色、關鍵字；沒填介紹時從 README 推一句，標 `readme`）。
pub(super) fn a_repo_profile(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let ProfileArgs { id } = parse("repo_profile", p)?;
    profile_json(&registry::get_repo(d, &id)?, None)
}

fn profile_json(r: &registry::Repo, changed: Option<bool>) -> Result<Value> {
    let mut v = serde_json::to_value(crate::profile::profile(r))?;
    v["ok"] = json!(true);
    if let Some(c) = changed {
        v["changed"] = json!(c);
    }
    Ok(v)
}

/// Rust 才有（F-57）：App 編輯名片。表單送三欄的完整內容，**只有跟 registry 不同的欄位**才寫
/// （走 `registry_describe`，同 CLI／MCP 的驗證與留痕）；都沒變就不寫、不記事件。
/// 關鍵字可用逗號、頓號、全形逗號或換行分隔，去重、保留順序。回傳更新後的名片＋`changed`。
#[derive(serde::Deserialize)]
struct DescribeArgs {
    id: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    keywords: Option<String>,
}

fn split_keywords(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in s.split([',', '，', '、', '\n']).map(str::trim) {
        if !k.is_empty() && !out.iter().any(|o| o == k) {
            out.push(k.to_string());
        }
    }
    out
}

pub(super) fn a_describe_repo(d: &Path, p: &Value, _t: Option<&dyn Terms>) -> Result<Value> {
    let a: DescribeArgs = parse("describe_repo", p)?;
    let cur = registry::get_repo(d, &a.id)?;
    let mut args = Map::new();
    for (k, new, old) in [
        ("description", &a.description, cur.description()),
        ("role", &a.role, cur.role()),
    ] {
        if let Some(n) = new.as_deref().map(str::trim) {
            if n != old.as_deref().map(str::trim).unwrap_or("") {
                args.insert(k.into(), json!(n));
            }
        }
    }
    if let Some(k) = a.keywords.as_deref().map(split_keywords) {
        if k != cur.keywords() {
            args.insert("keywords".into(), json!(k));
        }
    }
    if args.is_empty() {
        return profile_json(&cur, Some(false));
    }
    args.insert("id".into(), json!(a.id));
    ops::call(d, "registry_describe", Value::Object(args), Via::Ui)?;
    profile_json(&registry::get_repo(d, &a.id)?, Some(true))
}
