//! 方法表（N-52）：每個 MCP 工具在這裡宣告**一次**——名稱、唯讀與否、說明、處理函式；
//! 參數型別由處理函式的簽名帶出來，`inputSchema` 由參數型別生成（`params::input_schema`）。
//! `tools/list`、`tools/call` 的分派、唯讀標記、`--admin` 分層都只讀這張表。宣告順序＝tools/list 順序。
//!
//! 加一個工具：寫參數 struct＋處理函式（`work.rs`／`admin.rs`），在下面加一行。
//! 加一個參數：在參數 struct 加一個欄位（doc comment 就是說明）。
//! 兩者都會讓 `schema/mcp.tools.json` 過期——`CTX_UPDATE_SCHEMA=1 cargo test -p ctx-cli --test mcp_schema` 重生，
//! 相容規則見 `docs/DEVELOPMENT.md` §6。

use super::*;
use serde::de::DeserializeOwned;

/// tools/list 要不要標 `annotations.readOnlyHint`（MCP 標準欄位，2026-11-14）。
/// Codex 對沒標的工具每次都要使用者核准（`codex exec` 沒人可問，直接拒絕）；標了才會直接放行，
/// 查脈絡不必一直按同意。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// 只讀不寫：標 readOnlyHint。
    ReadOnly,
    /// 有副作用——會留痕的（log_decision、handoff…）、會記引用的（read_from）、會另開 agent 花額度的
    /// （ask_repo）都算；不標。管理層工具一律算這種（給人用的維護工具，不必自動放行）。
    Effects,
}

use Access::{Effects, ReadOnly};

/// 工作層工具：`f` 收參數型別 `P`（arguments 反序列化而來）、回 JSON。缺必填照 Python TypeError 報。
fn work<P, F>(name: &'static str, access: Access, description: &str, f: F) -> Tool
where
    P: JsonSchema + DeserializeOwned,
    F: Fn(&Path, P, &Path) -> Result<Value> + Send + Sync + 'static,
{
    let schema = input_schema::<P>();
    let required = required_of(&schema);
    Tool {
        name: name.into(),
        description: description.into(),
        schema,
        access,
        kind: ToolKind::Work(Box::new(move |d, a, here| {
            f(d, parse(name, a, &required, Missing::Positional)?, here)
        })),
    }
}

/// 管理層工具（`--admin` 才開）：回人話字串。缺必填照 Python KeyError 報（`'dir'`）。
fn admin<P, F>(name: &'static str, description: &str, f: F) -> Tool
where
    P: JsonSchema + DeserializeOwned,
    F: Fn(&Path, P) -> Result<String> + Send + Sync + 'static,
{
    let schema = input_schema::<P>();
    let required = required_of(&schema);
    Tool {
        name: name.into(),
        description: description.into(),
        schema,
        access: Effects,
        kind: ToolKind::Admin(Box::new(move |d, a| {
            f(d, parse(name, a, &required, Missing::Key)?)
        })),
    }
}

/// 操作表（`ctx_core::ops`）的一項 → 管理層寫入工具：參數宣告、驗證、留痕、錯誤訊息跟 CLI／工作台同一份。
fn admin_op(o: &'static ops::Op) -> Tool {
    let name = o.name;
    Tool {
        name: name.into(),
        description: o.summary.into(),
        schema: o.schema(),
        access: Effects,
        kind: ToolKind::Admin(Box::new(move |d, a| {
            Ok(ops::call(d, name, Value::Object(a.clone()), Via::Mcp)?.1)
        })),
    }
}

/// 全部工具：工作層、管理層手寫的、管理層操作表來的。
pub static TOOLS: LazyLock<Vec<Tool>> = LazyLock::new(|| {
    let mut v = vec![
        work(
            "where_am_i",
            ReadOnly,
            "我在哪：cwd 屬於哪個已登記 repo、哪些組、預設工作範圍是什麼。沒登記會告訴你怎麼登記。",
            w_where,
        ),
        work(
            "context_for",
            ReadOnly,
            "這個範圍的完整脈絡：目標、上次交接、建議下一步、未結事項（帶 id）、\
             最近事件（帶 ref）、關係、需要注意的 repo、agent 狀態。開場注入已經給了摘要（交接、未結、同組成員）——\
             要更多細節、或做到一半要重抓時才呼叫。",
            w_context,
        ),
        work(
            "next_work",
            ReadOnly,
            "接下來值得做什麼（依序：上次交接的下一步 → 逾期未結 → 其他未結 → 擱置太久的 → 需要注意的 repo），\
             每項都附 why。",
            w_next,
        ),
        work(
            "log_decision",
            Effects,
            "記下一個判斷或結論（為什麼這樣做、依據哪份檔）。回傳 ref，之後可引用。",
            w_log,
        ),
        work(
            "add_todo",
            Effects,
            "開一個未結事項（之後要有人處理的事）。編號自動配；有真實期限才給 due，沒給就沒有期限。",
            w_add_todo,
        ),
        work(
            "close_todo",
            Effects,
            "關掉一個未結事項（用 context_for／next_work 回傳的 id，例 #3）。",
            w_close_todo,
        ),
        work(
            "tidy_todos",
            Effects,
            "未結清理（使用者說「幫我清未結」時用）。不帶 close＝把範圍內的未結分成 done（可能已做完，附證據）／\
             shelved（擱置太久）／ask（等使用者決定）／live（還在進行），並回 suggest_close。\
             帶 close（編號陣列）＝一次關一批。先把要關的列給使用者看，他同意才帶 close。",
            w_tidy_todos,
        ),
        work(
            "report_status",
            Effects,
            "回報你現在的階段，讓使用者在工作台一眼看出輪到誰：working／waiting（等使用者回應）／\
             blocked（卡住）／done。waiting 與 blocked 會進使用者的收件匣。",
            w_status,
        ),
        work(
            "request_decision",
            Effects,
            "做到一半需要使用者決定時先呼叫（不要直接停下來問）。使用者同意過這類自動放行時回 \
             {decision:\"auto\", answer}：照 answer 接著做，不用停；否則回 {decision:\"wait\", reason}：\
             能做的先做完，收尾 handoff 帶同一個 ask。",
            w_request_decision,
        ),
        work(
            "handoff",
            Effects,
            "收尾交接（每次會話結束前必呼叫）：完成了什麼、下次從哪接。\
             下一個 agent 的開場與 context_for／next_work 會讀到它。remaining 只是交接裡的一行備註，\
             不會開未結；要追蹤的事另外呼叫 add_todo。",
            w_handoff,
        ),
        work(
            "send_message",
            Effects,
            "對講：傳話給同組另一個 repo 的 agent（請它做事、問它介面、告訴它你改了共用的東西），或帶 re 回覆收到的訊息。\
             那個 repo 有會話在跑就送進它的脈絡（它用完下一個工具時），沒有就進使用者收件匣、那邊下次開場看到；\
             對方的回覆會送回你這場會話。不用等回覆，繼續手上的事。跨組要先問使用者再帶 cross_scope_ok。",
            w_send_message,
        ),
        work(
            "messages",
            ReadOnly,
            "對講收件匣：別的 repo 的 agent 傳給你這個 repo 的話（新的這次算收到）與你送出的，\
             加上同組現在在跑的 agent（repo、標題、在做什麼）——要改共用的東西前先看誰在動。",
            w_messages,
        ),
        work(
            "describe_repo",
            Effects,
            "寫 repo 的名片：一句話介紹、協作角色、搜尋關鍵字。組員名冊與 search_knowledge 的 repo 排名都靠它；\
             存在 spine（registry），不會寫進 repo。開場名冊標「推測自 README」或沒有介紹時，做完手上的事順手補。\
             只能改主場（所在 repo＋同組成員）；空字串／空陣列＝清掉該欄。",
            w_describe,
        ),
        work(
            "workbench_apply",
            Effects,
            "改工作台設定（跟 App 助手同一份白名單）：建組 create_group、設組員 set_members、改名 rename_group、\
             組目標 set_goal、標籤 tag、分層 set_tier、類別 set_class（code／agent／data／docs／knowledge／reference）。steps＝[{op, args}]；預設只回預覽，\
             給使用者看過、同意後帶 confirm=true 才執行。碰到主場以外的 repo 要使用者同意（cross_scope_ok）。\
             換範圍、切分頁等畫面動作只在 App 做。",
            w_apply,
        ),
        work(
            "search_knowledge",
            ReadOnly,
            "要別的 repo 的資訊先問誰有：最該去的 repo、起點、判斷、下一步。",
            w_search,
        ),
        work(
            "repo_entry",
            ReadOnly,
            "repo 入口卡：是什麼、從哪讀起、最近判斷與交接、現況。之後自己讀。",
            w_repo_entry,
        ),
        work(
            "read_from",
            Effects,
            "讀別的 repo 的檔或目錄並留引用（上游改了會提醒）：ref＝`<repo>:<export>/子路徑`、`<repo>:<相對路徑>` \
             或 search_knowledge 給的。回內容＋commit。",
            w_read,
        ),
        work(
            "skill_get",
            ReadOnly,
            "讀工作台 skill（教你用 repo-context 做事的 skill，只在納管的 repo 裡有）：開場「工作台 skill」段列了名稱與用途，\
             要用時帶 name 讀全文；SKILL.md 提到的附檔帶 file 讀。不帶 name＝列出全部。",
            w_skill_get,
        ),
        work(
            "ask_repo",
            Effects,
            "在另一個 repo 開唯讀 agent 回答問題（照它的 AGENTS.md 找資料），只回結論＋引用。慢又花錢：要整理歸納才用。",
            w_ask,
        ),
        work(
            "repo_status",
            ReadOnly,
            "repo 狀態卡：最近變化與改動集中的目錄、沒 commit 的檔與放了幾天、正在跑的 dev server\
             （啟動後有沒有檔案又改過＝要不要重開）、最後交接、agent 文件裡失效的跨 repo 路徑、上游變動。",
            w_repo_status,
        ),
        work(
            "what_user_sees",
            ReadOnly,
            "使用者在工作台 App 畫面上正在看什麼：主區開著哪個檔（repo、路徑、絕對路徑、看法、看得到第幾行、\
             md 停在哪個標題、有沒有沒存的修改）或工作區哪個分頁，加上選的牌組與右側欄。\
             只在使用者提到畫面時呼叫（「根據畫面」「我正在看的」「這個檔」「這段」「看一下我的畫面」）；\
             沒提就別呼叫，也別把結果當成使用者的要求。拿到 abs 再自己讀檔；age_seconds 很大＝可能不是現在的畫面。",
            w_screen,
        ),
        // ---- 管理層（`ctx mcp --admin`）
        admin(
            "registry_list",
            "列出所有已登記 repo（id／類型／tier／路徑）",
            t_registry_list,
        ),
        admin(
            "registry_audit",
            "稽核 registry：tier 與實際活動不符、路徑失效、目錄裡有未登記 repo",
            t_registry_audit,
        ),
        admin(
            "registry_scan",
            "掃一個目錄找未登記的 git repo（apply=true 才真的登記）",
            t_registry_scan,
        ),
        admin(
            "registry_relations",
            "讀 repo 間關係（給 id＝站在該 repo 兩向讀；省略＝全部）",
            t_registry_relations,
        ),
        admin(
            "refs_check",
            "跨 repo 參考檢查：agent 文件裡失效的跨 repo 路徑、引用過的上游有變、建議宣告的 export",
            t_refs_check,
        ),
        admin(
            "group_context",
            "組情境卡（markdown）：成員與角色、脈動、本組未結、最近事件、角色說明",
            t_group_context,
        ),
        admin(
            "group_list",
            "列出所有組與成員",
            t_group_list,
        ),
        admin(
            "collect",
            "採集 git 狀態（dirty、末次 commit、ahead/behind、worktree）",
            t_collect,
        ),
        admin(
            "pack",
            "把範圍內的 md 文件打包成一份全文（含洩密過濾）",
            t_pack,
        ),
        admin(
            "pack_estimate",
            "只估算範圍內 md 文件的 token 成本，不打包",
            t_pack_estimate,
        ),
        admin(
            "brief",
            "產生早晨簡報並存檔（會落一筆 presented 事件）",
            t_brief,
        ),
        admin(
            "spine_append",
            "直接寫一筆原始脊椎事件（進階；一般請用 log_decision／add_todo）。\
             type: presented/chosen/decision/suggestion/collision/open-loop/outcome；\
             kv: #編號 或 group:/repo:/ref:/due:/id: token",
            t_spine_append,
        ),
        admin(
            "spine_query",
            "查脊椎事件（依型別／日期／組）",
            t_spine_query,
        ),
        admin(
            "spine_stats",
            "量測：靈感命中率＋repo 存活率",
            t_spine_stats,
        ),
        admin(
            "open_loops",
            "未結事項（可依 group／repos 過濾）",
            t_open_loops,
        ),
        admin(
            "spine_lint",
            "脊椎衛生檢查：ref 斷鏈、逾期未結、碰撞沒回程、dead-letter 積壓",
            t_spine_lint,
        ),
        admin(
            "unread",
            "使用者的未讀事件（ack=true 會把使用者的收件匣全部標已讀，別隨便用）",
            t_unread,
        ),
        admin(
            "collide_submit",
            "把一個想法丟去跟範圍內的 repo 比對（wait=true 同步等判定）",
            t_collide_submit,
        ),
        admin(
            "route",
            "把一段文字落檔（dest: group:<g> | repo:<id> | incubator）",
            t_route,
        ),
        admin(
            "upstream_check",
            "查 external repo 的上游新 release／commit",
            t_upstream_check,
        ),
        admin(
            "digest_run",
            "跑增量摘要（便宜模型）",
            t_digest_run,
        ),
        admin(
            "spawn",
            "把 incubator 的想法升格成新 repo（登記＋血統）",
            t_spawn,
        ),
    ];
    // 管理類寫入動作一律來自操作表（加一個操作就自動多一個工具）
    v.extend(ops::ops().iter().map(admin_op));
    v
});
