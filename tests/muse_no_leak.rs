//! Subprocess proof that a failing muse poll leaks nothing: the mock serves
//! a secret-bearing body on the parse-failure path (empty windows — the
//! branch where a raw-body diagnostic would print if one existed), and the
//! child's stdout+stderr must exclude every fixture secret. Own file: owns
//! its env. Hermetic: loopback endpoint, fixture auth, empty fake home.

mod common;
use common::MockServer;

#[test]
fn muse_failing_poll_prints_no_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    let server = MockServer::start(vec![(
        "/m".into(),
        200,
        r#"{"api_key": "fixture-muse-apikey", "user_email": "muse-user@example.com",
            "user_full_name": "Muse Fixture User", "subs_usage": {}}"#
            .into(),
    )]);
    let auth = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/home/.config/muse/auth.json");

    let out = std::process::Command::new(env!("CARGO_BIN_EXE_runwaybar"))
        .arg("status")
        .arg("--format")
        .arg("json")
        .env("RUNWAYBAR_FAKE_HOME", tmp.path())
        .env("XDG_CACHE_HOME", tmp.path().join("cache"))
        .env("XDG_CONFIG_HOME", tmp.path().join("config"))
        .env(
            "XDG_RUNTIME_DIR",
            std::env::temp_dir().join(format!("rwb-test-{}", std::process::id())),
        )
        .env("RUNWAYBAR_TEST_ALLOW_HTTP", "1")
        .env("RUNWAYBAR_LIVE_SMOKE", "1")
        .env("MUSE_AUTH_PATH", &auth)
        .env("MUSE_SUBSCRIPTION_ENDPOINT", format!("{}/m", server.base))
        .env_remove("Z_AI_API_KEY")
        .env_remove("BIGMODEL_API_KEY")
        .env_remove("ZHIPU_API_KEY")
        .env_remove("ZHIPUAI_API_KEY")
        .env_remove("GLM_API_KEY")
        .env_remove("OPENCODE_API_KEY")
        .env_remove("CODEX_HOME")
        .env_remove("CODEX_USAGE_ENDPOINT")
        .env_remove("CLAUDE_USAGE_ENDPOINT")
        .env_remove("Z_AI_QUOTA_ENDPOINT")
        .env_remove("Z_AI_QUOTA_CN_ENDPOINT")
        .env_remove("OPENCODE_USAGE_ENDPOINT")
        .env_remove("META_API_KEY")
        .output()
        .expect("spawn runwaybar status");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    // Execution proof: the empty-windows body takes the single-POST
    // parse-failure path (200s never retry), so exactly one request must
    // reach the mock and the failure class must render.
    assert_eq!(server.count(), 1, "the muse poll must execute");
    assert!(
        stdout.contains("\"muse\"") && stdout.contains("parse-failure"),
        "stdout: {stdout}\nstderr: {stderr}"
    );
    for secret in [
        "fixture-muse-token",
        "fixture-muse-apikey",
        "muse-user@example.com",
        "Muse Fixture User",
    ] {
        assert!(
            !stdout.contains(secret),
            "secret leaked into stdout: {secret}"
        );
        assert!(
            !stderr.contains(secret),
            "secret leaked into stderr: {secret}"
        );
    }
}
