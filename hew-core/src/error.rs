use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum HewError {
    #[error("missing required value in non-interactive mode: --{flag}")]
    #[diagnostic(
        code(hew::cli::missing_flag),
        help("re-run with --{flag} <value>, or drop --non-interactive / HEW_NON_INTERACTIVE=1")
    )]
    MissingFlag { flag: String },

    #[error("`bd` binary not found on PATH")]
    #[diagnostic(
        code(hew::bd::not_found),
        help("install Beads: `brew install beads` or `curl -sSL https://beads.sh/install | sh`")
    )]
    BdNotFound,

    #[error("`bd` exited with status {code}: {stderr}")]
    #[diagnostic(code(hew::bd::nonzero_exit))]
    BdNonZero { code: i32, stderr: String },

    #[error("io error: {0}")]
    #[diagnostic(code(hew::io))]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    #[diagnostic(code(hew::json))]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, HewError>;
