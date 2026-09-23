//! Codex: reads `$CODEX_HOME/auth.json` (default `~/.codex/auth.json`) read-only and
//! polls the usage endpoint. Never redeems the refresh token (no-refresh rule).

use super::{bounded_fetch, first_f64, walk, Ctx, PollResult, Provider};
use crate::error::{ErrorClass, ProviderError};
use crate::model::{RateWindow, WindowKind};
use crate::secret::SecretString;
use serde_json::Value;

pub struct Codex;

pub struct Credentials {
    pub access_token: SecretString,
    pub account_id: Option<String>,
}

impl Provider for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn label(&self) -> &'static str {
        "Codex"
    }
    fn tool_hint(&self) -> &'static str {
        "the codex CLI"
    }

    fn poll(&self, ctx: &Ctx) -> Result<PollResult, ProviderError> {
        if !codex_home(&ctx.home).join("auth.json").exists() {
            return Err(ProviderError::new(
                ErrorClass::NotInstalled,
                "codex not detected",
            ));
        }
        let cred = discover(&ctx.home).ok_or_else(|| {
            ProviderError::new(ErrorClass::MissingCredential, "auth.json unreadable")
        })?;
        let mut headers = vec![
            (
                "Authorization".into(),
                format!("Bearer {}", cred.access_token.expose()),
            ),
            ("Accept".into(), "application/json".into()),
        ];
        if let Some(acct) = &cred.account_id {
            headers.push(("ChatGPT-Account-Id".into(), acct.clone()));
        }
        let mut resp = bounded_fetch(&ctx.config.endpoints.codex_usage, headers.clone())?;
        if resp.status == 404 {
            // Documented fallback path when /wham/usage is not served (research §4).
            if let Some(fallback) = fallback_url(&ctx.config.endpoints.codex_usage) {
                resp = bounded_fetch(&fallback, headers)?;
            }
        }
        if resp.status != 200 {
            return Err(crate::http::status_error(
                resp.status,
                "codex usage endpoint",
            ));
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
        let account = walk(&body, &["plan_type"])
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(PollResult { windows, account })
    }
}

/// `https://host/backend-api/wham/usage` → `https://host/api/codex/usage` (same origin).
fn fallback_url(primary: &str) -> Option<String> {
    let scheme_end = primary.find("://")? + 3;
    let path_start = primary[scheme_end..].find('/')? + scheme_end;
    let origin = &primary[..path_start];
    Some(format!("{origin}/api/codex/usage"))
}

fn codex_home(home: &std::path::Path) -> std::path::PathBuf {
    if let Ok(env) = std::env::var("CODEX_HOME") {
        std::path::PathBuf::from(env)
    } else {
        home.join(".codex")
    }
}

/// Read-only discovery; keyring/ephemeral CLI modes leave no file → None → NotInstalled.
pub fn discover(home: &std::path::Path) -> Option<Credentials> {
    let path = codex_home(home).join("auth.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let token = walk(&v, &["tokens", "access_token"])?.as_str()?.to_string();
    if token.is_empty() {
        return None;
    }
    let account_id = v
        .get("chatgpt_account_id")
        .and_then(|a| a.as_str())
        .map(|s| s.to_string());
    Some(Credentials {
        access_token: SecretString::new(token),
        account_id,
    })
}

/// rate_limit.primary_window → Session, secondary_window → Weekly; tolerant field names.
pub fn parse_windows(body: &Value) -> Vec<RateWindow> {
    let mut out = Vec::new();
    let rl = match walk(body, &["rate_limit"]) {
        Some(rl) => rl,
        None => return out,
    };
    for (node, kind, label) in [
        (rl.get("primary_window"), WindowKind::Session, "primary"),
        (rl.get("secondary_window"), WindowKind::Weekly, "weekly"),
    ] {
        let Some(node) = node else { continue };
        let used = first_f64(node, &["used_percent", "percent", "used_pct"]);
        let resets = node
            .get("resets_at")
            .or_else(|| node.get("reset_at"))
            .and_then(crate::timefmt::flexible_to_rfc3339);
        out.push(RateWindow::new(kind, Some(label.to_string()), used, resets));
    }
    out.retain(|w| w.used_percent.is_some() || w.resets_at.is_some() || w.label.is_some());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn documented_shape() {
        let body = json!({
            "plan_type": "plus",
            "rate_limit": {
                "primary_window": {"used_percent": 44.0, "window_minutes": 300, "resets_at": 1759000000},
                "secondary_window": {"used_percent": 12.0, "window_minutes": 10080}
            }
        });
        let w = parse_windows(&body);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].used_percent, Some(44.0));
        assert!(w[0].resets_at.is_some());
        assert_eq!(w[1].kind, WindowKind::Weekly);
        assert!(walk(&body, &["plan_type"]).unwrap().as_str() == Some("plus"));
    }

    #[test]
    fn missing_windows_yield_empty_not_error_data() {
        assert!(parse_windows(&json!({})).is_empty());
        assert!(parse_windows(&json!({"rate_limit": {}})).is_empty());
    }

    #[test]
    fn discovery_via_codex_home_env_and_default() {
        // Safe: this test runs without RUNWAYBAR_FAKE_HOME; use an isolated home dir.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        assert!(discover(&home).is_none());
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex").join("auth.json"),
            r#"{"tokens": {"access_token": "at-1"}, "chatgpt_account_id": "acct-9"}"#,
        )
        .unwrap();
        let c = discover(&home).unwrap();
        assert_eq!(c.access_token.expose(), "at-1");
        assert_eq!(c.account_id.as_deref(), Some("acct-9"));
    }

    #[test]
    fn fallback_url_same_origin_only() {
        assert_eq!(
            fallback_url("https://chatgpt.com/backend-api/wham/usage").as_deref(),
            Some("https://chatgpt.com/api/codex/usage")
        );
        assert_eq!(
            fallback_url("https://host.example").as_deref(),
            None,
            "no path → no fallback"
        );
    }

    #[test]
    fn epoch_seconds_vs_ms_resets_handled() {
        let body = json!({"rate_limit": {"primary_window": {"used_percent": 1.0, "resets_at": 1759000000000i64}}});
        let w = parse_windows(&body);
        assert!(w[0].resets_at.as_deref().unwrap().starts_with("2025"));
    }
}
