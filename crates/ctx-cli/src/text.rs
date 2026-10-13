//! 人話輸出：CLI 與 MCP 管理層印同一份字的，放這裡共用（只做排版，資料由 `ctx_core` 給）。

use std::path::Path;

use ctx_core::registry;

/// `ctx registry relations [id]`：給 id＝這個 repo 的關係（帶方向）；不給＝全部。沒有關係時列出可用的 kind。
pub fn relations(d: &Path, id: Option<&str>) -> ctx_core::Result<String> {
    let lines: Vec<String> = match id.filter(|s| !s.is_empty()) {
        Some(id) => registry::relations_of(d, id)?
            .iter()
            .map(|r| {
                let head = if r.direction == registry::Direction::Out {
                    format!("{id} {}→ {}", r.label, r.peer)
                } else {
                    format!("{id} 的{} {}", r.label, r.peer)
                };
                format!("{head}（{}）{}", r.kind, crate::note_tail(&r.note))
            })
            .collect(),
        None => {
            let kinds = registry::relation_kinds(d)?;
            registry::relations(d, None)?
                .iter()
                .map(|r| {
                    let fwd = kinds
                        .get(&r.kind)
                        .map(|k| k.forward.clone())
                        .unwrap_or_else(|| r.kind.clone());
                    format!(
                        "{} {fwd}→ {}（{}）{}",
                        r.from,
                        r.to,
                        r.kind,
                        crate::note_tail(&r.note)
                    )
                })
                .collect()
        }
    };
    if lines.is_empty() {
        let kinds = registry::relation_kinds(d)?;
        let names: Vec<&str> = kinds.0.iter().map(|(k, _)| k.as_str()).collect();
        return Ok(format!("（無關係）可用 kind：{}", names.join(", ")));
    }
    Ok(lines.join("\n"))
}
