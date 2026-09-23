//! Same-user IPC over a Unix socket in a 0700 runtime dir.
//!
//! Stage 1 ships the client half (`try_request_status`); the daemon (Stage 2) serves
//! `{"cmd":"status"}` → snapshot and `{"cmd":"refresh"}` → ack, line-delimited JSON.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use crate::model::Snapshot;

pub const MAX_REQUEST_BYTES: usize = 64 * 1024;
pub const MAX_REPLY_BYTES: usize = 4 * 1024 * 1024;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);

/// Runtime dir: `$XDG_RUNTIME_DIR/runwaybar`, falling back to
/// `$TMPDIR|/tmp/runwaybar-$UID` (non-systemd sessions), always 0700 + owner-checked.
pub fn runtime_dir() -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let dir = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(x) if !x.is_empty() => PathBuf::from(x).join("runwaybar"),
        _ => {
            let tmp = std::env::var_os("TMPDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp"));
            let uid = nix_uid();
            tmp.join(format!("runwaybar-{uid}"))
        }
    };
    let created = !dir.exists();
    std::fs::create_dir_all(&dir)?;
    let meta = std::fs::metadata(&dir)?;
    if !meta.is_dir() {
        return Err(std::io::Error::other(
            "runtime path exists but is not a directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let perms = meta.permissions();
        if created || perms.mode() & 0o777 != 0o700 {
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        if meta.uid() != nix_uid() {
            return Err(std::io::Error::other(
                "runtime dir is owned by another user; refusing to use it",
            ));
        }
    }
    Ok(dir)
}

/// Process uid without a libc dep: `/proc/self` is owned by the process's effective uid.
#[cfg(unix)]
fn nix_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self")
        .or_else(|_| std::fs::metadata("/"))
        .map(|m| m.uid())
        .unwrap_or(u32::MAX)
}

pub fn socket_path() -> Option<PathBuf> {
    runtime_dir().ok().map(|d| d.join("sock"))
}

/// Client: ask a running daemon for its snapshot. None = no daemon / timeout /
/// malformed reply; callers fall back to the cache/one-shot path.
pub fn try_request_status() -> Option<Snapshot> {
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path).ok()?;
    stream.set_read_timeout(Some(CLIENT_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(CLIENT_TIMEOUT)).ok()?;
    writeln!(stream, r#"{{"cmd":"status"}}"#).ok()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    if line.len() > MAX_REPLY_BYTES {
        return None;
    }
    serde_json::from_str(line.trim()).ok()
}

/// Client: poke the daemon to refresh now. True if the daemon acked.
pub fn request_refresh() -> bool {
    let Some(path) = socket_path() else {
        return false;
    };
    let Ok(mut stream) = UnixStream::connect(&path) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));
    if writeln!(stream, r#"{{"cmd":"refresh"}}"#).is_err() {
        return false;
    }
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return false;
    }
    line.trim() == r#"{"ok":true}"#
}
