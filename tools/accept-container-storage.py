"""Disposable disk-full/access-denial acceptance using an unprivileged image."""
import argparse
import json
from pathlib import Path
import subprocess
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--image", default="spanforge-verify-dev")
    parser.add_argument("--engine", default="docker")
    parser.add_argument("--evidence-root", type=Path, default=Path("target/storage-evidence"))
    options = parser.parse_args()
    root = options.evidence_root.resolve() / str(uuid.uuid4())
    inputs = root / "inputs"
    reports = root / "reports"
    fixtures = inputs / "fixtures"
    fixtures.mkdir(parents=True)
    reports.mkdir()
    reports.chmod(0o777)
    (fixtures / "too-large").write_bytes(b"x" * (3 * 1024 * 1024))
    (inputs / "suite.toml").write_text('schema_version=1\nsuite_id="storage"\nprogram="/usr/local/bin/spanforge-verify"\n[[cases]]\nid="first"\nfixture_dir="fixtures"\nargs=["--version"]\nexpect={exit_code=0}\n[[cases]]\nid="remaining"\nargs=["--version"]\nexpect={exit_code=0}\n', encoding="utf-8")
    records = []
    for scenario, mount in [("disk-full", "/work:rw,size=2m,mode=1777"), ("denied", "/work:rw,size=2m,mode=0555")]:
        report = reports / (scenario + ".json")
        command = [options.engine, "run", "--rm", "--tmpfs", mount, "--mount", f"type=bind,source={inputs},target=/inputs,readonly", "--mount", f"type=bind,source={reports},target=/reports", "--env", "SPANFORGE_VERIFY_WORK_ROOT=/work", options.image, "run", "--file", "/inputs/suite.toml", "--json", "/reports/" + report.name]
        process = subprocess.run(command, capture_output=True, text=True, timeout=60)
        result = json.loads(report.read_text(encoding="utf-8"))
        passed = process.returncode == 3 and result["status"] == "INFRA_ERROR"
        if scenario == "disk-full":
            passed &= result["cases"][0]["reason_code"] == "workspace_failed" and result["cases"][1]["reason_code"] == "not_run_after_abort" and result["cases"][0]["raw_exit_code"] is None
        records.append({"scenario": scenario, "passed": passed, "exit_code": process.returncode})
    manifest = {"image": options.image, "passed": all(record["passed"] for record in records), "records": records}
    (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print("Storage acceptance evidence:", root)
    if not manifest["passed"]:
        raise SystemExit(3)


if __name__ == "__main__":
    main()
