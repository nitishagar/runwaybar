//! E2E: the muse provider's full error taxonomy. Own file: owns its env.
//! Every phase re-points the endpoint/auth overrides, drops the cache, and
//! asserts the TYPED status class — each failure mode is pinned, not just
//! "some error". Auth phases use throwaway files; nothing touches the real
//! home, and META_API_KEY must be ignored end to end.

mod common;
use common::MockServer;
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

/// One fresh poll; returns the muse provider's status class word.
fn muse_class() -> String {
    let snap = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .unwrap();
    match &snap.provider("muse").unwrap().status {
        Status::Ok => "ok".to_string(),
        Status::NotInstalled => "not-installed".to_string(),
        Status::Stale { .. } => "stale".to_string(),
        Status::Error { class, .. } => class.clone(),
    }
}

#[test]
fn muse_error_taxonomy() {
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("RUNWAYBAR_FAKE_HOME", tmp.path()); // empty home: nothing installed
    std::env::set_var("XDG_CACHE_HOME", tmp.path().join("cache"));
    std::env::set_var("XDG_CONFIG_HOME", tmp.path().join("config"));
    std::env::set_var("RUNWAYBAR_TEST_ALLOW_HTTP", "1");
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
        "META_API_KEY",
        "MUSE_AUTH_PATH",
        "MUSE_SUBSCRIPTION_ENDPOINT",
    ] {
        std::env::remove_var(var);
    }

    let server = MockServer::start(vec![
        ("/e500".into(), 500, r#"{"error":"boom"}"#.into()),
        ("/e401".into(), 401, r#"{"error":"expired"}"#.into()),
        ("/e429".into(), 429, r#"{"error":"slow down"}"#.into()),
        ("/e404".into(), 404, r#"{"error":"moved"}"#.into()),
        ("/e422".into(), 422, r#"{"error":"unprocessable"}"#.into()),
        ("/ebroken".into(), 200, "{oops".into()),
        ("/eempty".into(), 200, r#"{"subs_usage": {}}"#.into()),
        (
            "/einactive".into(),
            200,
            r#"{"is_subs_active": false, "subs_usage": {"window": {"used_percent": 5.0}}}"#.into(),
        ),
    ]);

    // Endpoint phases: valid OAuth file, server misbehaves.
    std::env::set_var(
        "MUSE_AUTH_PATH",
        fixture_home().join(".config/muse/auth.json"),
    );
    for (route, want) in [
        ("/e500", "provider-unavailable"),
        ("/e401", "authentication-expired"),
        ("/e429", "rate-limited"),
        // 404 is ProviderUnavailable per the shared status_error mapping;
        // ApiFailure is the other-4xx bucket (422 here).
        ("/e404", "provider-unavailable"),
        ("/e422", "api-failure"),
        ("/ebroken", "parse-failure"),
        ("/eempty", "parse-failure"),
        ("/einactive", "provider-unavailable"),
    ] {
        drop_cache();
        std::env::set_var(
            "MUSE_SUBSCRIPTION_ENDPOINT",
            format!("{}{route}", server.base),
        );
        assert_eq!(muse_class(), want, "route {route}");
    }

    // The 401 message must not echo credential material.
    drop_cache();
    std::env::set_var(
        "MUSE_SUBSCRIPTION_ENDPOINT",
        format!("{}/e401", server.base),
    );
    let snap = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .unwrap();
    match &snap.provider("muse").unwrap().status {
        Status::Error { message, .. } => assert!(
            !message.contains("fixture-muse-token"),
            "error message leaked the token: {message}"
        ),
        other => panic!("expected the 401 error, got {other:?}"),
    }

    // Auth-file phases: the endpoint is reachable, the credential is not.
    // Shapes mirror the real auth file (`providers.meta`), per discover_key.
    drop_cache();
    std::env::set_var(
        "MUSE_SUBSCRIPTION_ENDPOINT",
        format!("{}/e401", server.base),
    );
    let auth = tmp.path().join("auth.json");
    for (body, what) in [
        (
            r#"{"providers": {"meta": {"mechanism": "api_key", "api_key": "k"}}}"#,
            "non-oauth mechanism",
        ),
        (r#"{}"#, "empty object"),
        (
            r#"{"providers": {"meta": {"mechanism": "oauth"}}}"#,
            "oauth without token",
        ),
        (
            r#"{"providers": {"meta": {"mechanism": "oauth", "access_token": ""}}}"#,
            "empty token",
        ),
        (
            r#"{"providers": {"meta": {"mechanism": "oauth", "access_token": "  "}}}"#,
            "whitespace token",
        ),
    ] {
        drop_cache();
        std::fs::write(&auth, body).unwrap();
        std::env::set_var("MUSE_AUTH_PATH", &auth);
        assert_eq!(muse_class(), "missing-credential", "{what}");
    }

    // META_API_KEY must be ignored: env key + no file is NotInstalled.
    drop_cache();
    std::env::set_var("MUSE_AUTH_PATH", tmp.path().join("no-such-auth.json"));
    std::env::set_var("META_API_KEY", "fixture-meta-ignored");
    assert_eq!(muse_class(), "not-installed");
}
