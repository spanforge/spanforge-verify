# Implementation and acceptance evidence

Implemented: CLI execution, immutable fixtures and environments, sequential
cases, all seven stream modes, all file kinds, undeclared persistent changes,
private profiles, run/case limits, abort semantics, bounded diagnostics, literal
masking, real JSON/JUnit reports, atomic publication, native Windows/Linux adapters,
container gate and native release packaging. Added recorded-report comparison and
direct two-executable comparison with shared immutable inputs, exact stream/final
file hash checks, strict JSON pointer diffs and explicit ignore/CRLF options.
Named compatibility policies record reviewed allow/break rules and preserve
blocking counts beyond diagnostic caps. Selected generated JSON/text files have
content diffs. Bundle/doctor/replay and automatic failure export preserve original
immutable inputs and failure witnesses.
Offline ordered GET/HEAD fixtures, dynamic URL arguments/environment, delays,
status/redirect responses and request assertions integrate with run/compare/replay.
Onboarding `init --program` creates reviewed expectations without target execution;
coverage validates explicit behavior mappings and reports structural assertion gaps.
Multi-step scenarios retain nested results, typed scalar bindings, shared workspace
state and step-specific assertions with abort/cleanup semantics. Explicit parameter
rows and repeated attempts expand before launch. Repeatability reports exact stream
and final-workspace variability while preserving every attempt and source status.

111 Windows Rust unit/integration tests passed; Windows and Linux-target all-target
Clippy with warnings denied and rustfmt passed. A locked release build and automatic
failure-bundle/doctor/replay smoke passed, reproducing exit 37. Evidence is in
`target/product-workflow-evidence/cd27c15a-0ed6-4b35-bf82-6f43cec441c4`.
The smoke uses a workspace-local CLIVERIFYR_WORK_ROOT due to restricted default
temporary-directory access. Windows evidence includes validation/assertion and runner/report tests,
creation/modification/deletion checks, actual junction rejection and safe cleanup,
hardlink report aliases, no-clobber races, overwrite type changes, partial pair
publication and synthetic configuration/infrastructure reports. A real Windows
DACL denial of temporary-directory writes produced infrastructure exit 3 before
target launch; the harness restores its temporary-directory permissions. Reports retain
outcomes when excess diagnostics are omitted. Filesystem checks cover final
persistent state, not transient writes or a live disk quota.

Latest locked release smoke with native Windows curl passed the 503 and redirect
fixtures, coverage CI check and bundled redirect replay. Evidence:
`target/http-product-evidence/c6f78553-cc95-428e-83c6-578c372463a5`.
HTTP regression tests cover missing/wrong/extra requests, redirect and HEAD wire
content, resource/header validation, long-delay listener/worker cleanup, URL
substitution, comparison and replay. Coverage remains structural; Linux runtime,
TLS/POST/concurrent HTTP fixtures and automated onboarding discovery remain pending.

Latest workflow release smoke executed eight attempts across three groups from
`examples/workflows.toml`, passed structural coverage/repeatability and published
JSON/JUnit. Automatic export of an intentionally failed scenario passed doctor and
replayed the failure with all three steps (including skipped dependents). Evidence:
`target/workflow-evidence/32f6f1c3-fabd-4a0a-a87c-926adcc8c6e1`.
Workflow regression coverage includes shared state, typed exact-number bindings,
fail-stop/continue policies, global deadlines, secret masking and secret-env replay,
binary input rows, actual Unicode output paths, explicit provenance selection,
mixed pass/fail and successful output variability, and malformed repeat metadata.
Scenario stream comparison currently uses exact framed bytes; semantic stream
comparison, object/array bindings, automatic Cartesian/pairwise matrices and
statistical flakiness analysis are pending. Linux runtime remains unverified here.

Comparison tests cover different binary hashes, equal-length output/file changes,
JSON order/lossless-number behavior, ignored-field raw hash retention, escaped
JSON pointers, missing versus null, binary bytes, deep text excerpts, masking
before JSON escaping, both-target prelaunch validation, unchanged ordinary suite
hashes, immutable stdin across runs, capture limits/failure cleanup, incomplete
baseline candidate skipping and a shared run deadline. File content differences
support selected semantic JSON pointers/text excerpts; full hunks, substitutions
and numeric tolerances remain roadmap items. Reproduction tests cover source
mutation/deletion, failure witnesses, no-launch bundle/doctor checks, integrity
and directory-inventory rejection, original executable hashes, secret exclusion
(including JSON escaping), environment prerequisites and fixture-destination
protection. Second-machine replay adoption remains unvalidated. Previously packaged
ZIPs predate these commands.

The local Windows prototype passed 100 repetitions of 16 lifecycle scenarios in
ordinary and restricted parent-job sessions (3,200 measured experiments plus
warmups). Separate full-CLI CTRL_C_EVENT testing passed 100 repetitions with no
residual fixture processes. The harness owns an isolated hidden console. It
reproduced inherited Ctrl-C suppression; the runner explicitly enables delivery
after registering its cancellation handler. The final sleep-protected lifecycle
run is target/prototype-evidence/a68adff8-3857-4b69-92fa-d09f4064c088 (maximum
cleanup 58 ms; zero residuals). An earlier run was interrupted by Modern Standby;
its deadline failure remains preserved. Test scripts temporarily inhibit automatic
sleep and restore their preceding thread power state after completion.

The eight-case pinned httpstatr 1.0.0 pilot passed, including target-enforced timeout
and synthetic redaction. An intentional expectation regression returned FAIL/exit 1.
Artifacts record concrete versions/hashes. The laptop reports Windows build 26200,
display version 25H2, x64.

Linux code and Linux-target tests compile and pass Clippy with warnings denied.
Linux runtime tests and the nonprivileged 100-repeat container gate have not run.
The Dockerfile and optional workflow provide reproduction. Windows Server 2022
is optional and does not block Windows development or release.

Public-release work still requires:

- Linux/container runtime evidence before advertising Linux support.
- Controlled container disk-full/storage-denial acceptance on disposable storage;
  tools/accept-container-storage.py implements the harness, pending a container host.
- Review of pinned httpstatr contracts and packaged third-party notices.
- Release signing credentials if distributing a signed Windows binary.

No installed Linux runtime, container engine or signing certificate is available
here. These are verification/distribution limits; run is implemented and no longer
an execution stub. Packages produced now are unsigned candidates.
