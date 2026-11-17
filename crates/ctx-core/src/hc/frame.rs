//! herdr-core 的分幀模式（`hello {"framed": true}` 之後）：`[channel u32 LE][kind u8][len u32 LE][payload]`。
//!
//! channel 0 是 JSON-RPC（一個 frame 一則訊息）；其他 channel 是 attach／events 串流。payload 上限 16 MiB，
//! 超過就是對方壞了（或不是 herdr-core）——拒絕並斷線，不嘗試讀那麼多。
//! 規格：herdr-core `schema/README.md`「Framed mode」。這裡只有純函式與增量解碼器，不碰 socket。

use std::fmt;

/// 標頭長度。
pub const HEADER: usize = 9;
/// payload 上限（16 MiB）。
pub const MAX_PAYLOAD: usize = 16 << 20;
/// 每個串流一開始的額度（對方不等我們回 CREDIT 就能送的位元組；規格 256 KiB）。
pub const WINDOW: usize = 256 * 1024;

/// frame 種類。不認得的照原樣保留（之後版本的 core 可能多送幾種，收到就略過）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// channel 0：一則 JSON-RPC；其他 channel：串流位元組
    Data,
    /// u32 LE：接收端已經消化了幾個位元組
    Credit,
    /// 這個 channel 結束了；core 送的會帶 JSON 原因
    Close,
    /// u16 LE cols、u16 LE rows
    Resize,
    /// viewer → core：1＝拿到焦點、0＝失去
    Focus,
    /// viewer → core：顯示資訊（JSON）
    Facts,
    /// viewer → core：終端自動回的位元組（不是人打的）
    AutomaticInput,
    Other(u8),
}

impl Kind {
    pub fn byte(self) -> u8 {
        match self {
            Kind::Data => 0,
            Kind::Credit => 1,
            Kind::Close => 2,
            Kind::Resize => 3,
            Kind::Focus => 4,
            Kind::Facts => 5,
            Kind::AutomaticInput => 6,
            Kind::Other(b) => b,
        }
    }

    pub fn from_byte(b: u8) -> Kind {
        match b {
            0 => Kind::Data,
            1 => Kind::Credit,
            2 => Kind::Close,
            3 => Kind::Resize,
            4 => Kind::Focus,
            5 => Kind::Facts,
            6 => Kind::AutomaticInput,
            other => Kind::Other(other),
        }
    }
}

/// 一個 frame。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub channel: u32,
    pub kind: Kind,
    pub payload: Vec<u8>,
}

/// payload 超過上限：對方送壞了，連線不能再用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooLarge(pub usize);

impl fmt::Display for TooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "herdr-core 送來 {} bytes 的 frame，超過上限 {MAX_PAYLOAD}",
            self.0
        )
    }
}

/// 編一個 frame。payload 超過上限＝呼叫端的錯（我們自己不會送那麼大的東西）。
pub fn encode(channel: u32, kind: Kind, payload: &[u8]) -> Result<Vec<u8>, TooLarge> {
    if payload.len() > MAX_PAYLOAD {
        return Err(TooLarge(payload.len()));
    }
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.extend_from_slice(&channel.to_le_bytes());
    out.push(kind.byte());
    // 上面擋過上限，一定放得進 u32
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

/// CREDIT 的 payload：消化了幾個位元組（超過 u32 的部分分不出來也不會發生——單一 frame 最多 16 MiB）。
pub fn credit(n: usize) -> [u8; 4] {
    u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes()
}

/// RESIZE 的 payload。
pub fn resize(cols: u16, rows: u16) -> [u8; 4] {
    let c = cols.to_le_bytes();
    let r = rows.to_le_bytes();
    [c[0], c[1], r[0], r[1]]
}

/// 增量解碼器：socket 讀到多少就 [`Decoder::push`] 多少，再用 [`Decoder::next_frame`] 取出完整的 frame。
/// 讀取可以切在任何位置（標頭中間、payload 中間）。
#[derive(Debug, Default)]
pub struct Decoder {
    buf: Vec<u8>,
    /// `buf` 裡已經取走的前綴長度（攢一批再一起丟，免得每個 frame 都搬記憶體）
    start: usize,
}

impl Decoder {
    pub fn new() -> Self {
        Decoder::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start == self.buf.len() {
            self.buf.clear();
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// 下一個完整的 frame；資料還不夠＝`Ok(None)`。超過上限＝`Err`（之後不該再用這個解碼器）。
    pub fn next_frame(&mut self) -> Result<Option<Frame>, TooLarge> {
        let rest = &self.buf[self.start..];
        if rest.len() < HEADER {
            return Ok(None);
        }
        let channel = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let kind = Kind::from_byte(rest[4]);
        let len = u32::from_le_bytes([rest[5], rest[6], rest[7], rest[8]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(TooLarge(len));
        }
        if rest.len() < HEADER + len {
            return Ok(None);
        }
        let payload = rest[HEADER..HEADER + len].to_vec();
        self.start += HEADER + len;
        if self.start > 64 * 1024 && self.start * 2 > self.buf.len() {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        Ok(Some(Frame {
            channel,
            kind,
            payload,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_split_reads_at_every_position() {
        let frames = vec![
            Frame {
                channel: 0,
                kind: Kind::Data,
                payload: br#"{"jsonrpc":"2.0","id":1,"result":{}}"#.to_vec(),
            },
            Frame {
                channel: 7,
                kind: Kind::Credit,
                payload: credit(1234).to_vec(),
            },
            Frame {
                channel: 7,
                kind: Kind::Close,
                payload: Vec::new(),
            },
            Frame {
                channel: 3,
                kind: Kind::Other(42),
                payload: vec![1, 2, 3],
            },
        ];
        let wire: Vec<u8> = frames
            .iter()
            .flat_map(|f| encode(f.channel, f.kind, &f.payload).unwrap())
            .collect();
        // 一次全給
        let mut d = Decoder::new();
        d.push(&wire);
        let mut got = Vec::new();
        while let Some(f) = d.next_frame().unwrap() {
            got.push(f);
        }
        assert_eq!(got, frames);
        // 切在每一個位置
        for cut in 0..=wire.len() {
            let mut d = Decoder::new();
            let mut got = Vec::new();
            d.push(&wire[..cut]);
            while let Some(f) = d.next_frame().unwrap() {
                got.push(f);
            }
            d.push(&wire[cut..]);
            while let Some(f) = d.next_frame().unwrap() {
                got.push(f);
            }
            assert_eq!(got, frames, "cut at {cut}");
        }
        // 一次一個 byte
        let mut d = Decoder::new();
        let mut got = Vec::new();
        for b in &wire {
            d.push(std::slice::from_ref(b));
            while let Some(f) = d.next_frame().unwrap() {
                got.push(f);
            }
        }
        assert_eq!(got, frames);
    }

    #[test]
    fn rejects_payloads_over_16_mib() {
        let mut header = Vec::new();
        header.extend_from_slice(&1u32.to_le_bytes());
        header.push(0);
        header.extend_from_slice(&((MAX_PAYLOAD + 1) as u32).to_le_bytes());
        let mut d = Decoder::new();
        d.push(&header);
        assert_eq!(d.next_frame(), Err(TooLarge(MAX_PAYLOAD + 1)));
        // 剛好上限是可以的（只看標頭，資料還沒到＝None）
        let mut ok = Vec::new();
        ok.extend_from_slice(&1u32.to_le_bytes());
        ok.push(0);
        ok.extend_from_slice(&(MAX_PAYLOAD as u32).to_le_bytes());
        let mut d = Decoder::new();
        d.push(&ok);
        assert_eq!(d.next_frame(), Ok(None));
        assert!(encode(1, Kind::Data, &vec![0; MAX_PAYLOAD + 1]).is_err());
    }

    #[test]
    fn resize_and_credit_payloads_are_little_endian() {
        assert_eq!(resize(0x0102, 0x0304), [0x02, 0x01, 0x04, 0x03]);
        assert_eq!(credit(0x01020304), [0x04, 0x03, 0x02, 0x01]);
        assert_eq!(
            Kind::from_byte(Kind::AutomaticInput.byte()),
            Kind::AutomaticInput
        );
    }
}
