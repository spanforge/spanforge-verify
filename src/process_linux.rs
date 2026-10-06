//! Linux lifecycle adapter. Subreaper adoption accounts for descendants even
//! when they escape the initial session/process group. No cgroup privileges.
use crate::model::TerminationReason;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
static EXECUTION: Mutex<()> = Mutex::new(());
struct ExecImage {
    _arguments: Vec<std::ffi::CString>,
    _environment: Vec<std::ffi::CString>,
    argv: Vec<*const libc::c_char>,
    env: Vec<*const libc::c_char>,
}
// SAFETY: pointers only reference this owner's immutable CString allocations.
// Moving the owner preserves those allocations; no mutation is exposed.
unsafe impl Send for ExecImage {}
unsafe impl Sync for ExecImage {}
impl ExecImage {
    fn execute(&self, fd: i32) -> std::io::Error {
        unsafe {
            libc::syscall(
                libc::SYS_execveat,
                fd,
                c"".as_ptr(),
                self.argv.as_ptr(),
                self.env.as_ptr(),
                libc::AT_EMPTY_PATH,
            );
        }
        std::io::Error::last_os_error()
    }
}
pub struct Request<'a> {
    pub executable: &'a Path,
    pub executable_file: &'a fs::File,
    pub args: &'a [String],
    pub cwd: &'a Path,
    pub env: &'a BTreeMap<String, String>,
    pub stdin: &'a [u8],
    pub max_output_bytes: usize,
    pub timeout: Duration,
    pub run_deadline: Instant,
    pub cancelled: Arc<AtomicBool>,
}
#[derive(Debug)]
pub struct ProcessError {
    pub reason_code: &'static str,
    pub message: String,
}
impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.reason_code, self.message)
    }
}
impl std::error::Error for ProcessError {}
fn error(code: &'static str, message: impl std::fmt::Display) -> ProcessError {
    ProcessError {
        reason_code: code,
        message: message.to_string(),
    }
}
#[derive(Default)]
pub struct Captured {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}
pub struct Observation {
    pub raw_exit_code: Option<u32>,
    pub termination: Option<TerminationReason>,
    pub stdout: Captured,
    pub stderr: Captured,
    pub supplied_bytes: usize,
    pub written_bytes: usize,
    pub stdin_incomplete: bool,
}
fn nonblocking(fd: i32) -> Result<(), ProcessError> {
    // SAFETY: fcntl operates on an owned live pipe; flags are preserved.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(error("pipe_failed", std::io::Error::last_os_error()));
    }
    Ok(())
}
fn children(pid: u32) -> Result<Vec<u32>, ProcessError> {
    let mut result = BTreeSet::new();
    let tasks = match fs::read_dir(format!("/proc/{pid}/task")) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(error("accounting_failed", e)),
    };
    for task in tasks {
        let task = task.map_err(|e| error("accounting_failed", e))?;
        match fs::read_to_string(task.path().join("children")) {
            Ok(text) => {
                for value in text.split_whitespace() {
                    result.insert(
                        value
                            .parse()
                            .map_err(|_| error("accounting_failed", "Invalid /proc child PID"))?,
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(error("accounting_failed", e)),
        }
    }
    Ok(result.into_iter().collect())
}
fn members() -> Result<Vec<u32>, ProcessError> {
    let mut pending = children(std::process::id())?;
    let mut seen = BTreeSet::new();
    while let Some(pid) = pending.pop() {
        if seen.insert(pid) {
            pending.extend(children(pid)?);
        }
        if seen.len() > 10000 {
            return Err(error("accounting_failed", "More than 10000 processes"));
        }
    }
    Ok(seen.into_iter().collect())
}
fn kill(pid: u32) -> Result<(), ProcessError> {
    // pidfds protect against PID reuse. Unsupported kernels fail closed.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(error("cleanup_failed", e));
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    if unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            libc::SIGKILL,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    } < 0
    {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::ESRCH) {
            return Err(error("cleanup_failed", e));
        }
    }
    Ok(())
}
fn reap_except(root: u32) -> Result<(), ProcessError> {
    for pid in children(std::process::id())? {
        if pid == root {
            continue;
        }
        let mut status = 0;
        let waited = unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) };
        if waited < 0 {
            let e = std::io::Error::last_os_error();
            if !matches!(e.raw_os_error(), Some(libc::ECHILD | libc::EINTR)) {
                return Err(error("cleanup_failed", e));
            }
        }
    }
    Ok(())
}
fn drain(pipe: &mut impl Read, capture: &mut Captured, limit: usize) -> Result<bool, ProcessError> {
    let mut buffer = [0u8; 8192];
    // Bound work per iteration so flood output cannot starve cancellation.
    for _ in 0..16 {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                let retained = count.min(limit.saturating_sub(capture.bytes.len()));
                capture.bytes.extend_from_slice(&buffer[..retained]);
                if retained < count {
                    capture.truncated = true;
                    return Ok(false);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(error("pipe_failed", e)),
        }
    }
    Ok(false)
}
pub fn run(request: &Request<'_>) -> Result<Observation, ProcessError> {
    let _guard = EXECUTION
        .lock()
        .map_err(|_| error("accounting_failed", "Execution lock poisoned"))?;
    if !children(std::process::id())?.is_empty() {
        return Err(error(
            "accounting_failed",
            "Runner must have no unrelated child processes",
        ));
    }
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } < 0 {
        return Err(error("accounting_failed", std::io::Error::last_os_error()));
    }
    // Probe pidfd availability before launching any target.
    let probe = unsafe { libc::syscall(libc::SYS_pidfd_open, std::process::id(), 0) };
    if probe < 0 {
        return Err(error(
            "accounting_failed",
            "Kernel or container policy does not support pidfd_open",
        ));
    }
    let probe = unsafe { OwnedFd::from_raw_fd(probe as i32) };
    if unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            probe.as_raw_fd(),
            0,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    } < 0
    {
        return Err(error(
            "accounting_failed",
            "Container policy does not support pidfd_send_signal",
        ));
    }
    drop(probe);
    let mut command = Command::new(request.executable);
    command
        .args(request.args)
        .current_dir(request.cwd)
        .env_clear()
        .envs(request.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Execute the held native ELF file rather than reopening a replaceable path.
    // All CString storage and pointers are prepared before fork.
    let arguments = std::iter::once(request.executable.to_string_lossy().into_owned())
        .chain(request.args.iter().cloned())
        .map(std::ffi::CString::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| error("spawn_failed", e))?;
    let environment = request
        .env
        .iter()
        .map(|(k, v)| std::ffi::CString::new(format!("{k}={v}")))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| error("spawn_failed", e))?;
    let executable_fd = request.executable_file.as_raw_fd();
    let argv = arguments
        .iter()
        .map(|s| s.as_ptr())
        .chain([std::ptr::null()])
        .collect();
    let env = environment
        .iter()
        .map(|s| s.as_ptr())
        .chain([std::ptr::null()])
        .collect();
    let image = ExecImage {
        _arguments: arguments,
        _environment: environment,
        argv,
        env,
    };
    // SAFETY: only setsid and execveat syscalls run post-fork. Pointers reference
    // owned CString storage; the file stays open until spawn finishes.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Err(image.execute(executable_fd))
        });
    }
    let mut child = command.spawn().map_err(|e| error("spawn_failed", e))?;
    let root = child.id();
    let started = Instant::now();
    let deadline = started + request.timeout;
    let outcome = (|| {
        let mut stdin = child.stdin.take();
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| error("pipe_failed", "Missing stdout"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| error("pipe_failed", "Missing stderr"))?;
        for fd in [
            stdin
                .as_ref()
                .ok_or_else(|| error("pipe_failed", "Missing stdin"))?
                .as_raw_fd(),
            stdout.as_raw_fd(),
            stderr.as_raw_fd(),
        ] {
            nonblocking(fd)?;
        }
        let mut out = Captured::default();
        let mut err = Captured::default();
        let mut written = 0;
        let mut root_status = None;
        let mut root_exited = None;
        let mut termination = None;
        let mut stdout_eof = false;
        let mut stderr_eof = false;
        loop {
            let now = Instant::now();
            if request.cancelled.load(Ordering::Acquire) {
                termination = Some(TerminationReason::Cancelled);
                break;
            }
            if now >= request.run_deadline {
                termination = Some(TerminationReason::RunDeadline);
                break;
            }
            if now >= deadline {
                termination = Some(TerminationReason::Timeout);
                break;
            }
            if !stdout_eof {
                stdout_eof = drain(&mut stdout, &mut out, request.max_output_bytes)?;
            }
            if !stderr_eof {
                stderr_eof = drain(&mut stderr, &mut err, request.max_output_bytes)?;
            }
            if out.truncated || err.truncated {
                termination = Some(TerminationReason::OutputLimit);
                break;
            }
            if let Some(pipe) = &mut stdin {
                if written == request.stdin.len() {
                    stdin = None;
                } else {
                    match pipe.write(&request.stdin[written..]) {
                        Ok(n) => written += n,
                        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => stdin = None,
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) => {}
                        Err(e) => return Err(error("pipe_failed", e)),
                    }
                }
            }
            if root_status.is_none() {
                root_status = child
                    .try_wait()
                    .map_err(|e| error("accounting_failed", e))?;
            }
            reap_except(root)?;
            if root_status.is_some() {
                let exited = *root_exited.get_or_insert(now);
                if members()?.is_empty() && stdout_eof && stderr_eof {
                    break;
                }
                if now.duration_since(exited) >= Duration::from_millis(500) {
                    termination = Some(TerminationReason::LingeringDescendants);
                    break;
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        if termination.is_some() {
            shutdown(&mut child, root)?;
        } else {
            child.wait().map_err(|e| error("cleanup_failed", e))?;
        }
        // Nonblocking pipes cannot leave detached workers. Drain retained bytes.
        for _ in 0..32 {
            let a = drain(&mut stdout, &mut out, request.max_output_bytes)?;
            let b = drain(&mut stderr, &mut err, request.max_output_bytes)?;
            if a && b {
                break;
            }
        }
        let raw_exit_code = root_status.and_then(|s| s.code()).map(|code| code as u32);
        if termination.is_none() && (out.truncated || err.truncated) {
            termination = Some(TerminationReason::OutputLimit);
        }
        if termination.is_none() && raw_exit_code.is_none() {
            termination = Some(TerminationReason::Signal);
        }
        Ok(Observation {
            raw_exit_code,
            termination,
            stdout: out,
            stderr: err,
            supplied_bytes: request.stdin.len(),
            written_bytes: written,
            stdin_incomplete: written < request.stdin.len(),
        })
    })();
    if outcome.is_err() {
        shutdown(&mut child, root)?;
    }
    outcome
}
fn shutdown(child: &mut std::process::Child, root: u32) -> Result<(), ProcessError> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        // Direct children cannot have their PIDs reused until this parent reaps
        // them. Kill parents first; escaped descendants are then adopted.
        for pid in children(std::process::id())? {
            kill(pid)?;
        }
        child.try_wait().map_err(|e| error("cleanup_failed", e))?;
        reap_except(root)?;
        if children(std::process::id())?.is_empty() {
            child.wait().map_err(|e| error("cleanup_failed", e))?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(error(
                "cleanup_failed",
                "Residual descendant processes after shutdown",
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
}
