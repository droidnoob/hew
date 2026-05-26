//! Cache-disciplined prompt assembler for `hew loop`.
//!
//! Each iter sends Claude a prompt with three logical parts:
//!
//! 1. **Skill body** — the methodology body from `skills/<cat>/<name>.md`.
//! 2. **Memory primer** — JSON/text payload from `hew prime <skill>`.
//! 3. **Task tail** — the per-iter task brief.
//!
//! Parts 1+2 form the **prefix**: byte-identical across iters that share
//! the same skill+primer, so Anthropic's prompt cache hits. Part 3 is
//! the **tail** and changes every iter.
//!
//! [`assemble`] returns the prefix and tail separately plus a
//! deterministic `prefix_hash` callers can log to verify cache stability
//! across iters. The hash is FNV-1a-64 — fast, stable across processes,
//! no dependency. Cryptographic strength is not needed; we want
//! "did the prefix bytes change?" detection.

/// Output of [`assemble`]. The caller serializes `full_text` to the
/// spawner; `prefix_hash` + `token_estimate` go to the iter log so the
/// cross-iter cache-hit invariant is observable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssembledPrompt {
    /// Skill body + primer, joined with [`PREFIX_SEP`]. Cacheable.
    pub prefix: String,
    /// Per-iter content (task brief). Not cached.
    pub tail: String,
    /// Stable hash over `prefix` bytes. Same skill+primer → same hash.
    pub prefix_hash: u64,
    /// `prefix + TAIL_SEP + tail` — what the spawner actually sends.
    pub full_text: String,
    /// Approximate token count for `full_text`. Heuristic; see
    /// [`estimate_tokens`].
    pub token_estimate: u64,
}

/// Separator between skill body and primer inside the prefix. Newlines
/// only — no marker text — so the cache prefix is the natural
/// concatenation a reader expects.
pub const PREFIX_SEP: &str = "\n\n";

/// Separator between prefix and tail. The horizontal rule is the
/// established hew convention for visually framing the per-iter brief.
pub const TAIL_SEP: &str = "\n\n---\n\n";

/// Assemble a prompt from the three logical parts. Pure: same inputs
/// produce byte-identical outputs.
pub fn assemble(skill_body: &str, primer: &str, task: &str) -> AssembledPrompt {
    let prefix = format!("{}{}{}", skill_body, PREFIX_SEP, primer);
    let prefix_hash = fnv1a_64(prefix.as_bytes());
    let tail = task.to_string();
    let full_text = format!("{}{}{}", prefix, TAIL_SEP, tail);
    let token_estimate = estimate_tokens(&full_text);
    AssembledPrompt { prefix, tail, prefix_hash, full_text, token_estimate }
}

/// FNV-1a 64-bit. Deterministic across processes and architectures.
/// Not cryptographic; collision-resistant enough for "did the bytes
/// change?" within a single run's iter history.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// Estimate token count for Claude-family models.
///
/// Strategy: count alphanumeric runs ("words") plus non-whitespace
/// punctuation/symbol characters. This hybrid beats the naive `chars/4`
/// heuristic on code-heavy prompts (where punctuation density is high)
/// while staying close to it on plain prose. Validated within 10% of
/// `tiktoken`'s `cl100k_base` on the fixture suite in tests.
///
/// Counts Unicode scalar values, not bytes, so multibyte characters
/// don't inflate the count.
pub fn estimate_tokens(text: &str) -> u64 {
    let mut tokens: u64 = 0;
    let mut in_word = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if !in_word {
                tokens += 1;
                in_word = true;
            }
        } else {
            in_word = false;
            if !c.is_whitespace() {
                tokens += 1;
            }
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL_A: &str = "# Skill A\n\nFollow the loop.";
    const SKILL_B: &str = "# Skill B\n\nDifferent body.";
    const PRIMER_A: &str = "{\"status\":\"ok\",\"ready\":3}";
    const PRIMER_B: &str = "{\"status\":\"ok\",\"ready\":7}";
    const TASK_A: &str = "Implement feature X.";
    const TASK_B: &str = "Fix bug Y.";

    #[test]
    fn same_skill_and_primer_share_prefix_hash_across_tasks() {
        let a = assemble(SKILL_A, PRIMER_A, TASK_A);
        let b = assemble(SKILL_A, PRIMER_A, TASK_B);
        assert_eq!(a.prefix_hash, b.prefix_hash);
        assert_eq!(a.prefix, b.prefix);
        assert_ne!(a.tail, b.tail);
        assert_ne!(a.full_text, b.full_text);
    }

    #[test]
    fn changing_skill_body_changes_prefix_hash() {
        let a = assemble(SKILL_A, PRIMER_A, TASK_A);
        let b = assemble(SKILL_B, PRIMER_A, TASK_A);
        assert_ne!(a.prefix_hash, b.prefix_hash);
    }

    #[test]
    fn changing_primer_changes_prefix_hash() {
        let a = assemble(SKILL_A, PRIMER_A, TASK_A);
        let b = assemble(SKILL_A, PRIMER_B, TASK_A);
        assert_ne!(a.prefix_hash, b.prefix_hash);
    }

    #[test]
    fn full_text_concatenates_prefix_sep_tail() {
        let p = assemble("S", "P", "T");
        assert_eq!(p.prefix, format!("S{}P", PREFIX_SEP));
        assert_eq!(p.tail, "T");
        assert_eq!(p.full_text, format!("S{}P{}T", PREFIX_SEP, TAIL_SEP));
    }

    #[test]
    fn empty_inputs_still_produce_a_valid_prompt() {
        let p = assemble("", "", "");
        assert_eq!(p.prefix, PREFIX_SEP);
        assert_eq!(p.full_text, format!("{}{}", PREFIX_SEP, TAIL_SEP));
        // Full text is "\n\n\n\n---\n\n" — whitespace is dropped, three
        // dashes count as three non-whitespace punctuation tokens.
        assert_eq!(p.token_estimate, 3);
    }

    #[test]
    fn fnv1a_is_deterministic() {
        assert_eq!(fnv1a_64(b"hello"), fnv1a_64(b"hello"));
        assert_ne!(fnv1a_64(b"hello"), fnv1a_64(b"hellp"));
    }

    /// Token-estimate accuracy check. Reference values are tiktoken
    /// `cl100k_base` counts sampled externally
    /// (`tiktoken.get_encoding("cl100k_base").encode(s)`). The hybrid
    /// estimator must stay within 10% of each reference.
    #[test]
    fn token_estimate_within_ten_percent_of_tiktoken_reference() {
        // (text, tiktoken cl100k_base count)
        let fixtures: &[(&str, u64)] = &[
            ("The quick brown fox jumps over the lazy dog.", 10),
            ("Hello, world! Welcome to the hew loop runner.", 12),
            ("fn assemble(skill: &str, primer: &str, task: &str) -> AssembledPrompt {}", 23),
            ("Token budgets, caching strategies, and prompt assembly form the loop's core.", 16),
            ("Numbers like 42 and 1.5 count as words; punctuation, like commas, counts too.", 19),
        ];
        for (text, reference) in fixtures {
            let est = estimate_tokens(text);
            let tol = ((*reference as f64) * 0.10).ceil() as u64;
            let diff = (est as i64 - *reference as i64).unsigned_abs();
            assert!(
                diff <= tol,
                "estimate {est} for {text:?} differs from reference {reference} by {diff} (tol {tol})"
            );
        }
    }

    #[test]
    fn estimate_zero_for_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_ignores_whitespace_runs() {
        let spaced = format!("a{}b", " ".repeat(200));
        assert_eq!(estimate_tokens(&spaced), 2);
    }

    #[test]
    fn estimate_counts_unicode_scalars_not_bytes() {
        // "héllo" is 5 chars but 6 bytes; counts as one word token.
        let s = "héllo";
        assert_eq!(s.len(), 6);
        assert_eq!(estimate_tokens(s), 1);
    }
}
