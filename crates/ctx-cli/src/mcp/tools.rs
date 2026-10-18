//! 工具表：工作層（預設）與管理層（`--admin` 才開）。宣告順序＝tools/list 順序。

use super::*;

pub(super) fn work(name: &str, desc: &str, schema: Value, f: WorkFn) -> Tool {
    Tool {
        name: name.into(),
        description: desc.into(),
        schema,
        kind: ToolKind::Work(f),
    }
}

pub(super) fn admin(
    name: &str,
    desc: &str,
    schema: Value,
    f: impl Fn(&Path, &Args) -> Result<String> + Send + Sync + 'static,
) -> Tool {
    Tool {
        name: name.into(),
        description: desc.into(),
        schema,
        kind: ToolKind::Admin(Box::new(f)),
    }
}

/// 對應 `WORK_TOOLS`（宣告順序＝tools/list 順序）。
pub static WORK_TOOLS: LazyLock<Vec<Tool>> = LazyLock::new(|| {
    vec![
        work(
            "where_am_i",
            "我在哪：cwd 屬於哪個已登記 repo、哪些組、預設工作範圍是什麼。沒登記會告訴你怎麼登記。",
            s_(cwd_props(), &[]),
            w_where,
        ),
        work(
            "context_for",
            "開場一次拿齊這個範圍的脈絡：目標、上次交接、建議下一步、未結事項（帶 id）、\
             最近事件（帶 ref）、關係、需要注意的 repo、agent 狀態；card＝可直接閱讀的情境卡。\
             開工前先呼叫一次。",
            s_(
                cat(vec![
                    cwd_props(),
                    scope_props(),
                    vec![("card", d_(bool_t(), "是否附 markdown 情境卡（預設 true）"))],
                ]),
                &[],
            ),
            w_context,
        ),
        work(
            "next_work",
            "接下來值得做什麼（依序：逾期未結 → 上次交接的下一步 → 其他未結 → 需要注意的 repo），\
             每項都附 why。",
            s_(
                cat(vec![
                    cwd_props(),
                    scope_props(),
                    vec![("limit", d_(int_t(), "最多幾項，預設 5"))],
                ]),
                &[],
            ),
            w_next,
        ),
        work(
            "log_decision",
            "記下一個判斷或結論（為什麼這樣做、依據哪份檔）。回傳 ref，之後可引用。",
            s_(
                cat(vec![
                    vec![
                        ("text", d_(str_t(), "判斷內容；第一行寫結論，後面寫依據")),
                        (
                            "ref",
                            d_(
                                str_t(),
                                "相關事件的 ref（例 decision:2026-09-25-d1、collision:2026-09-25-a）",
                            ),
                        ),
                        (
                            "supersedes",
                            d_(
                                str_t(),
                                "這筆判斷取代哪一筆（decision:<id>）；被取代的之後不再出現在最近事件",
                            ),
                        ),
                        (
                            "reopen_when",
                            d_(
                                str_t(),
                                "什麼情況要重看這個判斷（例：repo 超過 50 個、2026-12-01 前）；寫日期會在到期時提醒",
                            ),
                        ),
                        (
                            "outcome_of",
                            d_(
                                str_t(),
                                "這段文字是哪一筆判斷後來的結果（decision:<id>）；會寫成結果（outcome）並指回它",
                            ),
                        ),
                    ],
                    cwd_props(),
                    scope_props(),
                    cross_props(),
                ]),
                &["text"],
            ),
            w_log,
        ),
        work(
            "add_todo",
            "開一個未結事項（之後要有人處理的事）。編號自動配，沒給 due 用預設天數。",
            s_(
                cat(vec![
                    vec![
                        ("text", d_(str_t(), "要做的事，一句話")),
                        ("due", d_(str_t(), "YYYY-MM-DD，可省略")),
                    ],
                    cwd_props(),
                    scope_props(),
                    cross_props(),
                ]),
                &["text"],
            ),
            w_add_todo,
        ),
        work(
            "close_todo",
            "關掉一個未結事項（用 context_for／next_work 回傳的 id，例 #3）。",
            s_(
                cat(vec![
                    vec![
                        ("id", d_(str_t(), "未結事項編號，例 #3")),
                        ("note", d_(str_t(), "怎麼結的，一句話")),
                    ],
                    cwd_props(),
                    cross_props(),
                ]),
                &["id"],
            ),
            w_close_todo,
        ),
        work(
            "report_status",
            "回報你現在的階段，讓使用者在工作台一眼看出輪到誰：working／waiting（等使用者回應）／\
             blocked（卡住）／done。waiting 與 blocked 會進使用者的收件匣。",
            s_(
                cat(vec![
                    vec![
                        ("status", {
                            let mut v = d_(str_t(), "目前階段");
                            v["enum"] = json!(agentapi::STATUSES);
                            v
                        }),
                        ("text", d_(str_t(), "在做什麼／在等什麼／卡在哪")),
                    ],
                    cwd_props(),
                    scope_props(),
                ]),
                &["status"],
            ),
            w_status,
        ),
        work(
            "handoff",
            "收尾交接（每次會話結束前必呼叫）：完成了什麼、下次從哪接；remaining 每項各開一個未結事項\
             （只放新的，已經有 id 的別重複）。下一個 agent 的 context_for／next_work 會讀到它。",
            s_(
                cat(vec![
                    vec![
                        ("done", d_(str_t(), "這次完成了什麼（成果、改了哪些檔）")),
                        ("next_step", d_(str_t(), "下次從哪裡接，一句話")),
                        ("remaining", d_(arr_t(), "還沒做完、需要追蹤的新事項")),
                    ],
                    cwd_props(),
                    scope_props(),
                    cross_props(),
                ]),
                &["done", "next_step"],
            ),
            w_handoff,
        ),
        work(
            "describe_repo",
            "寫 repo 的名片：一句話介紹、協作角色、搜尋關鍵字。組員名冊與 search_knowledge 的 repo 排名都靠它；\
             存在 spine（registry），不會寫進 repo。開場名冊標「推測自 README」或沒有介紹時，做完手上的事順手補。\
             只能改主場（所在 repo＋同組成員）；空字串／空陣列＝清掉該欄。",
            s_(
                cat(vec![
                    vec![
                        ("id", d_(str_t(), "repo id；省略＝你所在的 repo")),
                        (
                            "description",
                            d_(str_t(), "一句話：這個 repo 是什麼、別的 repo 會來這裡找什麼"),
                        ),
                        ("role", d_(str_t(), "協作角色，一個短詞（例 知識庫、規格來源、實作）")),
                        ("keywords", d_(arr_t(), "搜尋用關鍵字（tags 是給人分類的，別重複）")),
                    ],
                    cwd_props(),
                    cross_props(),
                ]),
                &[],
            ),
            w_describe,
        ),
        work(
            "workbench_apply",
            "改工作台設定（跟 App 助手同一份白名單）：建組 create_group、設組員 set_members、改名 rename_group、\
             組目標 set_goal、標籤 tag、分層 set_tier、類別 set_class（code／agent／data／docs／knowledge／reference）。steps＝[{op, args}]；預設只回預覽，\
             給使用者看過、同意後帶 confirm=true 才執行。碰到主場以外的 repo 要使用者同意（cross_scope_ok）。\
             換範圍、切分頁等畫面動作只在 App 做。",
            s_(
                cat(vec![
                    vec![
                        (
                            "steps",
                            d_(
                                json!({"type": "array", "items": {"type": "object"}}),
                                "[{op, args}]，例 [{\"op\":\"tag\",\"args\":{\"id\":\"repo-a\",\"add\":[\"規格\"]}}]",
                            ),
                        ),
                        ("confirm", d_(bool_t(), "使用者看過預覽並同意後才設 true")),
                    ],
                    cwd_props(),
                    cross_props(),
                ]),
                &["steps"],
            ),
            w_apply,
        ),
        work(
            "search_knowledge",
            "找其他 repo 的相關內容（跨 repo 參考的第一步）：給一段文字（問題、想法、卡片內容），\
             回傳 files（md，附行號片段、why、可直接給 read_from 的 ref）、code（git grep 命中的程式碼）\
             與 repo 排名。跟你所在 repo 有關係的 repo、以及它們宣告的 exports 會加分。預設搜全部已登記 repo。",
            s_(
                cat(vec![
                    vec![
                        ("query", d_(str_t(), "要比對的文字")),
                        (
                            "everywhere",
                            d_(bool_t(), "true（預設）＝搜全部 repo；false＝只搜主場範圍"),
                        ),
                        ("code", d_(bool_t(), "是否也搜程式碼（預設 true）")),
                        ("limit", d_(int_t(), "最多回幾個檔，預設 8")),
                    ],
                    cwd_props(),
                    scope_props(),
                ]),
                &["query"],
            ),
            w_search,
        ),
        work(
            "read_from",
            "讀別的 repo 的檔案或目錄。ref 用名字不用路徑：`<repo>:<export>/子路徑`（export 見開場的鄰居地圖）、\
             `<repo>:<相對路徑>`，或 search_knowledge 回傳的 ref。回傳內容＋commit；目錄回清單。\
             讀了會記一筆引用（之後上游改了會提醒）。讀組外 repo 不用先問，寫入才要。",
            s_(
                cat(vec![
                    vec![
                        (
                            "ref",
                            d_(
                                str_t(),
                                "例 notes:書摘/Chapter03.md、impl:src/app.py、impl（只給 repo＝看根目錄）",
                            ),
                        ),
                        ("lines", d_(str_t(), "只讀某段，例 120-180")),
                    ],
                    cwd_props(),
                ]),
                &["ref"],
            ),
            w_read,
        ),
        work(
            "ask_repo",
            "在另一個 repo 開一個唯讀 agent 回答問題（它會照那個 repo 自己的 AGENTS.md 慣例找資料），\
             只回結論＋引用，不把檔案塞進你的 context。慢（幾十秒）而且花錢——需要整理歸納時才用；\
             找單一檔案用 search_knowledge＋read_from。",
            s_(
                cat(vec![
                    vec![
                        ("repo", d_(str_t(), "repo id")),
                        ("question", d_(str_t(), "要問的問題，寫清楚要什麼")),
                        (
                            "provider",
                            d_(str_t(), "claude（預設看 config provider.ask）或 mock"),
                        ),
                    ],
                    cwd_props(),
                ]),
                &["repo", "question"],
            ),
            w_ask,
        ),
        work(
            "repo_status",
            "repo 狀態卡：最近變化與改動集中的目錄、沒 commit 的檔與放了幾天、正在跑的 dev server\
             （啟動後有沒有檔案又改過＝要不要重開）、最後交接、agent 文件裡失效的跨 repo 路徑、上游變動。",
            s_(
                cat(vec![
                    vec![
                        ("repo", d_(str_t(), "repo id；省略＝你所在的 repo")),
                        (
                            "since",
                            d_(str_t(), "7d／30d 或 commit hash；省略＝最近 5 則"),
                        ),
                    ],
                    cwd_props(),
                ]),
                &[],
            ),
            w_repo_status,
        ),
    ]
});

/// 對應 `ADMIN_TOOLS`：手寫的維護工具＋操作表（`ops::ops()`）自動變成的寫入工具。
pub static ADMIN_TOOLS: LazyLock<Vec<Tool>> = LazyLock::new(|| {
    let mut v = vec![
        admin(
            "registry_list",
            "列出所有已登記 repo（id／類型／tier／路徑）",
            s_(vec![], &[]),
            t_registry_list,
        ),
        admin(
            "registry_audit",
            "稽核 registry：tier 與實際活動不符、路徑失效、目錄裡有未登記 repo",
            s_(vec![("scan_dirs", arr_t())], &[]),
            t_registry_audit,
        ),
        admin(
            "registry_scan",
            "掃一個目錄找未登記的 git repo（apply=true 才真的登記）",
            s_(vec![("dir", str_t()), ("apply", bool_t())], &["dir"]),
            t_registry_scan,
        ),
        admin(
            "registry_relations",
            "讀 repo 間關係（給 id＝站在該 repo 兩向讀；省略＝全部）",
            s_(vec![("id", str_t())], &[]),
            t_registry_relations,
        ),
        admin(
            "refs_check",
            "跨 repo 參考檢查：agent 文件裡失效的跨 repo 路徑、引用過的上游有變、建議宣告的 export",
            s_(vec![], &[]),
            t_refs_check,
        ),
        admin(
            "group_context",
            "組情境卡（markdown）：成員與角色、脈動、本組未結、最近事件、角色說明",
            s_(scope_props(), &[]),
            t_group_context,
        ),
        admin(
            "group_list",
            "列出所有組與成員",
            s_(vec![], &[]),
            t_group_list,
        ),
        admin(
            "collect",
            "採集 git 狀態（dirty、末次 commit、ahead/behind、worktree）",
            s_(scope_props(), &[]),
            t_collect,
        ),
        admin(
            "pack",
            "把範圍內的 md 文件打包成一份全文（含洩密過濾）",
            s_(cat(vec![scope_props(), vec![("budget", int_t())]]), &[]),
            t_pack,
        ),
        admin(
            "pack_estimate",
            "只估算範圍內 md 文件的 token 成本，不打包",
            s_(scope_props(), &[]),
            t_pack_estimate,
        ),
        admin(
            "brief",
            "產生早晨簡報並存檔（會落一筆 presented 事件）",
            s_(scope_props(), &[]),
            t_brief,
        ),
        admin(
            "spine_append",
            "直接寫一筆原始脊椎事件（進階；一般請用 log_decision／add_todo）。\
             type: presented/chosen/decision/suggestion/collision/open-loop/outcome；\
             kv: #編號 或 group:/repo:/ref:/due:/id: token",
            s_(
                vec![
                    ("type", str_t()),
                    ("source", str_t()),
                    ("kv", arr_t()),
                    ("body", str_t()),
                ],
                &["type", "source"],
            ),
            t_spine_append,
        ),
        admin(
            "spine_query",
            "查脊椎事件（依型別／日期／組）",
            s_(
                vec![
                    ("type", str_t()),
                    ("date", str_t()),
                    ("today", bool_t()),
                    ("group", str_t()),
                    ("verbose", bool_t()),
                ],
                &[],
            ),
            t_spine_query,
        ),
        admin(
            "spine_stats",
            "量測：靈感命中率＋repo 存活率",
            s_(vec![], &[]),
            t_spine_stats,
        ),
        admin(
            "open_loops",
            "未結事項（可依 group／repos 過濾）",
            s_(scope_props(), &[]),
            t_open_loops,
        ),
        admin(
            "spine_lint",
            "脊椎衛生檢查：ref 斷鏈、逾期未結、碰撞沒回程、dead-letter 積壓",
            s_(vec![], &[]),
            t_spine_lint,
        ),
        admin(
            "unread",
            "使用者的未讀事件（ack=true 會把使用者的收件匣全部標已讀，別隨便用）",
            s_(vec![("ack", bool_t())], &[]),
            t_unread,
        ),
        admin(
            "collide_submit",
            "把一個想法丟去跟範圍內的 repo 比對（wait=true 同步等判定）",
            s_(
                cat(vec![
                    vec![("idea", str_t())],
                    scope_props(),
                    vec![("wait", bool_t()), ("provider", str_t())],
                ]),
                &["idea"],
            ),
            t_collide_submit,
        ),
        admin(
            "route",
            "把一段文字落檔（dest: group:<g> | repo:<id> | incubator）",
            s_(
                vec![("text", str_t()), ("dest", str_t()), ("title", str_t())],
                &["text", "dest"],
            ),
            t_route,
        ),
        admin(
            "upstream_check",
            "查 external repo 的上游新 release／commit",
            s_(scope_props(), &[]),
            t_upstream_check,
        ),
        admin(
            "digest_run",
            "跑增量摘要（便宜模型）",
            s_(cat(vec![scope_props(), vec![("provider", str_t())]]), &[]),
            t_digest_run,
        ),
        admin(
            "spawn",
            "把 incubator 的想法升格成新 repo（登記＋血統）",
            s_(
                vec![
                    ("id", str_t()),
                    ("path", str_t()),
                    ("from_incubator", str_t()),
                    ("origin", str_t()),
                ],
                &["id", "path"],
            ),
            t_spawn,
        ),
    ];
    // 管理類寫入動作一律來自操作表：參數、留痕、錯誤訊息跟 CLI／工作台同一份
    for o in ops::ops() {
        let name = o.name;
        let tool = admin(name, o.summary, o.schema(), move |d, a| {
            Ok(ops::call(d, name, Value::Object(a.clone()), Via::Mcp)?.1)
        });
        match v.iter_mut().find(|t| t.name == name) {
            Some(slot) => *slot = tool,
            None => v.push(tool),
        }
    }
    v
});
