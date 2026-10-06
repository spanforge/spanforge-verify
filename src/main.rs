use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, parser::ValueSource};
use std::{fs::OpenOptions, io::Write, path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(
    version,
    about = "SpanForge Verify: verify executable behavior against declarative contracts"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// Summarize every repeated attempt, including failures and output variability.
    Repeatability {
        #[arg(long)]
        report: PathBuf,
        #[arg(long)]
        fail_on_problems: bool,
    },
    /// Check bundle integrity, platform, secrets and target without executing it.
    Doctor {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        program: PathBuf,
    },
    /// Export immutable inputs for one reviewed case; never launches the target.
    Bundle {
        #[arg(long, default_value = "spanforge-verify.toml")]
        file: PathBuf,
        #[arg(long)]
        case: String,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        include_sensitive_inputs: bool,
    },
    /// Verify a trusted bundle and replay it with the original native executable.
    Replay {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        program: PathBuf,
    },
    /// Run two native executables against shared inputs and compare their content.
    Compare {
        #[arg(long, default_value = "spanforge-verify.toml")]
        file: PathBuf,
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        candidate: PathBuf,
        #[arg(long)]
        case: Option<String>,
        #[arg(long)]
        json_stdout: bool,
        #[arg(long)]
        json_stderr: bool,
        #[arg(long)]
        crlf_to_lf: bool,
        #[arg(long)]
        ignore_json_pointer: Vec<String>,
        #[arg(long)]
        json_file: Vec<String>,
        #[arg(long)]
        text_file: Vec<String>,
        #[arg(long)]
        policy: Option<PathBuf>,
    },
    /// Compare complete JSON contract reports; does not compare raw output.
    CompareReports {
        #[arg(long)]
        baseline: PathBuf,
        #[arg(long)]
        candidate: PathBuf,
    },
    Init {
        #[arg(long, default_value = "spanforge-verify.toml")]
        file: PathBuf,
        /// Generate a reviewed starter for this executable without launching it.
        #[arg(long)]
        program: Option<String>,
    },
    /// Report mappings and assertion gaps for explicitly declared behaviors.
    Coverage {
        #[arg(long, default_value = "spanforge-verify.toml")]
        file: PathBuf,
        #[arg(long)]
        fail_on_unmapped: bool,
    },
    Validate {
        #[arg(long, default_value = "spanforge-verify.toml")]
        file: PathBuf,
    },
    Run {
        #[arg(long, default_value = "spanforge-verify.toml")]
        file: PathBuf,
        #[arg(long)]
        case: Option<String>,
        #[arg(long)]
        json: Option<PathBuf>,
        #[arg(long)]
        junit: Option<PathBuf>,
        #[arg(long)]
        overwrite_reports: bool,
        #[arg(long)]
        bundle_on_failure: Option<PathBuf>,
    },
}
fn main() -> ExitCode {
    let invocation_started = std::time::Instant::now();
    let invocation_started_at = spanforge_verify::reports::utc_now();
    let matches = Cli::command().get_matches();
    let mut command = Cli::from_arg_matches(&matches)
        .expect("validated CLI arguments")
        .command;
    // Only implicit file defaults migrate. Explicit --file is always respected.
    let default_file = matches.subcommand().is_some_and(|(name, args)| {
        matches!(name, "bundle" | "compare" | "coverage" | "validate" | "run")
            && args.value_source("file") == Some(ValueSource::DefaultValue)
    });
    if default_file
        && !std::path::Path::new("spanforge-verify.toml").exists()
        && std::path::Path::new("cliverifyr.toml").exists()
    {
        match &mut command {
            Commands::Bundle { file, .. }
            | Commands::Compare { file, .. }
            | Commands::Coverage { file, .. }
            | Commands::Validate { file }
            | Commands::Run { file, .. } => *file = "cliverifyr.toml".into(),
            _ => unreachable!(),
        }
    }
    if let Commands::Repeatability {
        report,
        fail_on_problems,
    } = &command
    {
        return match spanforge_verify::comparison::read_report(report)
            .and_then(|r| spanforge_verify::repeatability::summarize(&r))
        {
            Ok(summary) => {
                let problems = summary.source_exit_code != 0
                    || summary.groups.is_empty()
                    || summary
                        .groups
                        .iter()
                        .any(|g| g.classification != "consistent_pass");
                match serde_json::to_string_pretty(&summary) {
                    Ok(json) => println!("{json}"),
                    Err(_) => return ExitCode::from(3),
                }
                ExitCode::from(u8::from(*fail_on_problems && problems))
            }
            Err(message) => {
                eprintln!("{}", spanforge_verify::privacy::terminal_escape(&message));
                ExitCode::from(2)
            }
        };
    }
    if let Commands::CompareReports {
        baseline,
        candidate,
    } = &command
    {
        let result = (|| {
            let baseline = spanforge_verify::comparison::read_report(baseline)?;
            let candidate = spanforge_verify::comparison::read_report(candidate)?;
            spanforge_verify::comparison::compare(&baseline, &candidate)
        })();
        return match result {
            Ok(comparison) => {
                match serde_json::to_string_pretty(&comparison) {
                    Ok(json) => println!("{json}"),
                    Err(_) => return ExitCode::from(3),
                }
                ExitCode::from(u8::from(!comparison.differences.is_empty()))
            }
            Err(message) => {
                eprintln!("{}", spanforge_verify::privacy::terminal_escape(&message));
                ExitCode::from(2)
            }
        };
    }
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if matches!(
        &command,
        Commands::Run { .. } | Commands::Compare { .. } | Commands::Replay { .. }
    ) {
        let flag = cancelled.clone();
        let handler: Result<(), String> = (|| {
            ctrlc::set_handler(move || flag.store(true, std::sync::atomic::Ordering::SeqCst))
                .map_err(|e| e.to_string())?;
            #[cfg(windows)]
            unsafe {
                windows::Win32::System::Console::SetConsoleCtrlHandler(None, false)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        })();
        if let Err(error) = handler {
            eprintln!("{}", spanforge_verify::privacy::terminal_escape(&error));
            return ExitCode::from(3);
        }
    }
    let replay_suite;
    let file = match &command {
        Commands::Init { file, .. }
        | Commands::Coverage { file, .. }
        | Commands::Validate { file }
        | Commands::Run { file, .. }
        | Commands::Compare { file, .. }
        | Commands::Bundle { file, .. } => file,
        Commands::Replay { bundle, .. } | Commands::Doctor { bundle, .. } => {
            replay_suite = spanforge_verify::reproduction::suite_path(bundle);
            &replay_suite
        }
        Commands::CompareReports { .. } | Commands::Repeatability { .. } => unreachable!(),
    };
    let masker = match spanforge_verify::privacy::suite_masker(file) {
        Ok(masker) => masker,
        Err(_) => {
            eprintln!("Masking configuration exceeds supported resources");
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Commands::Doctor {bundle,program} => (|| {
            let mut report = spanforge_verify::reproduction::doctor(&bundle,&program);
            for check in &mut report.checks {
                check.id = masker.diagnostic(&check.id).text;
                check.message = masker.diagnostic(&check.message).text;
            }
            let json = serde_json::to_vec_pretty(&report).map_err(|_| (3,"Cannot serialize environment diagnostics".into()))?;
            std::io::stdout().write_all(&json).and_then(|_| std::io::stdout().write_all(b"\n")).map_err(|_| (3,"Cannot write environment diagnostics".into()))?;
            if report.ready {Ok(())} else {Err((2,"Replay prerequisites are not ready; inspect doctor checks".into()))}
        })(),
        Commands::Bundle {file,case,out,include_sensitive_inputs} => {
            spanforge_verify::reproduction::create(&file,&case,&out,include_sensitive_inputs).map(|_| {
                println!("Bundle created. Replay with: spanforge-verify replay --bundle <bundle-directory> --program <original-executable>");
            })
        },
        Commands::Replay {bundle,program} => (|| {
            let report = spanforge_verify::reproduction::replay(&bundle,&program,cancelled,invocation_started,invocation_started_at)?;
            let json = serde_json::to_vec_pretty(&report).map_err(|_| (3,"Cannot serialize replay report".into()))?;
            if json.len() > 16*1024*1024 {return Err((3,"Replay report exceeds 16 MiB".into()));}
            std::io::stdout().write_all(&json).and_then(|_| std::io::stdout().write_all(b"\n")).map_err(|_| (3,"Cannot write replay report".into()))?;
            if report.exit_code == 0 {Ok(())} else {Err((report.exit_code,format!("Replay finished: {}",report.status)))}
        })(),
        Commands::Compare {
            file,
            baseline,
            candidate,
            case,
            json_stdout,
            json_stderr,
            crlf_to_lf,
            ignore_json_pointer,
            json_file,
            text_file,
            policy,
        } => (|| {
            let policy = policy
                .as_deref()
                .map(spanforge_verify::compatibility::Policy::read)
                .transpose()
                .map_err(|e| (2, e))?;
            let mut options = policy
                .as_ref()
                .map(|p| p.comparison.clone())
                .unwrap_or_default();
            options.json_stdout |= json_stdout;
            options.json_stderr |= json_stderr;
            options.crlf_to_lf |= crlf_to_lf;
            options.ignore_json_pointers.extend(ignore_json_pointer);
            options.ignore_json_pointers.sort();
            options.ignore_json_pointers.dedup();
            options.json_files.extend(json_file);
            options.text_files.extend(text_file);
            let report = spanforge_verify::compare_run::execute(spanforge_verify::compare_run::Request {
                file: &file,
                selected: case.as_deref(),
                baseline: &baseline,
                candidate: &candidate,
                options,
                policy,
                cancelled,
                started: invocation_started,
                started_at: invocation_started_at,
            })?;
            let json = serde_json::to_vec_pretty(&report)
                .map_err(|_| (3, "Cannot serialize comparison".into()))?;
            if json.len() > 16 * 1024 * 1024 {
                return Err((3, "Comparison report exceeds 16 MiB".into()));
            }
            std::io::stdout()
                .write_all(&json)
                .and_then(|_| std::io::stdout().write_all(b"\n"))
                .map_err(|_| (3, "Cannot write comparison report".into()))?;
            if report.exit_code == 0 {
                Ok(())
            } else {
                Err((
                    report.exit_code,
                    format!("Comparison finished: {}", report.status),
                ))
            }
        })(),
        Commands::CompareReports { .. } | Commands::Repeatability { .. } => unreachable!(),
        Commands::Coverage { file, fail_on_unmapped } => (|| {
            let suite = spanforge_verify::schema::validate(&file, None).map_err(|e| (2, e))?;
            let report = spanforge_verify::coverage::report(&suite);
            let json = serde_json::to_string_pretty(&report).map_err(|e| (3, e.to_string()))?;
            // Mask before writing, including IDs; never expose configuration secrets.
            let mut value: serde_json::Value = serde_json::from_str(&json).map_err(|e| (3,e.to_string()))?;
            fn mask_strings(value: &mut serde_json::Value, masker: &spanforge_verify::privacy::Masker) {
                match value {
                    serde_json::Value::String(s) => *s = masker.diagnostic(s).text,
                    serde_json::Value::Array(a) => a.iter_mut().for_each(|v| mask_strings(v,masker)),
                    serde_json::Value::Object(o) => o.values_mut().for_each(|v| mask_strings(v,masker)),
                    _ => {}
                }
            }
            mask_strings(&mut value, &masker);
            println!("{}",serde_json::to_string_pretty(&value).map_err(|e| (3,e.to_string()))?);
            if fail_on_unmapped && (report.declared_contracts == 0 || !report.unmapped_contracts.is_empty()) {Err((1,"Declared contract coverage has gaps".into()))} else {Ok(())}
        })(),
        Commands::Init { file, program } => (|| {
            let custom = program.map(|program| {
                let quoted = toml::Value::String(program).to_string();
                format!("# Review --help, exit code, and output expectations before running.\n# Add cases for invalid input, generated files, and failure paths.\nschema_version = 1\nsuite_id = 'my-cli'\nprogram = {quoted}\n\n[[contracts]]\nid = 'help'\ndescription = 'Help is available and describes usage'\ncases = ['help']\n\n[[contracts]]\nid = 'invalid-input'\ndescription = 'Invalid input fails with a useful diagnostic'\ncases = []\n\n[[cases]]\nid = 'help'\nargs = ['--help']\nexpect = {{ exit_code = 0 }}\nstdout = {{ mode = 'regex', pattern = '(?i)usage' }}\n")
            });
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(file)
                .map_err(|e| {
                    (
                        if e.kind() == std::io::ErrorKind::AlreadyExists {
                            2
                        } else {
                            3
                        },
                        e.to_string(),
                    )
                })?;
            #[cfg(windows)]
            let starter = include_bytes!("../examples/spanforge-verify.toml").as_slice();
            #[cfg(not(windows))]
            let starter = include_bytes!("../examples/spanforge-verify-linux.toml").as_slice();
            output.write_all(custom.as_ref().map(|s| s.as_bytes()).unwrap_or(starter)).map_err(|e| (3, e.to_string()))?;
            println!("Starter suite created; set program and approve its expectations.");
            Ok(())
        })(),
        Commands::Validate { file } => spanforge_verify::schema::validate(&file, None)
            .map(|suite| {
                println!(
                    "Valid suite {} ({} cases)",
                    spanforge_verify::privacy::terminal_escape(&masker.mask(&suite.suite_id)),
                    suite.cases.len()
                );
            })
            .map_err(|e| (2, e)),
        Commands::Run {
            file,
            case,
            json,
            junit,
            overwrite_reports,
            bundle_on_failure,
        } => (|| {
            let started = invocation_started;
            let started_at = invocation_started_at;
            let destinations = spanforge_verify::reports::Destinations::prepare(
                &file,
                json.as_deref(),
                junit.as_deref(),
                overwrite_reports,
            )
            .map_err(|e| (2, e))?;
            let plan = spanforge_verify::inputs::prepare(&file,case.as_deref());
            if let (Some(out),Ok(plan)) = (&bundle_on_failure,&plan) {
                spanforge_verify::reproduction::preflight_destination(plan,out).map_err(|e| (2,e))?;
            }
            let mut result = match &plan {
                Ok(plan) => spanforge_verify::runner::execute_plan_started(plan,cancelled,started,started_at,None),
                Err(message) => Err((if message == "input_changed" {3} else {2},message.clone())),
            }.unwrap_or_else(|(code, message)| spanforge_verify::reports::error_result(code, message));
            for error in &mut result.errors {
                error.message = masker.diagnostic(&error.message).text;
            }
            for case in &result.cases {
                println!(
                    "{} {}{}",
                    spanforge_verify::privacy::terminal_escape(&case.case_id),
                    case.status,
                    case.reason_code
                        .as_ref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default()
                );
                for assertion in case
                    .assertions
                    .iter()
                    .filter(|a| a.status == spanforge_verify::model::AssertionStatus::Fail)
                {
                    println!(
                        "  {}: {}",
                        spanforge_verify::privacy::terminal_escape(&assertion.check_id),
                        assertion
                            .reason_code
                            .as_deref()
                            .unwrap_or("assertion_failed")
                    );
                    for (label, summary) in [
                        ("expected", &assertion.expected_summary),
                        ("observed", &assertion.observed_summary),
                    ] {
                        if let Some(summary) = summary {
                            println!(
                                "    {label}: {}",
                                spanforge_verify::privacy::terminal_escape(summary)
                                    .replace('\n', "\n      ")
                            );
                        }
                    }
                }
            }
            for error in &result.errors {
                eprintln!("{}", spanforge_verify::privacy::terminal_escape(&error.message));
            }
            destinations
                .publish(&mut result)
                .map_err(|e| (3, format!("Report publication failed: {e}")))?;
            if let (Some(out),Ok(plan)) = (bundle_on_failure,plan.as_ref())
                && let Some((position,_)) = result.cases.iter().enumerate().find(|(_,case)| case.status != spanforge_verify::model::Status::Pass) {
                    let id = &plan.suite().cases[plan.cases()[position].index()].id;
                    spanforge_verify::reproduction::create_from_plan(plan,id,&out,false,Some(&result)).map_err(|(_,e)| (3,format!("Failure bundle creation failed: {e}")))?;
                    eprintln!("Failure bundle created from the run's immutable inputs.");
            }
            if result.exit_code == 0 {
                Ok(())
            } else {
                Err((
                    result.exit_code,
                    format!("Suite finished: {}", result.status),
                ))
            }
        })(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, message)) => {
            eprintln!(
                "{}",
                spanforge_verify::privacy::terminal_escape(&masker.diagnostic(&message).text)
            );
            ExitCode::from(code)
        }
    }
}
