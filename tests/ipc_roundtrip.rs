//! IPC round-trip against a real socket on an explicit path, plus runtime-dir
//! resolution rules (0700 fallback dir when XDG_RUNTIME_DIR is unset).
//! Own file: owns its env.

use runwaybar::ipc;
use runwaybar::model::{ProviderSnapshot, Snapshot, Status, SCHEMA_VERSION};
use runwaybar::state::HubState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

fn sample_snapshot(pct: f64) -> Snapshot {
    Snapshot {
        schema_version: SCHEMA_VERSION,
        generated_at: runwaybar::timefmt::now_rfc3339(),
        providers: vec![ProviderSnapshot {
            id: "claude-code".into(),
            label: "Claude Code".into(),
            status: Status::Ok,
            account: None,
            windows: vec![runwaybar::model::RateWindow::new(
                runwaybar::model::WindowKind::Session,
                None,
                Some(pct),
                None,
            )],
        }],
        cooldowns: Default::default(),
    }
}

#[test]
fn status_and_refresh_round_trip_oversize_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("sock");
    let state = Arc::new(Mutex::new(HubState::new(
        sample_snapshot(62.0),
        std::time::Duration::from_secs(300),
    )));
    let refresh: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));
    let shutdown = Arc::new(AtomicBool::new(false));
    std::thread::spawn({
        let (state, refresh, shutdown) = (state.clone(), refresh.clone(), shutdown.clone());
        let sock_for_thread = sock.clone();
        move || ipc::serve_at(sock_for_thread, state, refresh, shutdown)
    });
    // Wait for the socket to appear.
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Socket is user-only.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "socket mode {mode:o}");
    }

    // status round-trip returns the daemon's snapshot verbatim.
    let snap = ipc::request_status_at(&sock).expect("status reply");
    assert_eq!(
        snap.provider("claude-code").unwrap().worst_used_percent(),
        Some(62.0)
    );

    // refresh acks through the REAL client function (regression: the line-delimited
    // reply must be trimmed before comparison)...
    assert!(
        ipc::request_refresh_at(&sock),
        "refresh must ack via the client"
    );
    // ...and via the raw protocol, verifying the exact wire bytes:
    {
        use std::io::Write;
        let mut s = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        writeln!(s, r#"{{"cmd":"refresh"}}"#).unwrap();
        let mut line = String::new();
        use std::io::BufRead;
        std::io::BufReader::new(s).read_line(&mut line).unwrap();
        assert_eq!(line.trim(), r#"{"ok":true}"#);
    }
    assert!(*refresh.0.lock().unwrap(), "refresh flag must be set");

    // Oversize request (>64 KiB) is dropped without a reply.
    {
        use std::io::{BufRead, Write};
        let mut s = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let big = format!(
            "{{\"cmd\":\"status\",\"pad\":\"{}}}\"",
            "x".repeat(70 * 1024)
        );
        writeln!(s, "{big}").unwrap();
        let mut line = String::new();
        let n = std::io::BufReader::new(s).read_line(&mut line).unwrap_or(0);
        assert_eq!(
            n, 0,
            "oversize request must not be answered (got {n} bytes)"
        );
    }

    // Unknown command gets a typed error.
    {
        use std::io::{BufRead, Write};
        let mut s = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        writeln!(s, r#"{{"cmd":"nope"}}"#).unwrap();
        let mut line = String::new();
        std::io::BufReader::new(s).read_line(&mut line).unwrap();
        assert!(line.contains("unknown command"), "reply: {line}");
    }

    shutdown.store(true, Ordering::SeqCst);
    refresh.1.notify_all();
    // Server thread exits on next accept error/loop break; not joined (daemon-like).
}

#[test]
fn runtime_dir_fallback_without_xdg_runtime_dir() {
    // Fallback path is 0700 and owned by us; unset XDG_RUNTIME_DIR for this process.
    std::env::remove_var("XDG_RUNTIME_DIR");
    let dir = ipc::runtime_dir().expect("fallback runtime dir");
    assert!(
        dir.starts_with("/tmp") || dir.to_string_lossy().contains("runwaybar"),
        "dir: {dir:?}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "fallback dir mode {mode:o}");
    }
    let uid = runwaybar::ipc::socket_path().is_some();
    assert!(uid, "socket path resolvable under fallback");
}
