//! E2E: tool absence renders NotInstalled (invariant #13). Own file: owns its env.

use runwaybar::cli::{run_status, Format, StatusOpts};
use runwaybar::model::Status;

#[test]
fn absent_tools_render_not_installed() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("RUNWAYBAR_FAKE_HOME", tmp.path()); // empty home: nothing installed
    std::env::set_var("XDG_CACHE_HOME", tmp.path().join("cache"));
    std::env::set_var("XDG_CONFIG_HOME", tmp.path().join("config"));
    std::env::set_var("RUNWAYBAR_TEST_ALLOW_HTTP", "1");
    // no endpoints overridden → no reachable server, but nothing is discoverable either,
    // so no requests happen and every provider reports NotInstalled.
    let snap = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .unwrap();
    for p in &snap.providers {
        assert!(
            matches!(p.status, Status::NotInstalled),
            "{} should be not_installed, got {:?}",
            p.id,
            p.status
        );
    }
}
