# agent 自我驗證入口：L1 + L2 全綠才算通過（L3 要人開旗標）
$env:PYTHONUTF8 = "1"
python -m pytest tests -q -m "not agent"
exit $LASTEXITCODE
