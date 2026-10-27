//! The operator's time zone: the computer's, with its daylight-saving rules.
//!
//! A fixed UTC offset read once is wrong as soon as a daylight-saving change
//! passes, and for every timestamp on the other side of one. [`LocalZone`]
//! instead answers the offset for each instant, so a time is shown on the
//! clock the wall had *then*, and a typed wall time is read with the rules in
//! force *there*.
//!
//! The zone is a cheap-to-clone handle, so views pass it where a `UtcOffset`
//! used to go.
//!
//! [`LocalZone::refresh`] picks up a change of the computer's zone while the
//! console runs. `jiff` caches the system zone for five minutes, so it reads
//! what `jiff` would read (`TZ`, then the `/etc/localtime` link) itself and
//! compares it with the last read: two cheap system calls a tick.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

const LOCALTIME: &str = "/etc/localtime";

/// A time zone with its offset rules; cheap to clone.
#[derive(Debug, Clone)]
pub struct LocalZone(Arc<Zone>);

#[derive(Debug)]
struct Zone {
    tz: TimeZone,
    /// What the zone was read from, to notice a change of it.
    source: Source,
}

#[derive(Debug)]
enum Source {
    /// Chosen by name, never re-read.
    Fixed,
    System(SystemKey),
}

/// Everything the system zone is read from.
#[derive(Debug, PartialEq, Eq)]
struct SystemKey {
    tz_env: Option<OsString>,
    localtime: Option<PathBuf>,
}

impl SystemKey {
    fn read() -> Self {
        Self {
            tz_env: std::env::var_os("TZ"),
            localtime: std::fs::read_link(LOCALTIME).ok(),
        }
    }
}

impl LocalZone {
    /// UTC, for tests.
    pub fn utc() -> Self {
        Self::new(Zone {
            tz: TimeZone::UTC,
            source: Source::Fixed,
        })
    }

    /// The computer's zone now; UTC when it cannot be told.
    pub fn system() -> Self {
        let key = SystemKey::read();
        let tz = resolve(&key);
        Self::new(Zone {
            tz,
            source: Source::System(key),
        })
    }

    /// An IANA zone by name (`America/New_York`), never re-read.
    pub fn named(name: &str) -> Option<Self> {
        let tz = TimeZone::get(name).ok()?;
        Some(Self::new(Zone {
            tz,
            source: Source::Fixed,
        }))
    }

    fn new(zone: Zone) -> Self {
        Self(Arc::new(zone))
    }

    /// The zone now in force if the computer's differs from this one, which
    /// the caller then adopts. `None` for a zone chosen by name, and while
    /// the system's has not changed.
    pub fn refresh(&self) -> Option<Self> {
        let Source::System(known) = &self.0.source else {
            return None;
        };
        let key = SystemKey::read();
        if key == *known {
            return None;
        }
        let tz = resolve(&key);
        Some(Self::new(Zone {
            tz,
            source: Source::System(key),
        }))
    }

    /// The IANA name, when the zone has one (`Asia/Jakarta`).
    pub fn name(&self) -> Option<&str> {
        self.0.tz.iana_name()
    }

    /// Seconds east of UTC at the instant (Unix seconds); 0 beyond the
    /// range zones are defined for.
    pub fn offset_secs(&self, unix_secs: i64) -> i32 {
        Timestamp::from_second(unix_secs).map_or(0, |at| self.0.tz.to_offset(at).seconds())
    }

    /// The instant as a date and time on this zone's wall clock then;
    /// `None` out of the supported range.
    pub fn at(&self, unix_secs: i64) -> Option<OffsetDateTime> {
        let at = OffsetDateTime::from_unix_timestamp(unix_secs).ok()?;
        let offset = UtcOffset::from_whole_seconds(self.offset_secs(unix_secs)).ok()?;
        Some(at.to_offset(offset))
    }

    /// Now, on this zone's wall clock.
    pub fn now(&self) -> OffsetDateTime {
        let now = OffsetDateTime::now_utc();
        self.at(now.unix_timestamp()).unwrap_or(now)
    }

    /// The instant (Unix seconds) a wall time on this zone is. A time the
    /// clock skipped (spring forward) reads as the same wall time after the
    /// change, and one it showed twice (fall back) as the first of them.
    pub fn unix_from_local(&self, local: PrimitiveDateTime) -> Option<i64> {
        let part = |value: u8| i8::try_from(value).ok();
        let wall = DateTime::new(
            i16::try_from(local.year()).ok()?,
            part(u8::from(local.month()))?,
            part(local.day())?,
            part(local.hour())?,
            part(local.minute())?,
            part(local.second())?,
            0,
        )
        .ok()?;
        let at = self.0.tz.to_ambiguous_timestamp(wall).compatible().ok()?;
        Some(at.as_second())
    }
}

/// The zone for what the system says, as `jiff` reads it: `TZ` first, then
/// `/etc/localtime`.
fn resolve(key: &SystemKey) -> TimeZone {
    if key.tz_env.is_none() {
        if let Some(tz) = key.localtime.as_deref().and_then(from_link) {
            return tz;
        }
        if let Some(tz) = std::fs::read(LOCALTIME)
            .ok()
            .and_then(|bytes| TimeZone::tzif("localtime", &bytes).ok())
        {
            return tz;
        }
    }
    // `TZ` is fixed for the process, so `jiff`'s cached answer cannot be
    // stale for it.
    TimeZone::system()
}

fn from_link(target: &Path) -> Option<TimeZone> {
    TimeZone::get(zone_name(target.to_str()?)?).ok()
}

/// The IANA name at the end of a link into a zoneinfo directory:
/// `/var/db/timezone/zoneinfo/Asia/Jakarta` is `Asia/Jakarta`.
fn zone_name(target: &str) -> Option<&str> {
    const DIRECTORY: &str = "zoneinfo/";
    let at = target.rfind(DIRECTORY)?;
    Some(&target[at + DIRECTORY.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn new_york() -> LocalZone {
        LocalZone::named("America/New_York").expect("the system has the tz database")
    }

    #[test]
    fn each_instant_reads_the_offset_it_had() {
        let zone = new_york();
        // 2026-11-01 06:00 UTC: 02:00 EDT turns back to 01:00 EST.
        let before = 1_793_512_800 - 1;
        let after = 1_793_512_800;
        assert_eq!(zone.offset_secs(before), -4 * 3600);
        assert_eq!(zone.offset_secs(after), -5 * 3600);
        assert_eq!(zone.at(before).unwrap().offset().whole_hours(), -4);
        assert_eq!(zone.at(after).unwrap().offset().whole_hours(), -5);
    }

    #[test]
    fn noon_on_both_sides_of_the_change_is_noon() {
        let zone = new_york();
        // 2026-10-31 12:00 EDT and 2026-11-02 12:00 EST.
        let alarm_edt = 1_793_462_400;
        let alarm_est = 1_793_638_800;
        for (unix, offset) in [(alarm_edt, "-04:00:00"), (alarm_est, "-05:00:00")] {
            let at = zone.at(unix).unwrap();
            assert_eq!(at.hour(), 12, "{unix}");
            assert_eq!(at.minute(), 0);
            assert_eq!(at.offset().to_string(), offset);
        }
        // Two days apart on the wall, 49 hours in fact.
        assert_eq!(alarm_est - alarm_edt, 49 * 3600);
    }

    #[test]
    fn wall_times_round_trip_either_side_of_the_change() {
        let zone = new_york();
        for wall in [
            datetime!(2026-10-31 12:00),
            datetime!(2026-11-02 12:00),
            datetime!(2026-11-01 00:30),
            datetime!(2026-11-01 03:00),
            datetime!(2026-03-08 01:59:59),
            datetime!(2026-03-08 03:00),
        ] {
            let unix = zone.unix_from_local(wall).unwrap();
            let back = zone.at(unix).unwrap();
            assert_eq!(
                PrimitiveDateTime::new(back.date(), back.time()),
                wall,
                "{wall}"
            );
        }
        assert_eq!(
            zone.unix_from_local(datetime!(2026-10-31 12:00)),
            Some(1_793_462_400)
        );
        assert_eq!(
            zone.unix_from_local(datetime!(2026-11-02 12:00)),
            Some(1_793_638_800)
        );
    }

    #[test]
    fn the_repeated_hour_is_read_as_its_first_pass() {
        let zone = new_york();
        // 01:30 happens at 05:30 UTC (EDT) and again at 06:30 UTC (EST).
        assert_eq!(
            zone.unix_from_local(datetime!(2026-11-01 01:30)),
            Some(1_793_511_000)
        );
    }

    #[test]
    fn the_skipped_hour_is_read_after_the_change() {
        let zone = new_york();
        // 02:30 never happens on 2026-03-08; the clock goes 01:59:59 to 03:00.
        let unix = zone
            .unix_from_local(datetime!(2026-03-08 02:30))
            .expect("a skipped time still reads");
        let at = zone.at(unix).unwrap();
        assert_eq!((at.hour(), at.minute()), (3, 30));
        assert_eq!(at.offset().whole_hours(), -4);
    }

    #[test]
    fn a_night_shift_across_the_change_is_nine_or_seven_hours() {
        let zone = new_york();
        let window =
            |from, to| zone.unix_from_local(to).unwrap() - zone.unix_from_local(from).unwrap();
        // 22:00 to 06:00 as the wall shows it.
        assert_eq!(
            window(datetime!(2026-10-31 22:00), datetime!(2026-11-01 06:00)),
            9 * 3600,
            "the clocks go back: an extra hour"
        );
        assert_eq!(
            window(datetime!(2026-03-07 22:00), datetime!(2026-03-08 06:00)),
            7 * 3600,
            "the clocks go forward: a missing hour"
        );
        assert_eq!(
            window(datetime!(2026-06-01 22:00), datetime!(2026-06-02 06:00)),
            8 * 3600
        );
    }

    #[test]
    fn a_zone_without_daylight_saving_keeps_its_offset() {
        let jakarta = LocalZone::named("Asia/Jakarta").unwrap();
        assert_eq!(jakarta.offset_secs(1_793_462_400), 7 * 3600);
        assert_eq!(jakarta.offset_secs(1_781_000_000), 7 * 3600);
        assert_eq!(jakarta.name(), Some("Asia/Jakarta"));
        assert_eq!(LocalZone::utc().offset_secs(1_793_462_400), 0);
        assert_eq!(LocalZone::utc().at(0).unwrap().unix_timestamp(), 0);
    }

    #[test]
    fn out_of_range_instants_have_no_time() {
        assert_eq!(new_york().at(i64::MAX), None);
        assert_eq!(new_york().offset_secs(i64::MAX), 0);
        assert_eq!(
            new_york().unix_from_local(datetime!(9999-12-31 23:59)),
            None
        );
    }

    #[test]
    fn the_zone_name_is_what_follows_zoneinfo_in_the_link() {
        assert_eq!(
            zone_name("/var/db/timezone/zoneinfo/Asia/Jakarta"),
            Some("Asia/Jakarta")
        );
        assert_eq!(
            zone_name("/usr/share/zoneinfo/America/Argentina/Buenos_Aires"),
            Some("America/Argentina/Buenos_Aires")
        );
        assert_eq!(zone_name("../usr/share/zoneinfo/UTC"), Some("UTC"));
        assert_eq!(zone_name("/etc/some-other-file"), None);
    }

    #[test]
    fn only_a_system_zone_follows_the_system() {
        assert!(new_york().refresh().is_none());
        assert!(LocalZone::utc().refresh().is_none());
        // Nothing changed between two reads a moment apart.
        let system = LocalZone::system();
        assert!(system.refresh().is_none());
    }

    #[test]
    fn a_changed_link_gives_the_new_zone() {
        let old = SystemKey {
            tz_env: None,
            localtime: Some("/usr/share/zoneinfo/Asia/Jakarta".into()),
        };
        let new = SystemKey {
            tz_env: None,
            localtime: Some("/usr/share/zoneinfo/America/New_York".into()),
        };
        assert_ne!(old, new);
        let (jakarta, york) = (resolve(&old), resolve(&new));
        assert_eq!(jakarta.iana_name(), Some("Asia/Jakarta"));
        assert_eq!(york.iana_name(), Some("America/New_York"));
    }
}
