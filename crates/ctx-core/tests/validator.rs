//! L1：脊椎標頭嚴格文法（P10 三件套之一）。對應 tests/test_validator.py。
//! pytest 的 parametrize 在這裡是一個測試跑整張表（失敗訊息帶出哪一行）。

use ctx_core::spine::{parse_header, validate_block, EVENT_TYPES};
use ctx_core::Error;

const GOOD: [&str; 7] = [
    "## 07:42 presented [morning-brief] group:agent-tooling",
    "## 07:45 chosen [morning-brief] ref:presented:07:42",
    "## 21:10 collision [hotkey] group:臨時(repo-a,card_notes)",
    "## 21:12 open-loop [hotkey] #13 due:2026-09-15",
    "## 09:00 outcome [manual] ref:collision:2026-09-15-a",
    "## 23:59 decision [engine]",
    "## 00:00 suggestion [mcp] repo:repo-a id:2026-09-01-a",
];

const BAD: [&str; 8] = [
    "## 7:42 presented [morning-brief]",           // 時間要兩位數
    "## 24:00 presented [morning-brief]",          // 不存在的小時
    "## 07:42 unknown-type [x]",                   // 未知型別
    "## 07:42 presented morning-brief",            // source 少中括號
    "## 07:42 presented [morning-brief] foo=bar",  // kv 文法錯
    "## 07:42 presented [morning-brief] due:9/15", // due 格式錯
    "#07:42 presented [x]",                        // 不是 '## ' 開頭
    "## 07:42 presented [Morning_Brief]",          // source 限小寫與連字號
];

#[test]
fn test_good_headers() {
    for line in GOOD {
        let ev = parse_header(line, "").unwrap_or_else(|e| panic!("{line}: {e}"));
        assert!(EVENT_TYPES.contains(&ev.etype.as_str()), "{line}");
    }
}

#[test]
fn test_bad_headers_rejected() {
    for line in BAD {
        assert!(
            matches!(parse_header(line, ""), Err(Error::Validation(_))),
            "{line} 應被拒"
        );
    }
}

#[test]
fn test_body_must_not_contain_header_lines() {
    let r = validate_block("## 07:42 presented [manual]", "內文\n## 08:00 假標頭混進來");
    assert!(matches!(r, Err(Error::Validation(_))));
}

#[test]
fn test_kv_accessor() {
    let ev = parse_header(
        "## 21:12 open-loop [hotkey] #13 due:2026-09-15 group:g1",
        "",
    )
    .unwrap();
    assert_eq!(ev.kv("due"), Some("2026-09-15"));
    assert_eq!(ev.kv("group"), Some("g1"));
    assert!(ev.tokens.contains(&"#13".to_string()));
}

#[test]
fn test_header_roundtrip() {
    for line in GOOD {
        assert_eq!(parse_header(line, "").unwrap().header(), line);
    }
}

#[test]
fn error_messages_match_python() {
    let msg = |l: &str| parse_header(l, "").unwrap_err().to_string();
    assert_eq!(
        msg("#07:42 presented [x]"),
        "標頭不合文法: '#07:42 presented [x]'（需 '## HH:MM <type> [<source>] key:value ...'）"
    );
    assert_eq!(
        msg("## 24:00 presented [m]"),
        "時間不合法: '## 24:00 presented [m]'"
    );
    assert_eq!(
        msg("## 07:42 presented [m] foo=bar"),
        "kv token 不合法: 'foo=bar'（合法: #編號 或 group:/repo:/ref:/due:/id:）"
    );
    assert_eq!(
        msg("## 07:42 presented [m] due:9/15"),
        "due 需為 YYYY-MM-DD: 'due:9/15'"
    );
}
