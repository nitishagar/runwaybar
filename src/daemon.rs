//! The resident daemon: poll scheduler + tray + IPC + notifications.
//!
//! Exit-path hygiene: the flock is kernel-released on any death (crash included);
//! the socket file is unlinked before every bind, so a crash cannot wedge startup.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::model::Snapshot;
use crate::state::{provider_level, HubState};
use crate::tray::{menu_tree_text, RunwayTray};

pub struct ServeOpts {
    pub no_tray: bool,
    pub dry_run: bool,
    pub interval: Option<u64>,
}

pub fn serve(opts: ServeOpts) -> i32 {
    let config = match crate::config::Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("runwaybar: {e}; continuing with defaults");
            Config::default()
        }
    };
    let interval = opts
        .interval
        .map(|s| {
            Duration::from_secs(s.clamp(
                crate::config::MIN_INTERVAL_SECS,
                crate::config::MAX_INTERVAL_SECS,
            ))
        })
        .unwrap_or(config.interval);

    let seed = crate::store::read_last_good().unwrap_or_else(|| Snapshot {
        schema_version: crate::model::SCHEMA_VERSION,
        generated_at: "1970-01-01T00:00:00Z".to_string(),
        providers: vec![],
        cooldowns: Default::default(),
    });

    if opts.dry_run {
        let mut hub = HubState::new(seed, interval);
        hub.refresh_staleness();
        println!("{}", menu_tree_text(&hub.snapshot));
        println!("tooltip:\n{}", crate::tray::tooltip_text(&hub.snapshot));
        return 0;
    }

    // Single instance (kernel-released flock).
    let _guard = match crate::singleinst::acquire() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("runwaybar: {e}");
            return 0;
        }
    };

    let state = Arc::new(Mutex::new(HubState::new(seed, interval)));
    let shutdown = Arc::new(AtomicBool::new(false));
    let refresh: Arc<(Mutex<bool>, Condvar)> = Arc::new((Mutex::new(false), Condvar::new()));

    // IPC server (same-user socket in the 0700 runtime dir).
    let ipc_state = state.clone();
    let ipc_refresh = refresh.clone();
    let ipc_shutdown = shutdown.clone();
    std::thread::spawn(move || {
        crate::ipc::serve_loop(ipc_state, ipc_refresh, ipc_shutdown);
    });

    // Tray (optional; absence of an SNI host is not fatal — invariant #14).
    let tray_handle = if opts.no_tray {
        None
    } else {
        spawn_tray(state.clone(), shutdown.clone(), refresh.clone())
    };

    let mut policy = crate::notify::NotifyPolicy::new();
    let enabled_ids: Vec<String> = config
        .enabled
        .iter()
        .filter(|(_, v)| **v)
        .map(|(k, _)| k.clone())
        .collect();

    // First poll immediately (time-to-first-data), then on the cadence.
    do_poll(
        &state,
        &config,
        &enabled_ids,
        true,
        tray_handle.as_ref(),
        &mut policy,
        config.notifications,
    );
    let mut next_due = Instant::now() + interval;
    loop {
        {
            let (lock, cvar) = &*refresh;
            let Ok(flag) = lock.lock() else { break };
            let now = Instant::now();
            if now < next_due && !*flag && !shutdown.load(Ordering::SeqCst) {
                let (_guard, _timeout) = cvar
                    .wait_timeout(flag, next_due - now)
                    .unwrap_or_else(|e| e.into_inner());
            }
        }
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let user_initiated = take_refresh(&refresh);
        do_poll(
            &state,
            &config,
            &enabled_ids,
            user_initiated,
            tray_handle.as_ref(),
            &mut policy,
            config.notifications,
        );
        next_due = Instant::now() + interval;
    }

    if let Some(h) = &tray_handle {
        h.shutdown().wait();
    }
    0
}

fn take_refresh(refresh: &Arc<(Mutex<bool>, Condvar)>) -> bool {
    let (lock, _) = &**refresh;
    let mut guard = match lock.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    std::mem::replace(&mut *guard, false)
}

fn spawn_tray(
    state: Arc<Mutex<HubState>>,
    shutdown: Arc<AtomicBool>,
    refresh: Arc<(Mutex<bool>, Condvar)>,
) -> Option<ksni::blocking::Handle<RunwayTray>> {
    use ksni::blocking::TrayMethods;
    let tray = RunwayTray {
        state,
        shutdown,
        refresh,
    };
    match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!(
                "runwaybar: no system tray host available ({e}); running without the icon. \
                 The CLI (`runwaybar status`) and IPC keep working."
            );
            None
        }
    }
}

fn do_poll(
    state: &Arc<Mutex<HubState>>,
    config: &Config,
    enabled_ids: &[String],
    user_initiated: bool,
    tray_handle: Option<&ksni::blocking::Handle<RunwayTray>>,
    policy: &mut crate::notify::NotifyPolicy,
    notify_level: crate::config::NotifyLevel,
) {
    // Background polls honour cooldowns; a user-initiated refresh bypasses them once.
    let poll_ids = if user_initiated {
        enabled_ids.to_vec()
    } else {
        let now = crate::timefmt::now_epoch_ms();
        state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pollable_now(enabled_ids, now)
    };
    if poll_ids.is_empty() {
        return;
    }
    let mut filtered = config.clone();
    filtered.enabled.retain(|id, _| poll_ids.contains(id));
    let fresh = crate::providers::poll_all(Arc::new(filtered));
    let levels: Vec<(String, crate::state::Level, String)> = fresh
        .providers
        .iter()
        .map(|p| {
            let level = provider_level(p);
            let detail = match level {
                crate::state::Level::Warning | crate::state::Level::Critical => {
                    format!(
                        "{:.0}% of a rate window in use",
                        p.worst_used_percent().unwrap_or(0.0)
                    )
                }
                _ => p.status_text().to_string(),
            };
            (p.id.clone(), level, detail)
        })
        .collect();
    {
        let mut hub = state.lock().unwrap_or_else(|e| e.into_inner());
        hub.apply_poll(fresh);
        let _ = crate::store::write_last_good(&hub.snapshot);
    }
    if let Some(h) = tray_handle {
        h.update(|_| {}); // push icon/tooltip/menu to the host
    }
    for (summary, body) in policy.collect(&levels, notify_level) {
        crate::notify::send(&summary, &body);
    }
}
