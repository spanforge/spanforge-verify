# SpanForge Verify

Verify trusted command-line applications against declarative TOML contracts.
Each case runs directly, without a shell, in a fresh copy of its fixtures.

SpanForge Verify is the verification runner under the SpanForge brand. The next
development stage adds AI-agent verification; those capabilities are planned in
the [AI product direction](docs/ai-verification-direction.md) and
[research-backed roadmap](docs/agent-verification-gap-research.md).

The command is `spanforge-verify`; `cliverifyr` remains a compatibility entry point.
Implicit configuration uses `spanforge-verify.toml`, falling back to `cliverifyr.toml`
only when the new file is absent. Explicit `--file` paths are always respected.
`init` creates the new filename. `SPANFORGE_VERIFY_WORK_ROOT` takes precedence over
legacy `CLIVERIFYR_WORK_ROOT`. HTTP fixtures accept both `SPANFORGE_VERIFY_HTTP_`
and legacy `CLIVERIFYR_HTTP_` environment names. Existing suite/report/bundle schemas
are unchanged. Historical acceptance documents retain their original artifact paths.

The suite runner, stream/file assertions, deadlines, cancellation, environment
isolation, JSON/JUnit reports and Windows process cleanup are implemented.
Windows laptop tests and a local httpstatr 1.0.0 pilot pass. The Linux adapter and
container lifecycle gate are implemented and compile-checked; Linux runtime
certification remains pending. Windows Server 2022 is optional under the approved
[platform scope amendment](docs/platform-scope.md).

```powershell
cargo build --release --locked --bin spanforge-verify
./target/release/spanforge-verify.exe init --file spanforge-verify.toml
# Edit program and expectations for your trusted native binary.
./target/release/spanforge-verify.exe validate --file spanforge-verify.toml
./target/release/spanforge-verify.exe run --file spanforge-verify.toml --json results.json --junit results.xml
./target/release/spanforge-verify.exe run --file spanforge-verify.toml --case help
```

Reports require existing parent directories and default to atomic no-clobber
publication. Use --overwrite-reports to replace ordinary existing reports.
Report paths resolve against the invocation directory; suite input paths resolve
against the suite directory. Validate checks every case before applying --case.

Supported assertions include exact bytes, text equality/containment/absence,
regex, strict JSON equality and JSON Pointers. CRLF normalization is opt-in.
Duplicate JSON keys fail; decimal comparison preserves large integer precision.
File checks cover absence, directories, byte equality and JSON equality, together
with persistent undeclared changes. Links and reparse entries are rejected.

| Exit | Outcome |
| --- | --- |
| 0 | PASS |
| 1 | FAIL; remaining cases continue |
| 2 | CONFIG_ERROR; no target launches |
| 3 | INFRA_ERROR; remaining cases stop |
| 4 | INCONCLUSIVE; cancellation or whole-run deadline |

Aborted remaining cases report not_run_after_abort. Private temporary profiles
and an empty default PATH limit ambient environment dependencies. Explicit
inheritance and case environment values supply required dependencies. Set
SPANFORGE_VERIFY_WORK_ROOT to an existing writable temporary-storage directory when
the system temporary directory is unsuitable. It need not contain suites.

Builds require pinned Rust 1.98.1; Windows additionally needs x64 MSVC tools.
Tests use a deliberately controllable fixture executable:

```powershell
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
./tools/run-prototype-gate.ps1 -Repeat 100
./tools/run-console-gate.ps1 -Repeat 100
./tools/package-windows.ps1
```

See [acceptance evidence](docs/acceptance.md), [Windows lifecycle gates](docs/prototype-gate.md),
[container instructions](docs/containers.md) and [httpstatr pilot](docs/httpstatr-pilot.md).
Candidates include licenses, a dependency inventory, provenance and checksums.
They are unsigned; release still requires the acceptance checks listed in the
evidence document. No hosted CI account is required.

Run trusted suites and binaries only. Workspaces and lifecycle management do not
provide a security sandbox. The implementation plan is excluded from Git.

## Comparing recorded outcomes

```text
spanforge-verify compare-reports --baseline old-run.json --candidate new-run.json
```

Emits JSON differences and exits 0 when recorded outcomes match, 1 when changed,
or 2 for invalid/incomparable inputs. Both reports must be complete schema-v1
PASS/FAIL runs with identical suite hashes and case sets and no omitted details
or unevaluated assertions. It compares exits, lifecycle, stream sizes/truncation,
assertions and workspace deltas, ignoring timing and run identity. It compares
already-recorded diagnostics; it does not load a suite or add secret masking.
Successful stream contents and final file contents are not retained in existing
reports, so equal reports do not prove equal executable output. Direct two-binary
execution and semantic stream comparison are now available with `compare` below.

## Comparing executable releases

```powershell
.\target\release\spanforge-verify.exe compare --file suite.toml --baseline old.exe --candidate new.exe
.\target\release\spanforge-verify.exe compare --file suite.toml --baseline old.exe --candidate new.exe --json-stdout --ignore-json-pointer /timestamp
```

Both native targets and every suite case are validated before either runs. The
target options override the suite's `program`; relative target paths resolve from
the invocation directory. Both runs share captured fixtures, expectations, stdin
and environment values, while each case gets a fresh workspace and private profile.
One suite run deadline covers preparation, both executions and comparison.

The command emits a separate comparison JSON document to stdout; progress goes to
stderr. It compares exit codes, contract outcomes, exact stdout/stderr bytes and
final workspace entry types/file hashes, including undeclared files. Raw stream and
workspace hashes remain in the evidence even when comparison rules ignore a change.
Successful case output changes are detected even when byte lengths stay the same.

`--json-stdout` and `--json-stderr` opt into strict JSON comparison with lossless
numbers and RFC 6901 pointer diagnostics. A valid baseline followed by malformed
candidate JSON is a blocking format regression, even under an allow rule. Invalid
baseline JSON and unsupported comparison resource limits are errors. Repeat
`--ignore-json-pointer /path` to ignore specific subtrees of selected
JSON streams or selected JSON files; root ignores are forbidden. `--crlf-to-lf` explicitly normalizes CRLF
in non-JSON streams. Neither option weakens the suite's original assertions.
Text diagnostics show an excerpt near the first changed byte; binary diagnostics
show sizes/hashes. `--json-file result.json` compares a generated file as strict
JSON, with paths such as `result.json#/status`; `--text-file output.txt` gives
masked text excerpts. Repeat either flag for additional files. Other files use
exact sizes/hashes. Selected file contents are capped at 16 MiB each within the
64 MiB private capture budget. Timestamp/path substitutions, full text hunks and
numeric tolerance remain planned.

Exit 0 means both targets passed their contracts and no blocking changes were
found; 1 means a contract failed or a blocking change occurred; 2 indicates invalid input or
unsupported JSON content; 3 indicates infrastructure failure; 4 means comparison
was incomplete. An incomplete baseline skips the candidate. Only an explicit
baseline PASS to candidate FAIL is labelled `contract_regression`; other changes
are reported without assuming they break compatibility.

Private capture is bounded to 64 MiB per target, diagnostic output to 1,000
differences (with an omitted count), and the JSON document to 16 MiB. Comparisons
happen before masking; configured secrets are masked before excerpts or JSON
escaping. Raw stream bytes are never included wholesale in the report. Existing
`run` JSON/JUnit schema-v1 contracts and `compare-reports` remain unchanged.

## Reviewed compatibility policies

```powershell
.\target\release\spanforge-verify.exe compare --file suite.toml --baseline old.exe --candidate new.exe --policy examples/compatibility-policy.toml
```

A policy has a version, name, comparison settings and ordered rules. Each rule
selects a field and optionally a case and exact diagnostic path, and supplies an
impact (`allowed`, `breaking` or `changed`) plus a reason. The first matching rule
applies; put specific rules before broad rules. Unmatched changes block by default.
The example policy expects JSON stdout; adapt it to your CLI before use.

Allowed differences remain visible with their rule ID/reason and raw hashes.
Breaking/allowed/blocking totals include differences beyond the diagnostic cap.
The policy name/hash is recorded. Existing suite assertion failures always fail;
policies cannot allow assertion/contract failures. Policy comparison settings
combine with explicitly enabled CLI flags. This is a declared compatibility
contract, not an automatic semantic-version recommendation.

## Failure bundles, environment checks and replay

```powershell
.\target\release\spanforge-verify.exe run --file suite.toml --bundle-on-failure failure-bundle
.\target\release\spanforge-verify.exe doctor --bundle failure-bundle --program original.exe
.\target\release\spanforge-verify.exe replay --bundle failure-bundle --program original.exe
```

`run --bundle-on-failure` exports the first non-passing case's actual immutable
inputs and a failure witness. Later source changes do not alter those snapshots.
The directory must be new, its parent must exist, and it cannot be inside fixture
inputs. A passing run creates no bundle. Ordinary run JSON/JUnit reports are still
published; export errors are reported separately.

For a reviewed case without executing it, use:

```powershell
.\target\release\spanforge-verify.exe bundle --file suite.toml --case example --out case-bundle
```

Bundles include portable suite references, fixture files/empty directories, stdin,
expectations, file checksums, original binary/suite hashes and platform metadata.
The manifest is written last; incomplete exports cannot replay. Inputs are bounded
to 100 MiB/10,000 entries and the manifest to 4 MiB. Existing destinations are never
overwritten. Integrity checks detect missing/changed/extra files and directories.

Configured secret environment values are excluded and required by name and hash
at replay. Bundling refuses literal or decoded JSON/TOML secret values in inputs
by default. `bundle --include-sensitive-inputs` explicitly permits a private export
and records that decision; automatic failure exports keep the default exclusion.
Configured masking and explicit case secret values must agree before bundling.

`doctor` launches no target and reports integrity, platform, required secret inputs
and native target/suite prerequisites. `replay` rechecks them, requires the original
executable SHA256, runs in a fresh workspace and emits normal JSON run results to
stdout. A reproduced assertion failure exits 1. Replay requires the original OS
family/architecture; equivalent behavior on another supported machine is a goal
to validate, not a guarantee. Binaries, external services, absolute argument paths
and undeclared dependencies are not packaged or automatically remapped.


## Offline HTTP failure tests

A case can declare an ordered `http.responses` list. The runner starts a fresh
IPv4 loopback HTTP/1.1 server, supplies its base URL through `http.url_env`, and
replaces `{{http.url}}` in argument strings. Arguments go directly to the executable;
there is no shell or general environment interpolation. See
[examples/http-fixtures.toml](examples/http-fixtures.toml) for errors and redirects.
On Linux, change its program to the absolute path of a native curl binary.

Responses support GET/HEAD, exact paths including queries, status 200..599,
UTF-8 bodies, relative redirect locations and delays up to 60 seconds. Each request
consumes one ordered response. Missing, extra, wrong-method or wrong-path requests
fail the `http_requests` assertion even if the CLI exits successfully. Incomplete
process runs leave this assertion unevaluated. Comparison and replay recreate the
fixture separately; allocated ports can differ, so avoid asserting their exact value.

Limits are 64 responses, 64 KiB per response body, 16 KiB request headers and one
connection at a time. The listener closes and its worker joins on normal completion,
errors, cancellation and timeout. There is no TLS, POST/body matching, concurrent
request scheduling or general network isolation. Use `--noproxy '*'` with curl to
keep these tests independent of proxy settings. Faults affect this local endpoint.

## Start a reviewed suite and see coverage gaps

```powershell
.\target\release\spanforge-verify.exe init --program 'D:\tools\my-cli.exe' --file my-cli.toml
.\target\release\spanforge-verify.exe validate --file my-cli.toml
.\target\release\spanforge-verify.exe coverage --file my-cli.toml --fail-on-unmapped
```

`init --program` creates a starter without launching the target or overwriting an
existing file. Review the suggested `--help` argument, expected exit code and usage
pattern, then add invalid-input and failure-path cases. The template deliberately
leaves invalid-input behavior unmapped. The original `init` example remains available.

Declare behaviors with `[[contracts]]`, an `id`, a `description` and a `cases` list.
An empty list declares a known gap. `coverage` validates the entire suite without
launching it and emits JSON with mapped/unmapped contracts, cases without mappings,
and unasserted stdout/stderr. `--fail-on-unmapped` exits 1 for unmapped contracts or
an absent inventory, 2 for invalid configuration, and 0 for a complete mapping.
Mappings are structural evidence, not proof that assertions establish each behavior;
undeclared behaviors remain unknown. Stream gaps may be intentional. Bundles retain
contract declarations and restrict their case references to the selected case.

## Multi-step workflows, matrices and repeated attempts

[examples/workflows.toml](examples/workflows.toml) demonstrates all three using the
native development fixture. Build it with `cargo +stable build --release --locked
--bin spanforge-verify --bin spanforge-verify-fixture`, then:

```powershell
.\target\release\spanforge-verify.exe run --file examples/workflows.toml --json workflow-run.json
.\target\release\spanforge-verify.exe repeatability --report workflow-run.json --fail-on-problems
```

A scenario is a case with `args = []`, `expect = { exit_code = 0 }` and ordered
`[[cases.steps]]` entries. Its parent defines the initial fixture, inherited
ordinary environment, optional working directory and limits. Each step declares
its own arguments, expected exit, stream/file assertions, optional stdin, explicit
environment overrides and HTTP fixture. All steps use the same private workspace.
File-change checks use the snapshot immediately before that step; the scenario
also records the final delta from its initial fixture. Steps cannot declare their
own fixtures, inherited environment, nested workflows, matrices or repeat counts.
Every scenario attempt receives a fresh workspace and one scenario-level cleanup.

A step may extract strict JSON stdout scalars using named fields such as
`extract = { token = { pointer = '/token', kind = 'string' } }`. Supported kinds
are `string`, `number`, `boolean` and `null`. A later step can use `{{steps.token}}`
in arguments, environment values, inline stdin or literal text assertions. Exact
numbers use canonical coefficient/exponent text, avoiding floating-point loss.
Bindings must be declared earlier, unique, at most 4 KiB, and free of NUL. Values
are substituted in one pass without a shell. File-rule paths, cwd and source
filenames remain static for step bindings. Extracted values are not persisted as
a separate report field; configured secret masking still applies to diagnostics.

A failed step stops dependent steps by default. `continue_on_failure = true` on
the parent continues after ordinary assertion failures while keeping the scenario
failed. Extraction/binding failures always stop dependents; cancellation, deadline
and infrastructure failures abort. Skipped steps remain visible with unchecked
assertions. JSON reports include nested step results and prefixed parent checks;
JUnit retains scenario checks. Scenario stdout/stderr comparison uses length-framed
exact bytes for each completed step; JSON/CRLF stream comparison modes are rejected
for selected scenarios. Generated-file semantic comparison still uses final files.
Private framed stream capture is capped at 64 MiB per scenario.

`[[cases.matrix]]` declares explicit named input rows rather than an implicit
Cartesian product. A row can supply `values`, replace `args`, override `env`,
choose `fixture_dir`, and override either `stdin_text` or `stdin_file`. Use
`{{matrix.name}}` for row values in arguments, environment, inline stdin, literal
text assertions, fixture/cwd/source filenames and file-rule paths. Each resolved
row is validated before any target runs, including rows outside the selection.
Regex patterns, JSON assertion payloads and HTTP response definitions remain
literal. Rows execute in declared order; repeated attempts execute within each row.

`repeat = N` creates 1..100 separate attempts with immutable inputs and fresh
workspaces. IDs are stable: `case--matrix-row--repeat-001`. Expanded IDs must fit
64 characters; each case supports at most 64 rows, the configured `max_cases`
applies after expansion, workflows allow at most 1,000 total parent/step entries,
and expanded configuration is capped at 16 MiB. Contract mappings expand alongside
cases. `--case case` selects its rows/attempts; a row or exact attempt ID narrows it.
Bundle export requires an exact expanded ID and replays that attempt, not the full
campaign. Scenario bundles include all step inputs and expectations.

`repeatability --report` reports every group's passes, failures, missing/incomplete
attempts and distinct exact stream/final-workspace observations. Classifications
are `consistent_pass`, `consistent_failure`, `flaky` (mixed pass/fail), `variable`
(different observations without mixed pass/fail), and `incomplete`. Timing is
excluded. Every attempt remains in the run report; any failed attempt keeps run
exit 1. `--fail-on-problems` exits 1 for source-run failure, missing repeat groups
or any group other than `consistent_pass`; malformed metadata exits 2. A single
replayed attempt is an incomplete campaign. Variability is evidence, not a causal
or statistical diagnosis. Configured secret values are still excluded from bundles;
replaying step secret environment values requires the original value by name/hash.
