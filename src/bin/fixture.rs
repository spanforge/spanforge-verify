//! Trusted, deliberately controllable executable for prototype tests.
use std::{
    io::{self, Read, Write},
    process, thread,
    time::Duration,
};
#[allow(clippy::zombie_processes)] // Deliberate live descendants test Job Object cleanup.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("agent-task") => {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes).unwrap();
            let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            match args[1].as_str() {
                "good" => std::fs::write("answer.txt", b"correct").unwrap(),
                "alternate" => std::fs::write("answer.txt", b"alternate").unwrap(),
                "wrong" => std::fs::write("answer.txt", b"wrong").unwrap(),
                "tamper" => {
                    let _ = std::fs::write(&args[2], b"wrong");
                }
                _ => (),
            }
            let mut reply = serde_json::json!({"schema_version":1,"run_id":request["run_id"],"attempt_id":request["attempt_id"],"response":"Task complete; tests passed","claims":["completed"]});
            if args[1] == "forge" {
                reply["run_id"] = "forged".into();
            }
            if request["protocol"] == "jsonl" {
                let started = serde_json::json!({"schema_version":1,"run_id":request["run_id"],"attempt_id":request["attempt_id"],"sequence":0,"event_id":"start","kind":"started"});
                let mut tool = serde_json::json!({"schema_version":1,"run_id":request["run_id"],"attempt_id":request["attempt_id"],"sequence":1,"event_id":"tool-call","kind":"tool","name":"write_file","summary":args.get(2).cloned().unwrap_or_else(|| "Reported write attempt".into())});
                let mut final_event = reply;
                final_event["sequence"] = 2.into();
                final_event["event_id"] = "finish".into();
                final_event["kind"] = "final".into();
                match args[1].as_str() {
                    "event-gap" => tool["sequence"] = 3.into(),
                    "event-duplicate" => tool["event_id"] = "start".into(),
                    "event-spoof" => tool["source"] = "gateway_observed".into(),
                    _ => (),
                }
                println!("{started}");
                io::stdout().flush().unwrap();
                if args[1] == "event-cancel" {
                    std::fs::write(&args[2], b"ready").unwrap();
                    thread::sleep(Duration::from_secs(60));
                }
                if args[1] == "event-timeout" {
                    thread::sleep(Duration::from_secs(60));
                }
                println!("{tool}");
                if args[1] == "event-no-final" {
                    return;
                }
                println!("{final_event}");
                if args[1] == "event-after-final" {
                    println!("{tool}");
                }
            } else {
                println!("{reply}");
            }
        }
        Some("verify-task") => {
            let mut bytes = Vec::new();
            io::stdin().read_to_end(&mut bytes).unwrap();
            let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let root = std::path::Path::new(request["workspace"].as_str().unwrap());
            if args[1] == "count-check" {
                let count = std::fs::read_to_string(&args[3])
                    .ok()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                std::fs::write(&args[3], (count + 1).to_string()).unwrap();
            }
            match args[1].as_str() {
                "crash" => process::exit(37),
                "malformed" => {
                    print!(
                        "{{\"schema_version\":1,\"status\":\"PASS\",\"status\":\"FAIL\",\"summary\":\"bad\"}}"
                    );
                    return;
                }
                "timeout" => thread::sleep(Duration::from_secs(60)),
                "mutate" => {
                    std::fs::write(root.join("answer.txt"), b"changed-by-verifier").unwrap()
                }
                "overflow" => {
                    print!("{}", "x".repeat(65536));
                    return;
                }
                _ => (),
            }
            let accepted = std::fs::read_to_string(&args[2]).unwrap();
            let answer = std::fs::read_to_string(root.join("answer.txt")).unwrap_or_default();
            let status = if args[1] == "always-pass" {
                "PASS"
            } else if args[1] == "always-fail" {
                "FAIL"
            } else if args[1] == "flaky" {
                let count = std::fs::read_to_string(&args[3])
                    .ok()
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                std::fs::write(&args[3], (count + 1).to_string()).unwrap();
                if count.is_multiple_of(2) {
                    "PASS"
                } else {
                    "FAIL"
                }
            } else if args[1] == "unknown" {
                "INCONCLUSIVE"
            } else if accepted.lines().any(|s| s == answer) {
                "PASS"
            } else {
                "FAIL"
            };
            let summary = args.get(3).cloned().unwrap_or_else(|| {
                "Checked persisted answer against pinned expected alternatives".into()
            });
            println!(
                "{}",
                serde_json::json!({"schema_version":1,"status":status,"summary":summary})
            );
        }
        Some("http-get" | "http-url") => {
            use std::net::TcpStream;
            let url = if args[0] == "http-url" {
                args[1].clone()
            } else {
                std::env::var("SPANFORGE_VERIFY_HTTP_URL")
                    .or_else(|_| std::env::var("CLIVERIFYR_HTTP_URL"))
                    .unwrap()
            };
            let mut stream = TcpStream::connect(url.strip_prefix("http://").unwrap()).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let path = args
                .get(if args[0] == "http-url" { 2 } else { 1 })
                .map(String::as_str)
                .unwrap_or("/health");
            write!(
                stream,
                "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            print!("{response}");
        }
        Some("comparison-profile") => {
            // Read-only sidecar allows two real native executable installations
            // to exercise different release behavior with identical case inputs.
            let path = std::env::current_exe()
                .unwrap()
                .with_extension("profile.json");
            let bytes = std::fs::read(path).unwrap();
            let profile: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            if let Some(marker) = profile
                .get("launch_marker")
                .and_then(serde_json::Value::as_str)
            {
                std::fs::write(marker, b"launched").unwrap();
            }
            if let Some(path) = profile
                .get("mutate_source")
                .and_then(serde_json::Value::as_str)
            {
                std::fs::write(path, b"mutated").unwrap();
            }
            for (key, stream) in [("stdout", 1), ("stderr", 2)] {
                if let Some(text) = profile.get(key).and_then(serde_json::Value::as_str) {
                    if stream == 1 {
                        io::stdout().write_all(text.as_bytes()).unwrap();
                    } else {
                        io::stderr().write_all(text.as_bytes()).unwrap();
                    }
                }
            }
            if let Some(text) = profile.get("file").and_then(serde_json::Value::as_str) {
                std::fs::write("result.txt", text).unwrap();
            }
            if profile
                .get("echo_stdin")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                let mut bytes = vec![];
                io::stdin().read_to_end(&mut bytes).unwrap();
                io::stdout().write_all(&bytes).unwrap();
            }
            if let Some(ms) = profile.get("sleep_ms").and_then(serde_json::Value::as_u64) {
                thread::sleep(Duration::from_millis(ms));
            }
            process::exit(
                profile
                    .get("exit")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0) as i32,
            );
        }
        Some("touch") => {
            std::fs::write(args.get(1).expect("marker path"), b"resumed").unwrap();
        }
        Some("write") => std::fs::write(args.get(1).unwrap(), args.get(2).unwrap()).unwrap(),
        Some("read") => {
            io::stdout()
                .write_all(&std::fs::read(args.get(1).unwrap()).unwrap())
                .unwrap();
        }
        Some("pid") => println!("{}", process::id()),
        Some("alternating") => {
            let path = args.get(1).unwrap();
            let count = std::fs::read_to_string(path)
                .ok()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0)
                + 1;
            std::fs::write(path, count.to_string()).unwrap();
            println!("attempt {count}");
            process::exit(if count % 2 == 1 { 1 } else { 0 });
        }
        Some("remove") => std::fs::remove_file(args.get(1).unwrap()).unwrap(),
        Some("mkdir") => std::fs::create_dir_all(args.get(1).unwrap()).unwrap(),
        Some("replace-directory") => {
            let path = args.get(1).unwrap();
            std::fs::remove_file(path).unwrap();
            std::fs::create_dir(path).unwrap();
        }
        #[cfg(target_os = "linux")]
        Some("symlink") => {
            std::os::unix::fs::symlink(args.get(1).unwrap(), args.get(2).unwrap()).unwrap()
        }
        #[cfg(windows)]
        Some("junction") => junction(args.get(1).unwrap(), args.get(2).unwrap()).unwrap(),
        Some("early-stdin") => {
            #[cfg(target_os = "linux")]
            unsafe {
                libc::close(0);
            }
            #[cfg(windows)]
            unsafe {
                use windows::Win32::{
                    Foundation::CloseHandle,
                    System::Console::{GetStdHandle, STD_INPUT_HANDLE},
                };
                let _ = CloseHandle(GetStdHandle(STD_INPUT_HANDLE).unwrap());
            }
            thread::sleep(Duration::from_millis(100));
        }
        Some("tree") => {
            let depth: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2);
            if let Some(marker) = args.get(2) {
                std::fs::write(
                    std::path::Path::new(marker).with_extension(format!("pid-{depth}")),
                    process::id().to_string(),
                )
                .unwrap();
            }
            if depth > 0 {
                let mut command = process::Command::new(std::env::current_exe().unwrap());
                command.args(["tree", &(depth - 1).to_string()]);
                if let Some(marker) = args.get(2) {
                    command.arg(marker);
                }
                let child = command.spawn().unwrap();
                println!("pid:{}", child.id());
                io::stdout().flush().unwrap();
            }
            if depth == 0
                && let Some(marker) = args.get(2)
            {
                std::fs::write(marker, b"grandchild-ready").unwrap();
            }
            thread::sleep(Duration::from_secs(60));
        }
        Some("linger") => {
            let child = process::Command::new(std::env::current_exe().unwrap())
                .args(["tree", "1"])
                .spawn()
                .unwrap();
            println!("pid:{}", child.id());
            io::stdout().flush().unwrap();
        }
        #[cfg(target_os = "linux")]
        Some("escape") => {
            use std::os::unix::process::CommandExt;
            let mut command = process::Command::new(std::env::current_exe().unwrap());
            command.args(["tree", "1"]);
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
            let child = command.spawn().unwrap();
            println!("pid:{}", child.id());
            io::stdout().flush().unwrap();
        }
        #[cfg(target_os = "linux")]
        Some("signal") => unsafe {
            libc::raise(libc::SIGTERM);
        },
        Some("env") => println!(
            "{}",
            std::env::var(args.get(1).unwrap()).unwrap_or_default()
        ),
        Some("exit") => process::exit(args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0)),
        Some("argv") => {
            for arg in &args[1..] {
                println!("{}:{arg}", arg.len());
            }
        }
        Some("stdin") => {
            let mut data = Vec::new();
            io::stdin().read_to_end(&mut data).unwrap();
            io::stdout().write_all(&data).unwrap();
        }
        Some("sleep") => thread::sleep(Duration::from_millis(
            args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60_000),
        )),
        Some("binary") => {
            io::stdout().write_all(&[0, 255, 13, 10]).unwrap();
            io::stderr().write_all(b"stderr\n").unwrap();
        }
        Some("flood") => {
            let count: usize = args
                .get(1)
                .and_then(|s| s.parse().ok())
                .unwrap_or(1024 * 1024);
            let child = thread::spawn(move || io::stderr().write_all(&vec![b'e'; count]).unwrap());
            io::stdout().write_all(&vec![b'o'; count]).unwrap();
            child.join().unwrap();
        }
        _ => {
            eprintln!("fixture modes: exit, argv, stdin, sleep, binary, flood");
            process::exit(2);
        }
    }
}
#[cfg(windows)]
fn junction(target: &str, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    use std::{os::windows::ffi::OsStrExt, path::Path};
    use windows::{
        Win32::{Foundation::CloseHandle, Storage::FileSystem::*, System::IO::DeviceIoControl},
        core::PCWSTR,
    };
    std::fs::create_dir(path)?;
    let target = Path::new(target)
        .canonicalize()?
        .to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .to_string();
    let substitute = format!("\\??\\{target}").encode_utf16().collect::<Vec<_>>();
    let print = target.encode_utf16().collect::<Vec<_>>();
    let mut buffer = vec![];
    let payload_bytes = ((substitute.len() + print.len() + 2) * 2) as u16;
    buffer.extend(0xA0000003u32.to_le_bytes());
    buffer.extend((8 + payload_bytes).to_le_bytes());
    buffer.extend(0u16.to_le_bytes());
    for value in [
        0u16,
        (substitute.len() * 2) as u16,
        ((substitute.len() + 1) * 2) as u16,
        (print.len() * 2) as u16,
    ] {
        buffer.extend(value.to_le_bytes());
    }
    for value in substitute.into_iter().chain([0]).chain(print).chain([0]) {
        buffer.extend(value.to_le_bytes());
    }
    let wide = std::ffi::OsStr::new(path)
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            0x40000000,
            FILE_SHARE_MODE(0),
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }?;
    let mut returned = 0;
    let result = unsafe {
        DeviceIoControl(
            handle,
            0x000900A4,
            Some(buffer.as_ptr().cast()),
            buffer.len() as u32,
            None,
            0,
            Some(&mut returned),
            None,
        )
    };
    unsafe {
        CloseHandle(handle)?;
    }
    result?;
    Ok(())
}
