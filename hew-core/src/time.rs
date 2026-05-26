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

/// Parse a strict `YYYY-MM-DDTHH:MM:SSZ` string back into Unix
/// seconds. Returns `None` on any deviation from that shape — this is
/// the inverse of [`iso_from_unix`], not a tolerant general parser.
pub fn parse_iso_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let parse = |start: usize, end: usize| -> Option<i64> {
        std::str::from_utf8(&b[start..end]).ok()?.parse().ok()
    };
    let year: i64 = parse(0, 4)?;
    let month: i64 = parse(5, 7)?;
    let day: i64 = parse(8, 10)?;
    let hour: i64 = parse(11, 13)?;
    let minute: i64 = parse(14, 16)?;
    let second: i64 = parse(17, 19)?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..60).contains(&second)
    {
        return None;
    }
    // Howard Hinnant's days_from_civil. Inverse of the civil_from_days
    // used by iso_from_unix above. Same Gregorian proleptic calendar.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u64;
    let m = month as u64;
    let d = day as u64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
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
    fn parse_iso_utc_round_trips_known_values() {
        assert_eq!(parse_iso_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_utc("2024-01-01T00:00:00Z"), Some(1_704_067_200));
        assert_eq!(parse_iso_utc("2024-02-29T00:00:00Z"), Some(1_709_164_800));
        // Round-trip through iso_from_unix.
        for secs in [0, 1, 1_704_067_200, 1_709_164_800, 1_900_000_000] {
            assert_eq!(parse_iso_utc(&iso_from_unix(secs)), Some(secs));
        }
    }

    #[test]
    fn parse_iso_utc_rejects_malformed() {
        assert!(parse_iso_utc("").is_none());
        assert!(parse_iso_utc("2024-01-01").is_none());
        assert!(parse_iso_utc("2024-01-01T00:00:00").is_none());
        assert!(parse_iso_utc("2024/01/01T00:00:00Z").is_none());
        assert!(parse_iso_utc("2024-13-01T00:00:00Z").is_none());
        assert!(parse_iso_utc("2024-01-32T00:00:00Z").is_none());
        assert!(parse_iso_utc("2024-01-01T25:00:00Z").is_none());
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
