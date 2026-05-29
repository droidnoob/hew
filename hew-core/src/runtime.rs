//! Runtime spawner abstraction for `hew loop`.
//!
//! The loop drives one Claude (or other agent runtime) invocation per
//! iter. The [`RuntimeSpawner`] trait keeps the runner pure; the
//! production wiring is [`ClaudeSpawner`], which shells out to the
//! locally-installed Claude Code CLI.
//!
//! Verified against `claude --version 2.1.150` (Claude Code) on
//! 2026-05-26. The flags exercised here (`-p`, `--allowedTools`,
//! `--output-format json`) are stable and documented in `claude --help`
//! — bump the version note when re-confirming against a newer CLI.
//!
//! Tests use the [`MockSpawner`] in this module; the real spawner is
//! exercised by an integration test gated on `HEW_LOOP_E2E=1`.

use std::path::PathBuf;
use std::str::FromStr;

use crate::error::Result;
use crate::prompt::AssembledPrompt;
use crate::runner::TokenSpend;

/// Loop-side runtime selector. The two values are the subset of
/// [`crate::install::Runtime`] (5 variants: Claude/Cursor/Codex/Windsurf/
/// Generic) that the loop can actually drive — runtimes with a
/// non-interactive `-p`-style invocation. Install-side enums one for
/// methodology body fan-out; this one for spawner dispatch.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RuntimeKind {
    Claude,
    Codex,
}

impl RuntimeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// CLI-side accepted values for `--runtime`. Kept in sync with
    /// the `FromStr` arms so clap valid-values lists never drift.
    pub const VARIANTS: &'static [&'static str] = &["claude", "codex"];
}

impl FromStr for RuntimeKind {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            other => {
                Err(format!("unknown runtime `{other}`; expected one of {:?}", Self::VARIANTS))
            }
        }
    }
}

/// Environment variable that overrides the `claude` binary location.
/// Used by tests and for pinning to a specific install.
pub const CLAUDE_BIN_ENV: &str = "HEW_LOOP_CLAUDE_BIN";

/// Codex sandbox enum, mirroring the three values accepted by
/// `codex exec --sandbox`. The CLI string form (via [`Display`]) MUST
/// match exactly — codex rejects unknown values.
///
/// Per `DECISION:codex-sandbox-mapping` + `RESEARCH:codex-allowedtools-mapping`:
/// hew's per-iter `allowed_tools` list (a Claude concept) has no 1:1
/// codex equivalent. The mapping is lossy but deterministic — see
/// [`map_allowed_tools_to_sandbox`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SandboxPolicy {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxPolicy {
    /// CLI string accepted by `codex exec -s/--sandbox`. Stable per
    /// `RESEARCH:codex-sandbox-model` (codex-cli 0.120.0, 2026-05-29).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

impl std::fmt::Display for SandboxPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Map hew's `allowed_tools` (Claude tool names) to a codex sandbox
/// policy. If any element matches a known write-class tool name
/// (`Edit`, `Write`, `MultiEdit`, `NotebookEdit`, case-sensitive) the
/// sandbox is widened to [`SandboxPolicy::WorkspaceWrite`]; otherwise
/// the iter runs [`SandboxPolicy::ReadOnly`].
///
/// Known lossy translation: Bash subcommand restrictions like
/// `Bash(git:*)` cannot be expressed in codex's sandbox enum — they
/// are silently broadened to whatever bash the chosen sandbox allows.
/// Documented in `DECISION:codex-sandbox-mapping`. If this matters for
/// a future iter, the right fix is finer-grained codex gating, not a
/// per-call shell wrapper.
pub fn map_allowed_tools_to_sandbox(tools: &[String]) -> SandboxPolicy {
    const WRITE_CLASS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit"];
    for t in tools {
        if WRITE_CLASS.contains(&t.as_str()) {
            return SandboxPolicy::WorkspaceWrite;
        }
    }
    SandboxPolicy::ReadOnly
}

/// Sub-category of a runtime-level failure. Used to decide whether a
/// fallback runtime might succeed (`Auth` / `RateLimit` / `Server` —
/// usually yes) versus a deterministic refusal (`BadRequest` — usually
/// no). `Spawn` covers OS-level errors (missing binary, ETXTBSY).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RuntimeErrorKind {
    Spawn,
    Auth,
    RateLimit,
    BadRequest,
    Server,
    Unknown,
}

/// Categorical classification of an iter's outcome. Sits alongside the
/// raw `success` bool on [`SpawnOutcome`] so the runner can distinguish
/// "the runtime broke" (try fallback) from "guard tripped" / "budget
/// exhausted" (no point trying a different runtime).
#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub enum SpawnFailureClass {
    #[default]
    Success,
    RuntimeError(RuntimeErrorKind),
    GuardTrip,
    BudgetExhausted,
}

/// Map an HTTP status code observed in a runtime's failure payload
/// (Claude `--output-format json` error envelope, Codex `turn.failed`
/// nested status) to its [`RuntimeErrorKind`].
pub fn classify_http_status(status: u16) -> RuntimeErrorKind {
    match status {
        401 | 403 => RuntimeErrorKind::Auth,
        429 => RuntimeErrorKind::RateLimit,
        400 | 404 | 422 => RuntimeErrorKind::BadRequest,
        500..=599 => RuntimeErrorKind::Server,
        _ => RuntimeErrorKind::Unknown,
    }
}

/// Result of one iter's spawn. Richer than [`crate::runner::IterOutcome`]
/// because the runner needs both the categorical outcome *and* the raw
/// numbers (tokens, closed task id) for logging. The runner glue maps
/// this to `IterOutcome` after applying backpressure / strict rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnOutcome {
    /// Process exit was 0 AND JSON parsed cleanly.
    pub success: bool,
    /// Task id the agent closed during this iter, if any. Detected by
    /// scanning the result text for the `closed <id>` marker `hew task
    /// close` emits.
    pub closed_task: Option<String>,
    pub tokens: TokenSpend,
    /// Last lines of stderr (capped). Used for surfacing errors.
    pub stderr_tail: String,
    /// Raw result text from the runtime, post-JSON-unwrap. Logged
    /// verbatim into the iter record.
    pub raw_text: String,
    /// Categorical outcome for fallback / cooldown decisions. Defaults
    /// to [`SpawnFailureClass::Success`] for back-compat with existing
    /// spawners that haven't been wired to surface classification yet.
    pub failure_class: SpawnFailureClass,
}

/// Per-iter overrides threaded through [`RuntimeSpawner::spawn`]. Both
/// fields are `Option` so an unset value defers to whatever the spawner
/// was constructed with — `SpawnOpts::default()` is a no-op.
///
/// `model_override` injects `--model X` (Claude) or `-m X` (Codex) at
/// spawn time. Migrating model selection here unblocks per-iter / per-
/// task model resolution (Epic D) without sharing mutable state on the
/// spawner struct.
///
/// `working_dir` becomes `-C <dir>` for Codex; Claude has no equivalent
/// flag today, so the field is reserved for future use (and for the
/// worktree-parallel epic, where the runner can pre-cd the subprocess).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpawnOpts {
    pub model_override: Option<String>,
    pub working_dir: Option<PathBuf>,
}

/// Inject-at-runtime abstraction. Production wires [`ClaudeSpawner`];
/// tests use [`MockSpawner`].
///
/// The `opts` parameter (added in hew-uqg) carries per-iter overrides
/// — model selection and working directory — without polluting the
/// spawner struct's static configuration. Callers that don't need
/// per-iter control pass `&SpawnOpts::default()`.
pub trait RuntimeSpawner {
    fn spawn(
        &self,
        prompt: &AssembledPrompt,
        allowed_tools: &[String],
        opts: &SpawnOpts,
    ) -> Result<SpawnOutcome>;
}

/// Production spawner. Shells `claude -p <prompt> --allowedTools <list>
/// --output-format json`. Stateful only in the `bin` path — same
/// spawner can be reused across iters.
#[derive(Clone, Debug)]
pub struct ClaudeSpawner {
    /// Path to the `claude` binary. Defaults to `claude` (PATH lookup).
    pub bin: PathBuf,
    /// Extra CLI args appended verbatim. Used to pass through
    /// `--model`, `--max-budget-usd`, etc. from the loop CLI.
    pub extra_args: Vec<String>,
}

impl ClaudeSpawner {
    /// Resolve the binary from `HEW_LOOP_CLAUDE_BIN`, falling back to
    /// `claude` on PATH.
    pub fn from_env() -> Self {
        let bin =
            std::env::var(CLAUDE_BIN_ENV).map(PathBuf::from).unwrap_or_else(|_| "claude".into());
        Self { bin, extra_args: Vec::new() }
    }

    /// Build the argv (excluding bin) that would be passed to the
    /// subprocess. Exposed so tests can verify command construction
    /// without spawning a real process.
    ///
    /// `opts.model_override` injects `--model <X>` ahead of `extra_args`
    /// so a per-iter override wins even if `extra_args` carries a stale
    /// `--model` (Claude's CLI honors the last occurrence — keeping
    /// opts ahead of extras documents the intent and lets a future
    /// refactor strip `--model` out of `extra_args` cleanly).
    pub fn build_args(
        &self,
        prompt: &AssembledPrompt,
        allowed_tools: &[String],
        opts: &SpawnOpts,
    ) -> Vec<String> {
        let mut args = vec![
            "-p".to_string(),
            prompt.full_text.clone(),
            "--output-format".to_string(),
            "json".to_string(),
        ];
        if !allowed_tools.is_empty() {
            args.push("--allowedTools".to_string());
            args.push(allowed_tools.join(","));
        }
        if let Some(model) = opts.model_override.as_deref() {
            args.push("--model".to_string());
            args.push(model.to_string());
        }
        args.extend(self.extra_args.iter().cloned());
        args
    }
}

impl Default for ClaudeSpawner {
    fn default() -> Self {
        Self::from_env()
    }
}

impl RuntimeSpawner for ClaudeSpawner {
    fn spawn(
        &self,
        prompt: &AssembledPrompt,
        allowed_tools: &[String],
        opts: &SpawnOpts,
    ) -> Result<SpawnOutcome> {
        let args = self.build_args(prompt, allowed_tools, opts);
        let output = std::process::Command::new(&self.bin).args(&args).output()?;
        let stderr_tail = tail_text(&String::from_utf8_lossy(&output.stderr), 16);
        let exit_ok = output.status.success();
        match parse_claude_json(&output.stdout) {
            Ok((raw_text, tokens, failure_class)) => {
                let success = exit_ok && matches!(failure_class, SpawnFailureClass::Success);
                let closed_task = detect_closed_task(&raw_text);
                Ok(SpawnOutcome {
                    success,
                    closed_task,
                    tokens,
                    stderr_tail,
                    raw_text,
                    failure_class,
                })
            }
            Err(_) if !exit_ok => Ok(SpawnOutcome {
                success: false,
                closed_task: None,
                tokens: TokenSpend::default(),
                stderr_tail,
                raw_text: String::from_utf8_lossy(&output.stdout).into_owned(),
                failure_class: SpawnFailureClass::RuntimeError(RuntimeErrorKind::Unknown),
            }),
            Err(e) => Err(e),
        }
    }
}

/// Resolved fallback-runtime knob fed into the loop. CLI flags take
/// precedence over `loop.fallback_runtime` / `loop.fallback_cooldown_iters`
/// config; an unset runtime field disables fallback entirely (today's
/// behavior). Cooldown count is always positive — falls back to
/// [`crate::config::FALLBACK_COOLDOWN_ITERS_DEFAULT`] when unspecified.
/// Per `DECISION:loop-fallback-policy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FallbackConfig {
    pub runtime: Option<RuntimeKind>,
    pub cooldown_iters: u32,
}

impl FallbackConfig {
    /// Resolve from a CLI override (already parsed to [`RuntimeKind`])
    /// plus iters override, layered onto the persisted config strings.
    /// CLI wins; config is fallback. An empty or `None` CLI runtime
    /// and missing config means no fallback. The cooldown is clamped
    /// to at least 1 and defaults to
    /// [`crate::config::FALLBACK_COOLDOWN_ITERS_DEFAULT`].
    pub fn resolve(
        cli_runtime: Option<RuntimeKind>,
        cli_cooldown: Option<u32>,
        cfg_runtime: Option<&str>,
        cfg_cooldown: Option<u32>,
    ) -> std::result::Result<Self, String> {
        let runtime = match (cli_runtime, cfg_runtime) {
            (Some(r), _) => Some(r),
            (None, Some(s)) if !s.is_empty() => Some(s.parse::<RuntimeKind>()?),
            _ => None,
        };
        let cooldown = cli_cooldown
            .or(cfg_cooldown)
            .unwrap_or(crate::config::FALLBACK_COOLDOWN_ITERS_DEFAULT)
            .max(1);
        Ok(Self { runtime, cooldown_iters: cooldown })
    }
}

impl Default for FallbackConfig {
    fn default() -> Self {
        Self { runtime: None, cooldown_iters: crate::config::FALLBACK_COOLDOWN_ITERS_DEFAULT }
    }
}

/// Environment variable that overrides the `codex` binary location.
/// Used by tests and for pinning to a specific install. Mirrors
/// [`CLAUDE_BIN_ENV`].
pub const CODEX_BIN_ENV: &str = "HEW_LOOP_CODEX_BIN";

/// Production spawner for `codex exec --json`. Symmetric to
/// [`ClaudeSpawner`] — same trait, same shape, same reusability across
/// iters. The sandbox field is the explicit floor; if left at
/// [`SandboxPolicy::ReadOnly`] (default), per-iter `allowed_tools`
/// widen it via [`map_allowed_tools_to_sandbox`]. An explicit
/// `with_sandbox(...)` always takes precedence over the mapping.
///
/// Verified against `codex-cli 0.120.0` on 2026-05-29
/// (RESEARCH:codex-exec-json-stream + RESEARCH:codex-sandbox-model).
#[derive(Clone, Debug)]
pub struct CodexSpawner {
    /// Path to the `codex` binary. Defaults to `codex` (PATH lookup).
    pub bin: PathBuf,
    /// `-m <model>` override. None → omit flag (codex picks default).
    pub model_override: Option<String>,
    /// Explicit sandbox floor. ReadOnly = defer to the tools mapper.
    pub sandbox: SandboxPolicy,
    /// Extra CLI args appended after the computed flags and before
    /// the `--` / prompt separator. Used to pass through `-c key=val`
    /// overrides, `--add-dir`, `--profile`, etc.
    pub extra_args: Vec<String>,
    /// `-C <wd>` override. None → omit flag (codex uses CWD).
    pub working_dir: Option<PathBuf>,
}

impl CodexSpawner {
    /// Resolve the binary from `HEW_LOOP_CODEX_BIN`, falling back to
    /// `codex` on PATH.
    pub fn from_env() -> Self {
        let bin =
            std::env::var(CODEX_BIN_ENV).map(PathBuf::from).unwrap_or_else(|_| "codex".into());
        Self {
            bin,
            model_override: None,
            sandbox: SandboxPolicy::ReadOnly,
            extra_args: Vec::new(),
            working_dir: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model_override = Some(model.into());
        self
    }

    pub fn with_sandbox(mut self, sandbox: SandboxPolicy) -> Self {
        self.sandbox = sandbox;
        self
    }

    pub fn with_working_dir(mut self, wd: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(wd.into());
        self
    }

    /// Effective sandbox for this call. An explicit non-ReadOnly value
    /// on the spawner pins the policy; otherwise the per-iter tools
    /// list governs via [`map_allowed_tools_to_sandbox`].
    fn effective_sandbox(&self, tools: &[String]) -> SandboxPolicy {
        match self.sandbox {
            SandboxPolicy::ReadOnly => map_allowed_tools_to_sandbox(tools),
            explicit => explicit,
        }
    }

    /// Build the argv (excluding bin) passed to `codex exec`. The
    /// canonical shape per RESEARCH:codex-useful-flags is
    /// `exec --json --skip-git-repo-check --sandbox <X> [-m <model>]
    /// [-C <wd>] <extra_args...> -- <prompt>`.
    ///
    /// `--` separates flags from the positional prompt so a prompt
    /// starting with `-` cannot be mis-parsed as a flag.
    pub fn build_args(
        &self,
        prompt: &AssembledPrompt,
        allowed_tools: &[String],
        opts: &SpawnOpts,
    ) -> Vec<String> {
        let mut args = vec![
            "exec".to_string(),
            "--json".to_string(),
            "--skip-git-repo-check".to_string(),
            "--sandbox".to_string(),
            self.effective_sandbox(allowed_tools).as_str().to_string(),
        ];
        // opts wins over the spawner's construction-time defaults so a
        // per-iter resolver (Epic D) can override without rebuilding
        // the spawner. `as_ref` preserves the borrow shape so an unset
        // opts.model_override falls through to self.model_override.
        let model = opts.model_override.as_ref().or(self.model_override.as_ref());
        if let Some(m) = model {
            args.push("-m".to_string());
            args.push(m.clone());
        }
        let wd = opts.working_dir.as_ref().or(self.working_dir.as_ref());
        if let Some(p) = wd {
            args.push("-C".to_string());
            args.push(p.display().to_string());
        }
        args.extend(self.extra_args.iter().cloned());
        args.push("--".to_string());
        args.push(prompt.full_text.clone());
        args
    }
}

impl Default for CodexSpawner {
    fn default() -> Self {
        Self::from_env()
    }
}

impl RuntimeSpawner for CodexSpawner {
    fn spawn(
        &self,
        prompt: &AssembledPrompt,
        allowed_tools: &[String],
        opts: &SpawnOpts,
    ) -> Result<SpawnOutcome> {
        let args = self.build_args(prompt, allowed_tools, opts);
        let output = std::process::Command::new(&self.bin).args(&args).output()?;
        let stderr_tail = tail_text(&String::from_utf8_lossy(&output.stderr), 16);
        let exit_ok = output.status.success();
        match parse_codex_jsonl(&output.stdout) {
            Ok((raw_text, tokens, class)) => {
                let success = exit_ok && matches!(class, SpawnFailureClass::Success);
                let closed_task = detect_closed_task(&raw_text);
                Ok(SpawnOutcome {
                    success,
                    closed_task,
                    tokens,
                    stderr_tail,
                    raw_text,
                    failure_class: class,
                })
            }
            Err(_) if !exit_ok => Ok(SpawnOutcome {
                success: false,
                closed_task: None,
                tokens: TokenSpend::default(),
                stderr_tail,
                raw_text: String::from_utf8_lossy(&output.stdout).into_owned(),
                failure_class: SpawnFailureClass::RuntimeError(RuntimeErrorKind::Spawn),
            }),
            Err(e) => Err(e),
        }
    }
}

/// Parse `claude --output-format json`. The success shape is
/// `{ "result": "<text>", "usage": { "input_tokens": N, ... }, ... }`.
/// The error shape carries an `error` object — Anthropic's documented
/// envelope is `{ "type": "error", "error": { "type": "<kind>",
/// "status_code": <int>, "message": "<...>" } }`. `is_error: true` may
/// also appear at the top level.
///
/// Returns `(result_text, tokens, failure_class)`. When the response
/// is a success envelope, `failure_class` is
/// [`SpawnFailureClass::Success`]. When an error envelope is detected,
/// the kind is derived from `error.status_code` if present (via
/// [`classify_http_status`]), else from `error.type`. Missing fields
/// default to zero / empty so a partial response still produces a
/// usable triple.
pub fn parse_claude_json(bytes: &[u8]) -> Result<(String, TokenSpend, SpawnFailureClass)> {
    let v: serde_json::Value = serde_json::from_slice(bytes)?;
    let result_text = v.get("result").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let usage = v.get("usage").cloned().unwrap_or(serde_json::Value::Null);
    let pick = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let tokens = TokenSpend {
        input: pick("input_tokens"),
        output: pick("output_tokens"),
        cache_read: pick("cache_read_input_tokens"),
        cache_create: pick("cache_creation_input_tokens"),
    };

    let is_error = v.get("is_error").and_then(|x| x.as_bool()).unwrap_or(false)
        || v.get("type").and_then(|x| x.as_str()) == Some("error")
        || v.get("error").is_some();

    let failure_class = if is_error {
        let err = v.get("error");
        let status = err.and_then(|e| e.get("status_code")).and_then(|x| x.as_u64());
        let kind = match status {
            Some(s) => classify_http_status(s as u16),
            None => err
                .and_then(|e| e.get("type"))
                .and_then(|x| x.as_str())
                .map(classify_claude_error_type)
                .unwrap_or(RuntimeErrorKind::Unknown),
        };
        SpawnFailureClass::RuntimeError(kind)
    } else {
        SpawnFailureClass::Success
    };

    Ok((result_text, tokens, failure_class))
}

/// Map an Anthropic error `type` string to [`RuntimeErrorKind`]. Used
/// as a fallback when an error envelope lacks a numeric `status_code`.
/// Tags mirror Anthropic's documented API error vocabulary.
pub fn classify_claude_error_type(ty: &str) -> RuntimeErrorKind {
    match ty {
        "authentication_error" | "permission_error" => RuntimeErrorKind::Auth,
        "rate_limit_error" => RuntimeErrorKind::RateLimit,
        "invalid_request_error" | "not_found_error" => RuntimeErrorKind::BadRequest,
        "api_error" | "overloaded_error" => RuntimeErrorKind::Server,
        _ => RuntimeErrorKind::Unknown,
    }
}

/// Parse `codex exec --json` JSONL output.
///
/// The codex stream is line-delimited JSON; the terminus event decides
/// success vs failure (exit code is unreliable — codex exec exits 0 on
/// API 400). Per RESEARCH:codex-exec-json-stream:
///
/// - `turn.completed{usage:{input_tokens,cached_input_tokens,output_tokens}}`
///   → [`SpawnFailureClass::Success`].
/// - `turn.failed{error.message=<nested JSON string with status>}` →
///   classify via [`classify_http_status`] on the extracted `status`.
/// - `error` event with no following `turn.failed` →
///   `RuntimeError(Unknown)`.
/// - No terminus before EOF → `RuntimeError(Spawn)` (truncated stream).
///
/// The latest `item.completed{item.type=agent_message,text}` becomes the
/// returned text. Unknown event types are skipped (lenient). Lines that
/// fail to parse as JSON are skipped; if no event of any kind parses,
/// returns `Err` so callers can distinguish "not JSONL at all" from
/// "stream truncated mid-turn".
pub fn parse_codex_jsonl(bytes: &[u8]) -> Result<(String, TokenSpend, SpawnFailureClass)> {
    let text = std::str::from_utf8(bytes).unwrap_or("");

    let mut latest_text = String::new();
    let mut tokens = TokenSpend::default();
    let mut class: Option<SpawnFailureClass> = None;
    let mut saw_any_event = false;
    let mut saw_error_event = false;
    let mut first_parse_err: Option<serde_json::Error> = None;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v = match serde_json::from_str::<serde_json::Value>(line) {
            Ok(v) => v,
            Err(e) => {
                if first_parse_err.is_none() {
                    first_parse_err = Some(e);
                }
                continue;
            }
        };
        let Some(ty) = v.get("type").and_then(|x| x.as_str()) else {
            continue;
        };
        saw_any_event = true;
        match ty {
            "item.completed" => {
                if let Some(item) = v.get("item")
                    && item.get("type").and_then(|x| x.as_str()) == Some("agent_message")
                    && let Some(t) = item.get("text").and_then(|x| x.as_str())
                {
                    latest_text = t.to_string();
                }
            }
            "turn.completed" => {
                let usage = v.get("usage").cloned().unwrap_or(serde_json::Value::Null);
                let pick = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
                tokens = TokenSpend {
                    input: pick("input_tokens"),
                    output: pick("output_tokens"),
                    cache_read: pick("cached_input_tokens"),
                    cache_create: 0,
                };
                class = Some(SpawnFailureClass::Success);
            }
            "turn.failed" => {
                let status = v
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                    .and_then(|inner| inner.get("status").and_then(|x| x.as_u64()))
                    .map(|n| n as u16);
                let kind = match status {
                    Some(s) => classify_http_status(s),
                    None => RuntimeErrorKind::Unknown,
                };
                class = Some(SpawnFailureClass::RuntimeError(kind));
            }
            "error" => {
                saw_error_event = true;
            }
            _ => {} // lenient — skip unknown events
        }
    }

    if !saw_any_event {
        return Err(first_parse_err
            .unwrap_or_else(|| {
                serde_json::from_str::<serde_json::Value>("").expect_err("empty parses to err")
            })
            .into());
    }

    let final_class = class.unwrap_or(if saw_error_event {
        SpawnFailureClass::RuntimeError(RuntimeErrorKind::Unknown)
    } else {
        SpawnFailureClass::RuntimeError(RuntimeErrorKind::Spawn)
    });

    Ok((latest_text, tokens, final_class))
}

/// Scan `text` for `closed <id> —` markers (the literal line `hew task
/// close` emits). Returns the first id, prefering hew-style ids. Returns
/// `None` if no close marker is found.
pub fn detect_closed_task(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("closed ") {
            let id: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

/// Keep the last `lines` lines of `text`. The runner logs stderr_tail
/// into the iter record — bounded so a runaway stderr doesn't blow up
/// the log file.
fn tail_text(text: &str, lines: usize) -> String {
    let collected: Vec<&str> = text.lines().collect();
    let start = collected.len().saturating_sub(lines);
    collected[start..].join("\n")
}

/// In-memory spawner for tests. Configure with a canned outcome.
#[derive(Clone, Debug, Default)]
pub struct MockSpawner {
    pub outcome: SpawnOutcome,
    pub last_args: std::cell::RefCell<Option<(AssembledPrompt, Vec<String>)>>,
    pub last_opts: std::cell::RefCell<Option<SpawnOpts>>,
}

impl MockSpawner {
    pub fn new(outcome: SpawnOutcome) -> Self {
        Self {
            outcome,
            last_args: std::cell::RefCell::new(None),
            last_opts: std::cell::RefCell::new(None),
        }
    }
}

impl RuntimeSpawner for MockSpawner {
    fn spawn(
        &self,
        prompt: &AssembledPrompt,
        allowed_tools: &[String],
        opts: &SpawnOpts,
    ) -> Result<SpawnOutcome> {
        *self.last_args.borrow_mut() = Some((prompt.clone(), allowed_tools.to_vec()));
        *self.last_opts.borrow_mut() = Some(opts.clone());
        Ok(self.outcome.clone())
    }
}

impl Default for SpawnOutcome {
    fn default() -> Self {
        Self {
            success: true,
            closed_task: None,
            tokens: TokenSpend::default(),
            stderr_tail: String::new(),
            raw_text: String::new(),
            failure_class: SpawnFailureClass::Success,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::assemble;

    #[test]
    fn runtime_kind_parses_known_values() {
        assert_eq!("claude".parse::<RuntimeKind>().unwrap(), RuntimeKind::Claude);
        assert_eq!("codex".parse::<RuntimeKind>().unwrap(), RuntimeKind::Codex);
    }

    #[test]
    fn runtime_kind_rejects_unknown() {
        let err = "cursor".parse::<RuntimeKind>().unwrap_err();
        assert!(err.contains("cursor"));
        assert!(err.contains("claude"));
        assert!(err.contains("codex"));
    }

    #[test]
    fn runtime_kind_variants_matches_parser() {
        for v in RuntimeKind::VARIANTS {
            assert!(v.parse::<RuntimeKind>().is_ok(), "variant {v} must parse");
        }
    }

    #[test]
    fn runtime_kind_as_str_roundtrip() {
        for k in [RuntimeKind::Claude, RuntimeKind::Codex] {
            assert_eq!(k.as_str().parse::<RuntimeKind>().unwrap(), k);
        }
    }

    #[test]
    fn build_args_includes_print_json_and_tools() {
        let s = ClaudeSpawner { bin: "claude".into(), extra_args: vec![] };
        let p = assemble("S", "P", "T");
        let tools = vec!["Read".into(), "Bash(cargo:*)".into()];
        let args = s.build_args(&p, &tools, &SpawnOpts::default());
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], p.full_text);
        assert!(args.iter().any(|a| a == "--output-format"));
        assert!(args.iter().any(|a| a == "json"));
        assert!(args.iter().any(|a| a == "--allowedTools"));
        assert!(args.iter().any(|a| a == "Read,Bash(cargo:*)"));
    }

    #[test]
    fn build_args_omits_allowedtools_when_empty() {
        let s = ClaudeSpawner { bin: "claude".into(), extra_args: vec![] };
        let p = assemble("", "", "");
        let args = s.build_args(&p, &[], &SpawnOpts::default());
        assert!(!args.iter().any(|a| a == "--allowedTools"));
    }

    #[test]
    fn build_args_appends_extra_args() {
        let s = ClaudeSpawner {
            bin: "claude".into(),
            extra_args: vec!["--model".into(), "opus".into()],
        };
        let p = assemble("", "", "");
        let args = s.build_args(&p, &[], &SpawnOpts::default());
        assert_eq!(args.last().map(String::as_str), Some("opus"));
        let model_idx = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[model_idx + 1], "opus");
    }

    #[test]
    fn claude_spawner_threads_model_override_through_args() {
        let s = ClaudeSpawner { bin: "claude".into(), extra_args: vec![] };
        let p = assemble("", "", "");
        let opts = SpawnOpts { model_override: Some("opus-4.7".into()), ..SpawnOpts::default() };
        let args = s.build_args(&p, &[], &opts);
        let idx = args.iter().position(|a| a == "--model").expect("--model present");
        assert_eq!(args.get(idx + 1).map(String::as_str), Some("opus-4.7"));
    }

    #[test]
    fn claude_build_args_omits_model_when_opts_default() {
        let s = ClaudeSpawner { bin: "claude".into(), extra_args: vec![] };
        let p = assemble("", "", "");
        let args = s.build_args(&p, &[], &SpawnOpts::default());
        assert!(!args.iter().any(|a| a == "--model"), "args={args:?}");
    }

    #[test]
    fn parse_claude_json_extracts_result_and_usage() {
        let bytes = br#"{
            "type": "result",
            "result": "closed hew-abc - done",
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 500,
                "cache_read_input_tokens": 8000,
                "cache_creation_input_tokens": 200
            }
        }"#;
        let (text, tokens, class) = parse_claude_json(bytes).unwrap();
        assert!(text.contains("closed hew-abc"));
        assert_eq!(tokens.input, 1000);
        assert_eq!(tokens.output, 500);
        assert_eq!(tokens.cache_read, 8000);
        assert_eq!(tokens.cache_create, 200);
        assert_eq!(class, SpawnFailureClass::Success);
    }

    #[test]
    fn parse_claude_json_tolerates_missing_usage() {
        let bytes = br#"{"result":"hi"}"#;
        let (text, tokens, class) = parse_claude_json(bytes).unwrap();
        assert_eq!(text, "hi");
        assert_eq!(tokens.total(), 0);
        assert_eq!(class, SpawnFailureClass::Success);
    }

    // Fabricated from Anthropic's documented API error envelope spec
    // (see docs.anthropic.com/en/api/errors). Not live-captured — when
    // validating against a real CLI response, replace these strings.
    #[test]
    fn parse_claude_json_classifies_429_as_ratelimit() {
        let bytes = br#"{
            "type": "error",
            "error": {
                "type": "rate_limit_error",
                "status_code": 429,
                "message": "rate limit exceeded"
            }
        }"#;
        let (_text, _tokens, class) = parse_claude_json(bytes).unwrap();
        assert_eq!(class, SpawnFailureClass::RuntimeError(RuntimeErrorKind::RateLimit));
    }

    #[test]
    fn parse_claude_json_classifies_401_as_auth() {
        let bytes = br#"{
            "type": "error",
            "error": {
                "type": "authentication_error",
                "status_code": 401,
                "message": "invalid API key"
            }
        }"#;
        let (_text, _tokens, class) = parse_claude_json(bytes).unwrap();
        assert_eq!(class, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Auth));
    }

    #[test]
    fn parse_claude_json_classifies_500_as_server() {
        let bytes = br#"{
            "type": "error",
            "error": { "type": "api_error", "status_code": 500, "message": "boom" }
        }"#;
        let (_text, _tokens, class) = parse_claude_json(bytes).unwrap();
        assert_eq!(class, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Server));
    }

    #[test]
    fn parse_claude_json_falls_back_to_error_type_when_status_missing() {
        // No numeric status — classify_claude_error_type covers it.
        let bytes = br#"{"is_error": true, "error": {"type": "rate_limit_error"}}"#;
        let (_text, _tokens, class) = parse_claude_json(bytes).unwrap();
        assert_eq!(class, SpawnFailureClass::RuntimeError(RuntimeErrorKind::RateLimit));
    }

    #[test]
    fn classify_claude_error_type_table() {
        let cases = [
            ("authentication_error", RuntimeErrorKind::Auth),
            ("permission_error", RuntimeErrorKind::Auth),
            ("rate_limit_error", RuntimeErrorKind::RateLimit),
            ("invalid_request_error", RuntimeErrorKind::BadRequest),
            ("not_found_error", RuntimeErrorKind::BadRequest),
            ("api_error", RuntimeErrorKind::Server),
            ("overloaded_error", RuntimeErrorKind::Server),
            ("not_a_real_type", RuntimeErrorKind::Unknown),
        ];
        for (ty, want) in cases {
            assert_eq!(classify_claude_error_type(ty), want, "type {ty}");
        }
    }

    #[test]
    fn parse_claude_json_rejects_non_json() {
        let bytes = b"not json at all";
        assert!(parse_claude_json(bytes).is_err());
    }

    #[test]
    fn detect_closed_task_finds_hew_id() {
        let text = "doing some work\nclosed hew-3lg — done\nmore output";
        assert_eq!(detect_closed_task(text), Some("hew-3lg".to_string()));
    }

    #[test]
    fn detect_closed_task_ignores_non_close_lines() {
        let text = "the task is closed soon\ncloseddef without space";
        assert_eq!(detect_closed_task(text), None);
    }

    #[test]
    fn detect_closed_task_handles_leading_whitespace() {
        let text = "   closed hew-qia — ok";
        assert_eq!(detect_closed_task(text), Some("hew-qia".to_string()));
    }

    #[test]
    fn tail_text_caps_to_n_lines() {
        let s = "a\nb\nc\nd\ne\nf";
        assert_eq!(tail_text(s, 3), "d\ne\nf");
        assert_eq!(tail_text(s, 100), "a\nb\nc\nd\ne\nf");
    }

    #[test]
    fn mock_spawner_returns_configured_outcome_and_records_args() {
        let outcome = SpawnOutcome {
            success: true,
            closed_task: Some("hew-x".into()),
            tokens: TokenSpend { input: 5, output: 3, cache_read: 1, cache_create: 0 },
            stderr_tail: String::new(),
            raw_text: "closed hew-x".into(),
            failure_class: SpawnFailureClass::Success,
        };
        let m = MockSpawner::new(outcome.clone());
        let p = assemble("S", "P", "T");
        let tools = vec!["Read".to_string()];
        let opts = SpawnOpts { model_override: Some("opus".into()), ..SpawnOpts::default() };
        let result = m.spawn(&p, &tools, &opts).unwrap();
        assert_eq!(result, outcome);
        let last = m.last_args.borrow();
        let (recorded_prompt, recorded_tools) = last.as_ref().unwrap();
        assert_eq!(recorded_prompt.full_text, p.full_text);
        assert_eq!(recorded_tools, &tools);
        let recorded_opts = m.last_opts.borrow();
        assert_eq!(recorded_opts.as_ref().unwrap(), &opts);
    }

    #[test]
    fn from_env_honors_override() {
        let prev = std::env::var(CLAUDE_BIN_ENV).ok();
        // SAFETY: tests in this binary serialize through cargo test's
        // single-threaded default for env-touching tests. The other
        // tests in this module don't read CLAUDE_BIN_ENV.
        unsafe {
            std::env::set_var(CLAUDE_BIN_ENV, "/usr/local/bin/claude-test");
        }
        let s = ClaudeSpawner::from_env();
        assert_eq!(s.bin, PathBuf::from("/usr/local/bin/claude-test"));
        unsafe {
            match prev {
                Some(v) => std::env::set_var(CLAUDE_BIN_ENV, v),
                None => std::env::remove_var(CLAUDE_BIN_ENV),
            }
        }
    }

    /// E2E: only runs when HEW_LOOP_E2E=1 and `claude` is on PATH.
    /// Stays off CI by default; manual invocation only. Verifies field
    /// names (tokens > 0), result text containing the close marker
    /// (closed_task.is_some()), and exit-0 success path.
    #[test]
    fn e2e_real_claude_spawn() {
        if std::env::var("HEW_LOOP_E2E").as_deref() != Ok("1") {
            return;
        }
        let s = ClaudeSpawner::from_env();
        let p = assemble(
            "You are a test agent.",
            "",
            "Reply with exactly this single line and nothing else: closed hew-e2e — done",
        );
        let out = s.spawn(&p, &[], &SpawnOpts::default()).expect("spawn ok");
        assert!(out.success, "expected success, stderr={}", out.stderr_tail);
        assert!(out.tokens.total() > 0, "expected nonzero tokens, raw={}", out.raw_text);
        assert_eq!(
            out.closed_task.as_deref(),
            Some("hew-e2e"),
            "expected closed_task to be detected from result text, raw={}",
            out.raw_text
        );
    }

    /// E2E: only runs when HEW_LOOP_E2E=1 and `codex` is on PATH.
    /// Mirrors `e2e_real_claude_spawn` — manual invocation only, off
    /// the default CI matrix. Verifies the codex JSONL stream parser
    /// extracts non-empty text and Success class against a live codex
    /// install. Sandbox pinned to read-only since the test does no
    /// filesystem writes.
    #[test]
    fn e2e_real_codex_spawn() {
        if std::env::var("HEW_LOOP_E2E").as_deref() != Ok("1") {
            return;
        }
        if which::which("codex").is_err() {
            return;
        }
        let s = CodexSpawner::from_env();
        let p = assemble(
            "You are a test agent.",
            "",
            "Reply with exactly this single line and nothing else: pong",
        );
        let out = s.spawn(&p, &[], &SpawnOpts::default()).expect("spawn ok");
        assert_eq!(out.failure_class, SpawnFailureClass::Success, "stderr={}", out.stderr_tail);
        assert!(!out.raw_text.is_empty(), "expected non-empty reply, stderr={}", out.stderr_tail);
        assert!(out.tokens.total() > 0, "expected nonzero tokens, raw={}", out.raw_text);
    }

    #[test]
    fn classify_http_status_table() {
        let cases = [
            (200, RuntimeErrorKind::Unknown),
            (301, RuntimeErrorKind::Unknown),
            (400, RuntimeErrorKind::BadRequest),
            (401, RuntimeErrorKind::Auth),
            (403, RuntimeErrorKind::Auth),
            (404, RuntimeErrorKind::BadRequest),
            (422, RuntimeErrorKind::BadRequest),
            (429, RuntimeErrorKind::RateLimit),
            (500, RuntimeErrorKind::Server),
            (502, RuntimeErrorKind::Server),
            (503, RuntimeErrorKind::Server),
            (599, RuntimeErrorKind::Server),
            (0, RuntimeErrorKind::Unknown),
            (999, RuntimeErrorKind::Unknown),
        ];
        for (status, want) in cases {
            assert_eq!(classify_http_status(status), want, "status {status}");
        }
    }

    #[test]
    fn spawn_failure_class_defaults_success() {
        assert_eq!(SpawnFailureClass::default(), SpawnFailureClass::Success);
    }

    #[test]
    fn spawn_outcome_default_has_success_failure_class() {
        assert_eq!(SpawnOutcome::default().failure_class, SpawnFailureClass::Success);
    }

    /// Regression test against a captured real `claude -p` response
    /// (see `hew-core/tests/fixtures/claude-output.json`). If the live
    /// JSON shape changes — field renames in `usage`, or `result` no
    /// longer carrying the agent's text — this test will catch it.
    #[test]
    fn parse_claude_json_matches_captured_fixture() {
        let bytes = include_bytes!("../tests/fixtures/claude-output.json");
        let (text, tokens, class) = parse_claude_json(bytes).expect("fixture parses");
        assert_eq!(text, "closed hew-e2i — done");
        assert_eq!(tokens.input, 6);
        assert_eq!(tokens.output, 14);
        assert_eq!(tokens.cache_read, 22827);
        assert_eq!(tokens.cache_create, 23362);
        assert_eq!(class, SpawnFailureClass::Success);
        assert_eq!(detect_closed_task(&text).as_deref(), Some("hew-e2i"));
    }

    #[test]
    fn parse_codex_jsonl_extracts_usage_on_success() {
        let bytes = include_bytes!("../tests/fixtures/codex-exec-success.jsonl");
        let (text, tokens, class) = parse_codex_jsonl(bytes).expect("fixture parses");
        assert_eq!(text, "pong");
        assert_eq!(tokens.input, 11549);
        assert_eq!(tokens.cache_read, 3456);
        assert_eq!(tokens.output, 20);
        assert_eq!(tokens.cache_create, 0);
        assert_eq!(class, SpawnFailureClass::Success);
    }

    #[test]
    fn parse_codex_jsonl_classifies_turn_failed_400_as_badrequest() {
        let bytes = include_bytes!("../tests/fixtures/codex-exec-turn-failed-400.jsonl");
        let (_text, tokens, class) = parse_codex_jsonl(bytes).expect("fixture parses");
        assert_eq!(tokens.total(), 0);
        assert_eq!(class, SpawnFailureClass::RuntimeError(RuntimeErrorKind::BadRequest));
    }

    #[test]
    fn parse_codex_jsonl_classifies_truncated_stream_as_spawn_error() {
        let bytes = include_bytes!("../tests/fixtures/codex-exec-truncated.jsonl");
        let (text, _tokens, class) = parse_codex_jsonl(bytes).expect("fixture parses");
        assert_eq!(text, "partial reply, stream cut off");
        assert_eq!(class, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Spawn));
    }

    #[test]
    fn parse_codex_jsonl_rejects_non_jsonl() {
        assert!(parse_codex_jsonl(b"not json at all\nstill not json").is_err());
    }

    #[test]
    fn parse_codex_jsonl_handles_missing_trailing_newline() {
        let bytes = b"{\"type\":\"turn.started\"}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,\"cached_input_tokens\":2,\"output_tokens\":3}}";
        let (_text, tokens, class) = parse_codex_jsonl(bytes).expect("parses without trailing nl");
        assert_eq!(tokens.input, 1);
        assert_eq!(tokens.cache_read, 2);
        assert_eq!(tokens.output, 3);
        assert_eq!(class, SpawnFailureClass::Success);
    }

    #[test]
    fn parse_codex_jsonl_skips_malformed_lines_when_other_events_parse() {
        let bytes =
            b"garbage line\n{\"type\":\"turn.completed\",\"usage\":{\"output_tokens\":7}}\n";
        let (_text, tokens, class) = parse_codex_jsonl(bytes).expect("lenient skip");
        assert_eq!(tokens.output, 7);
        assert_eq!(class, SpawnFailureClass::Success);
    }

    #[test]
    fn parse_codex_jsonl_error_without_turn_failed_is_unknown_runtime_error() {
        let bytes = b"{\"type\":\"thread.started\"}\n{\"type\":\"error\",\"message\":\"boom\"}\n";
        let (_text, _tokens, class) = parse_codex_jsonl(bytes).expect("parses");
        assert_eq!(class, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Unknown));
    }

    fn arg_pair<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
    }

    #[test]
    fn codex_build_args_emits_exec_json_sandbox_model() {
        let s = CodexSpawner::from_env().with_model("gpt-5.4");
        let p = assemble("S", "P", "T");
        let tools = vec!["Edit".to_string()];
        let args = s.build_args(&p, &tools, &SpawnOpts::default());
        assert_eq!(args[0], "exec");
        assert!(args.iter().any(|a| a == "--json"));
        assert!(args.iter().any(|a| a == "--skip-git-repo-check"));
        assert_eq!(arg_pair(&args, "--sandbox"), Some("workspace-write"));
        assert_eq!(arg_pair(&args, "-m"), Some("gpt-5.4"));
        // Prompt comes after `--`.
        let dash = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[dash + 1], p.full_text);
    }

    #[test]
    fn codex_build_args_omits_model_when_none() {
        let s = CodexSpawner::from_env();
        let p = assemble("", "", "");
        let args = s.build_args(&p, &[], &SpawnOpts::default());
        assert!(!args.iter().any(|a| a == "-m"), "args={args:?}");
    }

    #[test]
    fn codex_build_args_omits_cd_when_no_working_dir() {
        let s = CodexSpawner::from_env();
        let p = assemble("", "", "");
        let args = s.build_args(&p, &[], &SpawnOpts::default());
        assert!(!args.iter().any(|a| a == "-C"), "args={args:?}");
    }

    #[test]
    fn codex_build_args_emits_cd_when_working_dir_set() {
        let s = CodexSpawner::from_env().with_working_dir("/tmp/wt");
        let p = assemble("", "", "");
        let args = s.build_args(&p, &[], &SpawnOpts::default());
        assert_eq!(arg_pair(&args, "-C"), Some("/tmp/wt"));
    }

    #[test]
    fn codex_build_args_sandbox_defaults_to_tool_mapping() {
        let s = CodexSpawner::from_env();
        // No write tools → read-only.
        let args =
            s.build_args(&assemble("", "", ""), &["Read".to_string()], &SpawnOpts::default());
        assert_eq!(arg_pair(&args, "--sandbox"), Some("read-only"));
        // Write tool present → workspace-write via mapper.
        let args =
            s.build_args(&assemble("", "", ""), &["Write".to_string()], &SpawnOpts::default());
        assert_eq!(arg_pair(&args, "--sandbox"), Some("workspace-write"));
    }

    #[test]
    fn codex_build_args_explicit_with_sandbox_overrides_mapper() {
        // No write tools — mapper would say read-only — but an
        // explicit DangerFullAccess wins.
        let s = CodexSpawner::from_env().with_sandbox(SandboxPolicy::DangerFullAccess);
        let args = s.build_args(&assemble("", "", ""), &[], &SpawnOpts::default());
        assert_eq!(arg_pair(&args, "--sandbox"), Some("danger-full-access"));
    }

    #[test]
    fn codex_build_args_extra_args_appear_before_dash_dash() {
        let mut s = CodexSpawner::from_env();
        s.extra_args = vec!["-c".into(), "model_reasoning_effort=high".into()];
        let p = assemble("", "", "");
        let args = s.build_args(&p, &[], &SpawnOpts::default());
        let extra_idx = args.iter().position(|a| a == "-c").unwrap();
        let dash = args.iter().position(|a| a == "--").unwrap();
        assert!(extra_idx < dash, "extra_args must precede `--`: args={args:?}");
    }

    #[test]
    fn codex_from_env_honors_override() {
        let prev = std::env::var(CODEX_BIN_ENV).ok();
        // SAFETY: same isolation argument as the claude env test above.
        unsafe {
            std::env::set_var(CODEX_BIN_ENV, "/usr/local/bin/codex-test");
        }
        let s = CodexSpawner::from_env();
        assert_eq!(s.bin, PathBuf::from("/usr/local/bin/codex-test"));
        unsafe {
            match prev {
                Some(v) => std::env::set_var(CODEX_BIN_ENV, v),
                None => std::env::remove_var(CODEX_BIN_ENV),
            }
        }
    }

    #[test]
    fn codex_spawner_threads_model_override_through_args() {
        let s = CodexSpawner::from_env();
        let p = assemble("", "", "");
        let opts = SpawnOpts { model_override: Some("gpt-5.4".into()), ..SpawnOpts::default() };
        let args = s.build_args(&p, &[], &opts);
        assert_eq!(arg_pair(&args, "-m"), Some("gpt-5.4"));
    }

    #[test]
    fn codex_spawner_opts_model_override_wins_over_struct_default() {
        let s = CodexSpawner::from_env().with_model("from-struct");
        let p = assemble("", "", "");
        let opts = SpawnOpts { model_override: Some("from-opts".into()), ..SpawnOpts::default() };
        let args = s.build_args(&p, &[], &opts);
        assert_eq!(arg_pair(&args, "-m"), Some("from-opts"));
    }

    #[test]
    fn codex_spawner_opts_working_dir_wins_over_struct_default() {
        let s = CodexSpawner::from_env().with_working_dir("/from/struct");
        let p = assemble("", "", "");
        let opts =
            SpawnOpts { working_dir: Some(PathBuf::from("/from/opts")), ..SpawnOpts::default() };
        let args = s.build_args(&p, &[], &opts);
        assert_eq!(arg_pair(&args, "-C"), Some("/from/opts"));
    }

    #[test]
    fn spawn_opts_default_is_no_op() {
        let opts = SpawnOpts::default();
        assert!(opts.model_override.is_none());
        assert!(opts.working_dir.is_none());
    }

    #[test]
    fn fallback_config_default_no_runtime_default_cooldown() {
        let fc = FallbackConfig::default();
        assert!(fc.runtime.is_none());
        assert_eq!(fc.cooldown_iters, crate::config::FALLBACK_COOLDOWN_ITERS_DEFAULT);
    }

    #[test]
    fn fallback_config_resolve_cli_overrides_config() {
        let fc =
            FallbackConfig::resolve(Some(RuntimeKind::Codex), Some(5), Some("claude"), Some(11))
                .unwrap();
        assert_eq!(fc.runtime, Some(RuntimeKind::Codex));
        assert_eq!(fc.cooldown_iters, 5);
    }

    #[test]
    fn fallback_config_resolve_config_when_cli_missing() {
        let fc = FallbackConfig::resolve(None, None, Some("codex"), Some(4)).unwrap();
        assert_eq!(fc.runtime, Some(RuntimeKind::Codex));
        assert_eq!(fc.cooldown_iters, 4);
    }

    #[test]
    fn fallback_config_resolve_none_when_neither_set() {
        let fc = FallbackConfig::resolve(None, None, None, None).unwrap();
        assert!(fc.runtime.is_none());
        assert_eq!(fc.cooldown_iters, crate::config::FALLBACK_COOLDOWN_ITERS_DEFAULT);
    }

    #[test]
    fn fallback_config_resolve_empty_config_string_is_none() {
        // Defensive: a config layer that hands back Some("") shouldn't
        // parse — treat it the same as no fallback.
        let fc = FallbackConfig::resolve(None, None, Some(""), None).unwrap();
        assert!(fc.runtime.is_none());
    }

    #[test]
    fn fallback_config_resolve_rejects_bogus_config_runtime() {
        let err = FallbackConfig::resolve(None, None, Some("cursor"), None).unwrap_err();
        assert!(err.contains("cursor"));
    }

    #[test]
    fn fallback_config_resolve_zero_cooldown_clamps_to_one() {
        let fc = FallbackConfig::resolve(Some(RuntimeKind::Codex), Some(0), None, None).unwrap();
        assert_eq!(fc.cooldown_iters, 1);
    }

    #[test]
    fn sandbox_policy_display_matches_codex_cli() {
        assert_eq!(SandboxPolicy::ReadOnly.to_string(), "read-only");
        assert_eq!(SandboxPolicy::WorkspaceWrite.to_string(), "workspace-write");
        assert_eq!(SandboxPolicy::DangerFullAccess.to_string(), "danger-full-access");
    }

    #[test]
    fn map_allowed_tools_to_sandbox_table() {
        let cases: &[(&[&str], SandboxPolicy)] = &[
            (&[], SandboxPolicy::ReadOnly),
            (&["Read"], SandboxPolicy::ReadOnly),
            (&["Read", "Edit"], SandboxPolicy::WorkspaceWrite),
            // Bash subcommand restriction is silently broadened to read-only:
            // the mapper has no way to express "git only" in codex's sandbox enum.
            (&["Bash(git:*)"], SandboxPolicy::ReadOnly),
            (&["NotebookEdit"], SandboxPolicy::WorkspaceWrite),
            (&["Write"], SandboxPolicy::WorkspaceWrite),
            (&["MultiEdit"], SandboxPolicy::WorkspaceWrite),
            // Case-sensitive: lowercase doesn't trigger write-class.
            (&["edit"], SandboxPolicy::ReadOnly),
        ];
        for (tools, want) in cases {
            let owned: Vec<String> = tools.iter().map(|s| s.to_string()).collect();
            assert_eq!(map_allowed_tools_to_sandbox(&owned), *want, "tools={tools:?}");
        }
    }

    #[test]
    fn parse_codex_jsonl_no_panic_on_empty_input() {
        assert!(parse_codex_jsonl(b"").is_err());
        assert!(parse_codex_jsonl(b"\n\n\n").is_err());
    }
}
