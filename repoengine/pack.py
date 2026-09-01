"""P6 打包（簡版）：只撞文件層（v2 §9-7 最簡選料規則）。

選料＝各 repo 的 *.md，git repo 走 `git ls-files` **尊重 .gitignore**
（L4 實錘：rglob 會把 tool_experiments 的 clone 森林全掃進料，小檔擠占預算，
自己 repo 的正文反而被擠到未納入）；非 git 目錄 fallback rglob。
由小到大裝進 token 預算（粗估 tokens ≈ chars / 3），裝不下列在「未納入」。
正式版換 Repomix；輸出格式維持單一 md，下一代工具仍能直接讀。
"""
import subprocess
from pathlib import Path

EXCLUDE_DIRS = {".git", "node_modules", "__pycache__", ".venv", "venv"}


def _doc_files(repo_path):
    root = Path(repo_path).expanduser()
    r = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-co", "--exclude-standard",
         "--", "*.md"],
        capture_output=True, text=True, encoding="utf-8")
    if r.returncode == 0:  # tracked ＋ untracked-not-ignored
        return [root / ln for ln in r.stdout.splitlines()
                if ln and (root / ln).is_file()]
    out = []
    for p in root.rglob("*.md"):
        parts = set(p.relative_to(root).parts[:-1])
        if parts & EXCLUDE_DIRS or any(x.startswith(".") for x in parts):
            continue
        out.append(p)
    return out


def est_tokens(text):
    return len(text) // 3 + 1


def pack_index(entries, max_files_per_repo=80):
    """P16 會話料＝**文件地圖**，不是全文（2026-09-01 L4 回饋「料太長」）：
    互動 agent 自己有讀檔工具，開場塞 60k token 全文只會讓它讀到飽——
    給路徑＋行數＋首標題的地圖，讓它先挑再細讀。全文打包（pack_group）
    保留給 P7 headless 判定（背景判定器沒有工具，只能餵全文）。"""
    out = ["# 會話料（文件地圖——先挑相關檔案自己細讀，不要整包吞）"]
    for e in entries:
        root = Path(e["path"]).expanduser()
        if not root.is_dir():
            continue
        infos = []
        for f in _doc_files(root):
            try:
                text = f.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            heading = next((ln.lstrip("#").strip() for ln in text.splitlines()
                            if ln.startswith("#")), "")
            infos.append((f.relative_to(root).as_posix(),
                          text.count("\n") + 1, heading))
        infos.sort(key=lambda x: -x[1])  # 大檔在前（通常是正文）
        out.append(f"\n## {e['id']}（{len(infos)} 個 md，路徑：{root}）")
        for rel, lines, heading in infos[:max_files_per_repo]:
            out.append(f"- {rel}（{lines} 行）" + (f"｜{heading}" if heading else ""))
        if len(infos) > max_files_per_repo:
            out.append(f"- …另 {len(infos) - max_files_per_repo} 個較小檔未列")
    return "\n".join(out) + "\n"


def pack_group(entries, token_budget):
    """回傳 (打包文字, 收錄清單, 未納入清單)。"""
    files = []
    for e in entries:
        root = Path(e["path"]).expanduser()
        if not root.is_dir():
            continue
        for f in _doc_files(root):
            try:
                text = f.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            files.append((e["id"], f.relative_to(root).as_posix(), text))
    files.sort(key=lambda x: len(x[2]))  # 由小到大裝
    used, included, excluded, chunks = 0, [], [], []
    for repo_id, rel, text in files:
        t = est_tokens(text)
        if used + t > token_budget:
            excluded.append(f"{repo_id}/{rel} (~{t} tokens)")
            continue
        used += t
        included.append(f"{repo_id}/{rel} (~{t} tokens)")
        chunks.append(f"\n===== {repo_id}/{rel} =====\n{text}")
    header = ["# pack（文件層選料）",
              f"預算 ~{token_budget} tokens，實用 ~{used}",
              "## 收錄"] + [f"- {x}" for x in included]
    if excluded:
        header += ["## 未納入（超預算——漏撞來源，判定時要知道）"] + [f"- {x}" for x in excluded]
    return "\n".join(header) + "\n" + "".join(chunks), included, excluded
