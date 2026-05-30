//! Persistent hew configuration.
//!
//! Stored as TOML at `<XDG_CONFIG_HOME>/hew/config.toml`, falling back
//! to `~/.config/hew/config.toml`. All fields have sensible defaults so
//! a missing file is not an error.

use std::collections::BTreeMap;
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
    #[serde(rename = "loop")]
    pub loop_cfg: LoopConfig,
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
            loop_cfg: LoopConfig::default(),
        }
    }
}

/// `hew loop` runtime knobs that persist across invocations. CLI flags
/// on `hew loop run` always override these per-run. Per
/// `DECISION:loop-fallback-policy`, both the CLI flag and this config
/// knob ship together so the user can pick either surface.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LoopConfig {
    /// Fallback runtime to switch to when the primary trips a
    /// [`crate::runtime::SpawnFailureClass::RuntimeError`]. `None` =
    /// no fallback (today's behavior). Accepts `"claude"` / `"codex"`.
    pub fallback_runtime: Option<String>,
    /// Iters the loop stays on the fallback before retrying the
    /// primary. `None` → default 3 per `DECISION:loop-fallback-policy`.
    pub fallback_cooldown_iters: Option<u32>,
    /// Per-task model selection knobs consumed by the dynamic-model
    /// resolver (epic `hew-1tq`). All-None / empty by default.
    pub model: LoopModelConfig,
    /// Planner-spawn knobs consumed by the iter-end batch-plan hook
    /// when `hew loop run --jobs N >= 2` (epic `hew-lf40` /
    /// `hew-7k1m`). Disabled / `0` for jobs == 1 — the entire layer
    /// is bypassed so the fast path stays free of planner overhead.
    pub planner: LoopPlannerConfig,
    /// End-of-run verification knobs. Opt-in (`verify_tests = false`
    /// by default) so existing runs stay byte-identical to today.
    /// See `hew-bon7`.
    pub end_of_run: LoopEndOfRunConfig,
}

/// `loop.end_of_run.*` knobs. Mandatory end-of-run test step that
/// proves the final stacked state is green before the loop reports
/// success. Off by default; flip via `loop.end_of_run.verify_tests`
/// in config or `--verify-tests` on `hew loop run`.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LoopEndOfRunConfig {
    /// Run the verify step at all. Default `false`.
    pub verify_tests: bool,
    /// User-supplied verify command (e.g. `"cargo nextest run --workspace"`).
    /// Empty = let [`crate::gate::detect`] resolve from project-authored
    /// signals (justfile/Makefile/package.json `test`).
    pub verify_command: String,
    /// Wall-clock cap on the verify step. `"10m"` default.
    pub verify_budget_wall: String,
}

impl Default for LoopEndOfRunConfig {
    fn default() -> Self {
        Self {
            verify_tests: false,
            verify_command: String::new(),
            verify_budget_wall: "10m".into(),
        }
    }
}

/// Planner-spawn knobs. The planner is the inter-iter advisor that
/// produces a [`crate::batch_plan::BatchPlan`] for the next iter when
/// the previous iter's agent output did not name one. All fields are
/// per-run overridable via CLI flags on `hew loop run` (see
/// `--no-planner` / `--planner-budget` / `--planner-runtime`).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LoopPlannerConfig {
    /// Whether the planner runs at all. `false` ⇒ every iter-end that
    /// doesn't find an agent-named batch writes a `Skipped` plan with
    /// `reason = "planner_disabled"` instead of spawning. Default
    /// `true`.
    pub enabled: bool,
    /// Pre-spawn token-estimate budget. The planner refuses to spawn
    /// when the assembled prompt would exceed this (and emits a
    /// `Skipped` plan with `reason = "budget_exceeded: ..."`). Default
    /// `10_000`. `0` is treated as "always exceeded" — useful for
    /// disabling planner spawns without flipping `enabled`.
    pub budget_tokens: u32,
    /// Runtime to drive the planner. `None` ⇒ inherit the loop's
    /// primary runtime. Accepts `"claude"` / `"codex"`.
    pub runtime: Option<String>,
}

impl Default for LoopPlannerConfig {
    fn default() -> Self {
        Self { enabled: true, budget_tokens: 10_000, runtime: None }
    }
}

/// Persistent inputs for the dynamic per-task model resolver. Model
/// names are free-form strings passed verbatim to the spawner; the
/// runtime CLI rejects unknown ones at invocation time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct LoopModelConfig {
    /// Fallback model used when no `by_priority` / `by_type` rule
    /// matches the task. `None` means "let the resolver fall through
    /// to the runtime's own default".
    pub default: Option<String>,
    /// Model override keyed by task priority label (`P0`..`P4`).
    pub by_priority: BTreeMap<String, String>,
    /// Model override keyed by task type (`task`, `bug`, `chore`,
    /// `feature`, `epic`, ...).
    pub by_type: BTreeMap<String, String>,
}

/// Effective default for `fallback_cooldown_iters` when neither the
/// CLI nor config provides one. Anchored in `DECISION:loop-fallback-policy`.
pub const FALLBACK_COOLDOWN_ITERS_DEFAULT: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct BranchingConfig {
    /// `none` | `epic` | `always`. Controls when hew-execute first-claim
    /// auto-creates a branch. Default `none` (manual via `hew branch new`).
    pub strategy: String,
}

impl Default for BranchingConfig {
    fn default() -> Self {
        // `epic` is the recommended out-of-box default: one branch per epic,
        // matching how most hew users actually structure their work. Switched
        // from `none` in IV.6 (hew-j7h).
        Self { strategy: "epic".to_string() }
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
    /// When true, `hew task close` auto-appends a symbol-level
    /// changelog (`hew blast`) to the task's notes — so the bd graph
    /// later answers "which functions / classes did this task move?"
    /// Requires the binary to be built with `--features treesitter`;
    /// otherwise the flag is silently ignored. Default `false`.
    pub symbol_trace: bool,
}

impl Default for CraftConfig {
    fn default() -> Self {
        Self { max_function_lines: 0, warn_on_unused: true, symbol_trace: false }
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SkillMode {
    Yes,
    No,
    #[default]
    Ask,
}

// Hand-rolled Deserialize so on-disk configs from the pre-tri-state era
// (where these fields were `bool`) still load: `true` -> Yes, `false` -> No.
impl<'de> Deserialize<'de> for SkillMode {
    fn deserialize<D>(de: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = SkillMode;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("\"yes\" | \"no\" | \"ask\" (or legacy bool)")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<SkillMode, E> {
                Ok(if v { SkillMode::Yes } else { SkillMode::No })
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<SkillMode, E> {
                match v.to_ascii_lowercase().as_str() {
                    "yes" => Ok(SkillMode::Yes),
                    "no" => Ok(SkillMode::No),
                    "ask" => Ok(SkillMode::Ask),
                    _ => Err(E::custom(format!("expected yes|no|ask (or legacy bool), got `{v}`"))),
                }
            }
        }
        de.deserialize_any(V)
    }
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
    // `HEW_CONFIG` is the documented escape hatch for tests / scripts:
    // it bypasses layering entirely and treats the named file as the
    // sole config source.
    if let Ok(p) = std::env::var("HEW_CONFIG") {
        return load_from(&PathBuf::from(p));
    }
    use etcetera::BaseStrategy;
    let strategy = etcetera::choose_base_strategy().map_err(|e| HewError::Io(io_other(e)))?;
    let user_path = strategy.config_dir().join("hew").join("config.toml");
    let cwd = std::env::current_dir().map_err(HewError::Io)?;
    let project_path = discover_project_root(&cwd).and_then(|root| discover_project_config(&root));
    load_layered(Some(&user_path), project_path.as_deref())
}

pub fn load_from(path: &Path) -> Result<Config> {
    match std::fs::read_to_string(path) {
        Ok(s) => toml::from_str(&s).map_err(|e| HewError::Io(io_other(e))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(HewError::Io(e)),
    }
}

/// Read the user and project config files (each missing = empty
/// [`Config`]) and merge them per the documented rules: project wins
/// for scalars, `Option::or` for `Option<T>`, concat+dedupe for
/// `Vec<T>`, recursive per-field merge for nested structs, and
/// project-wins-on-collision for `BTreeMap`. Project-side absent fields
/// still deserialize to `serde(default)` values — write project configs
/// sparsely.
pub fn load_layered(user: Option<&Path>, project: Option<&Path>) -> Result<Config> {
    let mut merged = match user {
        Some(p) if p.is_file() => load_from(p)?,
        _ => Config::default(),
    };
    if let Some(p) = project
        && p.is_file()
    {
        let project_cfg = load_from(p)?;
        merged.merge(project_cfg);
    }
    Ok(merged)
}

impl Config {
    /// Layer `other` (the project config) on top of `self` (the user
    /// config) in place. See [`load_layered`] for the merge contract.
    pub fn merge(&mut self, other: Config) {
        // Bare scalars: project wins outright. Because we serde(default)
        // every field, "absent in the project file" deserializes to the
        // default value — so projects should be written sparsely or risk
        // clobbering the user-level setting back to default.
        self.update_check = other.update_check;
        self.git_track = other.git_track;
        // Option<T>: project None falls back to user's Some.
        self.default_runtime = other.default_runtime.or_else(|| self.default_runtime.take());
        self.default_scope = other.default_scope.or_else(|| self.default_scope.take());
        // Nested structs: recurse per-field.
        self.optional_skills.merge(other.optional_skills);
        self.branching.merge(other.branching);
        self.research.merge(other.research);
        self.review.merge(other.review);
        self.testing.merge(other.testing);
        self.craft.merge(other.craft);
        self.compact.merge(other.compact);
        self.loop_cfg.merge(other.loop_cfg);
    }
}

impl OptionalSkills {
    pub fn merge(&mut self, other: OptionalSkills) {
        self.deps = other.deps;
        self.research = other.research;
        self.security = other.security;
    }
}

impl BranchingConfig {
    pub fn merge(&mut self, other: BranchingConfig) {
        self.strategy = other.strategy;
    }
}

impl ResearchConfig {
    pub fn merge(&mut self, other: ResearchConfig) {
        self.default = other.default;
    }
}

impl ReviewConfig {
    pub fn merge(&mut self, other: ReviewConfig) {
        self.after_n_tasks = other.after_n_tasks;
        self.after_epic = other.after_epic;
        self.batch_size = other.batch_size;
    }
}

impl TestingConfig {
    pub fn merge(&mut self, other: TestingConfig) {
        self.require = other.require;
    }
}

impl CraftConfig {
    pub fn merge(&mut self, other: CraftConfig) {
        self.max_function_lines = other.max_function_lines;
        self.warn_on_unused = other.warn_on_unused;
        self.symbol_trace = other.symbol_trace;
    }
}

impl CompactConfig {
    pub fn merge(&mut self, other: CompactConfig) {
        self.dry_run_default = other.dry_run_default;
        self.granularity_default = other.granularity_default;
        self.target_clusters_cap = other.target_clusters_cap;
        self.allow_recompact_default = other.allow_recompact_default;
        merge_vec_dedup(&mut self.exempt, other.exempt);
    }
}

impl LoopConfig {
    pub fn merge(&mut self, other: LoopConfig) {
        self.fallback_runtime = other.fallback_runtime.or_else(|| self.fallback_runtime.take());
        self.fallback_cooldown_iters =
            other.fallback_cooldown_iters.or(self.fallback_cooldown_iters);
        self.model.merge(other.model);
        self.planner.merge(other.planner);
        self.end_of_run.merge(other.end_of_run);
    }
}

impl LoopModelConfig {
    pub fn merge(&mut self, other: LoopModelConfig) {
        self.default = other.default.or_else(|| self.default.take());
        // BTreeMap: extend; project wins on key collision.
        for (k, v) in other.by_priority {
            self.by_priority.insert(k, v);
        }
        for (k, v) in other.by_type {
            self.by_type.insert(k, v);
        }
    }
}

impl LoopPlannerConfig {
    pub fn merge(&mut self, other: LoopPlannerConfig) {
        self.enabled = other.enabled;
        self.budget_tokens = other.budget_tokens;
        self.runtime = other.runtime.or_else(|| self.runtime.take());
    }
}

impl LoopEndOfRunConfig {
    pub fn merge(&mut self, other: LoopEndOfRunConfig) {
        self.verify_tests = other.verify_tests;
        self.verify_command = other.verify_command;
        self.verify_budget_wall = other.verify_budget_wall;
    }
}

fn merge_vec_dedup<T: PartialEq>(base: &mut Vec<T>, other: Vec<T>) {
    for item in other {
        if !base.contains(&item) {
            base.push(item);
        }
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
        "craft.symbol_trace" | "craft.symbol-trace" => Some(cfg.craft.symbol_trace.to_string()),
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
        "loop.fallback_runtime" | "loop.fallback-runtime" => cfg.loop_cfg.fallback_runtime.clone(),
        "loop.fallback_cooldown_iters" | "loop.fallback-cooldown-iters" => {
            cfg.loop_cfg.fallback_cooldown_iters.map(|n| n.to_string())
        }
        "loop.model.default" => cfg.loop_cfg.model.default.clone(),
        "loop.model.by_priority" | "loop.model.by-priority" => {
            Some(format_map(&cfg.loop_cfg.model.by_priority))
        }
        "loop.model.by_type" | "loop.model.by-type" => {
            Some(format_map(&cfg.loop_cfg.model.by_type))
        }
        "loop.planner.enabled" => Some(cfg.loop_cfg.planner.enabled.to_string()),
        "loop.planner.budget_tokens" | "loop.planner.budget-tokens" => {
            Some(cfg.loop_cfg.planner.budget_tokens.to_string())
        }
        "loop.planner.runtime" => cfg.loop_cfg.planner.runtime.clone(),
        "loop.end_of_run.verify_tests" | "loop.end_of_run.verify-tests" => {
            Some(cfg.loop_cfg.end_of_run.verify_tests.to_string())
        }
        "loop.end_of_run.verify_command" | "loop.end_of_run.verify-command" => {
            Some(cfg.loop_cfg.end_of_run.verify_command.clone())
        }
        "loop.end_of_run.verify_budget_wall" | "loop.end_of_run.verify-budget-wall" => {
            Some(cfg.loop_cfg.end_of_run.verify_budget_wall.clone())
        }
        k if k.starts_with("loop.model.by_priority.")
            || k.starts_with("loop.model.by-priority.") =>
        {
            let sub = k.rsplit_once('.').map(|(_, s)| s).unwrap_or_default();
            cfg.loop_cfg.model.by_priority.get(sub).cloned()
        }
        k if k.starts_with("loop.model.by_type.") || k.starts_with("loop.model.by-type.") => {
            let sub = k.rsplit_once('.').map(|(_, s)| s).unwrap_or_default();
            cfg.loop_cfg.model.by_type.get(sub).cloned()
        }
        _ => None,
    }
}

fn format_map(m: &BTreeMap<String, String>) -> String {
    m.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
}

fn parse_map(v: &str) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for entry in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (k, val) = entry.split_once('=').ok_or_else(|| HewError::MissingFlag {
            flag: format!("value (expected comma-separated KEY=VALUE pairs, got `{v}`)"),
        })?;
        let k = k.trim();
        let val = val.trim();
        if k.is_empty() || val.is_empty() {
            return Err(HewError::MissingFlag {
                flag: format!("value (empty key or value in `{entry}`)"),
            });
        }
        out.insert(k.to_string(), val.to_string());
    }
    Ok(out)
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
        "craft.symbol_trace" | "craft.symbol-trace" => cfg.craft.symbol_trace = bool_val(value)?,
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
        "loop.fallback_runtime" | "loop.fallback-runtime" => {
            if value.is_empty() {
                cfg.loop_cfg.fallback_runtime = None;
            } else {
                if !crate::runtime::RuntimeKind::VARIANTS.contains(&value) {
                    return Err(HewError::MissingFlag {
                        flag: format!(
                            "value (expected one of {}, got `{value}`)",
                            crate::runtime::RuntimeKind::VARIANTS.join("|")
                        ),
                    });
                }
                cfg.loop_cfg.fallback_runtime = Some(value.to_string());
            }
        }
        "loop.fallback_cooldown_iters" | "loop.fallback-cooldown-iters" => {
            if value.is_empty() {
                cfg.loop_cfg.fallback_cooldown_iters = None;
            } else {
                let n: u32 = value.parse().map_err(|_| HewError::MissingFlag {
                    flag: format!("value (expected positive integer, got `{value}`)"),
                })?;
                if n == 0 {
                    return Err(HewError::MissingFlag {
                        flag: "value (loop.fallback_cooldown_iters must be >= 1)".to_string(),
                    });
                }
                cfg.loop_cfg.fallback_cooldown_iters = Some(n);
            }
        }
        "loop.model.default" => {
            cfg.loop_cfg.model.default =
                if value.is_empty() { None } else { Some(value.to_string()) };
        }
        "loop.model.by_priority" | "loop.model.by-priority" => {
            cfg.loop_cfg.model.by_priority = parse_map(value)?;
        }
        "loop.model.by_type" | "loop.model.by-type" => {
            cfg.loop_cfg.model.by_type = parse_map(value)?;
        }
        k if k.starts_with("loop.model.by_priority.")
            || k.starts_with("loop.model.by-priority.") =>
        {
            let sub = k.rsplit_once('.').map(|(_, s)| s).unwrap_or_default();
            if sub.is_empty() {
                return Err(HewError::MissingFlag { flag: format!("key (missing sub-key: {k})") });
            }
            if value.is_empty() {
                cfg.loop_cfg.model.by_priority.remove(sub);
            } else {
                cfg.loop_cfg.model.by_priority.insert(sub.to_string(), value.to_string());
            }
        }
        k if k.starts_with("loop.model.by_type.") || k.starts_with("loop.model.by-type.") => {
            let sub = k.rsplit_once('.').map(|(_, s)| s).unwrap_or_default();
            if sub.is_empty() {
                return Err(HewError::MissingFlag { flag: format!("key (missing sub-key: {k})") });
            }
            if value.is_empty() {
                cfg.loop_cfg.model.by_type.remove(sub);
            } else {
                cfg.loop_cfg.model.by_type.insert(sub.to_string(), value.to_string());
            }
        }
        "loop.planner.enabled" => cfg.loop_cfg.planner.enabled = bool_val(value)?,
        "loop.planner.budget_tokens" | "loop.planner.budget-tokens" => {
            let n: u32 = value.parse().map_err(|_| HewError::MissingFlag {
                flag: format!("value (expected non-negative integer, got `{value}`)"),
            })?;
            cfg.loop_cfg.planner.budget_tokens = n;
        }
        "loop.planner.runtime" => {
            if value.is_empty() {
                cfg.loop_cfg.planner.runtime = None;
            } else {
                if !crate::runtime::RuntimeKind::VARIANTS.contains(&value) {
                    return Err(HewError::MissingFlag {
                        flag: format!(
                            "value (expected one of {}, got `{value}`)",
                            crate::runtime::RuntimeKind::VARIANTS.join("|")
                        ),
                    });
                }
                cfg.loop_cfg.planner.runtime = Some(value.to_string());
            }
        }
        "loop.end_of_run.verify_tests" | "loop.end_of_run.verify-tests" => {
            cfg.loop_cfg.end_of_run.verify_tests = bool_val(value)?;
        }
        "loop.end_of_run.verify_command" | "loop.end_of_run.verify-command" => {
            cfg.loop_cfg.end_of_run.verify_command = value.to_string();
        }
        "loop.end_of_run.verify_budget_wall" | "loop.end_of_run.verify-budget-wall" => {
            if value.is_empty() {
                cfg.loop_cfg.end_of_run.verify_budget_wall = "10m".into();
            } else {
                // Validate parseability — same s/m/h grammar as
                // `--budget-wall`. Reject bad values at set-time so
                // `hew loop run` doesn't trip on a stale config.
                parse_budget_wall(value).map_err(|e| HewError::MissingFlag {
                    flag: format!("value (expected s/m/h duration, got `{value}`: {e})"),
                })?;
                cfg.loop_cfg.end_of_run.verify_budget_wall = value.to_string();
            }
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
        "craft.symbol_trace",
        "compact.dry_run_default",
        "compact.granularity_default",
        "compact.target_clusters_cap",
        "compact.allow_recompact_default",
        "compact.exempt",
        "loop.fallback_runtime",
        "loop.fallback_cooldown_iters",
        "loop.model.default",
        "loop.model.by_priority",
        "loop.model.by_type",
        "loop.planner.enabled",
        "loop.planner.budget_tokens",
        "loop.planner.runtime",
        "loop.end_of_run.verify_tests",
        "loop.end_of_run.verify_command",
        "loop.end_of_run.verify_budget_wall",
    ]
}

/// Parse a `loop.end_of_run.verify_budget_wall` string into a
/// [`std::time::Duration`]. Accepts `<N>s` / `<N>m` / `<N>h`. Bare
/// helper here (not the CLI's `parse_duration`) so config-side
/// validation doesn't require pulling in the binary crate.
pub fn parse_budget_wall(raw: &str) -> Result<std::time::Duration> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(HewError::MissingFlag { flag: "empty duration".into() });
    }
    let (num, unit) = raw.split_at(raw.len() - 1);
    let n: u64 = num
        .parse()
        .map_err(|e| HewError::MissingFlag { flag: format!("invalid number `{num}`: {e}") })?;
    let dur = match unit {
        "s" => std::time::Duration::from_secs(n),
        "m" => std::time::Duration::from_secs(n * 60),
        "h" => std::time::Duration::from_secs(n * 3600),
        other => {
            return Err(HewError::MissingFlag {
                flag: format!("unknown duration unit `{other}` (expected s/m/h)"),
            });
        }
    };
    Ok(dur)
}

/// Walk `cwd` ancestors looking for the project root. At each level,
/// `.beads/` wins over `.git`; the first hit terminates the walk.
/// When `.git` is a file (worktree gitlink — common in hew's own
/// `~/.hew/wt/<run-id>/<n>/` workers), resolves to the underlying
/// main-repo working tree via `git rev-parse --show-toplevel` rather
/// than the worktree directory itself.
///
/// Returns `None` if neither marker is found before the filesystem
/// root.
pub fn discover_project_root(cwd: &Path) -> Option<PathBuf> {
    for dir in cwd.ancestors() {
        if dir.join(".beads").is_dir() {
            return Some(dir.to_path_buf());
        }
        let git = dir.join(".git");
        let meta = match std::fs::symlink_metadata(&git) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            return Some(dir.to_path_buf());
        }
        if meta.is_file() {
            return Some(resolve_worktree_root(dir).unwrap_or_else(|| dir.to_path_buf()));
        }
    }
    None
}

/// Resolve the real repo root for a git worktree by asking git
/// directly. Returns `None` on any failure — caller falls back to the
/// worktree directory.
fn resolve_worktree_root(worktree_dir: &Path) -> Option<PathBuf> {
    // `git rev-parse --show-toplevel` inside a linked worktree returns
    // the worktree's own working dir — not what we want here. The
    // main repo's working tree is the parent of its `.git` dir, which
    // `--git-common-dir` reports (shared across all linked worktrees).
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(worktree_dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = std::str::from_utf8(&out.stdout).ok()?.trim();
    if s.is_empty() {
        return None;
    }
    let common = PathBuf::from(s);
    // common dir is typically `<main-repo>/.git`; the main repo root
    // is its parent. Bare repos have no working tree — fall back to
    // the worktree dir in that case.
    let parent = common.parent()?;
    if parent.as_os_str().is_empty() { None } else { Some(parent.to_path_buf()) }
}

/// Locate the project-local config file at `<root>`. Prefers
/// `.hew.toml` (dotfile convention); falls back to `hew.toml`. When
/// both exist, the dotfile wins and a warning is emitted so the user
/// notices the duplicate.
pub fn discover_project_config(root: &Path) -> Option<PathBuf> {
    let dotfile = root.join(".hew.toml");
    let plain = root.join("hew.toml");
    let dot_present = dotfile.is_file();
    let plain_present = plain.is_file();
    if dot_present && plain_present {
        tracing::warn!(
            target: "hew::config",
            ".hew.toml and hew.toml both present in {}; using .hew.toml",
            root.display()
        );
    }
    if dot_present {
        Some(dotfile)
    } else if plain_present {
        Some(plain)
    } else {
        None
    }
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
                "loop.fallback_runtime" => "codex",
                "loop.fallback_cooldown_iters" => "5",
                "loop.model.default" => "sonnet-4-6",
                "loop.model.by_priority" => "P0=opus-4-7,P3=haiku-4-5",
                "loop.model.by_type" => "bug=sonnet-4-6,chore=haiku-4-5",
                "loop.planner.enabled" => "true",
                "loop.planner.budget_tokens" => "20000",
                "loop.planner.runtime" => "codex",
                "loop.end_of_run.verify_tests" => "true",
                "loop.end_of_run.verify_command" => "cargo nextest run",
                "loop.end_of_run.verify_budget_wall" => "10m",
                k if k.starts_with("optional-skills.") => "yes",
                _ => "true",
            };
            set(&mut cfg, k, probe_value).expect(k);
        }
    }

    #[test]
    fn branching_strategy_validates() {
        let mut cfg = Config::default();
        assert_eq!(cfg.branching.strategy, "epic");
        set(&mut cfg, "branching.strategy", "none").unwrap();
        assert_eq!(cfg.branching.strategy, "none");
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
    fn optional_skills_load_legacy_bool_config() {
        // Pre-tri-state configs stored deps/research/security as TOML bools.
        // Loading must not hard-error — true -> Yes, false -> No.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let legacy = r#"
update_check = true

[optional_skills]
deps = true
research = false
quick = true
security = false
"#;
        std::fs::write(&path, legacy).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.optional_skills.deps, SkillMode::Yes);
        assert_eq!(loaded.optional_skills.research, SkillMode::No);
        assert_eq!(loaded.optional_skills.security, SkillMode::No);
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

    // ──────── loop.* ────────

    #[test]
    fn loop_defaults_are_off() {
        let cfg = Config::default();
        assert!(cfg.loop_cfg.fallback_runtime.is_none());
        assert!(cfg.loop_cfg.fallback_cooldown_iters.is_none());
        assert_eq!(FALLBACK_COOLDOWN_ITERS_DEFAULT, 3);
    }

    #[test]
    fn loop_fallback_runtime_validates_runtime_kind() {
        let mut cfg = Config::default();
        set(&mut cfg, "loop.fallback_runtime", "codex").unwrap();
        assert_eq!(cfg.loop_cfg.fallback_runtime.as_deref(), Some("codex"));
        set(&mut cfg, "loop.fallback-runtime", "claude").unwrap();
        assert_eq!(cfg.loop_cfg.fallback_runtime.as_deref(), Some("claude"));
        assert!(set(&mut cfg, "loop.fallback_runtime", "cursor").is_err());
        // Empty clears.
        set(&mut cfg, "loop.fallback_runtime", "").unwrap();
        assert!(cfg.loop_cfg.fallback_runtime.is_none());
    }

    #[test]
    fn loop_fallback_cooldown_iters_rejects_zero() {
        let mut cfg = Config::default();
        set(&mut cfg, "loop.fallback_cooldown_iters", "5").unwrap();
        assert_eq!(cfg.loop_cfg.fallback_cooldown_iters, Some(5));
        assert!(set(&mut cfg, "loop.fallback_cooldown_iters", "0").is_err());
        assert!(set(&mut cfg, "loop.fallback_cooldown_iters", "abc").is_err());
        // Empty clears (falls back to default at use-site).
        set(&mut cfg, "loop.fallback_cooldown_iters", "").unwrap();
        assert!(cfg.loop_cfg.fallback_cooldown_iters.is_none());
    }

    #[test]
    fn loop_fallback_runtime_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let mut cfg = Config::default();
        set(&mut cfg, "loop.fallback_runtime", "codex").unwrap();
        set(&mut cfg, "loop.fallback_cooldown_iters", "7").unwrap();
        save_to(&path, &cfg).unwrap();

        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.loop_cfg.fallback_runtime.as_deref(), Some("codex"));
        assert_eq!(loaded.loop_cfg.fallback_cooldown_iters, Some(7));
    }

    // ──────── loop.model.* ────────

    #[test]
    fn loop_model_defaults_are_empty() {
        let cfg = Config::default();
        assert!(cfg.loop_cfg.model.default.is_none());
        assert!(cfg.loop_cfg.model.by_priority.is_empty());
        assert!(cfg.loop_cfg.model.by_type.is_empty());
        assert_eq!(get(&cfg, "loop.model.default"), None);
        assert_eq!(get(&cfg, "loop.model.by_priority"), Some(String::new()));
        assert_eq!(get(&cfg, "loop.model.by_type"), Some(String::new()));
    }

    #[test]
    fn loop_model_default_is_free_form_string() {
        let mut cfg = Config::default();
        // Per task `Craft:`, no validation against a model catalogue.
        set(&mut cfg, "loop.model.default", "sonnet-4-6").unwrap();
        assert_eq!(cfg.loop_cfg.model.default.as_deref(), Some("sonnet-4-6"));
        set(&mut cfg, "loop.model.default", "some-future-model-2030").unwrap();
        assert_eq!(cfg.loop_cfg.model.default.as_deref(), Some("some-future-model-2030"));
        set(&mut cfg, "loop.model.default", "").unwrap();
        assert!(cfg.loop_cfg.model.default.is_none());
    }

    #[test]
    fn loop_model_by_priority_dotted_keys() {
        let mut cfg = Config::default();
        set(&mut cfg, "loop.model.by_priority.P0", "opus-4-7").unwrap();
        set(&mut cfg, "loop.model.by_priority.P3", "haiku-4-5").unwrap();
        assert_eq!(get(&cfg, "loop.model.by_priority.P0"), Some("opus-4-7".into()));
        assert_eq!(get(&cfg, "loop.model.by_priority.P3"), Some("haiku-4-5".into()));
        assert_eq!(get(&cfg, "loop.model.by_priority.P9"), None);
        // Comma-list view stays deterministic (BTreeMap order).
        assert_eq!(get(&cfg, "loop.model.by_priority"), Some("P0=opus-4-7,P3=haiku-4-5".into()));
        // Empty value removes the entry.
        set(&mut cfg, "loop.model.by_priority.P0", "").unwrap();
        assert!(!cfg.loop_cfg.model.by_priority.contains_key("P0"));
    }

    #[test]
    fn loop_model_by_type_dotted_keys() {
        let mut cfg = Config::default();
        set(&mut cfg, "loop.model.by_type.bug", "sonnet-4-6").unwrap();
        set(&mut cfg, "loop.model.by_type.chore", "haiku-4-5").unwrap();
        assert_eq!(get(&cfg, "loop.model.by_type.bug"), Some("sonnet-4-6".into()));
        assert_eq!(get(&cfg, "loop.model.by-type.chore"), Some("haiku-4-5".into()));
        set(&mut cfg, "loop.model.by_type.bug", "").unwrap();
        assert!(!cfg.loop_cfg.model.by_type.contains_key("bug"));
    }

    #[test]
    fn loop_model_map_bulk_set_parses_comma_list() {
        let mut cfg = Config::default();
        set(&mut cfg, "loop.model.by_priority", "P0=opus-4-7, P1=sonnet-4-6,P3=haiku-4-5").unwrap();
        assert_eq!(cfg.loop_cfg.model.by_priority.get("P0").unwrap(), "opus-4-7");
        assert_eq!(cfg.loop_cfg.model.by_priority.get("P1").unwrap(), "sonnet-4-6");
        assert_eq!(cfg.loop_cfg.model.by_priority.get("P3").unwrap(), "haiku-4-5");
        // Empty bulk-set clears the map.
        set(&mut cfg, "loop.model.by_priority", "").unwrap();
        assert!(cfg.loop_cfg.model.by_priority.is_empty());
        // Malformed entry without `=` rejected.
        assert!(set(&mut cfg, "loop.model.by_priority", "P0,P1").is_err());
        assert!(set(&mut cfg, "loop.model.by_priority", "=opus").is_err());
        assert!(set(&mut cfg, "loop.model.by_priority", "P0=").is_err());
    }

    #[test]
    fn loop_model_partial_section_parses() {
        // Only `default` present — by_priority / by_type fall back to empty.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[loop.model]
default = "sonnet-4-6"
"#,
        )
        .unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.loop_cfg.model.default.as_deref(), Some("sonnet-4-6"));
        assert!(loaded.loop_cfg.model.by_priority.is_empty());
        assert!(loaded.loop_cfg.model.by_type.is_empty());
    }

    #[test]
    fn loop_model_missing_section_uses_defaults() {
        // Pre-existing on-disk config with no [loop.model] section at all.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
update_check = true

[loop]
fallback_runtime = "codex"
"#,
        )
        .unwrap();
        let loaded = load_from(&path).unwrap();
        assert!(loaded.loop_cfg.model.default.is_none());
        assert!(loaded.loop_cfg.model.by_priority.is_empty());
        assert!(loaded.loop_cfg.model.by_type.is_empty());
    }

    #[test]
    fn loop_model_round_trips_through_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let mut cfg = Config::default();
        set(&mut cfg, "loop.model.default", "sonnet-4-6").unwrap();
        set(&mut cfg, "loop.model.by_priority.P0", "opus-4-7").unwrap();
        set(&mut cfg, "loop.model.by_priority.P3", "haiku-4-5").unwrap();
        set(&mut cfg, "loop.model.by_type.bug", "sonnet-4-6").unwrap();
        save_to(&path, &cfg).unwrap();

        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.loop_cfg.model.default.as_deref(), Some("sonnet-4-6"));
        assert_eq!(loaded.loop_cfg.model.by_priority.get("P0").unwrap(), "opus-4-7");
        assert_eq!(loaded.loop_cfg.model.by_priority.get("P3").unwrap(), "haiku-4-5");
        assert_eq!(loaded.loop_cfg.model.by_type.get("bug").unwrap(), "sonnet-4-6");
    }

    // ──────── loop.planner.* ────────

    #[test]
    fn loop_planner_config_default_is_enabled_10k_tokens() {
        let cfg = Config::default();
        assert!(cfg.loop_cfg.planner.enabled);
        assert_eq!(cfg.loop_cfg.planner.budget_tokens, 10_000);
        assert!(cfg.loop_cfg.planner.runtime.is_none());
        assert_eq!(get(&cfg, "loop.planner.enabled"), Some("true".into()));
        assert_eq!(get(&cfg, "loop.planner.budget_tokens"), Some("10000".into()));
        assert_eq!(get(&cfg, "loop.planner.runtime"), None);
    }

    #[test]
    fn config_loop_planner_get_set_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        let mut cfg = Config::default();
        set(&mut cfg, "loop.planner.enabled", "false").unwrap();
        set(&mut cfg, "loop.planner.budget_tokens", "25000").unwrap();
        set(&mut cfg, "loop.planner.runtime", "codex").unwrap();
        assert!(!cfg.loop_cfg.planner.enabled);
        assert_eq!(cfg.loop_cfg.planner.budget_tokens, 25_000);
        assert_eq!(cfg.loop_cfg.planner.runtime.as_deref(), Some("codex"));
        save_to(&path, &cfg).unwrap();

        let loaded = load_from(&path).unwrap();
        assert!(!loaded.loop_cfg.planner.enabled);
        assert_eq!(loaded.loop_cfg.planner.budget_tokens, 25_000);
        assert_eq!(loaded.loop_cfg.planner.runtime.as_deref(), Some("codex"));

        // Clear runtime back to None.
        set(&mut cfg, "loop.planner.runtime", "").unwrap();
        assert!(cfg.loop_cfg.planner.runtime.is_none());
        // Invalid runtime rejected.
        assert!(set(&mut cfg, "loop.planner.runtime", "cursor").is_err());
        // Non-numeric budget rejected.
        assert!(set(&mut cfg, "loop.planner.budget_tokens", "lots").is_err());
        // Bool variants accepted.
        set(&mut cfg, "loop.planner.enabled", "yes").unwrap();
        assert!(cfg.loop_cfg.planner.enabled);
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

    // ──────── discover_project_root / discover_project_config ────────

    fn scrub_git_env_in_process() {
        // SAFETY: integration with the host pre-commit hook can leak
        // GIT_* into our subprocess invocations. The test binary may
        // run multiple tests in threads, but these vars only need to
        // be absent at the moment we spawn git; once removed they
        // stay removed for the process lifetime.
        for var in [
            "GIT_DIR",
            "GIT_INDEX_FILE",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CONFIG",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_SYSTEM",
        ] {
            unsafe { std::env::remove_var(var) };
        }
    }

    #[test]
    fn discover_root_finds_beads_dir_first() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".beads")).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        let found = discover_project_root(&sub).unwrap();
        assert_eq!(found.canonicalize().unwrap(), root.canonicalize().unwrap());
    }

    #[test]
    fn discover_root_falls_back_to_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        let found = discover_project_root(&sub).unwrap();
        assert_eq!(found.canonicalize().unwrap(), root.canonicalize().unwrap());
    }

    #[test]
    fn discover_root_returns_none_when_neither_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("x").join("y");
        std::fs::create_dir_all(&sub).unwrap();
        // Avoid walking into an ancestor that happens to contain
        // a .git (e.g. /Users/.../hew) by canonicalizing into the
        // tempdir then asserting Option::is_none only when None.
        // If a parent contains .git, this test would resolve there —
        // which is still a valid behavior; we only assert "no panic"
        // and that the result, if Some, is an ancestor of cwd.
        if let Some(found) = discover_project_root(&sub) {
            assert!(
                sub.starts_with(&found) || sub.canonicalize().unwrap().starts_with(&found),
                "found {found:?} is not an ancestor of {sub:?}"
            );
        }
    }

    #[test]
    fn discover_root_stops_at_filesystem_root_when_no_marker_found() {
        // Walking from "/" (an existing path with no .beads/.git of
        // our making) must terminate cleanly — not loop forever.
        // We can't guarantee "/" has no .git on the host, so we only
        // assert termination.
        let _ = discover_project_root(Path::new("/"));
    }

    #[test]
    fn discover_root_resolves_worktree_to_real_repo() {
        use std::process::Command;
        if which::which("git").is_err() {
            eprintln!("git not on PATH, skipping");
            return;
        }
        scrub_git_env_in_process();

        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();

        let git = |dir: &Path, args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(dir)
                .env("GIT_AUTHOR_NAME", "hew-test")
                .env("GIT_AUTHOR_EMAIL", "hew@test.local")
                .env("GIT_COMMITTER_NAME", "hew-test")
                .env("GIT_COMMITTER_EMAIL", "hew@test.local")
                .output()
                .expect("git invocation");
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };

        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("README"), "x\n").unwrap();
        git(&repo, &["add", "README"]);
        git(&repo, &["commit", "-q", "-m", "init"]);

        let wt = tmp.path().join("worker");
        git(&repo, &["worktree", "add", "-b", "worker-br", wt.to_str().unwrap(), "main"]);

        // From inside the worktree, discover_project_root must resolve
        // to the main repo, not the worktree dir.
        let found = discover_project_root(&wt).expect("found root");
        assert_eq!(found.canonicalize().unwrap(), repo.canonicalize().unwrap());
    }

    #[test]
    fn discover_project_config_dotfile_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join(".hew.toml"), "").unwrap();
        std::fs::write(root.join("hew.toml"), "").unwrap();
        let found = discover_project_config(root).unwrap();
        assert_eq!(found, root.join(".hew.toml"));
    }

    #[test]
    fn discover_project_config_falls_back_to_plain_name() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("hew.toml"), "").unwrap();
        let found = discover_project_config(root).unwrap();
        assert_eq!(found, root.join("hew.toml"));
    }

    #[test]
    fn discover_project_config_warns_when_both_exist() {
        // The warning goes through tracing; we can't easily intercept it
        // here without pulling in a subscriber. Smoke-check that the
        // dotfile is still selected deterministically.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join(".hew.toml"), "a = 1\n").unwrap();
        std::fs::write(root.join("hew.toml"), "b = 2\n").unwrap();
        assert_eq!(discover_project_config(root), Some(root.join(".hew.toml")));
    }

    #[test]
    fn discover_project_config_returns_none_when_neither() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(discover_project_config(tmp.path()).is_none());
    }

    // ──────── load_layered ────────

    #[test]
    fn load_layered_no_files_returns_default() {
        let cfg = load_layered(None, None).unwrap();
        let def = Config::default();
        // Spot-check a few fields across kinds.
        assert_eq!(cfg.update_check, def.update_check);
        assert_eq!(cfg.branching.strategy, def.branching.strategy);
        assert!(cfg.compact.exempt.is_empty());
    }

    #[test]
    fn load_layered_user_only_matches_legacy_behavior() {
        let tmp = tempfile::tempdir().unwrap();
        let user_path = tmp.path().join("user.toml");
        std::fs::write(
            &user_path,
            r#"
update_check = false
default_runtime = "claude"

[branching]
strategy = "always"

[compact]
exempt = ["STATUS:keep"]
"#,
        )
        .unwrap();
        let cfg = load_layered(Some(&user_path), None).unwrap();
        assert!(!cfg.update_check);
        assert_eq!(cfg.default_runtime.as_deref(), Some("claude"));
        assert_eq!(cfg.branching.strategy, "always");
        assert_eq!(cfg.compact.exempt, vec!["STATUS:keep"]);
    }

    #[test]
    fn load_layered_project_overrides_user_scalar() {
        let tmp = tempfile::tempdir().unwrap();
        let user_path = tmp.path().join("user.toml");
        let project_path = tmp.path().join(".hew.toml");
        std::fs::write(
            &user_path,
            r#"
[branching]
strategy = "none"

[review]
batch_size = 4
"#,
        )
        .unwrap();
        std::fs::write(
            &project_path,
            r#"
[branching]
strategy = "always"

[review]
batch_size = 16
"#,
        )
        .unwrap();
        let cfg = load_layered(Some(&user_path), Some(&project_path)).unwrap();
        assert_eq!(cfg.branching.strategy, "always");
        assert_eq!(cfg.review.batch_size, 16);
    }

    #[test]
    fn load_layered_project_inherits_user_when_project_value_is_none() {
        let tmp = tempfile::tempdir().unwrap();
        let user_path = tmp.path().join("user.toml");
        let project_path = tmp.path().join(".hew.toml");
        // User sets the Option<String> fields; project omits them.
        std::fs::write(
            &user_path,
            r#"
default_runtime = "codex"
default_scope = "epic"

[loop]
fallback_runtime = "claude"
fallback_cooldown_iters = 9

[loop.model]
default = "sonnet-4-6"

[loop.planner]
runtime = "codex"
"#,
        )
        .unwrap();
        // Empty project file → every Option<T> deserializes to None →
        // user value should survive via Option::or.
        std::fs::write(&project_path, "").unwrap();
        let cfg = load_layered(Some(&user_path), Some(&project_path)).unwrap();
        assert_eq!(cfg.default_runtime.as_deref(), Some("codex"));
        assert_eq!(cfg.default_scope.as_deref(), Some("epic"));
        assert_eq!(cfg.loop_cfg.fallback_runtime.as_deref(), Some("claude"));
        assert_eq!(cfg.loop_cfg.fallback_cooldown_iters, Some(9));
        assert_eq!(cfg.loop_cfg.model.default.as_deref(), Some("sonnet-4-6"));
        assert_eq!(cfg.loop_cfg.planner.runtime.as_deref(), Some("codex"));
    }

    #[test]
    fn load_layered_arrays_append_and_dedupe() {
        let tmp = tempfile::tempdir().unwrap();
        let user_path = tmp.path().join("user.toml");
        let project_path = tmp.path().join(".hew.toml");
        std::fs::write(
            &user_path,
            r#"
[compact]
exempt = ["STATUS:user-a", "STATUS:shared"]
"#,
        )
        .unwrap();
        std::fs::write(
            &project_path,
            r#"
[compact]
exempt = ["STATUS:shared", "STATUS:project-b"]
"#,
        )
        .unwrap();
        let cfg = load_layered(Some(&user_path), Some(&project_path)).unwrap();
        // Order preserved: user entries first, then new project entries;
        // duplicates from project dropped.
        assert_eq!(cfg.compact.exempt, vec!["STATUS:user-a", "STATUS:shared", "STATUS:project-b"]);
    }

    #[test]
    fn load_layered_nested_table_merge_recursive() {
        let tmp = tempfile::tempdir().unwrap();
        let user_path = tmp.path().join("user.toml");
        let project_path = tmp.path().join(".hew.toml");
        std::fs::write(
            &user_path,
            r#"
[loop.model]
default = "sonnet-4-6"

[loop.model.by_priority]
P0 = "opus-user"
P3 = "haiku-user"

[loop.model.by_type]
bug = "sonnet-user"
"#,
        )
        .unwrap();
        std::fs::write(
            &project_path,
            r#"
[loop.model.by_priority]
P0 = "opus-project"
P1 = "sonnet-project"

[loop.model.by_type]
chore = "haiku-project"
"#,
        )
        .unwrap();
        let cfg = load_layered(Some(&user_path), Some(&project_path)).unwrap();
        // user-only P3 survives; user P0 overridden; new P1 added.
        assert_eq!(
            cfg.loop_cfg.model.by_priority.get("P0").map(String::as_str),
            Some("opus-project")
        );
        assert_eq!(
            cfg.loop_cfg.model.by_priority.get("P1").map(String::as_str),
            Some("sonnet-project")
        );
        assert_eq!(
            cfg.loop_cfg.model.by_priority.get("P3").map(String::as_str),
            Some("haiku-user")
        );
        // by_type: user bug + project chore both present.
        assert_eq!(cfg.loop_cfg.model.by_type.get("bug").map(String::as_str), Some("sonnet-user"));
        assert_eq!(
            cfg.loop_cfg.model.by_type.get("chore").map(String::as_str),
            Some("haiku-project")
        );
        // Option default: user kept (project omitted).
        assert_eq!(cfg.loop_cfg.model.default.as_deref(), Some("sonnet-4-6"));
    }

    // The `load()` env-driven path mutates process-global state
    // (HEW_CONFIG + cwd), so the integration smokes below are kept to
    // one combined test that scrubs around itself. Running it in
    // isolation matches how the rest of this module handles env-touchy
    // assertions.
    #[test]
    fn load_env_var_hew_config_bypasses_layering_and_project_discovery() {
        // Build a sole-file config that load() should return unchanged
        // when HEW_CONFIG points at it — even though we drop the agent
        // inside a tempdir that has both `.beads/` AND `.hew.toml`.
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path();
        std::fs::create_dir(project_root.join(".beads")).unwrap();
        std::fs::write(
            project_root.join(".hew.toml"),
            r#"
update_check = false
"#,
        )
        .unwrap();
        let sole = project_root.join("sole.toml");
        std::fs::write(
            &sole,
            r#"
update_check = true
default_runtime = "claude"
"#,
        )
        .unwrap();

        let prev_cwd = std::env::current_dir().unwrap();
        let prev_hew_config = std::env::var_os("HEW_CONFIG");
        std::env::set_current_dir(project_root).unwrap();
        // SAFETY: see other env-mutating tests in this module — env is
        // process-global, tests touching it accept the race.
        unsafe { std::env::set_var("HEW_CONFIG", &sole) };

        let cfg = load().unwrap();

        // Restore before asserting so a panic still cleans up.
        match prev_hew_config {
            Some(v) => unsafe { std::env::set_var("HEW_CONFIG", v) },
            None => unsafe { std::env::remove_var("HEW_CONFIG") },
        }
        std::env::set_current_dir(prev_cwd).unwrap();

        // HEW_CONFIG path won; project's `.hew.toml` (which would have
        // flipped update_check to false) was bypassed.
        assert!(cfg.update_check);
        assert_eq!(cfg.default_runtime.as_deref(), Some("claude"));
    }
}
