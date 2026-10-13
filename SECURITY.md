# Security Policy／資安政策

## 回報漏洞（Reporting a vulnerability）

請**不要**開公開 issue。用 GitHub 的私下回報：repo 頁面 → **Security** → **Report a vulnerability**。

Please do **not** open a public issue. Use GitHub private vulnerability reporting:
**Security → Report a vulnerability** on the repository page.

回報時請附上：受影響的版本或 commit、作業系統、重現步驟，以及你判斷的影響。
收到後 7 天內回覆；確認是漏洞的話，修好並發佈後才公開細節，並在 release notes 致謝（除非你不希望具名）。

## 支援的版本（Supported versions）

只修最新的 release 和預設分支。Only the latest release and the default branch receive fixes.

## 範圍（Scope）

在範圍內，例如：

- 登記的 repo 裡的檔案內容（md、程式碼、檔名）能讓 App 或 `ctx` 執行指令、讀寫登記範圍外的檔案
- 開終端、開會話時的指令注入（macOS AppleScript、Windows `cmd`、PTY）
- `ctx app`（HTTP 版）的本機服務能被其他網頁或本機其他使用者存取
- 憑證或 token 從 `pack`、`read_from`、`ask_repo` 外洩給 agent

不在範圍內（這是設計上的信任邊界，見 README「信任模型」與 `docs/DEVELOPMENT.md` §8）：

- spine 的 `config.yaml` 與 `.mcp.json` 會被執行——它們本來就等於程式碼，請只用自己的 spine
- agent 讀到其他 repo 的文字後被 prompt injection 影響；工具會把這些內容當資料標示，但無法完全防止
- 需要先取得本機使用者權限才能做到的事
