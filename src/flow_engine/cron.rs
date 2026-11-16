//! Five-field cron expressions for the `schedule` trigger, matched against the
//! server's local time.
//!
//! `minute hour day-of-month month day-of-week`, each field `*`, a number, a
//! range `a-b`, a step `*/n`, `a-b/n` or `a/n`, or a comma list of those.
//! Months and weekdays also take three-letter names (`jan`, `mon`); weekday 0
//! and 7 are Sunday. When both day fields are restricted a day matches either
//! (classic cron). `@hourly`, `@daily`/`@midnight`, `@weekly`, `@monthly` and
//! `@yearly`/`@annually` are shorthands.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cron {
    minutes: u64,
    hours: u32,
    days: u32,
    months: u16,
    weekdays: u8,
    days_any: bool,
    weekdays_any: bool,
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
            days_any: fields[2] == "*",
            weekdays_any: fields[4] == "*",
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
        match (self.days_any, self.weekdays_any) {
            (true, true) => true,
            (false, true) => day,
            (true, false) => weekday,
            (false, false) => day || weekday,
        }
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
}
