# Product direction: dependable CLI contracts and release confidence

This document records the earlier CLI-focused direction. The user approved a
broader AI testing and verification direction on 6 October 2026; see
[docs/ai-verification-direction.md](ai-verification-direction.md). The CLI
capabilities and evidence below remain the execution foundation; delivery priorities
now follow the AI verification phases in implementationplan.md.

The user's objective is to make spanforge-verify a preferred tool by solving recurring
developer problems exceptionally well. Existing competitors do not invalidate
that objective. Prioritize complete useful workflows and measurable adoption over
the number of implemented features.

## Evidence and limits

- [Bats documentation](https://bats-core.readthedocs.io/en/stable/writing-tests.html#file-descriptor-3-read-this-if-bats-hangs)
  explains how background children inheriting a descriptor can hang execution and
  requires explicit descriptor handling by the test author.
- [Prysk's README](https://github.com/prysk/prysk) also warns that a daemon retaining
  stdout can hang the test shell. It provides transcript tests and snapshot review.
- [trycmd documentation](https://docs.rs/trycmd/latest/trycmd/) provides TOML and
  Markdown tests, snapshot generation/update, filesystem fixtures and output
  elision; it points to companion tools for custom predicates and interactive tests.

These establish specific lifecycle and authoring concerns. They do not prove that
all competitors lack replay, compatibility checks or fault simulation, nor that
developers will switch. The opportunities below are product hypotheses to validate
with actual CLI maintainers and real suites.

## Initial audience and product promise

Start with maintainers of executable CLIs used by automation, especially teams
shipping Windows and Linux builds and CLIs that access HTTP services or generate
files. httpstatr is our first concrete pilot. Developers should be able to define
the behavior they promise, check a release, understand a failure and reproduce it
with little project-specific glue.

Promise: protect CLI behavior that users and automation depend on, and turn a
failure into a precise, reproducible explanation.

## Opportunities ranked by developer outcome

| Developer problem | Product opportunity | Proof of value |
| --- | --- | --- |
| A green build still changes behavior used by scripts | Explicit release compatibility contracts for commands, arguments, exit conventions, JSON schemas and generated files; semantic two-version comparison with declared breaking-change rules | Detect deliberately introduced consumer-breaking changes while allowing declared compatible changes |
| A failure occurs only on another laptop or CI agent | Reproduction bundles and replay, executable hashes, dependency requirements and environment diagnostics; report unsupported reproduction rather than claiming portability | A second developer reproduces a failure from a clean supported environment without requesting extra files |
| Testing cancellation, unavailable services and failed writes takes custom infrastructure | Declarative HTTP/service fixtures and controlled failure scenarios, with local assigned ports and bounded teardown; storage faults only inside disposable environments | Verify retry limits, timeout exit codes, stderr explanations, partial files and cleanup without public services |
| Writing the first useful suite requires too much effort | Guided draft capture, validation diagnostics, reviewed expectations and reusable examples; observed output remains a draft, not proof of correctness | A new user gets a meaningful reviewed test running in five minutes |
| Snapshot changes are noisy and unexplained | Precise text/JSON/file differences, explicit normalization, repeated-run instability diagnostics and preserved raw hashes | A reviewer identifies the changed contract and explains allowed differences without broad ignores |
| Teams do not know which CLI promises remain untested | An explicit inventory of promised behaviors mapped to checks, including negative-input and boundary cases; report tested/untested contracts | Show real omissions; never label black-box contract coverage as source-code coverage |

## Delivery sequence

1. Complete the comparison workflow: semantic generated-file diffs, named
   compatibility policies, clear evidence and readable reports. Direct two-target
   execution and semantic JSON stream comparison already exist.
2. Deliver reproduction bundles plus replay and actionable environment checks as
   one end-to-end feature. Masked evidence cannot reconstruct excluded secrets;
   require named secret inputs at replay time. Report portability limitations.
3. Add declarative offline HTTP fixtures and scenario assertions; use httpstatr to
   prove slow responses, redirects, unexpected responses and request validation.
4. Build guided suite drafting and first-run examples. Drafts must require review;
   unstable-field suggestions must never silently broaden comparison rules.
5. Add explicit contract inventories and targeted boundary-case generation.
   Generated tests need valid oracles. A parser-inferred flag list is a suggestion,
   not an authoritative CLI specification.
6. Add workflow steps, matrices and repeatability when these improve the above
   user journeys. Watch mode, PTYs and broader integrations follow demonstrated use.

Linux runtime/container lifecycle and storage evidence remain mandatory before
advertising Linux support. They do not block independent Windows development.
Reliable installation, examples, stable configuration/report contracts and measured
diagnostic quality are part of each delivery, not post-release extras.

## Validation before expanding scope

Use real maintained CLIs from multiple language ecosystems and both target
platforms. Measure time to first useful test, time to reproduce a failure on a
second environment, agreement on labelled compatibility changes and maintenance
effort after several releases. Observe where maintainers still write glue scripts.
Record which proposed features they actually use and which failures the tool misses.

The five-minute onboarding and clean-environment replay goals are proposed targets,
not achieved measurements. No user interviews, competitive benchmark suite or
market-demand validation has been conducted yet. Interviews or external outreach
require separate authorization; no messages are sent by this document.
