//! Daemon hub state: merged snapshot + cooldowns + staleness, single-writer.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::model::{ProviderSnapshot, Snapshot, Status};

/// Display staleness (invariant #6): older than max(10 min, 2× interval) → Ok becomes Stale.
pub fn display_stale_threshold(interval: Duration) -> Duration {
    Duration::from_secs((interval.as_secs() * 2).max(600))
}

#[derive(Debug)]
pub struct HubState {
    pub snapshot: Snapshot,
    /// provider id → epoch-ms instant until which background polls are suppressed.
    pub cooldowns: BTreeMap<String, i64>,
    pub interval: Duration,
}

impl HubState {
    pub fn new(seed: Snapshot, interval: Duration) -> Self {
        let cooldowns = seed.cooldowns.clone();
        let mut s = HubState {
            snapshot: seed,
            cooldowns,
            interval,
        };
        s.refresh_staleness();
        s
    }

    /// Merge a fresh poll: failed providers keep previous windows (Stale), successes
    /// replace, cooldowns are recorded/advanced. Providers deliberately skipped this
    /// cycle (cooldown) keep their previous entries untouched.
    pub fn apply_poll(&mut self, fresh: Snapshot) {
        let mut next = fresh.clone();
        for p in &mut next.providers {
            if let Status::Error { class, .. } = &p.status {
                if let Some(prev) = self.snapshot.provider(&p.id) {
                    if !prev.windows.is_empty() {
                        p.windows = prev.windows.clone();
                        p.status = Status::Stale {
                            since: next.generated_at.clone(),
                            reason: Some(format!(
                                "last poll failed ({class}); showing previous data"
                            )),
                        };
                    }
                }
            }
        }
        // Carry forward providers that were skipped this cycle.
        for prev in &self.snapshot.providers {
            if next.provider(&prev.id).is_none() {
                next.providers.push(prev.clone());
            }
        }
        for (id, until) in fresh.cooldowns {
            let e = next.cooldowns.entry(id).or_insert(0);
            *e = (*e).max(until);
        }
        self.snapshot = next;
        self.refresh_staleness();
    }

    /// Providers eligible for a background poll now (enabled, cooldown expired).
    /// `all_ids` = enabled ids from config.
    pub fn pollable_now(&self, all_ids: &[String], now_ms: i64) -> Vec<String> {
        all_ids
            .iter()
            .filter(|id| {
                self.cooldowns
                    .get(*id)
                    .map(|until| now_ms >= *until)
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// Ok → Stale when the snapshot ages past the display threshold.
    pub fn refresh_staleness(&mut self) {
        let Ok(generated) = self.snapshot.generated_at.parse::<jiff::Timestamp>() else {
            return;
        };
        let age_ms = jiff::Timestamp::now().as_millisecond() - generated.as_millisecond();
        let threshold_ms = display_stale_threshold(self.interval).as_millis() as i64;
        if age_ms > threshold_ms {
            for p in &mut self.snapshot.providers {
                if let Status::Ok = p.status {
                    p.status = Status::Stale {
                        since: self.snapshot.generated_at.clone(),
                        reason: Some("data older than the staleness threshold".into()),
                    };
                }
            }
        }
    }
}

/// Level used by the notification policy (transitions notify, steady states do not).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Ok,
    Warning,
    Critical,
    Error,
    NotInstalled,
    Unknown,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Warning => "warning",
            Level::Critical => "critical",
            Level::Error => "error",
            Level::NotInstalled => "not installed",
            Level::Unknown => "unknown",
        }
    }
}

pub fn provider_level(p: &ProviderSnapshot) -> Level {
    let worst = p.worst_used_percent();
    match (&p.status, worst) {
        (Status::NotInstalled, _) => Level::NotInstalled,
        (Status::Error { .. }, _) | (Status::Stale { .. }, None) => Level::Error,
        (_, Some(v)) if v >= 100.0 => Level::Critical,
        (_, Some(v)) if v >= 80.0 => Level::Warning,
        (_, Some(_)) => Level::Ok,
        (_, None) => Level::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RateWindow, WindowKind, SCHEMA_VERSION};

    fn snap(gen: &str, provs: Vec<ProviderSnapshot>, cooldowns: BTreeMap<String, i64>) -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION,
            generated_at: gen.into(),
            providers: provs,
            cooldowns,
        }
    }

    fn ok_provider(id: &str, pct: f64) -> ProviderSnapshot {
        ProviderSnapshot {
            id: id.into(),
            label: id.into(),
            status: Status::Ok,
            account: None,
            windows: vec![RateWindow::new(WindowKind::Session, None, Some(pct), None)],
        }
    }

    fn err_provider(id: &str, class: &str) -> ProviderSnapshot {
        ProviderSnapshot {
            id: id.into(),
            label: id.into(),
            status: Status::Error {
                class: class.into(),
                message: "x".into(),
            },
            account: None,
            windows: vec![],
        }
    }

    #[test]
    fn failed_poll_keeps_previous_windows_as_stale() {
        let mut hub = HubState::new(
            snap(
                "2026-09-23T10:00:00Z",
                vec![ok_provider("a", 55.0)],
                BTreeMap::new(),
            ),
            Duration::from_secs(300),
        );
        let fresh = snap(
            "2026-09-23T10:05:00Z",
            vec![err_provider("a", "rate-limited")],
            BTreeMap::new(),
        );
        hub.apply_poll(fresh);
        let p = hub.snapshot.provider("a").unwrap();
        assert!(
            matches!(&p.status, Status::Stale { reason, .. } if reason.as_deref().unwrap_or_default().contains("rate-limited"))
        );
        assert_eq!(p.worst_used_percent(), Some(55.0));
    }

    #[test]
    fn cooldown_recorded_and_gates_background_polls() {
        let mut cooldowns = BTreeMap::new();
        cooldowns.insert("a".to_string(), crate::timefmt::now_epoch_ms() + 300_000);
        let fresh = snap(
            &crate::timefmt::now_rfc3339(),
            vec![err_provider("a", "rate-limited")],
            cooldowns,
        );
        let hub = HubState::new(fresh, Duration::from_secs(300));
        let pollable = hub.pollable_now(
            &["a".to_string(), "b".to_string()],
            crate::timefmt::now_epoch_ms(),
        );
        assert_eq!(
            pollable,
            vec!["b".to_string()],
            "cooled-down provider must be skipped"
        );
        // after the cooldown lapses it becomes pollable again
        let later = crate::timefmt::now_epoch_ms() + 301_000;
        assert!(hub
            .pollable_now(&["a".to_string()], later)
            .contains(&"a".to_string()));
    }

    #[test]
    fn skipped_providers_carried_forward() {
        let mut hub = HubState::new(
            snap(
                "2026-09-23T10:00:00Z",
                vec![ok_provider("a", 10.0), ok_provider("b", 20.0)],
                BTreeMap::new(),
            ),
            Duration::from_secs(300),
        );
        // cycle polls only "a" (b on cooldown)
        let fresh = snap(
            &crate::timefmt::now_rfc3339(),
            vec![ok_provider("a", 12.0)],
            BTreeMap::new(),
        );
        hub.apply_poll(fresh);
        assert!(
            hub.snapshot.provider("b").is_some(),
            "skipped provider must survive"
        );
        assert_eq!(
            hub.snapshot.provider("a").unwrap().worst_used_percent(),
            Some(12.0)
        );
    }

    #[test]
    fn aged_ok_becomes_display_stale() {
        let mut hub = HubState::new(
            snap(
                "2026-09-23T10:00:00Z",
                vec![ok_provider("a", 10.0)],
                BTreeMap::new(),
            ),
            Duration::from_secs(300),
        );
        hub.snapshot.generated_at = "2026-09-23T09:00:00Z".to_string();
        hub.refresh_staleness();
        assert!(matches!(
            hub.snapshot.provider("a").unwrap().status,
            Status::Stale { .. }
        ));
    }

    #[test]
    fn levels_map_for_notification_policy() {
        assert_eq!(provider_level(&ok_provider("a", 50.0)), Level::Ok);
        assert_eq!(provider_level(&ok_provider("a", 85.0)), Level::Warning);
        assert_eq!(provider_level(&ok_provider("a", 130.0)), Level::Critical);
        assert_eq!(
            provider_level(&err_provider("a", "network-failure")),
            Level::Error
        );
        let mut ni = ok_provider("a", 0.0);
        ni.status = Status::NotInstalled;
        assert_eq!(provider_level(&ni), Level::NotInstalled);
    }
}
