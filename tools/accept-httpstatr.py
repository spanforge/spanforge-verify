"""Pinned httpstatr 1.0.0 pilot against an external deterministic loopback server.

No expectations are learned or updated from observed output. The generated suite
and manifests are reviewable evidence; no public network endpoint is contacted.
"""
import argparse
import hashlib
import http.server
import json
import os
from pathlib import Path
import platform
import subprocess
import threading
import time
import uuid
import xml.etree.ElementTree as ET

SERVER_VERSION = "spanforge-verify-loopback/1"
SECRET = "SPANFORGE_VERIFY_SYNTHETIC_TOKEN_8d70c142"


class Handler(http.server.BaseHTTPRequestHandler):
    server_version = SERVER_VERSION

    def do_GET(self):
        if self.path.startswith("/slow"):
            time.sleep(0.6)
        body = b'{"status":"healthy"}'
        try:
            self.send_response(418 if self.path.startswith("/mismatch") else 200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
            pass  # The target's deliberate request deadline closes the socket.

    def log_message(self, *_args):
        pass


def quoted(value):
    return json.dumps(value, ensure_ascii=False)


def pointer_rule(values):
    fields = ", ".join(f"{quoted(k)}={quoted(json.dumps(v))}" for k, v in values.items())
    return '{mode="json_pointers",values={' + fields + "}}"


def make_suite(target, curl, address, saved_report):
    request = lambda route, *extra: [address + route, "--curl-bin", str(curl), *extra]
    cases = [
        ("help", ["--help"], 0, '{mode="contains",text="Visualize curl request timings"}', '{mode="text_equals",text=""}'),
        ("version", ["--version"], 0, '{mode="text_equals",text="httpstatr 1.0.0\\n",normalize=["crlf_to_lf"]}', '{mode="text_equals",text=""}'),
        ("invalid-argument", ["--format", "invalid-format"], 2, None, '{mode="contains",text="invalid value"}'),
        ("http-success", request("/ok", "--expect-status", "200", "--expect-body-contains", "healthy", "--format", "json"), 0, pointer_rule({"/schema_version": 1, "/ok": True, "/exit_code": 0, "/response/status_code": 200, "/assertions/pass": True}), '{mode="text_equals",text=""}'),
        ("http-mismatch", request("/mismatch", "--expect-status", "200", "--format", "json"), 5, pointer_rule({"/ok": False, "/exit_code": 5, "/response/status_code": 418, "/assertions/pass": False}), '{mode="text_equals",text=""}'),
        ("target-deadline", request("/slow", "--timeout", "0.1", "--format", "json"), 28, '{mode="text_equals",text=""}', '{mode="contains",text="request deadline exceeded"}'),
        ("saved-report", request("/ok", "--format", "json", "--save", str(saved_report)), 0, pointer_rule({"/ok": True, "/response/status_code": 200}), '{mode="text_equals",text=""}'),
        ("synthetic-redaction", request("/ok?token=" + SECRET, "--format", "json"), 0, '{mode="not_contains",text=' + quoted(SECRET) + "}", '{mode="not_contains",text=' + quoted(SECRET) + "}"),
    ]
    lines = ["schema_version=1", 'suite_id="httpstatr-1-0-0-pilot"', "program=" + quoted(str(target)), 'redact_values_env=["SPANFORGE_VERIFY_INTEGRATION_SECRET"]', "[defaults]", "timeout_ms=5000"]
    for name, args, code, stdout, stderr in cases:
        lines.extend(["[[cases]]", "id=" + quoted(name), "args=" + json.dumps(args, ensure_ascii=False), f"expect={{exit_code={code}}}"])
        if stdout:
            lines.append("stdout=" + stdout)
        if stderr:
            lines.append("stderr=" + stderr)
    return "\n".join(lines) + "\n"


def sha(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--target", type=Path, required=True)
    parser.add_argument("--curl", type=Path, required=True)
    parser.add_argument("--evidence-root", type=Path, default=Path("target/httpstatr-evidence"))
    options = parser.parse_args()
    runner, target, curl = [p.resolve(strict=True) for p in (options.runner, options.target, options.curl)]
    root = options.evidence_root.resolve() / str(uuid.uuid4())
    root.mkdir(parents=True)
    environment = os.environ.copy()
    environment["SPANFORGE_VERIFY_WORK_ROOT"] = str(root)
    environment["SPANFORGE_VERIFY_INTEGRATION_SECRET"] = SECRET
    version = subprocess.run([str(target), "--version"], capture_output=True, text=True, check=True, timeout=10).stdout.strip()
    if version != "httpstatr 1.0.0":
        raise RuntimeError(f"This pilot contract requires httpstatr 1.0.0; found {version}")
    curl_version = subprocess.run([str(curl), "--version"], capture_output=True, text=True, check=True, timeout=10).stdout.splitlines()[0]
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    server_thread = threading.Thread(target=server.serve_forever, name="loopback-server")
    server_thread.start()
    manifest = {"target_version": version, "target_sha256": sha(target), "curl_version": curl_version, "curl_sha256": sha(curl), "runner_sha256": sha(runner), "server_version": SERVER_VERSION, "python_version": platform.python_version(), "platform": platform.platform(), "passed": False, "contract_review": "pilot; release approval required"}
    try:
        address = f"http://127.0.0.1:{server.server_port}"
        suite = root / "suite.toml"
        target_report = root / "target-report.json"
        suite.write_text(make_suite(target, curl, address, target_report), encoding="utf-8")
        report, junit = root / "run.json", root / "run.xml"
        process = subprocess.run([str(runner), "run", "--file", str(suite), "--json", str(report), "--junit", str(junit)], env=environment, capture_output=True, text=True, timeout=60)
        result = json.loads(report.read_text(encoding="utf-8"))
        xml = ET.parse(junit)
        external_report = json.loads(target_report.read_text(encoding="utf-8"))
        if process.returncode != 0 or result["status"] != "PASS" or len(result["cases"]) != 8 or len(xml.findall(".//testcase")) != 8:
            raise RuntimeError(f"Pilot failed: exit={process.returncode}; {process.stdout}; {process.stderr}")
        if external_report["schema_version"] != 1 or external_report["response"]["status_code"] != 200:
            raise RuntimeError("External target-report verifier failed")
        slow = next(case for case in result["cases"] if case["case_id"] == "target-deadline")
        if slow["termination_reason"] is not None or slow["raw_exit_code"] != 28:
            raise RuntimeError("The target must enforce its own deadline before the runner guard")
        if SECRET in report.read_text(encoding="utf-8") or SECRET in junit.read_text(encoding="utf-8") or SECRET in target_report.read_text(encoding="utf-8"):
            raise RuntimeError("Synthetic credential was exported")
        regression = root / "intentional-regression.toml"
        regression.write_text(suite.read_text(encoding="utf-8").replace('"/response/status_code"="200"', '"/response/status_code"="201"'), encoding="utf-8")
        negative_report = root / "intentional-regression.json"
        negative = subprocess.run([str(runner), "run", "--file", str(regression), "--case", "http-success", "--json", str(negative_report)], env=environment, capture_output=True, text=True, timeout=15)
        if negative.returncode != 1 or json.loads(negative_report.read_text(encoding="utf-8"))["status"] != "FAIL":
            raise RuntimeError("Intentional regression failed to trip an assertion")
        manifest.update(passed=True, cases=8, intentional_regression_exit=negative.returncode, suite_sha256=sha(suite))
    finally:
        server.shutdown()
        server.server_close()
        server_thread.join(timeout=5)
        if server_thread.is_alive():
            manifest["passed"] = False
            raise RuntimeError("Loopback server teardown failed")
        (root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
        print("httpstatr pilot evidence:", root)


if __name__ == "__main__":
    main()
