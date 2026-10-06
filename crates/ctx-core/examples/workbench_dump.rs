//! 對拍用：印出同一個 spine 的工作台畫面資料（`scripts/parity_state.py` 拿它跟 Python 版比）。
//!
//! 用法：`cargo run -q -p ctx-core --example workbench_dump -- <spine>`
//! 輸出一個 JSON 物件，鍵＝情境名（state_all／state_g1／state_adhoc／scan_all／scan_g1／scan_adhoc），
//! 值＝`build_state`／`build_scan` 的結果。Python 端產生同一組鍵。

use std::path::PathBuf;

use ctx_core::workbench::{build_scan, build_state, StateScope};
use serde_json::{json, Value};

fn main() -> ctx_core::Result<()> {
    let spine = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| ctx_core::Error::invalid("用法：workbench_dump <spine>"))?,
    );
    let adhoc = vec!["repo-a".to_string()];
    let out: Value = json!({
        "state_all": build_state(&spine, &StateScope::default())?,
        "state_g1": build_state(&spine, &StateScope::new(Some("g1"), None))?,
        "state_adhoc": build_state(&spine, &StateScope::new(None, Some(&adhoc)))?,
        "scan_all": build_scan(&spine, None, None)?,
        "scan_g1": build_scan(&spine, Some("g1"), None)?,
        "scan_adhoc": build_scan(&spine, None, Some(&adhoc))?,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
