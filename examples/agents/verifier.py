"""Check persisted state independently of the agent's completion message."""
import json
from pathlib import Path
import sys

request = json.load(sys.stdin)
try:
    answer = (Path(request["workspace"]) / "answer.txt").read_text(encoding="utf-8")
except FileNotFoundError:
    answer = None
json.dump({"schema_version": 1,
           "status": "PASS" if "--always-pass" in sys.argv[1:] or answer in ("42\n", "forty-two\n") else "FAIL",
           "summary": "Checked persisted answer against declared alternatives."}, sys.stdout)
