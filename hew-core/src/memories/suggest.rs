//! Lexical "these look related" ranker for the `hew remember`
//! interactive prompt (ML.9 / hew-3wt).
//!
//! Heuristic: tokenize bodies into lowercase significant words
//! (drop stop-words, drop tokens shorter than 3 chars, drop the
//! UPPER prefix noise like `CONVENTION:`), then score each existing
//! memory against the new body by:
//!
//! - **Shared-token count** — 1 point per significant token that
//!   appears in both bodies.
//! - **Same-prefix bonus** — +2 when both bodies start with the
//!   same UPPER prefix (`GOTCHA:` matches `GOTCHA:`).
//!
//! Zero-score candidates are dropped. Top-N (caller-supplied)
//! survive, ordered by score descending then key ascending for
//! deterministic display under ties. The CLI layer prompts the
//! user with these suggestions; selections feed back through the
//! existing `--related` write path (no new bd surface).
//!
//! Stays in `hew_core` per `CONVENTION:hew-core-purity` — no I/O,
//! no clap, no inquire. The interactive prompt itself lives in
//! `hew/src/commands/remember.rs`.

use std::collections::BTreeSet;

/// A single suggestion the CLI prompt should offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub key: String,
    pub score: u32,
    pub reason: String,
}

/// English stop-words that show up everywhere in memory bodies and
/// would otherwise dominate the score. Intentionally small — every
/// addition makes the ranker more opinionated and harder to debug.
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "but", "not", "with", "from", "this", "that", "are", "was", "were",
    "have", "has", "had", "you", "your", "our", "use", "uses", "used", "via", "per", "any", "all",
    "must", "should", "would", "could", "into", "onto", "out", "off", "than", "then", "when",
    "where", "while", "what", "which", "who", "why", "how", "one", "two",
];

/// Rank existing memories by lexical similarity to `new_body`.
/// Returns up to `top` suggestions, ordered by score descending.
/// `top == 0` returns an empty vec.
pub fn rank_related<K, B>(new_body: &str, existing: &[(K, B)], top: usize) -> Vec<Suggestion>
where
    K: AsRef<str>,
    B: AsRef<str>,
{
    if top == 0 {
        return Vec::new();
    }
    let new_prefix = upper_prefix(new_body);
    let new_tokens = significant_tokens(strip_known_prefix(new_body));

    let mut scored: Vec<Suggestion> = existing
        .iter()
        .filter_map(|(k, body)| {
            let body = body.as_ref();
            let other_tokens = significant_tokens(strip_known_prefix(body));
            let shared: Vec<&str> =
                new_tokens.intersection(&other_tokens).map(|s| s.as_str()).collect();
            let shared_count = shared.len() as u32;
            let same_prefix = match (&new_prefix, upper_prefix(body)) {
                (Some(a), Some(b)) => a == &b,
                _ => false,
            };
            let prefix_bonus = if same_prefix { 2 } else { 0 };
            let score = shared_count + prefix_bonus;
            if score == 0 {
                return None;
            }
            let reason = build_reason(&shared, same_prefix);
            Some(Suggestion { key: k.as_ref().to_string(), score, reason })
        })
        .collect();

    scored.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.key.cmp(&b.key)));
    scored.truncate(top);
    scored
}

fn upper_prefix(body: &str) -> Option<String> {
    let trimmed = body.trim_start();
    let colon = trimmed.find(':')?;
    let prefix = &trimmed[..colon];
    if !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_uppercase()) {
        Some(prefix.to_string())
    } else {
        None
    }
}

/// Strip an UPPER prefix like `CONVENTION:` off a body for
/// tokenization. Leaves bodies without a known prefix unchanged.
fn strip_known_prefix(body: &str) -> &str {
    let trimmed = body.trim_start();
    if let Some(colon) = trimmed.find(':') {
        let prefix = &trimmed[..colon];
        if !prefix.is_empty() && prefix.bytes().all(|b| b.is_ascii_uppercase()) {
            return trimmed[colon + 1..].trim_start();
        }
    }
    trimmed
}

/// Lowercased significant tokens — alphanumeric runs of ≥3 chars,
/// minus the stop-word list. BTreeSet so set ops are deterministic
/// across runs.
fn significant_tokens(body: &str) -> BTreeSet<String> {
    let stop: BTreeSet<&str> = STOP_WORDS.iter().copied().collect();
    let mut out: BTreeSet<String> = BTreeSet::new();
    let mut buf = String::new();
    for ch in body.chars() {
        if ch.is_ascii_alphanumeric() {
            buf.push(ch.to_ascii_lowercase());
        } else if !buf.is_empty() {
            if buf.len() >= 3 && !stop.contains(buf.as_str()) {
                out.insert(buf.clone());
            }
            buf.clear();
        }
    }
    if !buf.is_empty() && buf.len() >= 3 && !stop.contains(buf.as_str()) {
        out.insert(buf);
    }
    out
}

fn build_reason(shared: &[&str], same_prefix: bool) -> String {
    let mut sample: Vec<&str> = shared.to_vec();
    sample.sort();
    sample.truncate(3);
    let token_part = if sample.is_empty() {
        "no shared tokens".to_string()
    } else {
        format!("shares: {}", sample.join(", "))
    };
    if same_prefix { format!("{token_part} (+ same prefix)") } else { token_part }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<(&'static str, &'static str)> {
        vec![
            ("convention-jwt-shape", "CONVENTION:JWT auth — refresh tokens rotate on use"),
            ("decision-jwt-issuer", "DECISION:JWT issuer is the AppUser id, refresh TTL 7 days"),
            (
                "gotcha-flake-etxtbsy",
                "GOTCHA:Linux ETXTBSY race when chmod fires after write — see PR 27",
            ),
            ("convention-errors", "CONVENTION:wrap every handler error in AppError"),
            ("research-axoupdater", "RESEARCH:axoupdater install-receipt absent under brew"),
        ]
    }

    #[test]
    fn shared_token_candidate_ranks_above_no_overlap() {
        // Real-overlap candidates (JWT-related decision + convention)
        // must rank ABOVE a prefix-only candidate (the ETXTBSY gotcha
        // shares the GOTCHA: prefix but no significant tokens — score
        // is just the +2 prefix bonus).
        let new_body = "GOTCHA:Implement JWT refresh rotation in /auth/refresh handler";
        let out = rank_related(new_body, &fixture(), 5);
        assert!(out.len() >= 3);
        let top_two: Vec<&str> = out.iter().take(2).map(|s| s.key.as_str()).collect();
        assert!(top_two.contains(&"convention-jwt-shape"), "got: {top_two:?}");
        assert!(top_two.contains(&"decision-jwt-issuer"), "got: {top_two:?}");
        // The ETXTBSY gotcha can appear but only after the real
        // overlaps, never above them.
        let etxtbsy_pos = out.iter().position(|s| s.key == "gotcha-flake-etxtbsy");
        if let Some(p) = etxtbsy_pos {
            assert!(p >= 2, "prefix-only match ranked above token-overlap matches: {out:?}");
        }
    }

    #[test]
    fn same_prefix_bonus_applied() {
        // A new GOTCHA body that shares one token with both a
        // GOTCHA memory and a non-GOTCHA memory should rank the
        // matching prefix higher even when the non-prefix candidate
        // shares the same token count.
        let new_body = "GOTCHA:linux file race when fires after write again";
        let existing = vec![
            ("gotcha-other-linux", "GOTCHA:another linux issue with concurrent file access"),
            (
                "convention-linux",
                "CONVENTION:on linux always use atomic write+rename, never write-in-place",
            ),
        ];
        let out = rank_related(new_body, &existing, 5);
        assert!(out.len() >= 2);
        // The GOTCHA-prefixed one wins the prefix bonus.
        assert_eq!(out[0].key, "gotcha-other-linux");
        assert!(out[0].reason.contains("same prefix"), "reason: {}", out[0].reason);
    }

    #[test]
    fn empty_existing_returns_empty() {
        let out = rank_related("CONVENTION:anything", &[] as &[(&str, &str)], 3);
        assert!(out.is_empty());
    }

    #[test]
    fn top_zero_returns_empty() {
        let out = rank_related("GOTCHA:jwt body", &fixture(), 0);
        assert!(out.is_empty());
    }

    #[test]
    fn top_cap_respected() {
        let new_body = "GOTCHA:JWT issuer refresh rotation token tokens";
        let out = rank_related(new_body, &fixture(), 2);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn stop_words_are_filtered() {
        // Two bodies that share ONLY stop-words should produce no
        // suggestion. Without filtering they would falsely rank.
        let new_body = "CONVENTION:the and for but not with from this that";
        let existing =
            vec![("noise", "CONVENTION:the and for but not with from this that"), ("real", "x y")];
        let out = rank_related(new_body, &existing, 5);
        // Only the same-prefix bonus survives (no shared significant
        // tokens). The other memory ("real") shares zero, so it
        // doesn't appear.
        let keys: Vec<&str> = out.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, vec!["noise"]);
        // Score should be just the prefix bonus, nothing else.
        assert_eq!(out[0].score, 2);
    }

    #[test]
    fn short_tokens_are_filtered() {
        // Tokens shorter than 3 chars are ignored — otherwise common
        // letters like "is" / "to" would dominate.
        let new_body = "CONVENTION:to a x is by on";
        let existing = vec![("a", "CONVENTION:to a x is by on")];
        let out = rank_related(new_body, &existing, 5);
        // Same prefix → bonus 2, no shared significant tokens → 2.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].score, 2);
    }

    #[test]
    fn reason_lists_up_to_three_shared_tokens() {
        let new_body =
            "GOTCHA:JWT issuer refresh rotation token tokens scheme handler endpoint flow";
        let existing = vec![(
            "match-all",
            "GOTCHA:JWT issuer refresh rotation token tokens scheme handler endpoint flow",
        )];
        let out = rank_related(new_body, &existing, 5);
        assert_eq!(out.len(), 1);
        // Reason text shows up to 3 sample tokens.
        let comma_count = out[0].reason.matches(", ").count();
        assert!(comma_count <= 2, "reason should list ≤3 tokens: {}", out[0].reason);
    }

    #[test]
    fn ties_break_by_key_ascending() {
        let new_body = "CONVENTION:JWT auth body";
        let existing = vec![
            ("zzz-equal-score", "CONVENTION:JWT auth body"),
            ("aaa-equal-score", "CONVENTION:JWT auth body"),
        ];
        let out = rank_related(new_body, &existing, 5);
        assert_eq!(out[0].key, "aaa-equal-score", "tie-break by key ascending");
    }

    #[test]
    fn prefix_is_stripped_before_tokenization() {
        // Without stripping, the literal token "convention" would
        // appear in every CONVENTION body and dominate the score.
        let new_body = "CONVENTION:database migration plan";
        let existing = vec![("c1", "CONVENTION:another body about something else")];
        let out = rank_related(new_body, &existing, 5);
        // Same prefix gives +2 but no significant overlap.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].score, 2);
        assert!(!out[0].reason.contains("convention"), "prefix leaked into reason");
    }
}
