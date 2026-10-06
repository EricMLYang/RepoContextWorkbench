# Python → Rust 搬遷規約

> 2026-10-06 起在 `rust-desktop` 分支把 `repo_context`（Python）整份搬成 Rust＋Tauri＋React。
> 方向見 `AGENTS.md`。Python 版在搬完、對拍通過前**不刪**——它是規格，也是使用者正在用的 `ctx`。

## 版面

```
Cargo.toml                 workspace
crates/ctx-core/           引擎 lib（對應 repo_context/*.py，不依賴 Tauri、不依賴 CLI 框架）
crates/ctx-cli/            bin `ctx`：CLI（clap）＋MCP server（stdio）＋hooks 入口
desktop/                   Tauri v2 殼（src-tauri，依賴 ctx-core）＋React（Vite＋TS）前端
```

模組一對一：`repo_context/spine.py` → `crates/ctx-core/src/spine.rs`，`registry/` → `src/registry/`，
`webui/state.py`、`webui/actions.py` → `src/workbench/{state,actions}.rs`（給桌面 App 用，與 HTTP 無關）。

## 合約（不准改）

- spine 事件檔 `spine/events/YYYY-MM-DD.md` 的標頭文法、dead-letter 檔、`.engine.lock` 鎖檔協定（O_CREAT|O_EXCL＋stale 回收）。
- `registry.yaml`、`config.yaml` 的鍵與語意；寫回時**保留不認得的鍵**（Python 版寫什麼、Rust 版要讀得回，反之亦然）。
- `.state/` 底下各 JSON／txt 的路徑與欄位。
- 環境變數：`REPOENGINE_SPINE`、`REPOCTX_NO_BACKGROUND`、`XDG_CONFIG_HOME`；使用者設定檔 `~/.config/repo-context/config.json`。
- CLI 子命令、旗標、`--json` 輸出的欄位、exit code（0 成功／1 有紅字／2 用法錯或被拒／3 找不到 spine）。
- MCP 工具名、inputSchema、回傳 JSON 欄位、錯誤形狀 `{ok:false,error,message,hint}`。
- 給人看的中文訊息：測試有斷言的要逐字一致；其餘盡量照抄。

兩邊同時讀寫同一個 spine 是常態（搬遷期間全域 `ctx` 仍是 Python），所以「格式相容」比「Rust 寫法漂亮」優先。

## Rust 寫法

- 錯誤一律 `ctx_core::Result<T>`（`src/error.rs`）。Python 的 `ValueError` → `Error::Invalid`，
  `spine.ValidationError` → `Error::Validation`，`AgentUnsure` 之類外部失敗 → `Error::External`。
- 只有程式自己保證的不變式才 `unwrap`／`expect`；讀檔、解析、外部指令都要傳錯誤。
- 型別表達狀態（例：事件型別用 enum、tier 用 enum），但**資料檔的序列化字串與 Python 相同**。
- 可擴充的設定／registry 欄位用 typed struct＋`#[serde(flatten)] extra` 保留未知鍵；
  真的是自由形狀的才用 `serde_yaml::Value`／`serde_json::Value`。
- 時間：`chrono::Local`；凡是 Python 收 `when=`／`today=` 參數的函式，Rust 也收 `Option<NaiveDateTime>` 之類，方便測試。
- 外部指令（git／gh／claude）用 `std::process::Command`，參數與 Python 相同；不引入 git2。
- 依賴從嚴：要加 crate 先看 workspace `Cargo.toml` 有沒有；新增要有理由（寫在 commit message）。
- 每個 pub 函式第一行 doc comment 寫它對應哪個 Python 函式，例：`/// 對應 spine.append_event`。
- 保留 Python 檔頭 docstring 的「為什麼」（精簡成模組 `//!` 註解），不要只留「做什麼」。

## 測試

- 每個 `tests/test_<x>.py` 搬成 `crates/ctx-core/tests/<x>.rs`（CLI e2e 搬到 `crates/ctx-cli/tests/`），
  測試名稱照 Python 函式名，一個 Python 測試對一個 Rust 測試；搬不了的寫明原因（`// 未搬：…`）。
- 共用 fixture 在 `ctx_core::testkit`（feature `testkit`）：temp spine、假 git repo（照 `tests/conftest.py`）。
- 測試裡設 `REPOCTX_NO_BACKGROUND=1`，不碰使用者真的 `~/.config`（測試用 temp `XDG_CONFIG_HOME`）。
- 完成條件：`cargo test --workspace`、`cargo clippy --workspace -- -D warnings`、`cargo fmt --check` 全過。

## 對拍

`scripts/parity.sh`：同一份 fixture spine 複製兩份，各跑 Python `python -m repo_context` 與
`target/debug/ctx` 的同一組指令，比較 `--json` 輸出（解析後比，不比位元組）與寫出的檔。
全部指令對拍通過才合併回 main、才把 hooks／MCP 改指向 Rust 版。
