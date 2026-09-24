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

// Full happy-path fixture: active subscription + tier, window with reset,
// weekly plain. Includes distractor secret fields the parse path must drop.
fn muse_body() -> String {
    r#"{"is_subs_active": true, "subs_tier_name": "Muse Code Test Tier",
        "api_key": "fixture-muse-apikey", "user_email": "muse-user@example.com",
        "user_full_name": "Muse Fixture User", "subs_usage": {
        "window": {"used_percent": 18.0, "window_duration_mins": 300, "resets_at": 1789068250},
        "weekly": {"used_percent": 9.0}
    }}"#
    .to_string()
}

#[test]
fn one_burst_then_cache_served() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("RUNWAYBAR_FAKE_HOME", fixture_home());
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

    let server = MockServer::start(vec![
        ("/claude".into(), 200, claude_body()),
        ("/codex".into(), 200, codex_body()),
        ("/zai".into(), 200, zai_body()),
        ("/opencode".into(), 200, opencode_body()),
        ("/muse".into(), 200, muse_body()),
    ]);
    std::env::set_var("CLAUDE_USAGE_ENDPOINT", format!("{}/claude", server.base));
    std::env::set_var("CODEX_USAGE_ENDPOINT", format!("{}/codex", server.base));
    std::env::set_var("Z_AI_QUOTA_ENDPOINT", format!("{}/zai", server.base));
    std::env::set_var(
        "OPENCODE_USAGE_ENDPOINT",
        format!("{}/opencode", server.base),
    );
    std::env::set_var(
        "MUSE_SUBSCRIPTION_ENDPOINT",
        format!("{}/muse", server.base),
    );

    // First call: live burst (5 providers → 5 requests), cache written.
    let snap1 = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .expect("first status");
    assert_eq!(snap1.providers.len(), 5, "all five providers should appear");
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
    let muse = snap1.provider("muse").unwrap();
    assert!(
        matches!(muse.status, Status::Ok),
        "muse status was {muse:?}"
    );
    assert_eq!(muse.worst_used_percent(), Some(18.0));
    assert_eq!(muse.account.as_deref(), Some("Muse Code Test Tier"));
    // The muse poll is exactly one POST to /muse carrying {} and the pinned
    // API version, with the OAuth token as a Bearer credential.
    let muse_reqs: Vec<_> = server
        .requests_snapshot()
        .into_iter()
        .filter(|r| r.path == "/muse")
        .collect();
    assert_eq!(muse_reqs.len(), 1, "expected one muse poll");
    let req = &muse_reqs[0];
    assert_eq!(req.method, "POST");
    assert_eq!(req.body, "{}");
    let header = |name: &str| {
        req.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(header("x-api-version"), Some("1.0.0"));
    assert_eq!(header("authorization"), Some("Bearer fixture-muse-token"));
    let after_first = server.count();
    assert!(
        after_first <= 5,
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
        "fixture-muse-token",
        "fixture-muse-apikey",
        "muse-user@example.com",
        "Muse Fixture User",
        "fixture-refresh",
    ] {
        assert!(
            !json.contains(secret),
            "secret leaked into snapshot JSON: {secret}"
        );
    }
}
