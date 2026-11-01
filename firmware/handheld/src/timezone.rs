//! Local time. The device clock (PCF8563) keeps UTC; the time zone setting converts it to local
//! time for display, game clocks and file names. Zones are POSIX TZ strings, so daylight saving
//! changes are handled by the C library.

use std::sync::atomic::{AtomicI32, Ordering};

use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

use crate::kvs::keys;

/// (name shown in Settings, POSIX TZ string). Only add to the end: the setting stores the index.
pub const ZONES: &[(&str, &str)] = &[
    ("UTC", "UTC0"),
    ("US Eastern", "EST5EDT,M3.2.0,M11.1.0"),
    ("US Central", "CST6CDT,M3.2.0,M11.1.0"),
    ("US Mountain", "MST7MDT,M3.2.0,M11.1.0"),
    ("US Arizona", "MST7"),
    ("US Pacific", "PST8PDT,M3.2.0,M11.1.0"),
    ("US Alaska", "AKST9AKDT,M3.2.0,M11.1.0"),
    ("US Hawaii", "HST10"),
    ("Canada Atlantic", "AST4ADT,M3.2.0,M11.1.0"),
    ("Newfoundland", "NST3:30NDT,M3.2.0,M11.1.0"),
    ("Mexico City", "CST6"),
    ("Colombia, Peru", "<-05>5"),
    ("Chile", "<-04>4<-03>,M9.1.6/24,M4.1.6/24"),
    ("Argentina, Brazil", "<-03>3"),
    ("Azores", "<-01>1<+00>,M3.5.0/0,M10.5.0/1"),
    ("UK, Ireland, Portugal", "GMT0BST,M3.5.0/1,M10.5.0"),
    ("Central Europe", "CET-1CEST,M3.5.0,M10.5.0/3"),
    ("Eastern Europe", "EET-2EEST,M3.5.0/3,M10.5.0/4"),
    ("South Africa", "SAST-2"),
    ("Turkey, Moscow", "<+03>-3"),
    ("Iran", "<+0330>-3:30"),
    ("Gulf (Dubai)", "<+04>-4"),
    ("Pakistan", "PKT-5"),
    ("India", "IST-5:30"),
    ("Nepal", "<+0545>-5:45"),
    ("Bangladesh", "<+06>-6"),
    ("Thailand, Indonesia W", "<+07>-7"),
    ("China, Singapore", "CST-8"),
    ("Australia West", "AWST-8"),
    ("Japan, Korea", "JST-9"),
    ("Australia Darwin", "ACST-9:30"),
    ("Australia Adelaide", "ACST-9:30ACDT,M10.1.0,M4.1.0/3"),
    ("Australia Brisbane", "AEST-10"),
    ("Australia East", "AEST-10AEDT,M10.1.0,M4.1.0/3"),
    ("New Zealand", "NZST-12NZDT,M9.5.0,M4.1.0/3"),
    ("UTC-12", "<-12>12"),
    ("UTC-11", "<-11>11"),
    ("UTC-10", "<-10>10"),
    ("UTC-9", "<-09>9"),
    ("UTC-8", "<-08>8"),
    ("UTC-7", "<-07>7"),
    ("UTC-6", "<-06>6"),
    ("UTC-5", "<-05>5"),
    ("UTC-4", "<-04>4"),
    ("UTC-3", "<-03>3"),
    ("UTC-2", "<-02>2"),
    ("UTC-1", "<-01>1"),
    ("UTC+1", "<+01>-1"),
    ("UTC+2", "<+02>-2"),
    ("UTC+3", "<+03>-3"),
    ("UTC+4", "<+04>-4"),
    ("UTC+5", "<+05>-5"),
    ("UTC+6", "<+06>-6"),
    ("UTC+7", "<+07>-7"),
    ("UTC+8", "<+08>-8"),
    ("UTC+9", "<+09>-9"),
    ("UTC+10", "<+10>-10"),
    ("UTC+11", "<+11>-11"),
    ("UTC+12", "<+12>-12"),
    ("UTC+13", "<+13>-13"),
    ("UTC+14", "<+14>-14"),
];

/// Names for the Settings list.
pub static ZONE_NAMES: [&str; ZONES.len()] = zone_names(ZONES);

const fn zone_names<const N: usize>(zones: &[(&'static str, &'static str)]) -> [&'static str; N] {
    let mut names = [""; N];
    let mut i = 0;
    while i < N {
        names[i] = zones[i].0;
        i += 1;
    }
    names
}

/// Zone currently given to the C library (-1: none yet).
static APPLIED: AtomicI32 = AtomicI32::new(-1);

/// Make sure the C library uses the zone from the settings (also used for file times).
pub fn apply() {
    let index = keys::TIME_ZONE.get().unwrap_or(0).clamp(0, ZONES.len() as i32 - 1);
    if APPLIED.swap(index, Ordering::Relaxed) == index {
        return;
    }
    let tz = std::ffi::CString::new(ZONES[index as usize].1).unwrap();
    unsafe {
        esp_idf_svc::sys::setenv(c"TZ".as_ptr(), tz.as_ptr(), 1);
        esp_idf_svc::sys::tzset();
    }
}

/// Offset from UTC of local time at the given moment.
pub fn offset_at(utc: OffsetDateTime) -> UtcOffset {
    apply();
    let t = utc.unix_timestamp() as esp_idf_svc::sys::time_t;
    let mut tm: esp_idf_svc::sys::tm = unsafe { std::mem::zeroed() };
    if unsafe { esp_idf_svc::sys::localtime_r(&t, &mut tm) }.is_null() {
        return UtcOffset::UTC;
    }
    let local = (|| -> Option<PrimitiveDateTime> {
        let month = time::Month::try_from(tm.tm_mon as u8 + 1).ok()?;
        let date = time::Date::from_calendar_date(tm.tm_year + 1900, month, tm.tm_mday as u8).ok()?;
        let time = time::Time::from_hms(tm.tm_hour as u8, tm.tm_min as u8, tm.tm_sec as u8).ok()?;
        Some(PrimitiveDateTime::new(date, time))
    })();
    let Some(local) = local else {
        return UtcOffset::UTC;
    };
    let utc = PrimitiveDateTime::new(utc.date(), utc.time()).replace_nanosecond(0).unwrap();
    let seconds = (local - utc).whole_seconds() as i32;
    UtcOffset::from_whole_seconds(seconds).unwrap_or(UtcOffset::UTC)
}

/// Convert a time to local time.
pub fn to_local(dt: OffsetDateTime) -> OffsetDateTime {
    dt.to_offset(offset_at(dt))
}

/// Interpret a local date and time (e.g. typed in Settings).
pub fn from_local(local: PrimitiveDateTime) -> OffsetDateTime {
    // The offset at the same wall-clock time read as UTC is right except within a few hours of
    // a daylight saving change, so check again with the result.
    let guess = local.assume_offset(offset_at(local.assume_utc()));
    local.assume_offset(offset_at(guess))
}

/// Short name of the offset, e.g. "UTC-05:00".
pub fn describe_offset(offset: UtcOffset) -> String {
    let (h, m, _) = offset.as_hms();
    if h == 0 && m == 0 {
        "UTC".into()
    } else {
        let sign = if offset.is_negative() { '-' } else { '+' };
        format!("UTC{sign}{:02}:{:02}", h.abs(), m.abs())
    }
}

/// The time for the status bar, e.g. "6:10 PM" or "18:10" (see Settings > General).
pub fn clock_text(local: OffsetDateTime) -> String {
    let (hour, minute) = (local.hour(), local.minute());
    if keys::CLOCK_FORMAT.get().unwrap_or(0) == 1 {
        format!("{hour:02}:{minute:02}")
    } else {
        let hour12 = if hour % 12 == 0 { 12 } else { hour % 12 };
        format!("{hour12}:{minute:02} {}", if hour < 12 { "AM" } else { "PM" })
    }
}
