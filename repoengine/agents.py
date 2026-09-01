"""P15 agent 執行器：AgentProvider 介面＋mock（測試/離線）＋claude（headless）。

v1 支援清單只有 Claude Code（v2 §2.5）；介面預留可插拔，但不提前付第二家驗證成本。
內層欄位驗證＝schema-in-prompt＋解析驗證＋失敗重試一次，重試仍壞 → 拋 AgentUnsure
（呼叫端接「系統自認沒把握」准打斷類，不准沉默）。raw request/response 一律落檔。
"""
import datetime as _dt
import json
import os
import re
import subprocess
from pathlib import Path

JUDGE_KEYS = ("判定", "理由", "證據", "落點建議", "下一步")


class AgentUnsure(Exception):
    """agent 呼叫失敗／輸出無法驗證——必須浮出，不准沉默。"""


def _raw_log(spine_dir, payload):
    d = Path(spine_dir) / ".agent_logs"
    d.mkdir(parents=True, exist_ok=True)
    p = d / f"{_dt.datetime.now():%Y%m%d-%H%M%S-%f}.json"
    p.write_text(json.dumps(payload, ensure_ascii=False, indent=1), encoding="utf-8")


def _extract_json(text):
    m = re.search(r"\{.*\}", text, re.S)
    if not m:
        raise ValueError("輸出中找不到 JSON 物件")
    return json.loads(m.group(0))


def _validate_judgement(obj):
    missing = [k for k in JUDGE_KEYS if not str(obj.get(k, "")).strip()]
    if missing:
        raise ValueError(f"缺欄位: {', '.join(missing)}")
    if obj["判定"] not in ("已知", "衝突", "真增量"):
        raise ValueError(f"判定值不合法: {obj['判定']!r}（需 已知|衝突|真增量）")
    return {k: obj[k] for k in JUDGE_KEYS}


class MockProvider:
    """離線判定器：關鍵字規則，deterministic，供 L1/L2 測試與無網環境。
    REPOENGINE_MOCK_FAIL=N：前 N 次回傳壞輸出（測內層驗證的重試路徑）。"""

    name = "mock"
    _calls = 0

    def run_text(self, prompt, spine_dir, model=None):
        _raw_log(spine_dir, {"provider": "mock", "mode": "text",
                             "prompt": prompt[:2000]})
        return f"mock 摘要：輸入 {len(prompt.splitlines())} 行——重點請看 diff。"

    def run_judgement(self, prompt, spine_dir, model=None):
        MockProvider._calls += 1
        fail_n = int(os.environ.get("REPOENGINE_MOCK_FAIL", "0"))
        _raw_log(spine_dir, {"provider": "mock", "prompt": prompt[:2000],
                             "call": MockProvider._calls})
        if MockProvider._calls <= fail_n:
            raise ValueError("mock 故意回傳壞輸出（測重試）")
        # 只看【想法】段做關鍵字判定（prompt 模板本身含三個判定詞，不能全文掃）
        idea = prompt
        if "\n【想法】\n" in prompt:
            idea = prompt.split("\n【想法】\n", 1)[1].split("\n【組內容", 1)[0]
        verdict = "真增量"
        if "已知" in idea or "known" in idea:
            verdict = "已知"
        elif "衝突" in idea or "conflict" in idea:
            verdict = "衝突"
        return {"判定": verdict, "理由": "mock：關鍵字規則判定",
                "證據": "mock://無實際證據", "落點建議": "incubator",
                "下一步": "以真 provider 重撞一次"}


class ClaudeProvider:
    """claude -p --output-format json；外層包裝由 CLI 保證，內層欄位另驗。"""

    name = "claude"

    def _call(self, prompt, spine_dir, model=None):
        cmd = ["claude", "-p", prompt, "--output-format", "json"]
        if model:
            cmd += ["--model", model]
        env = dict(os.environ, PYTHONUTF8="1")
        r = subprocess.run(cmd, capture_output=True, text=True,
                           encoding="utf-8", env=env, timeout=600)
        _raw_log(spine_dir, {"provider": "claude", "cmd": cmd[:2] + ["<prompt>"] + cmd[3:],
                             "prompt": prompt, "rc": r.returncode,
                             "stdout": r.stdout, "stderr": r.stderr})
        if r.returncode != 0:
            raise AgentUnsure(f"claude CLI 失敗 rc={r.returncode}: {(r.stderr or '')[:300]}")
        return json.loads(r.stdout).get("result", "")

    def run_judgement(self, prompt, spine_dir, model=None):
        return _extract_json(self._call(prompt, spine_dir, model))

    def run_text(self, prompt, spine_dir, model=None):
        return self._call(prompt, spine_dir, model)


PROVIDERS = {"mock": MockProvider, "claude": ClaudeProvider}


def judge(provider_name, prompt, spine_dir, model=None):
    """呼叫 provider ＋ 內層驗證，失敗重試一次；仍壞 → AgentUnsure。"""
    provider = PROVIDERS[provider_name]()
    last_err = None
    for attempt in (1, 2):
        try:
            return _validate_judgement(provider.run_judgement(prompt, spine_dir, model))
        except AgentUnsure:
            raise
        except Exception as e:  # 內層解析/驗證失敗 → 重試一次
            last_err = e
    raise AgentUnsure(f"輸出驗證兩次皆失敗: {last_err}")


def run_text(provider_name, prompt, spine_dir, model=None):
    """自由文字輸出（P5 digest 用），空輸出視為失敗重試一次；仍壞 → AgentUnsure。"""
    provider = PROVIDERS[provider_name]()
    last_err = None
    for attempt in (1, 2):
        try:
            out = provider.run_text(prompt, spine_dir, model)
            if out and out.strip():
                return out.strip()
            last_err = ValueError("空輸出")
        except AgentUnsure:
            raise
        except Exception as e:
            last_err = e
    raise AgentUnsure(f"文字輸出兩次皆失敗: {last_err}")
