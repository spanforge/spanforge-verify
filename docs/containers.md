# Linux containers

The Linux adapter executes a held native ELF file with direct argv and an explicit
environment. Pipes are nonblocking and bounded. A subreaper adopts orphaned
descendants, including children that call setsid; PID handles avoid signalling
reused IDs. Cleanup kills and reaps adopted children until accounting is empty.
There is no process-group-only or unmanaged fallback.

Linux requires /proc task/children accounting, subreaper support and permitted
pidfd_open, pidfd_send_signal and execveat syscalls. Restricted policies fail closed.
Container acceptance has not run here: the laptop has no Docker/Podman or installed
WSL distribution. Linux-target compilation and Clippy pass from Windows.
This is not runtime certification.

```sh
docker build --platform linux/amd64 -t spanforge-verify-dev .
docker run --rm spanforge-verify-dev --version
docker run --rm --mount type=bind,source=/absolute/suites,target=/suites,readonly \
  spanforge-verify-dev run --file /suites/spanforge-verify.toml
```

The build runs Rust tests, builds native binaries, then runs 100 repetitions of
14 lifecycle experiments as UID/GID 65532. The foundation stage retains evidence
under /evidence/linux-gate.json. The final image includes the executable, license,
notices and examples. No privileged mode, Docker socket or host cgroup mount is used.
The image pins Rust 1.98.1; --build-arg RUST_IMAGE can select a local image with that
exact compiler. Image availability/build completion remains unverified.

Inputs can be mounted read-only: workspaces use /tmp by default. Report directories
must be separately writable by UID 65532. For persisted reports:

```sh
docker run --rm \
  --mount type=bind,source=/absolute/suites,target=/suites,readonly \
  --mount type=bind,source=/absolute/reports,target=/reports \
  spanforge-verify-dev run --file /suites/spanforge-verify.toml --json /reports/run.json --junit /reports/run.xml
```

Use executable Linux ELF targets and readable fixtures. Windows binaries and
scripts are unsupported targets in this image. Linux paths/environment names are
case-sensitive; workspace names use the documented portable subset. Target signals
produce raw_exit_code=null and termination_reason=signal. SIGINT/SIGTERM to the
runner trigger cooperative cancellation and descendant cleanup.

On Linux, reproduce the gate with sh tools/run-linux-gate.sh 100. Hosted CI is optional.

After building the image, run `python3 tools/accept-container-storage.py --image spanforge-verify-dev`.
It tests a 2 MiB writable tmpfs against a larger immutable fixture, and a read-only
temporary directory, while reports use a separate writable mount. These tests
need no privileged container and never fill the host filesystem.
