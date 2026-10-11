//! P1 registry（register/edit/query/audit）＋ P2 組（同檔 groups 段）。對應 Python `repo_context/registry/`。
//!
//! registry.yaml 就地更新——寫入必經同一把 lock（撕裂寫入比 append 更可怕），讀–改–寫在同一把鎖內
//! （[`transaction`]），寫入是原子的（暫存檔＋rename）。不認得的鍵與鍵順序原樣保留（Python 版也讀寫這份檔）。
//! audit 抄 render_registry.py 稽核思路：tier 漂移、路徑失效、paused 無 resume_when、未登記 repo。
//!
//! 拆成 store（讀寫與 transaction）／repos／groups／relations／audit，這裡把公開名稱接出去，
//! 呼叫端照舊 `registry::xxx`。

pub mod audit;
pub mod groups;
pub mod relations;
pub mod repos;
pub mod store;

pub use audit::{
    add_folder, audit, audit_with, last_commit_days, scan_dir, scan_dir_with_self, scan_register,
};
pub use groups::{
    add_group, add_group_members, add_group_with_landing, check_group_name, get_group,
    remove_group, remove_group_members, rename_group, resolve_group, set_group_field,
    set_group_members, split_ids, update_group, GroupRemoved, GroupRenamed, GroupUpdate,
    GroupUpdated,
};
pub use relations::{
    builtin_relation_kinds, exports_of, relate, related_ids, relation_kinds, relation_lines,
    relations, relations_of, remove_export, set_export, unrelate, Direction, ExportSet,
    RelationKind, RelationKinds, RelationRow, RelationView,
};
pub use repos::{
    add_repo, all_tags, get_repo, remove_repo, set_field, survival, tag_repo, AddRepo, Survival,
    Tier, TIERS,
};
pub use store::{load, save, transaction, Export, Group, RegistryData, Relation, Repo};
