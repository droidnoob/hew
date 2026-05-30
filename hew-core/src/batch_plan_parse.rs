//! Extract a `next_iteration` batch suggestion from an iter agent's
//! raw close output.
//!
//! The iter agent is invited (via the close-output template) to emit a
//! list of task ids it believes can run in parallel on the next iter.
//! Two block forms are accepted:
//!
//! 1. Fenced code block tagged `next_iteration` carrying a JSON array:
//!
//!    ```text
//!    ```next_iteration
//!    ["hew-aaa", "hew-bbb"]
//!    ```
//!    ```
//!
//! 2. XML-style tag with CSV body: `<next_iteration>hew-aaa, hew-bbb</next_iteration>`.
//!
//! The parser is **best-effort**: malformed input returns `None` rather
//! than erroring, so a noisy iter never fails the loop. Absence
//! (`None`) and an explicit empty list (`Some(vec![])`) are distinct
//! signals — empty means "the agent chose to parallelize nothing,"
//! while `None` means "the agent didn't say."
//!
//! Multiple blocks in the same text: the first one wins.

/// Extract a `next_iteration` task-id batch from raw agent close text.
///
/// Returns:
/// - `None` if no block is present, or the block's body is unparseable.
/// - `Some(vec![])` if a block is present but contains no valid task ids
///   (either an explicit empty list, or every token was rejected as
///   malformed).
/// - `Some(vec![ids...])` with whitespace stripped and each id
///   validated against `^hew-[a-z0-9]+(\.[0-9]+)*$`. Malformed tokens
///   are silently dropped; duplicates are preserved in order.
pub fn extract_next_iteration(raw_text: &str) -> Option<Vec<String>> {
    if let Some(ids) = extract_fenced(raw_text) {
        return Some(ids);
    }
    extract_xml_tag(raw_text)
}

fn extract_fenced(raw_text: &str) -> Option<Vec<String>> {
    // Locate the opening fence. Accept any number of leading backticks
    // ≥3 followed by the language tag; we just look for the canonical
    // ```next_iteration marker.
    let start = raw_text.find("```next_iteration")?;
    let after_tag = &raw_text[start + "```next_iteration".len()..];
    // Body ends at the next ``` fence.
    let end = after_tag.find("```")?;
    let body = after_tag[..end].trim();
    parse_json_array(body)
}

fn extract_xml_tag(raw_text: &str) -> Option<Vec<String>> {
    let open = "<next_iteration>";
    let close = "</next_iteration>";
    let start = raw_text.find(open)?;
    let after = &raw_text[start + open.len()..];
    let end = after.find(close)?;
    let body = after[..end].trim();
    Some(parse_csv(body))
}

fn parse_json_array(body: &str) -> Option<Vec<String>> {
    // Strip a leading `[` and trailing `]` — anything else is malformed.
    let body = body.trim();
    let inner = body.strip_prefix('[')?.strip_suffix(']')?;
    let inner = inner.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for tok in inner.split(',') {
        let t = tok.trim().trim_matches(|c: char| c == '"' || c == '\'');
        if t.is_empty() {
            continue;
        }
        if is_valid_task_id(t) {
            out.push(t.to_string());
        }
    }
    Some(out)
}

fn parse_csv(body: &str) -> Vec<String> {
    if body.is_empty() {
        return Vec::new();
    }
    body.split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty() && is_valid_task_id(t))
        .map(|t| t.to_string())
        .collect()
}

/// Validate against `^hew-[a-z0-9]+(\.[0-9]+)*$`.
fn is_valid_task_id(s: &str) -> bool {
    let rest = match s.strip_prefix("hew-") {
        Some(r) => r,
        None => return false,
    };
    if rest.is_empty() {
        return false;
    }
    // First segment: [a-z0-9]+
    let mut chars = rest.chars().peekable();
    let mut first_seg_len = 0usize;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            chars.next();
            first_seg_len += 1;
        } else {
            break;
        }
    }
    if first_seg_len == 0 {
        return false;
    }
    // Remaining: zero or more `.[0-9]+` segments.
    while let Some(&c) = chars.peek() {
        if c != '.' {
            return false;
        }
        chars.next();
        let mut seg_len = 0usize;
        while let Some(&d) = chars.peek() {
            if d.is_ascii_digit() {
                chars.next();
                seg_len += 1;
            } else {
                break;
            }
        }
        if seg_len == 0 {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_returns_none_when_no_block() {
        assert_eq!(extract_next_iteration("just some close text\nno batch here"), None);
        assert_eq!(extract_next_iteration(""), None);
    }

    #[test]
    fn extract_returns_empty_vec_when_block_is_empty_array() {
        let fenced = "blah\n```next_iteration\n[]\n```\n";
        assert_eq!(extract_next_iteration(fenced), Some(vec![]));
        let xml = "<next_iteration></next_iteration>";
        assert_eq!(extract_next_iteration(xml), Some(vec![]));
    }

    #[test]
    fn extract_parses_fenced_json_array_form() {
        let raw = "closing notes\n\n```next_iteration\n[\"hew-aaa\", \"hew-bbb\"]\n```\n";
        assert_eq!(extract_next_iteration(raw), Some(vec!["hew-aaa".into(), "hew-bbb".into()]));
    }

    #[test]
    fn extract_parses_xml_tag_form() {
        let raw = "done.\n<next_iteration>hew-aaa, hew-bbb, hew-ccc</next_iteration>\n";
        assert_eq!(
            extract_next_iteration(raw),
            Some(vec!["hew-aaa".into(), "hew-bbb".into(), "hew-ccc".into()])
        );
    }

    #[test]
    fn extract_filters_malformed_task_ids() {
        let raw = "<next_iteration>hew-aaa, not-a-task, HEW-BBB, hew-ccc</next_iteration>";
        assert_eq!(extract_next_iteration(raw), Some(vec!["hew-aaa".into(), "hew-ccc".into()]));
    }

    #[test]
    fn extract_first_block_wins_on_duplicates() {
        // Fenced form takes priority over XML form too.
        let raw = "\
```next_iteration
[\"hew-aaa\"]
```

<next_iteration>hew-zzz</next_iteration>
";
        assert_eq!(extract_next_iteration(raw), Some(vec!["hew-aaa".into()]));

        // Two fenced blocks: first wins.
        let raw2 = "\
```next_iteration
[\"hew-first\"]
```

```next_iteration
[\"hew-second\"]
```
";
        assert_eq!(extract_next_iteration(raw2), Some(vec!["hew-first".into()]));
    }

    #[test]
    fn extract_tolerates_leading_trailing_whitespace() {
        let raw = "```next_iteration\n   [  \"hew-aaa\"  ,  \"hew-bbb\"  ]   \n```";
        assert_eq!(extract_next_iteration(raw), Some(vec!["hew-aaa".into(), "hew-bbb".into()]));
        let raw_xml = "<next_iteration>   hew-aaa  ,  hew-bbb   </next_iteration>";
        assert_eq!(extract_next_iteration(raw_xml), Some(vec!["hew-aaa".into(), "hew-bbb".into()]));
    }

    #[test]
    fn extract_handles_realistic_agent_close_output() {
        let mut raw = String::from("Closing hew-7klt. Implemented batch_plan_parse module.\n\n");
        for i in 0..400 {
            raw.push_str(&format!("Line {i}: some debug output here that nobody reads.\n"));
        }
        raw.push_str("\n## Suggested next iteration\n\n");
        raw.push_str("```next_iteration\n");
        raw.push_str("[\"hew-pxw9\", \"hew-rplg\", \"hew-7k1m.1\"]\n");
        raw.push_str("```\n\n");
        for i in 0..100 {
            raw.push_str(&format!("Trailing line {i}\n"));
        }
        assert_eq!(
            extract_next_iteration(&raw),
            Some(vec!["hew-pxw9".into(), "hew-rplg".into(), "hew-7k1m.1".into()])
        );
    }

    #[test]
    fn extract_subtask_dotted_ids_validated() {
        let raw = "<next_iteration>hew-a3f8.1, hew-a3f8.2.3, hew-bad., hew-.1</next_iteration>";
        assert_eq!(
            extract_next_iteration(raw),
            Some(vec!["hew-a3f8.1".into(), "hew-a3f8.2.3".into()])
        );
    }

    #[test]
    fn extract_returns_none_on_unparseable_fenced_body() {
        // Fenced block present but body isn't a JSON-ish array → reject.
        let raw = "```next_iteration\nnot an array\n```";
        assert_eq!(extract_next_iteration(raw), None);
    }

    #[test]
    fn extract_never_panics_on_adversarial_input() {
        // Deterministic LCG over a small set of bytes (control chars,
        // brackets, fence-ish patterns). Goal: confirm absence of panic
        // across many random shapes, not full coverage.
        let alphabet: &[u8] = b" \n\t<>`/[]\",hew-iteration_nxX0123456789.{}\\";
        let mut state: u64 = 0xC0FFEE;
        for _ in 0..1000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let len = (state as usize % 400) + 1;
            let mut buf = String::with_capacity(len);
            for i in 0..len {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let b = alphabet[((state >> 17) as usize + i) % alphabet.len()];
                buf.push(b as char);
            }
            // Must return Some or None without panicking.
            let _ = extract_next_iteration(&buf);
        }
    }

    #[test]
    fn extract_returns_none_when_fence_unclosed() {
        let raw = "```next_iteration\n[\"hew-aaa\"]\n";
        assert_eq!(extract_next_iteration(raw), None);
    }

    #[test]
    fn extract_returns_none_when_xml_unclosed() {
        let raw = "<next_iteration>hew-aaa";
        assert_eq!(extract_next_iteration(raw), None);
    }
}
