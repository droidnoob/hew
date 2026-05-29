use std::ffi::OsStr;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::config;
use hew_core::error::HewError;
use hew_core::git::{GitClient, RealGit};
use hew_core::install::{self, Runtime};
use hew_core::os::{self, OsKind};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Issue prefix passed through to `bd init` (default: directory name).
    #[arg(long)]
    pub prefix: Option<String>,

    /// Track .beads/ in git (default: false — added to .gitignore).
    #[arg(long, conflicts_with = "stealth")]
    pub git_track: bool,

    /// Stealth: force `.beads/` invisible (no git tracking, no `bd init`
    /// auto-commit, excluded via `.git/info/exclude`). Skips the interactive
    /// "share the task graph in git?" prompt. Equivalent to the default
    /// non-tracked path, but explicit and non-interactive.
    #[arg(long, conflicts_with = "git_track")]
    pub stealth: bool,

    /// Agent runtime(s) to install for. Defaults to auto-detect. Accepts
    /// comma-separated values (`--runtime=claude,codex`) or repeated flags
    /// (`--runtime=claude --runtime=codex`); both forms append. Empty when
    /// the flag is omitted, which preserves the detect-or-prompt fallback.
    #[arg(long, value_enum, value_delimiter = ',', action = clap::ArgAction::Append)]
    pub runtime: Vec<RuntimeArg>,

    /// Installation scope.
    #[arg(long, value_enum, default_value_t = Scope::Local)]
    pub scope: Scope,

    /// Project state: existing brownfield codebase or fresh start. Defaults
    /// to auto-detect (existing if any source-like files are present).
    #[arg(long, value_enum)]
    pub project_type: Option<ProjectTypeArg>,

    /// Auto-branching strategy. Defaults to `epic` (one branch per epic).
    #[arg(long, value_enum)]
    pub branching: Option<BranchingArg>,

    /// Deps skill mode for the plan picker. Default `ask`.
    #[arg(long, value_enum)]
    pub deps: Option<SkillModeArg>,

    /// Research skill mode for the plan picker. Default `ask`.
    #[arg(long, value_enum)]
    pub research: Option<SkillModeArg>,

    /// Security skill mode for the plan picker. Default `ask`.
    #[arg(long, value_enum)]
    pub security: Option<SkillModeArg>,

    /// Require tests before close (hew-guard fails close on missing test).
    #[arg(long, action = clap::ArgAction::SetTrue, overrides_with = "no_require_tests")]
    pub require_tests: bool,

    /// Explicit opt-out of require-tests (the default).
    #[arg(long = "no-require-tests", action = clap::ArgAction::SetTrue, overrides_with = "require_tests")]
    pub no_require_tests: bool,

    /// Default for the hew-plan research-or-decompose picker.
    #[arg(long, value_enum)]
    pub research_default: Option<ResearchDefaultArg>,

    /// Trigger review after this many closed tasks since last review. 0 = off.
    #[arg(long)]
    pub review_after_n: Option<u32>,

    /// Trigger review when an epic closes.
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub review_after_epic: bool,

    /// Accept all defaults non-interactively.
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, clap::ValueEnum)]
pub enum ResearchDefaultArg {
    Ask,
    AutoSkip,
    AutoRun,
}

impl ResearchDefaultArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::AutoSkip => "auto-skip",
            Self::AutoRun => "auto-run",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, clap::ValueEnum)]
pub enum SkillModeArg {
    Yes,
    No,
    Ask,
}

impl SkillModeArg {
    fn into_core(self) -> hew_core::config::SkillMode {
        use hew_core::config::SkillMode;
        match self {
            Self::Yes => SkillMode::Yes,
            Self::No => SkillMode::No,
            Self::Ask => SkillMode::Ask,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, clap::ValueEnum)]
pub enum BranchingArg {
    Epic,
    None,
    Always,
}

impl BranchingArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::Epic => "epic",
            Self::None => "none",
            Self::Always => "always",
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, clap::ValueEnum)]
pub enum ProjectTypeArg {
    New,
    Existing,
}

impl ProjectTypeArg {
    fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Existing => "existing",
        }
    }
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum RuntimeArg {
    Claude,
    Cursor,
    Codex,
    Windsurf,
    Generic,
}

impl From<RuntimeArg> for Runtime {
    fn from(a: RuntimeArg) -> Self {
        match a {
            RuntimeArg::Claude => Self::Claude,
            RuntimeArg::Cursor => Self::Cursor,
            RuntimeArg::Codex => Self::Codex,
            RuntimeArg::Windsurf => Self::Windsurf,
            RuntimeArg::Generic => Self::Generic,
        }
    }
}

#[derive(Debug, Copy, Clone, clap::ValueEnum, PartialEq, Eq)]
pub enum Scope {
    Local,
    Global,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let project_root = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;

    crate::ui::banner::render(ctx);

    let detected = install::detect_runtimes(&project_root);
    let runtimes = resolve_runtimes(&args, detected, ctx.interactive, interactive_runtime_pick)?;
    let install_root = resolve_install_root(args.scope, &project_root)?;

    let bd = ensure_bd(ctx)?;

    // Git must be initialised BEFORE bd init: in non-stealth mode bd
    // auto-commits beads files; in stealth mode bd writes to
    // .git/info/exclude. Either way, .git/ has to exist.
    ensure_git(ctx);
    init_git_repo(ctx, &project_root)?;

    let git_track = resolve_git_track(ctx, &args, &project_root);
    if !git_track {
        let _ = install::ensure_beads_gitignored(&project_root)
            .map_err(|e| miette::miette!("update .gitignore: {e}"))?;
    }

    let beads_pre_existed = project_root.join(".beads").exists();
    run_bd_init(&bd, &project_root, args.prefix.as_deref(), git_track)?;
    if !beads_pre_existed && !ctx.quiet {
        println!("beads: ✓ task graph initialised in .beads/");
    }

    let project_type = resolve_project_type(ctx, &args, &project_root);
    let branching = resolve_branching(ctx, &args);
    let skills = resolve_optional_skills(ctx, &args);
    let require_tests = resolve_require_tests(ctx, &args);
    let advanced = resolve_advanced(ctx, &args);

    persist_config(ctx, |cfg| {
        cfg.git_track = git_track;
        cfg.branching.strategy = branching.as_str().to_string();
        cfg.optional_skills.deps = skills.0.into_core();
        cfg.optional_skills.research = skills.1.into_core();
        cfg.optional_skills.security = skills.2.into_core();
        cfg.testing.require = require_tests;
        cfg.research.default = advanced.research_default.as_str().to_string();
        cfg.review.after_n_tasks = advanced.review_after_n;
        cfg.review.after_epic = advanced.review_after_epic;
    });

    let mut plans = Vec::with_capacity(runtimes.len());
    for rt in &runtimes {
        let plan = install::install(*rt, &install_root)?;
        if ctx.quiet {
            // Scripts get the one-liner per runtime; panel is for humans only.
            println!(
                "hew installed for {} ({:?} scope) → {} files under {}",
                plan.runtime.as_str(),
                args.scope,
                plan.written.len(),
                plan.root.display()
            );
        } else {
            print_runtime_block(&plan);
        }
        plans.push(plan);
    }

    if !ctx.quiet {
        print_summary_panel(
            &plans,
            args.scope,
            git_track,
            project_type,
            branching,
            &skills,
            require_tests,
            &advanced,
        );
    }
    Ok(())
}

/// One-line banner per runtime install: `<Name>: ✓ N files into <artifact>`.
/// Called once per element of the runtimes vec; the aggregate panel below
/// shows the cross-runtime totals.
fn print_runtime_block(plan: &install::InstallPlan) {
    let artifact = runtime_artifact_label(plan.runtime);
    let title = runtime_display_name(plan.runtime);
    if artifact.is_empty() {
        println!("─ {}: ✓ {} files written", title, plan.written.len());
    } else {
        println!("─ {}: ✓ {} files written into {}", title, plan.written.len(), artifact);
    }
}

fn runtime_display_name(rt: Runtime) -> &'static str {
    match rt {
        Runtime::Claude => "Claude",
        Runtime::Cursor => "Cursor",
        Runtime::Codex => "Codex",
        Runtime::Windsurf => "Windsurf",
        Runtime::Generic => "Generic",
    }
}

fn runtime_artifact_label(rt: Runtime) -> &'static str {
    match rt {
        Runtime::Claude => ".claude/",
        Runtime::Cursor => ".cursorrules",
        Runtime::Codex => ".codex/ + AGENTS.md",
        Runtime::Windsurf => ".windsurfrules",
        Runtime::Generic => "",
    }
}

#[allow(clippy::too_many_arguments)] // IV.13 refactor will collapse this into a FlowChoices struct.
fn print_summary_panel(
    plans: &[install::InstallPlan],
    scope: Scope,
    git_track: bool,
    project_type: ProjectTypeArg,
    branching: BranchingArg,
    skills: &(SkillModeArg, SkillModeArg, SkillModeArg),
    require_tests: bool,
    advanced: &AdvancedKnobs,
) {
    let bar = "──────────────────────────────";
    let runtimes_str = plans.iter().map(|p| p.runtime.as_str()).collect::<Vec<_>>().join(", ");
    let total_files: usize = plans.iter().map(|p| p.written.len()).sum();
    let root_display = plans.first().map(|p| p.root.display().to_string()).unwrap_or_default();
    println!();
    println!("Setup complete");
    println!("{bar}");
    println!("  runtime           {runtimes_str}");
    println!(
        "  scope             {}",
        match scope {
            Scope::Local => "local",
            Scope::Global => "global",
        }
    );
    println!("  install root      {root_display}");
    println!("  files written     {total_files}");
    println!(
        "  git track         {}",
        if git_track { "yes (.beads/ tracked)" } else { "no (.beads/ ignored)" }
    );
    println!("  project state     {}", project_type.as_str());
    println!("  branching         {}", branching.as_str());
    println!(
        "  optional skills   deps={}, research={}, security={}",
        skills.0.into_core(),
        skills.1.into_core(),
        skills.2.into_core(),
    );
    println!("  require tests     {}", if require_tests { "yes" } else { "no" });
    println!("  research default  {}", advanced.research_default.as_str());
    let review = match (advanced.review_after_n, advanced.review_after_epic) {
        (0, false) => "off".to_string(),
        (0, true) => "on-epic".to_string(),
        (n, false) => format!("every-{n}"),
        (n, true) => format!("on-epic + every-{n}"),
    };
    println!("  review cadence    {review}");
    println!("{bar}");
    match project_type {
        ProjectTypeArg::New => println!("Next: /hew:new-project to bootstrap"),
        ProjectTypeArg::Existing => println!("Next: /hew:scan to map this codebase"),
    }
}

/// Auto-detect whether `project_root` looks like an existing codebase or a
/// fresh directory. Anything that isn't a known scaffolding/meta file counts
/// as a source file.
fn detect_project_type(project_root: &std::path::Path) -> ProjectTypeArg {
    const IGNORE: &[&str] =
        &["README.md", "README", "LICENSE", "LICENSE.md", ".gitignore", ".git", ".beads"];
    let Ok(entries) = std::fs::read_dir(project_root) else {
        return ProjectTypeArg::New;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        if name_str.starts_with('.') || IGNORE.contains(&name_str) {
            continue;
        }
        // Skip runtime-marker dirs we created during this very run.
        if matches!(name_str, "CLAUDE.md" | "AGENTS.md" | "WINDSURF.md" | ".cursor") {
            continue;
        }
        return ProjectTypeArg::Existing;
    }
    ProjectTypeArg::New
}

#[derive(Debug, Clone, Copy)]
struct AdvancedKnobs {
    research_default: ResearchDefaultArg,
    review_after_n: u32,
    review_after_epic: bool,
}

fn resolve_advanced(ctx: &Ctx, args: &Args) -> AdvancedKnobs {
    let mut out = AdvancedKnobs {
        research_default: args.research_default.unwrap_or(ResearchDefaultArg::Ask),
        review_after_n: args.review_after_n.unwrap_or(0),
        review_after_epic: args.review_after_epic,
    };
    // Any explicit flag short-circuits the gate.
    let any_explicit =
        args.research_default.is_some() || args.review_after_n.is_some() || args.review_after_epic;
    if any_explicit || !ctx.interactive {
        return out;
    }
    use inquire::{Confirm, Select};
    let configure_more = Confirm::new("Configure more?")
        .with_default(false)
        .with_help_message("research default + review cadence")
        .prompt()
        .unwrap_or(false);
    if !configure_more {
        return out;
    }
    if let Ok(pick) =
        Select::new("Research default at plan picker?", vec!["ask", "auto-skip", "auto-run"])
            .with_starting_cursor(0)
            .prompt()
    {
        out.research_default = match pick {
            "auto-skip" => ResearchDefaultArg::AutoSkip,
            "auto-run" => ResearchDefaultArg::AutoRun,
            _ => ResearchDefaultArg::Ask,
        };
    }
    if let Ok(pick) =
        Select::new("Review cadence?", vec!["off", "on-epic", "every-3", "every-5", "every-10"])
            .with_starting_cursor(0)
            .prompt()
    {
        match pick {
            "on-epic" => {
                out.review_after_epic = true;
                out.review_after_n = 0;
            }
            "every-3" => out.review_after_n = 3,
            "every-5" => out.review_after_n = 5,
            "every-10" => out.review_after_n = 10,
            _ => {
                out.review_after_n = 0;
                out.review_after_epic = false;
            }
        }
    }
    out
}

fn resolve_require_tests(ctx: &Ctx, args: &Args) -> bool {
    if args.require_tests {
        return true;
    }
    if args.no_require_tests {
        return false;
    }
    if !ctx.interactive {
        return false;
    }
    use inquire::Confirm;
    Confirm::new("Require tests before close?")
        .with_default(false)
        .with_help_message(
            "When on, hew-guard fails close if a behavior-changing task ships without a test. \
             More tokens at close time, better maintainability.",
        )
        .prompt()
        .unwrap_or(false)
}

fn resolve_optional_skills(ctx: &Ctx, args: &Args) -> (SkillModeArg, SkillModeArg, SkillModeArg) {
    // All three flags given? skip the whole prompt block entirely.
    if let (Some(d), Some(r), Some(s)) = (args.deps, args.research, args.security) {
        return (d, r, s);
    }
    let want_prompt = ctx.interactive;
    if want_prompt && !ctx.quiet {
        println!();
        println!("Plan-chain optional skills (more tokens per plan, but better outcomes):");
    }
    let deps = resolve_single_skill(ctx, args.deps, "deps", "vet new dependencies before adopting");
    let research =
        resolve_single_skill(ctx, args.research, "research", "web-cited research before planning");
    let security = resolve_single_skill(ctx, args.security, "security", "auth/input/secret checks");
    (deps, research, security)
}

fn resolve_single_skill(
    ctx: &Ctx,
    flag: Option<SkillModeArg>,
    name: &str,
    blurb: &str,
) -> SkillModeArg {
    if let Some(v) = flag {
        return v;
    }
    if !ctx.interactive {
        return SkillModeArg::Ask;
    }
    use inquire::Select;
    let opts = vec!["ask", "yes", "no"];
    let pick = Select::new(&format!("{name:<8} — {blurb}"), opts).with_starting_cursor(0).prompt();
    match pick {
        Ok("yes") => SkillModeArg::Yes,
        Ok("no") => SkillModeArg::No,
        _ => SkillModeArg::Ask,
    }
}

fn resolve_branching(ctx: &Ctx, args: &Args) -> BranchingArg {
    if let Some(b) = args.branching {
        return b;
    }
    if !ctx.interactive {
        return BranchingArg::Epic;
    }
    use inquire::Select;
    let opts = vec!["epic", "none", "always"];
    let pick = Select::new("Auto-branching strategy?", opts)
        .with_help_message(
            "epic = one branch per epic (recommended); none = manual; always = one branch per task",
        )
        .with_starting_cursor(0)
        .prompt();
    match pick {
        Ok("none") => BranchingArg::None,
        Ok("always") => BranchingArg::Always,
        _ => BranchingArg::Epic,
    }
}

fn resolve_project_type(ctx: &Ctx, args: &Args, project_root: &std::path::Path) -> ProjectTypeArg {
    if let Some(p) = args.project_type {
        return p;
    }
    let detected = detect_project_type(project_root);
    if !ctx.interactive {
        return detected;
    }
    use inquire::Select;
    let opts = vec![ProjectTypeArg::Existing.as_str(), ProjectTypeArg::New.as_str()];
    let cursor = if detected == ProjectTypeArg::Existing { 0 } else { 1 };
    let pick = Select::new("Project state?", opts).with_starting_cursor(cursor).prompt();
    match pick {
        Ok("existing") => ProjectTypeArg::Existing,
        Ok("new") => ProjectTypeArg::New,
        _ => detected,
    }
}

/// Decide which runtimes to install for. Pure: no I/O. The interactive
/// picker (if needed) is injected by the caller. Six-cell matrix over
/// {empty,one,many} args × {interactive,non-interactive}:
///
/// - args non-empty → map RuntimeArg→Runtime, preserve order, dedupe.
/// - args empty + interactive → invoke picker with detected as pre-checked.
/// - args empty + non-interactive + zero detected → `MissingFlag`.
/// - args empty + non-interactive + ≥1 detected → return all detected
///   (refresh-all; was an error today, see hew-a41 / DECISION).
fn resolve_runtimes<F>(
    args: &Args,
    detected: Vec<Runtime>,
    interactive: bool,
    picker: F,
) -> miette::Result<Vec<Runtime>>
where
    F: FnOnce(&[Runtime]) -> miette::Result<Vec<Runtime>>,
{
    if !args.runtime.is_empty() {
        let mut out: Vec<Runtime> = Vec::with_capacity(args.runtime.len());
        for r in &args.runtime {
            let rt: Runtime = (*r).into();
            if !out.contains(&rt) {
                out.push(rt);
            }
        }
        return Ok(out);
    }
    if interactive {
        return picker(&detected);
    }
    if detected.is_empty() {
        return Err(HewError::MissingFlag { flag: "runtime".into() }.into());
    }
    Ok(detected)
}

/// Multi-select picker for the interactive empty-args path. Items in
/// `pre_checked` (typically `install::detect_runtimes()` output on re-runs)
/// start checked; users toggle with space, confirm with enter. Empty
/// selections re-prompt so the caller always gets at least one runtime.
fn interactive_runtime_pick(pre_checked: &[Runtime]) -> miette::Result<Vec<Runtime>> {
    use inquire::MultiSelect;
    let options: Vec<Runtime> =
        vec![Runtime::Claude, Runtime::Cursor, Runtime::Codex, Runtime::Windsurf, Runtime::Generic];
    let labels: Vec<&'static str> = options.iter().map(|r| r.as_str()).collect();
    let default_indices: Vec<usize> = options
        .iter()
        .enumerate()
        .filter(|(_, r)| pre_checked.contains(r))
        .map(|(i, _)| i)
        .collect();

    loop {
        let picked = MultiSelect::new("Which agent runtime(s)?", labels.clone())
            .with_default(&default_indices)
            .with_help_message("space to toggle, enter to confirm — at least one required")
            .prompt()
            .map_err(|e| miette::miette!("runtime pick: {e}"))?;
        if picked.is_empty() {
            eprintln!("hew init: pick at least one runtime.");
            continue;
        }
        return Ok(picked
            .iter()
            .filter_map(|l| options.iter().find(|r| r.as_str() == *l).copied())
            .collect());
    }
}

fn resolve_install_root(scope: Scope, project_root: &std::path::Path) -> miette::Result<PathBuf> {
    match scope {
        Scope::Local => Ok(project_root.to_path_buf()),
        Scope::Global => {
            use etcetera::BaseStrategy;
            // ~/ — adapters lay down their own subpaths beneath this root.
            let strategy = etcetera::choose_base_strategy()
                .map_err(|e| miette::miette!("home strategy: {e}"))?;
            Ok(strategy.home_dir().to_path_buf())
        }
    }
}

/// Beads is a hard requirement. If it's not on PATH, install it.
/// Prefers `brew` if available, falls back to `curl ... beads.sh/install | sh`.
fn ensure_bd(ctx: &Ctx) -> miette::Result<RealBd> {
    if let Ok(bd) = RealBd::discover() {
        if !ctx.quiet {
            println!("beads: ✓ on PATH");
        }
        return Ok(bd);
    }

    let installer = if which::which("brew").is_ok() {
        "brew"
    } else if which::which("curl").is_ok() && which::which("sh").is_ok() {
        "curl"
    } else {
        return Err(miette::miette!(
            "Beads is required and neither `brew` nor `curl` is on PATH. \
             Install Homebrew (https://brew.sh) or curl, then re-run. \
             Beads docs: https://gastownhall.github.io/beads/"
        ));
    };

    if !ctx.quiet {
        println!("beads: ✗ not on PATH — installing via {installer}...");
    }

    match installer {
        "brew" => install_via_brew(ctx)?,
        _ => install_via_curl(ctx)?,
    }

    let bd = RealBd::discover().map_err(|_| {
        miette::miette!(
            "Beads install ran but `bd` still isn't on PATH. \
             Open a new shell so PATH refreshes, then re-run `hew init`."
        )
    })?;
    if !ctx.quiet {
        println!("beads: ✓ installed");
    }
    Ok(bd)
}

fn install_via_brew(_ctx: &Ctx) -> miette::Result<()> {
    run_streaming(std::process::Command::new("brew").args(["install", "beads"]))
        .map_err(|e| miette::miette!("brew install beads failed: {e}"))
}

fn install_via_curl(_ctx: &Ctx) -> miette::Result<()> {
    // Pipe curl into sh. Manual two-stage so we can stream both stdouts.
    let mut curl = std::process::Command::new("curl")
        .args(["-sSL", "https://beads.sh/install"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| miette::miette!("spawn curl: {e}"))?;
    let curl_stdout = curl.stdout.take().expect("piped");
    let sh_status = std::process::Command::new("sh")
        .stdin(curl_stdout)
        .status()
        .map_err(|e| miette::miette!("spawn sh: {e}"))?;
    let curl_status = curl.wait().map_err(|e| miette::miette!("wait curl: {e}"))?;
    if !curl_status.success() {
        return Err(miette::miette!("curl exited {:?}", curl_status.code()));
    }
    if !sh_status.success() {
        return Err(miette::miette!("beads install script exited {:?}", sh_status.code()));
    }
    Ok(())
}

fn run_streaming(cmd: &mut std::process::Command) -> std::io::Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!("exit {:?}", status.code())));
    }
    Ok(())
}

/// Git is *optional* for hew. Detect, then per DECISION:git-install-policy:
///
/// - Present → status line, continue.
/// - Missing + non-interactive → warn that auto-branching will be skipped.
/// - Missing + interactive → auto-attempt sudo-free install (no prompt). On
///   failure, print the hint and let the user run it themselves. Never sudo.
///   Never fail init.
fn ensure_git(ctx: &Ctx) {
    if RealGit::is_available() {
        if !ctx.quiet {
            println!("git: ✓ on PATH");
        }
        return;
    }

    if !ctx.interactive {
        if !ctx.quiet {
            eprintln!(
                "hew init: `git` not on PATH — auto-branching will be skipped. \
                 Install git and re-run if you want hew-execute to manage branches."
            );
        }
        return;
    }

    // Interactive + missing: auto-install, no Confirm prompt.
    let os = os::detect_os();
    let hint = os::git_install_hint(&os);
    if !ctx.quiet {
        println!("git: ✗ not on PATH — installing...");
    }

    let outcome = os::try_install_git_sudo_free(&os);
    let installed = matches!(&outcome, Ok(true));
    if installed && !ctx.quiet {
        println!("git: ✓ installed");
        return;
    }

    if !ctx.quiet {
        match &outcome {
            Ok(false) => eprintln!(
                "git: ✗ install needs sudo on this OS. Run: {hint}\n  see https://git-scm.com/downloads"
            ),
            Err(e) => eprintln!(
                "git: ✗ install failed: {e}. Run: {hint}\n  see https://git-scm.com/downloads"
            ),
            _ => {}
        }
        if matches!(os, OsKind::MacOs) {
            eprintln!("  (or install Homebrew first: https://brew.sh)");
        }
    }
}

/// Initialise a git repo in `project_root` if git is available and no
/// `.git/` exists. Never fails the install; just prints a status line.
/// Decide whether to track `.beads/` in git. CLI flag wins. Otherwise:
/// no .git/ → false; non-interactive → false (default); interactive → ask.
fn resolve_git_track(ctx: &Ctx, args: &Args, project_root: &std::path::Path) -> bool {
    if args.git_track {
        return true;
    }
    if args.stealth {
        return false;
    }
    if !project_root.join(".git").exists() {
        return false;
    }
    if !ctx.interactive {
        return false;
    }
    use inquire::Confirm;
    Confirm::new("Share the task graph in git?")
        .with_default(false)
        .with_help_message(".beads/ would be tracked alongside source.")
        .prompt()
        .unwrap_or(false)
}

/// Best-effort config write. Logs to stderr on failure; never aborts init.
fn persist_config<F>(ctx: &Ctx, mutate: F)
where
    F: FnOnce(&mut config::Config),
{
    let mut cfg = config::load().unwrap_or_default();
    mutate(&mut cfg);
    if let Err(e) = config::save(&cfg)
        && !ctx.quiet
    {
        eprintln!("hew init: warning — could not persist config: {e}");
    }
}

fn init_git_repo(ctx: &Ctx, project_root: &std::path::Path) -> miette::Result<()> {
    if !RealGit::is_available() {
        return Ok(());
    }
    if project_root.join(".git").exists() {
        return Ok(());
    }
    let git = match RealGit::discover() {
        Ok(g) => g,
        Err(_) => return Ok(()),
    };
    let root_os = std::ffi::OsString::from(project_root.as_os_str());
    let args: [&OsStr; 4] =
        [OsStr::new("-C"), root_os.as_os_str(), OsStr::new("init"), OsStr::new("--quiet")];
    match git.run_raw(&args) {
        Ok(_) => {
            if !ctx.quiet {
                println!("git: ✓ initialised repo");
            }
        }
        Err(e) => {
            if !ctx.quiet {
                eprintln!("git: ✗ git init failed: {e}");
            }
        }
    }
    Ok(())
}

fn run_bd_init(
    bd: &RealBd,
    project_root: &std::path::Path,
    prefix: Option<&str>,
    git_track: bool,
) -> miette::Result<()> {
    if project_root.join(".beads").exists() {
        return Ok(()); // already initialized
    }
    // --skip-agents: hew owns CLAUDE.md / AGENTS.md / .claude/settings.json.
    //   Without this, bd init writes a competing CLAUDE.md telling agents to
    //   "use bd for ALL task tracking" — direct conflict with the
    //   prefer-hew-over-bd contract — and clobbers our SessionStart hook.
    // --stealth (when !git_track): excludes .beads/ via .git/info/exclude
    //   and suppresses bd's auto-commit of beads files.
    let mut args: Vec<&OsStr> =
        vec![OsStr::new("init"), OsStr::new("--non-interactive"), OsStr::new("--skip-agents")];
    if !git_track {
        args.push(OsStr::new("--stealth"));
    }
    let pfx_os: std::ffi::OsString;
    if let Some(p) = prefix {
        args.push(OsStr::new("--prefix"));
        pfx_os = std::ffi::OsString::from(p);
        args.push(pfx_os.as_os_str());
    }
    bd.run_raw(&args)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Args-parsing tests for `--runtime` multi-value semantics (hew-0tz).
    //!
    //! Parse `hew init` argv via a minimal clap shim that wraps the real
    //! `Args` struct; this exercises the same `#[arg(...)]` attributes the
    //! production binary uses, without depending on the full `Cli` tree.
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct InitOnly {
        #[command(flatten)]
        args: Args,
    }

    fn parse(argv: &[&str]) -> Result<Args, clap::Error> {
        let mut full = vec!["hew-init-test"];
        full.extend_from_slice(argv);
        InitOnly::try_parse_from(full).map(|p| p.args)
    }

    #[test]
    fn runtime_csv_form_parses_to_vec() {
        let args = parse(&["--runtime=claude,codex"]).expect("parse");
        assert!(matches!(args.runtime.as_slice(), [RuntimeArg::Claude, RuntimeArg::Codex]));
    }

    #[test]
    fn runtime_repeated_flag_form_parses_to_vec() {
        let args = parse(&["--runtime=claude", "--runtime=codex"]).expect("parse");
        assert!(matches!(args.runtime.as_slice(), [RuntimeArg::Claude, RuntimeArg::Codex]));
    }

    #[test]
    fn runtime_mixed_csv_and_repeated_appends_in_order() {
        let args = parse(&["--runtime=claude,codex", "--runtime=windsurf"]).expect("parse");
        assert!(matches!(
            args.runtime.as_slice(),
            [RuntimeArg::Claude, RuntimeArg::Codex, RuntimeArg::Windsurf]
        ));
    }

    #[test]
    fn runtime_absent_yields_empty_vec() {
        let args = parse(&[]).expect("parse");
        assert!(args.runtime.is_empty());
    }

    #[test]
    fn runtime_csv_with_duplicates_is_deduped_in_resolver() {
        let args = parse(&["--runtime=claude,claude,codex"]).expect("parse");
        let panic_picker = |_: &[Runtime]| -> miette::Result<Vec<Runtime>> {
            panic!("picker must not be called when args non-empty")
        };
        let out = resolve_runtimes(&args, vec![], false, panic_picker).expect("resolve");
        assert_eq!(out, vec![Runtime::Claude, Runtime::Codex]);
    }

    // --- six-cell matrix: {empty,one,many} args × {interactive,non-interactive} ---

    fn no_picker(_: &[Runtime]) -> miette::Result<Vec<Runtime>> {
        panic!("picker called but should not be in this cell")
    }

    #[test]
    fn resolve_one_arg_non_interactive() {
        let args = parse(&["--runtime=claude"]).expect("parse");
        let out = resolve_runtimes(&args, vec![], false, no_picker).expect("resolve");
        assert_eq!(out, vec![Runtime::Claude]);
    }

    #[test]
    fn resolve_one_arg_interactive_skips_picker() {
        // Non-empty args short-circuits the picker even in interactive mode.
        let args = parse(&["--runtime=codex"]).expect("parse");
        let out = resolve_runtimes(&args, vec![Runtime::Claude], true, no_picker).expect("resolve");
        assert_eq!(out, vec![Runtime::Codex]);
    }

    #[test]
    fn resolve_many_args_non_interactive_preserves_order() {
        let args = parse(&["--runtime=codex,claude,windsurf"]).expect("parse");
        let out = resolve_runtimes(&args, vec![], false, no_picker).expect("resolve");
        assert_eq!(out, vec![Runtime::Codex, Runtime::Claude, Runtime::Windsurf]);
    }

    #[test]
    fn resolve_many_args_interactive_skips_picker() {
        let args = parse(&["--runtime=codex,claude"]).expect("parse");
        let out =
            resolve_runtimes(&args, vec![Runtime::Windsurf], true, no_picker).expect("resolve");
        assert_eq!(out, vec![Runtime::Codex, Runtime::Claude]);
    }

    #[test]
    fn resolve_empty_args_non_interactive_zero_detected_errors() {
        let args = parse(&[]).expect("parse");
        let err = resolve_runtimes(&args, vec![], false, no_picker).expect_err("should error");
        let msg = format!("{err:?}");
        assert!(msg.contains("runtime"), "expected MissingFlag(runtime), got: {msg}");
    }

    #[test]
    fn resolve_empty_args_non_interactive_with_detected_refreshes_all() {
        // hew-a41: was an error today; now returns all detected.
        let args = parse(&[]).expect("parse");
        let detected = vec![Runtime::Claude, Runtime::Codex];
        let out = resolve_runtimes(&args, detected.clone(), false, no_picker).expect("resolve");
        assert_eq!(out, detected);
    }

    #[test]
    fn resolve_empty_args_interactive_delegates_to_picker_with_detected() {
        let args = parse(&[]).expect("parse");
        let detected = vec![Runtime::Claude, Runtime::Codex];
        let detected_seen = std::cell::RefCell::new(Vec::<Runtime>::new());
        let picker = |pre: &[Runtime]| {
            *detected_seen.borrow_mut() = pre.to_vec();
            Ok(vec![Runtime::Windsurf])
        };
        let out = resolve_runtimes(&args, detected.clone(), true, picker).expect("resolve");
        assert_eq!(out, vec![Runtime::Windsurf]);
        assert_eq!(*detected_seen.borrow(), detected);
    }

    #[test]
    fn resolve_empty_args_interactive_zero_detected_still_delegates() {
        let args = parse(&[]).expect("parse");
        let picker = |pre: &[Runtime]| {
            assert!(pre.is_empty());
            Ok(vec![Runtime::Generic])
        };
        let out = resolve_runtimes(&args, vec![], true, picker).expect("resolve");
        assert_eq!(out, vec![Runtime::Generic]);
    }

    #[test]
    fn runtime_display_names_cover_all_variants() {
        assert_eq!(runtime_display_name(Runtime::Claude), "Claude");
        assert_eq!(runtime_display_name(Runtime::Cursor), "Cursor");
        assert_eq!(runtime_display_name(Runtime::Codex), "Codex");
        assert_eq!(runtime_display_name(Runtime::Windsurf), "Windsurf");
        assert_eq!(runtime_display_name(Runtime::Generic), "Generic");
    }

    #[test]
    fn runtime_artifact_labels_match_install_layout() {
        assert_eq!(runtime_artifact_label(Runtime::Claude), ".claude/");
        assert_eq!(runtime_artifact_label(Runtime::Cursor), ".cursorrules");
        assert_eq!(runtime_artifact_label(Runtime::Codex), ".codex/ + AGENTS.md");
        assert_eq!(runtime_artifact_label(Runtime::Windsurf), ".windsurfrules");
        assert_eq!(runtime_artifact_label(Runtime::Generic), "");
    }

    #[test]
    fn runtime_invalid_value_rejected_by_clap() {
        let err = parse(&["--runtime=bogus"]).expect_err("should fail");
        // clap renders a "possible values" list — assert one of them surfaces.
        let msg = err.to_string();
        assert!(msg.contains("claude"), "expected possible-values list, got: {msg}");
    }
}
