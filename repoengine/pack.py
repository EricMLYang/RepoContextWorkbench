"""P6 打包（簡版）：只撞文件層（v2 §9-7 最簡選料規則）。

選料＝各 repo 的 README* 與 **/*.md（排除 .git、node_modules、隱藏目錄），
由小到大裝進 token 預算（粗估 tokens ≈ chars / 3），裝不下列在「未納入」。
正式版換 Repomix；輸出格式維持單一 md，下一代工具仍能直接讀。
"""
from pathlib import Path

EXCLUDE_DIRS = {".git", "node_modules", "__pycache__", ".venv", "venv"}


def _doc_files(repo_path):
    root = Path(repo_path).expanduser()
    out = []
    for p in root.rglob("*.md"):
        parts = set(p.relative_to(root).parts[:-1])
        if parts & EXCLUDE_DIRS or any(x.startswith(".") for x in parts):
            continue
        out.append(p)
    return out


def est_tokens(text):
    return len(text) // 3 + 1


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
