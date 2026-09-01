"""P8 落 route：結果路由到 spine groups/<g>/materials、某 repo 01_inbox/、或 incubator。"""
import datetime as _dt
from pathlib import Path

from . import registry, spine


def route(spine_dir, text, dest, title="idea", when=None):
    """dest：'group:<g>' | 'repo:<id>' | 'incubator'。回傳寫入的檔案路徑。"""
    now = when or _dt.datetime.now()
    fname = f"{now:%Y%m%d-%H%M%S}_{title}.md"
    if dest.startswith("group:"):
        target = Path(spine_dir) / "groups" / dest[6:] / "materials" / fname
    elif dest.startswith("repo:"):
        entry = registry.get_repo(spine_dir, dest[5:])
        target = Path(entry["path"]).expanduser() / "01_inbox" / fname
    elif dest == "incubator":
        target = Path(spine_dir) / "incubator" / fname
    else:
        raise ValueError(f"dest 不合法: {dest}（group:<g> | repo:<id> | incubator）")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(text, encoding="utf-8")
    tokens = ["repo:" + dest[5:]] if dest.startswith("repo:") else []
    spine.append_event(spine_dir, "decision", "route", tokens,
                       body=f"落點：{dest} → {target.name}", when=now)
    return target
