//! 寫進 spine 前遮蔽秘密（2026-10-16，ideation N-39；Python 沒有）。
//!
//! spine 是 git repo、可能推到遠端；agent 的 `log_decision`／`handoff`／`add_todo` 文字原樣落地，貼錯一段
//! 終端輸出就把 token 留進歷史。規則抄 Better Agent Terminal 的 `shared/transfer-redaction.mjs`，
//! 再收緊「`password=` 類」的值：至少 6 個 ASCII 字元才遮（中文說明、`${VAR}`、`<token>` 不動）。

use std::sync::LazyLock;

use regex::{Captures, Regex};

/// 一條規則：比對到的整段換成 `replace` 的結果。
struct Rule {
    re: Regex,
    replace: fn(&Captures<'_>) -> String,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let rule = |pattern: &str, replace: fn(&Captures<'_>) -> String| Rule {
        // 寫死的規則，編不過是程式錯
        re: Regex::new(pattern).expect("redact pattern"),
        replace,
    };
    vec![
        rule(
            r"(?s)-----BEGIN(?: [A-Z0-9]+)? PRIVATE KEY-----.*?-----END(?: [A-Z0-9]+)? PRIVATE KEY-----",
            |_| "[REDACTED PRIVATE KEY]".into(),
        ),
        rule(
            r"\b(?:sk-ant-|sk-|github_pat_|gh[pousr]_|xox[abprs]-)[A-Za-z0-9_-]{12,}|\bAKIA[0-9A-Z]{16}\b",
            |_| "[REDACTED TOKEN]".into(),
        ),
        rule(r"(?i)(authorization\s*:\s*bearer\s+)[^\s\x22']+", |c| {
            format!("{}[REDACTED]", &c[1])
        }),
        rule(
            r#"(?i)(\b(?:[a-z0-9]+[_-])*(?:api[_-]?key|access[_-]?token|auth[_-]?token|secret|password|passwd|cookie)["']?\s*[:=]\s*)(?:"[^"\r\n]{6,}"|'[^'\r\n]{6,}'|[A-Za-z0-9_\-./+=~!@#%^&*]{6,})"#,
            |c| format!("{}[REDACTED]", &c[1]),
        ),
    ]
});

/// 遮蔽 `text` 裡的秘密，回傳（遮完的文字, 遮了幾處）。沒東西可遮時原樣回傳。
pub fn redact_secrets(text: &str) -> (String, usize) {
    let mut out = text.to_string();
    let mut count = 0;
    for r in RULES.iter() {
        let n = r.re.find_iter(&out).count();
        if n == 0 {
            continue;
        }
        count += n;
        out =
            r.re.replace_all(&out, |c: &Captures<'_>| (r.replace)(c))
                .into_owned();
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_tokens_keys_and_assignments() {
        let key =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\ndef\n-----END OPENSSH PRIVATE KEY-----";
        let text = format!(
            "金鑰：{key}\n用 sk-ant-api03-AbCdEf123456789 和 ghp_abcdefghijklmnop1234 呼叫；\
             AWS AKIAIOSFODNN7EXAMPLE\nAuthorization: Bearer eyJhbGciOi.x.y\n\
             OPENAI_API_KEY=sk_live_abcdef password: \"hunter2hunter2\" db_secret='s3cr3t-value'"
        );
        let (out, n) = redact_secrets(&text);
        assert_eq!(n, 8, "{out}");
        for leaked in [
            "abc\ndef",
            "sk-ant-api03",
            "ghp_abcdef",
            "AKIAIOSFODNN7",
            "eyJhbGciOi",
            "sk_live_abcdef",
            "hunter2",
            "s3cr3t",
        ] {
            assert!(!out.contains(leaked), "{leaked} 沒遮掉：{out}");
        }
        assert!(out.contains("金鑰：[REDACTED PRIVATE KEY]"));
        assert!(out.contains("Authorization: Bearer [REDACTED]"));
        assert!(out.contains("OPENAI_API_KEY=[REDACTED]"));
        assert!(out.contains("password: [REDACTED]"));
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for text in [
            "決定：password 欄位改成雜湊，secret: 不寫進 repo",
            "api_key=${API_KEY}、token=<你的 token>、password=***",
            "task-abcdefghijklmnop 跟 risk-assessment-document 都不是 token",
            "## 標題\n- 完成：#12 關掉",
        ] {
            assert_eq!(redact_secrets(text), (text.to_string(), 0), "{text}");
        }
    }
}

#[cfg(test)]
mod spine_tests {
    #[test]
    fn spine_writes_never_keep_secrets() {
        let t = tempfile::tempdir().unwrap();
        let ev = crate::spine::append_event(
            t.path(),
            "decision",
            "agent",
            &["repo:x"],
            "改用新金鑰 ghp_abcdefghijklmnop1234 部署",
            None,
        )
        .unwrap();
        assert_eq!(ev.body, "改用新金鑰 [REDACTED TOKEN] 部署");
        let all = crate::spine::iter_events(t.path()).unwrap();
        assert_eq!(all[0].body, "改用新金鑰 [REDACTED TOKEN] 部署");
    }
}
