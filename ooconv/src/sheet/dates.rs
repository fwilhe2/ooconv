// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Excel's two date systems against ODF's serials, and ISO 8601 for `office:date-value`. The
//! 1900-system facts are grind's (`xlsx/src/dates.rs`, measured against the oracle):
//!
//! | Excel serial | Excel means | model (ODF) serial |
//! |---|---|---|
//! | 0 ≤ s < 60 | 1900-01-00 … 1900-02-28 | s + 1 |
//! | 60 | 1900-02-29, which does not exist | 60 |
//! | s ≥ 61 | 1900-03-01 onwards | unchanged |
//!
//! Only a *date* is corrected; a time is a duration and has no epoch. The 1904 system is
//! 1462 days later than the model's epoch.

/// Days from 1899-12-30 to 1904-01-01.
pub const DAYS_1904: f64 = 1462.0;

/// An Excel 1900-system date serial as a model serial.
pub fn from_excel_1900(serial: f64) -> f64 {
    if (0.0..60.0).contains(&serial) {
        serial + 1.0
    } else {
        serial
    }
}

/// A model serial as an Excel 1900-system date serial — the inverse, where one exists. Model
/// serial 60 is 1900-02-28 and goes to 59, the same day; below 1 there is no Excel date.
pub fn to_excel_1900(serial: f64) -> f64 {
    if (1.0..61.0).contains(&serial) {
        serial - 1.0
    } else {
        serial
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 1899-12-30 as days since 1970-01-01.
const EPOCH: i64 = -25_569;

/// A model serial as `office:date-value`: `YYYY-MM-DD`, with `THH:MM:SS(.fff)` when it has a
/// time of day. Rounded to the millisecond, which is finer than either application shows.
pub fn to_iso(serial: f64) -> String {
    let ms_total = (serial * 86_400_000.0).round() as i64;
    let days = ms_total.div_euclid(86_400_000);
    let ms = ms_total.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days + EPOCH);
    let date = format!("{y:04}-{m:02}-{d:02}");
    if ms == 0 {
        return date;
    }
    let (h, rest) = (ms / 3_600_000, ms % 3_600_000);
    let (min, rest) = (rest / 60_000, rest % 60_000);
    let (s, frac) = (rest / 1000, rest % 1000);
    match frac {
        0 => format!("{date}T{h:02}:{min:02}:{s:02}"),
        f => format!("{date}T{h:02}:{min:02}:{s:02}.{f:03}"),
    }
}

/// `office:date-value` → a model serial. Accepts a date, a date-time, fractional seconds and a
/// trailing zone designator, which is ignored (a cell has no zone).
pub fn from_iso(s: &str) -> Option<f64> {
    let s = s.trim();
    let (date, time) = match s.split_once('T') {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let negative = date.starts_with('-');
    let mut parts = date.trim_start_matches('-').splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let y = if negative { -y } else { y };
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut serial = (days_from_civil(y, m, d) - EPOCH) as f64;
    if let Some(t) = time {
        let t = t.trim_end_matches('Z');
        let t = t.split(['+']).next()?;
        let t = match t.rfind('-') {
            Some(i) if i > 5 => &t[..i],
            _ => t,
        };
        let mut hms = t.split(':');
        let h: f64 = hms.next()?.parse().ok()?;
        let min: f64 = hms.next().unwrap_or("0").parse().ok()?;
        let sec: f64 = hms.next().unwrap_or("0").parse().ok()?;
        serial += (h * 3600.0 + min * 60.0 + sec) / 86_400.0;
    }
    Some(serial)
}

/// A duration in days as `office:time-value`: `PT36H00M00S`.
pub fn to_duration(days: f64) -> String {
    let negative = days < 0.0;
    let ms_total = (days.abs() * 86_400_000.0).round() as i64;
    let (h, rest) = (ms_total / 3_600_000, ms_total % 3_600_000);
    let (m, rest) = (rest / 60_000, rest % 60_000);
    let (s, frac) = (rest / 1000, rest % 1000);
    let sign = if negative { "-" } else { "" };
    match frac {
        0 => format!("{sign}PT{h:02}H{m:02}M{s:02}S"),
        f => format!("{sign}PT{h:02}H{m:02}M{s:02}.{f:03}S"),
    }
}

/// `office:time-value` (an XSD duration) → days. Years and months have no fixed length and are
/// refused; LibreOffice writes only `PnDTnHnMnS`.
pub fn from_duration(s: &str) -> Option<f64> {
    let s = s.trim();
    let (negative, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let s = s.strip_prefix('P')?;
    let (date, time) = s.split_once('T').unwrap_or((s, ""));
    let mut days = 0.0;
    let mut num = String::new();
    for c in date.chars() {
        match c {
            'D' => days += std::mem::take(&mut num).parse::<f64>().ok()?,
            c if c.is_ascii_digit() || c == '.' => num.push(c),
            _ => return None,
        }
    }
    for c in time.chars() {
        let unit = match c {
            'H' => 1.0 / 24.0,
            'M' => 1.0 / 1440.0,
            'S' => 1.0 / 86_400.0,
            c if c.is_ascii_digit() || c == '.' => {
                num.push(c);
                continue;
            }
            _ => return None,
        };
        days += std::mem::take(&mut num).parse::<f64>().ok()? * unit;
    }
    num.is_empty()
        .then_some(if negative { -days } else { days })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boundary every implementation gets wrong once: 59, 60 and 61.
    #[test]
    fn the_phantom_day_and_both_sides_of_it() {
        assert_eq!(to_iso(from_excel_1900(1.0)), "1900-01-01");
        assert_eq!(to_iso(from_excel_1900(59.0)), "1900-02-28");
        assert_eq!(to_iso(from_excel_1900(60.0)), "1900-02-28");
        assert_eq!(to_iso(from_excel_1900(61.0)), "1900-03-01");
        assert_eq!(to_iso(from_excel_1900(25569.0)), "1970-01-01");
        assert_eq!(to_iso(from_excel_1900(2958465.0)), "9999-12-31");
        for s in [1.0, 59.0, 61.0, 45368.75] {
            assert_eq!(to_excel_1900(from_excel_1900(s)), s);
        }
    }

    #[test]
    fn iso_round_trips_with_time_of_day() {
        for (serial, iso) in [
            (46336.0, "2026-11-10"),
            (46336.5, "2026-11-10T12:00:00"),
            (0.0, "1899-12-30"),
            (-1.0, "1899-12-29"),
            (46336.0 + 1.5 / 86400.0, "2026-11-10T00:00:01.500"),
        ] {
            assert_eq!(to_iso(serial), iso);
            assert!((from_iso(iso).unwrap() - serial).abs() < 1e-9, "{iso}");
        }
        assert!(from_iso("2026-11-10T12:00:00Z").is_some());
        assert!(from_iso("2026-13-01").is_none());
    }

    #[test]
    fn durations_both_ways() {
        assert_eq!(to_duration(1.5), "PT36H00M00S");
        assert_eq!(from_duration("PT36H00M00S"), Some(1.5));
        assert_eq!(from_duration("P1DT12H"), Some(1.5));
        assert_eq!(from_duration("-PT12H"), Some(-0.5));
        assert_eq!(from_duration("P1Y"), None);
    }
}
