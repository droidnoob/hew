//! Persistent hew configuration.
//!
//! Stored as TOML at `<XDG_CONFIG_HOME>/hew/config.toml`, falling back
//! to `~/.config/hew/config.toml`. All fields have sensible defaults so
//! a missing file is not an error.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{HewError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct Config {
    pub update_check: bool,
    pub default_runtime: Option<String>,
    pub default_scope: Option<String>,
    pub git_track: bool,
    pub optional_skills: OptionalSkills,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            update_check: true,
            default_runtime: None,
            default_scope: None,
            git_track: false,
            optional_skills: OptionalSkills::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(default)]
pub struct OptionalSkills {
    pub deps: bool,
    pub research: bool,
    pub quick: bool,
    pub security: bool,
}

/// Resolve the user-config path. Honors `HEW_CONFIG` for tests.
pub fn config_path() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("HEW_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    use etcetera::BaseStrategy;
    let strategy = etcetera::choose_base_strategy().map_err(|e| HewError::Io(io_other(e)))?;
    Ok(strategy.config_dir().join("hew").join("config.toml"))
}

fn io_other(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

pub fn load() -> Result<Config> {
    load_from(&config_path()?)
}

pub fn load_from(path: &Path) -> Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).map_err(|e| HewError::Io(io_other(e))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(HewError::Io(e)),
    }
}

pub fn save(cfg: &Config) -> Result<PathBuf> {
    let path = config_path()?;
    save_to(&path, cfg)?;
    Ok(path)
}

pub fn save_to(path: &Path, cfg: &Config) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let serialized = toml::to_string_pretty(cfg).map_err(|e| HewError::Io(io_other(e)))?;
    std::fs::write(path, serialized)?;
    Ok(())
}

/// Get a single key's value as a string (for `hew config get`).
pub fn get(cfg: &Config, key: &str) -> Option<String> {
    match key {
        "update-check" | "update_check" => Some(cfg.update_check.to_string()),
        "default-runtime" | "default_runtime" => cfg.default_runtime.clone(),
        "default-scope" | "default_scope" => cfg.default_scope.clone(),
        "git-track" | "git_track" => Some(cfg.git_track.to_string()),
        "optional-skills.deps" => Some(cfg.optional_skills.deps.to_string()),
        "optional-skills.research" => Some(cfg.optional_skills.research.to_string()),
        "optional-skills.quick" => Some(cfg.optional_skills.quick.to_string()),
        "optional-skills.security" => Some(cfg.optional_skills.security.to_string()),
        _ => None,
    }
}

/// Set a key. Returns error for unknown keys or invalid values.
pub fn set(cfg: &mut Config, key: &str, value: &str) -> Result<()> {
    let bool_val = |v: &str| -> Result<bool> {
        match v {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => {
                Err(HewError::MissingFlag { flag: format!("value (expected boolean, got `{v}`)") })
            }
        }
    };

    match key {
        "update-check" | "update_check" => cfg.update_check = bool_val(value)?,
        "default-runtime" | "default_runtime" => {
            cfg.default_runtime = if value.is_empty() { None } else { Some(value.to_string()) }
        }
        "default-scope" | "default_scope" => {
            cfg.default_scope = if value.is_empty() { None } else { Some(value.to_string()) }
        }
        "git-track" | "git_track" => cfg.git_track = bool_val(value)?,
        "optional-skills.deps" => cfg.optional_skills.deps = bool_val(value)?,
        "optional-skills.research" => cfg.optional_skills.research = bool_val(value)?,
        "optional-skills.quick" => cfg.optional_skills.quick = bool_val(value)?,
        "optional-skills.security" => cfg.optional_skills.security = bool_val(value)?,
        _ => {
            return Err(HewError::MissingFlag { flag: format!("key (unknown: {key})") });
        }
    }
    Ok(())
}

/// All settable keys, for `hew config list`.
pub fn keys() -> &'static [&'static str] {
    &[
        "update-check",
        "default-runtime",
        "default-scope",
        "git-track",
        "optional-skills.deps",
        "optional-skills.research",
        "optional-skills.quick",
        "optional-skills.security",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let mut cfg = Config::default();
        cfg.default_runtime = Some("claude".into());
        cfg.optional_skills.deps = true;
        cfg.update_check = false;

        save_to(&path, &cfg).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.default_runtime.as_deref(), Some("claude"));
        assert!(loaded.optional_skills.deps);
        assert!(!loaded.update_check);
    }

    #[test]
    fn missing_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("does-not-exist.toml");
        let cfg = load_from(&path).unwrap();
        assert!(cfg.update_check);
        assert!(cfg.default_runtime.is_none());
    }

    #[test]
    fn get_returns_known_keys() {
        let cfg = Config::default();
        assert_eq!(get(&cfg, "update-check"), Some("true".into()));
        assert_eq!(get(&cfg, "default-runtime"), None);
        assert_eq!(get(&cfg, "bogus"), None);
    }

    #[test]
    fn set_known_bool_key() {
        let mut cfg = Config::default();
        set(&mut cfg, "update-check", "false").unwrap();
        assert!(!cfg.update_check);
        set(&mut cfg, "update-check", "yes").unwrap();
        assert!(cfg.update_check);
    }

    #[test]
    fn set_string_key() {
        let mut cfg = Config::default();
        set(&mut cfg, "default-runtime", "cursor").unwrap();
        assert_eq!(cfg.default_runtime.as_deref(), Some("cursor"));
        set(&mut cfg, "default-runtime", "").unwrap();
        assert!(cfg.default_runtime.is_none());
    }

    #[test]
    fn set_unknown_key_errors() {
        let mut cfg = Config::default();
        assert!(set(&mut cfg, "nope", "x").is_err());
    }

    #[test]
    fn set_invalid_bool_errors() {
        let mut cfg = Config::default();
        assert!(set(&mut cfg, "update-check", "maybe").is_err());
    }

    #[test]
    fn keys_includes_every_settable_path() {
        let ks = keys();
        for k in ks {
            // Each key must roundtrip get/set without panic.
            let mut cfg = Config::default();
            let probe_value = if k.starts_with("default-") { "x" } else { "true" };
            set(&mut cfg, k, probe_value).expect(k);
        }
    }
}
