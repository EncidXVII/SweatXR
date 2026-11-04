//! Unix seconds, local days and ISO 8601. Days are counted in local time so a session at
//! 23:59 lands on the day the user lived it.

use std::time::{SystemTime, UNIX_EPOCH};

/// Days since 1970-01-01 in local time.
pub type Day = i32;

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Offset of local time from UTC at `ts`. Falls back to UTC if the CRT cannot tell.
pub fn local_offset(ts: i64) -> i32 {
    crate::ffi::local_offset(ts).unwrap_or(0)
}

pub fn day_of(ts: i64, offset: i32) -> Day {
    (ts + i64::from(offset)).div_euclid(86_400) as Day
}

pub fn local_day(ts: i64) -> Day {
    day_of(ts, local_offset(ts))
}

/// Monday = 0.
pub fn weekday(day: Day) -> u32 {
    (day + 3).rem_euclid(7) as u32
}

/// (year, month 1-12, day 1-31), from H. Hinnant's public-domain date algorithms.
pub fn civil(day: Day) -> (i32, u32, u32) {
    let z = i64::from(day) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m as u32, d as u32)
}

pub fn days_from_civil(y: i32, m: u32, d: u32) -> Day {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * if m > 2 { m - 3 } else { m + 9 } + 2) / 5 + i64::from(d) - 1;
    (era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468) as Day
}

pub fn iso_utc(ts: i64) -> String {
    let (y, mo, d) = civil(ts.div_euclid(86_400) as Day);
    let s = ts.rem_euclid(86_400);
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

pub fn parse_iso_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[19] != b'Z' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<u32>().ok();
    let day = days_from_civil(n(0..4)? as i32, n(5..7)?, n(8..10)?);
    Some(i64::from(day) * 86_400 + i64::from(n(11..13)? * 3600 + n(14..16)? * 60 + n(17..19)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips() {
        for day in [-1, 0, 11_016, 20_000, 20_396, 25_000] {
            let (y, m, d) = civil(day);
            assert_eq!(days_from_civil(y, m, d), day);
        }
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(weekday(0), 3, "1970-01-01 was a Thursday");
    }

    #[test]
    fn iso_round_trips() {
        let ts = i64::from(days_from_civil(2026, 11, 5)) * 86_400 + 3_723;
        assert_eq!(iso_utc(ts), "2026-11-05T01:02:03Z");
        assert_eq!(parse_iso_utc(&iso_utc(ts)), Some(ts));
    }

    #[test]
    fn day_boundary_follows_offset() {
        // 2026-03-29 00:30 UTC: still the 28th in New York (-4 h), already the 29th in Berlin (+2 h).
        let ts = i64::from(days_from_civil(2026, 3, 29)) * 86_400 + 1_800;
        assert_eq!(civil(day_of(ts, -4 * 3600)), (2026, 3, 28));
        assert_eq!(civil(day_of(ts, 2 * 3600)), (2026, 3, 29));
    }

    #[test]
    fn local_offset_is_sane() {
        let off = local_offset(now_unix());
        assert!((-14 * 3600..=14 * 3600).contains(&off));
    }
}
