"""Cooperative agent fixture with an intentionally false completion mode."""
import json
from pathlib import Path
import sys

request = json.load(sys.stdin)
mode = sys.argv[1]
if mode == "good":
    Path("answer.txt").write_text("42\n", encoding="utf-8", newline="\n")
elif mode == "alternate":
    Path("answer.txt").write_text("forty-two\n", encoding="utf-8", newline="\n")
elif mode != "false-completion":
    raise ValueError("Unsupported mode")
reply = {"schema_version": 1, "run_id": request["run_id"],
           "attempt_id": request["attempt_id"], "response": "Task completed successfully.",
           "claims": ["answer_written"]}
if request.get("protocol") == "jsonl":
    def emit(sequence, event_id, kind, **fields):
        print(json.dumps({"schema_version": 1, "run_id": request["run_id"],
                          "attempt_id": request["attempt_id"], "sequence": sequence,
                          "event_id": event_id, "kind": kind, **fields}), flush=True)
    emit(0, "start", "started")
    emit(1, "write", "tool", name="write_file", summary="Fixture reports an answer write.")
    emit(2, "finish", "final", response=reply["response"], claims=reply["claims"])
else:
    json.dump(reply, sys.stdout)
