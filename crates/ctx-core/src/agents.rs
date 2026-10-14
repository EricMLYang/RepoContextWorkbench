//! P15 agent 執行器：`AgentProvider` 介面＋mock（測試/離線）＋claude（headless）。對應 `repo_context/agents.py`。
//!
//! v1 支援清單只有 Claude Code（v2 §2.5）；介面預留可插拔，但不提前付第二家驗證成本。
//! 內層欄位驗證＝schema-in-prompt＋解析驗證＋失敗重試一次，重試仍壞 → `Error::External`
//! （＝Python `AgentUnsure`：呼叫端接「系統自認沒把握」准打斷類，不准沉默）。raw request/response 一律落檔。
//!
//! 錯誤分兩類（對應 Python 的 `except AgentUnsure: raise` / `except Exception: 重試`）：
//! provider 回 `Error::External` ＝ AgentUnsure，**不重試**直接往上拋；其他錯誤（解析、驗證、Io…）重試一次。

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};

use crate::{Error, Result};

/// 對應 `agents.JUDGE_KEYS`：判定五欄位（順序即輸出順序）。
pub const JUDGE_KEYS: [&str; 5] = ["判定", "理由", "證據", "落點建議", "下一步"];

/// claude 子進程逾時（Python `timeout=600`）。
const CLAUDE_TIMEOUT: Duration = Duration::from_secs(600);

/// 判定值。序列化字串與 Python 相同。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verdict {
    #[serde(rename = "已知")]
    Known,
    #[serde(rename = "衝突")]
    Conflict,
    #[serde(rename = "真增量")]
    Novel,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Known => "已知",
            Verdict::Conflict => "衝突",
            Verdict::Novel => "真增量",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "已知" => Some(Verdict::Known),
            "衝突" => Some(Verdict::Conflict),
            "真增量" => Some(Verdict::Novel),
            _ => None,
        }
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 驗證過的判定（對應 `_validate_judgement` 回傳的五欄 dict；序列化鍵名同 Python）。
/// 非判定欄位若 agent 給的不是字串，存成它的 JSON 文字。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Judgement {
    #[serde(rename = "判定")]
    pub verdict: Verdict,
    #[serde(rename = "理由")]
    pub reason: String,
    #[serde(rename = "證據")]
    pub evidence: String,
    #[serde(rename = "落點建議")]
    pub placement: String,
    #[serde(rename = "下一步")]
    pub next_step: String,
}

/// 可插拔的 agent provider（對應 Python 的 MockProvider／ClaudeProvider 鴨子型別）。
pub trait AgentProvider {
    /// provider 名（"mock"／"claude"）。
    fn name(&self) -> &'static str;
    /// 判定用：回傳 agent 輸出解析成的 JSON（尚未驗欄位）。
    fn run_judgement(&self, prompt: &str, spine_dir: &Path, model: Option<&str>) -> Result<Value>;
    /// 自由文字輸出。`cwd`＝在哪個目錄跑；`read_only`＝只准讀（ask_repo 用）。
    fn run_text(
        &self,
        prompt: &str,
        spine_dir: &Path,
        model: Option<&str>,
        cwd: Option<&Path>,
        read_only: bool,
    ) -> Result<String>;
}

/// 對應 `agents._raw_log`：raw request/response 落 `spine/.agent_logs/<時間>.json`（indent=1）。
fn raw_log(spine_dir: &Path, payload: &Value) -> Result<()> {
    let d = spine_dir.join(".agent_logs");
    std::fs::create_dir_all(&d)?;
    let name = chrono::Local::now().format("%Y%m%d-%H%M%S-%6f").to_string();
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    payload.serialize(&mut ser)?;
    std::fs::write(d.join(format!("{name}.json")), buf)?;
    Ok(())
}

/// 對應 `agents._extract_json`：取第一個 `{` 到最後一個 `}`（Python `re.search(r"\{.*\}", re.S)`）。
pub fn extract_json(text: &str) -> Result<Value> {
    let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) else {
        return Err(Error::invalid("輸出中找不到 JSON 物件"));
    };
    if b < a {
        return Err(Error::invalid("輸出中找不到 JSON 物件"));
    }
    Ok(serde_json::from_str(&text[a..=b])?)
}

/// Python `str(v)` 的近似：字串原樣，其他用 JSON 文字；缺欄＝空字串。
fn field_str(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// 對應 `agents._validate_judgement`：五欄齊、判定值合法。
pub fn validate_judgement(obj: &Value) -> Result<Judgement> {
    let Some(map) = obj.as_object() else {
        return Err(Error::invalid("輸出不是 JSON 物件"));
    };
    let missing: Vec<&str> = JUDGE_KEYS
        .iter()
        .copied()
        .filter(|k| field_str(map.get(*k)).trim().is_empty())
        .collect();
    if !missing.is_empty() {
        return Err(Error::invalid(format!("缺欄位: {}", missing.join(", "))));
    }
    let v = field_str(map.get("判定"));
    let Some(verdict) = Verdict::parse(&v) else {
        return Err(Error::invalid(format!(
            "判定值不合法: '{v}'（需 已知|衝突|真增量）"
        )));
    };
    Ok(Judgement {
        verdict,
        reason: field_str(map.get("理由")),
        evidence: field_str(map.get("證據")),
        placement: field_str(map.get("落點建議")),
        next_step: field_str(map.get("下一步")),
    })
}

/// 對應 `agents.MockProvider`：離線判定器，關鍵字規則，deterministic，供測試與無網環境。
///
/// `fail_n`：前 N 次 `run_judgement` 回傳壞輸出（測內層驗證的重試路徑）。Python 版讀
/// `REPOENGINE_MOCK_FAIL` 且計數器是 class 層級（整個進程共用）；Rust 版計數器掛在實例上，
/// `from_env()` 讀同一個環境變數——測試平行跑也互不干擾。
#[derive(Debug, Default)]
pub struct MockProvider {
    pub fail_n: usize,
    calls: AtomicUsize,
}

impl MockProvider {
    pub fn new(fail_n: usize) -> Self {
        MockProvider {
            fail_n,
            calls: AtomicUsize::new(0),
        }
    }
    /// 讀 `REPOENGINE_MOCK_FAIL`（缺或不是數字＝0）。
    pub fn from_env() -> Self {
        let n = std::env::var("REPOENGINE_MOCK_FAIL")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        Self::new(n)
    }
    /// 到目前為止的 run_judgement 次數。
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AgentProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn run_text(
        &self,
        prompt: &str,
        spine_dir: &Path,
        _model: Option<&str>,
        cwd: Option<&Path>,
        _read_only: bool,
    ) -> Result<String> {
        raw_log(
            spine_dir,
            &json!({"provider": "mock", "mode": "text",
                    "cwd": cwd.map(|c| c.to_string_lossy().into_owned()).unwrap_or_default(),
                    "prompt": crate::pystr::take_chars(prompt, 2000)}),
        )?;
        let lines = crate::pystr::splitlines(prompt).len();
        Ok(format!("mock 摘要：輸入 {lines} 行——重點請看 diff。"))
    }

    fn run_judgement(&self, prompt: &str, spine_dir: &Path, _model: Option<&str>) -> Result<Value> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        raw_log(
            spine_dir,
            &json!({"provider": "mock", "prompt": crate::pystr::take_chars(prompt, 2000), "call": call}),
        )?;
        if call <= self.fail_n {
            return Err(Error::invalid("mock 故意回傳壞輸出（測重試）"));
        }
        // 只看【想法】段做關鍵字判定（prompt 模板本身含三個判定詞，不能全文掃）
        let mut idea = prompt;
        if let Some((_, rest)) = prompt.split_once("\n【想法】\n") {
            idea = rest.split_once("\n【組內容").map_or(rest, |(a, _)| a);
        }
        let verdict = if idea.contains("已知") || idea.contains("known") {
            "已知"
        } else if idea.contains("衝突") || idea.contains("conflict") {
            "衝突"
        } else {
            "真增量"
        };
        Ok(json!({"判定": verdict, "理由": "mock：關鍵字規則判定",
                  "證據": "mock://無實際證據", "落點建議": "incubator",
                  "下一步": "以真 provider 重撞一次"}))
    }
}

/// 對應 `agents.ClaudeProvider`：`claude -p --output-format json`；外層包裝由 CLI 保證，內層欄位另驗。
#[derive(Debug, Default, Clone, Copy)]
pub struct ClaudeProvider;

impl ClaudeProvider {
    /// read_only：給「到別的 repo 開一個 agent 回答問題」（ask_repo）用——只准讀，不准改檔、不准跑指令。
    /// -p 模式下沒列在 allowedTools 的工具一律拒絕，再明列 disallowed 當第二道保險。
    /// 不載使用者層設定（hook、權限、MCP 設定都在那裡）、不掛任何 MCP。
    pub const ISOLATE_ARGS: [&'static str; 3] =
        ["--setting-sources", "project,local", "--strict-mcp-config"];

    pub const READ_ONLY_ARGS: [&'static str; 10] = [
        "--allowedTools",
        "Read",
        "Grep",
        "Glob",
        "LS",
        "--disallowedTools",
        "Edit",
        "Write",
        "Bash",
        "NotebookEdit",
    ];

    /// 對應 `ClaudeProvider.build_cmd`。
    pub fn build_cmd(&self, prompt: &str, model: Option<&str>, read_only: bool) -> Vec<String> {
        let mut cmd: Vec<String> = ["claude", "-p", prompt, "--output-format", "json"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // 刻意差異（2026-10-15）：一次性的判定／問答不載使用者層設定、不掛 MCP——
        // 不然使用者的 SessionStart／SessionEnd hook（`ctx setup --claude` 裝的）會把每次呼叫記成
        // 「agent 會話結束但沒有交接」，灌進收件匣（碰撞判定、ask_repo、App 助手都中過）。
        cmd.extend(Self::ISOLATE_ARGS.iter().map(|s| s.to_string()));
        if let Some(m) = model.filter(|m| !m.is_empty()) {
            cmd.push("--model".into());
            cmd.push(m.into());
        }
        if read_only {
            cmd.extend(Self::READ_ONLY_ARGS.iter().map(|s| s.to_string()));
        }
        cmd
    }

    /// 對應 `ClaudeProvider._call`：跑子進程、落 raw log；rc≠0 → External（AgentUnsure）。
    /// 找不到 claude、逾時 → Io 錯誤（同 Python 的 OSError／TimeoutExpired：由 judge／run_text 重試一次）。
    fn call(
        &self,
        prompt: &str,
        spine_dir: &Path,
        model: Option<&str>,
        cwd: Option<&Path>,
        read_only: bool,
    ) -> Result<String> {
        let cmd = self.build_cmd(prompt, model, read_only);
        let mut c = Command::new(&cmd[0]);
        c.args(&cmd[1..])
            .env("PYTHONUTF8", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(d) = cwd {
            c.current_dir(d);
        }
        let (rc, stdout, stderr) = run_with_timeout(c, CLAUDE_TIMEOUT)?;
        let mut shown: Vec<String> = cmd[..2].to_vec();
        shown.push("<prompt>".into());
        shown.extend(cmd[3..].iter().cloned());
        raw_log(
            spine_dir,
            &json!({"provider": "claude", "cmd": shown,
                    "cwd": cwd.map(|c| c.to_string_lossy().into_owned()).unwrap_or_default(),
                    "prompt": prompt, "rc": rc, "stdout": stdout, "stderr": stderr}),
        )?;
        if rc != 0 {
            return Err(Error::External(format!(
                "claude CLI 失敗 rc={rc}: {}",
                crate::pystr::take_chars(&stderr, 300)
            )));
        }
        let v: Value = serde_json::from_str(&stdout)?;
        Ok(match v.get("result") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
        })
    }
}

/// 跑子進程並限時；stdout/stderr 用執行緒讀，避免管線塞滿卡死。回傳 (rc, stdout, stderr)。
fn run_with_timeout(mut c: Command, timeout: Duration) -> Result<(i32, String, String)> {
    let mut child = c.spawn()?;
    let mut out_pipe = child.stdout.take();
    let mut err_pipe = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(p) = out_pipe.as_mut() {
            let _ = p.read_to_end(&mut b);
        }
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(p) = err_pipe.as_mut() {
            let _ = p.read_to_end(&mut b);
        }
        b
    });
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("claude CLI 逾時（{} 秒）", timeout.as_secs()),
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = t_out.join().unwrap_or_default();
    let err = t_err.join().unwrap_or_default();
    Ok((
        status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out).into_owned(),
        String::from_utf8_lossy(&err).into_owned(),
    ))
}

impl AgentProvider for ClaudeProvider {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn run_judgement(&self, prompt: &str, spine_dir: &Path, model: Option<&str>) -> Result<Value> {
        extract_json(&self.call(prompt, spine_dir, model, None, false)?)
    }

    fn run_text(
        &self,
        prompt: &str,
        spine_dir: &Path,
        model: Option<&str>,
        cwd: Option<&Path>,
        read_only: bool,
    ) -> Result<String> {
        self.call(prompt, spine_dir, model, cwd, read_only)
    }
}

/// 對應 `agents.PROVIDERS` 的鍵。
pub const PROVIDER_NAMES: [&str; 2] = ["mock", "claude"];

/// 依名字建 provider（mock 讀 `REPOENGINE_MOCK_FAIL`）。未知名字回 None。
pub fn provider_by_name(name: &str) -> Option<Box<dyn AgentProvider>> {
    match name {
        "mock" => Some(Box::new(MockProvider::from_env())),
        "claude" => Some(Box::new(ClaudeProvider)),
        _ => None,
    }
}

/// 對應 `agents.judge`：呼叫 provider ＋ 內層驗證，失敗重試一次；仍壞 → `Error::External`（AgentUnsure）。
/// Python 對未知 provider 會 KeyError；Rust 回 `Error::Invalid`。
pub fn judge(
    provider_name: &str,
    prompt: &str,
    spine_dir: &Path,
    model: Option<&str>,
) -> Result<Judgement> {
    let p = provider_by_name(provider_name).ok_or_else(|| {
        Error::invalid(format!(
            "未知 provider: {provider_name}（可用：{}）",
            PROVIDER_NAMES.join(", ")
        ))
    })?;
    judge_with(p.as_ref(), prompt, spine_dir, model)
}

/// `judge` 的本體，provider 由呼叫端給（測試注入 mock 用）。
pub fn judge_with(
    provider: &dyn AgentProvider,
    prompt: &str,
    spine_dir: &Path,
    model: Option<&str>,
) -> Result<Judgement> {
    let mut last_err = String::new();
    for _attempt in 0..2 {
        match provider
            .run_judgement(prompt, spine_dir, model)
            .and_then(|v| validate_judgement(&v))
        {
            Ok(j) => return Ok(j),
            Err(e @ Error::External(_)) => return Err(e),
            Err(e) => last_err = e.to_string(), // 內層解析/驗證失敗 → 重試一次
        }
    }
    Err(Error::External(format!("輸出驗證兩次皆失敗: {last_err}")))
}

/// 對應 `agents.run_text`：自由文字輸出（P5 digest、ask_repo 用），空輸出視為失敗重試一次；
/// 仍壞 → `Error::External`。cwd＝在哪個目錄跑（ask_repo：對方 repo，它的 AGENTS.md／skill 才會生效）。
pub fn run_text(
    provider_name: &str,
    prompt: &str,
    spine_dir: &Path,
    model: Option<&str>,
    cwd: Option<&Path>,
    read_only: bool,
) -> Result<String> {
    let p = provider_by_name(provider_name).ok_or_else(|| {
        Error::External(format!(
            "未知 provider: {provider_name}（可用：{}）",
            PROVIDER_NAMES.join(", ")
        ))
    })?;
    run_text_with(p.as_ref(), prompt, spine_dir, model, cwd, read_only)
}

/// `run_text` 的本體，provider 由呼叫端給。
pub fn run_text_with(
    provider: &dyn AgentProvider,
    prompt: &str,
    spine_dir: &Path,
    model: Option<&str>,
    cwd: Option<&Path>,
    read_only: bool,
) -> Result<String> {
    let mut last_err = String::new();
    for _attempt in 0..2 {
        match provider.run_text(prompt, spine_dir, model, cwd, read_only) {
            Ok(out) if !out.trim().is_empty() => return Ok(out.trim().to_string()),
            Ok(_) => last_err = "空輸出".into(),
            Err(e @ Error::External(_)) => return Err(e),
            Err(e) => last_err = e.to_string(),
        }
    }
    Err(Error::External(format!("文字輸出兩次皆失敗: {last_err}")))
}
