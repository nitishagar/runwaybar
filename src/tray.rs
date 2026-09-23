//! SNI tray via ksni (blocking). Menu/tooltip/icon are derived from HubState on each
//! `update()`; every state change calls `handle.update()` (advisor A5).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::model::{Snapshot, Status, WindowKind};
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
                let pct = w
                    .used_percent
                    .map(|v| format!("{v:.0}%"))
                    .unwrap_or_else(|| "unknown".into());
                let reset = w
                    .resets_at
                    .as_deref()
                    .and_then(|r| timefmt::countdown(r, now_ms))
                    .map(|c| format!(", resets in {c}"))
                    .unwrap_or_default();
                let overage = if w.overage { " (over limit)" } else { "" };
                items.push(MenuItem::Standard(ksni::menu::StandardItem {
                    label: format!("    {} {pct}{reset}{overage}", kind_str(w.kind)),
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

pub fn worst_percent(snap: &Snapshot) -> Option<f64> {
    snap.providers
        .iter()
        .filter_map(|p| p.worst_used_percent())
        .fold(None::<f64>, |acc: Option<f64>, v| {
            Some(acc.map_or(v, |a| a.max(v)))
        })
}

/// Tooltip body: one line per provider, ≤ ~200 chars total.
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
                    format!("{} {}{}", kind_str(w.kind), pct, reset)
                })
                .collect();
            if windows.is_empty() {
                format!("{}: {}", p.label, crate::state::provider_level(p).as_str())
            } else {
                format!("{}: {}", p.label, windows.join(" · "))
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
            let pct = w
                .used_percent
                .map(|v| format!("{v:.0}%"))
                .unwrap_or_else(|| "unknown".into());
            let reset = w
                .resets_at
                .as_deref()
                .and_then(|r| timefmt::countdown(r, now_ms))
                .map(|c| format!(", resets in {c}"))
                .unwrap_or_default();
            out.push_str(&format!("    {} {pct}{reset}\n", kind_str(w.kind)));
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
}
