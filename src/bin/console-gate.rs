#[cfg(not(windows))]
fn main() {
    eprintln!("The console gate requires Windows.");
    std::process::exit(3);
}
#[cfg(windows)]
fn main() {
    if let Err(error) = windows_gate::execute() {
        let args = std::env::args().collect::<Vec<_>>();
        if let Some(index) = args.iter().position(|a| a == "--evidence")
            && let Some(path) = args.get(index + 1)
        {
            let _ = std::fs::write(
                std::path::Path::new(path).with_extension("error.txt"),
                error.to_string(),
            );
        }
        eprintln!("{error}");
        std::process::exit(3);
    }
}
#[cfg(windows)]
mod windows_gate {
    use clap::Parser;
    use std::{
        fs,
        io::Write,
        path::PathBuf,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    use windows::Win32::System::Console::{
        AllocConsole, CTRL_C_EVENT, FreeConsole, GenerateConsoleCtrlEvent, GetConsoleWindow,
        SetConsoleCtrlHandler,
    };
    use windows::core::BOOL;
    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(window: *mut std::ffi::c_void, command: i32) -> i32;
    }
    unsafe extern "system" fn ignore(_: u32) -> BOOL {
        BOOL(1)
    }
    #[derive(Parser)]
    struct Options {
        #[arg(long)]
        runner: PathBuf,
        #[arg(long)]
        fixture: PathBuf,
        #[arg(long)]
        evidence: PathBuf,
        #[arg(long, default_value_t = 100)]
        repeat: usize,
    }
    pub fn execute() -> Result<(), Box<dyn std::error::Error>> {
        let options = Options::parse();
        if !(1..=1000).contains(&options.repeat) {
            return Err("repeat must be 1..1000".into());
        }
        let runner = options.runner.canonicalize()?;
        let fixture = options.fixture.canonicalize()?;
        let parent = options
            .evidence
            .parent()
            .ok_or("Evidence parent missing")?
            .canonicalize()?;
        // Own an isolated console. Never broadcast to the user's terminal.
        unsafe {
            let _ = FreeConsole();
            AllocConsole()?;
            ShowWindow(GetConsoleWindow().0, 0);
            SetConsoleCtrlHandler(Some(ignore), true)?;
            SetConsoleCtrlHandler(None, false)?;
        }
        let mut records = vec![];
        for repetition in 1..=options.repeat {
            let scratch = tempfile::tempdir_in(&parent)?;
            let marker = scratch.path().join("ready");
            let suite = scratch.path().join("suite.toml");
            let report = scratch.path().join("report.json");
            fs::write(
                &suite,
                format!(
                    "schema_version=1\nsuite_id='console'\nprogram='{}'\n[[cases]]\nid='tree'\nargs=['tree','2','{}']\nexpect={{exit_code=0}}\n[[cases]]\nid='skipped'\nargs=['exit','0']\nexpect={{exit_code=0}}\n",
                    fixture.to_string_lossy().trim_start_matches("\\\\?\\"),
                    marker.display()
                ),
            )?;
            let mut child = Command::new(&runner)
                .env("SPANFORGE_VERIFY_WORK_ROOT", scratch.path())
                .args(["run", "--file"])
                .arg(&suite)
                .arg("--json")
                .arg(&report)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
            let readiness = Instant::now() + Duration::from_secs(5);
            while !marker.exists() && Instant::now() < readiness {
                if child.try_wait()?.is_some() {
                    return Err("Runner exited before tree readiness".into());
                }
                thread::sleep(Duration::from_millis(2));
            }
            if !marker.exists() {
                child.kill()?;
                child.wait()?;
                return Err("Tree readiness exceeded 5 seconds".into());
            }
            let started = Instant::now();
            unsafe {
                GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0)?;
            }
            let limit = started + Duration::from_secs(7);
            let exit = loop {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                if Instant::now() >= limit {
                    child.kill()?;
                    child.wait()?;
                    return Err("Console cancellation exceeded cleanup budget".into());
                }
                thread::sleep(Duration::from_millis(2));
            };
            let result: spanforge_verify::model::RunResult =
                serde_json::from_slice(&fs::read(report)?)?;
            let pids = (0..=2)
                .map(|depth| {
                    fs::read_to_string(marker.with_extension(format!("pid-{depth}")))
                        .and_then(|s| s.parse::<u32>().map_err(std::io::Error::other))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let residual = pids
                .iter()
                .copied()
                .filter(|pid| {
                    spanforge_verify::process_windows::process_alive(*pid).unwrap_or(true)
                })
                .collect::<Vec<_>>();
            let passed = exit.code() == Some(4)
                && matches!(
                    result.cases[0].termination_reason,
                    Some(spanforge_verify::model::TerminationReason::Cancelled)
                )
                && result.cases[1].reason_code.as_deref() == Some("not_run_after_abort")
                && residual.is_empty();
            records.push(serde_json::json!({"repetition":repetition,"passed":passed,"elapsed_ms":started.elapsed().as_millis(),"exit_code":exit.code(),"residual_processes":residual}));
            if !passed {
                return Err(format!(
                    "Console cancellation failed in repetition {repetition}: {records:?}"
                )
                .into());
            }
        }
        unsafe {
            FreeConsole()?;
        }
        let evidence = serde_json::json!({"passed":true,"event":"CTRL_C_EVENT","repeat":options.repeat,"os":spanforge_verify::reports::os_build(),"records":records});
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(options.evidence)?;
        file.write_all(&serde_json::to_vec_pretty(&evidence)?)?;
        file.sync_all()?;
        Ok(())
    }
}
