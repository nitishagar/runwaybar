//! Cache atomicity + Config::load invalid-file behaviour. Own file: owns its env
//! (XDG_CACHE_HOME / XDG_CONFIG_HOME point at tempdirs).

use runwaybar::config::Config;
use runwaybar::model::{ProviderSnapshot, Snapshot, Status, SCHEMA_VERSION};
use runwaybar::store;

fn snap(pct: f64) -> Snapshot {
    Snapshot {
        schema_version: SCHEMA_VERSION,
        generated_at: runwaybar::timefmt::now_rfc3339(),
        providers: vec![ProviderSnapshot {
            id: "t".into(),
            label: "T".into(),
            status: Status::Ok,
            account: None,
            windows: vec![runwaybar::model::RateWindow::new(
                runwaybar::model::WindowKind::Session,
                None,
                Some(pct),
                None,
            )],
        }],
        cooldowns: Default::default(),
    }
}

#[test]
fn cache_and_config_behaviour() {
    // Phase 1: cache atomicity/perms (single test: the phases share process env).
    {
        cache_write_phase();
    }
    // Phase 2: concurrent writers.
    {
        concurrent_writers_phase();
    }
    // Phase 3: invalid config.
    {
        invalid_config_phase();
    }
}

fn cache_write_phase() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CACHE_HOME", tmp.path());
    store::write_last_good(&snap(42.0)).expect("write");
    let path = tmp.path().join("runwaybar").join("last-good.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "cache mode {mode:o}");
    }
    let back = store::read_last_good().expect("read back");
    assert_eq!(back.providers[0].worst_used_percent(), Some(42.0));
    // no pid-suffixed temp leftovers
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path().join("runwaybar"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
        .collect();
    assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
    // overwrite path (rename over existing) works
    store::write_last_good(&snap(90.0)).expect("overwrite");
    assert_eq!(
        store::read_last_good().unwrap().providers[0].worst_used_percent(),
        Some(90.0)
    );
}

fn concurrent_writers_phase() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CACHE_HOME", tmp.path());
    let handles: Vec<_> = (0..8)
        .map(|i| {
            std::thread::spawn(move || {
                for j in 0..5 {
                    store::write_last_good(&snap(i as f64 * 10.0 + j as f64)).expect("write");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let back = store::read_last_good().expect("final cache must be valid JSON");
    assert!(back.providers[0].worst_used_percent().is_some());
}

fn invalid_config_phase() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg_dir = tmp.path().join("cfg");
    std::fs::create_dir_all(cfg_dir.join("runwaybar")).unwrap();
    std::fs::write(cfg_dir.join("runwaybar").join("config.toml"), "interval = ").unwrap();
    std::env::set_var("XDG_CONFIG_HOME", cfg_dir.clone());
    match Config::load() {
        Err(e) => {
            let msg = format!("{e}");
            assert!(msg.contains("invalid"), "message: {msg}");
            assert!(msg.contains("left untouched"), "message: {msg}");
        }
        Ok(_) => panic!("invalid config must be an error"),
    }
    // file untouched
    assert_eq!(
        std::fs::read_to_string(cfg_dir.join("runwaybar").join("config.toml")).unwrap(),
        "interval = "
    );
    // first-run in a clean dir writes defaults 0600
    let fresh = tempfile::tempdir().unwrap();
    std::env::set_var("XDG_CONFIG_HOME", fresh.path());
    let cfg = Config::load().expect("defaults on first run");
    assert_eq!(cfg.interval, std::time::Duration::from_secs(300));
    let path = fresh.path().join("runwaybar").join("config.toml");
    assert!(path.exists(), "defaults file written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config mode {mode:o}");
    }
}
