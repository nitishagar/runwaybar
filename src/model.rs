//! Snapshot model — the stable JSON contract between providers, cache, IPC and renders.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u32 = 1;

/// One full observation across all providers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub schema_version: u32,
    /// RFC3339 instant at which the snapshot was produced.
    pub generated_at: String,
    pub providers: Vec<ProviderSnapshot>,
    /// provider id → epoch-ms instant before which background polling must not
    /// re-poll (429/timeout cooldowns). Informational for CLI renders; enforced by
    /// the daemon scheduler.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cooldowns: BTreeMap<String, i64>,
}

impl Snapshot {
    pub fn provider(&self, id: &str) -> Option<&ProviderSnapshot> {
        self.providers.iter().find(|p| p.id == id)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProviderSnapshot {
    pub id: String,
    pub label: String,
    pub status: Status,
    /// Provider-siloed, non-sensitive descriptor (e.g. plan name). Never a credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<RateWindow>,
}

impl ProviderSnapshot {
    /// Worst (highest) used percent across known windows — None stays None (unknown).
    pub fn worst_used_percent(&self) -> Option<f64> {
        self.windows
            .iter()
            .filter_map(|w| w.used_percent)
            .fold(None::<f64>, |acc, v| match acc {
                Some(a) if a >= v => Some(a),
                _ => Some(v),
            })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Status {
    Ok,
    /// Last poll failed; `windows` still carry the previous (possibly outdated) result.
    Stale {
        /// RFC3339 since when data has not refreshed.
        since: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Error {
        class: String,
        message: String,
    },
    NotInstalled,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    Session,
    Weekly,
    Monthly,
    Other,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RateWindow {
    pub kind: WindowKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// None = unknown. Values > 100 represent overage and are kept as-is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    /// RFC3339 absolute reset instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub overage: bool,
}

impl RateWindow {
    pub fn new(
        kind: WindowKind,
        label: Option<String>,
        used_percent: Option<f64>,
        resets_at: Option<String>,
    ) -> Self {
        let overage = used_percent.map(|p| p > 100.0).unwrap_or(false);
        RateWindow {
            kind,
            label,
            used_percent,
            resets_at,
            overage,
        }
    }
}
