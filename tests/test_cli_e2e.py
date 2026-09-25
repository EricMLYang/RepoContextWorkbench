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


def test_scan_lint_estimate_cli(tmp_path):
    """拆機報告落地輪：registry scan（mani 課）／lint（衛生迴圈）／pack --estimate。"""
    spine_dir = tmp_path / "s"
    make_git_repo(tmp_path / "zone", "repo-new")
    run_cli("init", str(spine_dir), check=True)
    r = run_cli("registry", "scan", str(tmp_path / "zone"),
                spine_dir=spine_dir, check=True)
    assert "候選" in r.stdout and "repo-new" in r.stdout
    run_cli("registry", "scan", str(tmp_path / "zone"), "--apply",
            spine_dir=spine_dir, check=True)
    r = run_cli("registry", "list", spine_dir=spine_dir, check=True)
    assert "repo-new" in r.stdout
    r = run_cli("pack", "--estimate", spine_dir=spine_dir, check=True)
    assert "repo-new" in r.stdout and "合計" in r.stdout
    r = run_cli("lint", spine_dir=spine_dir, check=True)
    assert "零紅字" in r.stdout
    # 弄髒脊椎（斷 ref）→ lint exit 1（跟 audit 同形態）
    run_cli("append", "--type", "chosen", "--source", "manual",
            "--kv", "ref:presented:00:01", "--body", "斷鏈",
            spine_dir=spine_dir, check=True)
    r = run_cli("lint", spine_dir=spine_dir)
    assert r.returncode == 1 and "ref 斷鏈" in r.stdout


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


def test_new_primitives_pipeline(tmp_path):
    """P4/P5/P9/P12/P13/P16 走真 CLI：upstream 無事不報、digest 增量、spawn 出生登記、
    timer --once 補課、notify 分類、session --print 指令組裝。"""
    spine_dir = tmp_path / "s"
    ra = make_git_repo(tmp_path, "repo-a", days_old=1)
    run_cli("init", str(spine_dir), check=True)
    run_cli("registry", "add", "repo-a", str(ra), spine_dir=spine_dir, check=True)
    run_cli("group", "add", "g1", "repo-a", spine_dir=spine_dir, check=True)

    # P4：組內無 external → 講清楚，不是沉默
    r = run_cli("upstream", "--group", "g1", spine_dir=spine_dir, check=True)
    assert "無 external repo" in r.stdout

    # P5：mock digest 首輪 ok、二輪水位線擋住
    r = run_cli("digest", "--group", "g1", spine_dir=spine_dir, check=True)
    assert "repo-a: ok" in r.stdout
    assert list((spine_dir / "groups" / "g1" / "digests").glob("*.md"))
    r = run_cli("digest", "--group", "g1", spine_dir=spine_dir, check=True)
    assert "無新 commit" in r.stdout

    # P7→P9：碰撞 → 升格 → 命中率/存活率都算得出來
    run_cli("collide", "submit", "值得長成 repo 的想法", "--group", "g1",
            "--no-run", spine_dir=spine_dir, check=True)
    cid = f"{dt.date.today():%Y-%m-%d}-a"
    r = run_cli("spawn", "born-repo", str(tmp_path / "born"),
                "--origin", f"collision:{cid}", spine_dir=spine_dir, check=True)
    assert "已升格" in r.stdout and (tmp_path / "born" / ".git").is_dir()
    r = run_cli("query", "--stats", spine_dir=spine_dir, check=True)
    assert "靈感命中率 100%" in r.stdout and "存活率 100%" in r.stdout

    # P12：schedule 進 config → timer --once 當日補跑一次、同日不重跑
    cfg = (spine_dir / "config.yaml").read_text(encoding="utf-8")
    (spine_dir / "config.yaml").write_text(
        cfg + '\nschedule:\n  - {task: commit, at: "00:00"}\n', encoding="utf-8")
    r = run_cli("timer", "--once", spine_dir=spine_dir, check=True)
    assert "✔ commit@00:00@" in r.stdout
    assert _commit_count(spine_dir) == 1  # commit 任務＝單一提交者的排程形態
    r = run_cli("timer", "--once", spine_dir=spine_dir, check=True)
    assert "無到期任務" in r.stdout

    # P13：目前未讀全是一般事件 → notify 只累積不打斷
    r = run_cli("notify", spine_dir=spine_dir, check=True)
    assert "[打斷]" not in r.stdout and "未讀" in r.stdout

    # P16：session --print——料檔＋claude 指令＋.mcp.json
    r = run_cli("session", "--group", "g1", "--task", "寫文", "--print",
                spine_dir=spine_dir, check=True)
    assert "料：" in r.stdout and "claude" in r.stdout
    assert (spine_dir / ".mcp.json").exists()


def test_mcp_stdio_roundtrip(tmp_path):
    """MCP server 走真 stdio：initialize → tools/list → tools/call（含 validator 拒寫）。"""
    import json as _json
    import sys
    from tests.conftest import ROOT
    spine_dir = tmp_path / "s"
    run_cli("init", str(spine_dir), check=True)
    msgs = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
        {"jsonrpc": "2.0", "id": 3, "method": "tools/call",
         "params": {"name": "spine_append",
                    "arguments": {"type": "decision", "source": "agent",
                                  "body": "MCP 往返"}}},
        {"jsonrpc": "2.0", "id": 4, "method": "tools/call",
         "params": {"name": "spine_append",
                    "arguments": {"type": "bogus", "source": "agent"}}},
    ]
    import os
    env = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8")
    r = subprocess.run(
        [sys.executable, "-m", "repo_context", "--spine", str(spine_dir), "mcp", "--admin"],
        input="\n".join(_json.dumps(m, ensure_ascii=False) for m in msgs) + "\n",
        capture_output=True, text=True, encoding="utf-8", cwd=ROOT, env=env,
        timeout=60)
    lines = [_json.loads(x) for x in r.stdout.strip().splitlines()]
    assert len(lines) == 4  # notification 不回
    by_id = {x["id"]: x for x in lines}
    assert by_id[1]["result"]["serverInfo"]["name"] == "repo_context"
    assert any(t["name"] == "collide_submit" for t in by_id[2]["result"]["tools"])
    assert by_id[3]["result"]["isError"] is False
    assert by_id[4]["result"]["isError"] is True
    assert "未知事件型別" in by_id[4]["result"]["content"][0]["text"]
    # 事件真的落了脊椎
    day = spine_dir / "spine" / "events" / f"{dt.date.today():%Y-%m-%d}.md"
    assert "MCP 往返" in day.read_text(encoding="utf-8")


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


def test_relations_context_pulse_cli(tmp_path):
    """2026-09-08 組為單位輪：關係設定／查詢、組情境卡、活動脈動的 CLI 全路徑。"""
    spine_dir = tmp_path / "s"
    ra = make_git_repo(tmp_path, "repo-a", days_old=1)
    rb = make_git_repo(tmp_path, "repo-b", days_old=3)
    run_cli("init", str(spine_dir), check=True)
    run_cli("registry", "add", "repo-a", str(ra), spine_dir=spine_dir, check=True)
    run_cli("registry", "add", "repo-b", str(rb), spine_dir=spine_dir, check=True)
    run_cli("group", "add", "g1", "repo-a,repo-b", spine_dir=spine_dir, check=True)
    r = run_cli("registry", "relate", "repo-a", "repo-b", "--kind", "pm-of",
                "--note", "a 規劃 b", spine_dir=spine_dir, check=True)
    assert "pm-of" in r.stdout
    r = run_cli("registry", "relations", "repo-b", spine_dir=spine_dir, check=True)
    assert "PM 是" in r.stdout and "repo-a" in r.stdout
    r = run_cli("registry", "relate", "repo-a", "repo-b", "--kind", "nope", spine_dir=spine_dir)
    assert r.returncode != 0 and "kind" in (r.stdout + r.stderr)
    r = run_cli("group", "context", "g1", spine_dir=spine_dir, check=True)
    assert "# 組情境卡 g1" in r.stdout and "規劃（PM）→ repo-b" in r.stdout
    r = run_cli("pulse", "--group", "g1", spine_dir=spine_dir, check=True)
    assert "近 7 天" in r.stdout and "init" in r.stdout    # 最近 commit 主旨列出
    run_cli("registry", "unrelate", "repo-a", "repo-b", spine_dir=spine_dir, check=True)
    r = run_cli("registry", "relations", spine_dir=spine_dir, check=True)
    assert "無關係" in r.stdout


def test_group_summary_and_goal_cli(tmp_path):
    """2026-09-12 §3：工作摘要與組目標在命令列也拿得到（跟工作台同一份資料）。"""
    spine_dir = tmp_path / "s"
    ra = make_git_repo(tmp_path, "repo-a", days_old=1)
    run_cli("init", str(spine_dir), check=True)
    run_cli("registry", "add", "repo-a", str(ra), spine_dir=spine_dir, check=True)
    run_cli("group", "add", "g1", "repo-a", spine_dir=spine_dir, check=True)
    # 還沒登記目標／沒有進度 → 摘要直說沒有依據，不編話
    r = run_cli("group", "summary", "g1", spine_dir=spine_dir, check=True)
    assert "# 本組工作摘要 g1" in r.stdout
    assert "尚未登記這組的目標" in r.stdout and "脊椎沒有本組的進度紀錄" in r.stdout
    r = run_cli("group", "goal", "g1", spine_dir=spine_dir, check=True)
    assert "尚未登記目標" in r.stdout
    # 寫目標＝人的動作，留痕
    run_cli("group", "goal", "g1", "--text", "把兩個 repo 的說法對齊",
            spine_dir=spine_dir, check=True)
    r = run_cli("group", "summary", "g1", spine_dir=spine_dir, check=True)
    assert "把兩個 repo 的說法對齊" in r.stdout and "registry.yaml" in r.stdout
    r = run_cli("query", "--type", "decision", spine_dir=spine_dir, check=True)
    assert "decision [cli] group:g1" in r.stdout      # 寫目標是人的出手，留痕（[cli]＝自己出手，不進收件匣）
    day = next((spine_dir / "spine" / "events").glob("*.md"))
    assert "本組目標：把兩個 repo 的說法對齊（從 CLI）" in day.read_text(encoding="utf-8")
