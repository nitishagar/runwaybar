//! SNI tray via ksni (blocking). Menu/tooltip/icon are derived from HubState on each
//! `update()`; every state change calls `handle.update()` (advisor A5).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::cli::{sanitize, usage_bar};
use crate::model::{ProviderSnapshot, Snapshot, Status, WindowKind};
use crate::state::HubState;
use crate::timefmt;
use ksni::menu::MenuItem;
use ksni::Tray;

pub struct RunwayTray {
    pub state: Arc<Mutex<HubState>>,
    pub shutdown: Arc<AtomicBool>,
    pub refresh: Arc<(Mutex<bool>, Condvar)>,
}

impl RunwayTray {
    fn snapshot(&self) -> Snapshot {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot
            .clone()
    }

    fn request_refresh(&self) {
        let (lock, cvar) = &*self.refresh;
        if let Ok(mut flag) = lock.lock() {
            *flag = true;
        }
        cvar.notify_all();
    }
}

impl Tray for RunwayTray {
    fn id(&self) -> String {
        "in.applair.RunwayBar".into()
    }

    fn title(&self) -> String {
        "RunwayBar".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "RunwayBar".into(),
            description: tooltip_text(&self.snapshot()),
            ..Default::default()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![crate::icon::draw(worst_percent(&self.snapshot()))]
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let snap = self.snapshot();
        let now_ms = timefmt::now_epoch_ms();
        let mut items: Vec<MenuItem<Self>> = Vec::new();
        for p in &snap.providers {
            let status = match &p.status {
                Status::Ok => p
                    .worst_used_percent()
                    .map(|v| format!("{v:.0}%"))
                    .unwrap_or_else(|| "ok".into()),
                Status::Stale { .. } => "stale".into(),
                Status::Error { class, .. } => format!("error ({class})"),
                Status::NotInstalled => "not installed".into(),
            };
            let header = ksni::menu::StandardItem {
                label: format!("{} — {status}", p.label),
                enabled: false,
                ..Default::default()
            };
            items.push(MenuItem::Standard(header));
            for w in &p.windows {
                items.push(MenuItem::Standard(ksni::menu::StandardItem {
                    label: format!("    {}", window_line(&p.status, w, now_ms, "unknown")),
                    enabled: false,
                    ..Default::default()
                }));
            }
            // Account after the windows so the header→first-window adjacency
            // asserted by `menu_tree_structure` is undisturbed.
            if let Some(acct) = account_line(p) {
                items.push(MenuItem::Standard(ksni::menu::StandardItem {
                    label: format!("    account: {acct}"),
                    enabled: false,
                    ..Default::default()
                }));
            }
        }
        items.push(MenuItem::Separator);
        items.push(MenuItem::Standard(ksni::menu::StandardItem {
            label: "Refresh now".into(),
            activate: Box::new(|tray: &mut Self| tray.request_refresh()),
            ..Default::default()
        }));
        items.push(MenuItem::Standard(ksni::menu::StandardItem {
            label: format!("About RunwayBar {}", crate::VERSION),
            enabled: false,
            ..Default::default()
        }));
        items.push(MenuItem::Standard(ksni::menu::StandardItem {
            label: "Quit".into(),
            activate: Box::new(|tray: &mut Self| {
                tray.shutdown.store(true, Ordering::SeqCst);
                let (_, cvar) = &*tray.refresh;
                cvar.notify_all();
            }),
            ..Default::default()
        }));
        items
    }
}

fn kind_str(k: WindowKind) -> &'static str {
    match k {
        WindowKind::Session => "session",
        WindowKind::Weekly => "weekly",
        WindowKind::Monthly => "monthly",
        WindowKind::Other => "other",
    }
}

/// `% left` fragment, computed at render time (never stored — schema stays 1).
fn left_str(used_percent: Option<f64>) -> String {
    used_percent
        .map(|v| format!(" · {:.0}% left", (100.0 - v).max(0.0)))
        .unwrap_or_default()
}

/// Per-window badge for non-Ok providers.
fn window_badge(status: &Status) -> String {
    match status {
        Status::Ok => String::new(),
        Status::Stale { .. } => " [stale]".to_string(),
        Status::Error { class, .. } => format!(" [error ({class})]"),
        Status::NotInstalled => " [not installed]".to_string(),
    }
}

/// Multiline window line shared by `menu()` and `menu_tree_text` (the two
/// paths must not diverge again): `session 62% · 38% left · resets in 1h 5m`.
/// `unknown_pct` is `"unknown"` (menu/tree) or `"?"` (tooltips).
fn window_line(
    status: &Status,
    w: &crate::model::RateWindow,
    now_ms: i64,
    unknown_pct: &str,
) -> String {
    let pct = w
        .used_percent
        .map(|v| format!("{v:.0}%"))
        .unwrap_or_else(|| unknown_pct.into());
    let reset = w
        .resets_at
        .as_deref()
        .and_then(|r| timefmt::countdown(r, now_ms))
        .map(|c| format!(" · resets in {c}"))
        .unwrap_or_default();
    let overage = if w.overage { " (over limit)" } else { "" };
    format!(
        "{} {pct}{}{reset}{overage}{}",
        kind_str(w.kind),
        left_str(w.used_percent),
        window_badge(status),
    )
}

/// Sanitized per-provider account line, or None when absent.
/// EDGE-5: a not-installed provider must stay quiet — no account leak.
fn account_line(p: &ProviderSnapshot) -> Option<String> {
    match &p.account {
        Some(a) if !matches!(p.status, Status::NotInstalled) => Some(sanitize(a)),
        _ => None,
    }
}

pub fn worst_percent(snap: &Snapshot) -> Option<f64> {
    snap.providers
        .iter()
        .filter_map(|p| p.worst_used_percent())
        .fold(None::<f64>, |acc: Option<f64>, v| {
            Some(acc.map_or(v, |a| a.max(v)))
        })
}

/// Tooltip body: one line per provider, ≤ ~160 chars/line, ≤ ~640 total.
/// Measured worst case is 154 chars for a 2-window provider with bars,
/// `% left`, account, and badges; four-provider total is ~390 chars.
/// Window bars are fixed 10-cell `▓░`; `% left` is computed, never stored.
pub fn tooltip_text(snap: &Snapshot) -> String {
    let now_ms = timefmt::now_epoch_ms();
    snap.providers
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
                    let overage = if w.overage { " (over limit)" } else { "" };
                    format!(
                        "{} {pct} {}{}{reset}{overage}{}",
                        kind_str(w.kind),
                        usage_bar(w.used_percent),
                        left_str(w.used_percent),
                        window_badge(&p.status),
                    )
                })
                .collect();
            let account = account_line(p)
                .map(|a| format!(" · account: {a}"))
                .unwrap_or_default();
            if windows.is_empty() {
                format!(
                    "{}: {}{account}",
                    p.label,
                    crate::state::provider_level(p).as_str()
                )
            } else {
                format!("{}: {}{account}", p.label, windows.join(" · "))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Headless harness output: the menu tree as text (golden-testable without D-Bus).
pub fn menu_tree_text(snap: &Snapshot) -> String {
    let now_ms = timefmt::now_epoch_ms();
    let mut out = String::new();
    for p in &snap.providers {
        let status = match &p.status {
            Status::Ok => p
                .worst_used_percent()
                .map(|v| format!("{v:.0}%"))
                .unwrap_or_else(|| "ok".into()),
            Status::Stale { .. } => "stale".into(),
            Status::Error { class, .. } => format!("error ({class})"),
            Status::NotInstalled => "not installed".into(),
        };
        out.push_str(&format!("{} — {status}\n", p.label));
        for w in &p.windows {
            out.push_str(&format!(
                "    {}\n",
                window_line(&p.status, w, now_ms, "unknown")
            ));
        }
        if let Some(acct) = account_line(p) {
            out.push_str(&format!("    account: {acct}\n"));
        }
    }
    out.push_str("---\nRefresh now\nAbout\nQuit\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderSnapshot, RateWindow, SCHEMA_VERSION};

    fn snap() -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION,
            generated_at: timefmt::now_rfc3339(),
            providers: vec![
                ProviderSnapshot {
                    id: "claude-code".into(),
                    label: "Claude Code".into(),
                    status: Status::Ok,
                    account: None,
                    windows: vec![RateWindow::new(WindowKind::Session, None, Some(62.0), None)],
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
    fn worst_percent_across_providers() {
        assert_eq!(worst_percent(&snap()), Some(62.0));
    }

    #[test]
    fn menu_tree_structure() {
        let tree = menu_tree_text(&snap());
        assert!(
            tree.contains("Claude Code — 62%\n    session 62%"),
            "tree:\n{tree}"
        );
        assert!(tree.contains("Codex — not installed\n"));
        assert!(tree.contains("---\nRefresh now\nAbout\nQuit\n"));
    }

    #[test]
    fn tooltip_lists_providers() {
        let tip = tooltip_text(&snap());
        assert!(tip.contains("Claude Code: session 62%"));
        assert!(tip.contains("Codex: not installed"));
    }

    fn rich_snap() -> Snapshot {
        Snapshot {
            schema_version: SCHEMA_VERSION,
            generated_at: timefmt::now_rfc3339(),
            providers: vec![ProviderSnapshot {
                id: "x".into(),
                label: "X".into(),
                status: Status::Stale {
                    since: timefmt::now_rfc3339(),
                    reason: Some("poll failed".into()),
                },
                account: Some("Pro\u{7}\u{1b}[2J".into()),
                windows: vec![
                    RateWindow::new(WindowKind::Session, None, Some(62.0), None),
                    RateWindow::new(WindowKind::Weekly, None, Some(105.0), None),
                    RateWindow::new(WindowKind::Monthly, None, None, None),
                ],
            }],
            cooldowns: Default::default(),
        }
    }

    #[test]
    fn menu_tree_percent_left_overage_and_stale_badge() {
        let tree = menu_tree_text(&rich_snap());
        assert!(tree.contains("session 62% · 38% left"), "tree:\n{tree}");
        assert!(tree.contains("105% · 0% left"), "tree:\n{tree}");
        assert!(tree.contains("(over limit)"), "tree:\n{tree}");
        assert!(tree.contains("[stale]"), "tree:\n{tree}");
        assert!(tree.contains("monthly unknown"), "tree:\n{tree}");
    }

    #[test]
    fn menu_tree_and_tooltip_sanitize_account() {
        let tree = menu_tree_text(&rich_snap());
        assert!(tree.contains("account: Pro[2J"), "tree:\n{tree}");
        assert!(!tree.contains('\u{7}'), "tree:\n{tree}");
        let tip = tooltip_text(&rich_snap());
        assert!(tip.contains("account: Pro[2J"), "tip:\n{tip}");
        assert!(!tip.contains('\u{7}'), "tip:\n{tip}");
    }

    #[test]
    fn tooltip_bars_and_left() {
        let tip = tooltip_text(&rich_snap());
        assert!(tip.contains("▓▓▓▓▓▓░░░░"), "tip:\n{tip}");
        assert!(tip.contains("38% left"), "tip:\n{tip}");
        assert!(tip.contains("? ░░░░░░░░░░"), "tip:\n{tip}");
        assert!(tip.contains("[stale]"), "tip:\n{tip}");
    }

    #[test]
    fn menu_tree_error_badge() {
        let mut snap = rich_snap();
        snap.providers[0].status = Status::Error {
            class: "net".into(),
            message: "down".into(),
        };
        let tree = menu_tree_text(&snap);
        assert!(tree.contains("[error (net)]"), "tree:\n{tree}");
        let tip = tooltip_text(&snap);
        assert!(tip.contains("[error (net)]"), "tip:\n{tip}");
    }
}
