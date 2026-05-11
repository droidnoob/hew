use assert_cmd::Command;
use predicates::str::contains;

fn hew() -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c
}

#[test]
fn completions_bash_starts_with_function() {
    hew().args(["completions", "bash"]).assert().success().stdout(contains("_hew()"));
}

#[test]
fn completions_zsh_has_compdef() {
    hew().args(["completions", "zsh"]).assert().success().stdout(contains("#compdef hew"));
}

#[test]
fn completions_fish_uses_complete() {
    hew().args(["completions", "fish"]).assert().success().stdout(contains("complete -c hew"));
}

#[test]
fn completions_powershell_emits_register_block() {
    hew()
        .args(["completions", "power-shell"])
        .assert()
        .success()
        .stdout(contains("Register-ArgumentCompleter"));
}

#[test]
fn completions_unknown_shell_rejected() {
    hew().args(["completions", "fishy"]).assert().failure().code(2);
}

#[test]
fn manpage_emits_roff_header() {
    hew()
        .arg("manpage")
        .assert()
        .success()
        .stdout(contains(".TH hew 1"))
        .stdout(contains("hew 0.1.0"));
}
