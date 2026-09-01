"""L2 E2E：subprocess 跑真 CLI，全流程＋不變式 1/3（append-only、git 單一提交者）。"""
import datetime as dt
import subprocess

import pytest

from tests.conftest import make_git_repo, run_cli

pytestmark = pytest.mark.e2e


def _commit_count(repo_dir):
    r = subprocess.run(["git", "-C", str(repo_dir), "rev-list", "--count", "HEAD"],
                       capture_output=True, text=True)
    return int(r.stdout.strip()) if r.returncode == 0 else 0


def test_full_pipeline(tmp_path):
    spine_dir = tmp_path / "s"
    ra = make_git_repo(tmp_path, "repo-a", days_old=1)
    rb = make_git_repo(tmp_path, "repo-b", days_old=2, dirty_files=1)
    import os, time
    old = time.time() - 6 * 86400
    for f in rb.glob("wip*.md"):
        os.utime(f, (old, old))

    # init → register → group
    run_cli("init", str(spine_dir), check=True)
    run_cli("registry", "add", "repo-a", str(ra), spine_dir=spine_dir, check=True)
    run_cli("registry", "add", "repo-b", str(rb), spine_dir=spine_dir, check=True)
    run_cli("group", "add", "g1", "repo-a,repo-b", spine_dir=spine_dir, check=True)
    r = run_cli("registry", "list", spine_dir=spine_dir, check=True)
    assert "repo-a" in r.stdout and "repo-b" in r.stdout

    # collect / pack / brief
    r = run_cli("collect", "--group", "g1", spine_dir=spine_dir, check=True)
    assert "repo-b" in r.stdout
    r = run_cli("pack", "--group", "g1", spine_dir=spine_dir, check=True)
    assert "打包完成" in r.stdout
    r = run_cli("brief", "--group", "g1", spine_dir=spine_dir, check=True)
    assert "早晨簡報" in r.stdout and "沉默摘要" in r.stdout

    # 事件寫入：合法／壞事件 dead-letter
    run_cli("append", "--type", "open-loop", "--source", "manual",
            "--kv", "#1", "--body", "opened →「試一下」",
            spine_dir=spine_dir, check=True)
    r = run_cli("append", "--type", "bogus", "--source", "hotkey",
                "--body", "想法別丟", "--dead-letter", spine_dir=spine_dir)
    assert r.returncode == 1 and "dead-letter" in r.stdout

    # 碰撞（mock、同步）→ outcome 回連 → stats
    r = run_cli("collide", "submit", "全新的想法試撞", "--group", "g1",
                "--provider", "mock", "--wait", spine_dir=spine_dir, check=True)
    assert "已丟進碰撞" in r.stdout and "判定" in r.stdout
    cid = f"{dt.date.today():%Y-%m-%d}-a"
    run_cli("append", "--type", "outcome", "--source", "manual",
            "--kv", f"ref:collision:{cid}", "--body", "長成了一份筆記",
            spine_dir=spine_dir, check=True)
    r = run_cli("query", "--stats", spine_dir=spine_dir, check=True)
    assert "靈感命中率 100%" in r.stdout

    # route → repo inbox
    run_cli("route", "--text", "撞出來的結論", "--dest", "repo:repo-a",
            spine_dir=spine_dir, check=True)
    assert list((ra / "01_inbox").glob("*.md"))

    # 不變式 3：以上所有動作皆不產生 commit；只有 spine commit 會
    assert _commit_count(spine_dir) == 0
    run_cli("commit", "-m", "ritual: e2e", spine_dir=spine_dir, check=True)
    assert _commit_count(spine_dir) == 1

    # 不變式 1：append-only——再寫一筆後，舊內容仍是新內容的前綴
    day_file = spine_dir / "spine" / "events" / f"{dt.date.today():%Y-%m-%d}.md"
    before = day_file.read_text(encoding="utf-8")
    run_cli("append", "--type", "decision", "--source", "manual",
            "--body", "尾筆", spine_dir=spine_dir, check=True)
    after = day_file.read_text(encoding="utf-8")
    assert after.startswith(before) and len(after) > len(before)

    # 往返驗證：整條脊椎 parse 得回來（query 不炸、筆數合理）
    r = run_cli("query", "--today", spine_dir=spine_dir, check=True)
    assert len(r.stdout.strip().splitlines()) >= 6

    # unread → ack
    r = run_cli("unread", spine_dir=spine_dir, check=True)
    assert "未讀 0 則" not in r.stdout
    run_cli("unread", "--ack", spine_dir=spine_dir, check=True)
    r = run_cli("unread", spine_dir=spine_dir, check=True)
    assert "未讀 0 則" in r.stdout


def test_audit_exit_code(tmp_path):
    spine_dir = tmp_path / "s"
    run_cli("init", str(spine_dir), check=True)
    run_cli("registry", "add", "ghost", str(tmp_path / "nope"),
            spine_dir=spine_dir, check=True)
    r = run_cli("registry", "audit", spine_dir=spine_dir)
    assert r.returncode == 1 and "路徑失效" in r.stdout


def test_cli_refuses_non_spine_dir(tmp_path):
    r = run_cli("query", spine_dir=tmp_path / "empty")
    assert r.returncode != 0 and "不是 spine repo" in (r.stderr + r.stdout)


def test_registry_set_field(tmp_path):
    """回歸：registry set <id> <key> <value> 之前因 argparse 位置參數共用而寫錯欄位。"""
    spine_dir = tmp_path / "s"
    ra = make_git_repo(tmp_path, "repo-a", days_old=1)
    run_cli("init", str(spine_dir), check=True)
    run_cli("registry", "add", "repo-a", str(ra), "--tier", "paused",
            spine_dir=spine_dir, check=True)
    r = run_cli("registry", "set", "repo-a", "resume_when", "等 Q4",
                spine_dir=spine_dir, check=True)
    assert "resume_when" in r.stdout and "等 Q4" in r.stdout
    reds = run_cli("registry", "audit", spine_dir=spine_dir)
    assert reds.returncode == 0  # resume_when 補上後 audit 不再紅字
    import yaml
    data = yaml.safe_load((spine_dir / "registry.yaml").read_text(encoding="utf-8"))
    entry = next(x for x in data["repos"] if x["id"] == "repo-a")
    assert entry.get("resume_when") == "等 Q4"
    assert "等 Q4" not in entry  # 沒有變成錯的 key 名


def test_registry_remove(tmp_path):
    spine_dir = tmp_path / "s"
    ra = make_git_repo(tmp_path, "repo-a", days_old=1)
    run_cli("init", str(spine_dir), check=True)
    run_cli("registry", "add", "repo-a", str(ra), spine_dir=spine_dir, check=True)
    run_cli("group", "add", "g1", "repo-a", spine_dir=spine_dir, check=True)
    r = run_cli("registry", "remove", "repo-a", spine_dir=spine_dir, check=True)
    assert "已移除" in r.stdout
    r = run_cli("registry", "list", spine_dir=spine_dir, check=True)
    assert "repo-a" not in r.stdout
    r = run_cli("registry", "remove", "repo-a", spine_dir=spine_dir)
    assert r.returncode != 0  # 不存在要報錯
