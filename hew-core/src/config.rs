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
    pub compact: CompactConfig,
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
            compact: CompactConfig::default(),
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

/// Memory-compaction config. Knobs the `/hew:compact` skill + CLI
/// reads when drafting and applying a [`crate::compact::CompactPlan`].
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct CompactConfig {
    /// `/hew:compact` starts in dry-run mode unless the user passes
    /// `--apply`. Default `true` (per `DECISION:compact-safety`).
    pub dry_run_default: bool,
    /// `broad` (strict prompt → fewer, broader clusters) or `fine`
    /// (relaxed prompt → finer-grained). Default `"broad"` per
    /// `DECISION:compact-granularity-default`.
    pub granularity_default: String,
    /// Upper bound on the cluster count `default_k(n)` returns. The
    /// formula is `ceil(sqrt(n)).clamp(1, cap)`. Default `6`. Must be
    /// `>= 1`.
    pub target_clusters_cap: u32,
    /// `--allow-recompact` default. Setting this to `true` would let
    /// the skill silently re-compact already-compacted memories;
    /// strongly discouraged. Default `false` per
    /// `DECISION:compact-drift-guard`.
    pub allow_recompact_default: bool,
    /// Literal memory keys that `compact::apply` refuses to forget,
    /// regardless of plan. Hardcoded exemptions (STATUS:scan etc.)
    /// always apply in addition to this list. Default `[]`.
    pub exempt: Vec<String>,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            dry_run_default: true,
            granularity_default: "broad".to_string(),
            target_clusters_cap: 6,
            allow_recompact_default: false,
            exempt: Vec::new(),
        }
    }
}

pub const COMPACT_GRANULARITIES: &[&str] = &["broad", "fine"];

/// Tri-state opt-in for plan-chain optional skills. `Ask` means the
/// hew-plan picker prompts the user; `Yes`/`No` make the call upfront.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum SkillMode {
    Yes,
    No,
    #[default]
    Ask,
}

impl SkillMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Ask => "ask",
        }
    }
}

impl std::fmt::Display for SkillMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub const SKILL_MODES: &[&str] = &["yes", "no", "ask"];

#[derive(Debug, Clone, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(default)]
pub struct OptionalSkills {
    pub deps: SkillMode,
    pub research: SkillMode,
    pub security: SkillMode,
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
        "compact.dry_run_default" | "compact.dry-run-default" => {
            Some(cfg.compact.dry_run_default.to_string())
        }
        "compact.granularity_default" | "compact.granularity-default" => {
            Some(cfg.compact.granularity_default.clone())
        }
        "compact.target_clusters_cap" | "compact.target-clusters-cap" => {
            Some(cfg.compact.target_clusters_cap.to_string())
        }
        "compact.allow_recompact_default" | "compact.allow-recompact-default" => {
            Some(cfg.compact.allow_recompact_default.to_string())
        }
        "compact.exempt" => Some(cfg.compact.exempt.join(",")),
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

    let skill_mode_val = |v: &str| -> Result<SkillMode> {
        match v.to_ascii_lowercase().as_str() {
            "yes" => Ok(SkillMode::Yes),
            "no" => Ok(SkillMode::No),
            "ask" => Ok(SkillMode::Ask),
            _ => Err(HewError::MissingFlag {
                flag: format!("value (expected one of yes|no|ask, got `{v}`)"),
            }),
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
        "optional-skills.deps" => cfg.optional_skills.deps = skill_mode_val(value)?,
        "optional-skills.research" => cfg.optional_skills.research = skill_mode_val(value)?,
        "optional-skills.security" => cfg.optional_skills.security = skill_mode_val(value)?,
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
        "compact.dry_run_default" | "compact.dry-run-default" => {
            cfg.compact.dry_run_default = bool_val(value)?
        }
        "compact.granularity_default" | "compact.granularity-default" => {
            if !COMPACT_GRANULARITIES.contains(&value) {
                return Err(HewError::MissingFlag {
                    flag: format!(
                        "value (expected one of {}, got `{value}`)",
                        COMPACT_GRANULARITIES.join("|")
                    ),
                });
            }
            cfg.compact.granularity_default = value.to_string();
        }
        "compact.target_clusters_cap" | "compact.target-clusters-cap" => {
            let n: u32 = value.parse().map_err(|_| HewError::MissingFlag {
                flag: format!("value (expected positive integer, got `{value}`)"),
            })?;
            if n == 0 {
                return Err(HewError::MissingFlag {
                    flag: "value (compact.target_clusters_cap must be >= 1)".to_string(),
                });
            }
            cfg.compact.target_clusters_cap = n;
        }
        "compact.allow_recompact_default" | "compact.allow-recompact-default" => {
            cfg.compact.allow_recompact_default = bool_val(value)?
        }
        "compact.exempt" => {
            cfg.compact.exempt = if value.is_empty() {
                Vec::new()
            } else {
                value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
            };
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
        "optional-skills.security",
        "branching.strategy",
        "research.default",
        "review.after_n_tasks",
        "review.after_epic",
        "review.batch_size",
        "testing.require",
        "craft.max_function_lines",
        "craft.warn_on_unused",
        "compact.dry_run_default",
        "compact.granularity_default",
        "compact.target_clusters_cap",
        "compact.allow_recompact_default",
        "compact.exempt",
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
            optional_skills: OptionalSkills { deps: SkillMode::Yes, ..Default::default() },
            update_check: false,
            ..Default::default()
        };

        save_to(&path, &cfg).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.default_runtime.as_deref(), Some("claude"));
        assert_eq!(loaded.optional_skills.deps, SkillMode::Yes);
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
                "compact.granularity_default" => "fine",
                "compact.target_clusters_cap" => "8",
                "compact.exempt" => "STATUS:custom,SOMETHING:else",
                k if k.starts_with("optional-skills.") => "yes",
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

    // ──────── optional-skills.* ────────

    #[test]
    fn optional_skills_default_to_ask() {
        let cfg = Config::default();
        assert_eq!(cfg.optional_skills.deps, SkillMode::Ask);
        assert_eq!(cfg.optional_skills.research, SkillMode::Ask);
        assert_eq!(cfg.optional_skills.security, SkillMode::Ask);
        assert_eq!(get(&cfg, "optional-skills.deps"), Some("ask".into()));
        assert_eq!(get(&cfg, "optional-skills.research"), Some("ask".into()));
        assert_eq!(get(&cfg, "optional-skills.security"), Some("ask".into()));
    }

    #[test]
    fn optional_skills_quick_key_is_gone() {
        let mut cfg = Config::default();
        assert!(get(&cfg, "optional-skills.quick").is_none());
        assert!(set(&mut cfg, "optional-skills.quick", "yes").is_err());
        assert!(!keys().contains(&"optional-skills.quick"));
    }

    #[test]
    fn optional_skills_accept_yes_no_ask_case_insensitive() {
        let mut cfg = Config::default();
        set(&mut cfg, "optional-skills.deps", "yes").unwrap();
        assert_eq!(cfg.optional_skills.deps, SkillMode::Yes);
        set(&mut cfg, "optional-skills.deps", "NO").unwrap();
        assert_eq!(cfg.optional_skills.deps, SkillMode::No);
        set(&mut cfg, "optional-skills.deps", "Ask").unwrap();
        assert_eq!(cfg.optional_skills.deps, SkillMode::Ask);
    }

    #[test]
    fn optional_skills_reject_invalid_values() {
        let mut cfg = Config::default();
        assert!(set(&mut cfg, "optional-skills.deps", "true").is_err());
        assert!(set(&mut cfg, "optional-skills.research", "false").is_err());
        assert!(set(&mut cfg, "optional-skills.security", "maybe").is_err());
        assert!(set(&mut cfg, "optional-skills.security", "").is_err());
    }

    #[test]
    fn optional_skills_survive_disk_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let mut cfg = Config::default();
        set(&mut cfg, "optional-skills.deps", "yes").unwrap();
        set(&mut cfg, "optional-skills.research", "no").unwrap();
        // security stays at default Ask — should survive too.
        save_to(&path, &cfg).unwrap();

        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.optional_skills.deps, SkillMode::Yes);
        assert_eq!(loaded.optional_skills.research, SkillMode::No);
        assert_eq!(loaded.optional_skills.security, SkillMode::Ask);
    }

    // ──────── compact.* ────────

    #[test]
    fn compact_defaults_match_decisions() {
        let cfg = Config::default();
        assert!(cfg.compact.dry_run_default, "DECISION:compact-safety");
        assert_eq!(
            cfg.compact.granularity_default, "broad",
            "DECISION:compact-granularity-default"
        );
        assert_eq!(cfg.compact.target_clusters_cap, 6, "DECISION:compact-k-default");
        assert!(!cfg.compact.allow_recompact_default, "DECISION:compact-drift-guard");
        assert!(cfg.compact.exempt.is_empty());
    }

    #[test]
    fn compact_get_returns_known_keys() {
        let cfg = Config::default();
        assert_eq!(get(&cfg, "compact.dry_run_default"), Some("true".into()));
        assert_eq!(get(&cfg, "compact.granularity-default"), Some("broad".into()));
        assert_eq!(get(&cfg, "compact.target_clusters_cap"), Some("6".into()));
        assert_eq!(get(&cfg, "compact.allow-recompact-default"), Some("false".into()));
        assert_eq!(get(&cfg, "compact.exempt"), Some(String::new()));
    }

    #[test]
    fn compact_granularity_validates() {
        let mut cfg = Config::default();
        set(&mut cfg, "compact.granularity_default", "fine").unwrap();
        assert_eq!(cfg.compact.granularity_default, "fine");
        set(&mut cfg, "compact.granularity-default", "broad").unwrap();
        assert_eq!(cfg.compact.granularity_default, "broad");
        assert!(set(&mut cfg, "compact.granularity_default", "ultra-fine").is_err());
    }

    #[test]
    fn compact_target_clusters_cap_rejects_zero() {
        let mut cfg = Config::default();
        set(&mut cfg, "compact.target_clusters_cap", "8").unwrap();
        assert_eq!(cfg.compact.target_clusters_cap, 8);
        assert!(set(&mut cfg, "compact.target_clusters_cap", "0").is_err());
        assert!(set(&mut cfg, "compact.target_clusters_cap", "abc").is_err());
    }

    #[test]
    fn compact_exempt_parses_comma_list() {
        let mut cfg = Config::default();
        set(&mut cfg, "compact.exempt", "STATUS:foo, DECISION:bar,SECURITY:baz").unwrap();
        assert_eq!(cfg.compact.exempt, vec!["STATUS:foo", "DECISION:bar", "SECURITY:baz"]);
        assert_eq!(
            get(&cfg, "compact.exempt"),
            Some("STATUS:foo,DECISION:bar,SECURITY:baz".into())
        );
        // Empty clears the list.
        set(&mut cfg, "compact.exempt", "").unwrap();
        assert!(cfg.compact.exempt.is_empty());
    }

    #[test]
    fn compact_bool_keys_accept_bool() {
        let mut cfg = Config::default();
        set(&mut cfg, "compact.dry_run_default", "false").unwrap();
        assert!(!cfg.compact.dry_run_default);
        set(&mut cfg, "compact.allow-recompact-default", "true").unwrap();
        assert!(cfg.compact.allow_recompact_default);
        assert!(set(&mut cfg, "compact.dry_run_default", "later").is_err());
    }

    #[test]
    fn compact_keys_survive_disk_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let mut cfg = Config::default();
        set(&mut cfg, "compact.dry_run_default", "false").unwrap();
        set(&mut cfg, "compact.granularity_default", "fine").unwrap();
        set(&mut cfg, "compact.target_clusters_cap", "4").unwrap();
        set(&mut cfg, "compact.allow_recompact_default", "true").unwrap();
        set(&mut cfg, "compact.exempt", "STATUS:custom,STATUS:other").unwrap();
        save_to(&path, &cfg).unwrap();

        let loaded = load_from(&path).unwrap();
        assert!(!loaded.compact.dry_run_default);
        assert_eq!(loaded.compact.granularity_default, "fine");
        assert_eq!(loaded.compact.target_clusters_cap, 4);
        assert!(loaded.compact.allow_recompact_default);
        assert_eq!(loaded.compact.exempt, vec!["STATUS:custom", "STATUS:other"]);
    }
}
