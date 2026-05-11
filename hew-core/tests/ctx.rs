use hew_core::{Ctx, OutputMode};

#[test]
fn force_non_interactive_sets_interactive_false() {
    let ctx = Ctx::new(true, OutputMode::Json, false, 0);
    assert!(!ctx.interactive);
}

#[test]
fn explicit_json_output_is_preserved() {
    let ctx = Ctx::new(true, OutputMode::Json, false, 0);
    assert_eq!(ctx.output, OutputMode::Json);
}

#[test]
fn explicit_text_output_is_preserved() {
    let ctx = Ctx::new(true, OutputMode::Text, false, 0);
    assert_eq!(ctx.output, OutputMode::Text);
}

#[test]
fn auto_output_resolves_to_concrete() {
    let ctx = Ctx::new(true, OutputMode::Auto, false, 0);
    assert!(matches!(ctx.output, OutputMode::Json | OutputMode::Text));
}
