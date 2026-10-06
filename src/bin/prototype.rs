#[cfg(windows)]
fn main() {
    use clap::Parser;
    use spanforge_verify::{
        process_windows::{self, Request},
        prototype_gate,
    };
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    #[derive(Parser)]
    struct Options {
        #[arg(long)]
        fixture: PathBuf,
        #[arg(long, default_value_t = 100)]
        repeat: usize,
        #[arg(long)]
        parent_job: bool,
        #[arg(long)]
        evidence: Option<PathBuf>,
    }
    let options = Options::parse();
    let cancelled = Arc::new(AtomicBool::new(false));
    let signal = cancelled.clone();
    ctrlc::set_handler(move || signal.store(true, Ordering::Release)).expect("Ctrl-C handler");
    let fixture = std::fs::canonicalize(&options.fixture).expect("fixture path");
    if options.parent_job {
        let executable = std::env::current_exe().unwrap();
        let cwd = std::env::current_dir().unwrap();
        let args = vec![
            "--fixture".into(),
            fixture.to_string_lossy().into(),
            "--repeat".into(),
            options.repeat.to_string(),
        ];
        let env: BTreeMap<String, String> = std::env::vars().collect();
        let request = Request {
            executable: &executable,
            args: &args,
            cwd: &cwd,
            env: &env,
            stdin: &[],
            max_output_bytes: 16 * 1024 * 1024,
            timeout: Duration::from_secs(1800),
            run_deadline: Instant::now() + Duration::from_secs(1800),
            cancelled,
        };
        match process_windows::prototype_parent_job(&request) {
            Ok(observation) => {
                use std::io::Write;
                if let Some(path) = &options.evidence {
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .expect("new evidence file");
                    file.write_all(&observation.stdout.bytes)
                        .expect("evidence write");
                }
                if options.evidence.is_none() {
                    std::io::stdout()
                        .write_all(&observation.stdout.bytes)
                        .unwrap();
                }
                std::io::stderr()
                    .write_all(&observation.stderr.bytes)
                    .unwrap();
                if observation.termination.is_some() {
                    eprintln!("Parent job aborted: {:?}", observation.termination);
                    std::process::exit(3);
                }
                std::process::exit(observation.raw_exit_code.unwrap_or(3) as i32);
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(3);
            }
        }
    }
    let evidence = prototype_gate::run_with_cancellation(&fixture, options.repeat, &cancelled)
        .expect("prototype execution");
    let passed = evidence.iter().all(|record| record.passed);
    let json = serde_json::to_string_pretty(&evidence).unwrap();
    if let Some(path) = &options.evidence {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .expect("new evidence file");
        file.write_all(json.as_bytes()).expect("evidence write");
    } else {
        println!("{json}");
    }
    if cancelled.load(Ordering::Acquire) {
        std::process::exit(4);
    }
    std::process::exit(if passed { 0 } else { 1 });
}
#[cfg(not(windows))]
fn main() {
    eprintln!("The lifecycle prototype requires Windows x64");
    std::process::exit(3);
}
