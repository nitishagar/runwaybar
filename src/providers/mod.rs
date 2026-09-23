//! Provider registry and shared tolerant-JSON helpers.

pub mod claude;
pub mod codex;
pub mod opencode;
pub mod zai;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::thread;

use crate::config::Config;
use crate::error::{ErrorClass, ProviderError};
use crate::http::fetch_bounded;
use crate::model::{ProviderSnapshot, Snapshot};
use crate::timefmt::now_rfc3339;

pub const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
pub const MAX_RETRY_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Everything a provider needs: home root + resolved config. Cheap to clone per thread.
#[derive(Clone)]
pub struct Ctx {
    pub home: std::path::PathBuf,
    pub config: std::sync::Arc<Config>,
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    /// One-line human name of the owning tool, used in user hints ("sign in with X first").
    fn tool_hint(&self) -> &'static str;
    fn poll(&self, ctx: &Ctx) -> Result<PollResult, ProviderError>;
}

pub struct PollResult {
    pub windows: Vec<crate::model::RateWindow>,
    pub account: Option<String>,
}

pub fn registry() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(claude::ClaudeCode),
        Box::new(codex::Codex),
        Box::new(zai::Zai),
        Box::new(opencode::OpenCode),
    ]
}

/// Poll all enabled providers in parallel scoped threads; each provider is isolated
/// (catch_unwind + independent result). Partial failures never affect healthy providers.
/// Each thread returns its snapshot plus an optional cooldown deadline (429/timeout).
pub fn poll_all(config: std::sync::Arc<Config>) -> Snapshot {
    let ctx = Ctx {
        home: crate::config::home_root(),
        config,
    };
    let providers: Vec<Box<dyn Provider>> = registry()
        .into_iter()
        .filter(|p| *ctx.config.enabled.get(p.id()).unwrap_or(&true))
        .collect();
    let results: Vec<(ProviderSnapshot, Option<i64>)> = thread::scope(|s| {
        let handles: Vec<_> = providers
            .iter()
            .map(|p| {
                let ctx = ctx.clone();
                s.spawn(move || {
                    let cooldown = |e: &ProviderError| -> Option<i64> {
                        e.cooldown
                            .map(|d| crate::timefmt::now_epoch_ms() + d.as_millis() as i64)
                    };
                    match catch_unwind(AssertUnwindSafe(|| p.poll(&ctx))) {
                        Ok(Ok(data)) => {
                            let snap = ProviderSnapshot {
                                id: p.id().to_string(),
                                label: p.label().to_string(),
                                status: crate::model::Status::Ok,
                                account: data.account,
                                windows: data.windows,
                            };
                            (snap, None)
                        }
                        Ok(Err(e)) => {
                            let cd = cooldown(&e);
                            let status = if e.class == ErrorClass::NotInstalled {
                                crate::model::Status::NotInstalled
                            } else {
                                crate::model::Status::Error {
                                    class: e.class.as_str().to_string(),
                                    message: e.user_hint(p.tool_hint()),
                                }
                            };
                            let snap = ProviderSnapshot {
                                id: p.id().to_string(),
                                label: p.label().to_string(),
                                status,
                                account: None,
                                windows: Vec::new(),
                            };
                            (snap, cd)
                        }
                        Err(_) => {
                            let snap = ProviderSnapshot {
                                id: p.id().to_string(),
                                label: p.label().to_string(),
                                status: crate::model::Status::Error {
                                    class: ErrorClass::ParseFailure.as_str().to_string(),
                                    message: "internal provider error (panic captured)".to_string(),
                                },
                                account: None,
                                windows: Vec::new(),
                            };
                            (snap, None)
                        }
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| (panic_provider(), None)))
            .collect()
    });
    let mut cooldowns = std::collections::BTreeMap::new();
    let mut snaps = Vec::with_capacity(results.len());
    for (snap, cd) in results {
        if let Some(until) = cd {
            cooldowns.insert(snap.id.clone(), until);
        }
        snaps.push(snap);
    }
    Snapshot {
        schema_version: crate::model::SCHEMA_VERSION,
        generated_at: now_rfc3339(),
        providers: snaps,
        cooldowns,
    }
}

fn panic_provider() -> ProviderSnapshot {
    ProviderSnapshot {
        id: "unknown".to_string(),
        label: "unknown".to_string(),
        status: crate::model::Status::Error {
            class: ErrorClass::ParseFailure.as_str().to_string(),
            message: "provider thread failed".to_string(),
        },
        account: None,
        windows: Vec::new(),
    }
}

/// Shared bounded fetch used by every provider (see `fetch_bounded` for the guard rationale).
pub(crate) fn bounded_fetch(
    url: &str,
    headers: Vec<(String, String)>,
) -> Result<crate::http::HttpResponse, ProviderError> {
    fetch_bounded(url.to_string(), headers, REQUEST_TIMEOUT, MAX_RETRY_WAIT)
}

// ---- tolerant JSON walking (unknown keys are ignored, never fatal — E7 discipline) ----

pub(crate) fn walk<'a>(v: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    let mut cur = v;
    for key in path {
        cur = cur.get(key)?;
    }
    Some(cur)
}

pub(crate) fn as_f64(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

pub(crate) fn first_f64(v: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| v.get(*k).and_then(as_f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Status, WindowKind};
    use serde_json::json;

    #[test]
    fn walk_and_numbers() {
        let v = json!({"data": {"limits": [{"percentage": "42.5"}]}});
        let lim = walk(&v, &["data", "limits"]).unwrap();
        assert!(lim.is_array());
        assert_eq!(
            first_f64(&lim[0], &["percentage", "usedPercentage"]),
            Some(42.5)
        );
        assert!(walk(&v, &["nope"]).is_none());
    }

    #[test]
    fn unknown_window_keys_are_ignored_not_fatal() {
        let v = json!({"five_hour": {"used_pct": 62.5, "surprise_new_field": {"x": 1}}});
        assert_eq!(
            first_f64(&v["five_hour"], &["used_pct", "used_percent"]),
            Some(62.5)
        );
    }

    #[test]
    fn worst_percent_ignores_unknowns() {
        let p = ProviderSnapshot {
            id: "t".into(),
            label: "T".into(),
            status: Status::Ok,
            account: None,
            windows: vec![
                crate::model::RateWindow::new(WindowKind::Session, None, Some(40.0), None),
                crate::model::RateWindow::new(WindowKind::Weekly, None, None, None),
                crate::model::RateWindow::new(WindowKind::Weekly, None, Some(75.0), None),
            ],
        };
        assert_eq!(p.worst_used_percent(), Some(75.0));
    }
}
