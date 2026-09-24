//! Env endpoint overrides are https-only (invariant #19): a non-https override is
//! rejected; the loopback test hook allows only 127.0.0.1. Own file: owns its env.

use runwaybar::config::{Config, Endpoints};

#[test]
fn non_https_overrides_rejected_loopback_allowed_with_hook() {
    std::env::remove_var("RUNWAYBAR_TEST_ALLOW_HTTP");
    std::env::set_var("CODEX_USAGE_ENDPOINT", "http://evil.example.com/x");
    std::env::set_var("CLAUDE_USAGE_ENDPOINT", "ftp://evil.example.com/y");
    std::env::set_var("MUSE_SUBSCRIPTION_ENDPOINT", "http://evil.example.com/z");
    let cfg = Config::default();
    assert_eq!(
        cfg.endpoints.codex_usage,
        Endpoints::default().codex_usage,
        "http override must be ignored"
    );
    assert_eq!(
        cfg.endpoints.claude_usage,
        Endpoints::default().claude_usage,
        "ftp override must be ignored"
    );
    assert_eq!(
        cfg.endpoints.muse_subscription,
        Endpoints::default().muse_subscription,
        "http override must be ignored"
    );

    // Hook on, but non-loopback http still refused; loopback allowed.
    std::env::set_var("RUNWAYBAR_TEST_ALLOW_HTTP", "1");
    std::env::set_var("CODEX_USAGE_ENDPOINT", "http://evil.example.com/x");
    std::env::set_var("Z_AI_QUOTA_ENDPOINT", "http://127.0.0.1:9999/quota");
    std::env::set_var("MUSE_SUBSCRIPTION_ENDPOINT", "http://127.0.0.1:9999/muse");
    let cfg = Config::default();
    assert_eq!(
        cfg.endpoints.codex_usage,
        Endpoints::default().codex_usage,
        "non-loopback http must be ignored even with the hook"
    );
    assert_eq!(cfg.endpoints.zai_quota, "http://127.0.0.1:9999/quota");
    assert_eq!(
        cfg.endpoints.muse_subscription,
        "http://127.0.0.1:9999/muse"
    );

    // Hook off again: loopback http refused too.
    std::env::remove_var("RUNWAYBAR_TEST_ALLOW_HTTP");
    let cfg = Config::default();
    assert_eq!(cfg.endpoints.zai_quota, Endpoints::default().zai_quota);
    assert_eq!(
        cfg.endpoints.muse_subscription,
        Endpoints::default().muse_subscription
    );
}
