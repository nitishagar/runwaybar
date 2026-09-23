//! Claude Code: reads `~/.claude/.credentials.json` (read-only), polls the
//! subscription usage endpoint. 429-sensitive: cooldown 300 s.

use super::{bounded_fetch, first_f64, walk, Ctx, PollResult, Provider};
use crate::error::{ErrorClass, ProviderError};
use crate::model::{RateWindow, WindowKind};
use crate::secret::SecretString;
use serde_json::Value;

pub struct ClaudeCode;

/// Session/weekly key aliases observed in the wild (research E7: schema drifts).
const SESSION_KEYS: [&str; 6] = [
    "five_hour",
    "5_hour",
    "session",
    "fivehour",
    "five_hour_window",
    "rolling",
];
const WEEKLY_KEYS: [&str; 7] = [
    "seven_day",
    "7_day",
    "weekly",
    "sevenday",
    "seven_day_window",
    "seven_day_max",
    "week",
];

impl Provider for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude-code"
    }
    fn label(&self) -> &'static str {
        "Claude Code"
    }
    fn tool_hint(&self) -> &'static str {
        "the claude CLI"
    }

    fn poll(&self, ctx: &Ctx) -> Result<PollResult, ProviderError> {
        if !ctx.home.join(".claude").join(".credentials.json").exists() {
            return Err(ProviderError::new(
                ErrorClass::NotInstalled,
                "claude code not detected",
            ));
        }
        let token = discover_token(&ctx.home).ok_or_else(|| {
            ProviderError::new(ErrorClass::MissingCredential, "credentials file unreadable")
        })?;
        let resp = bounded_fetch(
            &ctx.config.endpoints.claude_usage,
            vec![
                ("Authorization".into(), format!("Bearer {}", token.expose())),
                ("anthropic-beta".into(), "oauth-2025-04-20".into()),
                ("Accept".into(), "application/json".into()),
            ],
        )?;
        if resp.status != 200 {
            return Err(super_status(resp.status, "claude usage endpoint"));
        }
        let body: Value = serde_json::from_str(&resp.body).map_err(|e| {
            ProviderError::new(ErrorClass::ParseFailure, format!("usage json: {e}"))
        })?;
        let windows = parse_windows(&body);
        if windows.is_empty() {
            return Err(ProviderError::new(
                ErrorClass::ParseFailure,
                "no recognisable windows in usage response",
            ));
        }
        let account = walk(&body, &["plan", "type"])
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(PollResult { windows, account })
    }
}

fn super_status(status: u16, label: &str) -> ProviderError {
    crate::http::status_error(status, label)
}

/// Read the access token; never writes. Absence → None (NotInstalled handled by caller
/// via MissingCredential — the file missing means the CLI never signed in here).
pub fn discover_token(home: &std::path::Path) -> Option<SecretString> {
    let path = home.join(".claude").join(".credentials.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let token = walk(&v, &["claudeAiOauth", "accessToken"])?
        .as_str()?
        .to_string();
    if token.is_empty() {
        None
    } else {
        Some(SecretString::new(token))
    }
}

/// Accepts the documented `usage` array shape and a flat map shape; unknown keys ignored.
pub fn parse_windows(body: &Value) -> Vec<RateWindow> {
    let mut out = Vec::new();
    let entries: Vec<(String, Value)> =
        if let Some(arr) = walk(body, &["usage"]).and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|e| {
                    e.get("key")
                        .and_then(|k| k.as_str())
                        .map(|k| (k.to_string(), e.clone()))
                })
                .collect()
        } else if let Some(map) = body.get("windows").and_then(|v| v.as_object()) {
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        } else {
            Vec::new()
        };
    for (key, v) in entries {
        let kind = if SESSION_KEYS.contains(&key.as_str()) {
            WindowKind::Session
        } else if WEEKLY_KEYS.contains(&key.as_str()) {
            WindowKind::Weekly
        } else {
            continue; // model-scoped/extra windows: ignore-with-note, never guess
        };
        let used = first_f64(&v, &["used_pct", "used_percent", "percent", "pct"]);
        let resets = v
            .get("resets_in_sec")
            .and_then(|r| r.as_f64())
            .and_then(crate::timefmt::relative_secs_to_rfc3339)
            .or_else(|| {
                v.get("resets_at")
                    .and_then(crate::timefmt::flexible_to_rfc3339)
            });
        out.push(RateWindow::new(kind, Some(key), used, resets));
    }
    out.sort_by_key(|w| (w.kind != WindowKind::Session, w.label.clone()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn win(body: &Value) -> Vec<RateWindow> {
        parse_windows(body)
    }

    #[test]
    fn documented_usage_array_shape() {
        let body = json!({
            "usage": [
                {"key": "five_hour", "used_pct": 62.5, "resets_in_sec": 4321},
                {"key": "seven_day", "used_percent": 41.0, "resets_at": "2026-10-01T00:00:00Z"}
            ]
        });
        let w = win(&body);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].used_percent, Some(62.5));
        assert!(w[0].resets_at.is_some());
        assert_eq!(w[1].kind, WindowKind::Weekly);
    }

    #[test]
    fn flat_windows_map_shape_and_aliases() {
        let body = json!({"windows": {"5_hour": {"percent": 80.0}, "weekly": {"used_pct": 10.0}}});
        let w = win(&body);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].used_percent, Some(80.0));
    }

    #[test]
    fn unknown_keys_are_dropped_not_mapped() {
        let body = json!({"usage": [
            {"key": "iguana_necktie", "used_pct": 50.0},
            {"key": "seven_day_routines", "used_pct": 30.0}
        ]});
        assert!(win(&body).is_empty());
    }

    #[test]
    fn overage_and_unknown_percent_preserved() {
        let body = json!({"usage": [
            {"key": "five_hour", "used_pct": 130.0},
            {"key": "seven_day"}
        ]});
        let w = win(&body);
        assert_eq!(w[0].used_percent, Some(130.0));
        assert!(w[0].overage);
        assert_eq!(w[1].used_percent, None);
        assert!(!w[1].overage);
    }

    #[test]
    fn discovery_absent_and_present() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover_token(tmp.path()).is_none());
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            tmp.path().join(".claude").join(".credentials.json"),
            r#"{"claudeAiOauth": {"accessToken": "tok-abc", "expiresAt": "2026-01-01T00:00:00Z"}}"#,
        )
        .unwrap();
        let s = discover_token(tmp.path()).unwrap();
        assert_eq!(s.expose(), "tok-abc");
    }

    #[test]
    #[cfg(unix)]
    fn discovery_tolerates_unreadable_mode() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        let cred = tmp.path().join(".claude").join(".credentials.json");
        std::fs::write(&cred, r#"{"claudeAiOauth": {"accessToken": "tok"}}"#).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cred, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(
            discover_token(tmp.path()).is_none(),
            "unreadable file must not yield a token"
        );
        std::fs::set_permissions(&cred, std::fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn discovery_tolerates_garbage() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            tmp.path().join(".claude").join(".credentials.json"),
            "not json",
        )
        .unwrap();
        assert!(discover_token(tmp.path()).is_none());
        std::fs::write(
            tmp.path().join(".claude").join(".credentials.json"),
            r#"{"claudeAiOauth": {"accessToken": ""}}"#,
        )
        .unwrap();
        assert!(discover_token(tmp.path()).is_none());
    }
}
