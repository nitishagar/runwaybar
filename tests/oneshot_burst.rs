//! E2E: discovery → poll → parse → cache-first one-shot behaviour against a loopback
//! mock server with an isolated HOME. One test per file: this process owns its env.

mod common;

use common::MockServer;
use runwaybar::cli::{run_status, Format, StatusOpts};
use runwaybar::model::Status;

fn fixture_home() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home")
}

fn claude_body() -> String {
    r#"{"usage": [
        {"key": "five_hour", "used_pct": 62.5, "resets_in_sec": 7200},
        {"key": "seven_day", "used_percent": 31.0}
    ], "plan": {"type": "pro"}}"#
        .to_string()
}

fn codex_body() -> String {
    r#"{"plan_type": "plus", "rate_limit": {
        "primary_window": {"used_percent": 44.0, "window_minutes": 300, "resets_at": 1759000000},
        "secondary_window": {"used_percent": 12.0, "window_minutes": 10080}
    }}"#
    .to_string()
}

fn zai_body() -> String {
    r#"{"data": {"limits": [
        {"type": "TOKENS_LIMIT", "unit": 3, "percentage": 42.0, "nextResetTime": 1758900000000},
        {"type": "TOKENS_LIMIT", "unit": 6, "percentage": 10.0}
    ]}}"#
        .to_string()
}

fn opencode_body() -> String {
    r#"{"usage": {"rolling": {"percent": 55, "resetInSec": 3600}, "weekly": {"percent": 12}}}"#
        .to_string()
}

#[test]
fn one_burst_then_cache_served() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("RUNWAYBAR_FAKE_HOME", fixture_home());
    std::env::set_var("XDG_CACHE_HOME", tmp.path().join("cache"));
    std::env::set_var("XDG_CONFIG_HOME", tmp.path().join("config"));
    std::env::set_var("RUNWAYBAR_TEST_ALLOW_HTTP", "1");

    let server = MockServer::start(vec![
        ("/claude".into(), 200, claude_body()),
        ("/codex".into(), 200, codex_body()),
        ("/zai".into(), 200, zai_body()),
        ("/opencode".into(), 200, opencode_body()),
    ]);
    std::env::set_var("CLAUDE_USAGE_ENDPOINT", format!("{}/claude", server.base));
    std::env::set_var("CODEX_USAGE_ENDPOINT", format!("{}/codex", server.base));
    std::env::set_var("Z_AI_QUOTA_ENDPOINT", format!("{}/zai", server.base));
    std::env::set_var(
        "OPENCODE_USAGE_ENDPOINT",
        format!("{}/opencode", server.base),
    );

    // First call: live burst (4 providers → 4 requests), cache written.
    let snap1 = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .expect("first status");
    assert_eq!(snap1.providers.len(), 4, "all four providers should appear");
    let claude = snap1.provider("claude-code").unwrap();
    assert!(
        matches!(claude.status, Status::Ok),
        "claude status was {claude:?}"
    );
    assert_eq!(claude.worst_used_percent(), Some(62.5));
    let codex = snap1.provider("codex").unwrap();
    assert!(
        matches!(codex.status, Status::Ok),
        "codex status was {codex:?}"
    );
    assert_eq!(codex.worst_used_percent(), Some(44.0));
    let zai = snap1.provider("zai").unwrap();
    assert!(matches!(zai.status, Status::Ok), "zai status was {zai:?}");
    assert_eq!(zai.worst_used_percent(), Some(42.0));
    let after_first = server.count();
    assert!(
        after_first <= 4,
        "first burst should poll each provider at most once, got {after_first}"
    );

    // Second + third calls: cache-first — no additional network traffic.
    let _snap2 = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .unwrap();
    let _snap3 = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .unwrap();
    assert_eq!(
        server.count(),
        after_first,
        "cache-first violated: extra requests issued"
    );

    // No secret material ever reaches rendered output.
    let json = runwaybar::cli::render_json(&_snap2);
    for secret in [
        "fixture-claude-token",
        "fixture-codex-token",
        "fixture-zai-key",
        "fixture-opencode-key",
        "fixture-refresh",
    ] {
        assert!(
            !json.contains(secret),
            "secret leaked into snapshot JSON: {secret}"
        );
    }
}
