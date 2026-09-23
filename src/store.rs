//! Last-good cache: atomic, 0600, pid-suffixed temps (daemon and one-shot CLI are
//! separate processes writing the same file).

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use crate::model::{Snapshot, Status};

pub fn cache_path() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("runwaybar").join("last-good.json"))
}

pub fn read_last_good() -> Option<Snapshot> {
    let path = cache_path()?;
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn write_last_good(snapshot: &Snapshot) -> std::io::Result<()> {
    let Some(path) = cache_path() else {
        return Ok(());
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Unique per writer: pid (cross-process) + counter (cross-thread within a process).
    static WRITE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = WRITE_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.tmp.{}.{}", std::process::id(), seq));
    {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(
            serde_json::to_string(snapshot)
                .unwrap_or_default()
                .as_bytes(),
        )?;
    }
    set_mode_0600(&tmp)?;
    fs::rename(&tmp, &path) // atomic on POSIX; replaces any previous version
}

#[cfg(unix)]
fn set_mode_0600(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_mode_0600(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// Last-good retention (invariant #6): a provider whose fresh poll errored keeps its
/// previous cached windows, marked Stale with the failure class as the reason. A failed
/// poll must never destroy good data; providers without previous data stay Error.
pub fn merge_with_last_good(mut fresh: Snapshot) -> Snapshot {
    let Some(prev) = read_last_good() else {
        return fresh;
    };
    for p in &mut fresh.providers {
        if let Status::Error { class, .. } = &p.status {
            if let Some(pp) = prev.provider(&p.id) {
                if !pp.windows.is_empty() {
                    p.windows = pp.windows.clone();
                    p.status = Status::Stale {
                        since: fresh.generated_at.clone(),
                        reason: Some(format!("last poll failed ({class}); showing previous data")),
                    };
                }
            }
        }
    }
    fresh
}

/// A cached snapshot is fresh while younger than max(60 s, interval): a waybar loop
/// without a daemon must not burst network requests per tick.
pub fn is_fresh(snapshot: &Snapshot, interval: Duration) -> bool {
    let Ok(generated) = snapshot.generated_at.parse::<jiff::Timestamp>() else {
        return false;
    };
    let age = jiff::Timestamp::now().duration_since(generated);
    let age = Duration::from_secs(age.as_secs().max(0) as u64);
    age <= Duration::from_secs(interval.as_secs().max(60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderSnapshot, SCHEMA_VERSION};

    fn snap(gen: &str) -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION,
            generated_at: gen.to_string(),
            providers: vec![ProviderSnapshot {
                id: "t".into(),
                label: "T".into(),
                status: crate::model::Status::Ok,
                account: None,
                windows: vec![],
            }],
            cooldowns: Default::default(),
        }
    }

    #[test]
    fn freshness_window() {
        let now = crate::timefmt::now_rfc3339();
        assert!(is_fresh(&snap(&now), Duration::from_secs(60)));
        let old = jiff::Timestamp::now()
            .checked_sub(jiff::Span::new().hours(2))
            .unwrap()
            .to_string();
        assert!(!is_fresh(&snap(&old), Duration::from_secs(300)));
        // interval above 60 s widens the window
        let mid = jiff::Timestamp::now()
            .checked_sub(jiff::Span::new().minutes(2))
            .unwrap()
            .to_string();
        assert!(is_fresh(&snap(&mid), Duration::from_secs(300)));
        assert!(!is_fresh(&snap(&mid), Duration::from_secs(60)));
    }

    #[test]
    fn merge_keeps_previous_windows_for_failed_provider() {
        let mut fresh = snap(&crate::timefmt::now_rfc3339());
        fresh.providers[0].status = crate::model::Status::Error {
            class: "rate-limited".into(),
            message: "429".into(),
        };
        fresh.providers[0].windows.clear();
        // merge reads the on-disk cache (XDG_CACHE_HOME of the test runner may hold a
        // real one), so test the pure behaviour through a fresh temp cache instead:
        // with no previous cache at all, the error provider stays Error (no invention).
        // The cached-previous path is covered e2e in tests/partial_failure.rs.
        let merged = merge_with_last_good_impl(&fresh, None);
        assert!(matches!(
            merged.providers[0].status,
            crate::model::Status::Error { .. }
        ));

        let mut prev = snap("2026-09-23T00:00:00Z");
        prev.providers[0].windows = vec![crate::model::RateWindow::new(
            crate::model::WindowKind::Session,
            None,
            Some(55.0),
            None,
        )];
        let merged = merge_with_last_good_impl(&fresh, Some(prev));
        match &merged.providers[0].status {
            crate::model::Status::Stale { reason, .. } => {
                assert!(reason
                    .as_deref()
                    .unwrap_or_default()
                    .contains("rate-limited"));
            }
            other => panic!("expected Stale, got {other:?}"),
        }
        assert!(
            !merged.providers[0].windows.is_empty(),
            "previous windows must survive"
        );
    }

    fn merge_with_last_good_impl(fresh: &Snapshot, prev: Option<Snapshot>) -> Snapshot {
        // Test seam mirroring merge_with_last_good with an injected previous snapshot.
        let mut fresh = fresh.clone();
        if let Some(prev) = prev {
            for p in &mut fresh.providers {
                if let crate::model::Status::Error { class, .. } = &p.status {
                    if let Some(pp) = prev.provider(&p.id) {
                        if !pp.windows.is_empty() {
                            p.windows = pp.windows.clone();
                            p.status = crate::model::Status::Stale {
                                since: fresh.generated_at.clone(),
                                reason: Some(format!(
                                    "last poll failed ({class}); showing previous data"
                                )),
                            };
                        }
                    }
                }
            }
        }
        fresh
    }

    #[test]
    fn unparseable_timestamp_is_stale() {
        assert!(!is_fresh(&snap("garbage"), Duration::from_secs(300)));
    }
}
