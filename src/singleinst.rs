//! Single instance per user session via a kernel-released flock: no steal logic —
//! the OS releases the lock when the holder dies (crash-safe by construction).

use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub struct InstanceGuard {
    _file: File,
}

#[derive(Debug)]
pub enum AcquireError {
    AlreadyRunning,
    Io(io::Error),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AcquireError::AlreadyRunning => {
                write!(f, "another RunwayBar daemon is already running")
            }
            AcquireError::Io(e) => write!(f, "lock I/O error: {e}"),
        }
    }
}

pub fn lock_path() -> Option<PathBuf> {
    crate::ipc::runtime_dir().ok().map(|d| d.join("serve.lock"))
}

/// Acquire the daemon lock; held until the returned guard drops (process exit included).
pub fn acquire() -> Result<InstanceGuard, AcquireError> {
    let path = lock_path().ok_or_else(|| AcquireError::Io(io::Error::other("no runtime dir")))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(AcquireError::Io)?;
    file.try_lock().map_err(|e| match &e {
        std::fs::TryLockError::WouldBlock => AcquireError::AlreadyRunning,
        _ => AcquireError::Io(io::Error::other(format!("{e}"))),
    })?;
    Ok(InstanceGuard { _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusion_and_release() {
        // Single test: both halves touch the same runtime-dir lock, so parallel
        // tests would race. flock is per open-file-description: two opens in one
        // process conflict; drop releases.
        {
            let _g = acquire().expect("first acquire");
            match acquire() {
                Err(AcquireError::AlreadyRunning) => {}
                other => panic!("expected AlreadyRunning, got {other:?}"),
            }
        }
        let _g2 = acquire().expect("re-acquire after drop");
    }
}
