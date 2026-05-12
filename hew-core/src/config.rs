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
    pub branching: BranchingConfig,
    pub research: ResearchConfig,
    pub review: ReviewConfig,
    pub testing: TestingConfig,
    pub craft: CraftConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            update_check: true,
            default_runtime: None,
            default_scope: None,
            git_track: false,
            optional_skills: OptionalSkills::default(),
            branching: BranchingConfig::default(),
            research: ResearchConfig::default(),
            review: ReviewConfig::default(),
            testing: TestingConfig::default(),
            craft: CraftConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct BranchingConfig {
    /// `none` | `epic` | `always`. Controls when hew-execute first-claim
    /// auto-creates a branch. Default `none` (manual via `hew branch new`).
    pub strategy: String,
}

impl Default for BranchingConfig {
    fn default() -> Self {
        Self { strategy: "none".to_string() }
    }
}

pub const BRANCHING_STRATEGIES: &[&str] = &["none", "epic", "always"];

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ResearchConfig {
    /// `ask` | `auto-skip` | `auto-run`. Default selection at the
    /// hew-plan research-or-decompose picker. Default `ask`.
    pub default: String,
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self { default: "ask".to_string() }
    }
}

pub const RESEARCH_DEFAULTS: &[&str] = &["ask", "auto-skip", "auto-run"];

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ReviewConfig {
    /// Fire the Step 10 picker after this many closed tasks since the last
    /// review marker. `0` disables this trigger entirely. Default `0`.
    pub after_n_tasks: u32,
    /// Fire the Step 10 picker when an epic closes (and at least one task
    /// has closed since the last review). Default `false`.
    pub after_epic: bool,
    /// Default scope size for `hew review-bundle` when `--n` is not passed.
    /// Must be `>= 1`. Default `8`.
    pub batch_size: u32,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self { after_n_tasks: 0, after_epic: false, batch_size: 8 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct TestingConfig {
    /// When `true`, hew-guard fails the close if a behavior-changing
    /// task ships without a test. Default `false` — soft warn only.
    pub require: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct CraftConfig {
    /// Soft-warn when a changed function exceeds this many lines.
    /// `0` disables the check. Default `0`.
    pub max_function_lines: u32,
    /// Soft-warn when the diff adds unused imports / dead code that
    /// language-specific lints surface. Default `true`.
    pub warn_on_unused: bool,
}

impl Default for CraftConfig {
    fn default() -> Self {
        Self { max_function_lines: 0, warn_on_unused: true }
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
        "branching.strategy" => Some(cfg.branching.strategy.clone()),
        "research.default" => Some(cfg.research.default.clone()),
        "review.after_n_tasks" | "review.after-n-tasks" => {
            Some(cfg.review.after_n_tasks.to_string())
        }
        "review.after_epic" | "review.after-epic" => Some(cfg.review.after_epic.to_string()),
        "review.batch_size" | "review.batch-size" => Some(cfg.review.batch_size.to_string()),
        "testing.require" => Some(cfg.testing.require.to_string()),
        "craft.max_function_lines" | "craft.max-function-lines" => {
            Some(cfg.craft.max_function_lines.to_string())
        }
        "craft.warn_on_unused" | "craft.warn-on-unused" => {
            Some(cfg.craft.warn_on_unused.to_string())
        }
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
        "branching.strategy" => {
            if !BRANCHING_STRATEGIES.contains(&value) {
                return Err(HewError::MissingFlag {
                    flag: format!(
                        "value (expected one of {}, got `{value}`)",
                        BRANCHING_STRATEGIES.join("|")
                    ),
                });
            }
            cfg.branching.strategy = value.to_string();
        }
        "research.default" => {
            if !RESEARCH_DEFAULTS.contains(&value) {
                return Err(HewError::MissingFlag {
                    flag: format!(
                        "value (expected one of {}, got `{value}`)",
                        RESEARCH_DEFAULTS.join("|")
                    ),
                });
            }
            cfg.research.default = value.to_string();
        }
        "review.after_n_tasks" | "review.after-n-tasks" => {
            let n: u32 = value.parse().map_err(|_| HewError::MissingFlag {
                flag: format!("value (expected non-negative integer, got `{value}`)"),
            })?;
            cfg.review.after_n_tasks = n;
        }
        "review.after_epic" | "review.after-epic" => cfg.review.after_epic = bool_val(value)?,
        "review.batch_size" | "review.batch-size" => {
            let n: u32 = value.parse().map_err(|_| HewError::MissingFlag {
                flag: format!("value (expected positive integer, got `{value}`)"),
            })?;
            if n == 0 {
                return Err(HewError::MissingFlag {
                    flag: "value (review.batch_size must be >= 1)".to_string(),
                });
            }
            cfg.review.batch_size = n;
        }
        "testing.require" => cfg.testing.require = bool_val(value)?,
        "craft.max_function_lines" | "craft.max-function-lines" => {
            let n: u32 = value.parse().map_err(|_| HewError::MissingFlag {
                flag: format!("value (expected non-negative integer, got `{value}`)"),
            })?;
            cfg.craft.max_function_lines = n;
        }
        "craft.warn_on_unused" | "craft.warn-on-unused" => {
            cfg.craft.warn_on_unused = bool_val(value)?
        }
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
        "branching.strategy",
        "research.default",
        "review.after_n_tasks",
        "review.after_epic",
        "review.batch_size",
        "testing.require",
        "craft.max_function_lines",
        "craft.warn_on_unused",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let cfg = Config {
            default_runtime: Some("claude".into()),
            optional_skills: OptionalSkills { deps: true, ..Default::default() },
            update_check: false,
            ..Default::default()
        };

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
            let probe_value = match *k {
                k if k.starts_with("default-") => "x",
                "branching.strategy" => "epic",
                "research.default" => "auto-skip",
                "review.after_n_tasks" => "5",
                "review.batch_size" => "10",
                "review.after_epic" => "true",
                "craft.max_function_lines" => "20",
                _ => "true",
            };
            set(&mut cfg, k, probe_value).expect(k);
        }
    }

    #[test]
    fn branching_strategy_validates() {
        let mut cfg = Config::default();
        assert_eq!(cfg.branching.strategy, "none");
        set(&mut cfg, "branching.strategy", "epic").unwrap();
        assert_eq!(cfg.branching.strategy, "epic");
        set(&mut cfg, "branching.strategy", "always").unwrap();
        assert_eq!(cfg.branching.strategy, "always");
        assert!(set(&mut cfg, "branching.strategy", "weekly").is_err());
    }

    #[test]
    fn review_after_n_tasks_accepts_integers() {
        let mut cfg = Config::default();
        assert_eq!(cfg.review.after_n_tasks, 0);
        set(&mut cfg, "review.after_n_tasks", "5").unwrap();
        assert_eq!(cfg.review.after_n_tasks, 5);
        set(&mut cfg, "review.after_n_tasks", "0").unwrap(); // 0 = disabled, valid
        assert_eq!(cfg.review.after_n_tasks, 0);
        assert!(set(&mut cfg, "review.after_n_tasks", "not-a-number").is_err());
        assert!(set(&mut cfg, "review.after_n_tasks", "-1").is_err());
    }

    #[test]
    fn review_after_epic_accepts_bool() {
        let mut cfg = Config::default();
        assert!(!cfg.review.after_epic);
        set(&mut cfg, "review.after_epic", "true").unwrap();
        assert!(cfg.review.after_epic);
        set(&mut cfg, "review.after_epic", "off").unwrap();
        assert!(!cfg.review.after_epic);
        assert!(set(&mut cfg, "review.after_epic", "maybe").is_err());
    }

    #[test]
    fn review_batch_size_rejects_zero() {
        let mut cfg = Config::default();
        assert_eq!(cfg.review.batch_size, 8);
        set(&mut cfg, "review.batch_size", "16").unwrap();
        assert_eq!(cfg.review.batch_size, 16);
        assert!(set(&mut cfg, "review.batch_size", "0").is_err());
        assert!(set(&mut cfg, "review.batch_size", "abc").is_err());
    }

    #[test]
    fn research_default_validates() {
        let mut cfg = Config::default();
        assert_eq!(cfg.research.default, "ask");
        set(&mut cfg, "research.default", "auto-skip").unwrap();
        assert_eq!(cfg.research.default, "auto-skip");
        set(&mut cfg, "research.default", "auto-run").unwrap();
        assert_eq!(cfg.research.default, "auto-run");
        assert!(set(&mut cfg, "research.default", "maybe").is_err());
    }

    #[test]
    fn testing_require_defaults_to_false() {
        let cfg = Config::default();
        assert!(!cfg.testing.require);
        assert_eq!(get(&cfg, "testing.require"), Some("false".into()));
    }

    #[test]
    fn testing_require_accepts_bool() {
        let mut cfg = Config::default();
        set(&mut cfg, "testing.require", "true").unwrap();
        assert!(cfg.testing.require);
        set(&mut cfg, "testing.require", "off").unwrap();
        assert!(!cfg.testing.require);
        assert!(set(&mut cfg, "testing.require", "maybe").is_err());
    }

    #[test]
    fn craft_max_function_lines_defaults_to_zero_disabled() {
        let cfg = Config::default();
        assert_eq!(cfg.craft.max_function_lines, 0);
        assert_eq!(get(&cfg, "craft.max_function_lines"), Some("0".into()));
    }

    #[test]
    fn craft_max_function_lines_accepts_integers() {
        let mut cfg = Config::default();
        set(&mut cfg, "craft.max_function_lines", "20").unwrap();
        assert_eq!(cfg.craft.max_function_lines, 20);
        set(&mut cfg, "craft.max_function_lines", "0").unwrap(); // 0 = disabled
        assert_eq!(cfg.craft.max_function_lines, 0);
        assert!(set(&mut cfg, "craft.max_function_lines", "abc").is_err());
        assert!(set(&mut cfg, "craft.max_function_lines", "-1").is_err());
    }

    #[test]
    fn craft_warn_on_unused_defaults_to_true() {
        let cfg = Config::default();
        assert!(cfg.craft.warn_on_unused);
    }

    #[test]
    fn craft_warn_on_unused_accepts_bool() {
        let mut cfg = Config::default();
        set(&mut cfg, "craft.warn_on_unused", "false").unwrap();
        assert!(!cfg.craft.warn_on_unused);
        set(&mut cfg, "craft.warn_on_unused", "yes").unwrap();
        assert!(cfg.craft.warn_on_unused);
        assert!(set(&mut cfg, "craft.warn_on_unused", "later").is_err());
    }

    #[test]
    fn config_keys_survive_disk_roundtrip() {
        // The new TestingConfig + CraftConfig must serialize through the
        // serde(default) path; missing in the on-disk file means defaults.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");

        let mut cfg = Config::default();
        set(&mut cfg, "testing.require", "true").unwrap();
        set(&mut cfg, "craft.max_function_lines", "30").unwrap();
        set(&mut cfg, "craft.warn_on_unused", "false").unwrap();
        save_to(&path, &cfg).unwrap();

        let loaded = load_from(&path).unwrap();
        assert!(loaded.testing.require);
        assert_eq!(loaded.craft.max_function_lines, 30);
        assert!(!loaded.craft.warn_on_unused);
    }
}
