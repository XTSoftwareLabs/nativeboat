//! What `Intl` learns from Windows: the default locale, the names, patterns
//! and separators of locales other than American English, and time zones.

use serde_json::{Value, json};
use std::cell::RefCell;
use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows_sys::Win32::Globalization::{
    GetDateFormatEx, GetLocaleInfoEx, GetUserDefaultLocaleName, LOCALE_ICURRENCY,
    LOCALE_IFIRSTDAYOFWEEK, LOCALE_INEGATIVEPERCENT, LOCALE_INEGCURR, LOCALE_IPOSITIVEPERCENT,
    LOCALE_RETURN_NUMBER, LOCALE_SABBREVDAYNAME1, LOCALE_SABBREVMONTHNAME1, LOCALE_SAM,
    LOCALE_SCURRENCY, LOCALE_SDAYNAME1, LOCALE_SDECIMAL, LOCALE_SGROUPING, LOCALE_SINTLSYMBOL,
    LOCALE_SLIST, LOCALE_SLONGDATE, LOCALE_SMONTHDAY, LOCALE_SMONTHNAME1, LOCALE_SNAME,
    LOCALE_SNEGATIVESIGN, LOCALE_SPERCENT, LOCALE_SPM, LOCALE_SSHORTDATE, LOCALE_SSHORTESTDAYNAME1,
    LOCALE_SSHORTTIME, LOCALE_STHOUSAND, LOCALE_STIMEFORMAT, LOCALE_SYEARMONTH, ResolveLocaleName,
};
use windows_sys::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, EnumDynamicTimeZoneInformation, FileTimeToSystemTime,
    GetDynamicTimeZoneInformation, GetTimeZoneInformationForYear, SystemTimeToTzSpecificLocalTime,
    TIME_ZONE_INFORMATION,
};

thread_local! {
    /// The locale the application chose, as with Chromium's `--lang` switch,
    /// for the hosts of the user interface thread.
    static DEFAULT_LOCALE: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn text(buffer: &[u16], length: i32) -> String {
    let length = usize::try_from(length)
        .unwrap_or_default()
        .saturating_sub(1);
    String::from_utf16_lossy(&buffer[..length.min(buffer.len())])
}

/// Sets the default locale of the thread's hosts, or goes back to the
/// Windows user locale with `None`.
pub fn set_default_locale(locale: Option<String>) {
    DEFAULT_LOCALE.set(locale);
}

/// The default locale: the application's choice, otherwise the user's.
pub fn default_locale() -> String {
    DEFAULT_LOCALE
        .with_borrow(Clone::clone)
        .unwrap_or_else(system_locale)
}

/// The locale of the Windows user.
pub fn system_locale() -> String {
    let mut buffer = [0u16; 85];
    // SAFETY: the buffer holds `buffer.len()` characters.
    let length = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
    text(&buffer, length)
}

fn info(locale: &[u16], kind: u32) -> String {
    let mut buffer = [0u16; 256];
    // SAFETY: the locale name is NUL-terminated and the buffer holds
    // `buffer.len()` characters.
    let length = unsafe {
        GetLocaleInfoEx(
            locale.as_ptr(),
            kind,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    text(&buffer, length)
}

fn number(locale: &[u16], kind: u32) -> u32 {
    let mut value = 0u32;
    // SAFETY: with LOCALE_RETURN_NUMBER the buffer is one DWORD, two
    // characters long.
    unsafe {
        GetLocaleInfoEx(
            locale.as_ptr(),
            kind | LOCALE_RETURN_NUMBER,
            (&mut value as *mut u32).cast(),
            2,
        )
    };
    value
}

/// A month name as dates use it, which some languages inflect: the text
/// Windows writes for "d MMMM" without the day.
fn genitive_month(locale: &[u16], month: u16) -> String {
    let date = SYSTEMTIME {
        wYear: 2000,
        wMonth: month,
        wDay: 1,
        ..SYSTEMTIME::default()
    };
    let picture = wide("d MMMM");
    let mut buffer = [0u16; 128];
    // SAFETY: the strings are NUL-terminated and the buffer holds
    // `buffer.len()` characters.
    let length = unsafe {
        GetDateFormatEx(
            locale.as_ptr(),
            0,
            &date,
            picture.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as i32,
            std::ptr::null(),
        )
    };
    text(&buffer, length)
        .trim_start_matches(|character: char| character.is_ascii_digit())
        .trim_start_matches(['.', ' ', '\u{a0}'])
        .to_owned()
}

/// The names, patterns and separators of a locale Windows knows.
pub fn locale_data(tag: &str) -> Option<Value> {
    // Windows makes up data for any well-formed tag; only locales it
    // resolves to one of its own have real data.
    let mut resolved = [0u16; 85];
    // SAFETY: the name is NUL-terminated and the buffer holds
    // `resolved.len()` characters.
    let length = unsafe {
        ResolveLocaleName(
            wide(tag).as_ptr(),
            resolved.as_mut_ptr(),
            resolved.len() as i32,
        )
    };
    let name = text(&resolved, length);
    if name.is_empty() {
        return None;
    }
    let locale = wide(&name);
    let get = |kind| info(&locale, kind);
    let list = |first: u32, count: u32| {
        (0..count)
            .map(|index| get(first + index))
            .collect::<Vec<_>>()
    };
    // Windows lists the days from Monday, JavaScript from Sunday.
    let days = |first| {
        let mut days = list(first, 7);
        days.rotate_right(1);
        days
    };
    let genitive: Vec<String> = (1..=12)
        .map(|month| genitive_month(&locale, month))
        .collect();
    Some(json!({
        "name": get(LOCALE_SNAME),
        "months": list(LOCALE_SMONTHNAME1, 12),
        "monthsGenitive": genitive,
        "monthsShort": list(LOCALE_SABBREVMONTHNAME1, 12),
        "weekdays": days(LOCALE_SDAYNAME1),
        "weekdaysShort": days(LOCALE_SABBREVDAYNAME1),
        "weekdaysNarrow": days(LOCALE_SSHORTESTDAYNAME1),
        "am": get(LOCALE_SAM),
        "pm": get(LOCALE_SPM),
        "shortDate": get(LOCALE_SSHORTDATE),
        "longDate": get(LOCALE_SLONGDATE),
        "yearMonth": get(LOCALE_SYEARMONTH),
        "monthDay": get(LOCALE_SMONTHDAY),
        "time": get(LOCALE_STIMEFORMAT),
        "shortTime": get(LOCALE_SSHORTTIME),
        "decimal": get(LOCALE_SDECIMAL),
        "group": get(LOCALE_STHOUSAND),
        "grouping": get(LOCALE_SGROUPING),
        "minus": get(LOCALE_SNEGATIVESIGN),
        "percent": get(LOCALE_SPERCENT),
        "percentPattern": number(&locale, LOCALE_IPOSITIVEPERCENT),
        "negativePercentPattern": number(&locale, LOCALE_INEGATIVEPERCENT),
        "currencySymbol": get(LOCALE_SCURRENCY),
        "currency": get(LOCALE_SINTLSYMBOL),
        "currencyPattern": number(&locale, LOCALE_ICURRENCY),
        "negativeCurrencyPattern": number(&locale, LOCALE_INEGCURR),
        "list": get(LOCALE_SLIST),
        "firstDay": number(&locale, LOCALE_IFIRSTDAYOFWEEK),
    }))
}

fn key_name(zone: &DYNAMIC_TIME_ZONE_INFORMATION) -> String {
    let key = &zone.TimeZoneKeyName;
    let length = key.iter().position(|unit| *unit == 0).unwrap_or(key.len());
    String::from_utf16_lossy(&key[..length])
}

/// The Windows key of the local time zone, such as "Pacific Standard Time".
pub fn local_time_zone() -> String {
    let mut zone = DYNAMIC_TIME_ZONE_INFORMATION::default();
    // SAFETY: `zone` is writable.
    unsafe { GetDynamicTimeZoneInformation(&mut zone) };
    key_name(&zone)
}

/// Days from 1970-01-01 to a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146_097 + of_cycle - 719_468
}

fn minutes(time: &SYSTEMTIME) -> i64 {
    let days = days_from_civil(
        i64::from(time.wYear),
        i64::from(time.wMonth),
        i64::from(time.wDay),
    );
    days * 1440 + i64::from(time.wHour) * 60 + i64::from(time.wMinute)
}

/// The offset from UTC in minutes of a Windows time zone at a moment, or
/// `None` for an unknown zone or a moment Windows cannot represent.
pub fn zone_offset(key: &str, epoch_ms: f64) -> Option<f64> {
    let mut zone = DYNAMIC_TIME_ZONE_INFORMATION::default();
    let mut index = 0;
    loop {
        // SAFETY: `zone` is writable.
        if unsafe { EnumDynamicTimeZoneInformation(index, &mut zone) } != 0 {
            return None;
        }
        if key_name(&zone).eq_ignore_ascii_case(key) {
            break;
        }
        index += 1;
    }
    // Windows file times count 100 ns intervals from 1601.
    let ticks = (epoch_ms + 11_644_473_600_000.0) * 10_000.0;
    if !(0.0..1.0e18).contains(&ticks) {
        return None;
    }
    let ticks = ticks as u64;
    let file_time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut rules = TIME_ZONE_INFORMATION::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: all structures are valid and writable as the calls require.
    let converted = unsafe {
        FileTimeToSystemTime(&file_time, &mut utc) != 0
            && GetTimeZoneInformationForYear(utc.wYear, &zone, &mut rules) != 0
            && SystemTimeToTzSpecificLocalTime(&rules, &utc, &mut local) != 0
    };
    converted.then(|| (minutes(&local) - minutes(&utc)) as f64)
}

#[cfg(test)]
mod tests;
