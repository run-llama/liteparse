//! Text of "today" date fields in PowerPoint's US English formats ([MS-PPT]
//! DateTimeMCAtom index and HeadersFootersAtom formatId, 0..=12). Times are UTC.
//! The short date has a two-digit year, as PowerPoint for Mac writes it.

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
];

pub fn system_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// (year, month 1..=12, day 1..=31) of a day count since 1970-01-01 (proleptic
/// Gregorian; H. Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, usize, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month as usize, day)
}

/// The text of a date field showing `now` (seconds since the epoch) in `format`.
/// Unknown formats use the short date (format 0).
pub fn format(now: i64, format: u8) -> String {
    let days = now.div_euclid(86_400);
    let secs = now.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    let (hh, mm, ss) = (secs / 3600, secs / 60 % 60, secs % 60);
    let month = MONTHS[m - 1];
    let short = format!("{m}/{d}/{:02}", y % 100);
    let h12 = if hh % 12 == 0 { 12 } else { hh % 12 };
    let ampm = if hh < 12 { "AM" } else { "PM" };
    match format {
        1 => format!(
            "{}, {month} {d:02}, {y}",
            WEEKDAYS[days.rem_euclid(7) as usize]
        ),
        2 => format!("{d:02} {month} {y}"),
        3 => format!("{month} {d:02}, {y}"),
        4 => format!("{d:02}-{}-{:02}", &month[..3], y % 100),
        5 => format!("{month} {:02}", y % 100),
        6 => format!("{}-{:02}", &month[..3], y % 100),
        7 => format!("{short} {h12}:{mm:02} {ampm}"),
        8 => format!("{short} {h12}:{mm:02}:{ss:02} {ampm}"),
        9 => format!("{hh:02}:{mm:02}"),
        10 => format!("{hh:02}:{mm:02}:{ss:02}"),
        11 => format!("{h12}:{mm:02} {ampm}"),
        12 => format!("{h12}:{mm:02}:{ss:02} {ampm}"),
        _ => short,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_follow_the_powerpoint_date_list() {
        // 2026-10-28 18:02:13 UTC, a Wednesday.
        let t = 1_793_210_533;
        assert_eq!(format(t, 0), "10/28/26");
        assert_eq!(format(t, 1), "Wednesday, October 28, 2026");
        assert_eq!(format(t, 4), "28-Oct-26");
        assert_eq!(format(t, 8), "10/28/26 6:02:13 PM");
        assert_eq!(format(t, 9), "18:02");
        assert_eq!(format(t, 200), "10/28/26");
    }

    #[test]
    fn civil_dates_cross_leap_days_and_the_epoch() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(11_016), (2000, 2, 29));
        assert_eq!(civil(11_017), (2000, 3, 1));
    }
}
