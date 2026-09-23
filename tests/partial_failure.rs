//! E2E: per-provider partial-failure isolation (invariant #18) + last-good retention
//! (invariant #6) against a loopback mock server. Own file: owns its env.

mod common;

use common::MockServer;
use runwaybar::cli::{run_status, Format, StatusOpts};
use runwaybar::model::Status;

fn fixture_home() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/home")
}

#[test]
fn one_failing_provider_does_not_blank_healthy_ones() {
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
    ] {
        std::env::remove_var(var);
    }

    // claude returns 500 persistently; the others are healthy.
    let server = MockServer::start(vec![
        ("/claude".into(), 500, "{}".into()),
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

    // Poll 1: claude errors (after its one bounded retry), others succeed.
    let snap1 = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .unwrap();
    let claude = snap1.provider("claude-code").unwrap();
    assert!(
        matches!(&claude.status, Status::Error { class, .. } if class == "provider-unavailable"),
        "claude should surface the 500, got {:?}",
        claude.status
    );
    for id in ["codex", "zai", "opencode"] {
        let p = snap1.provider(id).unwrap();
        assert!(
            matches!(p.status, Status::Ok),
            "{id} should be ok, got {:?}",
            p.status
        );
        assert!(!p.windows.is_empty(), "{id} should have windows");
    }

    // Poll 2 (force a stale cache read cycle by making the cache old): claude now
    // FAILS AGAIN — she must keep her previous windows marked stale, not blank them.
    // We simulate by writing the first snapshot with an old timestamp into the cache,
    // then re-running with claude healthy→no: keep 500. The merge keeps... claude had
    // no windows in poll 1 (error from the start), so seed a good claude cache first.
    let mut seeded = snap1.clone();
    seeded.generated_at = "2026-09-23T00:00:00Z".to_string(); // forces stale cache
    let idx = seeded
        .providers
        .iter()
        .position(|p| p.id == "claude-code")
        .unwrap();
    seeded.providers[idx] = runwaybar::model::ProviderSnapshot {
        id: "claude-code".into(),
        label: "Claude Code".into(),
        status: Status::Ok,
        account: Some("pro".into()),
        windows: vec![runwaybar::model::RateWindow::new(
            runwaybar::model::WindowKind::Session,
            None,
            Some(55.0),
            None,
        )],
    };
    std::fs::create_dir_all(tmp.path().join("cache").join("runwaybar")).unwrap();
    std::fs::write(
        tmp.path()
            .join("cache")
            .join("runwaybar")
            .join("last-good.json"),
        serde_json::to_string(&seeded).unwrap(),
    )
    .unwrap();

    // Request count: 4 providers + claude's single bounded retry.
    assert!(
        server.count() >= 5,
        "expected 5+ requests, got {}",
        server.count()
    );
    let snap2 = run_status(StatusOpts {
        format: Format::Json,
        providers: vec![],
        no_fetch: false,
    })
    .unwrap();
    let claude2 = snap2.provider("claude-code").unwrap();
    match &claude2.status {
        Status::Stale { reason, .. } => {
            assert!(reason
                .as_deref()
                .unwrap_or_default()
                .contains("provider-unavailable"));
        }
        other => {
            panic!("claude should be Stale after a failed poll with prior good data, got {other:?}")
        }
    }
    assert_eq!(
        claude2.worst_used_percent(),
        Some(55.0),
        "previous windows must survive"
    );
    // Healthy providers remain ok in the merged snapshot too.
    for id in ["codex", "zai", "opencode"] {
        assert!(
            matches!(snap2.provider(id).unwrap().status, Status::Ok),
            "{id} regressed"
        );
    }
}

fn codex_body() -> String {
    r#"{"plan_type": "plus", "rate_limit": {"primary_window": {"used_percent": 44.0}}}"#.into()
}
fn zai_body() -> String {
    r#"{"data": {"limits": [{"type": "TOKENS_LIMIT", "unit": 3, "percentage": 42.0}]}}"#.into()
}
fn opencode_body() -> String {
    r#"{"usage": {"rolling": {"percent": 55}}}"#.into()
}
