//! OpenCode: `~/.local/share/opencode/auth.json` (or `OPENCODE_API_KEY`), Zen Go usage
//! endpoint whose percent fields are already 0–100 with relative reset seconds.

use super::{bounded_fetch, first_f64, walk, Ctx, PollResult, Provider};
use crate::error::{ErrorClass, ProviderError};
use crate::model::{RateWindow, WindowKind};
use crate::secret::SecretString;
use serde_json::Value;

pub struct OpenCode;

impl Provider for OpenCode {
    fn id(&self) -> &'static str {
        "opencode"
    }
    fn label(&self) -> &'static str {
        "OpenCode"
    }
    fn tool_hint(&self) -> &'static str {
        "opencode (run `opencode auth login` or set OPENCODE_API_KEY)"
    }

    fn poll(&self, ctx: &Ctx) -> Result<PollResult, ProviderError> {
        let cred_path = ctx
            .home
            .join(".local")
            .join("share")
            .join("opencode")
            .join("auth.json");
        let env_key = std::env::var("OPENCODE_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
            .is_some();
        if !cred_path.exists() && !env_key {
            return Err(ProviderError::new(
                ErrorClass::NotInstalled,
                "opencode not detected",
            ));
        }
        let key = discover_key(&ctx.home).ok_or_else(|| {
            ProviderError::new(ErrorClass::MissingCredential, "auth.json unreadable")
        })?;
        let resp = bounded_fetch(
            &ctx.config.endpoints.opencode_usage,
            vec![
                ("Authorization".into(), format!("Bearer {}", key.expose())),
                ("Accept".into(), "application/json".into()),
            ],
        )?;
        if resp.status != 200 {
            return Err(crate::http::status_error(
                resp.status,
                "opencode usage endpoint",
            ));
        }
        let body: Value = serde_json::from_str(&resp.body).map_err(|e| {
            ProviderError::new(ErrorClass::ParseFailure, format!("usage json: {e}"))
        })?;
        let windows = parse_windows(&body);
        if windows.is_empty() {
            return Err(ProviderError::new(
                ErrorClass::ParseFailure,
                "no recognisable usage windows in response",
            ));
        }
        Ok(PollResult {
            windows,
            account: None,
        })
    }
}

pub fn discover_key(home: &std::path::Path) -> Option<SecretString> {
    if let Ok(k) = std::env::var("OPENCODE_API_KEY") {
        if !k.is_empty() {
            return Some(SecretString::new(k));
        }
    }
    let path = home
        .join(".local")
        .join("share")
        .join("opencode")
        .join("auth.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    let key = walk(&v, &["opencode", "key"])?.as_str()?.to_string();
    if key.is_empty() {
        None
    } else {
        Some(SecretString::new(key))
    }
}

/// `usage.{rolling,weekly,monthly}.percent` (+ `resetInSec`) → kinds.
pub fn parse_windows(body: &Value) -> Vec<RateWindow> {
    let Some(usage) = walk(body, &["usage"]) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (node, kind, label) in [
        (usage.get("rolling"), WindowKind::Session, "rolling"),
        (usage.get("weekly"), WindowKind::Weekly, "weekly"),
        (usage.get("monthly"), WindowKind::Monthly, "monthly"),
    ] {
        let Some(node) = node else { continue };
        let used = first_f64(node, &["percent", "percentage", "used_percent"]);
        let resets = node
            .get("resetInSec")
            .or_else(|| node.get("resets_in_sec"))
            .and_then(|r| r.as_f64())
            .and_then(crate::timefmt::relative_secs_to_rfc3339);
        if used.is_none() && resets.is_none() {
            continue;
        }
        out.push(RateWindow::new(kind, Some(label.to_string()), used, resets));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn documented_shape() {
        let body = json!({"usage": {
            "rolling": {"percent": 62, "resetInSec": 5400},
            "weekly": {"percent": 20.5, "resetInSec": 250000},
            "monthly": {"percent": 9.0}
        }});
        let w = parse_windows(&body);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].used_percent, Some(62.0));
        assert!(w[0].resets_at.is_some());
        assert_eq!(w[2].kind, WindowKind::Monthly);
        assert!(w[2].resets_at.is_none());
    }

    #[test]
    fn absent_usage_section_is_empty() {
        assert!(parse_windows(&json!({})).is_empty());
        assert!(parse_windows(&json!({"usage": {}})).is_empty());
    }

    #[test]
    fn discovery_env_and_file() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        // file path (env unset in this process by design; tested via file only)
        assert!(discover_key(&home).is_none());
        let dir = home.join(".local").join("share").join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("auth.json"),
            r#"{"opencode": {"type": "zen", "key": "oc-key"}}"#,
        )
        .unwrap();
        assert_eq!(discover_key(&home).unwrap().expose(), "oc-key");
        std::fs::write(dir.join("auth.json"), "broken").unwrap();
        assert!(discover_key(&home).is_none());
    }
}
