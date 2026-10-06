#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("The Linux lifecycle gate requires Linux.");
    std::process::exit(3);
}
#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = linux::execute() {
        eprintln!("{error}");
        std::process::exit(3);
    }
}
#[cfg(target_os = "linux")]
mod linux {
    use clap::Parser;
    use spanforge_verify::{
        model::TerminationReason as T,
        process_linux::{self, Request},
    };
    use std::{
        collections::BTreeMap,
        fs,
        io::Write,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };
    #[derive(Parser)]
    struct Options {
        #[arg(long)]
        fixture: PathBuf,
        #[arg(long, default_value_t = 100)]
        repeat: usize,
        #[arg(long)]
        evidence: PathBuf,
    }
    pub fn execute() -> Result<(), Box<dyn std::error::Error>> {
        let options = Options::parse();
        if !(1..=1000).contains(&options.repeat) {
            return Err("repeat must be 1..1000".into());
        }
        let fixture = options.fixture.canonicalize()?;
        let executable_file = fs::File::open(&fixture)?;
        let scratch = tempfile::tempdir()?;
        let mut env = BTreeMap::from([
            ("PATH".into(), String::new()),
            ("HOME".into(), scratch.path().to_string_lossy().into_owned()),
            (
                "TMPDIR".into(),
                scratch.path().to_string_lossy().into_owned(),
            ),
        ]);
        for name in [
            "XDG_CONFIG_HOME",
            "XDG_DATA_HOME",
            "XDG_CACHE_HOME",
            "XDG_STATE_HOME",
        ] {
            env.insert(name.into(), scratch.path().to_string_lossy().into_owned());
        }
        let mut records = vec![];
        for repetition in 0..=options.repeat {
            for scenario in [
                "exit",
                "argv",
                "stdin",
                "binary",
                "boundary",
                "overflow",
                "early_stdin",
                "blocked_stdin",
                "timeout_tree",
                "cancel_tree",
                "run_deadline",
                "linger",
                "escape",
                "signal",
            ] {
                let marker = scratch.path().join("ready");
                let _ = fs::remove_file(&marker);
                let mut args = vec![];
                let mut stdin = vec![];
                let limit = 1024;
                let mut timeout = Duration::from_secs(5);
                let mut run_deadline = Instant::now() + Duration::from_secs(10);
                match scenario {
                    "exit" => args.extend(["exit".into(), "37".into()]),
                    "argv" => args.extend([
                        "argv".into(),
                        "".into(),
                        "a b".into(),
                        "a\"b\\".into(),
                        "தமிழ் 😀".into(),
                    ]),
                    "stdin" => {
                        args.push("stdin".into());
                        stdin = b"hello\0world".to_vec();
                    }
                    "binary" => args.push("binary".into()),
                    "boundary" | "overflow" => args.extend([
                        "flood".into(),
                        if scenario == "boundary" {
                            "1024"
                        } else {
                            "1025"
                        }
                        .into(),
                    ]),
                    "early_stdin" => {
                        args.push("early-stdin".into());
                        stdin = vec![b'x'; 1024 * 1024];
                    }
                    "blocked_stdin" => {
                        args.extend(["sleep".into(), "60000".into()]);
                        stdin = vec![b'x'; 1024 * 1024];
                        timeout = Duration::from_millis(100);
                    }
                    "timeout_tree" => {
                        args.extend(["tree".into(), "2".into()]);
                        timeout = Duration::from_millis(500);
                    }
                    "cancel_tree" => args.extend([
                        "tree".into(),
                        "2".into(),
                        marker.to_string_lossy().into_owned(),
                    ]),
                    "run_deadline" => {
                        args.extend(["tree".into(), "2".into()]);
                        run_deadline = Instant::now() + Duration::from_millis(100);
                    }
                    other => args.push(other.into()),
                }
                let cancelled = Arc::new(AtomicBool::new(false));
                let worker = if scenario == "cancel_tree" {
                    let flag = cancelled.clone();
                    Some(thread::spawn(move || {
                        let deadline = Instant::now() + Duration::from_secs(3);
                        while !marker.exists() && Instant::now() < deadline {
                            thread::sleep(Duration::from_millis(2));
                        }
                        if marker.exists() {
                            flag.store(true, Ordering::Release);
                        }
                    }))
                } else {
                    None
                };
                let before = fs::read_dir("/proc/self/fd")?.count();
                let started = Instant::now();
                let observed = process_linux::run(&Request {
                    executable: &fixture,
                    executable_file: &executable_file,
                    args: &args,
                    cwd: scratch.path(),
                    env: &env,
                    stdin: &stdin,
                    max_output_bytes: limit,
                    timeout,
                    run_deadline,
                    cancelled,
                })?;
                if let Some(worker) = worker {
                    worker.join().map_err(|_| "Cancellation worker panicked")?;
                }
                let elapsed = started.elapsed();
                let after = fs::read_dir("/proc/self/fd")?.count();
                let term = observed.termination.as_ref();
                let passed = before == after
                    && match scenario {
                        "exit" => term.is_none() && observed.raw_exit_code == Some(37),
                        "argv" => {
                            term.is_none()
                                && observed.stdout.bytes
                                    == args[1..]
                                        .iter()
                                        .map(|a| format!("{}:{a}\n", a.len()))
                                        .collect::<String>()
                                        .as_bytes()
                        }
                        "stdin" => {
                            term.is_none()
                                && observed.stdout.bytes == stdin
                                && observed.written_bytes == stdin.len()
                        }
                        "binary" => {
                            term.is_none()
                                && observed.stdout.bytes == [0, 255, 13, 10]
                                && observed.stderr.bytes == b"stderr\n"
                        }
                        "boundary" => {
                            term.is_none()
                                && observed.stdout.bytes.len() == limit
                                && observed.stderr.bytes.len() == limit
                        }
                        "overflow" => {
                            matches!(term, Some(T::OutputLimit))
                                && (observed.stdout.truncated || observed.stderr.truncated)
                        }
                        "early_stdin" => term.is_none() && observed.stdin_incomplete,
                        "blocked_stdin" | "timeout_tree" => matches!(term, Some(T::Timeout)),
                        "cancel_tree" => matches!(term, Some(T::Cancelled)),
                        "run_deadline" => matches!(term, Some(T::RunDeadline)),
                        "linger" | "escape" => matches!(term, Some(T::LingeringDescendants)),
                        "signal" => {
                            matches!(term, Some(T::Signal)) && observed.raw_exit_code.is_none()
                        }
                        _ => false,
                    };
                records.push(serde_json::json!({"repetition":repetition,"scenario":scenario,"passed":passed,"duration_ms":elapsed.as_millis(),"fd_before":before,"fd_after":after,"raw_exit_code":observed.raw_exit_code,"termination":observed.termination}));
                if !passed {
                    let data = serde_json::json!({"passed":false,"records":records});
                    std::fs::write(&options.evidence, serde_json::to_vec_pretty(&data)?)?;
                    return Err(format!("{scenario} failed in repetition {repetition}").into());
                }
            }
        }
        let data = serde_json::json!({"passed":true,"os":spanforge_verify::reports::os_build(),"arch":std::env::consts::ARCH,"repeat":options.repeat,"scenarios":14,"uid":unsafe{libc::geteuid()},"records":records});
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(options.evidence)?;
        file.write_all(&serde_json::to_vec_pretty(&data)?)?;
        file.sync_all()?;
        println!(
            "Linux lifecycle gate passed: {} measured experiments",
            options.repeat * 14
        );
        Ok(())
    }
}
