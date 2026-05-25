//! CHECKPOINT memory shape helpers.
//!
//! A well-formed checkpoint body is `CHECKPOINT:<ISO-8601> — <text>`.
//! `hew prime resume` parses the leading ISO token to pick the most
//! recent checkpoint, so getting the shape right matters — a malformed
//! body silently shadows newer good ones in the resume primer.
//!
//! These helpers exist so callers (the `hew checkpoint` subcommand,
//! tests, future tooling) don't reinvent the prefixing logic and
//! mis-shape the body the way `hew remember --raw …` made too easy.

use crate::time::looks_like_iso_date;

/// Default key shape: `checkpoint-<sanitised-iso>`. Colons in the ISO
/// stamp are replaced with `-` so the key matches the LINK-row
/// charset (`[a-z0-9._-]+`) and survives `hew remember --related`.
pub fn build_checkpoint_key(now_iso: &str) -> String {
    let sanitised: String = now_iso
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            ':' => '-',
            _ => c,
        })
        .collect();
    format!("checkpoint-{sanitised}")
}

/// Compose the canonical body. Three branches:
///
/// 1. `body` already starts with `CHECKPOINT:<ISO> …` — return verbatim
///    (the caller did the work).
/// 2. `body` starts with `CHECKPOINT:` but the next token isn't an ISO
///    date — strip the broken prefix and reapply (the common bug
///    shape, see GitHub issue #40).
/// 3. `body` has no `CHECKPOINT:` prefix — prepend one with timestamp
///    and an em-dash separator.
pub fn build_checkpoint_body(body: &str, now_iso: &str) -> String {
    let trimmed = body.trim_start();
    if let Some(rest) = trimmed.strip_prefix("CHECKPOINT:") {
        let first = rest.split_whitespace().next().unwrap_or("");
        if looks_like_iso_date(first) {
            return trimmed.to_string();
        }
        let payload = rest.trim_start();
        if payload.is_empty() {
            return format!("CHECKPOINT:{now_iso}");
        }
        return format!("CHECKPOINT:{now_iso} — {payload}");
    }
    if trimmed.is_empty() {
        return format!("CHECKPOINT:{now_iso}");
    }
    format!("CHECKPOINT:{now_iso} — {trimmed}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISO: &str = "2026-05-25T12:34:56Z";

    #[test]
    fn key_lowercases_and_replaces_colons() {
        assert_eq!(build_checkpoint_key("2026-05-25T12:34:56Z"), "checkpoint-2026-05-25t12-34-56z");
    }

    #[test]
    fn body_well_formed_passes_through() {
        let good = "CHECKPOINT:2026-05-20T08:00:00Z — already correct";
        assert_eq!(build_checkpoint_body(good, ISO), good);
    }

    #[test]
    fn body_well_formed_trims_leading_whitespace() {
        let good = "  CHECKPOINT:2026-05-20T08:00:00Z — leading ws";
        assert_eq!(
            build_checkpoint_body(good, ISO),
            "CHECKPOINT:2026-05-20T08:00:00Z — leading ws"
        );
    }

    #[test]
    fn body_missing_prefix_gets_one() {
        // Matches what the agent in issue #40 was typing — bare body,
        // no prefix at all.
        assert_eq!(
            build_checkpoint_body("Working on practice-svc-l3.2; refresh-rotation in flight.", ISO),
            "CHECKPOINT:2026-05-25T12:34:56Z — Working on practice-svc-l3.2; refresh-rotation in flight."
        );
    }

    #[test]
    fn body_with_broken_prefix_gets_rewritten() {
        // The exact bug shape from issue #40: prefix present but next
        // token is not an ISO date.
        let bad = "CHECKPOINT:practice-svc-l3.2 — work in flight";
        assert_eq!(
            build_checkpoint_body(bad, ISO),
            "CHECKPOINT:2026-05-25T12:34:56Z — practice-svc-l3.2 — work in flight"
        );
    }

    #[test]
    fn body_with_bare_checkpoint_colon_gets_timestamp() {
        // `CHECKPOINT:\nbody…` — the bug variant where the agent typed
        // the prefix and then a newline before the rest.
        let bad = "CHECKPOINT:\nworking on X";
        assert_eq!(
            build_checkpoint_body(bad, ISO),
            "CHECKPOINT:2026-05-25T12:34:56Z — working on X"
        );
    }

    #[test]
    fn body_with_only_checkpoint_prefix_becomes_timestamp_only() {
        assert_eq!(build_checkpoint_body("CHECKPOINT:", ISO), "CHECKPOINT:2026-05-25T12:34:56Z");
    }

    #[test]
    fn body_empty_becomes_timestamp_only() {
        assert_eq!(build_checkpoint_body("", ISO), "CHECKPOINT:2026-05-25T12:34:56Z");
        assert_eq!(build_checkpoint_body("   ", ISO), "CHECKPOINT:2026-05-25T12:34:56Z");
    }
}
