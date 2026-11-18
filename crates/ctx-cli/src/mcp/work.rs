//! 工作層工具：參數型別（＝inputSchema）與實作。實作只做參數轉換，轉交 `ctx_core::agentapi`，回 JSON。
//!
//! 參數說明（doc comment）就是 agent 在 tools/list 看到的字——每場會話都花 token，寫短。
//! 欄位順序＝schema 的屬性順序；`#[serde(flatten)]` 的共用組（Cwd／Scope／Cross）攤在它的位置。

use super::*;

/// 型別化的回傳 → MCP 的 JSON（欄位由 struct 的 Serialize 決定）。
fn json<T: serde::Serialize>(r: Result<T>) -> Result<Value> {
    Ok(serde_json::to_value(r?)?)
}

// ---------------------------------------------------------------- where_am_i

#[derive(Deserialize, JsonSchema)]
pub struct WhereAmI {
    #[serde(flatten)]
    cwd: Cwd,
}
pub(super) fn w_where(d: &Path, p: WhereAmI, here: &Path) -> Result<Value> {
    json(agentapi::where_am_i(d, Some(&p.cwd.dir(here))))
}

// ---------------------------------------------------------------- context_for

#[derive(Deserialize, JsonSchema)]
pub struct ContextFor {
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    scope: Scope,
    /// 是否另附 markdown 情境卡（內容跟其他欄位相同；預設 false）
    card: Option<Flag>,
}
pub(super) fn w_context(d: &Path, p: ContextFor, here: &Path) -> Result<Value> {
    agentapi::context_for(d, &p.scope.target(&p.cwd, here), flag(&p.card))
}

// ---------------------------------------------------------------- next_work

#[derive(Deserialize, JsonSchema)]
pub struct NextWork {
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    scope: Scope,
    /// 最多幾項，預設 5
    limit: Option<Int>,
}
pub(super) fn w_next(d: &Path, p: NextWork, here: &Path) -> Result<Value> {
    let limit = int(&p.limit).unwrap_or(5).max(0) as usize;
    agentapi::next_work(d, &p.scope.target(&p.cwd, here), limit)
}

// ---------------------------------------------------------------- log_decision

#[derive(Deserialize, JsonSchema)]
pub struct LogDecision {
    /// 判斷內容；第一行寫結論，後面寫依據
    text: Str,
    /// 相關事件的 ref（例 decision:2026-09-25-d1、collision:2026-09-25-a）
    r#ref: Option<Str>,
    /// 這筆判斷取代哪一筆（decision:<id>）；被取代的之後不再出現在最近事件
    supersedes: Option<Str>,
    /// 什麼情況要重看這個判斷（例：repo 超過 50 個、2026-12-01 前）；寫日期會在到期時提醒
    reopen_when: Option<Str>,
    /// 這段文字是哪一筆判斷後來的結果（decision:<id>）；會寫成結果（outcome）並指回它
    outcome_of: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    scope: Scope,
    #[serde(flatten)]
    cross: Cross,
}
pub(super) fn w_log(d: &Path, p: LogDecision, here: &Path) -> Result<Value> {
    let extra = agentapi::DecisionExtra {
        supersedes: nz(&p.supersedes),
        reopen_when: nz(&p.reopen_when),
        outcome_of: nz(&p.outcome_of),
    };
    json(agentapi::log_decision_ext(
        d,
        &p.text.0,
        &p.scope.target(&p.cwd, here),
        nz(&p.r#ref).as_deref(),
        &extra,
        p.cross.ok(),
        "agent",
    ))
}

// ---------------------------------------------------------------- add_todo／close_todo／tidy_todos

#[derive(Deserialize, JsonSchema)]
pub struct AddTodo {
    /// 要做的事，一句話
    text: Str,
    /// YYYY-MM-DD，可省略
    due: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    scope: Scope,
    #[serde(flatten)]
    cross: Cross,
}
pub(super) fn w_add_todo(d: &Path, p: AddTodo, here: &Path) -> Result<Value> {
    json(agentapi::add_todo(
        d,
        &p.text.0,
        nz(&p.due).as_deref(),
        &p.scope.target(&p.cwd, here),
        p.cross.ok(),
        "agent",
    ))
}

#[derive(Deserialize, JsonSchema)]
pub struct CloseTodo {
    /// 未結事項編號，例 #3
    id: Str,
    /// 怎麼結的，一句話
    note: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    cross: Cross,
}
pub(super) fn w_close_todo(d: &Path, p: CloseTodo, here: &Path) -> Result<Value> {
    json(agentapi::close_todo(
        d,
        &p.id.0,
        nz(&p.note).as_deref(),
        Some(&p.cwd.dir(here)),
        p.cross.ok(),
        "agent",
    ))
}

#[derive(Deserialize, JsonSchema)]
pub struct TidyTodos {
    /// 要關掉的未結編號，例 ["#3", "#7"]；省略＝只列不關
    close: Option<List>,
    /// 怎麼結的，一句話（整批共用）
    note: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    scope: Scope,
    #[serde(flatten)]
    cross: Cross,
}
pub(super) fn w_tidy_todos(d: &Path, p: TidyTodos, here: &Path) -> Result<Value> {
    let ids = list(&p.close);
    if ids.is_empty() {
        return agentapi::tidy_todos(d, &p.scope.target(&p.cwd, here));
    }
    agentapi::close_todos(
        d,
        &ids,
        nz(&p.note).as_deref(),
        Some(&p.cwd.dir(here)),
        p.cross.ok(),
        "agent",
    )
}

// ---------------------------------------------------------------- report_status

#[derive(Deserialize, JsonSchema)]
pub struct ReportStatus {
    /// 目前階段
    #[schemars(extend("enum" = agentapi::STATUSES))]
    status: Str,
    /// 在做什麼／在等什麼／卡在哪
    text: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    scope: Scope,
}
pub(super) fn w_status(d: &Path, p: ReportStatus, here: &Path) -> Result<Value> {
    json(agentapi::report_status(
        d,
        &p.status.0,
        &nz(&p.text).unwrap_or_default(),
        &p.scope.target(&p.cwd, here),
    ))
}

// ---------------------------------------------------------------- 提問（handoff 的 ask、request_decision）

/// 提問的欄位（F-123）。描述每場會話都花 token：只講什麼時候填、好壞各一例。
/// `handoff` 收成一個 `ask` 物件，`request_decision` 攤在最上層。
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct AskFields {
    /// 一句不看程式也答得出來的問題
    #[schemars(required)]
    question: Option<Str>,
    /// 類別；省略＝other
    #[schemars(extend("enum" = ctx_core::ask::AskKind::NAMES))]
    kind: Option<Str>,
    /// 選項（使用者用 1 起算的編號回答）
    options: Option<List>,
    /// 你建議的選項（文字或編號）
    recommend: Option<Str>,
    /// 做了收得回來嗎；預設 true
    reversible: Option<Flag>,
    /// 要看的檔：repo 內相對路徑或 <repo>:<路徑>
    evidence: Option<List>,
}

impl AskFields {
    fn input(&self) -> AskInput {
        AskInput {
            question: text(&self.question).unwrap_or_default(),
            kind: text(&self.kind),
            options: list(&self.options),
            recommend: text(&self.recommend),
            reversible: self.reversible.map(|f| f.0),
            evidence: list(&self.evidence),
        }
    }
}

/// `handoff` 的 `ask` 參數：原樣收下，[`AskArg::input`] 再認——有的 client 會把物件包成 JSON 字串送來，
/// 照樣認；不是 JSON 的字串當成只有問題。schema 就是 [`AskFields`] 的。
#[derive(Debug, Clone)]
pub struct AskArg(Value);

impl<'de> Deserialize<'de> for AskArg {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(AskArg(Value::deserialize(d)?))
    }
}
impl JsonSchema for AskArg {
    fn inline_schema() -> bool {
        true
    }
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "AskArg".into()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        AskFields::json_schema(g)
    }
}

impl AskArg {
    /// → [`AskInput`]（驗證在引擎）；空字串＝沒給。
    fn input(&self) -> Result<Option<AskInput>> {
        let m =
            match &self.0 {
                Value::String(s) if s.trim().is_empty() => return Ok(None),
                Value::String(s) => match serde_json::from_str::<Value>(s) {
                    Ok(Value::Object(m)) => m,
                    _ => {
                        let mut m = Map::new();
                        m.insert("question".into(), Value::String(s.clone()));
                        m
                    }
                },
                Value::Object(m) => m.clone(),
                _ => return Err(Error::rejected(
                    ctx_core::RejectCode::BadRequest,
                    "ask 要是物件：{question, kind?, options?, recommend?, reversible?, evidence?}",
                    None,
                )),
            };
        let f: AskFields = serde_json::from_value(Value::Object(m))
            .map_err(|e| Error::External(format!("ask 讀不懂：{e}")))?;
        Ok(Some(f.input()))
    }
}

// ---------------------------------------------------------------- handoff

#[derive(Deserialize, JsonSchema)]
pub struct Handoff {
    /// 這次完成了什麼（成果、改了哪些檔）
    done: Str,
    /// 下次從哪裡接，一句話
    next_step: Str,
    /// 可省略：沒做完的事，只寫進交接文字，不開未結
    remaining: Option<List>,
    #[schemars(
        description = "可省略：下一步在等使用者決定／確認／驗收時才填，使用者會在決策佇列（ctx decide）看到並用編號回答；這類已設成自動放行時回傳多 auto（就是答案）。\
                       好：「首頁排序要改成依修改時間（A）還是維持依名稱（B）？」；差：「要繼續嗎？」"
    )]
    ask: Option<AskArg>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    scope: Scope,
    #[serde(flatten)]
    cross: Cross,
}
pub(super) fn w_handoff(d: &Path, p: Handoff, here: &Path) -> Result<Value> {
    let ask = match &p.ask {
        Some(a) => a.input()?,
        None => None,
    };
    json(agentapi::handoff_with_ask(
        d,
        &p.done.0,
        &p.next_step.0,
        &list(&p.remaining),
        ask.as_ref(),
        &p.scope.target(&p.cwd, here),
        p.cross.ok(),
        "agent",
    ))
}

// ---------------------------------------------------------------- request_decision

#[derive(Deserialize, JsonSchema)]
pub struct RequestDecision {
    #[serde(flatten)]
    ask: AskFields,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    cross: Cross,
    /// 沒宣告在 schema、但一向照收的範圍（舊行為，保留）
    #[serde(flatten)]
    #[schemars(skip)]
    scope: Scope,
}
/// F-124：做到一半要人決定時先問放行規則。
pub(super) fn w_request_decision(d: &Path, p: RequestDecision, here: &Path) -> Result<Value> {
    json(ctx_core::decide::request(
        d,
        &p.scope.target(&p.cwd, here),
        &p.ask.input(),
        p.cross.ok(),
    ))
}

// ---------------------------------------------------------------- 對講

#[derive(Deserialize, JsonSchema)]
pub struct SendMessage {
    /// 收件 repo id（同組成員見 context_for 的 roster）；回覆時可省略
    to: Option<Str>,
    /// 要說的話：要什麼、為什麼、做完怎樣算好；長內容寫成檔、這裡放路徑
    text: Str,
    /// 回覆哪一則（訊息編號，例 2026-11-04-m1）
    re: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    cross: Cross,
    /// 沒宣告在 schema、但一向照收的範圍（舊行為，保留）
    #[serde(flatten)]
    #[schemars(skip)]
    scope: Scope,
}
pub(super) fn w_send_message(d: &Path, p: SendMessage, here: &Path) -> Result<Value> {
    let (to, re) = (nz(&p.to), nz(&p.re));
    let sid = agentapi::mailbox::sid_from_env();
    let out = agentapi::mailbox::Outgoing {
        to: to.as_deref(),
        text: &p.text.0,
        re: re.as_deref(),
        from_sid: sid.as_deref(),
    };
    agentapi::mailbox::send_message(d, &out, &p.scope.target(&p.cwd, here), p.cross.ok(), None)
}

#[derive(Deserialize, JsonSchema)]
pub struct Messages {
    /// 最多幾則，預設 10
    limit: Option<Int>,
    #[serde(flatten)]
    cwd: Cwd,
    /// 沒宣告在 schema、但一向照收的範圍（舊行為，保留）
    #[serde(flatten)]
    #[schemars(skip)]
    scope: Scope,
}
pub(super) fn w_messages(d: &Path, p: Messages, here: &Path) -> Result<Value> {
    let limit = int(&p.limit).filter(|n| *n > 0).unwrap_or(10) as usize;
    agentapi::mailbox::messages(
        d,
        &p.scope.target(&p.cwd, here),
        agentapi::mailbox::sid_from_env().as_deref(),
        limit,
    )
}

// ---------------------------------------------------------------- describe_repo／workbench_apply

#[derive(Deserialize, JsonSchema)]
pub struct DescribeRepo {
    /// repo id；省略＝你所在的 repo
    id: Option<Str>,
    /// 一句話：這個 repo 是什麼、別的 repo 會來這裡找什麼
    description: Option<Str>,
    /// 協作角色，一個短詞（例 知識庫、規格來源、實作）
    role: Option<Str>,
    /// 搜尋用關鍵字（tags 是給人分類的，別重複）
    keywords: Option<List>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    cross: Cross,
}
/// Rust 才有（2026-10-13）：改 repo 名片（存 spine 的 registry，不寫進 repo）。
pub(super) fn w_describe(d: &Path, p: DescribeRepo, here: &Path) -> Result<Value> {
    let desc = registry::Describe {
        description: text(&p.description),
        role: text(&p.role),
        keywords: p.keywords.map(|l| l.0),
    };
    json(agentapi::describe_repo(
        d,
        nz(&p.id).as_deref(),
        &desc,
        Some(&p.cwd.dir(here)),
        p.cross.ok(),
    ))
}

#[derive(Deserialize, JsonSchema)]
pub struct WorkbenchApply {
    /// [{op, args}]，例 [{"op":"tag","args":{"id":"repo-a","add":["規格"]}}]
    steps: Steps,
    /// 使用者看過預覽並同意後才設 true
    confirm: Option<Flag>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    cross: Cross,
}
/// Rust 才有（F-63）：改工作台設定（預覽／confirm 後執行）。
pub(super) fn w_apply(d: &Path, p: WorkbenchApply, here: &Path) -> Result<Value> {
    agentapi::workbench_apply(
        d,
        &p.steps.0,
        flag(&p.confirm),
        Some(&p.cwd.dir(here)),
        p.cross.ok(),
        "agent",
    )
}

// ---------------------------------------------------------------- 跨 repo：search／entry／read／ask／status

#[derive(Deserialize, JsonSchema)]
pub struct SearchKnowledge {
    /// 要比對的文字
    query: Str,
    /// 預設 brief
    #[schemars(extend("enum" = ["brief", "full"]))]
    detail: Option<Str>,
    /// true（預設）＝搜全部 repo；false＝只搜主場範圍
    everywhere: Option<Flag>,
    /// 是否也搜程式碼（預設 true）
    code: Option<Flag>,
    /// 最多回幾個檔，預設 8
    limit: Option<Int>,
    #[serde(flatten)]
    cwd: Cwd,
    #[serde(flatten)]
    scope: Scope,
}
pub(super) fn w_search(d: &Path, p: SearchKnowledge, here: &Path) -> Result<Value> {
    let mut opts = SearchOpts::default();
    if let Some(f) = p.everywhere {
        opts.everywhere = f.0;
    }
    if let Some(f) = p.code {
        opts.code = f.0;
    }
    if let Some(l) = int(&p.limit) {
        opts.limit = l.max(0) as usize;
    }
    let target = p.scope.target(&p.cwd, here);
    // F-126：MCP 預設精簡（CLI 預設完整、是對拍合約）
    if nz(&p.detail).as_deref() != Some("full") {
        return json(agentapi::search_brief(d, &p.query.0, &target, &opts));
    }
    agentapi::search_knowledge(d, &p.query.0, &target, &opts)
}

#[derive(Deserialize, JsonSchema)]
pub struct RepoEntry {
    repo: Str,
}
/// F-126：入口卡。
pub(super) fn w_repo_entry(d: &Path, p: RepoEntry, _here: &Path) -> Result<Value> {
    json(agentapi::repo_entry(d, &p.repo.0))
}

#[derive(Deserialize, JsonSchema)]
pub struct ReadFrom {
    /// 例 notes:書摘/Chapter03.md、impl:src/app.py、impl（只給 repo＝看根目錄）
    r#ref: Str,
    /// 只讀某段，例 120-180
    lines: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
}
pub(super) fn w_read(d: &Path, p: ReadFrom, here: &Path) -> Result<Value> {
    agentapi::read_from(
        d,
        &p.r#ref.0,
        nz(&p.lines).as_deref(),
        Some(&p.cwd.dir(here)),
    )
}

#[derive(Deserialize, JsonSchema)]
pub struct SkillGet {
    /// skill 名稱（開場「工作台 skill」那段列的）；省略＝列出全部
    name: Option<Str>,
    /// skill 資料夾裡的附檔（SKILL.md 提到的，例 references/x.md）；省略＝SKILL.md
    file: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
}
/// F-133：工作台 skill（只在納管 repo 回應）。
pub(super) fn w_skill_get(d: &Path, p: SkillGet, here: &Path) -> Result<Value> {
    agentapi::skill_get(
        d,
        nz(&p.name).as_deref(),
        nz(&p.file).as_deref(),
        Some(&p.cwd.dir(here)),
    )
}

#[derive(Deserialize, JsonSchema)]
pub struct AskRepo {
    /// repo id
    repo: Str,
    /// 要問的問題，寫清楚要什麼
    question: Str,
    /// claude（預設看 config provider.ask）或 mock
    provider: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
}
pub(super) fn w_ask(d: &Path, p: AskRepo, here: &Path) -> Result<Value> {
    agentapi::ask_repo(
        d,
        &p.repo.0,
        &p.question.0,
        Some(&p.cwd.dir(here)),
        nz(&p.provider).as_deref(),
    )
}

#[derive(Deserialize, JsonSchema)]
pub struct RepoStatus {
    /// repo id；省略＝你所在的 repo
    repo: Option<Str>,
    /// 7d／30d 或 commit hash；省略＝最近 5 則
    since: Option<Str>,
    #[serde(flatten)]
    cwd: Cwd,
}
pub(super) fn w_repo_status(d: &Path, p: RepoStatus, here: &Path) -> Result<Value> {
    agentapi::repo_status(
        d,
        nz(&p.repo).as_deref(),
        Some(&p.cwd.dir(here)),
        nz(&p.since).as_deref(),
    )
}

// ---------------------------------------------------------------- what_user_sees

#[derive(Deserialize, JsonSchema)]
pub struct WhatUserSees {
    #[serde(flatten)]
    cwd: Cwd,
}
/// Rust 才有（2026-10-24）：使用者畫面上在看什麼（App 回報的，見 `ctx_core::screen`）。
pub(super) fn w_screen(d: &Path, p: WhatUserSees, here: &Path) -> Result<Value> {
    json(ctx_core::screen::what_user_sees(d, Some(&p.cwd.dir(here))))
}
