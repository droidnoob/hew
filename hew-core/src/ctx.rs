use crate::tty;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OutputMode {
    Auto,
    Json,
    Text,
}

impl OutputMode {
    pub fn resolve(self) -> Self {
        match self {
            Self::Auto => {
                use std::io::IsTerminal;
                if std::io::stdout().is_terminal() { Self::Text } else { Self::Json }
            }
            other => other,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ctx {
    pub interactive: bool,
    pub output: OutputMode,
    pub quiet: bool,
    pub verbose: u8,
}

impl Ctx {
    pub fn new(force_non_interactive: bool, output: OutputMode, quiet: bool, verbose: u8) -> Self {
        let interactive = !tty::is_non_interactive(force_non_interactive);
        Self { interactive, output: output.resolve(), quiet, verbose }
    }
}
