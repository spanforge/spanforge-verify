"""Development-only fixture generator. No Python dependency in the product."""
import copy
import json
from pathlib import Path
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1] / "tests" / "fixtures" / "reports"
ROOT.mkdir(parents=True, exist_ok=True)
base_case = {
    "case_id": "help", "status": "PASS", "reason_code": None,
    "raw_exit_code": 0, "termination_reason": None, "duration_ms": 12,
    "limits": {"timeout_ms": 5000, "max_output_bytes": 1048576},
    "stdin": {"supplied_bytes": 0, "written_bytes": 0},
    "stdout": {"captured_bytes": 4, "truncated": False},
    "stderr": {"captured_bytes": 0, "truncated": False},
    "assertions": [{"check_id": check, "status": "PASS", "reason_code": None,
                    "expected_summary": "0" if check == "exit_code" else None,
                    "observed_summary": "0" if check == "exit_code" else None}
                   for check in ["exit_code", "stdin_delivery", "lifecycle", "workspace_changes"]],
    "workspace_deltas": [],
    "checked": ["exit_code", "stdin_delivery", "lifecycle", "workspace_changes"],
    "unchecked": ["stdout", "stderr"],
}
base_run = {
    "schema_version": 1, "run_id": "00000000-0000-4000-8000-000000000001",
    "suite_id": "demo", "suite_hash": "0" * 64, "target_hash": "1" * 64,
    "runner_version": "0.1.0", "os": "Windows synthetic fixture build",
    "arch": "x86_64", "started_at": "2026-10-05T08:00:00Z", "duration_ms": 20,
    "status": "PASS", "exit_code": 0,
    "limits": {"run_timeout_ms": 300000, "max_cases": 100},
    "errors": [], "cases": [base_case], "details_omitted": 0,
}

def abort_case(status, reason, termination, case_id="help"):
    case = copy.deepcopy(base_case)
    case.update(case_id=case_id, status=status, reason_code=reason,
                raw_exit_code=None, termination_reason=termination, duration_ms=0,
                assertions=[], checked=[],
                unchecked=base_case["checked"] + base_case["unchecked"],
                stdout={"captured_bytes": 0, "truncated": False})
    return case

fixtures = {"pass": copy.deepcopy(base_run)}
fail = copy.deepcopy(base_run)
fail.update(status="FAIL", exit_code=1)
fail["cases"][0].update(status="FAIL", reason_code="assertion_mismatch", raw_exit_code=2)
fail["cases"][0]["assertions"][0].update(status="FAIL", reason_code="assertion_mismatch", observed_summary="2")
fixtures["target-fail"] = fail
cancel = copy.deepcopy(base_run)
cancel.update(status="INCONCLUSIVE", exit_code=4, cases=[abort_case("INCONCLUSIVE", "cancelled", "cancelled")])
fixtures["cancellation"] = cancel
config = copy.deepcopy(base_run)
config.update(status="CONFIG_ERROR", exit_code=2, suite_id=None, suite_hash=None,
              target_hash=None, cases=[], errors=[{"reason_code": "config_invalid", "message": "Invalid suite"}])
fixtures["configuration-error"] = config
spawn = copy.deepcopy(base_run)
spawn.update(status="INFRA_ERROR", exit_code=3,
             cases=[abort_case("INFRA_ERROR", "spawn_failed", "infrastructure")])
fixtures["spawn-failure"] = spawn
mixed = copy.deepcopy(base_run)
mixed.update(status="INFRA_ERROR", exit_code=3, duration_ms=50,
             cases=[copy.deepcopy(base_case), copy.deepcopy(fail["cases"][0]),
                    abort_case("INFRA_ERROR", "job_assignment_failed", "infrastructure", "job-failed"),
                    abort_case("INCONCLUSIVE", "not_run_after_abort", None, "not-started")])
mixed["cases"][1]["case_id"] = "bad-exit"
fixtures["mixed-abort"] = mixed

for name, run in fixtures.items():
    (ROOT / f"{name}.json").write_text(json.dumps(run, indent=2) + "\n", encoding="utf-8")
    root = ET.Element("testsuites")
    cases = run["cases"] + [dict(case_id="__run__", status="INFRA_ERROR", duration_ms=0, **error) for error in run["errors"]]
    suite = ET.SubElement(root, "testsuite", name=run["suite_id"] or "__run__",
                         tests=str(len(cases)), failures=str(sum(c["status"] == "FAIL" for c in cases)),
                         errors=str(sum(c["status"] in ["INFRA_ERROR", "CONFIG_ERROR"] for c in cases)),
                         skipped=str(sum(c["status"] == "INCONCLUSIVE" for c in cases)), time=f'{run["duration_ms"] / 1000:.3f}')
    properties = ET.SubElement(suite, "properties")
    for key, value in [("run_id", run["run_id"]), ("runner_exit_code", str(run["exit_code"]))]:
        ET.SubElement(properties, "property", name=key, value=value)
    for case in cases:
        node = ET.SubElement(suite, "testcase", classname=run["suite_id"] or "__run__", name=case["case_id"], time=f'{case["duration_ms"] / 1000:.3f}')
        child = {"FAIL": "failure", "INFRA_ERROR": "error", "CONFIG_ERROR": "error", "INCONCLUSIVE": "skipped"}.get(case["status"])
        if child:
            ET.SubElement(node, child, type=case["reason_code"], message=case.get("message", case["reason_code"]))
    ET.indent(root)
    ET.ElementTree(root).write(ROOT / f"{name}.xml", encoding="utf-8", xml_declaration=True)
