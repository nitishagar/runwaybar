//! Freedesktop notifications on state TRANSITIONS only (max one per provider per
//! cycle; warnings-only by default). Fail-soft: no notification daemon → no-op.

use crate::config::NotifyLevel;
use crate::state::Level;

#[derive(Debug, Default)]
pub struct NotifyPolicy {
    last_levels: std::collections::BTreeMap<String, Level>,
}

impl NotifyPolicy {
    pub fn new() -> Self {
        NotifyPolicy::default()
    }

    /// Returns the notifications to emit for this cycle given the fresh levels.
    /// Rules (config `notifications`):
    /// - off: never
    /// - warnings (default): entering Warning/Critical from below, or entering Error from Ok
    /// - all: any level change except to Ok/NotInstalled/Unknown quieting down
    pub fn collect(
        &mut self,
        levels: &[(String, Level, String)],
        cfg: NotifyLevel,
    ) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (id, level, detail) in levels {
            let prev = self.last_levels.get(id).copied();
            self.last_levels.insert(id.clone(), *level);
            let Some(prev) = prev else { continue }; // first observation: never notify
            if prev == *level {
                continue;
            }
            let should = match cfg {
                NotifyLevel::Off => false,
                NotifyLevel::Warnings => match (*level, prev) {
                    (Level::Critical, _) => prev < Level::Critical && prev != Level::Error,
                    (Level::Warning, _) => prev < Level::Warning && prev != Level::Error,
                    (Level::Error, p) => {
                        p == Level::Ok || p == Level::Warning || p == Level::Critical
                    }
                    _ => false,
                },
                NotifyLevel::All => {
                    !matches!(*level, Level::Ok | Level::NotInstalled | Level::Unknown)
                }
            };
            if should {
                out.push((format!("RunwayBar — {id}"), detail.clone()));
            }
        }
        out
    }
}

/// Fire-and-forget; notification-daemon absence is not an error.
pub fn send(summary: &str, body: &str) {
    use notify_rust::Notification;
    let _ = Notification::new()
        .summary(summary)
        .body(body)
        .timeout(8000)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(
        p: &mut NotifyPolicy,
        cfg: NotifyLevel,
        obs: &[(&str, Level, &str)],
    ) -> Vec<(String, String)> {
        let owned: Vec<(String, Level, String)> = obs
            .iter()
            .map(|(id, l, d)| (id.to_string(), *l, d.to_string()))
            .collect();
        p.collect(&owned, cfg)
    }

    #[test]
    fn first_observation_never_notifies() {
        let mut p = NotifyPolicy::new();
        assert!(step(
            &mut p,
            NotifyLevel::Warnings,
            &[("a", Level::Critical, "100%")]
        )
        .is_empty());
    }

    #[test]
    fn entering_warning_notifies_once_under_warnings_policy() {
        let mut p = NotifyPolicy::new();
        step(&mut p, NotifyLevel::Warnings, &[("a", Level::Ok, "50%")]);
        let n = step(
            &mut p,
            NotifyLevel::Warnings,
            &[("a", Level::Warning, "82%")],
        );
        assert_eq!(n.len(), 1);
        // staying at warning: silent
        assert!(step(
            &mut p,
            NotifyLevel::Warnings,
            &[("a", Level::Warning, "83%")]
        )
        .is_empty());
        // escalating to critical: notify again
        let n = step(
            &mut p,
            NotifyLevel::Warnings,
            &[("a", Level::Critical, "101%")],
        );
        assert_eq!(n.len(), 1);
        // recovery to ok: silent under warnings policy
        assert!(step(&mut p, NotifyLevel::Warnings, &[("a", Level::Ok, "40%")]).is_empty());
    }

    #[test]
    fn off_policy_never_notifies() {
        let mut p = NotifyPolicy::new();
        step(&mut p, NotifyLevel::Off, &[("a", Level::Ok, "50%")]);
        assert!(step(&mut p, NotifyLevel::Off, &[("a", Level::Critical, "x")]).is_empty());
    }

    #[test]
    fn error_from_ok_notifies_under_warnings() {
        let mut p = NotifyPolicy::new();
        step(&mut p, NotifyLevel::Warnings, &[("a", Level::Ok, "50%")]);
        let n = step(
            &mut p,
            NotifyLevel::Warnings,
            &[("a", Level::Error, "network down")],
        );
        assert_eq!(n.len(), 1);
        // repeated error: silent
        assert!(step(
            &mut p,
            NotifyLevel::Warnings,
            &[("a", Level::Error, "still down")]
        )
        .is_empty());
    }

    #[test]
    fn all_policy_notifies_level_changes_except_recovery() {
        let mut p = NotifyPolicy::new();
        step(&mut p, NotifyLevel::All, &[("a", Level::Ok, "50%")]);
        assert_eq!(
            step(&mut p, NotifyLevel::All, &[("a", Level::Warning, "82%")]).len(),
            1
        );
        assert!(
            step(&mut p, NotifyLevel::All, &[("a", Level::Ok, "40%")]).is_empty(),
            "recovery silent"
        );
    }
}
