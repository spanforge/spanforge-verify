# Platform scope amendment

Approved direction: 6 October 2026. This amendment supersedes the Windows-only
platform scope and mandatory Windows Server 2022 release matrix in specification
v1.2. The original DOCX is retained as the historical baseline.

## Supported platform targets

The first portable release targets Windows 11 x64 laptops and Linux x64 containers.
A Windows Server installation, account, runner or CI provider is
not required to develop or release the Windows laptop build. macOS and ARM64
remain separate platform targets until their adapters and acceptance evidence exist.
Native Linux laptop distribution is outside this first-release scope.

Windows keeps suspended native launch and Job Objects. Linux needs a native
process adapter with direct argv execution, concurrent bounded pipes, deadlines,
signal cancellation, descendant termination and reaping. Process groups alone
do not prove cleanup of children that create a new session. Prove supported
cleanup using Linux subreaper/process accounting and, where available, cgroups;
fail closed when the required cleanup semantics cannot be established. Do not
claim a Windows executable runs inside a Linux container: targets and runner
must match the container's OS and architecture.

Containers must work without privileged mode, Docker socket access or a host
cgroup mount. Stronger cgroup-based containment can be an explicitly documented
deployment option. Run only trusted targets; neither a workspace nor process
lifecycle management provides hostile-code isolation.

## Revised process gate

Process correctness still requires evidence, but it is evaluated per platform.
The Windows gate uses a local Windows 11 laptop in direct and restricted parent
Job Object sessions. Existing local evidence can satisfy the covered experiments;
real console cancellation and outstanding acceptance tests remain required.
Server 2022 results are optional compatibility evidence and never block laptop
development, runner integration or release.

The Linux gate runs on a laptop/VM or ordinary Linux container without elevated
container privileges. Exercise direct argv, empty/Unicode/metacharacter arguments,
stdin EOF/early close/blocked writes, independent binary streams, exact output
limits, timeouts, SIGINT/SIGTERM, child/grandchild cleanup, retained pipe handles,
orphan reaping and explicitly tested session-escape behavior. Run lifecycle
scenarios 100 times and retain OS/kernel/container/toolchain metadata, residual
process checks, descriptor counts and cleanup timings.

An unavailable platform's gate does not block implementing other platforms.
It does block claiming that platform as supported. No platform may silently fall
back to an unmanaged target launch.

## Schema and reporting portability

Keep suite schema version 1: declarative assertions, limits and exit-code meanings
remain intact. Validate native executable permissions on Linux, rather than an
`.exe` suffix. Execute only resolved absolute program paths, without a shell.
Use Windows ordinal case-insensitive path/environment comparison on Windows;
Linux comparisons are case-sensitive. Workspace names currently use the stricter
Windows-compatible naming subset on both platforms; Linux workspace paths must use
forward slashes. Native POSIX-only workspace names can be a later schema capability.
Symlinks/reparse entries remain rejected.
Use Unix device/inode identity for distinct input accounting.

A Unix signal-terminated target has no ordinary exit code and cannot satisfy a
required exit assertion. Report signal termination explicitly; do not fabricate
Windows-style status values. Record actual OS/architecture and platform-specific
environment baselines. Windows private directory rules continue to apply there;
Linux uses private HOME/TMPDIR and XDG profile/cache directories. Cross-platform
report consumers retain the same required fields and stable outcome precedence.

## Delivery changes

Provide native Windows and Linux binaries plus a Linux container image when the
Linux gate passes. Container builds and CI are optional ways to reproduce tests,
not prerequisites requiring a hosted account. Documentation must distinguish
implemented, tested and planned platform support. The runner, real reports and
local Windows/httpstatr acceptance are implemented. The Linux adapter uses
subreaper accounting, PID handles, nonblocking pipes and a held ELF file.
Linux runtime certification and storage-fault acceptance remain pending; see
[acceptance evidence](acceptance.md).
