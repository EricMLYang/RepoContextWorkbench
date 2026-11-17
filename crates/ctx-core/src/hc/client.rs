//! herdr-core 的一條分幀連線（Unix socket）：channel 0 上的 JSON-RPC、attach／events 串流、CREDIT 流量控制。
//!
//! - 連上後先在 plain 模式送 `hello {"framed": true}`，讀完那一行回覆就切到分幀（[`super::frame`]）。
//! - 一條讀取執行緒解所有 frame：channel 0 的回覆交給等它的呼叫端；串流的 frame 交給註冊在那個 channel 的處理器。
//! - **CREDIT**：串流一開始有 256 KiB 額度；每個 DATA frame 交給處理器之後，立刻回 CREDIT（那個 frame 的長度）——
//!   不回，core 送滿額度就停（畫面凍住）。
//! - 開串流的呼叫（`terminal.attach`、`events.subscribe`）把處理器跟請求一起登記：讀取執行緒收到回覆時，
//!   **先**把處理器掛到回覆裡的 channel、再處理下一個 frame——規格保證回覆先於串流 frame，這樣第一筆畫面不會掉。
//! - 讀取執行緒碰到 EOF／錯誤：所有等回覆的呼叫拿到錯誤、所有串流收到 [`StreamMsg::Lost`]，再呼叫 `on_lost`（重連交給上層）。
//! - 寫入：一把鎖保護 socket，一個 frame 一次 `write_all`，沒有額外緩衝（輸入延遲只多一次 syscall）。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{json, Value};

use super::frame::{self, Decoder, Kind};

/// 一般呼叫等回覆的上限。
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// 送不出去就放棄的上限（core 卡住時不要讓輸入永遠擋著）。
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// herdr-core 回的錯誤（JSON-RPC error：`kind` 與 `hint` 保留下來），或連線層的問題（`kind` = `unavailable`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub kind: String,
    pub hint: String,
}

impl RpcError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        RpcError {
            code: -32000,
            message: message.into(),
            kind: "unavailable".into(),
            hint: String::new(),
        }
    }

    fn from_json(v: &Value) -> Self {
        RpcError {
            code: v.get("code").and_then(Value::as_i64).unwrap_or(-32000),
            message: v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("herdr-core 回了錯誤但沒有說明")
                .to_string(),
            kind: v
                .pointer("/data/kind")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            hint: v
                .pointer("/data/hint")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        match (self.kind.is_empty(), self.hint.is_empty()) {
            (true, true) => Ok(()),
            (false, true) => write!(f, "（{}）", self.kind),
            (true, false) => write!(f, "（{}）", self.hint),
            (false, false) => write!(f, "（{}：{}）", self.kind, self.hint),
        }
    }
}

impl std::error::Error for RpcError {}

impl From<RpcError> for crate::Error {
    fn from(e: RpcError) -> Self {
        crate::Error::External(format!("herdr-core：{e}"))
    }
}

/// 串流收到的東西。
#[derive(Debug)]
pub enum StreamMsg<'a> {
    /// 串流位元組（attach：第一畫面然後即時輸出；events：JSON 行）
    Data(&'a [u8]),
    /// core 告訴觀察者的新尺寸
    Resize(u16, u16),
    /// channel 結束：core 給的原因（`{"reason":"exited","exit_code":0,…}`）；我們自己 detach 的回應是 None
    Close(Option<Value>),
    /// 整條連線斷了（原因）
    Lost(&'a str),
}

pub type StreamHandler = Box<dyn FnMut(StreamMsg<'_>) + Send>;
type SharedHandler = Arc<Mutex<StreamHandler>>;
type Reply = std::result::Result<Value, RpcError>;

struct Pending {
    reply: mpsc::Sender<Reply>,
    stream: Option<StreamHandler>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 一條分幀連線。
pub struct Client {
    writer: Mutex<UnixStream>,
    pending: Mutex<HashMap<u64, Pending>>,
    streams: Mutex<HashMap<u32, SharedHandler>>,
    next_id: AtomicU64,
    dead: Mutex<Option<String>>,
    /// 我們自己要關（App 結束）：之後的 EOF 不算斷線
    closing: AtomicBool,
    hello: Value,
}

impl Client {
    /// 連上 `socket`、切到分幀、起讀取執行緒。`on_lost` 在連線意外斷掉時呼叫一次（自己關的不會）。
    pub fn connect(
        socket: &Path,
        on_lost: impl FnOnce(String) + Send + 'static,
    ) -> std::result::Result<Arc<Client>, RpcError> {
        let lost = |e: std::io::Error| {
            RpcError::unavailable(format!("連不上 herdr-core（{}）：{e}", socket.display()))
        };
        let mut stream = UnixStream::connect(socket).map_err(lost)?;
        stream.set_read_timeout(Some(CALL_TIMEOUT)).map_err(lost)?;
        stream
            .set_write_timeout(Some(WRITE_TIMEOUT))
            .map_err(lost)?;
        let hello = json!({"jsonrpc": "2.0", "id": 0, "method": "hello",
                           "params": {"framed": true, "generation": 1}});
        stream
            .write_all(format!("{hello}\n").as_bytes())
            .map_err(lost)?;
        // 一個 byte 一個 byte 讀到換行：切到分幀之後的 frame 不能被讀進這一行
        let mut line = Vec::new();
        let mut b = [0u8; 1];
        loop {
            match stream.read(&mut b) {
                Ok(0) => return Err(RpcError::unavailable("herdr-core 在 hello 之前就關了連線")),
                Ok(_) if b[0] == b'\n' => break,
                Ok(_) => {
                    line.push(b[0]);
                    if line.len() > frame::MAX_PAYLOAD {
                        return Err(RpcError::unavailable("herdr-core 的 hello 回覆太長"));
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(lost(e)),
            }
        }
        let reply: Value = serde_json::from_slice(&line).map_err(|e| {
            RpcError::unavailable(format!("herdr-core 的 hello 回覆不是 JSON：{e}"))
        })?;
        if let Some(err) = reply.get("error") {
            return Err(RpcError::from_json(err));
        }
        let result = reply.get("result").cloned().unwrap_or(Value::Null);
        if result.get("framed") != Some(&Value::Bool(true)) {
            return Err(RpcError::unavailable(
                "herdr-core 沒有切到分幀模式（版本太舊？）",
            ));
        }
        stream.set_read_timeout(None).map_err(lost)?;
        let reader = stream.try_clone().map_err(lost)?;
        let client = Arc::new(Client {
            writer: Mutex::new(stream),
            pending: Mutex::new(HashMap::new()),
            streams: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            dead: Mutex::new(None),
            closing: AtomicBool::new(false),
            hello: result,
        });
        let c2 = client.clone();
        std::thread::Builder::new()
            .name("hc-read".into())
            .spawn(move || {
                let why = c2.read_loop(reader);
                c2.fail_all(&why);
                if !c2.closing.load(Ordering::SeqCst) {
                    on_lost(why);
                }
            })
            .map_err(lost)?;
        Ok(client)
    }

    /// `hello` 的回覆（版本、features…）。
    pub fn hello(&self) -> &Value {
        &self.hello
    }

    /// core 的版本（`hello.version`）。
    pub fn version(&self) -> &str {
        self.hello
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    /// 連線還能用嗎。
    pub fn alive(&self) -> bool {
        lock(&self.dead).is_none()
    }

    fn read_loop(&self, mut reader: UnixStream) -> String {
        let mut dec = Decoder::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) => return "herdr-core 關了連線".into(),
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return format!("讀 herdr-core 失敗：{e}"),
            };
            dec.push(&buf[..n]);
            loop {
                match dec.next_frame() {
                    Ok(Some(f)) => self.dispatch(f),
                    Ok(None) => break,
                    Err(e) => return e.to_string(),
                }
            }
        }
    }

    fn dispatch(&self, f: frame::Frame) {
        if f.channel == 0 {
            if f.kind == Kind::Data {
                self.on_rpc(&f.payload);
            }
            return;
        }
        let handler = lock(&self.streams).get(&f.channel).cloned();
        match f.kind {
            Kind::Data => {
                if let Some(h) = handler {
                    (lock(&h))(StreamMsg::Data(&f.payload));
                }
                // 消化了就還額度（沒有處理器也要還：可能是剛 detach 的尾巴，不還的話那條串流會卡住）
                self.send(f.channel, Kind::Credit, &frame::credit(f.payload.len()));
            }
            Kind::Close => {
                lock(&self.streams).remove(&f.channel);
                if let Some(h) = handler {
                    let reason = serde_json::from_slice::<Value>(&f.payload).ok();
                    (lock(&h))(StreamMsg::Close(reason));
                }
            }
            Kind::Resize if f.payload.len() >= 4 => {
                if let Some(h) = handler {
                    let cols = u16::from_le_bytes([f.payload[0], f.payload[1]]);
                    let rows = u16::from_le_bytes([f.payload[2], f.payload[3]]);
                    (lock(&h))(StreamMsg::Resize(cols, rows));
                }
            }
            _ => {}
        }
    }

    fn on_rpc(&self, payload: &[u8]) {
        let Ok(msg) = serde_json::from_slice::<Value>(payload) else {
            return;
        };
        // 沒有 id 的是通知（plain 模式的 events）；分幀模式的 events 走自己的 channel，這裡略過
        let Some(id) = msg.get("id").and_then(Value::as_u64) else {
            return;
        };
        let Some(p) = lock(&self.pending).remove(&id) else {
            return; // 不等回覆的呼叫（notify）或已逾時
        };
        let reply = match msg.get("error") {
            Some(err) => Err(RpcError::from_json(err)),
            None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
        };
        if let (Ok(result), Some(h)) = (&reply, p.stream) {
            // 先掛處理器，讀取執行緒才去讀下一個 frame：串流的第一筆一定有人收
            if let Some(ch) = result
                .get("channel")
                .and_then(Value::as_u64)
                .and_then(|c| u32::try_from(c).ok())
            {
                lock(&self.streams).insert(ch, Arc::new(Mutex::new(h)));
            }
        }
        let _ = p.reply.send(reply);
    }

    fn fail_all(&self, why: &str) {
        *lock(&self.dead) = Some(why.to_string());
        let pending: Vec<Pending> = lock(&self.pending).drain().map(|(_, p)| p).collect();
        for p in pending {
            let _ = p.reply.send(Err(RpcError::unavailable(why.to_string())));
        }
        let streams: Vec<SharedHandler> = lock(&self.streams).drain().map(|(_, h)| h).collect();
        for h in streams {
            (lock(&h))(StreamMsg::Lost(why));
        }
    }

    /// 送一個 frame；送不出去回 false（連線斷了）。
    pub fn send(&self, channel: u32, kind: Kind, payload: &[u8]) -> bool {
        let Ok(bytes) = frame::encode(channel, kind, payload) else {
            return false;
        };
        if !self.alive() {
            return false;
        }
        lock(&self.writer).write_all(&bytes).is_ok()
    }

    fn request(
        &self,
        method: &str,
        params: Value,
        stream: Option<StreamHandler>,
    ) -> std::result::Result<(u64, mpsc::Receiver<Reply>), RpcError> {
        if let Some(why) = lock(&self.dead).clone() {
            return Err(RpcError::unavailable(why));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        lock(&self.pending).insert(id, Pending { reply: tx, stream });
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        if !self.send(0, Kind::Data, msg.to_string().as_bytes()) {
            lock(&self.pending).remove(&id);
            return Err(RpcError::unavailable(format!(
                "送不出 {method} 給 herdr-core（連線斷了）"
            )));
        }
        Ok((id, rx))
    }

    fn wait(&self, id: u64, rx: mpsc::Receiver<Reply>, method: &str, timeout: Duration) -> Reply {
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => {
                lock(&self.pending).remove(&id);
                Err(RpcError::unavailable(format!(
                    "herdr-core 沒在 {} 秒內回 {method}",
                    timeout.as_secs()
                )))
            }
        }
    }

    /// 一個呼叫，等回覆（[`CALL_TIMEOUT`]）。**不可以在串流處理器裡呼叫**（處理器跑在讀取執行緒上，會等不到自己）。
    pub fn call(&self, method: &str, params: Value) -> Reply {
        self.call_with_timeout(method, params, CALL_TIMEOUT)
    }

    pub fn call_with_timeout(&self, method: &str, params: Value, timeout: Duration) -> Reply {
        let (id, rx) = self.request(method, params, None)?;
        self.wait(id, rx, method, timeout)
    }

    /// 開串流的呼叫：`handler` 在回覆到的那一刻掛上回覆裡的 `channel`。回傳回覆（含 `channel`）。
    pub fn call_stream(&self, method: &str, params: Value, handler: StreamHandler) -> Reply {
        let (id, rx) = self.request(method, params, Some(handler))?;
        self.wait(id, rx, method, CALL_TIMEOUT)
    }

    /// 不等回覆的呼叫（讀取執行緒、結束收尾用）。
    pub fn notify(&self, method: &str, params: Value) -> bool {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send(0, Kind::Data, msg.to_string().as_bytes())
    }

    /// 拆掉一條串流（detach）：先拿掉處理器，再送 CLOSE。之後那個 channel 的 frame 都丟掉（DATA 照樣還額度）。
    pub fn close_stream(&self, channel: u32) {
        lock(&self.streams).remove(&channel);
        self.send(channel, Kind::Close, &[]);
    }

    /// 我們自己關掉連線（App 結束、manager 丟掉）：之後的 EOF 不算斷線、不重連，串流也不會收到 Lost。
    pub fn shutdown(&self) {
        self.closing.store(true, Ordering::SeqCst);
        lock(&self.streams).clear();
        let _ = lock(&self.writer).shutdown(std::net::Shutdown::Both);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.closing.store(true, Ordering::SeqCst);
        let _ = lock(&self.writer).shutdown(std::net::Shutdown::Both);
    }
}
