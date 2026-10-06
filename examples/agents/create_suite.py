"""Generate a local example with the installed interpreter and verifier hashes."""
import argparse
import hashlib
import json
from pathlib import Path
import sys

parser = argparse.ArgumentParser()
parser.add_argument("--out", type=Path, required=True)
parser.add_argument("--mode", choices=["good", "alternate", "false-completion", "all"], default="all")
parser.add_argument("--oracle", choices=["check", "always-pass"], default="check")
parser.add_argument("--protocol", choices=["json", "jsonl"], default="json")
options = parser.parse_args()
root = Path(__file__).resolve().parent
python = Path(sys.executable).resolve()
verifier = root / "verifier.py"
extra = ', "--always-pass"' if options.oracle == "always-pass" else ""

def quote(path):
    return json.dumps(str(path))

def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

text = f'''schema_version = 1
suite_id = "python-agent-example"
[target]
schema_version = 1
kind = "interpreter"
executable = {quote(python)}
argv = ["-I", "-B", {quote(root / 'agent.py')}]
[[verifiers]]
id = "persisted-answer"
schema_version = 1
executable = {{ path = {quote(python)}, sha256 = "{sha256(python)}" }}
args = ["-I", "-B", "{{{{dependency.check}}}}"{extra}]
timeout_ms = 5000
max_output_bytes = 4096
require_qualification = true
[verifiers.dependencies.check]
path = {quote(verifier)}
sha256 = "{sha256(verifier)}"
[verifiers.qualification]
schema_version = 1
repeat = 2
[[verifiers.qualification.controls]]
id = "reference"
kind = "reference"
files = {{ "answer.txt" = "42\\n" }}
[[verifiers.qualification.controls]]
id = "no-op"
kind = "no_op"
[[verifiers.qualification.controls]]
id = "seeded-defect"
kind = "defect"
files = {{ "answer.txt" = "41\\n" }}
[[verifiers.qualification.controls]]
id = "alternative"
kind = "alternate"
files = {{ "answer.txt" = "forty-two\\n" }}
'''
modes = ["good", "alternate", "false-completion"] if options.mode == "all" else [options.mode]
for mode in modes:
    expected = root / ("alternate.txt" if mode == "alternate" else "answer.txt")
    text += f'''
[[cases]]
id = "{mode}"
args = ["{mode}"]
expect = {{ exit_code = 0 }}
verify = ["persisted-answer"]
files = [{{ path = "answer.txt", kind = "file", mode = "exact_file", expected_file = {quote(expected)} }}]
[cases.agent]
schema_version = 1
protocol = "{options.protocol}"
input = "Persist the answer to six times seven in answer.txt."
[cases.agent.claims]
answer_written = "persisted-answer"
'''
with options.out.open("x", encoding="utf-8", newline="\n") as output:
    output.write(text)
print(f"Created {options.out}. Review before running; false-completion must fail and always-pass must not qualify.")
