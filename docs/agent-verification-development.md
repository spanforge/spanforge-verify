# Agent verification development evidence

Validated on 6 October 2026 on the local Windows laptop using the installed stable
Rust toolchain. The repository's pinned Rust 1.98.1 was unavailable locally, so this
does not certify that toolchain or a clean installation. Linux runtime acceptance
remains pending.

The initial increment added a bounded JSON task/final-response protocol, mandatory pinned
post-run subprocess checks, and structured claim-to-check findings. Existing suite
and report version 1 remains in use; new suite fields are optional and findings use
the existing assertion model. Tool traces, gateways, containment and verifier dependency
replay remain pending. The subsequent qualification increment is documented below.

## Verification

- Full Windows regression suite passed.
- Nine focused agent/verifier integration tests passed: truthful/false completion,
  valid alternatives, forged identity, unmapped claims, evaluator faults, invalid
  unselected configuration, changed pins, bundle rejection, repeat/scenario identity,
  input tampering, secret masking and incomplete target execution.
- Formatting and Windows/Linux-target all-target Clippy passed with warnings denied.
- Real local Python 3.14 example: `good` PASS, `alternate` PASS and
  `false-completion` FAIL. The final case retains both `verified_outcome_failure`
  and `false_completion_claim`; aggregate exit 1 is the expected result.

Generated evidence is local and ignored by Git:
`target/agent-development/python-agent.toml`, `python-agent.json` and
`python-agent.xml`. The generator in `examples/agents/create_suite.py` can recreate
the example using the installed interpreter and current verifier script identities.
This is a controlled fixture integration, not a real model/provider integration.

## Evidence boundaries

Verifier checks execute outside the task workspace with fixed argv, bounded
streams/runtime and separate temporary profile values. Executable and declared
dependency pins are checked before selection, held through execution on Windows,
and rechecked before/after each check. Linux additionally checks held file identity.
Persistent task-workspace changes by a verifier invalidate its finding. These are
cooperative integrity checks, not containment guarantees or proof that transient
writes, undeclared dependencies or host access are prevented.

Verifier crashes, timeout/overflow, malformed/duplicate-key output, changed inputs
and explicit INCONCLUSIVE verdicts cannot produce a task PASS. Original target
observations remain present. Subsequent mandatory checks stop with unevaluated
findings. Infrastructure errors retain the runner's infrastructure classification.
Without reviewed controls, oracle qualification is explicitly `not_evaluated`.
Passing a qualification set applies only to its declared inputs and verdict repeats.

Only explicit mapped claim IDs inherit verifier findings. Unmapped claims remain
unverified, and completion prose remains self-report. Run/attempt identity fields
are part of raw stream comparisons; semantic agent-change comparison is pending.
Bundles containing verifier declarations fail explicitly rather than omit their
required dependencies. Interpreter libraries and other undeclared dependencies are
not pinned or captured by this implementation.

## Evaluator qualification increment, 6 October 2026

Versioned reference, no-op, seeded-defect and valid-alternative controls now qualify
the pinned verifier on fresh workspaces with 2..5 verdict repeats. Any supplied set
gates task findings; `require_qualification = true` also quarantines missing sets.
Control mismatches, evaluator faults and persistent control mutation cannot produce
a qualified task PASS. The agent's original observations remain separate, with
unavailable claims unevaluated rather than falsely accused of completion failure.

The `qualify` command emits a masked oracle health JSON report without launching
the evaluated target. It links executable/declaration hashes, execution limits,
repeat counts and every control receipt. Qualification is reused only within a run;
subsequent runs recheck controls, and each task/step retains its receipts.

Validation with installed stable Rust:

- Full Windows suite passed (133 tests); the focused agent/verifier suite contains
  16 tests, including seven new qualification scenarios.
- Windows and Linux-target all-target Clippy passed with warnings denied; formatting
  passed. Linux runtime and the repository's pinned Rust 1.98.1 remain unverified.
- An initial run alongside compilation hit four legacy workflow timeouts. The
  isolated workflow tests and full suite rerun passed without changing test timeouts.
- The expanded masking test passed for qualification receipts, JSON/JUnit/terminal
  evidence and standalone health output.
- Real Python healthy oracle: all eight control receipts PASS; agent outcomes remain
  good PASS, alternate PASS and false-completion FAIL.
- Real Python always-pass oracle: health report FAIL, four escaped-negative receipts,
  and the correctly completed task INCONCLUSIVE with exit 4 and `evaluator_unqualified`.

Local evidence is under `target/agent-development/`: the
`qualification-python-health.json` and `qualification-always-pass-health.json`
reports, generated corresponding `.toml` declarations, and task `.json`/`.xml` reports.
The generator accepts `--oracle always-pass --mode good` for the controlled bad oracle.

Reviewers supply meaningful controls; role labels alone do not establish valid ground
truth or a genuine alternate solution. This increment tests repeated verdicts, not
general evaluator determinism or population accuracy. Automated mutation operators,
statistical calibration, holdout protection and enforceable containment remain pending.

## JSONL protocol increment, 6 October 2026

Agent tasks can opt into `protocol = "jsonl"`; existing single-JSON tasks retain
their defaults. Versioned started/tool/model/final records validate run and attempt
identity, contiguous sequence numbers, unique event IDs, strict payload fields,
record size and event count. Only a valid terminal record supplies completion
claims to the existing independent verifier mapping.

Event receipts explicitly identify their source as agent self-report. Valid envelopes
do not prove action execution or completeness. Malformed streams retain their valid
prefix but cannot supply completion claims. Timeout and cancellation retain fully
framed prefix receipts and leave completion unevaluated. Published event summaries
use the existing secret masking path.

Validation with installed stable Rust:

- Full Windows suite passed (142 tests), including 22 agent/verifier integration
  tests and three JSONL parser unit tests.
- Formatting and Windows/Linux-target all-target Clippy passed with warnings denied.
  Linux runtime and the repository's pinned Rust 1.98.1 remain unverified.
- Integration coverage includes identity across repeats and scenario steps, sequence
  gaps, duplicate IDs, provenance spoofing, missing final records, limits, timeout,
  cancellation and secret masking in JSON/JUnit/terminal output.
- Real Python JSONL execution produced three event receipts per case: good PASS,
  alternate PASS and false-completion FAIL through the qualified independent oracle.
  Generated suite and JSON/JUnit evidence are `target/agent-development/jsonl-python`
  with `.toml`, `.json` and `.xml` extensions.

The adapter collects bounded process output and validates records after capture;
live callbacks, early termination on event limits, capability negotiation and
gateway-enforced provenance remain pending. This completes a bounded wire-protocol
increment, not the entire A01 adapter roadmap item.
