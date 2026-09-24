//! Shared mock HTTP server for integration tests (loopback only).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// Shared harness compiled once per test binary; each binary uses a different
// subset, so per-binary dead-code lints would be noise.
#[allow(dead_code)]
/// One captured inbound request: method, path, headers, and body.
#[derive(Clone, Debug)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

#[allow(dead_code)]
pub struct MockServer {
    pub base: String,
    pub requests: Arc<AtomicUsize>,
    captured: Arc<std::sync::Mutex<Vec<CapturedRequest>>>,
}

#[allow(dead_code)]
impl MockServer {
    /// Serves `routes` (path → status, body) forever on a background thread; counts requests.
    pub fn start(routes: Vec<(String, u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let requests = Arc::new(AtomicUsize::new(0));
        let rc = requests.clone();
        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let cap = captured.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let rc = rc.clone();
                let cap = cap.clone();
                let routes = routes.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(&stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        return;
                    }
                    let mut parts = line.split_whitespace();
                    let method = parts.next().unwrap_or("").to_string();
                    let path = parts.next().unwrap_or("/").to_string();
                    let mut headers = Vec::new();
                    let mut content_len = 0usize;
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).is_err()
                            || h == "\r\n"
                            || h == "\n"
                            || h.is_empty()
                        {
                            break;
                        }
                        if let Some((k, v)) = h.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") {
                                content_len = v.trim().parse().unwrap_or(0);
                            }
                            headers.push((k.trim().to_string(), v.trim().to_string()));
                        }
                    }
                    let mut body = vec![0u8; content_len.min(1 << 20)];
                    if !body.is_empty() {
                        use std::io::Read;
                        let _ = reader.read_exact(&mut body);
                    }
                    cap.lock().unwrap().push(CapturedRequest {
                        method,
                        path: path.clone(),
                        headers,
                        body: String::from_utf8_lossy(&body).into_owned(),
                    });
                    rc.fetch_add(1, Ordering::SeqCst);
                    let (status, body) = routes
                        .iter()
                        .find(|(p, _, _)| path == *p)
                        .map(|(_, s, b)| (*s, b.clone()))
                        .unwrap_or((200, "{}".to_string()));
                    let reason = if status == 200 { "OK" } else { "Err" };
                    let resp = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = (&stream).write_all(resp.as_bytes());
                });
            }
        });
        MockServer {
            base,
            requests,
            captured,
        }
    }

    pub fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// Snapshot of all requests received so far, in arrival order.
    pub fn requests_snapshot(&self) -> Vec<CapturedRequest> {
        self.captured.lock().unwrap().clone()
    }
}
