//! Shared mock HTTP server for integration tests (loopback only).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

pub struct MockServer {
    pub base: String,
    pub requests: Arc<AtomicUsize>,
}

impl MockServer {
    /// Serves `routes` (path → status, body) forever on a background thread; counts requests.
    pub fn start(routes: Vec<(String, u16, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let requests = Arc::new(AtomicUsize::new(0));
        let rc = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let rc = rc.clone();
                let routes = routes.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(&stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() {
                        return;
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    loop {
                        let mut h = String::new();
                        if reader.read_line(&mut h).is_err()
                            || h == "\r\n"
                            || h == "\n"
                            || h.is_empty()
                        {
                            break;
                        }
                    }
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
        MockServer { base, requests }
    }

    pub fn count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}
