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

/// Inject-at-runtime abstraction. Production wires [`ClaudeSpawner`];
/// tests use [`MockSpawner`].
pub trait RuntimeSpawner {
    fn spawn(&self, prompt: &AssembledPrompt, allowed_tools: &[String]) -> Result<SpawnOutcome>;
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
    pub fn build_args(&self, prompt: &AssembledPrompt, allowed_tools: &[String]) -> Vec<String> {
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
    fn spawn(&self, prompt: &AssembledPrompt, allowed_tools: &[String]) -> Result<SpawnOutcome> {
        let args = self.build_args(prompt, allowed_tools);
        let output = std::process::Command::new(&self.bin).args(&args).output()?;
        let stderr_tail = tail_text(&String::from_utf8_lossy(&output.stderr), 16);
        let exit_ok = output.status.success();
        match parse_claude_json(&output.stdout) {
            Ok((raw_text, tokens)) => {
                let closed_task = detect_closed_task(&raw_text);
                Ok(SpawnOutcome {
                    success: exit_ok,
                    closed_task,
                    tokens,
                    stderr_tail,
                    raw_text,
                    failure_class: SpawnFailureClass::Success,
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

/// Parse `claude --output-format json`. The JSON shape is
/// `{ "result": "<text>", "usage": { "input_tokens": N, ... }, ... }`.
/// Missing fields default to zero / empty so a partial response still
/// produces a `SpawnOutcome` instead of erroring out.
pub fn parse_claude_json(bytes: &[u8]) -> Result<(String, TokenSpend)> {
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
    Ok((result_text, tokens))
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
}

impl MockSpawner {
    pub fn new(outcome: SpawnOutcome) -> Self {
        Self { outcome, last_args: std::cell::RefCell::new(None) }
    }
}

impl RuntimeSpawner for MockSpawner {
    fn spawn(&self, prompt: &AssembledPrompt, allowed_tools: &[String]) -> Result<SpawnOutcome> {
        *self.last_args.borrow_mut() = Some((prompt.clone(), allowed_tools.to_vec()));
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
        let args = s.build_args(&p, &tools);
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
        let args = s.build_args(&p, &[]);
        assert!(!args.iter().any(|a| a == "--allowedTools"));
    }

    #[test]
    fn build_args_appends_extra_args() {
        let s = ClaudeSpawner {
            bin: "claude".into(),
            extra_args: vec!["--model".into(), "opus".into()],
        };
        let p = assemble("", "", "");
        let args = s.build_args(&p, &[]);
        assert_eq!(args.last().map(String::as_str), Some("opus"));
        let model_idx = args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(args[model_idx + 1], "opus");
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
        let (text, tokens) = parse_claude_json(bytes).unwrap();
        assert!(text.contains("closed hew-abc"));
        assert_eq!(tokens.input, 1000);
        assert_eq!(tokens.output, 500);
        assert_eq!(tokens.cache_read, 8000);
        assert_eq!(tokens.cache_create, 200);
    }

    #[test]
    fn parse_claude_json_tolerates_missing_usage() {
        let bytes = br#"{"result":"hi"}"#;
        let (text, tokens) = parse_claude_json(bytes).unwrap();
        assert_eq!(text, "hi");
        assert_eq!(tokens.total(), 0);
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
        let result = m.spawn(&p, &tools).unwrap();
        assert_eq!(result, outcome);
        let last = m.last_args.borrow();
        let (recorded_prompt, recorded_tools) = last.as_ref().unwrap();
        assert_eq!(recorded_prompt.full_text, p.full_text);
        assert_eq!(recorded_tools, &tools);
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
        let out = s.spawn(&p, &[]).expect("spawn ok");
        assert!(out.success, "expected success, stderr={}", out.stderr_tail);
        assert!(out.tokens.total() > 0, "expected nonzero tokens, raw={}", out.raw_text);
        assert_eq!(
            out.closed_task.as_deref(),
            Some("hew-e2e"),
            "expected closed_task to be detected from result text, raw={}",
            out.raw_text
        );
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
        let (text, tokens) = parse_claude_json(bytes).expect("fixture parses");
        assert_eq!(text, "closed hew-e2i — done");
        assert_eq!(tokens.input, 6);
        assert_eq!(tokens.output, 14);
        assert_eq!(tokens.cache_read, 22827);
        assert_eq!(tokens.cache_create, 23362);
        assert_eq!(detect_closed_task(&text).as_deref(), Some("hew-e2i"));
    }
}
