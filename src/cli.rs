//! `status` command: daemon socket (Stage 2) → fresh cache → live one-shot poll.
//! Render formats: json / text / waybar.

use crate::error::ProviderError;
use crate::model::{Snapshot, Status, WindowKind};
use crate::timefmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum Format {
    Json,
    Text,
    Waybar,
}

pub struct StatusOpts {
    pub format: Format,
    pub providers: Vec<String>,
    pub no_fetch: bool,
}

/// Test/e2e hook: render this file instead of any live/cache path.
pub const SNAPSHOT_FILE_ENV: &str = "RUNWAYBAR_SNAPSHOT_FILE";

pub fn run_status(opts: StatusOpts) -> Result<Snapshot, ProviderError> {
    // 1. Injected snapshot (tests / offline e2e).
    if let Ok(path) = std::env::var(SNAPSHOT_FILE_ENV) {
        let raw = std::fs::read_to_string(&path).map_err(|e| {
            ProviderError::new(
                crate::error::ErrorClass::ParseFailure,
                format!("snapshot file: {e}"),
            )
        })?;
        let snap: Snapshot = serde_json::from_str(&raw).map_err(|e| {
            ProviderError::new(
                crate::error::ErrorClass::ParseFailure,
                format!("snapshot json: {e}"),
            )
        })?;
        return Ok(filter(snap, &opts.providers));
    }

    // 2. Daemon socket (Stage 2 provides the server; absent here → skip silently).
    if let Some(snap) = crate::ipc::try_request_status() {
        return Ok(filter(snap, &opts.providers));
    }

    // 3. Fresh cache. An invalid config is REPORTED (never silently swapped).
    let config = load_config_or_report();
    if let Some(cached) = crate::store::read_last_good() {
        if crate::store::is_fresh(&cached, config.interval) {
            return Ok(filter(cached, &opts.providers));
        }
        if opts.no_fetch {
            return Ok(filter(mark_stale(cached), &opts.providers));
        }
    } else if opts.no_fetch {
        return Ok(no_data_snapshot(
            "--no-fetch set, no daemon and no cached data",
        ));
    }

    // 4. Live one-shot (cache-first: only on staleness when no daemon owns the data).
    // A provider that errors keeps its previous (cached) windows marked stale — a
    // failed poll must never destroy last-good data.
    let config = std::sync::Arc::new(config);
    let snap = crate::providers::poll_all(config.clone());
    let merged = crate::store::merge_with_last_good(snap);
    let _ = crate::store::write_last_good(&merged);
    Ok(filter(merged, &opts.providers))
}

/// User-initiated refresh bypasses cache and cooldowns exactly once (invariant #6).
pub fn force_refresh() -> Result<Snapshot, ProviderError> {
    let config = std::sync::Arc::new(load_config_or_report());
    let snap = crate::providers::poll_all(config);
    let merged = crate::store::merge_with_last_good(snap);
    let _ = crate::store::write_last_good(&merged);
    Ok(merged)
}

fn load_config_or_report() -> crate::config::Config {
    match crate::config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("runwaybar: {e}; continuing with defaults");
            crate::config::Config::default()
        }
    }
}

/// Honest absence of data: stale with no windows — never fabricated not_installed/0%.
fn no_data_snapshot(reason: &str) -> Snapshot {
    Snapshot {
        schema_version: crate::model::SCHEMA_VERSION,
        generated_at: timefmt::now_rfc3339(),
        providers: crate::providers::registry()
            .into_iter()
            .map(|p| crate::model::ProviderSnapshot {
                id: p.id().to_string(),
                label: p.label().to_string(),
                status: Status::Stale {
                    since: timefmt::now_rfc3339(),
                    reason: Some(reason.to_string()),
                },
                account: None,
                windows: vec![],
            })
            .collect(),
        cooldowns: Default::default(),
    }
}

fn mark_stale(mut snap: Snapshot) -> Snapshot {
    for p in &mut snap.providers {
        if let Status::Ok = p.status {
            p.status = Status::Stale {
                since: timefmt::now_rfc3339(),
                reason: Some("no daemon running; cache older than the freshness window".into()),
            };
        }
    }
    snap
}

fn filter(mut snap: Snapshot, ids: &[String]) -> Snapshot {
    if ids.is_empty() {
        return snap;
    }
    snap.providers.retain(|p| ids.iter().any(|i| i == &p.id));
    snap
}

// ---- renders ----

pub fn render_json(snap: &Snapshot) -> String {
    serde_json::to_string_pretty(snap).unwrap_or_else(|_| "{}".to_string())
}

pub fn render_text(snap: &Snapshot) -> String {
    let now_ms = timefmt::now_epoch_ms();
    let mut out = String::new();
    for p in &snap.providers {
        let head = match &p.status {
            Status::Ok => format!("{}: ok", p.label),
            Status::Stale { reason, .. } => format!(
                "{}: stale ({})",
                p.label,
                reason.as_deref().unwrap_or("previous data")
            ),
            Status::Error { class, message } => format!("{}: {} — {}", p.label, class, message),
            Status::NotInstalled => format!("{}: not installed", p.label),
        };
        out.push_str(&head);
        out.push('\n');
        if let Some(acct) = &p.account {
            out.push_str(&format!("  account: {}\n", sanitize(acct)));
        }
        for w in &p.windows {
            let pct = w
                .used_percent
                .map(|p| format!("{p:.0}%"))
                .unwrap_or_else(|| "unknown".to_string());
            let reset = w
                .resets_at
                .as_deref()
                .and_then(|r| timefmt::countdown(r, now_ms))
                .map(|c| format!(", resets in {c}"))
                .unwrap_or_default();
            let overage = if w.overage { " (over limit)" } else { "" };
            out.push_str(&format!(
                "  {:<8} {pct}{reset}{overage}\n",
                kind_str(w.kind)
            ));
        }
    }
    out
}

fn kind_str(k: WindowKind) -> &'static str {
    match k {
        WindowKind::Session => "session",
        WindowKind::Weekly => "weekly",
        WindowKind::Monthly => "monthly",
        WindowKind::Other => "other",
    }
}

/// Waybar custom-module JSON: one line, worst runway + class for CSS.
pub fn render_waybar(snap: &Snapshot) -> String {
    let now_ms = timefmt::now_epoch_ms();
    let worst = snap
        .providers
        .iter()
        .filter_map(|p| p.worst_used_percent().map(|v| (p, v)))
        .max_by(|a, b| a.1.total_cmp(&b.1));
    let (text, class) = match worst {
        Some((_p, v)) => {
            let t = format!("{v:.0}%{}", if v > 100.0 { "!" } else { "" });
            let c = if v >= 85.0 {
                "runway-critical"
            } else if v >= 60.0 {
                "runway-warning"
            } else {
                "runway-ok"
            };
            (t, c)
        }
        None => ("—".to_string(), "runway-unknown"),
    };
    let tooltip = snap
        .providers
        .iter()
        .map(|p| {
            let windows: Vec<String> = p
                .windows
                .iter()
                .map(|w| {
                    let pct = w
                        .used_percent
                        .map(|v| format!("{v:.0}%"))
                        .unwrap_or_else(|| "?".into());
                    let reset = w
                        .resets_at
                        .as_deref()
                        .and_then(|r| timefmt::countdown(r, now_ms))
                        .map(|c| format!(" ({c})"))
                        .unwrap_or_default();
                    format!("{} {}{}", kind_str(w.kind), pct, reset)
                })
                .collect();
            if windows.is_empty() {
                format!("{}: {}", p.label, short_status(&p.status))
            } else {
                format!("{}: {}", p.label, windows.join(" · "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    serde_json::json!({"text": text, "tooltip": tooltip, "class": class}).to_string()
}

/// Remote-derived strings (plan names, labels) reach terminal output: strip
/// control characters so a hostile response cannot inject ANSI escapes.
fn sanitize(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

fn short_status(s: &Status) -> String {
    match s {
        Status::Ok => "ok".into(),
        Status::Stale { .. } => "stale".into(),
        Status::Error { class, .. } => format!("error ({class})"),
        Status::NotInstalled => "not installed".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderSnapshot, RateWindow, SCHEMA_VERSION};

    fn sample() -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION,
            generated_at: "2026-09-23T10:00:00Z".to_string(),
            providers: vec![
                ProviderSnapshot {
                    id: "claude-code".into(),
                    label: "Claude Code".into(),
                    status: Status::Ok,
                    account: Some("Pro".into()),
                    windows: vec![
                        RateWindow::new(WindowKind::Session, None, Some(62.5), None),
                        RateWindow::new(WindowKind::Weekly, None, Some(31.0), None),
                    ],
                },
                ProviderSnapshot {
                    id: "codex".into(),
                    label: "Codex".into(),
                    status: Status::NotInstalled,
                    account: None,
                    windows: vec![],
                },
            ],
            cooldowns: Default::default(),
        }
    }

    #[test]
    fn text_render_shows_unknown_and_silos_accounts() {
        let text = render_text(&sample());
        assert!(text.contains("Claude Code: ok"));
        assert!(text.contains("session  62%"));
        assert!(text.contains("Codex: not installed"));
        // siloing: the account appears once, under its own provider block only
        assert_eq!(text.matches("Pro").count(), 1);
    }

    #[test]
    fn waybar_render_takes_worst_and_class() {
        let w = render_waybar(&sample());
        let v: serde_json::Value = serde_json::from_str(&w).unwrap();
        assert_eq!(v["text"], "62%");
        assert_eq!(v["class"], "runway-warning"); // 62.5% falls in the >=60 band
        assert!(v["tooltip"].as_str().unwrap().contains("Claude Code"));
    }

    #[test]
    fn waybar_unknown_when_no_data() {
        let w = render_waybar(&Snapshot {
            schema_version: SCHEMA_VERSION,
            generated_at: "2026-09-23T10:00:00Z".into(),
            providers: vec![ProviderSnapshot {
                id: "x".into(),
                label: "X".into(),
                status: Status::Error {
                    class: "network-failure".into(),
                    message: "down".into(),
                },
                account: None,
                windows: vec![],
            }],
            cooldowns: Default::default(),
        });
        let v: serde_json::Value = serde_json::from_str(&w).unwrap();
        assert_eq!(v["class"], "runway-unknown");
        assert!(v["tooltip"]
            .as_str()
            .unwrap()
            .contains("error (network-failure)"));
    }

    #[test]
    fn filter_keeps_only_named_providers() {
        let s = filter(sample(), &["codex".to_string()]);
        assert_eq!(s.providers.len(), 1);
        assert_eq!(s.providers[0].id, "codex");
    }

    #[test]
    fn json_render_roundtrips() {
        let json = render_json(&sample());
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, sample());
    }
}
