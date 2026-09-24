//! Muse Code: `~/.config/muse/auth.json` (or `MUSE_AUTH_PATH`), subscription
//! quota via the CLI's own `POST {MUSE_SUBSCRIPTION_ENDPOINT}` startup call
//! (default `https://api.meta.ai/muse-code/key`; never the `auth.json`-supplied
//! `api_base_url`). OAuth logins only — API-key logins carry no subscription
//! quota. Linux file only: macOS `storage: "keychain"` logins have no file
//! token and are unseen here (documented gap).

use super::{bounded_post, first_f64, walk, Ctx, PollResult, Provider};
use crate::error::{ErrorClass, ProviderError};
use crate::model::{RateWindow, WindowKind};
use crate::secret::SecretString;
use serde_json::Value;

pub struct Muse;

impl Provider for Muse {
    fn id(&self) -> &'static str {
        "muse"
    }
    fn label(&self) -> &'static str {
        "Muse Code"
    }
    fn tool_hint(&self) -> &'static str {
        "muse (run `muse login`; API-key logins have no subscription quota)"
    }

    fn poll(&self, ctx: &Ctx) -> Result<PollResult, ProviderError> {
        let cred_path = auth_path(&ctx.home);
        if !cred_path.exists() {
            return Err(ProviderError::new(
                ErrorClass::NotInstalled,
                "muse not detected",
            ));
        }
        let key = discover_key(&ctx.home).ok_or_else(|| {
            ProviderError::new(ErrorClass::MissingCredential, "muse auth unreadable")
        })?;
        let resp = bounded_post(
            &ctx.config.endpoints.muse_subscription,
            vec![
                ("Authorization".into(), format!("Bearer {}", key.expose())),
                ("x-api-version".into(), "1.0.0".into()),
                ("Accept".into(), "application/json".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
            "{}",
        )?;
        if resp.status != 200 {
            return Err(crate::http::status_error(
                resp.status,
                "muse subscription endpoint",
            ));
        }
        let body: Value = serde_json::from_str(&resp.body).map_err(|e| {
            ProviderError::new(ErrorClass::ParseFailure, format!("usage json: {e}"))
        })?;
        ensure_active(&body)?;
        let windows = parse_windows(&body);
        if windows.is_empty() {
            // No raw-body diagnostic here (contrast claude.rs): the /key response
            // also carries api key, name and email — never log it, even in smoke.
            return Err(ProviderError::new(
                ErrorClass::ParseFailure,
                "no readable quota windows in Muse Code response",
            ));
        }
        Ok(PollResult {
            windows,
            account: parse_account(&body),
        })
    }
}

/// Resolved credential path: `MUSE_AUTH_PATH` wins, else the default location.
/// All existence/content checks apply to this resolved path (a set-but-missing
/// override is NotInstalled, never MissingCredential).
fn auth_path(home: &std::path::Path) -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("MUSE_AUTH_PATH") {
        return std::path::PathBuf::from(p);
    }
    home.join(".config").join("muse").join("auth.json")
}

pub fn discover_key(home: &std::path::Path) -> Option<SecretString> {
    let raw = std::fs::read_to_string(auth_path(home)).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let meta = walk(&v, &["providers", "meta"])?;
    if meta.get("mechanism").and_then(|m| m.as_str()) != Some("oauth") {
        return None;
    }
    let token = meta.get("access_token")?.as_str()?.trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(SecretString::new(token))
    }
}

/// Inactive subscriptions carry no quota and must not render as 0%.
fn ensure_active(body: &Value) -> Result<(), ProviderError> {
    if body.get("is_subs_active").and_then(|v| v.as_bool()) == Some(false) {
        return Err(ProviderError::new(
            ErrorClass::ProviderUnavailable,
            "no active Muse Code subscription",
        ));
    }
    Ok(())
}

/// `subs_usage.{window,weekly}` → Session("5-hour") + Weekly("weekly").
///
/// Percent-absent windows are skipped (a resets-only row is noise, and the
/// reference parser requires `used`); resets-absent windows keep `None`.
/// Unknown keys are ignored, never fatal.
pub fn parse_windows(body: &Value) -> Vec<RateWindow> {
    let Some(usage) = walk(body, &["subs_usage"]).filter(|u| u.is_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (node, kind, label) in [
        (usage.get("window"), WindowKind::Session, "5-hour"),
        (usage.get("weekly"), WindowKind::Weekly, "weekly"),
    ] {
        let Some(node) = node else { continue };
        let Some(used) = first_f64(node, &["used_percent"]) else {
            continue;
        };
        let resets = node
            .get("resets_at")
            .and_then(crate::timefmt::flexible_to_rfc3339);
        out.push(RateWindow::new(
            kind,
            Some(label.to_string()),
            Some(used),
            resets,
        ));
    }
    out
}

/// Top-level `subs_tier_name` (e.g. "Muse Code Power Usage"), sanitized:
/// remote strings reach terminal output, so control characters are stripped.
fn parse_account(body: &Value) -> Option<String> {
    let name = crate::cli::sanitize(body.get("subs_tier_name")?.as_str()?);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn documented_body() -> Value {
        json!({
            "is_subs_active": true,
            "subs_tier_name": "Muse Code Power Usage",
            "subs_usage": {
                "window": {"used_percent": 4, "window_duration_mins": 300, "resets_at": 1789068250},
                "weekly": {"used_percent": 28, "resets_at": 1789344000},
                "tier": "1000000000000001"
            }
        })
    }

    #[test]
    fn documented_shape() {
        let w = parse_windows(&documented_body());
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].label.as_deref(), Some("5-hour"));
        assert_eq!(w[0].used_percent, Some(4.0));
        assert_eq!(w[0].resets_at.as_deref(), Some("2026-09-10T19:24:10Z"));
        assert!(!w[0].overage);
        assert_eq!(w[1].kind, WindowKind::Weekly);
        assert_eq!(w[1].label.as_deref(), Some("weekly"));
        assert_eq!(w[1].used_percent, Some(28.0));
        assert_eq!(w[1].resets_at.as_deref(), Some("2026-09-14T00:00:00Z"));
    }

    #[test]
    fn inactive_plan_errors() {
        let err = ensure_active(&json!({"is_subs_active": false, "subs_usage": null}))
            .expect_err("inactive plan must error");
        assert_eq!(err.class, ErrorClass::ProviderUnavailable);
        assert!(ensure_active(&json!({})).is_ok());
        assert!(ensure_active(&json!({"is_subs_active": true})).is_ok());
    }

    #[test]
    fn missing_subs_usage_is_empty() {
        assert!(parse_windows(&json!({})).is_empty());
        assert!(parse_windows(&json!({"subs_usage": {}})).is_empty());
        assert!(parse_windows(&json!({"subs_usage": null})).is_empty());
        assert!(parse_windows(&json!({"subs_usage": []})).is_empty());
    }

    #[test]
    fn percent_absent_window_skipped_resets_malformed_kept() {
        let w = parse_windows(&json!({"subs_usage": {
            "window": {"resets_at": 1789068250},
            "weekly": {"used_percent": 28, "resets_at": "not-a-date"}
        }}));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].kind, WindowKind::Weekly);
        assert_eq!(w[0].used_percent, Some(28.0));
        assert!(w[0].resets_at.is_none());
    }

    #[test]
    fn overage_kept_as_is() {
        let w = parse_windows(&json!({"subs_usage": {
            "weekly": {"used_percent": 140, "resets_at": 5}
        }}));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].used_percent, Some(140.0));
        assert!(w[0].overage);
        assert_eq!(w[0].resets_at.as_deref(), Some("1970-01-01T00:00:05Z"));
    }

    #[test]
    fn percent_string_forms_and_null() {
        // Numeric strings parse; garbage and null skip the window.
        let w = parse_windows(&json!({"subs_usage": {
            "window": {"used_percent": "12.5"}
        }}));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].used_percent, Some(12.5));
        assert!(parse_windows(&json!({"subs_usage": {
            "window": {"used_percent": "abc"}
        }}))
        .is_empty());
        assert!(parse_windows(&json!({"subs_usage": {
            "window": {"used_percent": null}
        }}))
        .is_empty());
    }

    #[test]
    fn resets_absent_none_string_passthrough() {
        let w = parse_windows(&json!({"subs_usage": {
            "window": {"used_percent": 10}
        }}));
        assert_eq!(w.len(), 1);
        assert!(w[0].resets_at.is_none());
        let w = parse_windows(&json!({"subs_usage": {
            "weekly": {"used_percent": 10, "resets_at": "2026-09-23T12:00:00Z"}
        }}));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].resets_at.as_deref(), Some("2026-09-23T12:00:00Z"));
    }

    #[test]
    fn window_only_shape_yields_session() {
        // The weekly node is optional: a window-only body is one Session row.
        let w = parse_windows(&json!({"subs_usage": {
            "window": {"used_percent": 22.0}
        }}));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].label.as_deref(), Some("5-hour"));
        assert_eq!(w[0].used_percent, Some(22.0));
    }

    #[test]
    fn account_tier_sanitized() {
        assert_eq!(
            parse_account(&documented_body()).as_deref(),
            Some("Muse Code Power Usage")
        );
        assert!(parse_account(&json!({})).is_none());
        assert!(parse_account(&json!({"subs_tier_name": ""})).is_none());
        assert_eq!(
            parse_account(&json!({"subs_tier_name": "a\x07b\nc"})).as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn discovery_file_shapes() {
        // File only: the `MUSE_AUTH_PATH` override is covered by the
        // env-owning integration file (absent_tools). The override is
        // removed here (and restored after) so a leaked outer value cannot
        // hijack this test's file-only discovery; this file's tests are the
        // only readers, so a mid-test panic cannot corrupt another suite.
        let saved = std::env::var_os("MUSE_AUTH_PATH");
        std::env::remove_var("MUSE_AUTH_PATH");
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        assert!(discover_key(&home).is_none());
        let dir = home.join(".config").join("muse");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |s: &str| std::fs::write(dir.join("auth.json"), s).unwrap();
        write(r#"{"providers": {"meta": {"mechanism": "oauth", "access_token": " tok "}}}"#);
        assert_eq!(discover_key(&home).unwrap().expose(), "tok");
        write(r#"{"providers": {"meta": {"mechanism": "api_key", "api_key": "k"}}}"#);
        assert!(discover_key(&home).is_none());
        write(r#"{"providers": {"meta": {"mechanism": "oauth", "access_token": ""}}}"#);
        assert!(discover_key(&home).is_none());
        write(r#"{"providers": {"meta": {}}}"#);
        assert!(discover_key(&home).is_none());
        write("broken");
        assert!(discover_key(&home).is_none());
        if let Some(v) = saved {
            std::env::set_var("MUSE_AUTH_PATH", v);
        }
    }
}
