//! Read-once configuration: TOML file + env endpoint overrides.
//!
//! Read exactly once at process start and immutable afterwards (PLAN_1): a config
//! change requires a restart, which trivially upholds "superseded selections discarded".
//! Writes are atomic (pid-suffixed temp + rename) and 0600; an invalid file is
//! reported, never overwritten.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const PROVIDER_IDS: [&str; 4] = ["claude-code", "codex", "zai", "opencode"];
pub const DEFAULT_INTERVAL_SECS: u64 = 300;
pub const MIN_INTERVAL_SECS: u64 = 60;
pub const MAX_INTERVAL_SECS: u64 = 3600;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub interval: Duration,
    /// provider id → enabled. Providers whose tool is absent stay enabled but render
    /// NotInstalled (absence is not an error).
    pub enabled: BTreeMap<String, bool>,
    pub notifications: NotifyLevel,
    pub endpoints: Endpoints,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotifyLevel {
    Off,
    #[default]
    Warnings,
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoints {
    pub codex_usage: String,
    pub claude_usage: String,
    pub zai_quota: String,
    pub zai_quota_cn: String,
    pub opencode_usage: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            codex_usage: "https://chatgpt.com/backend-api/wham/usage".to_string(),
            claude_usage: "https://api.anthropic.com/api/oauth/usage".to_string(),
            zai_quota: "https://api.z.ai/api/monitor/usage/quota/limit".to_string(),
            zai_quota_cn: "https://open.bigmodel.cn/api/monitor/usage/quota/limit".to_string(),
            opencode_usage: "https://opencode.ai/zen/go/v1/usage".to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("config file {path} is invalid: {msg} (file left untouched)")]
    Invalid { path: PathBuf, msg: String },
    #[error("config I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Serialize, Deserialize, Default)]
pub struct FileConfig {
    interval: Option<u64>,
    providers: Option<BTreeMap<String, bool>>,
    notifications: Option<FileNotify>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileNotify {
    level: Option<String>,
}

pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("runwaybar").join("config.toml"))
}

impl Config {
    /// Load from the user config file (creating defaults on first run), then apply env
    /// endpoint overrides. Fails loudly on an invalid file without touching it.
    pub fn load() -> Result<Config, ConfigError> {
        let path = config_path().ok_or_else(|| ConfigError::Invalid {
            path: PathBuf::from("$XDG_CONFIG_HOME"),
            msg: "no user config dir resolvable".to_string(),
        })?;
        if !path.exists() {
            let defaults = Self::default();
            write_defaults(&path, &defaults)?;
            return Ok(defaults);
        }
        let raw = fs::read_to_string(&path)?;
        let parsed: FileConfig = toml::from_str(&raw).map_err(|e| ConfigError::Invalid {
            path: path.clone(),
            msg: e.to_string(),
        })?;
        Ok(Self::from_file(parsed))
    }

    pub fn from_file(f: FileConfig) -> Config {
        let interval = f
            .interval
            .unwrap_or(DEFAULT_INTERVAL_SECS)
            .clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS);
        let mut enabled = BTreeMap::new();
        for id in PROVIDER_IDS {
            enabled.insert(
                id.to_string(),
                f.providers
                    .as_ref()
                    .and_then(|p| p.get(id))
                    .copied()
                    .unwrap_or(true),
            );
        }
        let notifications = match f.notifications.and_then(|n| n.level).as_deref() {
            Some("off") => NotifyLevel::Off,
            Some("all") => NotifyLevel::All,
            _ => NotifyLevel::Warnings,
        };
        let mut endpoints = Endpoints::default();
        endpoints.apply_env();
        Config {
            interval: Duration::from_secs(interval),
            enabled,
            notifications,
            endpoints,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let enabled = PROVIDER_IDS
            .iter()
            .map(|id| (id.to_string(), true))
            .collect();
        let mut endpoints = Endpoints::default();
        endpoints.apply_env();
        Config {
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
            enabled,
            notifications: NotifyLevel::Warnings,
            endpoints,
        }
    }
}

impl Endpoints {
    /// Env overrides, https-only: an http:// override is rejected (value ignored with
    /// an explicit error rather than silently downgraded).
    fn apply_env(&mut self) {
        // Test hook: integration tests point providers at a loopback mock server.
        // Loopback-only, so it grants no capability the user does not already have
        // over their own process.
        let allow_http = std::env::var("RUNWAYBAR_TEST_ALLOW_HTTP").ok().as_deref() == Some("1");
        // Strict loopback check on the authority component: no userinfo, so
        // `http://127.0.0.1:1@evil/` must NOT pass and leak a token off-loopback.
        let is_loopback = |v: &str| -> bool {
            let Some(rest) = v.strip_prefix("http://") else {
                return false;
            };
            let authority = rest.split('/').next().unwrap_or("");
            !authority.contains('@')
                && authority
                    .strip_prefix("127.0.0.1:")
                    .and_then(|port| port.parse::<u16>().ok())
                    .is_some()
        };
        let try_set = |slot: &mut String, var: &str| {
            if let Ok(v) = std::env::var(var) {
                if v.starts_with("https://") || (allow_http && is_loopback(&v)) {
                    *slot = v;
                } else {
                    eprintln!("runwaybar: ignoring non-https {var} override");
                }
            }
        };
        try_set(&mut self.codex_usage, "CODEX_USAGE_ENDPOINT");
        try_set(&mut self.claude_usage, "CLAUDE_USAGE_ENDPOINT");
        try_set(&mut self.zai_quota, "Z_AI_QUOTA_ENDPOINT");
        try_set(&mut self.zai_quota_cn, "Z_AI_QUOTA_CN_ENDPOINT");
        try_set(&mut self.opencode_usage, "OPENCODE_USAGE_ENDPOINT");
    }
}

fn write_defaults(path: &std::path::Path, cfg: &Config) -> std::result::Result<(), ConfigError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let body = format!(
        "# RunwayBar configuration (read once at start; restart to apply changes)\n\
         interval = {}\n\n\
         [providers]\n{}\n\n\
         [notifications]\n# off | warnings | all\nlevel = \"{}\"\n",
        cfg.interval.as_secs(),
        cfg.enabled
            .iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect::<Vec<_>>()
            .join("\n"),
        match cfg.notifications {
            NotifyLevel::Off => "off",
            NotifyLevel::Warnings => "warnings",
            NotifyLevel::All => "all",
        },
    );
    let tmp = path.with_extension(format!("toml.tmp.{}", std::process::id()));
    {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(body.as_bytes())?;
        set_file_mode_0600(&tmp)?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(unix)]
fn set_file_mode_0600(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_file_mode_0600(_path: &std::path::Path) -> std::io::Result<()> {
    Ok(())
}

/// Test/CI hook: force the home root used for credential discovery.
pub fn fake_home() -> Option<PathBuf> {
    std::env::var_os("RUNWAYBAR_FAKE_HOME").map(PathBuf::from)
}

/// Home root for discovery (real HOME or the test override).
pub fn home_root() -> PathBuf {
    fake_home().unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_enable_all_providers() {
        let c = Config::default();
        assert_eq!(c.interval, Duration::from_secs(300));
        assert!(c.enabled.values().all(|v| *v));
        assert_eq!(c.notifications, NotifyLevel::Warnings);
    }

    #[test]
    fn interval_clamped_to_spec_range() {
        let f = FileConfig {
            interval: Some(5),
            ..Default::default()
        };
        assert_eq!(
            Config::from_file(f).interval,
            Duration::from_secs(MIN_INTERVAL_SECS)
        );
        let f = FileConfig {
            interval: Some(999_999),
            ..Default::default()
        };
        assert_eq!(
            Config::from_file(f).interval,
            Duration::from_secs(MAX_INTERVAL_SECS)
        );
    }

    #[test]
    fn loopback_hook_rejects_userinfo_bypass() {
        let is_loopback = |v: &str| -> bool {
            let Some(rest) = v.strip_prefix("http://") else {
                return false;
            };
            let authority = rest.split('/').next().unwrap_or("");
            !authority.contains('@')
                && authority
                    .strip_prefix("127.0.0.1:")
                    .and_then(|port| port.parse::<u16>().ok())
                    .is_some()
        };
        assert!(is_loopback("http://127.0.0.1:8080/x"));
        assert!(
            !is_loopback("http://127.0.0.1:1@evil.example/x"),
            "userinfo bypass must fail"
        );
        assert!(!is_loopback("http://evil.example/x"));
        assert!(
            !is_loopback("https://127.0.0.1:8080/x"),
            "https handled by the primary rule"
        );
    }

    #[test]
    fn notify_level_parsed_tolerantly() {
        let f = FileConfig {
            notifications: Some(FileNotify {
                level: Some("all".into()),
            }),
            ..Default::default()
        };
        assert_eq!(Config::from_file(f).notifications, NotifyLevel::All);
        let f = FileConfig {
            notifications: Some(FileNotify {
                level: Some("gibberish".into()),
            }),
            ..Default::default()
        };
        assert_eq!(Config::from_file(f).notifications, NotifyLevel::Warnings);
    }

    #[test]
    fn invalid_toml_reports_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "interval = ").unwrap();
        let err = toml::from_str::<FileConfig>(&fs::read_to_string(&path).unwrap());
        assert!(err.is_err());
        // The error message names the file's problem and the file is untouched.
        assert_eq!(fs::read_to_string(&path).unwrap(), "interval = ");
    }

    #[test]
    fn defaults_file_written_atomically_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let cfg = Config::default();
        write_defaults(&path, &cfg).unwrap();
        let meta = fs::metadata(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
        assert!(path.exists());
        assert!(dir.path().join("nested").exists());
        // No temp leftovers.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("nested"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files left: {leftovers:?}");
    }
}
