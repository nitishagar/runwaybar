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
}
