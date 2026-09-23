//! z.ai / ZCode: API key from ZCode's `~/.zcode/v2/config.json` provider entries (or
//! env), quota endpoint with global/CN region selection. Multiple key entries are
//! enumerated; the first usable one is polled.

use super::{bounded_fetch, first_f64, walk, Ctx, PollResult, Provider};
use crate::error::{ErrorClass, ProviderError};
use crate::model::{RateWindow, WindowKind};
use crate::secret::SecretString;
use serde_json::Value;

pub struct Zai;

#[derive(Clone, Debug, PartialEq)]
pub struct ZaiKey {
    pub source: String,
    pub region: Region,
    pub key: SecretString,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Region {
    Global,
    Cn,
}

impl Provider for Zai {
    fn id(&self) -> &'static str {
        "zai"
    }
    fn label(&self) -> &'static str {
        "z.ai / ZCode"
    }
    fn tool_hint(&self) -> &'static str {
        "z.ai (set Z_AI_API_KEY or configure ZCode)"
    }

    fn poll(&self, ctx: &Ctx) -> Result<PollResult, ProviderError> {
        let keys = discover_keys(&ctx.home);
        let key = keys.first().cloned().ok_or_else(|| {
            if ctx
                .home
                .join(".zcode")
                .join("v2")
                .join("config.json")
                .exists()
            {
                ProviderError::new(
                    ErrorClass::MissingCredential,
                    "zcode config has no usable API key",
                )
            } else {
                ProviderError::new(ErrorClass::NotInstalled, "z.ai/ZCode not detected")
            }
        })?;
        let endpoint = match key.region {
            Region::Global => &ctx.config.endpoints.zai_quota,
            Region::Cn => &ctx.config.endpoints.zai_quota_cn,
        };
        let resp = bounded_fetch(
            endpoint,
            vec![
                (
                    "Authorization".into(),
                    format!("Bearer {}", key.key.expose()),
                ),
                ("Accept".into(), "application/json".into()),
            ],
        )?;
        if resp.status != 200 {
            return Err(crate::http::status_error(
                resp.status,
                "z.ai quota endpoint",
            ));
        }
        let body: Value = serde_json::from_str(&resp.body).map_err(|e| {
            ProviderError::new(ErrorClass::ParseFailure, format!("quota json: {e}"))
        })?;
        let windows = parse_windows(&body);
        if windows.is_empty() {
            return Err(ProviderError::new(
                ErrorClass::ParseFailure,
                "no recognisable limits in quota response",
            ));
        }
        let account = Some(format!("{} ({})", key.source, region_str(key.region)));
        Ok(PollResult { windows, account })
    }
}

fn region_str(r: Region) -> &'static str {
    match r {
        Region::Global => "global",
        Region::Cn => "CN",
    }
}

/// Env keys first (explicit user intent), then ZCode config entries in file order.
pub fn discover_keys(home: &std::path::Path) -> Vec<ZaiKey> {
    let mut out = Vec::new();
    if let Ok(k) = std::env::var("Z_AI_API_KEY") {
        if !k.is_empty() {
            out.push(ZaiKey {
                source: "Z_AI_API_KEY".into(),
                region: Region::Global,
                key: SecretString::new(k),
            });
        }
    }
    for var in [
        "BIGMODEL_API_KEY",
        "ZHIPU_API_KEY",
        "ZHIPUAI_API_KEY",
        "GLM_API_KEY",
    ] {
        if let Ok(k) = std::env::var(var) {
            if !k.is_empty() {
                out.push(ZaiKey {
                    source: var.into(),
                    region: Region::Cn,
                    key: SecretString::new(k),
                });
            }
        }
    }
    out.extend(zcode_config_keys(home));
    out
}

/// Tolerant parse of `~/.zcode/v2/config.json`: accepts `providers` as map or array;
/// each entry needs `options.apiKey`; `options.baseURL` containing bigmodel.cn → CN.
pub fn zcode_config_keys(home: &std::path::Path) -> Vec<ZaiKey> {
    let path = home.join(".zcode").join("v2").join("config.json");
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let v: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(providers) = v.get("providers") else {
        return Vec::new();
    };
    let entries: Vec<(String, &Value)> = if let Some(map) = providers.as_object() {
        map.iter().map(|(k, val)| (k.clone(), val)).collect()
    } else if let Some(arr) = providers.as_array() {
        arr.iter()
            .filter_map(|e| {
                let id = e.get("id").or_else(|| e.get("key"))?.as_str()?.to_string();
                Some((id, e))
            })
            .collect()
    } else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|(id, e)| {
            let key = walk(e, &["options", "apiKey"])?.as_str()?.to_string();
            if key.is_empty() {
                return None;
            }
            let base = walk(e, &["options", "baseURL"])
                .and_then(|b| b.as_str())
                .unwrap_or("");
            let region = if base.contains("bigmodel.cn") {
                Region::Cn
            } else {
                Region::Global
            };
            Some(ZaiKey {
                source: format!("zcode:{id}"),
                region,
                key: SecretString::new(key),
            })
        })
        .collect()
}

/// `data.limits[]` with `unit` 3 = 5 h rolling, 6 = weekly (research §14); per (kind)
/// keep the worst usage (conservative runway). Types TOKENS/CREDIT/TIME become labels.
pub fn parse_windows(body: &Value) -> Vec<RateWindow> {
    let mut session: Option<RateWindow> = None;
    let mut weekly: Option<RateWindow> = None;
    let mut others: Vec<RateWindow> = Vec::new();
    let Some(limits) = walk(body, &["data", "limits"]).and_then(|l| l.as_array()) else {
        return Vec::new();
    };
    for lim in limits {
        let t = lim.get("type").and_then(|t| t.as_str()).unwrap_or("limit");
        let unit = lim.get("unit").and_then(|u| u.as_i64());
        let used = first_f64(
            lim,
            &["percentage", "usedPercentage", "used_percentage", "percent"],
        );
        let resets = lim
            .get("nextResetTime")
            .or_else(|| lim.get("next_reset_time"))
            .and_then(crate::timefmt::flexible_to_rfc3339);
        let label = Some(t.to_string());
        let kind = match unit {
            Some(3) => WindowKind::Session,
            Some(6) => WindowKind::Weekly,
            _ => {
                others.push(RateWindow::new(WindowKind::Other, label, used, resets));
                continue;
            }
        };
        let candidate = RateWindow::new(kind, label.clone(), used, resets.clone());
        let slot = if kind == WindowKind::Session {
            &mut session
        } else {
            &mut weekly
        };
        *slot = Some(match slot.take() {
            Some(prev) => {
                // Worst usage wins; a reset instant survives from whichever entry had one.
                let used = match (prev.used_percent, candidate.used_percent) {
                    (Some(a), Some(b)) => Some(a.max(b)),
                    (Some(a), None) | (None, Some(a)) => Some(a),
                    (None, None) => None,
                };
                RateWindow::new(kind, prev.label.or(label), used, prev.resets_at.or(resets))
            }
            None => candidate,
        });
    }
    let mut out = Vec::new();
    if let Some(w) = session {
        out.push(w);
    }
    if let Some(w) = weekly {
        out.push(w);
    }
    out.extend(others);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn limits_mapped_by_unit_with_worst_kept() {
        let body = json!({"data": {"limits": [
            {"type": "TOKENS_LIMIT", "unit": 3, "percentage": 42.0, "nextResetTime": 1758900000000i64},
            {"type": "CREDIT_LIMIT", "unit": 3, "percentage": 55.0},
            {"type": "TOKENS_LIMIT", "unit": 6, "percentage": 10.0},
            {"type": "TIME_LIMIT", "unit": 9, "percentage": 3.0}
        ]}});
        let w = parse_windows(&body);
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].kind, WindowKind::Session);
        assert_eq!(w[0].used_percent, Some(55.0)); // worst of the two unit-3 entries
        assert!(w[0].resets_at.is_some());
        assert_eq!(w[1].kind, WindowKind::Weekly);
        assert_eq!(w[2].kind, WindowKind::Other);
        assert_eq!(w[2].label.as_deref(), Some("TIME_LIMIT"));
    }

    #[test]
    fn empty_and_absent_limits() {
        assert!(parse_windows(&json!({})).is_empty());
        assert!(parse_windows(&json!({"data": {"limits": []}})).is_empty());
    }

    #[test]
    fn zcode_config_map_and_array_shapes() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        assert!(zcode_config_keys(&home).is_empty());
        std::fs::create_dir_all(home.join(".zcode").join("v2")).unwrap();
        std::fs::write(
            home.join(".zcode").join("v2").join("config.json"),
            r#"{"providers": {
                "builtin:zai-coding-plan": {"options": {"apiKey": "k-global", "baseURL": "https://api.z.ai"}},
                "builtin:bigmodel-coding-plan": {"options": {"apiKey": "k-cn", "baseURL": "https://open.bigmodel.cn"}},
                "builtin:no-key": {"options": {}}
            }}"#,
        )
        .unwrap();
        let keys = zcode_config_keys(&home);
        assert_eq!(keys.len(), 2);
        let global = keys
            .iter()
            .find(|k| k.source == "zcode:builtin:zai-coding-plan")
            .unwrap();
        let cn = keys
            .iter()
            .find(|k| k.source == "zcode:builtin:bigmodel-coding-plan")
            .unwrap();
        assert_eq!(global.region, Region::Global);
        assert_eq!(global.key.expose(), "k-global");
        assert_eq!(cn.region, Region::Cn);

        std::fs::write(
            home.join(".zcode").join("v2").join("config.json"),
            r#"{"providers": [{"id": "builtin:zai-coding-plan", "options": {"apiKey": "arr-key"}}]}"#,
        )
        .unwrap();
        let keys = zcode_config_keys(&home);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key.expose(), "arr-key");
    }
}
