"""L1：脊椎標頭嚴格文法（P10 三件套之一）。"""
import pytest

from repoengine.spine import ValidationError, parse_header, validate_block, EVENT_TYPES


GOOD = [
    "## 07:42 presented [morning-brief] group:agent-tooling",
    "## 07:45 chosen [morning-brief] ref:presented:07:42",
    "## 21:10 collision [hotkey] group:臨時(repo-a,card_notes)",
    "## 21:12 open-loop [hotkey] #13 due:2026-09-15",
    "## 09:00 outcome [manual] ref:collision:2026-09-15-a",
    "## 23:59 decision [engine]",
    "## 00:00 suggestion [mcp] repo:repo-a id:2026-09-01-a",
]

BAD = [
    "## 7:42 presented [morning-brief]",          # 時間要兩位數
    "## 24:00 presented [morning-brief]",         # 不存在的小時
    "## 07:42 unknown-type [x]",                  # 未知型別
    "## 07:42 presented morning-brief",           # source 少中括號
    "## 07:42 presented [morning-brief] foo=bar", # kv 文法錯
    "## 07:42 presented [morning-brief] due:9/15",# due 格式錯
    "#07:42 presented [x]",                       # 不是 '## ' 開頭
    "## 07:42 presented [Morning_Brief]",         # source 限小寫與連字號
]


@pytest.mark.parametrize("line", GOOD)
def test_good_headers(line):
    ev = parse_header(line)
    assert ev.type in EVENT_TYPES


@pytest.mark.parametrize("line", BAD)
def test_bad_headers_rejected(line):
    with pytest.raises(ValidationError):
        parse_header(line)


def test_body_must_not_contain_header_lines():
    with pytest.raises(ValidationError):
        validate_block("## 07:42 presented [manual]", "內文\n## 08:00 假標頭混進來")


def test_kv_accessor():
    ev = parse_header("## 21:12 open-loop [hotkey] #13 due:2026-09-15 group:g1")
    assert ev.kv("due") == "2026-09-15"
    assert ev.kv("group") == "g1"
    assert "#13" in ev.tokens


def test_header_roundtrip():
    for line in GOOD:
        assert parse_header(line).header() == line
