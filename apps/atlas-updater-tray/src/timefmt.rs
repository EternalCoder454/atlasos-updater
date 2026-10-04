//! A time as the user's locale writes it, short and without seconds, like
//! `QLocale::toString(…, QLocale::ShortFormat)` in the tooltip before.
//! `main` sets the locale from the environment (`setlocale(LC_ALL, "")`).

use std::ffi::{CStr, CString};

fn strftime(fmt: &str, tm: &libc::tm) -> String {
    let Ok(fmt) = CString::new(fmt) else {
        return String::new();
    };
    let mut buf = [0u8; 128];
    // SAFETY: the buffer and its length match, `fmt` is NUL-terminated and
    // `tm` is a valid broken-down time.
    let n = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), tm) };
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// Whether the locale writes times with AM/PM.
fn twelve_hour() -> bool {
    // SAFETY: nl_langinfo returns a pointer to a static, NUL-terminated
    // string (valid until the next setlocale, which only main calls, first).
    let fmt = unsafe { CStr::from_ptr(libc::nl_langinfo(libc::T_FMT)) };
    let fmt = fmt.to_string_lossy();
    fmt.contains("%p") || fmt.contains("%r") || fmt.contains("%I") || fmt.contains("%l")
}

/// `t` (Unix seconds) in local time: the locale's date, then hours and
/// minutes ("10/04/26 3:00 PM", "04.10.26 15:00").
pub fn short(t: i64) -> String {
    let secs = t as libc::time_t;
    // SAFETY: zeroed is a valid `tm`; localtime_r fills it or fails.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return String::new();
    }
    let time = if twelve_hour() {
        strftime("%l:%M %p", &tm)
    } else {
        strftime("%H:%M", &tm)
    };
    format!("{} {}", strftime("%x", &tm), time.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_no_seconds() {
        // The C locale: "%x" is MM/DD/YY and the clock is 24 h.
        let s = short(1_700_000_000);
        assert!(!s.is_empty());
        assert_eq!(s.matches(':').count(), 1, "{s}");
    }
}
