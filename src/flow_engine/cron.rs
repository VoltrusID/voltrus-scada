//! Five-field cron expressions for the `schedule` trigger, matched against the
//! server's local time.
//!
//! `minute hour day-of-month month day-of-week`, each field `*`, a number, a
//! range `a-b`, a step `*/n`, `a-b/n` or `a/n`, or a comma list of those.
//! Months and weekdays also take three-letter names (`jan`, `mon`); weekday 0
//! and 7 are Sunday. When both day fields are restricted a day matches either
//! (classic cron); as in Vixie cron a field that starts with `*` (`*/2`)
//! counts as unrestricted, so `0 0 */2 * mon` runs on odd days that are
//! Mondays. `@hourly`, `@daily`/`@midnight`, `@weekly`, `@monthly` and
//! `@yearly`/`@annually` are shorthands.
//!
//! Daylight saving time (see `Cron::due`) follows Vixie cron: a job with a
//! fixed minute and hour runs once when the clock goes back and repeats an
//! hour, and runs at the change when the clock goes forward over its time.
//! A job with `*` in the minute or hour field just follows the wall clock.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    days_any: bool,
    weekdays_any: bool,
    /// Neither minute nor hour starts with `*`: Vixie cron's "fixed-time" job.
    fixed_time: bool,
}

/// The local wall-clock fields a cron expression is matched against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTime {
    pub minute: u8,
    pub hour: u8,
    /// 1-31
    pub day: u8,
    /// 1-12
    pub month: u8,
    /// 0 = Sunday
    pub weekday: u8,
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

impl Cron {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let expr = expr.trim();
        let expanded = match expr.to_ascii_lowercase().as_str() {
            "@hourly" => "0 * * * *".to_string(),
            "@daily" | "@midnight" => "0 0 * * *".to_string(),
            "@weekly" => "0 0 * * 0".to_string(),
            "@monthly" => "0 0 1 * *".to_string(),
            "@yearly" | "@annually" => "0 0 1 1 *".to_string(),
            other if other.starts_with('@') => {
                return Err(format!("unknown cron shorthand '{expr}'"));
            }
            _ => expr.to_string(),
        };
        let fields: Vec<&str> = expanded.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!(
                "a cron expression has 5 fields (minute hour day month weekday), '{expr}' has {}",
                fields.len()
            ));
        }
        let minutes = parse_field(fields[0], 0, 59, &[], "minute")?;
        let hours = parse_field(fields[1], 0, 23, &[], "hour")?;
        let days = parse_field(fields[2], 1, 31, &[], "day of month")?;
        let months = parse_field(fields[3], 1, 12, &MONTHS, "month")?;
        let mut weekdays = parse_field(fields[4], 0, 7, &WEEKDAYS, "day of week")?;
        if weekdays & (1 << 7) != 0 {
            weekdays = (weekdays & !(1 << 7)) | 1;
        }
        Ok(Self {
            minutes,
            hours: hours as u32,
            days: days as u32,
            months: months as u16,
            weekdays: weekdays as u8,
            days_any: fields[2].starts_with('*'),
            weekdays_any: fields[4].starts_with('*'),
            fixed_time: !fields[0].starts_with('*') && !fields[1].starts_with('*'),
        })
    }

    pub fn matches(&self, t: LocalTime) -> bool {
        let bit = |mask: u64, v: u8| mask & (1u64 << v) != 0;
        if !bit(self.minutes, t.minute)
            || !bit(self.hours as u64, t.hour)
            || !bit(self.months as u64, t.month)
        {
            return false;
        }
        let day = bit(self.days as u64, t.day);
        let weekday = bit(self.weekdays as u64, t.weekday);
        // Vixie cron: with either day field starred both must match (a
        // plain `*` matches every day anyway); with both restricted, either.
        if self.days_any || self.weekdays_any {
            day && weekday
        } else {
            day || weekday
        }
    }
}

/// Clock changes further than this are not daylight saving time (a clock
/// set by hand, a zone change) and are not caught up or deduplicated.
const MAX_DST_SHIFT_SECS: i64 = 3 * 3600;

impl Cron {
    /// Whether the schedule fires in the UTC minute containing `unix_secs`,
    /// in the server's local zone.
    pub fn due(&self, unix_secs: i64) -> bool {
        self.due_with(unix_secs, local_offset_secs)
    }

    /// `due` with the zone given as a function from a UTC instant to its
    /// offset in seconds.
    pub fn due_with(&self, unix_secs: i64, offset_at: impl Fn(i64) -> i64) -> bool {
        let minute = unix_secs.div_euclid(60) * 60;
        let offset = offset_at(minute);
        if self.matches(fields_at(minute, offset)) {
            if !self.fixed_time {
                return true;
            }
            // Clock went back: this wall-clock minute already happened under
            // the earlier, larger offset; a fixed-time job ran then.
            let before = offset_at(minute - MAX_DST_SHIFT_SECS);
            if before > offset {
                let first = minute + offset - before;
                if first < minute && offset_at(first) == before {
                    return false;
                }
            }
            return true;
        }
        // Clock went forward at the start of this minute: a fixed-time job
        // whose time fell in the skipped wall-clock minutes runs now.
        let previous = offset_at(minute - 60);
        let jump = offset - previous;
        if self.fixed_time && jump > 0 && jump <= MAX_DST_SHIFT_SECS {
            let mut skipped = minute - 60 + previous + 60;
            while skipped < minute + offset {
                if self.matches(fields_at(skipped, 0)) {
                    return true;
                }
                skipped += 60;
            }
        }
        false
    }
}

fn parse_value(s: &str, names: &[&str], what: &str) -> Result<u32, String> {
    if let Ok(n) = s.parse::<u32>() {
        return Ok(n);
    }
    let lower = s.to_ascii_lowercase();
    names
        .iter()
        .position(|n| *n == lower)
        .map(|i| i as u32 + u32::from(names.len() == 12))
        .ok_or_else(|| format!("'{s}' is not a valid {what}"))
}

fn parse_field(field: &str, min: u32, max: u32, names: &[&str], what: &str) -> Result<u64, String> {
    let mut mask = 0u64;
    for part in field.split(',') {
        if part.is_empty() {
            return Err(format!("empty entry in the {what} field '{field}'"));
        }
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s
                    .parse()
                    .map_err(|_| format!("'{s}' is not a valid step in the {what} field"))?;
                if step == 0 {
                    return Err(format!("step 0 in the {what} field"));
                }
                (r, step)
            }
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (parse_value(a, names, what)?, parse_value(b, names, what)?)
        } else {
            let v = parse_value(range, names, what)?;
            // `a/n` runs from a to the end of the field.
            (v, if part.contains('/') { max } else { v })
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!(
                "{what} '{range}' is outside {min}-{max} or reversed"
            ));
        }
        let mut v = lo;
        while v <= hi {
            mask |= 1 << v;
            v += step;
        }
    }
    Ok(mask)
}

/// The server's UTC offset in seconds at `unix_secs`.
#[cfg(unix)]
fn local_offset_secs(unix_secs: i64) -> i64 {
    // `time` refuses to read the local offset once the process has threads;
    // localtime_r is thread-safe and reads the same zone data.
    let t = unix_secs as libc::time_t;
    // SAFETY: localtime_r only writes the `tm` we own; a zeroed `tm` is valid.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff as i64
        }
    }
}

#[cfg(not(unix))]
fn local_offset_secs(_unix_secs: i64) -> i64 {
    time::UtcOffset::current_local_offset()
        .map(|o| o.whole_seconds() as i64)
        .unwrap_or(0)
}

/// Wall-clock fields at `unix_secs` shifted by `offset_secs`.
pub fn fields_at(unix_secs: i64, offset_secs: i64) -> LocalTime {
    let local = time::OffsetDateTime::from_unix_timestamp(unix_secs + offset_secs)
        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    LocalTime {
        minute: local.minute(),
        hour: local.hour(),
        day: local.day(),
        month: u8::from(local.month()),
        weekday: local.weekday().number_days_from_sunday(),
    }
}

/// Local wall-clock fields at `unix_secs`.
pub fn local_time(unix_secs: i64) -> LocalTime {
    fields_at(unix_secs, local_offset_secs(unix_secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(minute: u8, hour: u8, day: u8, month: u8, weekday: u8) -> LocalTime {
        LocalTime {
            minute,
            hour,
            day,
            month,
            weekday,
        }
    }

    #[test]
    fn every_minute() {
        let c = Cron::parse("* * * * *").unwrap();
        assert!(c.matches(t(0, 0, 1, 1, 0)));
        assert!(c.matches(t(59, 23, 31, 12, 6)));
    }

    #[test]
    fn steps_ranges_and_lists() {
        let c = Cron::parse("*/15 8-17 * * mon-fri").unwrap();
        assert!(c.matches(t(0, 8, 3, 6, 1)));
        assert!(c.matches(t(45, 17, 3, 6, 5)));
        assert!(!c.matches(t(10, 8, 3, 6, 1)));
        assert!(!c.matches(t(0, 18, 3, 6, 1)));
        assert!(!c.matches(t(0, 9, 3, 6, 0)), "sunday");

        let c = Cron::parse("5,35 0/6 * * *").unwrap();
        assert!(c.matches(t(35, 18, 1, 1, 0)));
        assert!(!c.matches(t(35, 19, 1, 1, 0)));

        let c = Cron::parse("0 0 1-10/3 * *").unwrap();
        for d in [1, 4, 7, 10] {
            assert!(c.matches(t(0, 0, d, 2, 3)), "day {d}");
        }
        assert!(!c.matches(t(0, 0, 2, 2, 3)));
    }

    #[test]
    fn names_and_sunday_as_7() {
        let c = Cron::parse("0 12 * jul sun").unwrap();
        assert!(c.matches(t(0, 12, 6, 7, 0)));
        assert!(!c.matches(t(0, 12, 6, 8, 0)));
        let c = Cron::parse("0 12 * * 7").unwrap();
        assert!(c.matches(t(0, 12, 6, 7, 0)));
    }

    #[test]
    fn day_fields_are_or_when_both_restricted() {
        let c = Cron::parse("0 0 13 * fri").unwrap();
        assert!(c.matches(t(0, 0, 13, 5, 2)), "the 13th");
        assert!(c.matches(t(0, 0, 9, 5, 5)), "a friday");
        assert!(!c.matches(t(0, 0, 9, 5, 4)));
    }

    #[test]
    fn shorthands() {
        assert_eq!(
            Cron::parse("@daily").unwrap(),
            Cron::parse("0 0 * * *").unwrap()
        );
        assert!(Cron::parse("@hourly").unwrap().matches(t(0, 7, 2, 3, 4)));
        assert!(Cron::parse("@sometimes").is_err());
    }

    #[test]
    fn invalid_expressions() {
        for bad in [
            "",
            "* * * *",
            "* * * * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "x * * * *",
            "1,,2 * * * *",
            "* * * foo *",
        ] {
            assert!(Cron::parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn fields_at_applies_the_offset() {
        // 2024-01-01 00:30:00 UTC is a Monday; at +07:00 it is 07:30.
        let ts = 1_704_069_000;
        assert_eq!(fields_at(ts, 0), t(30, 0, 1, 1, 1));
        assert_eq!(fields_at(ts, 7 * 3600), t(30, 7, 1, 1, 1));
        // At -01:00 it is still Sunday 31 December.
        assert_eq!(fields_at(ts, -3600), t(30, 23, 31, 12, 0));
    }

    #[test]
    fn a_starred_step_in_a_day_field_counts_as_unrestricted() {
        // Vixie: `*/2` in day of month is "*", so both day fields must match.
        let c = Cron::parse("0 0 */2 * mon").unwrap();
        assert!(c.matches(t(0, 0, 3, 6, 1)), "odd day and a monday");
        assert!(!c.matches(t(0, 0, 4, 6, 1)), "a monday on an even day");
        assert!(
            !c.matches(t(0, 0, 3, 6, 2)),
            "an odd day that is not monday"
        );
        // Both restricted without a star: either matches.
        let c = Cron::parse("0 0 1-31/2 * mon").unwrap();
        assert!(c.matches(t(0, 0, 4, 6, 1)));
    }

    // A zone at +01:00 that moves to +02:00 at SPRING (01:00 UTC, 02:00 local
    // becomes 03:00) and back to +01:00 at FALL (01:00 UTC, 03:00 local
    // becomes 02:00), like central Europe. 2024-03-31 and 2024-10-27.
    const SPRING: i64 = 1_711_846_800;
    const FALL: i64 = 1_729_990_800;
    fn europe(ts: i64) -> i64 {
        if (SPRING..FALL).contains(&ts) {
            7200
        } else {
            3600
        }
    }

    fn fires(c: &Cron, from: i64, to: i64) -> Vec<i64> {
        (from / 60..to / 60)
            .map(|m| m * 60)
            .filter(|ts| c.due_with(*ts, europe))
            .collect()
    }

    #[test]
    fn a_fixed_time_job_runs_once_when_the_clock_goes_back() {
        // 02:30 local happens at 00:30 UTC (+02:00) and again at 01:30 UTC.
        let c = Cron::parse("30 2 * * *").unwrap();
        let runs = fires(&c, FALL - 3 * 3600, FALL + 3 * 3600);
        assert_eq!(runs, vec![FALL - 1800]);
    }

    #[test]
    fn a_wildcard_job_follows_the_wall_clock_when_it_goes_back() {
        let c = Cron::parse("30 * * * *").unwrap();
        let runs = fires(&c, FALL - 3 * 3600, FALL + 3 * 3600);
        // Every hour, including both 02:30s.
        assert_eq!(runs.len(), 6);
    }

    #[test]
    fn a_fixed_time_job_in_the_skipped_hour_runs_at_the_change() {
        // 02:30 local does not exist on the spring day: it runs at 03:00.
        let c = Cron::parse("30 2 * * *").unwrap();
        let runs = fires(&c, SPRING - 3 * 3600, SPRING + 3 * 3600);
        assert_eq!(runs, vec![SPRING]);
        // A job at 03:00 runs once, at 03:00, not twice.
        let c = Cron::parse("0 3 * * *").unwrap();
        assert_eq!(
            fires(&c, SPRING - 3 * 3600, SPRING + 3 * 3600),
            vec![SPRING]
        );
        // An hourly job runs every real hour (local 02:30 just does not
        // exist), and is not run a second time at the change.
        let c = Cron::parse("30 * * * *").unwrap();
        assert_eq!(fires(&c, SPRING - 3 * 3600, SPRING + 3 * 3600).len(), 6);
    }

    #[test]
    fn without_a_clock_change_due_is_matches() {
        let c = Cron::parse("15 10 * * *").unwrap();
        let ts = 1_704_104_100; // 2024-01-01 10:15 UTC
        assert!(c.due_with(ts + 59, |_| 0));
        assert!(!c.due_with(ts + 60, |_| 0));
    }
}
