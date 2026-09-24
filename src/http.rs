//! Bounded HTTP fetch: one agent, per-request timeout, 8 MiB body cap, one bounded
//! retry on 429/5xx honouring `Retry-After` (capped), detached-worker resolver-hang guard.

use std::io::Read;
use std::time::Duration;

use crate::error::{ErrorClass, ProviderError};

const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub retry_after: Option<Duration>,
}

fn agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // We want the response (headers + body) for 429/5xx too, to honour Retry-After.
        .http_status_as_error(false)
        .build();
    config.new_agent()
}

/// Single GET. `headers` are applied verbatim; callers must never pass secret
/// material anywhere that could end up in an error string.
pub fn fetch_json(
    url: &str,
    headers: &[(String, String)],
    timeout: Duration,
) -> Result<HttpResponse, ProviderError> {
    let a = agent(timeout);
    let mut req = a.get(url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .call()
        .map_err(|e| ProviderError::new(ErrorClass::NetworkFailure, format!("transport: {e}")))?;
    read_capped_response(resp)
}

/// Single POST with a caller-supplied body. Same guards as [`fetch_json`];
/// the one retry (see [`post_with_retry`]) makes POST safe only for
/// idempotent endpoints — callers must establish that first.
pub fn post_json(
    url: &str,
    headers: &[(String, String)],
    body: &str,
    timeout: Duration,
) -> Result<HttpResponse, ProviderError> {
    let a = agent(timeout);
    let mut req = a.post(url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req
        .send(body)
        .map_err(|e| ProviderError::new(ErrorClass::NetworkFailure, format!("transport: {e}")))?;
    read_capped_response(resp)
}

/// Shared response drain: status + `Retry-After` + body capped at 8 MiB.
fn read_capped_response(
    resp: ureq::http::Response<ureq::Body>,
) -> Result<HttpResponse, ProviderError> {
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let mut body = String::new();
    let mut reader = resp
        .into_body()
        .into_reader()
        .take((MAX_BODY_BYTES + 1) as u64);
    reader
        .read_to_string(&mut body)
        .map_err(|e| ProviderError::new(ErrorClass::NetworkFailure, format!("body read: {e}")))?;
    if body.len() > MAX_BODY_BYTES {
        return Err(ProviderError::new(
            ErrorClass::ParseFailure,
            format!("response exceeds the {} byte cap", MAX_BODY_BYTES),
        ));
    }
    Ok(HttpResponse {
        status,
        body,
        retry_after,
    })
}

/// Map an HTTP status to the shared error taxonomy (no body content in the message).
pub fn status_error(status: u16, endpoint_label: &str) -> ProviderError {
    match status {
        401 | 403 => ProviderError::new(
            if status == 401 {
                ErrorClass::AuthExpired
            } else {
                ErrorClass::PermissionDenied
            },
            format!("{endpoint_label} returned {status}"),
        ),
        429 => ProviderError::new(
            ErrorClass::RateLimited,
            format!("{endpoint_label} returned 429"),
        )
        .with_cooldown(Duration::from_secs(300)),
        404 => ProviderError::new(
            ErrorClass::ProviderUnavailable,
            format!("{endpoint_label} returned 404"),
        ),
        s if (500..=599).contains(&s) => ProviderError::new(
            ErrorClass::ProviderUnavailable,
            format!("{endpoint_label} returned {s}"),
        ),
        s => ProviderError::new(
            ErrorClass::ApiFailure,
            format!("{endpoint_label} returned {s}"),
        ),
    }
}

/// Fetch with the bounded retry policy: on 429/5xx wait `min(Retry-After, max_retry_wait)`
/// once, then refetch; the second result is final.
pub fn fetch_with_retry(
    url: &str,
    headers: &[(String, String)],
    timeout: Duration,
    max_retry_wait: Duration,
) -> Result<HttpResponse, ProviderError> {
    let first = fetch_json(url, headers, timeout)?;
    if first.status != 429 && !(500..=599).contains(&first.status) {
        return Ok(first);
    }
    let wait = first
        .retry_after
        .map(|d| d.min(max_retry_wait))
        .unwrap_or(Duration::ZERO);
    std::thread::sleep(wait);
    fetch_json(url, headers, timeout)
}

/// Resolver-hang guard: `getaddrinfo` is not interruptible by the HTTP timeout, so the
/// real fetch runs in a detached worker and we only block for a bounded window. A worker
/// abandoned on timeout is acceptable because the caller puts the provider on a cooldown
/// (bounded accumulation); see PLAN_1 resolver-hang guard.
pub fn fetch_bounded(
    url: String,
    headers: Vec<(String, String)>,
    timeout: Duration,
    max_retry_wait: Duration,
) -> Result<HttpResponse, ProviderError> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<HttpResponse, ProviderError>>();
    std::thread::spawn(move || {
        // Send errors are only "receiver gone" — nothing to do.
        let _ = tx.send(fetch_with_retry(&url, &headers, timeout, max_retry_wait));
    });
    let guard = timeout.saturating_mul(3) + Duration::from_secs(15);
    match rx.recv_timeout(guard) {
        Ok(r) => r,
        Err(_) => Err(
            ProviderError::new(ErrorClass::Timeout, "fetch exceeded the bounded window")
                .with_cooldown(Duration::from_secs(300)),
        ),
    }
}

/// POST twin of [`fetch_with_retry`]: on 429/5xx wait `min(Retry-After,
/// max_retry_wait)` once, then re-POST; the second result is final. Only for
/// idempotent endpoints (the retry re-sends the body).
pub fn post_with_retry(
    url: &str,
    headers: &[(String, String)],
    body: &str,
    timeout: Duration,
    max_retry_wait: Duration,
) -> Result<HttpResponse, ProviderError> {
    let first = post_json(url, headers, body, timeout)?;
    if first.status != 429 && !(500..=599).contains(&first.status) {
        return Ok(first);
    }
    let wait = first
        .retry_after
        .map(|d| d.min(max_retry_wait))
        .unwrap_or(Duration::ZERO);
    std::thread::sleep(wait);
    post_json(url, headers, body, timeout)
}

/// POST twin of [`fetch_bounded`]: same detached-worker resolver-hang guard.
pub fn post_bounded(
    url: String,
    headers: Vec<(String, String)>,
    body: String,
    timeout: Duration,
    max_retry_wait: Duration,
) -> Result<HttpResponse, ProviderError> {
    let (tx, rx) = std::sync::mpsc::channel::<Result<HttpResponse, ProviderError>>();
    std::thread::spawn(move || {
        let _ = tx.send(post_with_retry(
            &url,
            &headers,
            &body,
            timeout,
            max_retry_wait,
        ));
    });
    let guard = timeout.saturating_mul(3) + Duration::from_secs(15);
    match rx.recv_timeout(guard) {
        Ok(r) => r,
        Err(_) => Err(
            ProviderError::new(ErrorClass::Timeout, "fetch exceeded the bounded window")
                .with_cooldown(Duration::from_secs(300)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Minimal one-shot HTTP server; each `respond` closure handles one request.
    fn serve(responses: Vec<(u16, Option<u64>, String)>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/usage");
        let handle = std::thread::spawn(move || {
            for (status, retry_after, body) in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap(); // request line
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h == "\r\n" || h == "\n" || h.is_empty() {
                        break;
                    }
                }
                let mut out = stream;
                let ra = retry_after
                    .map(|s| format!("Retry-After: {s}\r\n"))
                    .unwrap_or_default();
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{ra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                out.write_all(resp.as_bytes()).unwrap();
            }
        });
        (url, handle)
    }

    /// One captured inbound request for POST-ness assertions. `serve()` is left
    /// untouched so its callers keep byte-identical behavior; new tests use
    /// this harness instead.
    #[derive(Debug)]
    struct CapturedRequest {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    /// `serve()` twin that records method/path/headers/body per request.
    fn serve_capturing(
        responses: Vec<(u16, Option<u64>, String)>,
    ) -> (
        String,
        std::sync::Arc<std::sync::Mutex<Vec<CapturedRequest>>>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/usage");
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = captured.clone();
        let handle = std::thread::spawn(move || {
            use std::io::Read;
            for (status, retry_after, body) in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap(); // request line
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap_or("").to_string();
                let path = parts.next().unwrap_or("").to_string();
                let mut headers = Vec::new();
                let mut content_len = 0usize;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h == "\r\n" || h == "\n" || h.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = h.trim().split_once(':') {
                        if k.trim().eq_ignore_ascii_case("content-length") {
                            content_len = v.trim().parse().unwrap_or(0);
                        }
                        headers.push((k.trim().to_string(), v.trim().to_string()));
                    }
                }
                // Body bytes may already sit in the BufReader — read via `reader`.
                let mut raw = vec![0u8; content_len];
                reader.read_exact(&mut raw).unwrap();
                sink.lock().unwrap().push(CapturedRequest {
                    method,
                    path,
                    headers,
                    body: String::from_utf8_lossy(&raw).into_owned(),
                });
                let mut out = stream;
                let ra = retry_after
                    .map(|s| format!("Retry-After: {s}\r\n"))
                    .unwrap_or_default();
                let resp = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n{ra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                out.write_all(resp.as_bytes()).unwrap();
            }
        });
        (url, captured, handle)
    }

    #[test]
    fn ok_response_body_and_status() {
        let (url, h) = serve(vec![(200, None, r#"{"a":1}"#.into())]);
        let r = fetch_json(&url, &[], Duration::from_secs(5)).unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, r#"{"a":1}"#);
        h.join().unwrap();
    }

    #[test]
    fn retry_honours_retry_after_capped() {
        // First 429 with Retry-After: 30, then 200. Cap 500ms → total wait ≈ 500ms, not 30s.
        let (url, h) = serve(vec![
            (429, Some(30), "{}".into()),
            (200, None, r#"{"ok":true}"#.into()),
        ]);
        let start = std::time::Instant::now();
        let r = fetch_with_retry(
            &url,
            &[],
            Duration::from_secs(5),
            Duration::from_millis(500),
        )
        .unwrap();
        let elapsed = start.elapsed();
        assert_eq!(r.status, 200);
        assert!(
            elapsed >= Duration::from_millis(450) && elapsed < Duration::from_secs(3),
            "elapsed {elapsed:?}"
        );
        h.join().unwrap();
    }

    #[test]
    fn no_retry_on_success_first_try_single_request() {
        let (url, h) = serve(vec![(200, None, "{}".into())]);
        let r = fetch_with_retry(
            &url,
            &[],
            Duration::from_secs(5),
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(r.status, 200);
        h.join().unwrap(); // server handled exactly one connection or test hangs
    }

    #[test]
    fn body_over_cap_rejected() {
        let big = "x".repeat(MAX_BODY_BYTES + 1024);
        let (url, h) = serve(vec![(200, None, big)]);
        let r = fetch_json(&url, &[], Duration::from_secs(10));
        assert!(r.is_err());
        h.join().unwrap();
    }

    #[test]
    fn retry_without_retry_after_is_immediate() {
        // 429 with no Retry-After header, then 200: one quick retry, no 10 s wait.
        let (url, h) = serve(vec![
            (429, None, "{}".into()),
            (200, None, r#"{"ok":true}"#.into()),
        ]);
        let start = std::time::Instant::now();
        let r = fetch_with_retry(
            &url,
            &[],
            Duration::from_secs(5),
            Duration::from_millis(500),
        )
        .unwrap();
        assert_eq!(r.status, 200);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "elapsed {:?}",
            start.elapsed()
        );
        h.join().unwrap();
    }

    #[test]
    fn truncated_body_is_network_error() {
        // Content-Length lies (declares more than sent); connection closes early.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/t");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{BufRead, BufReader, Write};
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            loop {
                let mut hl = String::new();
                reader.read_line(&mut hl).unwrap();
                if hl == "\r\n" || hl.is_empty() {
                    break;
                }
            }
            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"trun";
            stream.write_all(resp.as_bytes()).unwrap();
            drop(stream);
        });
        let r = fetch_json(&url, &[], Duration::from_secs(5));
        assert!(
            r.is_err(),
            "truncated body must error, got {:?}",
            r.ok().map(|x| x.body)
        );
        handle.join().unwrap();
    }

    #[test]
    fn status_error_maps_taxonomy() {
        assert_eq!(status_error(401, "e").class, ErrorClass::AuthExpired);
        assert_eq!(status_error(403, "e").class, ErrorClass::PermissionDenied);
        let e429 = status_error(429, "e");
        assert_eq!(e429.class, ErrorClass::RateLimited);
        assert_eq!(e429.cooldown, Some(Duration::from_secs(300)));
        assert_eq!(
            status_error(503, "e").class,
            ErrorClass::ProviderUnavailable
        );
    }

    #[test]
    fn post_sends_method_body_and_headers() {
        let (url, captured, h) = serve_capturing(vec![(200, None, r#"{"ok":true}"#.into())]);
        let r = post_json(
            &url,
            &[
                ("x-api-version".to_string(), "1.0.0".to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            "{}",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, r#"{"ok":true}"#);
        h.join().unwrap();
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "POST");
        assert_eq!(reqs[0].path, "/usage");
        assert_eq!(reqs[0].body, "{}");
        assert!(
            reqs[0]
                .headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("x-api-version") && v == "1.0.0"),
            "headers were {:?}",
            reqs[0].headers
        );
    }

    #[test]
    fn post_retry_resends_once_then_final() {
        let (url, captured, h) = serve_capturing(vec![
            (429, None, "{}".into()),
            (200, None, r#"{"ok":true}"#.into()),
        ]);
        let r = post_with_retry(
            &url,
            &[],
            "{}",
            Duration::from_secs(5),
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(r.status, 200);
        h.join().unwrap();
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        assert!(reqs.iter().all(|q| q.method == "POST" && q.body == "{}"));
    }

    #[test]
    fn post_retry_honours_retry_after_capped() {
        // First 429 with Retry-After: 30, then 200. Cap 500ms → ≈500ms, not 30s.
        let (url, captured, h) = serve_capturing(vec![
            (429, Some(30), "{}".into()),
            (200, None, r#"{"ok":true}"#.into()),
        ]);
        let start = std::time::Instant::now();
        let r = post_with_retry(
            &url,
            &[],
            "{}",
            Duration::from_secs(5),
            Duration::from_millis(500),
        )
        .unwrap();
        let elapsed = start.elapsed();
        assert_eq!(r.status, 200);
        assert!(
            elapsed >= Duration::from_millis(450) && elapsed < Duration::from_secs(3),
            "elapsed {elapsed:?}"
        );
        h.join().unwrap();
        assert_eq!(captured.lock().unwrap().len(), 2);
    }

    #[test]
    fn post_no_retry_on_success_single_request() {
        let (url, captured, h) = serve_capturing(vec![(200, None, "{}".into())]);
        let r = post_with_retry(
            &url,
            &[],
            "{}",
            Duration::from_secs(5),
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(r.status, 200);
        h.join().unwrap(); // exactly one connection or the test hangs
        assert_eq!(captured.lock().unwrap().len(), 1);
    }

    #[test]
    fn post_body_over_cap_rejected() {
        // POST twin of body_over_cap_rejected: an over-cap response is an
        // error no matter the method.
        let big = "x".repeat(MAX_BODY_BYTES + 1024);
        let (url, h) = serve(vec![(200, None, big)]);
        let r = post_json(&url, &[], "{}", Duration::from_secs(10));
        assert!(r.is_err());
        h.join().unwrap();
    }

    #[test]
    fn post_retry_503_then_success() {
        // 503 is retryable for POST exactly as for GET.
        let (url, captured, h) = serve_capturing(vec![
            (503, None, "{}".into()),
            (200, None, r#"{"ok":true}"#.into()),
        ]);
        let r = post_with_retry(
            &url,
            &[],
            "{}",
            Duration::from_secs(5),
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(r.status, 200);
        h.join().unwrap();
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        assert!(reqs.iter().all(|q| q.method == "POST" && q.body == "{}"));
    }

    #[test]
    fn post_persistent_429_returns_final_without_third_request() {
        // Two 429s, no Retry-After: no wait, no third attempt; the second
        // response is final and the caller maps it to RateLimited.
        let (url, captured, h) =
            serve_capturing(vec![(429, None, "{}".into()), (429, None, "{}".into())]);
        let start = std::time::Instant::now();
        let r = post_with_retry(
            &url,
            &[],
            "{}",
            Duration::from_secs(5),
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(r.status, 429);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "elapsed {:?}",
            start.elapsed()
        );
        h.join().unwrap();
        assert_eq!(captured.lock().unwrap().len(), 2);
    }

    #[test]
    fn post_bounded_success_wiring() {
        // The bounded worker relays a successful POST end to end.
        let (url, captured, h) = serve_capturing(vec![(200, None, r#"{"ok":true}"#.into())]);
        let r = post_bounded(
            url,
            vec![],
            "{}".to_string(),
            Duration::from_secs(5),
            Duration::from_millis(100),
        )
        .unwrap();
        assert_eq!(r.status, 200);
        assert_eq!(r.body, r#"{"ok":true}"#);
        h.join().unwrap();
        let reqs = captured.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0].method, "POST");
    }
}
