"""跨 repo 參考輪（2026-09-25）：agent 在 A repo 工作，又快又準地拿到 B repo 的東西。

釘住：① exports＋resolve（用名字不用路徑）；② read 附 commit、記引用、同日去重、洩密拒讀；
③ drift（引用過的上游改了）與 suggest_exports；④ agent 文件失效路徑 lint（只報有把握的）；
⑤ 搜尋：標題加權、鄰居加分、why、程式碼 git grep；⑥ ask_repo（唯讀、引用落帳）；
⑦ repo 狀態卡（水位線、目錄彙總、dirty、正在跑的程序）；⑧ 開場注入鄰居地圖；⑨ MCP／CLI 入口。
"""
import datetime as dt
import json
import time

import pytest

from repo_context import agentapi as api
from repo_context import agents, crossref, hooks, mcpserver, registry, repocard
from tests.conftest import _run_git, make_git_repo, run_cli


def _commit(repo, msg="edit"):
    _run_git(repo, "add", "-A")
    _run_git(repo, "commit", "-q", "-m", msg)


@pytest.fixture
def world(spine, tmp_path):
    """notes（知識庫，有書摘目錄）feeds course（課程 repo）；other 是不相關的 repo。"""
    notes = make_git_repo(tmp_path, "notes")
    (notes / "books" / "cache-book").mkdir(parents=True)
    (notes / "books" / "cache-book" / "ch01.md").write_text(
        "# 第一章 快取失效\n快取失效是最難的問題之一。\n", encoding="utf-8")
    (notes / "books" / "cache-book" / "ch02.md").write_text(
        "# 第二章 一致性\n寫入策略與一致性。\n", encoding="utf-8")
    (notes / "keys.md").write_text("api_key = 'ABCDEFGHIJKLMNOPQRSTUV'\n", encoding="utf-8")
    _commit(notes, "books")
    course = make_git_repo(tmp_path, "course")
    other = make_git_repo(tmp_path, "other")
    for rid, p in (("notes", notes), ("course", course), ("other", other)):
        registry.add_repo(spine, rid, p)
    registry.add_group(spine, "課程", ["course"])
    registry.add_group(spine, "知識", ["notes"])
    registry.set_export(spine, "notes", "書摘", "books", desc="逐章書摘")
    registry.relate(spine, "notes", "course", "feeds", exports="書摘")
    return spine, {"notes": notes, "course": course, "other": other}


# ---- ① exports ＋ resolve ----

def test_export_validation(world):
    sp, repos = world
    with pytest.raises(ValueError, match="不存在"):
        registry.set_export(sp, "notes", "x", "nope")
    with pytest.raises(ValueError, match="跳出"):
        registry.set_export(sp, "notes", "x", "../course")
    with pytest.raises(ValueError, match="不能"):
        registry.set_export(sp, "notes", "a:b", "books")
    with pytest.raises(ValueError, match="沒有 export"):
        registry.relate(sp, "notes", "other", "feeds", exports="不存在的")
    assert registry.relations(sp, "notes")[0]["exports"] == ["書摘"]
    assert registry.remove_export(sp, "notes", "書摘")
    assert "exports" not in registry.relations(sp, "notes")[0]   # 關係上的引用一起清掉


def test_resolve_by_name(world):
    sp, repos = world
    r = crossref.resolve(sp, "notes:書摘/cache-book/ch01.md")
    assert r["export"] == "書摘" and r["kind"] == "file"
    assert r["rel"] == "books/cache-book/ch01.md" and r["desc"] == "逐章書摘"
    assert crossref.resolve(sp, "notes/README.md")["kind"] == "file"   # repo/相對路徑
    assert crossref.resolve(sp, "notes")["rel"] == "."
    miss = crossref.resolve(sp, "notes:書摘/nope.md")
    assert not miss["exists"] and "書摘" in miss["hint"]
    with pytest.raises(crossref.RefError) as e:
        crossref.resolve(sp, "notes:../../etc")
    assert e.value.code == "bad_request"
    with pytest.raises(crossref.RefError) as e:
        crossref.resolve(sp, "note")
    assert e.value.code == "not_found" and "notes" in e.value.hint


# ---- ② read ＋ 引用紀錄 ----

def test_read_records_cite_once_per_day(world):
    sp, repos = world
    r = api.read_from(sp, "notes:書摘/cache-book/ch01.md", cwd=repos["course"])
    assert "快取失效" in r["content"] and r["commit"] and r["file_commit"]
    api.read_from(sp, "notes:書摘/cache-book/ch01.md", cwd=repos["course"])
    cites = crossref.load_cites(sp)
    assert len(cites) == 1                                   # 同日同檔只記一次
    assert (cites[0]["consumer"], cites[0]["provider"]) == ("course", "notes")
    api.read_from(sp, "notes:README.md", cwd=repos["notes"])  # 自己讀自己不記
    api.read_from(sp, "notes:README.md", cwd=sp)              # spine 裡讀（沒有引用方）不記
    assert len(crossref.load_cites(sp)) == 1


def test_read_dir_lines_and_secret(world):
    sp, repos = world
    d = api.read_from(sp, "notes:書摘", cwd=repos["course"])
    assert d["kind"] == "dir" and d["entries"][0] == {"name": "cache-book/", "files": 2}
    r = api.read_from(sp, "notes:書摘/cache-book/ch01.md", lines="2", cwd=repos["course"])
    assert r["content"] == "快取失效是最難的問題之一。" and r["lines"] == "2-2"
    with pytest.raises(api.AgentError) as e:
        api.read_from(sp, "notes:keys.md", cwd=repos["course"])
    assert e.value.code == "secret"
    with pytest.raises(api.AgentError) as e:
        api.read_from(sp, "notes:nope.md", cwd=repos["course"])
    assert e.value.code == "not_found"


# ---- ③ drift ＋ suggest ----

def test_drift_after_upstream_change(world):
    sp, repos = world
    api.read_from(sp, "notes:書摘/cache-book/ch01.md", cwd=repos["course"])
    assert crossref.drift(sp, consumer="course") == []
    (repos["notes"] / "books" / "cache-book" / "ch02.md").write_text("改了別章\n", encoding="utf-8")
    _commit(repos["notes"], "改第二章")
    assert crossref.drift(sp, consumer="course") == []        # 改的不是引用的那份
    (repos["notes"] / "books" / "cache-book" / "ch01.md").write_text("# 第一章\n改寫\n", encoding="utf-8")
    _commit(repos["notes"], "改寫第一章")
    d = crossref.drift(sp, consumer="course")
    assert len(d) == 1 and d[0]["changes"] == 1 and d[0]["latest"]["subject"] == "改寫第一章"
    assert "改了 1 次" in crossref.drift_lines(d)[0]


def test_suggest_exports(world):
    sp, repos = world
    (repos["other"] / "docs").mkdir()
    for i in range(3):
        (repos["other"] / "docs" / f"d{i}.md").write_text(f"# d{i}\n", encoding="utf-8")
    _commit(repos["other"], "docs")
    for i in range(3):
        api.read_from(sp, f"other:docs/d{i}.md", cwd=repos["course"])
    api.read_from(sp, "notes:書摘/cache-book/ch01.md", cwd=repos["course"])  # 已被 export 涵蓋
    sug = crossref.suggest_exports(sp)
    assert [(s["repo"], s["path"], s["cites"]) for s in sug] == [("other", "docs", 3)]
    registry.set_export(sp, "other", "文件", "docs")
    assert crossref.suggest_exports(sp) == []


# ---- ④ agent 文件路徑 lint ----

def test_lint_doc_refs_reports_only_confident_breaks(world, tmp_path):
    sp, repos = world
    (repos["course"] / "AGENTS.md").write_text("\n".join([
        "# 規則",
        "書摘讀 `../notes/books/cache-book/`。",            # 相對路徑存在 → OK
        "上游在 `notes/books/`。",                         # repo 名開頭、子路徑存在 → OK
        "舊位置 `projects/notes/books/`。",                # 失效 → 報
        "筆記 `notes/nope/`。",                            # repo 名開頭但子路徑不存在 → 報
        "示意 `notes/.../x.md`。",                         # 省略號 → 不驗
        "~~廢棄 `projects/notes/`~~",                      # 刪除線 → 不驗
        "自己的 `src/course/x.py`。",                      # 沒提到別的 repo → 不驗
        "```", "cat projects/notes/a.md", "```",           # 程式碼區塊 → 不驗
    ]), encoding="utf-8")
    found = crossref.lint_doc_refs(sp, ["course"])
    assert [(b["line"], b["ref"], b["target"]) for b in found] == [
        (4, "projects/notes/books/", "notes"), (5, "notes/nope/", "notes")]
    assert "`notes:books`" in found[0]["fix"]


def test_audit_cli_includes_doc_refs(world):
    sp, repos = world
    (repos["course"] / "AGENTS.md").write_text("看 `projects/notes/`\n", encoding="utf-8")
    r = run_cli("registry", "audit", spine_dir=sp)
    assert r.returncode == 1 and "[文件路徑失效]" in r.stdout


# ---- ⑤ 搜尋 ----

def test_search_neighbor_boost_and_why(world):
    sp, repos = world
    text = "# 雜記\n快取失效是最難的問題之一。\n"
    (repos["other"] / "misc.md").write_text(text, encoding="utf-8")
    _commit(repos["other"], "misc")
    plain = api.search_knowledge(sp, "快取失效", cwd=sp, code=False)
    from_course = api.search_knowledge(sp, "快取失效", cwd=repos["course"], code=False)
    top = from_course["files"][0]
    assert top["repo"] == "notes" and top["ref"].startswith("notes:books/")
    assert any("有關係" in w for w in top["why"]) and any("書摘" in w for w in top["why"])
    assert "標題命中" in top["why"]
    notes_score = lambda r: max(f["score"] for f in r["files"] if f["repo"] == "notes")
    assert notes_score(from_course) > notes_score(plain)   # 同一份檔，從鄰居查分數較高


def test_code_search(world):
    sp, repos = world
    (repos["other"] / "cache.py").write_text(
        "def evict_lru(cache):\n    return cache.popitem()\n", encoding="utf-8")
    _commit(repos["other"], "code")
    r = api.search_knowledge(sp, "evict_lru 怎麼做", cwd=repos["course"])
    assert r["code"][0]["file"] == "cache.py" and r["code"][0]["ref"] == "other:cache.py"
    assert r["code"][0]["snippets"][0]["line"] == 1
    assert all(not c["file"].endswith(".md") for c in r["code"])


# ---- ⑥ ask_repo ----

def test_ask_repo_read_only_and_cites(world, monkeypatch):
    sp, repos = world
    cmd = agents.ClaudeProvider().build_cmd("q", read_only=True)
    assert "--allowedTools" in cmd and "Edit" in cmd[cmd.index("--disallowedTools"):]
    seen = {}

    def fake(provider, prompt, spine_dir, model=None, cwd=None, read_only=False):
        seen.update(provider=provider, cwd=cwd, read_only=read_only, prompt=prompt)
        return 'x {"answer": "LRU", "citations": [{"file": "books/cache-book/ch01.md", "why": "定義"}]}'
    monkeypatch.setattr(agents, "run_text", fake)
    r = api.ask_repo(sp, "notes", "快取怎麼失效？", cwd=repos["course"])
    assert r["answer"] == "LRU" and r["provider"] == "claude"      # 預設 claude
    assert seen["cwd"] == repos["notes"].resolve() and seen["read_only"]
    assert "快取怎麼失效" in seen["prompt"] and "course" in seen["prompt"]
    assert crossref.load_cites(sp)[0]["via"] == "ask"
    monkeypatch.setattr(agents, "run_text", lambda *a, **k: "純文字回答")
    r = api.ask_repo(sp, "notes", "再問一次", cwd=repos["course"])
    assert r["answer"] == "純文字回答" and r["citations"] == [] and r["note"]


def test_ask_repo_mock_provider(world):
    sp, repos = world
    r = api.ask_repo(sp, "notes", "快取？", cwd=repos["course"], provider="mock")
    assert r["ok"] and r["provider"] == "mock" and r["answer"]


# ---- ⑦ repo 狀態卡 ----

def test_repo_card_since_seen_and_areas(world):
    sp, repos = world
    card = repocard.build(sp, "notes", mark=True)
    assert card["window"]["kind"] == "recent" and card["marked_seen"]
    (repos["notes"] / "books" / "cache-book" / "ch03.md").write_text("# 三\n", encoding="utf-8")
    _commit(repos["notes"], "第三章")
    card = repocard.build(sp, "notes", mark=False)
    assert card["window"]["kind"] == "seen"
    assert [c["subject"] for c in card["new_commits"]] == ["第三章"]
    assert card["areas"][0]["area"] == "books/" and card["areas"][0]["files"] == 1
    again = repocard.build(sp, "notes")                      # 沒 mark＝水位線沒動
    assert [c["subject"] for c in again["new_commits"]] == ["第三章"]
    assert "books/ 1 檔" in repocard.render(card)


def test_repo_card_dirty_running_and_agent(world):
    sp, repos = world
    (repos["course"] / "wip.md").write_text("草稿\n", encoding="utf-8")
    old = time.time() - 3 * 86400
    import os
    os.utime(repos["course"] / "wip.md", (old, old))
    started = dt.datetime.now() - dt.timedelta(hours=1)
    ps = [(99999, started, f"node {repos['course'].resolve()}/node_modules/.bin/vite"),
          (99998, started, f"{repos['course'].resolve()}/node_modules/esbuild/bin/esbuild --ping"),
          (99997, started, "node /somewhere/else/server.js")]
    (repos["course"] / "README.md").write_text("# 改過\n", encoding="utf-8")  # 啟動後改
    api.handoff(sp, "做完大綱", "下次寫第二週", cwd=repos["course"])
    card = repocard.build(sp, "course", ps_rows=ps, cwd_pids=set())
    assert card["dirty"][0]["file"] == "wip.md" and card["dirty"][0]["days"] >= 2.9
    assert [p["pid"] for p in card["running"]] == [99999]     # 子程序與別的 repo 的程序不列
    assert card["running"][0]["changed_since_start"] >= 1
    assert "README.md" in card["running"][0]["changed_files"]
    assert card["last_handoff"]["text"].startswith("做完大綱")
    text = repocard.render(card)
    assert "正在跑" in text and "要重開" in text and "放最久 3 天" in text


def test_repo_status_agent_does_not_move_watermark(world):
    sp, repos = world
    r = api.repo_status(sp, cwd=repos["course"])
    assert r["repo"] == "course" and "marked_seen" not in r
    assert repocard.get_seen(sp, "course") is None


# ---- ⑧ 開場注入鄰居地圖 ----

def test_session_start_injects_neighbor_map_and_drift(world):
    sp, repos = world
    api.read_from(sp, "notes:書摘/cache-book/ch01.md", cwd=repos["course"])
    (repos["notes"] / "books" / "cache-book" / "ch01.md").write_text("改\n", encoding="utf-8")
    _commit(repos["notes"], "改第一章")
    out = hooks.session_start(sp, {"session_id": "s1", "cwd": str(repos["course"])})
    ctx = json.loads(out)["hookSpecificOutput"]["additionalContext"]
    assert "鄰居 repo" in ctx and "`notes:書摘`→books/" in ctx and "逐章書摘" in ctx
    assert "改了 1 次" in ctx and "read_from" in ctx
    ctx_other = json.loads(hooks.session_start(sp, {"session_id": "s2", "cwd": str(repos["other"])}))
    assert "鄰居 repo" not in ctx_other["hookSpecificOutput"]["additionalContext"]


# ---- ⑨ MCP／CLI ----

def test_mcp_work_tools_include_crossref(world):
    sp, repos = world
    names = [t["name"] for t in mcpserver.handle_message(
        sp, {"id": 1, "method": "tools/list"}, admin=False)["result"]["tools"]]
    for n in ("read_from", "ask_repo", "repo_status", "search_knowledge"):
        assert n in names
    assert "registry_export" not in names
    res = mcpserver.handle_message(sp, {"id": 2, "method": "tools/call", "params": {
        "name": "read_from", "arguments": {"ref": "notes:書摘/cache-book/ch01.md"}}},
        cwd=str(repos["course"]))["result"]
    assert not res["isError"] and "快取失效" in json.loads(res["content"][0]["text"])["content"]
    res = mcpserver.handle_message(sp, {"id": 3, "method": "tools/call", "params": {
        "name": "registry_export", "arguments": {"id": "other", "name": "全部", "path": "."}}},
        admin=True)["result"]
    assert not res["isError"] and "other:全部" in res["content"][0]["text"]


@pytest.mark.e2e
def test_cli_crossref_flow(world):
    sp, repos = world
    r = run_cli("resolve", "notes:書摘", spine_dir=sp, check=True)
    assert r.stdout.strip().splitlines()[0].endswith("books")
    r = run_cli("read", "notes:書摘/cache-book/ch01.md", "--cwd", str(repos["course"]),
                "--json", spine_dir=sp, check=True)
    assert json.loads(r.stdout)["repo"] == "notes"
    r = run_cli("refs", "cite", "notes:書摘/cache-book/ch02.md", "--note", "複製進講義",
                "--cwd", str(repos["course"]), spine_dir=sp, check=True)
    assert "已記引用" in r.stdout
    r = run_cli("repo", "notes", "--peek", "--json", spine_dir=sp, check=True)
    assert json.loads(r.stdout)["repo"] == "notes"
    r = run_cli("registry", "exports", "--json", spine_dir=sp, check=True)
    assert json.loads(r.stdout)[0]["name"] == "書摘"
    (repos["course"] / "AGENTS.md").write_text("看 `projects/notes/`\n", encoding="utf-8")
    r = run_cli("refs", "check", spine_dir=sp)
    assert r.returncode == 1 and "projects/notes/" in r.stdout
    r = run_cli("ask", "notes", "快取？", "--provider", "mock", "--cwd", str(repos["course"]),
                spine_dir=sp, check=True)
    assert "notes 的回答" in r.stdout
