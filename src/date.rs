//! Today's local date as `YYYY-MM-DD` and the time of day, using the C
//! library (no date crate).
//!
//! One of the two places with `unsafe` code in Rubrique (the other is
//! `touch_id/macos.rs`): one call to `localtime_r`, wrapped in the safe
//! function [`local_time`].

use std::ffi::{c_char, c_int, c_long};
use std::time::{SystemTime, UNIX_EPOCH};

/// `struct tm` from `<time.h>`. The layout is the same on macOS, glibc and
/// musl: nine ints, then `tm_gmtoff` and `tm_zone`.
#[repr(C)]
#[expect(clippy::struct_field_names, reason = "field names mirror the C `struct tm`")]
struct Tm {
    tm_sec: c_int,
    tm_min: c_int,
    tm_hour: c_int,
    tm_mday: c_int,
    tm_mon: c_int,
    tm_year: c_int,
    tm_wday: c_int,
    tm_yday: c_int,
    tm_isdst: c_int,
    tm_gmtoff: c_long,
    tm_zone: *const c_char,
}

impl Tm {
    /// All zeros, the value `localtime_r` overwrites.
    fn zeroed() -> Self {
        Tm {
            tm_sec: 0,
            tm_min: 0,
            tm_hour: 0,
            tm_mday: 0,
            tm_mon: 0,
            tm_year: 0,
            tm_wday: 0,
            tm_yday: 0,
            tm_isdst: 0,
            tm_gmtoff: 0,
            tm_zone: std::ptr::null(),
        }
    }
}

#[expect(unsafe_code, reason = "FFI declaration of `localtime_r`; no date crate per the dependency policy")]
unsafe extern "C" {
    /// Thread-safe conversion of a Unix timestamp to local time.
    /// `time_t` is a 64-bit signed integer on the 64-bit macOS and Linux
    /// targets Rubrique supports, hence `i64`.
    fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
}

/// Today's date in the user's local time zone, e.g. `2026-10-27`.
pub fn today_iso() -> String {
    now_local().date
}

/// The local date and time of day, for names like `2026-10-27-1403`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalNow {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub hour: u8,
    pub minute: u8,
}

/// The current local date and time.
pub fn now_local() -> LocalNow {
    // A clock before 1970 or past the year 292 billion is not worth an error
    // path: fall back to the epoch.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0);
    local_at(now).unwrap_or_else(|| LocalNow { date: iso_date(1970, 1, 1), hour: 0, minute: 0 })
}

/// The local date and time of a Unix timestamp (seconds); `None` when the
/// C library cannot represent it.
pub fn local_at(timestamp: i64) -> Option<LocalNow> {
    let local = local_time(timestamp)?;
    Some(LocalNow {
        date: iso_date(local.tm_year + 1900, local.tm_mon + 1, local.tm_mday),
        hour: u8::try_from(local.tm_hour).unwrap_or(0),
        minute: u8::try_from(local.tm_min).unwrap_or(0),
    })
}

/// Convert a Unix timestamp to local calendar time; `None` when the C
/// library cannot represent it.
#[expect(unsafe_code, reason = "the single FFI call to `localtime_r`, kept behind this safe function")]
fn local_time(timestamp: i64) -> Option<Tm> {
    let mut local = Tm::zeroed();
    // SAFETY: both pointers come from live, properly aligned locals that
    // outlive the call. `localtime_r` reads `timestamp` and only writes into
    // `local`; it returns null on failure, which we check before reading.
    let result = unsafe { localtime_r(&raw const timestamp, &raw mut local) };
    (!result.is_null()).then_some(local)
}

/// Format a calendar date as `YYYY-MM-DD`.
fn iso_date(year: c_int, month: c_int, day: c_int) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_an_iso_date_this_century() {
        let today = today_iso();
        assert_eq!(today.len(), 10);
        assert!(today.starts_with("20"));
        assert_eq!(&today[4..5], "-");
        assert_eq!(&today[7..8], "-");
    }

    #[test]
    fn time_of_day_is_in_range() {
        let now = now_local();
        assert!(now.hour < 24 && now.minute < 60);
        assert_eq!(now.date.len(), 10);
    }

    #[test]
    fn iso_date_pads_month_and_day() {
        assert_eq!(iso_date(2026, 3, 7), "2026-03-07");
    }

    #[test]
    fn local_time_of_a_known_timestamp() {
        // 2026-06-15 12:00 UTC is the same calendar day in every time zone
        // from UTC-11 to UTC+11.
        let local = local_time(1_781_524_800).unwrap();
        assert_eq!((local.tm_year + 1900, local.tm_mon + 1, local.tm_mday), (2026, 6, 15));
    }
}
