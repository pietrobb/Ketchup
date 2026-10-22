//! How far the user's clock is ahead of UTC, from the operating system's time zone
//! (daylight saving included), so a date written for the user is the user's date.
//! The standard library does not tell the time zone, so this module asks the system
//! (`localtime_r`, `GetLocalTime`), which needs `unsafe`.
#![allow(unsafe_code)]

/// Seconds the local clock is ahead of UTC now; 0 where the system does not say.
#[cfg(unix)]
pub(crate) fn utc_offset_seconds() -> i64 {
    // SAFETY: `time` with a null pointer only returns the time; `localtime_r` reads
    // `time` and writes only the `tm` it is given, both owned here for the whole
    // call, and is the thread-safe variant.
    unsafe {
        let time = libc::time(std::ptr::null_mut());
        let mut local: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&time, &mut local).is_null() {
            return 0;
        }
        local.tm_gmtoff
    }
}

/// Seconds the local clock is ahead of UTC now; 0 where the system does not say.
#[cfg(windows)]
pub(crate) fn utc_offset_seconds() -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
    // SAFETY: both calls only write the `SYSTEMTIME` they are given, owned here.
    let (local, utc) = unsafe {
        let mut local: SYSTEMTIME = std::mem::zeroed();
        let mut utc: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut local);
        GetSystemTime(&mut utc);
        (local, utc)
    };
    let date = |time: &SYSTEMTIME| (time.wYear, time.wMonth, time.wDay);
    // An offset is less than a day, so the two dates differ by at most one.
    let days = match date(&local).cmp(&date(&utc)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    };
    let seconds = |time: &SYSTEMTIME| {
        i64::from(time.wHour) * 3_600 + i64::from(time.wMinute) * 60 + i64::from(time.wSecond)
    };
    let offset = days * 86_400 + seconds(&local) - seconds(&utc);
    // The two readings may straddle a second: offsets are whole minutes.
    (offset + 30).div_euclid(60) * 60
}

/// Seconds the local clock is ahead of UTC; this system does not say, so UTC.
#[cfg(not(any(unix, windows)))]
pub(crate) fn utc_offset_seconds() -> i64 {
    0
}
