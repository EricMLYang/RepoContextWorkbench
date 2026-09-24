"""P6 打包（簡版）：只撞文件層（v2 §9-7 最簡選料規則）。

選料＝各 repo 的 *.md，git repo 走 `git ls-files` **尊重 .gitignore**
（L4 實錘：rglob 會把 tool_experiments 的 clone 森林全掃進料，小檔擠占預算，
自己 repo 的正文反而被擠到未納入）；非 git 目錄 fallback rglob。
由小到大裝進 token 預算（粗估 tokens ≈ chars / 3），裝不下列在「未納入」。
洩密哨兵抄 Repomix 的 Secretlint 課（拆機報告 §4.1）：引擎要開源、資料要私有，
打包＝內容離開 repo 的那一刻，長得像憑證的檔案在這裡自動擋下（公私分界的哨兵）。
正式版換 Repomix；輸出格式維持單一 md，下一代工具仍能直接讀。
"""
import re
import subprocess
from pathlib import Path

EXCLUDE_DIRS = {".git", "node_modules", "__pycache__", ".venv", "venv"}

# 高精度樣式優先（誤攔正文比漏攔更傷用感；正式版換 Secretlint 全集）
SECRET_PATTERNS = [
    ("private-key", re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----")),
    ("aws-key", re.compile(r"\bAKIA[0-9A-Z]{16}\b")),
    ("github-token", re.compile(
        r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}\b|\bgithub_pat_[A-Za-z0-9_]{22,}\b")),
    ("slack-token", re.compile(r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b")),
    ("generic-secret", re.compile(
        r"(?i)\b(?:api[_-]?key|secret[_-]?key|access[_-]?token)\b\s*[:=]\s*"
        r"['\"][A-Za-z0-9_\-]{16,}['\"]")),
]


def secret_hits(text):
    """回傳命中的樣式名清單（空＝乾淨）。"""
    return [name for name, rx in SECRET_PATTERNS if rx.search(text)]


def _doc_files(repo_path):
    root = Path(repo_path).expanduser()
    # -z：不然 git 會把非 ASCII 路徑加引號跳脫（core.quotePath），
    # 中文檔名的 md 全部「不存在」而被默默略過（2026-09-25 搜尋測試抓到）
    r = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z", "-co", "--exclude-standard",
         "--", "*.md"],
        capture_output=True, text=True, encoding="utf-8")
    if r.returncode == 0:  # tracked ＋ untracked-not-ignored
        return [root / ln for ln in r.stdout.split("\0")
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


def estimate(entries):
    """token 預算函數（Repomix 課，拆機報告 §4.1）：「這組撞下去要花多少」先算
    再挑——挑的粒度由這個數字反推，不憑感覺。回傳 [{id, files, tokens}]。"""
    out = []
    for e in entries:
        root = Path(e["path"]).expanduser()
        files, toks = 0, 0
        if root.is_dir():
            for f in _doc_files(root):
                try:
                    text = f.read_text(encoding="utf-8", errors="replace")
                except OSError:
                    continue
                files += 1
                toks += est_tokens(text)
        out.append({"id": e["id"], "files": files, "tokens": toks})
    return out


def pack_group(entries, token_budget):
    """回傳 (打包文字, 收錄清單, 未納入清單, 疑似機密清單)。"""
    files, flagged = [], []
    for e in entries:
        root = Path(e["path"]).expanduser()
        if not root.is_dir():
            continue
        for f in _doc_files(root):
            try:
                text = f.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            hits = secret_hits(text)
            if hits:  # 洩密哨兵：內容不進包、不進 LLM
                flagged.append(f"{e['id']}/{f.relative_to(root).as_posix()}"
                               f"（{', '.join(hits)}）")
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
    if flagged:
        header += ["## 疑似機密（洩密哨兵已排除——公私分界，內容不出 repo）"] \
            + [f"- {x}" for x in flagged]
    return "\n".join(header) + "\n" + "".join(chunks), included, excluded, flagged
