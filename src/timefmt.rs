//! Time normalisation: epochs (s vs ms), RFC3339 strings, relative seconds, countdowns.

use jiff::Timestamp;
use serde_json::Value;

pub fn now_rfc3339() -> String {
    Timestamp::now().to_string()
}

pub fn now_epoch_ms() -> i64 {
    Timestamp::now().as_millisecond()
}

/// Epoch number (seconds or milliseconds, disambiguated by magnitude) → RFC3339.
pub fn epoch_to_rfc3339(v: f64) -> Option<String> {
    let ms = if v.abs() >= 1e12 { v } else { v * 1000.0 };
    Timestamp::from_millisecond(ms as i64)
        .ok()
        .map(|t| t.to_string())
}

/// `resets_at`-style value → RFC3339. Accepts epoch numbers and RFC3339 strings;
/// everything else is None (unknown stays unknown).
pub fn flexible_to_rfc3339(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => n.as_f64().and_then(epoch_to_rfc3339),
        Value::String(s) => s.trim().parse::<Timestamp>().ok().map(|t| t.to_string()),
        _ => None,
    }
}

/// Relative seconds (from now) → RFC3339.
pub fn relative_secs_to_rfc3339(secs: f64) -> Option<String> {
    Timestamp::now()
        .checked_add(jiff::Span::new().seconds(secs as i64))
        .ok()
        .map(|t| t.to_string())
}

/// "1h 12m" / "3d 4h" / "now" style countdown from an RFC3339 instant to `now_epoch_ms`.
/// Past instants render as "now" (reset already due); unparseable → None.
pub fn countdown(resets_at: &str, now_epoch_ms: i64) -> Option<String> {
    let target: Timestamp = resets_at.parse().ok()?;
    let diff_ms = target.as_millisecond() - now_epoch_ms;
    if diff_ms <= 0 {
        return Some("now".to_string());
    }
    let secs = diff_ms / 1000;
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let out = if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{secs}s")
    };
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn epoch_ms_and_s_disambiguated() {
        let ms = epoch_to_rfc3339(1_758_000_000_000.0).unwrap();
        let s = epoch_to_rfc3339(1_758_000_000.0).unwrap();
        assert_eq!(ms, s);
        assert!(ms.starts_with("2025"));
    }

    #[test]
    fn rfc3339_string_passthrough_normalised() {
        let v = json!("2026-09-23T12:00:00Z");
        assert_eq!(
            flexible_to_rfc3339(&v).as_deref(),
            Some("2026-09-23T12:00:00Z")
        );
    }

    #[test]
    fn unknown_shapes_stay_unknown() {
        assert!(flexible_to_rfc3339(&json!(null)).is_none());
        assert!(flexible_to_rfc3339(&json!("not a date")).is_none());
        assert!(flexible_to_rfc3339(&json!(true)).is_none());
    }

    #[test]
    fn past_reset_is_now() {
        let past = epoch_to_rfc3339(1_000_000_000.0).unwrap();
        assert_eq!(countdown(&past, now_epoch_ms()).as_deref(), Some("now"));
    }

    #[test]
    fn countdown_formats() {
        let target = now_epoch_ms() + (3900 * 1000); // 1h 5m
        let t = epoch_to_rfc3339(target as f64).unwrap();
        assert_eq!(countdown(&t, now_epoch_ms()).as_deref(), Some("1h 5m"));
        let target2 = now_epoch_ms() + (3 * 86400 * 1000 + 3600 * 1000);
        let t2 = epoch_to_rfc3339(target2 as f64).unwrap();
        assert_eq!(countdown(&t2, now_epoch_ms()).as_deref(), Some("3d 1h"));
    }
}
