//! Windows lifecycle adapter shared by the runner and standalone process gate.
//! Children are suspended until Job Object assignment succeeds. No fallback launch.
use crate::model::TerminationReason;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::SECURITY_ATTRIBUTES,
        System::{IO::CancelSynchronousIo, JobObjects::*, Pipes::CreatePipe, Threading::*},
    },
    core::{PCWSTR, PWSTR},
};

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
fn error(code: &'static str, value: impl std::fmt::Display) -> ProcessError {
    ProcessError {
        reason_code: code,
        message: value.to_string(),
    }
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}
impl Handle {
    fn file(self) -> File {
        let raw = self.0.0;
        std::mem::forget(self);
        // SAFETY: transfers one owned real pipe handle to File.
        unsafe { File::from_raw_handle(raw) }
    }
}
struct Attributes {
    _storage: Vec<usize>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}
impl Attributes {
    fn new(handles: &[HANDLE]) -> Result<Self, ProcessError> {
        let mut bytes = 0;
        unsafe {
            let _ = InitializeProcThreadAttributeList(None, 1, None, &mut bytes);
        }
        if bytes == 0 {
            return Err(error("spawn_failed", "Attribute list allocation failed"));
        }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(storage.as_mut_ptr().cast());
        unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut bytes) }
            .map_err(|e| error("spawn_failed", e))?;
        let owner = Self {
            _storage: storage,
            list,
        };
        unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                std::mem::size_of_val(handles),
                None,
                None,
            )
        }
        .map_err(|e| error("spawn_failed", e))?;
        Ok(owner)
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.list);
        }
    }
}
fn pipe(parent_reads: bool) -> Result<(Handle, Handle), ProcessError> {
    let security = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        bInheritHandle: true.into(),
        ..Default::default()
    };
    let mut read = HANDLE::default();
    let mut write = HANDLE::default();
    unsafe { CreatePipe(&mut read, &mut write, Some(&security), 0) }
        .map_err(|e| error("pipe_failed", e))?;
    let read = Handle(read);
    let write = Handle(write);
    let parent = if parent_reads { read.0 } else { write.0 };
    unsafe { SetHandleInformation(parent, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }
        .map_err(|e| error("pipe_failed", e))?;
    Ok((read, write))
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
/// MSVC/standard Windows argv quoting. Every argument is quoted, including empty.
pub fn quote(argument: &str) -> String {
    let mut output = String::from("\"");
    let mut slashes = 0;
    for c in argument.chars() {
        if c == '\\' {
            slashes += 1;
            continue;
        }
        if c == '"' {
            output.push_str(&"\\".repeat(slashes * 2 + 1));
            output.push('"');
        } else {
            output.push_str(&"\\".repeat(slashes));
            output.push(c);
        }
        slashes = 0;
    }
    output.push_str(&"\\".repeat(slashes * 2));
    output.push('"');
    output
}
pub struct Request<'a> {
    pub executable: &'a Path,
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
pub struct Captured {
    pub bytes: Vec<u8>,
    pub truncated: bool,
}
#[derive(Debug)]
pub struct Observation {
    pub raw_exit_code: Option<u32>,
    pub termination: Option<TerminationReason>,
    pub stdout: Captured,
    pub stderr: Captured,
    pub supplied_bytes: usize,
    pub written_bytes: usize,
    pub stdin_incomplete: bool,
    pub active_members_after_cleanup: u32,
    pub pid: u32,
    pub duration: Duration,
    pub cleanup_duration: Duration,
    pub member_pids_at_shutdown: Vec<u32>,
    pub residual_processes: Vec<u32>,
}
struct CaptureResult {
    capture: Captured,
    error: Option<std::io::Error>,
}
struct WriteResult {
    written: usize,
    error: Option<std::io::Error>,
}
fn reader(
    mut file: File,
    limit: usize,
    overflow: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<CaptureResult>> {
    thread::Builder::new()
        .name("spanforge-verify-capture".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 8192];
            let mut truncated = false;
            let mut failure = None;
            loop {
                match file.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        let remaining = limit.saturating_sub(bytes.len());
                        bytes.extend_from_slice(&buffer[..count.min(remaining)]);
                        if count > remaining {
                            truncated = true;
                            overflow.store(true, Ordering::Release);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        failed.store(true, Ordering::Release);
                        failure = Some(e);
                        break;
                    }
                }
            }
            CaptureResult {
                capture: Captured { bytes, truncated },
                error: failure,
            }
        })
}
fn writer(
    mut file: File,
    bytes: Vec<u8>,
    failed: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<WriteResult>> {
    thread::Builder::new()
        .name("spanforge-verify-stdin".into())
        .spawn(move || {
            let mut written = 0;
            let mut failure = None;
            while written < bytes.len() {
                match file.write(&bytes[written..]) {
                    Ok(0) => {
                        failure = Some(std::io::ErrorKind::WriteZero.into());
                        break;
                    }
                    Ok(count) => written += count,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        if e.kind() != std::io::ErrorKind::BrokenPipe {
                            failed.store(true, Ordering::Release);
                        }
                        failure = Some(e);
                        break;
                    }
                }
            }
            WriteResult {
                written,
                error: failure,
            }
        })
}
fn cancel<T>(worker: &JoinHandle<T>) {
    // SAFETY: JoinHandle retains this real thread handle through the call.
    unsafe {
        let _ = CancelSynchronousIo(HANDLE(worker.as_raw_handle()));
    }
}
struct Job {
    handle: Handle,
}
impl Job {
    fn members(&self) -> Result<Vec<(u32, Handle)>, ProcessError> {
        // Aligned storage for the variable-length PROCESS_ID_LIST header and IDs.
        let mut storage = vec![0usize; 1026];
        loop {
            let status = unsafe {
                QueryInformationJobObject(
                    Some(self.handle.0),
                    JobObjectBasicProcessIdList,
                    storage.as_mut_ptr().cast(),
                    (storage.len() * size_of::<usize>()) as u32,
                    None,
                )
            };
            if let Err(e) = status {
                if e.code().0 as u32 == 0x800700EA && storage.len() < 65538 {
                    storage.resize(storage.len() * 2, 0);
                    continue;
                }
                return Err(error("cleanup_failed", e));
            }
            let list = unsafe { &*storage.as_ptr().cast::<JOBOBJECT_BASIC_PROCESS_ID_LIST>() };
            let count = list.NumberOfProcessIdsInList as usize;
            if count > storage.len() - 1 {
                return Err(error("cleanup_failed", "Invalid membership count"));
            }
            let ids = unsafe { std::slice::from_raw_parts(list.ProcessIdList.as_ptr(), count) };
            let mut members = Vec::new();
            for id in ids {
                match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, *id as u32) } {
                    Ok(handle) => members.push((*id as u32, Handle(handle))),
                    // A member may exit between the accounting query and OpenProcess.
                    Err(e) if e.code().0 as u32 == 0x80070057 => (),
                    Err(e) => return Err(error("cleanup_failed", e)),
                }
            }
            return Ok(members);
        }
    }
    fn new(parent_limit: Option<u32>) -> Result<Self, ProcessError> {
        let handle = Handle(
            unsafe { CreateJobObjectW(None, PCWSTR::null()) }
                .map_err(|e| error("job_assignment_failed", e))?,
        );
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if let Some(limit) = parent_limit {
            limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
            limits.BasicLimitInformation.ActiveProcessLimit = limit;
        }
        unsafe {
            SetInformationJobObject(
                handle.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|e| error("job_assignment_failed", e))?;
        Ok(Self { handle })
    }
    fn active(&self) -> Result<u32, ProcessError> {
        let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
        unsafe {
            QueryInformationJobObject(
                Some(self.handle.0),
                JobObjectBasicAccountingInformation,
                (&mut accounting as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                None,
            )
        }
        .map_err(|e| error("cleanup_failed", e))?;
        Ok(accounting.ActiveProcesses)
    }
    fn terminate(&self) -> Result<(), ProcessError> {
        unsafe { TerminateJobObject(self.handle.0, 1) }.map_err(|e| error("cleanup_failed", e))
    }
}
struct Root {
    process: Handle,
    thread: Handle,
    reaped: bool,
}
impl Drop for Root {
    fn drop(&mut self) {
        if !self.reaped {
            unsafe {
                let _ = TerminateProcess(self.process.0, 1);
                let _ = WaitForSingleObject(self.process.0, 2000);
            }
        }
    }
}
pub fn run(request: &Request<'_>) -> Result<Observation, ProcessError> {
    execute(request, false, None)
}
/// Runs the gate harness under a parent Job Object with a 64-process CI-like limit.
pub fn prototype_parent_job(request: &Request<'_>) -> Result<Observation, ProcessError> {
    execute(request, false, Some(64))
}
/// Gate-only fault injection: perform a rejected assignment against an invalid job.
/// The suspended target must be terminated/reaped without ever being resumed.
pub fn prototype_rejected_assignment(request: &Request<'_>) -> Result<Observation, ProcessError> {
    execute(request, true, None)
}
fn execute(
    request: &Request<'_>,
    reject_assignment: bool,
    parent_limit: Option<u32>,
) -> Result<Observation, ProcessError> {
    if !request.executable.is_absolute()
        || !request.cwd.is_absolute()
        || request.max_output_bytes == 0
    {
        return Err(error("spawn_failed", "Invalid prototype request"));
    }
    let executable = request
        .executable
        .to_str()
        .ok_or_else(|| error("spawn_failed", "Non-Unicode executable"))?;
    let cwd = request
        .cwd
        .to_str()
        .ok_or_else(|| error("spawn_failed", "Non-Unicode cwd"))?;
    if executable.contains('\0')
        || cwd.contains('\0')
        || request.args.iter().any(|a| a.contains('\0'))
    {
        return Err(error("spawn_failed", "NUL in launch input"));
    }
    let mut command = quote(executable);
    for arg in request.args {
        command.push(' ');
        command.push_str(&quote(arg));
    }
    let mut command = wide(&command);
    if command.len() > 32767 {
        return Err(error("spawn_failed", "Windows command line exceeds limit"));
    }
    let application = wide(executable);
    let cwd = wide(cwd);
    let mut env_pairs: Vec<_> = request.env.iter().collect();
    env_pairs.sort_by(|a, b| crate::inputs::ordinal_cmp(a.0, b.0));
    if !crate::inputs::unique_names(env_pairs.iter().map(|(k, _)| k.as_str()))
        || env_pairs
            .iter()
            .any(|(k, v)| k.is_empty() || k.contains(['\0', '=']) || v.contains('\0'))
    {
        return Err(error("spawn_failed", "Invalid environment"));
    }
    let mut environment = Vec::<u16>::new();
    for (name, value) in env_pairs {
        environment.extend(format!("{name}={value}").encode_utf16());
        environment.push(0);
    }
    environment.push(0);
    if environment.len() == 1 {
        environment.push(0);
    }
    let job = Job::new(parent_limit)?;
    let (stdin_read, stdin_write) = pipe(false)?;
    let (stdout_read, stdout_write) = pipe(true)?;
    let (stderr_read, stderr_write) = pipe(true)?;
    let inherited = [stdin_read.0, stdout_write.0, stderr_write.0];
    let attributes = Attributes::new(&inherited)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = stdin_read.0;
    startup.StartupInfo.hStdOutput = stdout_write.0;
    startup.StartupInfo.hStdError = stderr_write.0;
    startup.lpAttributeList = attributes.list;
    let mut info = PROCESS_INFORMATION::default();
    let started = Instant::now();
    let case_deadline = started + request.timeout;
    if request.cancelled.load(Ordering::Acquire) {
        return Err(error("cancelled", "Cancelled before launch"));
    }
    if started >= request.run_deadline {
        return Err(error("run_deadline", "Deadline before launch"));
    }
    // SAFETY: all buffers/attribute handles remain alive through this synchronous call;
    // application path is explicit and the only inherited handles are the three pipes.
    unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            Some(PWSTR(command.as_mut_ptr())),
            None,
            None,
            true,
            CREATE_SUSPENDED
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT
                | CREATE_NO_WINDOW,
            Some(environment.as_ptr().cast()),
            PCWSTR(cwd.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        )
    }
    .map_err(|e| error("spawn_failed", e))?;
    let mut root = Root {
        process: Handle(info.hProcess),
        thread: Handle(info.hThread),
        reaped: false,
    };
    drop(stdin_read);
    drop(stdout_write);
    drop(stderr_write);
    drop(attributes);
    let assignment = unsafe {
        AssignProcessToJobObject(
            if reject_assignment {
                HANDLE::default()
            } else {
                job.handle.0
            },
            root.process.0,
        )
    };
    if let Err(e) = assignment {
        unsafe { TerminateProcess(root.process.0, 1) }.map_err(|e| error("cleanup_failed", e))?;
        if unsafe { WaitForSingleObject(root.process.0, 2000) } != WAIT_OBJECT_0 {
            return Err(error(
                "cleanup_failed",
                "Suspended process did not terminate",
            ));
        }
        root.reaped = true;
        return Err(error("job_assignment_failed", e));
    }
    let overflow = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let stdout = reader(
        stdout_read.file(),
        request.max_output_bytes,
        overflow.clone(),
        failed.clone(),
    )
    .map_err(|e| error("pipe_failed", e))?;
    let stderr = match reader(
        stderr_read.file(),
        request.max_output_bytes,
        overflow.clone(),
        failed.clone(),
    ) {
        Ok(worker) => worker,
        Err(e) => {
            job.terminate()?;
            cancel(&stdout);
            return Err(error("pipe_failed", e));
        }
    };
    let stdin = match writer(stdin_write.file(), request.stdin.to_vec(), failed.clone()) {
        Ok(worker) => worker,
        Err(e) => {
            job.terminate()?;
            cancel(&stdout);
            cancel(&stderr);
            return Err(error("pipe_failed", e));
        }
    };
    if unsafe { ResumeThread(root.thread.0) } == u32::MAX {
        job.terminate()?;
        cancel(&stdout);
        cancel(&stderr);
        cancel(&stdin);
        return Err(error("spawn_failed", "ResumeThread failed"));
    }
    let mut root_exited = None;
    let mut termination = None;
    let mut infrastructure = None;
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
        if now >= case_deadline {
            termination = Some(TerminationReason::Timeout);
            break;
        }
        if overflow.load(Ordering::Acquire) {
            termination = Some(TerminationReason::OutputLimit);
            break;
        }
        if failed.load(Ordering::Acquire) {
            infrastructure = Some(error("pipe_failed", "Pipe worker failed"));
            break;
        }
        let wait = unsafe { WaitForSingleObject(root.process.0, 0) };
        if wait == WAIT_FAILED {
            infrastructure = Some(error("pipe_failed", "Process wait failed"));
            break;
        }
        if wait == WAIT_OBJECT_0 {
            let exit_time = *root_exited.get_or_insert(now);
            match job.active() {
                Ok(0) => break,
                Ok(_) => {
                    if now.duration_since(exit_time) >= Duration::from_millis(500) {
                        termination = Some(TerminationReason::LingeringDescendants);
                        break;
                    }
                }
                Err(e) => {
                    infrastructure = Some(e);
                    break;
                }
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
    let cleanup = Instant::now();
    let shutdown_deadline = cleanup + Duration::from_secs(2);
    // Hold member handles before termination: PID-only residual checks can race
    // process teardown and PID reuse after termination.
    let members = job.members()?;
    let member_pids_at_shutdown = members.iter().map(|(pid, _)| *pid).collect();
    if termination.is_some() || infrastructure.is_some() {
        job.terminate()?;
    }
    let mut active = job.active()?;
    while active != 0 && Instant::now() < shutdown_deadline {
        thread::sleep(Duration::from_millis(2));
        active = job.active()?;
    }
    if active != 0 {
        cancel(&stdout);
        cancel(&stderr);
        cancel(&stdin);
        return Err(error(
            "cleanup_failed",
            format!("Job still has {active} active members"),
        ));
    }
    // Accounting can reach zero before the root process handle becomes signalled.
    // Reap it within the same shutdown budget instead of treating that race as failure.
    let remaining = shutdown_deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min(u32::MAX as u128) as u32;
    let wait = unsafe { WaitForSingleObject(root.process.0, remaining) };
    if wait != WAIT_OBJECT_0 {
        return Err(error(
            "cleanup_failed",
            "Root status unavailable after job emptied",
        ));
    }
    root.reaped = true;
    let mut residual_processes = Vec::new();
    for (pid, member) in &members {
        let remaining = shutdown_deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(u32::MAX as u128) as u32;
        if unsafe { WaitForSingleObject(member.0, remaining) } != WAIT_OBJECT_0 {
            residual_processes.push(*pid);
        }
    }
    if !residual_processes.is_empty() {
        return Err(error(
            "cleanup_failed",
            format!("Residual job processes: {residual_processes:?}"),
        ));
    }
    while !(stdout.is_finished() && stderr.is_finished() && stdin.is_finished())
        && Instant::now() < shutdown_deadline
    {
        // Job membership is zero, so retry cancellation to close the race with workers
        // entering a synchronous read/write after an earlier cancellation request.
        if termination.is_some() || infrastructure.is_some() {
            cancel(&stdout);
            cancel(&stderr);
            cancel(&stdin);
        }
        thread::sleep(Duration::from_millis(2));
    }
    if !(stdout.is_finished() && stderr.is_finished() && stdin.is_finished()) {
        cancel(&stdout);
        cancel(&stderr);
        cancel(&stdin);
        return Err(error(
            "cleanup_failed",
            "Pipe drain exceeded shutdown budget",
        ));
    }
    let stdout = stdout
        .join()
        .map_err(|_| error("pipe_failed", "stdout worker panicked"))?;
    let stderr = stderr
        .join()
        .map_err(|_| error("pipe_failed", "stderr worker panicked"))?;
    let stdin = stdin
        .join()
        .map_err(|_| error("pipe_failed", "stdin worker panicked"))?;
    if let Some(error) = infrastructure {
        return Err(error);
    }
    for failure in [stdout.error, stderr.error].into_iter().flatten() {
        if termination.is_none() || failure.raw_os_error() != Some(995) {
            return Err(error("pipe_failed", failure));
        }
    }
    if let Some(failure) = &stdin.error
        && failure.kind() != std::io::ErrorKind::BrokenPipe
        && !(termination.is_some() && failure.raw_os_error() == Some(995))
    {
        return Err(error("pipe_failed", failure));
    }
    if termination.is_none() && (stdout.capture.truncated || stderr.capture.truncated) {
        termination = Some(TerminationReason::OutputLimit);
    }
    let mut status = 0;
    unsafe { GetExitCodeProcess(root.process.0, &mut status) }
        .map_err(|e| error("pipe_failed", e))?;
    Ok(Observation {
        raw_exit_code: Some(status),
        termination,
        stdout: stdout.capture,
        stderr: stderr.capture,
        supplied_bytes: request.stdin.len(),
        written_bytes: stdin.written,
        stdin_incomplete: stdin.written < request.stdin.len(),
        active_members_after_cleanup: active,
        pid: info.dwProcessId,
        duration: started.elapsed(),
        cleanup_duration: cleanup.elapsed(),
        member_pids_at_shutdown,
        residual_processes,
    })
}
pub fn handle_count() -> Result<u32, ProcessError> {
    let mut count = 0;
    unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) }
        .map_err(|e| error("cleanup_failed", e))?;
    Ok(count)
}

pub fn process_alive(pid: u32) -> Result<bool, ProcessError> {
    let process = match unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            false,
            pid,
        )
    } {
        Ok(handle) => Handle(handle),
        Err(e) if e.code().0 as u32 == 0x80070057 => return Ok(false),
        Err(e) => return Err(error("cleanup_failed", e)),
    };
    match unsafe { WaitForSingleObject(process.0, 0) } {
        WAIT_TIMEOUT => Ok(true),
        WAIT_OBJECT_0 => Ok(false),
        _ => Err(error("cleanup_failed", "Residual process query failed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quote_edges() {
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("a b"), "\"a b\"");
        assert_eq!(quote("a\\"), "\"a\\\\\"");
        assert_eq!(quote("a\"b"), "\"a\\\"b\"");
    }
}
