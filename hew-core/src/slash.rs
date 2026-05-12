//! Compile-time registry of slash commands shipped with the binary.
//!
//! Mirrors the skills registry. `hew init` writes these into the
//! Claude runtime's commands directory; users invoke them as
//! `/hew:<name>` from inside Claude Code.

#[derive(Copy, Clone, Debug)]
pub struct SlashCommand {
    pub name: &'static str,
    pub body: &'static str,
}

macro_rules! cmd {
    ($name:expr, $file:expr) => {
        SlashCommand { name: $name, body: include_str!(concat!("../../commands/", $file)) }
    };
}

pub const ALL: &[SlashCommand] = &[
    cmd!("do", "do.md"),
    cmd!("next", "next.md"),
    cmd!("auto", "auto.md"),
    cmd!("plan", "plan.md"),
    cmd!("work", "work.md"),
    cmd!("quick", "quick.md"),
    cmd!("verify", "verify.md"),
    cmd!("ship", "ship.md"),
    cmd!("test", "test.md"),
    cmd!("add", "add.md"),
    cmd!("drop", "drop.md"),
    cmd!("epic", "epic.md"),
    cmd!("note", "note.md"),
    cmd!("ingest", "ingest.md"),
    cmd!("debug", "debug.md"),
    cmd!("forensic", "forensic.md"),
    cmd!("review", "review.md"),
    cmd!("status", "status.md"),
    cmd!("report", "report.md"),
    cmd!("doctor", "doctor.md"),
    cmd!("config", "config.md"),
    cmd!("help", "help.md"),
    cmd!("update", "update.md"),
    cmd!("checkpoint", "checkpoint.md"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_twenty_four_commands() {
        assert_eq!(ALL.len(), 24);
    }

    #[test]
    fn every_command_has_a_nonempty_body() {
        for c in ALL {
            assert!(!c.body.trim().is_empty(), "{}", c.name);
            assert!(c.body.contains("description:"), "{} missing frontmatter", c.name);
        }
    }

    #[test]
    fn command_names_are_unique() {
        let mut names: Vec<&str> = ALL.iter().map(|c| c.name).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }
}
