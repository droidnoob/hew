//! Tiny ISO-8601 UTC formatter. Kept pure (no chrono/jiff dep) so
//! every crate can format `SystemTime::now()` the same way without
//! pulling in a calendar library for one timestamp emit.

/// Format `SystemTime::now()` as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn iso_now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso_from_unix(secs)
}

/// Convert a Unix timestamp (seconds since epoch) into an ISO-8601
/// UTC string. Algorithm: civil_from_days from Howard Hinnant's
/// public-domain date-calendar code, restated for the Gregorian
/// proleptic calendar — handles every year hew will plausibly see.
pub fn iso_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let second = sod % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y_shift = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y_shift + 1 } else { y_shift };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// True if the leading token looks like an ISO-8601 date — four ASCII
/// digits, a `-`, and at least one more ASCII digit. Used by callers
/// that need to distinguish a real `CHECKPOINT:<iso>` shape from a
/// malformed checkpoint body whose first whitespace-token is some
/// other word.
pub fn looks_like_iso_date(token: &str) -> bool {
    let b = token.as_bytes();
    b.len() >= 6
        && b[0].is_ascii_digit()
        && b[1].is_ascii_digit()
        && b[2].is_ascii_digit()
        && b[3].is_ascii_digit()
        && b[4] == b'-'
        && b[5].is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_from_unix_epoch_is_1970() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn iso_from_unix_known_value() {
        // 2024-01-01T00:00:00Z = 1_704_067_200 (well-known constant).
        assert_eq!(iso_from_unix(1_704_067_200), "2024-01-01T00:00:00Z");
        // Leap-day combo.
        assert_eq!(iso_from_unix(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn iso_now_utc_emits_yyyy_dash() {
        let s = iso_now_utc();
        // 2020-01-01 .. 2099-12-31 covers the realistic window.
        let b = s.as_bytes();
        assert_eq!(b.len(), 20, "expected YYYY-MM-DDTHH:MM:SSZ, got {s:?}");
        assert_eq!(b[4], b'-');
        assert_eq!(b[7], b'-');
        assert_eq!(b[10], b'T');
        assert_eq!(b[19], b'Z');
    }

    #[test]
    fn iso_token_recogniser() {
        assert!(looks_like_iso_date("2026-05-25T12:34:56Z"));
        assert!(looks_like_iso_date("2026-05-25"));
        assert!(looks_like_iso_date("1999-12-31T23:59:59Z"));
        assert!(!looks_like_iso_date("practice-svc-l3.2"));
        assert!(!looks_like_iso_date("checkpoint-2026"));
        assert!(!looks_like_iso_date("2026"));
        assert!(!looks_like_iso_date(""));
        assert!(!looks_like_iso_date("—"));
    }
}
