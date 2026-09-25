"""P1 registry（register/edit/query/audit）＋ P2 組（同檔 groups 段）。

registry.yaml 就地更新——寫入必經同一把 lock（撕裂寫入比 append 更可怕）。
audit 抄 render_registry.py 稽核思路：tier 漂移、路徑失效、paused 無 resume_when、未登記 repo。

關係層（2026-09-08 組為單位輪）：關係存在**來源 repo** 的 `relations:` 欄（`{to, kind, note?}`，v1 §3.1 草案落地）。
詞彙表是起手式（用滿四週在週復盤修），私有層可用 config `relations.kinds` 擴充：
    relations:
      kinds:
        tests: {forward: 驗證, inverse: 被驗證於}
forward＝從來源 repo 讀的說法（A 規劃（PM）B）、inverse＝從目標 repo 讀的說法（B 的 PM 是 A）；
symmetric＝對稱關係兩向同字。

2026-09-25 架構檢查：拆成 store（讀寫與 transaction）／repos／groups／relations／audit，
這裡把公開名稱接出去，呼叫端照舊 `registry.xxx`。
"""
from .store import load, save, transaction
from .repos import TIERS, add_repo, set_field, tag_repo, all_tags, remove_repo, get_repo, survival
from .groups import (
    add_group, get_group, set_group_field, update_group, add_group_members,
    remove_group_members, set_group_members, rename_group, remove_group, resolve_group,
)
from .relations import (
    RELATION_KINDS, relation_kinds, relate, set_export, remove_export, exports_of, unrelate,
    relations, relations_of, related_ids, relation_lines,
)
from .audit import last_commit_days, scan_dir, scan_register, audit
