//! Last-good cache: atomic, 0600, pid-suffixed temps (daemon and one-shot CLI are
//! separate processes writing the same file).

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use crate::model::Snapshot;

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
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
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
    fn unparseable_timestamp_is_stale() {
        assert!(!is_fresh(&snap("garbage"), Duration::from_secs(300)));
    }
}
