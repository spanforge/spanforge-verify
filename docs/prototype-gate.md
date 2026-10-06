# Windows laptop lifecycle prototype gate

The standalone `prototype` binary exercises the process adapter independently of
the full suite runner and reporters. The approved [platform amendment](platform-scope.md)
removes the mandatory Server 2022 matrix: local Windows laptop evidence is accepted,
and Linux containers have an independent gate. `spanforge-verify run` is implemented;
an unavailable Server 2022 runner does not prevent development or local acceptance.

From a Windows x64 MSVC development environment:

```powershell
./tools/run-prototype-gate.ps1 -Repeat 100
```

Evidence is retained in a unique directory under `target/prototype-evidence`:
ordinary and parent-job JSON plus a manifest of OS build, toolchain, binaries,
dependency lock hash, repeat count and both exit codes. The prototype deliberately
refuses to overwrite evidence files. Iteration zero warms standard library and OS
initialization while still checking scenario correctness; iterations 1–100 require
stable per-scenario handle counts. Each case records job accounting, held member
handle checks, process IDs and cleanup timing. Membership accounting and process
handle signalling are both required: either alone can precede completed teardown.

The parent harness constrains its job to 64 active processes and forbids breakaway.
The inner prototype creates its own jobs, verifying nested assignment and cleanup.
Windows Server 2022 CI is optional compatibility evidence.
The Windows 11 job needs a self-hosted runner explicitly labelled
`cliverifyr-windows11`; a hosted `windows-latest` label does not establish Windows 11.
Register only a trusted dedicated runner; the fixtures launch trusted test programs.

Scenarios cover argv (including Tamil/emoji, empty arguments, quotes, backslashes
and metacharacters), unsigned exit status, stdin EOF, binary/independent streams,
exact output budget and overflow, dual pipe pressure with blocked stdin, early
stdin closure, deadlines, child/grandchild cleanup, retained inherited output
handles, cooperative cancellation, and rejected assignment without target resume.
The rejected assignment performs a real failing Windows API call; its marker file
must never appear. It is a prototype hook, not a public suite feature.

Cooperative cancellation tests the adapter flag used by a Ctrl-C handler. Actual
console delivery is separately tested through the runner by tools/run-console-gate.ps1.
Its isolated hidden console sends CTRL_C_EVENT after grandchild readiness, checks
exit 4 and partial reports, and verifies that all fixture processes exited.
The prototype binary handles external Ctrl-C and exits
4 after preserving completed experiment records.

The first local stress run exposed a PID-only residual inspection race. Failed
evidence was preserved in `target/prototype-ordinary.json` and
`target/prototype-parent-job.json`; revised runs use `*-v2.json`. The adapter now
holds membership handles before termination and waits for their signalling inside
the shutdown budget rather than reopening terminated PIDs. Reproducers remain in
the `tree`, `linger`, timeout and cancellation experiments. Cancellation now waits
for a grandchild readiness marker instead of racing target startup.

On 6 October 2026, the final local `*-v4.json` runs passed all 100 repetitions of
16 scenarios in both direct and restricted parent-job modes (3,200 measured
experiments plus 32 warmups). Build was 26200.9457, display version 25H2, x64.
Both evidence files recorded zero residual members, unchanged handle counts after
warmup and successful outcomes. Metadata and binary/lock hashes are in
`target/prototype-v4-manifest.json`. The preceding v3 run encountered a simultaneous
48-second launch stall during concurrent build activity and correctly failed its
deadline experiments; it remains preserved. Final gate runs were isolated from
builds. These local results are accepted evidence for the covered Windows laptop
experiments. Actual console cancellation subsequently passed 100 repetitions;
target/console-gate-100.json recorded zero residual processes and a maximum observed
55 ms from event delivery to runner exit. This exposed inherited Ctrl-C suppression;
the runner now enables delivery after handler registration. Linux container
support requires separate Linux evidence.

Review the platform's logs, rejected-assignment cleanup, absent residual members,
handle counts and elapsed shutdown times before recording M2 complete. Keep M2
open if any experiment fails. Never add an unmanaged execution fallback.

Implementation references: [Microsoft Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
and [explicit inherited handle lists](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute).

The final refreshed gate also passed 100 repetitions in both modes, with max process cleanup 58 ms and no residuals: target/prototype-evidence/a68adff8-3857-4b69-92fa-d09f4064c088. A preceding refreshed run was interrupted by an 18-minute Modern Standby interval (Windows power events 506/507); its run_deadline result and 125 ms cleanup are preserved under 66dc2b51-e140-4340-94df-e45e44658988. Gate scripts now inhibit automatic sleep temporarily and restore the prior thread power state in finally; permanent power policy is unchanged.
