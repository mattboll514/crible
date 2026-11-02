// SPDX-License-Identifier: Apache-2.0

//! ISO 8601 dates read into UTC, and UTC rendered as the strings SQLite stores.
//!
//! References: ADR-0122, decision 9 (a bound of a read is an ISO 8601 date with
//! an offset, both bounds are included, and the comparison of the file is a
//! literal comparison on the stored strings) and ADR-0124, decision 2 (the same
//! rule for `GET /v1/stats`, at the resolution of a minute).
//!
//! The module owns no clock: it reads the instant a caller names and renders it
//! in the canonical form the analytics, the jobs and the debug journal store.
//! The Gregorian arithmetic is Howard Hinnant's `days_from_civil` and
//! `civil_from_days`, which need no crate (`docs/07`, ADR-0124, decision 14).

use crate::error::{CribleError, ErrorCode};

/// An instant read from an ISO 8601 date, at millisecond resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcInstant {
    /// Whole seconds since `1970-01-01T00:00:00Z`.
    pub seconds: i64,
    /// Milliseconds of that second, from 0 to 999.
    pub millis: u32,
}

impl UtcInstant {
    /// The same instant floored to its minute, milliseconds dropped.
    ///
    /// A row of `minute_events` is an aggregate of one whole minute, so a bound
    /// is read at that resolution: `23:59:30` and `23:59:45` select the same
    /// minute, and its events are counted once (ADR-0124, decision 2).
    #[must_use]
    pub fn floor_to_minute(self) -> Self {
        Self {
            seconds: self.seconds - self.seconds.rem_euclid(60),
            millis: 0,
        }
    }
}

/// Reads an ISO 8601 date, applies its offset and returns the instant in UTC.
///
/// The date may carry the `Z` suffix or a `±HH:MM` offset; a calendar day that
/// does not exist, a leap second and a malformed offset are refused.
///
/// # Errors
///
/// [`ErrorCode::InvalidRequest`] with a message naming `parameter` when `raw`
/// is not an ISO 8601 date this engine reads (ADR-0122, decision 9).
pub fn parse_iso8601(raw: &str, parameter: &str) -> Result<UtcInstant, CribleError> {
    read(raw).ok_or_else(|| refused(parameter))
}

/// Renders an instant as `YYYY-MM-DDTHH:MM:SS.mmmZ`, the form of `created_at`.
#[must_use]
pub fn format_millis(instant: UtcInstant) -> String {
    let (year, month, day, hour, minute, second) = split(instant.seconds);
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{:03}Z",
        instant.millis
    )
}

/// Renders an instant as `YYYY-MM-DDTHH:MM:00Z`, the form of a `minute`.
#[must_use]
pub fn format_minute(instant: UtcInstant) -> String {
    let (year, month, day, hour, minute, _) = split(instant.seconds);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:00Z")
}

/// The refusal of a date, naming the parameter (ADR-0122, decision 9).
fn refused(parameter: &str) -> CribleError {
    CribleError::new(
        ErrorCode::InvalidRequest,
        format!(
            "'{parameter}' must be an ISO 8601 date, such as \
             2026-10-31T10:00:00.123Z or 2026-10-31T12:00:00+02:00"
        ),
    )
}

/// Splits an instant into its UTC calendar fields.
fn split(seconds: i64) -> (i64, u32, u32, i64, i64, i64) {
    let days = seconds.div_euclid(86_400);
    let remaining = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        remaining / 3_600,
        remaining % 3_600 / 60,
        remaining % 60,
    )
}

/// Reads one ISO 8601 date, or `None` when it is outside its grammar.
#[allow(clippy::too_many_lines)]
fn read(raw: &str) -> Option<UtcInstant> {
    let (body, offset) = split_offset(raw)?;
    let bytes = body.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let year = i64::from(ascii_digits(body, 0, 4)?);
    let month = ascii_digits(body, 5, 7)?;
    let day = ascii_digits(body, 8, 10)?;
    let hour = ascii_digits(body, 11, 13)?;
    let minute = ascii_digits(body, 14, 16)?;
    let second = ascii_digits(body, 17, 19)?;
    if !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let fraction = body.get(19..)?;
    let millis = if fraction.is_empty() {
        0
    } else {
        let digits = fraction.strip_prefix('.')?;
        if digits.is_empty()
            || digits.len() > 9
            || !digits.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let truncated = digits.get(..digits.len().min(3))?;
        let mut padded = truncated.to_owned();
        while padded.len() < 3 {
            padded.push('0');
        }
        padded.parse::<u32>().ok()?
    };
    // The offset states the local reading; UTC is that reading minus it, so a
    // date east of Greenwich is earlier in UTC (ADR-0122, decision 9).
    let local = days_from_civil(year, month, day) * 86_400
        + i64::from(hour) * 3_600
        + i64::from(minute) * 60
        + i64::from(second);
    let seconds = local - i64::from(offset);
    let days = seconds.div_euclid(86_400);
    let (year, _, _) = civil_from_days(days);
    if !(0..=9_999).contains(&year) {
        return None;
    }
    Some(UtcInstant { seconds, millis })
}

/// Splits an ISO 8601 date into its local reading and its offset in seconds east
/// of UTC: `Z` is zero, `+HH:MM` and `-HH:MM` are read.
///
/// The sign is searched after the time, so the dashes of the calendar date are
/// never read as an offset. A suffix of any other shape yields `None`.
fn split_offset(raw: &str) -> Option<(&str, i32)> {
    if let Some(body) = raw.strip_suffix('Z') {
        return Some((body, 0));
    }
    let digits = raw.get(19..)?;
    let position = 19 + digits.find(['+', '-'])?;
    let sign = match raw.as_bytes().get(position)? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    if raw.as_bytes().get(position + 3) != Some(&b':') || raw.len() != position + 6 {
        return None;
    }
    let hours = ascii_digits(raw, position + 1, position + 3)?;
    let minutes = ascii_digits(raw, position + 4, position + 6)?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    let offset = i32::try_from(hours * 3_600 + minutes * 60).ok()?;
    Some((raw.get(..position)?, sign * offset))
}

/// Number of days from 1970-01-01 to `year-month-day`, Gregorian calendar.
///
/// Howard Hinnant's `days_from_civil`, with the year starting in March so that
/// the leap day is the last day of the year. `crible-store` reads its `date32`
/// values and its `YYYY-MM-DD` filters through it, so one arithmetic serves the
/// analytics and the store (ADR-0124, decision 14).
#[must_use]
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = i64::from((month + 9) % 12);
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The Gregorian date `year-month-day` of a number of days from 1970-01-01.
///
/// The inverse of [`days_from_civil`], exact over the whole `i32` range of a
/// `date32` and independent of any platform clock or time zone.
#[must_use]
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

/// Reads the ASCII digits of `raw` in `[start, end)`.
fn ascii_digits(raw: &str, start: usize, end: usize) -> Option<u32> {
    let slice = raw.get(start..end)?;
    if !slice.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    slice.parse().ok()
}

/// Number of days of `month` in `year`, on the Gregorian calendar.
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::{UtcInstant, format_millis, format_minute, parse_iso8601};
    use crate::error::ErrorCode;

    /// The instant of an accepted date, at minute resolution.
    fn minute(raw: &str) -> String {
        let instant = parse_iso8601(raw, "from").expect("the date is accepted");
        format_minute(instant.floor_to_minute())
    }

    #[test]
    fn a_date_reads_zulu_and_a_signed_offset_as_utc() {
        // ADR-0122, decision 9: the offset is applied, and the file stores UTC.
        assert_eq!(minute("2026-10-31T12:00:00Z"), "2026-10-31T12:00:00Z");
        assert_eq!(minute("2026-10-31T12:00:00+02:00"), "2026-10-31T10:00:00Z");
        assert_eq!(minute("2026-10-31T12:00:00-05:30"), "2026-10-31T17:30:00Z");
        assert_eq!(minute("2026-01-01T01:00:00+02:00"), "2025-12-31T23:00:00Z");
    }

    #[test]
    fn a_bound_is_floored_to_its_minute() {
        // ADR-0124, decision 2: the rows count whole minutes, so a bound inside
        // a minute selects that minute.
        assert_eq!(minute("2026-10-31T12:00:30Z"), "2026-10-31T12:00:00Z");
        assert_eq!(minute("2026-10-31T12:00:59.999Z"), "2026-10-31T12:00:00Z");
        assert_eq!(
            parse_iso8601("2026-10-31T12:00:00.123Z", "to").expect("the date is accepted"),
            UtcInstant {
                seconds: parse_iso8601("2026-10-31T12:00:00Z", "to")
                    .expect("the date is accepted")
                    .seconds,
                millis: 123
            }
        );
    }

    #[test]
    fn the_millisecond_form_is_the_one_the_files_store() {
        let instant =
            parse_iso8601("2026-10-31T12:00:00.007+02:00", "from").expect("the date is accepted");
        assert_eq!(format_millis(instant), "2026-10-31T10:00:00.007Z");
    }

    #[test]
    fn a_malformed_date_names_its_parameter() {
        for raw in [
            "2026-10-31T12:00:00",
            "2026-10-31T12:00:00+2:00",
            "2026-10-31T12:00:00+24:00",
            "2026-10-31T12:00:00+02:60",
            "2026-10-31T12:00:00+0200",
            "2026-02-30T12:00:00Z",
            "",
        ] {
            let failure = parse_iso8601(raw, "to").expect_err(raw);
            assert_eq!(failure.code(), ErrorCode::InvalidRequest);
            assert!(failure.message().contains("'to'"), "{raw}: {failure:?}");
        }
    }
}
