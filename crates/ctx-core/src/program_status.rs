//! OSC 7501（Program Status Protocol）：agent 把自己的狀態寫進終端輸出（F-130，Rust 才有）。
//!
//! `ESC ] 7501 ; <body> ST`（ST＝`ESC \`，也接受 BEL）。body 是冒號分隔的 `key=value`：
//! `state=idle|working|done|blocked|error|clear`、`kind=permission|question|auth`（只跟 blocked）、
//! `progress=0..100`（只跟 working／blocked）、`app`、`msg`／`title`（base64、單行）、`id`（子紀錄，`a/b` 階層）。
//! 探詢是 body 只有 `?`，終端回 [`QUERY_REPLY`]。文法與上限照規格與 Orca（MIT）的
//! `src/shared/program-status/protocol.ts`，用 Rust 自己寫。
//!
//! 這裡只有純函式與純狀態機：[`Scanner`] 吃 PTY 讀到的位元組片段、吐出 [`ProgramEvent`]，
//! **不改動、不吃掉**輸出（呼叫端照常把同一份位元組往前端送）。序列可以被切在兩次讀取之間的任何位置。
//! 誰回應探詢、狀態怎麼存，見 `term.rs` 的 `reader_loop` 與 `session_status::record_program`。

use serde::{Deserialize, Serialize};

/// 序列引導：`ESC ] 7501 ;`。
const PREFIX: &[u8] = b"\x1b]7501;";

/// 規格：整個序列最多 4096 bytes；`ESC ] 7501 ;` 與 `ESC \` 佔掉 9 個。
pub const MAX_SEQUENCE_BYTES: usize = 4096;
const MAX_BODY_BYTES: usize = MAX_SEQUENCE_BYTES - PREFIX.len() - 2;
const MAX_KEY_BYTES: usize = 16;
const MAX_APP_BYTES: usize = 32;
const MAX_MSG_ENCODED: usize = 2732;
const MAX_MSG_DECODED: usize = 2048;
const MAX_TITLE_ENCODED: usize = 256;
const MAX_TITLE_DECODED: usize = 192;
const MAX_ID_LEN: usize = 128;
const MAX_ID_DEPTH: usize = 8;
const MAX_ID_SEGMENT: usize = 32;

/// 探詢的回應：明列這個終端接受的 state 與 kind，之後版本的程式才知道能送什麼。
/// 只有這串會被寫回 PTY。
pub const QUERY_REPLY: &[u8] =
    b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

/// 程式回報的狀態。`Clear`＝撤掉這筆紀錄（不是一個顯示狀態）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProgramState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
    Clear,
}

impl ProgramState {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "idle" => ProgramState::Idle,
            "working" => ProgramState::Working,
            "done" => ProgramState::Done,
            "blocked" => ProgramState::Blocked,
            "error" => ProgramState::Error,
            "clear" => ProgramState::Clear,
            _ => return None,
        })
    }

    /// 只有進行中的狀態可以帶 `progress`。
    fn carries_progress(self) -> bool {
        matches!(self, ProgramState::Working | ProgramState::Blocked)
    }
}

/// blocked 的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockedKind {
    /// 要你允許工具
    Permission,
    /// 要你回答
    Question,
    /// 要你登入
    Auth,
}

impl BlockedKind {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "permission" => BlockedKind::Permission,
            "question" => BlockedKind::Question,
            "auth" => BlockedKind::Auth,
            _ => return None,
        })
    }
}

/// 一筆程式回報。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramReport {
    /// 子紀錄的 id（`a/b`）；None＝根紀錄（agent 本身）。
    pub id: Option<String>,
    pub state: ProgramState,
    pub kind: Option<BlockedKind>,
    pub progress: Option<u8>,
    pub app: Option<String>,
    pub msg: Option<String>,
    pub title: Option<String>,
}

/// 掃描器吐出的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramEvent {
    /// `OSC 7501 ; ?`：程式在問終端支不支援。
    Query,
    Report(ProgramReport),
}

/// 掃描狀態：一般輸出、比對引導字串中（已對上幾個 byte）、body 裡、body 裡遇到 ESC。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scan {
    Ground,
    Prefix(usize),
    Body,
    BodyEsc,
}

/// 一個 PTY 串流的 OSC 7501 掃描器。快路徑：一般輸出只用 `position` 找下一個 ESC，不逐 byte 做事；
/// 緩衝最多 [`MAX_SEQUENCE_BYTES`]，超過就整筆丟掉、回到一般狀態。
#[derive(Debug)]
pub struct Scanner {
    state: Scan,
    body: Vec<u8>,
}

impl Default for Scanner {
    fn default() -> Self {
        Scanner::new()
    }
}

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

impl Scanner {
    pub fn new() -> Self {
        Scanner {
            state: Scan::Ground,
            body: Vec::new(),
        }
    }

    /// 餵一段輸出；每認出一個序列就呼叫 `on` 一次（依位元組順序）。不改動 `data`。
    pub fn feed(&mut self, data: &[u8], mut on: impl FnMut(ProgramEvent)) {
        let mut i = 0;
        while i < data.len() {
            match self.state {
                Scan::Ground => match data[i..].iter().position(|&b| b == ESC) {
                    Some(p) => {
                        self.state = Scan::Prefix(1);
                        i += p + 1;
                    }
                    None => return,
                },
                Scan::Prefix(n) => {
                    let b = data[i];
                    if b == PREFIX[n] {
                        if n + 1 == PREFIX.len() {
                            self.body.clear();
                            self.state = Scan::Body;
                        } else {
                            self.state = Scan::Prefix(n + 1);
                        }
                        i += 1;
                    } else {
                        // 不是 7501（CSI、別的 OSC…）：這個 byte 重新當一般輸出看（它可能就是下一個 ESC）
                        self.state = Scan::Ground;
                        if b == ESC {
                            self.state = Scan::Prefix(1);
                            i += 1;
                        }
                    }
                }
                Scan::Body => {
                    let rest = &data[i..];
                    let end = rest
                        .iter()
                        .position(|&b| matches!(b, BEL | ESC | CAN | SUB));
                    let take = end.unwrap_or(rest.len());
                    if self.body.len() + take > MAX_BODY_BYTES {
                        // 超過上限：整筆丟掉，從這裡起當一般輸出（不卡住、不累積記憶體）
                        self.discard();
                        continue;
                    }
                    self.body.extend_from_slice(&rest[..take]);
                    i += take;
                    let Some(_) = end else { return };
                    let t = data[i];
                    i += 1;
                    match t {
                        BEL => self.finish(&mut on),
                        ESC => self.state = Scan::BodyEsc,
                        _ => self.discard(), // CAN／SUB：序列被取消
                    }
                }
                Scan::BodyEsc => {
                    // ST（ESC \）結束；照 xterm，ESC 後面接別的也算結束，而那個 ESC 開始下一個序列
                    self.finish(&mut on);
                    if data[i] == b'\\' {
                        i += 1;
                    } else {
                        self.state = Scan::Prefix(1);
                    }
                }
            }
        }
    }

    fn discard(&mut self) {
        self.body.clear();
        self.body.shrink_to(MAX_BODY_BYTES);
        self.state = Scan::Ground;
    }

    fn finish(&mut self, on: &mut impl FnMut(ProgramEvent)) {
        if let Some(ev) = parse_body(&self.body) {
            on(ev);
        }
        self.body.clear();
        self.state = Scan::Ground;
    }
}

/// 解析 `OSC 7501 ;` 與 ST 之間的 body。None＝整筆忽略（沒有 state、id 不合法、超過上限、不是 UTF-8）。
/// 寫法不合的單一 `key=value` 只略過那一對，其餘照用（規格）。
pub fn parse_body(body: &[u8]) -> Option<ProgramEvent> {
    if body.len() > MAX_BODY_BYTES {
        return None;
    }
    let body = std::str::from_utf8(body).ok()?;
    // 只有恰好是「?」才是探詢；我們自己的回應（`?:states=…`）被回顯時不能又當成探詢
    if body.trim() == "?" {
        return Some(ProgramEvent::Query);
    }
    let mut state = None;
    let mut kind = None;
    let mut progress = None;
    let mut app = None;
    let mut id = None;
    let mut msg = None;
    let mut title = None;
    for pair in body.split(':') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        // 上限每一次出現都檢查，而且在略過格式不合的之前：後面重複的鍵不能洗掉超限的那個
        let over = key.len() > MAX_KEY_BYTES
            || (key == "app" && value.len() > MAX_APP_BYTES)
            || (key == "msg" && value.len() > MAX_MSG_ENCODED)
            || (key == "title" && value.len() > MAX_TITLE_ENCODED)
            || (key == "id" && !valid_id(value));
        if over {
            return None;
        }
        if !key.bytes().all(|b| b.is_ascii_lowercase())
            || key.is_empty()
            || !value.bytes().all(value_byte)
        {
            continue;
        }
        match key {
            "msg" => msg = Some(decode_text(value, MAX_MSG_DECODED)?),
            "title" => title = Some(decode_text(value, MAX_TITLE_DECODED)?),
            "state" => state = Some(value),
            "kind" => kind = Some(value),
            "progress" => progress = Some(value),
            "app" => app = Some(value),
            "id" => id = Some(value),
            _ => {}
        }
    }
    let state = ProgramState::parse(state?)?;
    let kind = kind
        .filter(|_| state == ProgramState::Blocked)
        .and_then(BlockedKind::parse);
    let progress = progress
        .filter(|_| state.carries_progress())
        .and_then(parse_progress);
    let app = app
        .filter(|a| !a.is_empty() && a.bytes().all(segment_byte))
        .map(str::to_string);
    Some(ProgramEvent::Report(ProgramReport {
        id: id.map(str::to_string),
        state,
        kind,
        progress,
        app,
        msg: msg.filter(|m| !m.is_empty()),
        title: title.filter(|t| !t.is_empty()),
    }))
}

/// value 允許的字元：`[A-Za-z0-9_.,+/=-]`。
fn value_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"_.,+/=-".contains(&b)
}

/// id 片段與 app 允許的字元：`[A-Za-z0-9_.+-]`。
fn segment_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"_.+-".contains(&b)
}

fn valid_id(id: &str) -> bool {
    id.len() <= MAX_ID_LEN
        && id.split('/').count() <= MAX_ID_DEPTH
        && id
            .split('/')
            .all(|s| (1..=MAX_ID_SEGMENT).contains(&s.len()) && s.bytes().all(segment_byte))
}

/// `0*\d{1,3}`，且不超過 100。
fn parse_progress(v: &str) -> Option<u8> {
    let digits = v.trim_start_matches('0');
    let digits = if digits.is_empty() && !v.is_empty() {
        "0"
    } else {
        digits
    };
    if digits.is_empty() || digits.len() > 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u8>().ok().filter(|p| *p <= 100)
}

/// base64 → 單行 UTF-8 文字；不合法、太長、含控制字元＝None（整筆回報丟掉）。
fn decode_text(encoded: &str, max_decoded: usize) -> Option<String> {
    let unpadded = encoded.trim_end_matches('=');
    if encoded.len() - unpadded.len() > 2 || unpadded.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(unpadded.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for b in unpadded.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    if out.len() > max_decoded {
        return None;
    }
    let text = String::from_utf8(out).ok()?;
    let control = |c: char| matches!(c as u32, 0x00..=0x1f | 0x7f..=0x9f);
    (!text.chars().any(control)).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(chunks: &[&[u8]]) -> Vec<ProgramEvent> {
        let mut s = Scanner::new();
        let mut out = Vec::new();
        for c in chunks {
            s.feed(c, |e| out.push(e));
        }
        out
    }

    fn report(state: ProgramState, app: &str) -> ProgramEvent {
        ProgramEvent::Report(ProgramReport {
            id: None,
            state,
            kind: None,
            progress: None,
            app: Some(app.into()),
            msg: None,
            title: None,
        })
    }

    #[test]
    fn full_sequences_with_st_and_bel() {
        assert_eq!(
            scan(&[b"\x1b]7501;state=working:app=codex\x1b\\"]),
            [report(ProgramState::Working, "codex")]
        );
        assert_eq!(
            scan(&[b"\x1b]7501;state=done:app=claude-code\x07"]),
            [report(ProgramState::Done, "claude-code")]
        );
        assert_eq!(scan(&[b"\x1b]7501;?\x07"]), [ProgramEvent::Query]);
        assert_eq!(scan(&[b"\x1b]7501; ? \x1b\\"]), [ProgramEvent::Query]);
        // 我們自己的回應被回顯：不是探詢，也沒有 state，什麼都不是
        assert!(scan(&[QUERY_REPLY]).is_empty());
    }

    #[test]
    fn every_split_point_gives_the_same_events() {
        let stream: &[u8] = b"hi \x1b[31mred\x1b[0m \x1b]0;title\x07\x1b]7501;?\x07mid\
            \x1b]8;;https://x.y\x1b\\link\x1b]8;;\x1b\\\x1b]7501;state=blocked:kind=permission:progress=042:app=codex\x1b\\tail\
            \x1b]7501;state=done:app=codex\x07";
        let whole = scan(&[stream]);
        assert_eq!(whole.len(), 3);
        assert_eq!(whole[0], ProgramEvent::Query);
        assert_eq!(
            whole[1],
            ProgramEvent::Report(ProgramReport {
                id: None,
                state: ProgramState::Blocked,
                kind: Some(BlockedKind::Permission),
                progress: Some(42),
                app: Some("codex".into()),
                msg: None,
                title: None,
            })
        );
        for cut in 0..=stream.len() {
            let (a, b) = stream.split_at(cut);
            assert_eq!(scan(&[a, b]), whole, "切在 {cut}");
        }
        // 一個 byte 一個 byte 餵也一樣
        let bytes: Vec<&[u8]> = stream.chunks(1).collect();
        assert_eq!(scan(&bytes), whole);
    }

    #[test]
    fn over_long_sequences_are_dropped_and_the_next_one_still_parses() {
        let mut long = b"\x1b]7501;state=working:app=".to_vec();
        long.extend(std::iter::repeat_n(b'a', MAX_SEQUENCE_BYTES));
        long.extend_from_slice(b"\x1b\\");
        long.extend_from_slice(b"\x1b]7501;state=done:app=codex\x1b\\");
        let mut s = Scanner::new();
        let mut out = Vec::new();
        for c in long.chunks(100) {
            s.feed(c, |e| out.push(e));
            assert!(s.body.len() <= MAX_BODY_BYTES);
        }
        assert_eq!(out, [report(ProgramState::Done, "codex")]);
        // 剛好在上限內的 body 照收（app 太長會被丟，所以用不認得的鍵墊）
        let mut fit = b"\x1b]7501;state=idle:zz=".to_vec();
        let pad = MAX_BODY_BYTES - "state=idle:zz=".len();
        fit.extend(std::iter::repeat_n(b'a', pad));
        fit.extend_from_slice(b"\x1b\\");
        assert_eq!(fit.len(), MAX_SEQUENCE_BYTES);
        assert_eq!(scan(&[&fit]).len(), 1);
    }

    #[test]
    fn malformed_bodies_are_ignored() {
        for bad in [
            &b"\x1b]7501;app=codex\x07"[..],         // 沒有 state
            b"\x1b]7501;state=sleeping\x07",         // 不認得的 state
            b"\x1b]7501;state=working:id=a//b\x07",  // id 空片段
            b"\x1b]7501;state=working:msg=!!!!\x07", // msg 不是 base64（格式不合→略過那對，但…）
            b"\x1b]7501;state=working:msg=QQ=Q\x07", // base64 中間有 =
            b"\x1b]7501;state=working:msg=AA==\x07", // 解出來是 NUL（控制字元）
            b"\x1b]7501;state=working:\xff\xfe\x07", // 不是 UTF-8
            b"\x1b]7501;state=work\x18ing\x07",      // CAN 取消
        ] {
            let got = scan(&[bad]);
            // msg=!!!! 不符合 value 字元集：只略過那對，其餘照用（規格）
            if bad == b"\x1b]7501;state=working:msg=!!!!\x07" {
                assert_eq!(got.len(), 1);
                continue;
            }
            assert!(
                got.is_empty(),
                "{:?} → {got:?}",
                String::from_utf8_lossy(bad)
            );
        }
        // kind 只跟 blocked、progress 只跟 working／blocked 且不超過 100
        let ProgramEvent::Report(r) =
            parse_body(b"state=done:kind=permission:progress=50").unwrap()
        else {
            panic!()
        };
        assert_eq!((r.kind, r.progress), (None, None));
        let ProgramEvent::Report(r) = parse_body(b"state=working:progress=101").unwrap() else {
            panic!()
        };
        assert_eq!(r.progress, None);
    }

    #[test]
    fn msg_title_and_child_ids_decode() {
        // 「等你回答」＝base64 562J5L2g5Zue562U；title 不帶補位的 =
        let body = b"state=blocked:kind=question:msg=562J5L2g5Zue562U:title=VGFzaw:id=sub/a1";
        let ProgramEvent::Report(r) = parse_body(body).unwrap() else {
            panic!()
        };
        assert_eq!(r.kind, Some(BlockedKind::Question));
        assert_eq!(r.msg.as_deref(), Some("等你回答"));
        assert_eq!(r.title.as_deref(), Some("Task"));
        assert_eq!(r.id.as_deref(), Some("sub/a1"));
    }

    #[test]
    fn output_is_left_untouched() {
        // 掃描器只讀不寫：餵過的位元組與原本完全一樣（借用檢查也保證，這裡寫明意圖）
        let data = b"a\x1b]7501;state=working:app=x\x1b\\b".to_vec();
        let copy = data.clone();
        let mut s = Scanner::new();
        s.feed(&data, |_| {});
        assert_eq!(data, copy);
    }
}
