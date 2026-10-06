//! Bounded offline HTTP/1.1 fixtures. One fresh loopback listener per case.
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub url_env: String,
    pub responses: Vec<Response>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub method: String,
    pub path: String,
    pub status: u16,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub delay_ms: u64,
    pub location: Option<String>,
}
impl Fixture {
    pub fn validate(&self) -> Result<(), String> {
        let valid_prefix = ["SPANFORGE_VERIFY_HTTP_", "CLIVERIFYR_HTTP_"]
            .iter()
            .any(|prefix| self.url_env.starts_with(prefix) && self.url_env.len() > prefix.len());
        if !valid_prefix
            || self.url_env.len() > 64
            || !self
                .url_env
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err("HTTP url_env must be an uppercase SPANFORGE_VERIFY_HTTP_ name (legacy CLIVERIFYR_HTTP_ accepted)".into());
        }
        if self.responses.is_empty() || self.responses.len() > 64 {
            return Err("HTTP fixtures require 1..64 ordered responses".into());
        }
        for r in &self.responses {
            if !["GET", "HEAD"].contains(&r.method.as_str())
                || !r.path.starts_with('/')
                || r.path.bytes().any(|b| b <= 32 || b >= 127)
                || r.path.len() > 4096
                || !(200..=599).contains(&r.status)
                || r.body.len() > 65536
                || r.delay_ms > 60000
                || r.location
                    .as_ref()
                    .is_some_and(|s| s.len() > 4096 || s.bytes().any(|b| !(32..127).contains(&b)))
            {
                return Err(
                    "Invalid or oversized HTTP response; only GET/HEAD are supported".into(),
                );
            }
        }
        Ok(())
    }
}
pub struct Server {
    pub url: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<(), String>>>,
}
impl Server {
    pub fn start(fixture: &Fixture, deadline: Instant) -> Result<Self, String> {
        fixture.validate()?;
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let url = format!(
            "http://{}",
            listener.local_addr().map_err(|e| e.to_string())?
        );
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let fixture = fixture.clone();
        let worker = thread::spawn(move || serve(listener, &fixture, &flag, deadline));
        Ok(Self {
            url,
            stop,
            worker: Some(worker),
        })
    }
    pub fn finish(mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::SeqCst);
        self.worker
            .take()
            .unwrap()
            .join()
            .map_err(|_| "HTTP fixture worker panicked".to_string())?
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn live(stop: &AtomicBool, deadline: Instant) -> bool {
    !stop.load(Ordering::SeqCst) && Instant::now() < deadline
}
fn serve(
    listener: TcpListener,
    fixture: &Fixture,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<(), String> {
    let mut seen = 0;
    let mut failed = false;
    while live(stop, deadline) {
        match listener.accept() {
            Ok((mut stream, _)) => match request(&mut stream, stop, deadline) {
                Ok(line) => {
                    let fields: Vec<_> = line.split_whitespace().collect();
                    if let Some(response) = fixture.responses.get(seen) {
                        seen += 1;
                        if fields.len() != 3
                            || fields[0] != response.method
                            || fields[1] != response.path
                            || fields[2] != "HTTP/1.1"
                        {
                            failed = true;
                        }
                        let until = Instant::now() + Duration::from_millis(response.delay_ms);
                        while live(stop, deadline) && Instant::now() < until {
                            thread::sleep(Duration::from_millis(2));
                        }
                        if live(stop, deadline) {
                            let location = response
                                .location
                                .as_ref()
                                .map(|s| format!("Location: {s}\r\n"))
                                .unwrap_or_default();
                            let body = if response.method == "HEAD" {
                                ""
                            } else {
                                &response.body
                            };
                            let wire = format!(
                                "HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                                response.status,
                                response.body.len(),
                                location,
                                body
                            );
                            stream
                                .set_write_timeout(Some(Duration::from_millis(50)))
                                .map_err(|e| e.to_string())?;
                            if stream.write_all(wire.as_bytes()).is_err() {
                                failed = true;
                            }
                        }
                    } else {
                        failed = true;
                        let _ = stream.write_all(b"HTTP/1.1 500 Unexpected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
                Err(_) => failed = true,
            },
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2))
            }
            Err(e) => return Err(e.to_string()),
        }
    }
    if failed || seen != fixture.responses.len() {
        Err(format!(
            "HTTP request contract failed: received {seen} of {} expected requests",
            fixture.responses.len()
        ))
    } else {
        Ok(())
    }
}
fn request(stream: &mut TcpStream, stop: &AtomicBool, deadline: Instant) -> Result<String, String> {
    stream
        .set_write_timeout(Some(Duration::from_millis(50)))
        .map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .map_err(|e| e.to_string())?;
    let connection_deadline = deadline.min(Instant::now() + Duration::from_millis(1000));
    let mut bytes = Vec::new();
    while live(stop, connection_deadline) && bytes.len() < 16384 {
        let mut buffer = [0; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                bytes.extend_from_slice(&buffer[..n]);
                if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                    return String::from_utf8(bytes)
                        .map_err(|_| "Non-UTF8 HTTP request".into())
                        .map(|s| s.lines().next().unwrap_or_default().to_owned());
                }
            }
            Err(e)
                if [std::io::ErrorKind::WouldBlock, std::io::ErrorKind::TimedOut]
                    .contains(&e.kind()) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("Incomplete or oversized HTTP headers".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Fixture {
        Fixture {
            url_env: "CLIVERIFYR_HTTP_URL".into(),
            responses: vec![
                Response {
                    method: "GET".into(),
                    path: "/first".into(),
                    status: 302,
                    body: String::new(),
                    delay_ms: 0,
                    location: Some("/second".into()),
                },
                Response {
                    method: "HEAD".into(),
                    path: "/second".into(),
                    status: 200,
                    body: "hello".into(),
                    delay_ms: 0,
                    location: None,
                },
            ],
        }
    }
    fn send(url: &str, request: &str) -> String {
        let mut client = TcpStream::connect(url.strip_prefix("http://").unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client.write_all(request.as_bytes()).unwrap();
        let mut result = String::new();
        client.read_to_string(&mut result).unwrap();
        result
    }
    #[test]
    fn ordered_redirect_and_head_have_correct_wire_content() {
        let server = Server::start(&fixture(), Instant::now() + Duration::from_secs(3)).unwrap();
        let first = send(
            &server.url,
            "GET /first HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        assert!(first.contains("302 Fixture\r\n"));
        assert!(first.contains("Location: /second\r\n"));
        let second = send(
            &server.url,
            "HEAD /second HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        assert!(second.contains("Content-Length: 5\r\n"));
        assert!(second.ends_with("\r\n\r\n"));
        server.finish().unwrap();
    }
    #[test]
    fn extra_requests_fail_contract() {
        let mut config = fixture();
        config.responses.truncate(1);
        let server = Server::start(&config, Instant::now() + Duration::from_secs(3)).unwrap();
        send(
            &server.url,
            "GET /first HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        let extra = send(
            &server.url,
            "GET /first HTTP/1.1\r\nHost: localhost\r\n\r\n",
        );
        assert!(extra.contains("500 Unexpected"));
        assert!(server.finish().is_err());
    }
    #[test]
    fn dropping_long_delay_stops_worker_and_closes_listener() {
        let mut config = fixture();
        config.responses.truncate(1);
        config.responses[0].delay_ms = 60000;
        let server = Server::start(&config, Instant::now() + Duration::from_secs(65)).unwrap();
        let address = server.url.strip_prefix("http://").unwrap().to_string();
        let mut client = TcpStream::connect(&address).unwrap();
        client
            .write_all(b"GET /first HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        thread::sleep(Duration::from_millis(30));
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_millis(500));
        assert!(TcpStream::connect(&address).is_err());
    }
    #[test]
    fn bounded_validation_rejects_header_injection_and_excess_resources() {
        let mut config = fixture();
        config.responses[0].location = Some("/ok\r\nInjected: yes".into());
        assert!(config.validate().is_err());
        config.responses[0].location = None;
        config.responses[0].body = "x".repeat(65537);
        assert!(config.validate().is_err());
    }
}
