//! E2E: tool absence renders NotInstalled (invariant #13). Own file: owns its env.
//! Also pins the `MUSE_AUTH_PATH` override semantics (EDGE-1): a set-but-missing
//! path is NotInstalled, and a set path wins over the empty fake home.

use runwaybar::cli::{run_status, Format, StatusOpts};
use runwaybar::model::Status;

fn fixture_home() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home")
}

fn drop_cache() {
    // run_status serves a fresh cache without polling; drop it so each phase
    // below re-polls live (filename must match store::cache_path exactly).
    let state = std::env::var("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap()
        .join("runwaybar/last-good.json");
    let _ = std::fs::remove_file(&state);
}

#[test]
fn absent_tools_render_not_installed() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("RUNWAYBAR_FAKE_HOME", tmp.path()); // empty home: nothing installed
    std::env::set_var("XDG_CACHE_HOME", tmp.path().join("cache"));
    std::env::set_var("XDG_CONFIG_HOME", tmp.path().join("config"));
    std::env::set_var("RUNWAYBAR_TEST_ALLOW_HTTP", "1");
    // Full env isolation: private runtime dir (socket/lock), no inherited provider keys.
    let runtime_dir = std::env::temp_dir().join(format!("rwb-test-{}", std::process::id()));
    std::env::set_var("XDG_RUNTIME_DIR", runtime_dir);
    for var in [
        "Z_AI_API_KEY",
        "BIGMODEL_API_KEY",
        "ZHIPU_API_KEY",
        "ZHIPUAI_API_KEY",
        "GLM_API_KEY",
        "OPENCODE_API_KEY",
        "CODEX_HOME",
        "CODEX_USAGE_ENDPOINT",
        "CLAUDE_USAGE_ENDPOINT",
        "Z_AI_QUOTA_ENDPOINT",
        "Z_AI_QUOTA_CN_ENDPOINT",
        "OPENCODE_USAGE_ENDPOINT",
        "MUSE_AUTH_PATH",
        "MUSE_SUBSCRIPTION_ENDPOINT",
    ] {
        std::env::remove_var(var);
    }
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

    // Phase 2 (EDGE-1): a set-but-missing override is NotInstalled, never
    // MissingCredential — checks apply to the resolved path.
    drop_cache();
    std::env::set_var("MUSE_AUTH_PATH", tmp.path().join("no-such-auth.json"));
    let snap = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .unwrap();
    assert!(
        matches!(snap.provider("muse").unwrap().status, Status::NotInstalled),
        "set-but-missing MUSE_AUTH_PATH should be not_installed"
    );

    // Phase 3: the override wins over the (empty) fake home. The endpoint is
    // a closed ephemeral loopback port (bound then dropped), so discovery
    // succeeds and the poll fails closed with a typed network-failure —
    // proving the override file was read without any real egress.
    drop_cache();
    let closed_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    std::env::set_var(
        "MUSE_AUTH_PATH",
        fixture_home().join(".config/muse/auth.json"),
    );
    std::env::set_var(
        "MUSE_SUBSCRIPTION_ENDPOINT",
        format!("http://127.0.0.1:{closed_port}/unreachable"),
    );
    let snap = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .unwrap();
    match &snap.provider("muse").unwrap().status {
        Status::Error { class, .. } => assert_eq!(
            class, "network-failure",
            "override file should be read (network failure, not not_installed)"
        ),
        other => panic!("expected typed network-failure, got {other:?}"),
    }
}
