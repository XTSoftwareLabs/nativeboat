//! `Date` as ECMAScript 2024 specifies it, with V8's string formats and
//! date parsing. Local time comes from the Windows time zone (dynamic,
//! year-specific rules) and locale strings from the user's Windows locale,
//! as Electron's ICU defaults do. Years outside Windows' 1601–30827 range
//! use the rules of the year 400·n years away, which shares the calendar.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::object::{Key, Kind, Object, Property, Slot};
use crate::ops;
use crate::realm::{ErrorKind, Realm};
use crate::string::JsStr;
use crate::value::{Completion, Thrown, Value};
use core::ffi::c_void;

/// Windows `SYSTEMTIME`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SystemTime {
    pub(crate) year: u16,
    pub(crate) month: u16,
    pub(crate) day_of_week: u16,
    pub(crate) day: u16,
    pub(crate) hour: u16,
    pub(crate) minute: u16,
    pub(crate) second: u16,
    pub(crate) milliseconds: u16,
}

/// Windows `DYNAMIC_TIME_ZONE_INFORMATION`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct TimeZone {
    pub(crate) bias: i32,
    pub(crate) standard_name: [u16; 32],
    pub(crate) standard_date: SystemTime,
    pub(crate) standard_bias: i32,
    pub(crate) daylight_name: [u16; 32],
    pub(crate) daylight_date: SystemTime,
    pub(crate) daylight_bias: i32,
    pub(crate) key_name: [u16; 128],
    pub(crate) dynamic_daylight_time_disabled: u8,
}

impl Default for TimeZone {
    fn default() -> TimeZone {
        TimeZone {
            bias: 0,
            standard_name: [0; 32],
            standard_date: SystemTime::default(),
            standard_bias: 0,
            daylight_name: [0; 32],
            daylight_date: SystemTime::default(),
            daylight_bias: 0,
            key_name: [0; 128],
            dynamic_daylight_time_disabled: 0,
        }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemTimePreciseAsFileTime(time: *mut u64);
    fn GetDynamicTimeZoneInformation(zone: *mut TimeZone) -> u32;
    fn SystemTimeToTzSpecificLocalTimeEx(
        zone: *const TimeZone,
        utc: *const SystemTime,
        local: *mut SystemTime,
    ) -> i32;
    fn SystemTimeToTzSpecificLocalTime(
        zone: *const TimeZone,
        utc: *const SystemTime,
        local: *mut SystemTime,
    ) -> i32;
    fn GetDateFormatEx(
        locale: *const u16,
        flags: u32,
        date: *const SystemTime,
        format: *const u16,
        output: *mut u16,
        length: i32,
        calendar: *const c_void,
    ) -> i32;
    fn GetTimeFormatEx(
        locale: *const u16,
        flags: u32,
        time: *const SystemTime,
        format: *const u16,
        output: *mut u16,
        length: i32,
    ) -> i32;
    fn GetUserDefaultLocaleName(name: *mut u16, length: i32) -> i32;
}

const MS_PER_SECOND: f64 = 1000.0;
const MS_PER_MINUTE: f64 = 60_000.0;
const MS_PER_HOUR: f64 = 3_600_000.0;
const MS_PER_DAY: f64 = 86_400_000.0;
/// The largest time value magnitude, 100 million days.
const MAX_TIME: f64 = 8.64e15;
/// Days in 400 Gregorian years: shifting by them keeps every date's weekday.
const CYCLE_DAYS: f64 = 146_097.0;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn floor(value: f64) -> f64 {
    crate::float::floor(value)
}

fn modulo(value: f64, divisor: f64) -> f64 {
    // Adding zero turns a negative zero remainder into zero.
    crate::float::rem_euclid(value, divisor) + 0.0
}

/// `Day(t)`.
fn day(t: f64) -> f64 {
    floor(t / MS_PER_DAY)
}

/// `DayFromYear(y)`.
fn day_from_year(y: f64) -> f64 {
    365.0 * (y - 1970.0) + floor((y - 1969.0) / 4.0) - floor((y - 1901.0) / 100.0)
        + floor((y - 1601.0) / 400.0)
}

fn leap(year: f64) -> bool {
    (modulo(year, 4.0) == 0.0 && modulo(year, 100.0) != 0.0) || modulo(year, 400.0) == 0.0
}

/// `YearFromTime(t)`.
fn year_from_time(t: f64) -> f64 {
    let days = day(t);
    let mut year = floor(days / 365.2425) + 1970.0;
    while day_from_year(year) > days {
        year -= 1.0;
    }
    while day_from_year(year + 1.0) <= days {
        year += 1.0;
    }
    year
}

/// The first day of each month, in days from the start of the year.
fn month_start(month: usize, leap: bool) -> f64 {
    const STARTS: [f64; 13] = [
        0.0, 31.0, 59.0, 90.0, 120.0, 151.0, 181.0, 212.0, 243.0, 273.0, 304.0, 334.0, 365.0,
    ];
    STARTS[month] + if leap && month >= 2 { 1.0 } else { 0.0 }
}

/// Year, month (0–11) and date (1–31) of a time value.
fn date_parts(t: f64) -> (f64, usize, f64) {
    let year = year_from_time(t);
    let within = day(t) - day_from_year(year);
    let is_leap = leap(year);
    let month = (1..12)
        .take_while(|month| month_start(*month, is_leap) <= within)
        .count();
    (year, month, within - month_start(month, is_leap) + 1.0)
}

fn week_day(t: f64) -> usize {
    modulo(day(t) + 4.0, 7.0) as usize
}

fn hours(t: f64) -> f64 {
    modulo(floor(t / MS_PER_HOUR), 24.0)
}

fn minutes(t: f64) -> f64 {
    modulo(floor(t / MS_PER_MINUTE), 60.0)
}

fn seconds(t: f64) -> f64 {
    modulo(floor(t / MS_PER_SECOND), 60.0)
}

fn milliseconds(t: f64) -> f64 {
    modulo(t, MS_PER_SECOND)
}

fn integer(value: f64) -> f64 {
    crate::float::trunc(value) + 0.0
}

/// `MakeTime`.
fn make_time(hour: f64, minute: f64, second: f64, millisecond: f64) -> f64 {
    if !(hour.is_finite() && minute.is_finite() && second.is_finite() && millisecond.is_finite()) {
        return f64::NAN;
    }
    integer(hour) * MS_PER_HOUR
        + integer(minute) * MS_PER_MINUTE
        + integer(second) * MS_PER_SECOND
        + integer(millisecond)
}

/// `MakeDay`.
fn make_day(year: f64, month: f64, date: f64) -> f64 {
    if !(year.is_finite() && month.is_finite() && date.is_finite()) {
        return f64::NAN;
    }
    let (year, month, date) = (integer(year), integer(month), integer(date));
    let year = year + floor(month / 12.0);
    if year.abs() > 400_000.0 {
        return f64::NAN;
    }
    let month = modulo(month, 12.0) as usize;
    day_from_year(year) + month_start(month, leap(year)) + date - 1.0
}

/// `MakeDate`.
fn make_date(day: f64, time: f64) -> f64 {
    let date = day * MS_PER_DAY + time;
    if date.is_finite() { date } else { f64::NAN }
}

/// `TimeClip`.
fn time_clip(time: f64) -> f64 {
    if !time.is_finite() || time.abs() > MAX_TIME {
        f64::NAN
    } else {
        integer(time)
    }
}

/// The current time in milliseconds since the epoch.
pub(crate) fn now() -> f64 {
    let mut ticks = 0u64;
    // SAFETY: the output location is valid.
    unsafe { GetSystemTimePreciseAsFileTime(&mut ticks) };
    // Windows counts 100 ns ticks from 1601.
    floor((ticks as f64 - 116_444_736_000_000_000.0) / 10_000.0)
}

/// A UTC time value within Windows' supported years, and the shift applied.
fn in_range(t: f64) -> (f64, f64) {
    let year = year_from_time(t);
    let cycles = if year < 1601.0 {
        floor((1601.0 - year) / 400.0) + 1.0
    } else if year > 30827.0 {
        -(floor((year - 30827.0) / 400.0) + 1.0)
    } else {
        0.0
    };
    let shift = cycles * CYCLE_DAYS * MS_PER_DAY;
    (t + shift, shift)
}

fn system_time(t: f64) -> SystemTime {
    let (year, month, date) = date_parts(t);
    SystemTime {
        year: year as u16,
        month: month as u16 + 1,
        day_of_week: week_day(t) as u16,
        day: date as u16,
        hour: hours(t) as u16,
        minute: minutes(t) as u16,
        second: seconds(t) as u16,
        milliseconds: milliseconds(t) as u16,
    }
}

fn time_value(time: &SystemTime) -> f64 {
    let day = make_day(
        f64::from(time.year),
        f64::from(time.month) - 1.0,
        f64::from(time.day),
    );
    make_date(
        day,
        make_time(
            f64::from(time.hour),
            f64::from(time.minute),
            f64::from(time.second),
            f64::from(time.milliseconds),
        ),
    )
}

/// The realm's time zone, read from Windows on first use.
fn zone(realm: &mut Realm) -> TimeZone {
    *realm.time_zone.get_or_insert_with(|| {
        let mut zone = TimeZone::default();
        // SAFETY: the output structure is valid and fully sized.
        unsafe { GetDynamicTimeZoneInformation(&mut zone) };
        zone
    })
}

/// The standard offset of a zone, in milliseconds, when conversion fails.
fn standard_offset(zone: &TimeZone) -> f64 {
    -f64::from(zone.bias + zone.standard_bias) * MS_PER_MINUTE
}

/// The local time zone's offset from UTC at a UTC time value.
fn offset(realm: &mut Realm, t: f64) -> f64 {
    let zone = zone(realm);
    let (shifted, _) = in_range(t);
    let utc = system_time(shifted);
    let mut local = SystemTime::default();
    // A named Windows zone uses its year-specific rules; a zone without a
    // name (as tests construct) uses its fixed rules. The dynamic structure
    // begins with the classic one, which the second function reads.
    // SAFETY: all three structures are valid.
    let converted = unsafe {
        if zone.key_name[0] == 0 {
            SystemTimeToTzSpecificLocalTime(&zone, &utc, &mut local)
        } else {
            SystemTimeToTzSpecificLocalTimeEx(&zone, &utc, &mut local)
        }
    };
    if converted == 0 {
        return standard_offset(&zone);
    }
    time_value(&local) - shifted
}

/// `LocalTime(t)`.
fn local_time(realm: &mut Realm, t: f64) -> f64 {
    t + offset(realm, t)
}

/// `UTC(t)`: the UTC time value of a local time value. Of the instants
/// whose local time is `t`, the earliest is chosen; in a gap skipped by a
/// transition, the offset before it applies, as ECMAScript specifies.
/// Candidate offsets are those a day before and after, which covers every
/// transition shorter than a day.
fn utc(realm: &mut Realm, t: f64) -> f64 {
    if !t.is_finite() {
        return f64::NAN;
    }
    let before = offset(realm, t - MS_PER_DAY);
    let after = offset(realm, t + MS_PER_DAY);
    let mut earliest: Option<f64> = None;
    for candidate in [before, after] {
        let instant = t - candidate;
        if offset(realm, instant) == candidate {
            earliest = Some(earliest.map_or(instant, |earliest| earliest.min(instant)));
        }
    }
    earliest.unwrap_or(t - before)
}

/// The time value of a date object.
pub(crate) fn time_of(object: &Object) -> Option<f64> {
    match object.data().kind {
        Kind::Date(time) => Some(time),
        _ => None,
    }
}

fn date_value(realm: &Realm, this: &Value, method: &str) -> Result<f64, Thrown> {
    this.as_object().and_then(time_of).ok_or_else(|| {
        realm.throw(
            ErrorKind::Type,
            &format!("this is not a Date object. ({method})"),
        )
    })
}

fn set_value(this: &Value, time: f64) -> Completion {
    if let Some(object) = this.as_object() {
        object.data_mut().kind = Kind::Date(time);
    }
    Ok(Value::Number(time))
}

fn number_arg(realm: &mut Realm, args: &[Value], index: usize) -> Result<Option<f64>, Thrown> {
    match args.get(index) {
        Some(value) => ops::to_number(realm, value).map(Some),
        None => Ok(None),
    }
}

/// A year as `DateString` writes it: a sign for negative years and at
/// least four digits.
fn year_text(year: f64) -> String {
    if year < 0.0 {
        format!("-{:04}", -year)
    } else {
        format!("{year:04}")
    }
}

/// `DateString` of a local time value.
fn date_string(t: f64) -> String {
    let (year, month, date) = date_parts(t);
    format!(
        "{} {} {date:02} {}",
        DAYS[week_day(t)],
        MONTHS[month],
        year_text(year)
    )
}

fn time_string(t: f64) -> String {
    format!("{:02}:{:02}:{:02}", hours(t), minutes(t), seconds(t))
}

fn zone_name(zone: &TimeZone, daylight: bool) -> String {
    let name = if daylight {
        &zone.daylight_name
    } else {
        &zone.standard_name
    };
    let length = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    String::from_utf16_lossy(&name[..length])
}

/// `TimeZoneString`: `GMT+0800 (name)`.
fn zone_string(realm: &mut Realm, t: f64) -> String {
    let offset = offset(realm, t);
    let zone = zone(realm);
    let daylight = offset != standard_offset(&zone);
    let minutes = (offset / MS_PER_MINUTE).abs() as i64;
    let sign = if offset < 0.0 { '-' } else { '+' };
    format!(
        "GMT{sign}{:02}{:02} ({})",
        minutes / 60,
        minutes % 60,
        zone_name(&zone, daylight)
    )
}

/// The Windows user locale's short date and long time of a local time.
fn locale_parts(realm: &mut Realm, t: f64) -> Option<(String, String, bool)> {
    let (shifted, _) = in_range(t);
    if shifted != t {
        return None;
    }
    let time = system_time(t);
    let locale = realm.locale.clone().unwrap_or_else(|| {
        let mut name = [0u16; 85];
        // SAFETY: the buffer is valid for its length.
        let length = unsafe { GetUserDefaultLocaleName(name.as_mut_ptr(), name.len() as i32) };
        name[..length.max(1) as usize].to_vec()
    });
    let mut date = [0u16; 128];
    let mut clock = [0u16; 128];
    // SAFETY: the locale is null-terminated and the buffers are valid for
    // their lengths; DATE_SHORTDATE (1) and the default long time format.
    let (date_length, clock_length) = unsafe {
        (
            GetDateFormatEx(
                locale.as_ptr(),
                1,
                &time,
                core::ptr::null(),
                date.as_mut_ptr(),
                128,
                core::ptr::null(),
            ),
            GetTimeFormatEx(
                locale.as_ptr(),
                0,
                &time,
                core::ptr::null(),
                clock.as_mut_ptr(),
                128,
            ),
        )
    };
    if date_length <= 0 || clock_length <= 0 {
        return None;
    }
    let text = |units: &[u16], length: i32| String::from_utf16_lossy(&units[..length as usize - 1]);
    let english = locale.starts_with(&[u16::from(b'e'), u16::from(b'n')]);
    Some((
        text(&date, date_length),
        text(&clock, clock_length),
        english,
    ))
}

/// `toISOString`'s text of a valid time value.
fn iso_string(t: f64) -> String {
    let (year, month, date) = date_parts(t);
    let year = if (0.0..=9999.0).contains(&year) {
        format!("{year:04}")
    } else if year < 0.0 {
        format!("-{:06}", -year)
    } else {
        format!("+{year:06}")
    };
    format!(
        "{year}-{:02}-{date:02}T{:02}:{:02}:{:02}.{:03}Z",
        month + 1,
        hours(t),
        minutes(t),
        seconds(t),
        milliseconds(t)
    )
}

/// What a formatting method writes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    Full,
    Date,
    Time,
    Utc,
    LocaleFull,
    LocaleDate,
    LocaleTime,
}

fn format_time(realm: &mut Realm, t: f64, format: Format) -> String {
    if t.is_nan() {
        return "Invalid Date".to_owned();
    }
    if format == Format::Utc {
        let (year, month, date) = date_parts(t);
        return format!(
            "{}, {date:02} {} {} {} GMT",
            DAYS[week_day(t)],
            MONTHS[month],
            year_text(year),
            time_string(t)
        );
    }
    let local = local_time(realm, t);
    match format {
        Format::Full => format!(
            "{} {} {}",
            date_string(local),
            time_string(local),
            zone_string(realm, t)
        ),
        Format::Date => date_string(local),
        Format::Time => format!("{} {}", time_string(local), zone_string(realm, t)),
        _ => match locale_parts(realm, local) {
            Some((date, clock, english)) => match format {
                Format::LocaleDate => date,
                Format::LocaleTime => clock,
                _ if english => format!("{date}, {clock}"),
                _ => format!("{date} {clock}"),
            },
            None => format!("{} {}", date_string(local), time_string(local)),
        },
    }
}

/// Parses `digits` decimal digits at `index`.
fn digits(text: &[u8], index: usize, count: usize) -> Option<f64> {
    let slice = text.get(index..index + count)?;
    slice.iter().all(u8::is_ascii_digit).then(|| {
        slice
            .iter()
            .fold(0.0, |value, digit| value * 10.0 + f64::from(digit - b'0'))
    })
}

/// The date time string format of ECMAScript, or `None` when the text
/// does not follow it. Date-only forms are UTC, others local unless offset.
fn parse_iso(realm: &mut Realm, text: &[u8]) -> Option<f64> {
    let mut index = 0;
    let year = match text.first() {
        Some(sign @ (b'+' | b'-')) => {
            let value = digits(text, 1, 6)?;
            if *sign == b'-' && value == 0.0 {
                return None;
            }
            index = 7;
            if *sign == b'-' { -value } else { value }
        }
        _ => {
            index += 4;
            digits(text, 0, 4)?
        }
    };
    let mut month = 1.0;
    let mut date = 1.0;
    if text.get(index) == Some(&b'-') {
        month = digits(text, index + 1, 2)?;
        index += 3;
        if text.get(index) == Some(&b'-') {
            date = digits(text, index + 1, 2)?;
            index += 3;
        }
    }
    let (mut hour, mut minute, mut second, mut millisecond) = (0.0, 0.0, 0.0, 0.0);
    let mut timed = false;
    if text.get(index) == Some(&b'T') {
        timed = true;
        hour = digits(text, index + 1, 2)?;
        if text.get(index + 3) != Some(&b':') {
            return None;
        }
        minute = digits(text, index + 4, 2)?;
        index += 6;
        if text.get(index) == Some(&b':') {
            second = digits(text, index + 1, 2)?;
            index += 3;
            if text.get(index) == Some(&b'.') {
                let start = index + 1;
                let end = start
                    + text[start..]
                        .iter()
                        .take_while(|byte| byte.is_ascii_digit())
                        .count();
                if end == start {
                    return None;
                }
                let fraction = &text[start..end.min(start + 3)];
                let value = digits(fraction, 0, fraction.len())?;
                millisecond = value * [100.0, 10.0, 1.0][fraction.len() - 1];
                index = end;
            }
        }
    }
    let mut offset = None;
    match text.get(index) {
        Some(b'Z') if timed => {
            offset = Some(0.0);
            index += 1;
        }
        Some(sign @ (b'+' | b'-')) if timed => {
            let hours = digits(text, index + 1, 2)?;
            if text.get(index + 3) != Some(&b':') {
                return None;
            }
            let minutes = digits(text, index + 4, 2)?;
            let value = (hours * 60.0 + minutes) * MS_PER_MINUTE;
            offset = Some(if *sign == b'-' { -value } else { value });
            index += 6;
        }
        _ => {}
    }
    if index != text.len()
        || !(1.0..=12.0).contains(&month)
        || !(1.0..=31.0).contains(&date)
        || hour > 24.0
        || minute > 59.0
        || second > 59.0
        || (hour == 24.0 && (minute, second, millisecond) != (0.0, 0.0, 0.0))
    {
        return None;
    }
    let time = make_date(
        make_day(year, month - 1.0, date),
        make_time(hour, minute, second, millisecond),
    );
    Some(match offset {
        Some(offset) => time - offset,
        None if timed => utc(realm, time),
        None => time,
    })
}

/// V8's fallback parser for other common forms: month names, numeric
/// `m/d/y` and `y/m/d` dates, times with AM/PM, and GMT/UTC offsets.
fn parse_legacy(realm: &mut Realm, text: &str) -> f64 {
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    let mut numbers: Vec<f64> = Vec::new();
    let mut month_name = None;
    let mut time: Option<(f64, f64, f64, f64)> = None;
    let mut offset: Option<f64> = None;
    let mut meridiem: Option<bool> = None;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'(' {
            index += bytes[index..]
                .iter()
                .position(|byte| *byte == b')')
                .map_or(bytes.len() - index, |end| end + 1);
            continue;
        }
        if byte.is_ascii_alphabetic() {
            let end = index
                + bytes[index..]
                    .iter()
                    .take_while(|byte| byte.is_ascii_alphabetic())
                    .count();
            let word = &lower[index..end];
            index = end;
            if let Some(month) = MONTHS.iter().position(|month| {
                word.len() >= 3 && month.to_ascii_lowercase().starts_with(&word[..3])
            }) {
                month_name = Some(month as f64);
            } else if word == "am" || word == "pm" {
                meridiem = Some(word == "pm");
            } else if ["utc", "gmt", "ut", "z"].contains(&word) {
                offset.get_or_insert(0.0);
            } else if let Some(zone) = ["est", "edt", "cst", "cdt", "mst", "mdt", "pst", "pdt"]
                .iter()
                .position(|zone| *zone == word)
            {
                offset = Some(-(5.0 + (zone / 2) as f64 - (zone % 2) as f64) * MS_PER_HOUR);
            } else if word != "t"
                && !DAYS
                    .iter()
                    .any(|day| word.len() >= 3 && day.to_ascii_lowercase().starts_with(&word[..3]))
            {
                return f64::NAN;
            }
            continue;
        }
        let zoned = offset.is_some() || time.is_some();
        if (byte == b'+' || byte == b'-')
            && zoned
            && bytes.get(index + 1).is_some_and(u8::is_ascii_digit)
        {
            let start = index + 1;
            let end = start
                + bytes[start..]
                    .iter()
                    .take_while(|byte| byte.is_ascii_digit() || **byte == b':')
                    .count();
            let digits: String = lower[start..end]
                .chars()
                .filter(char::is_ascii_digit)
                .collect();
            let value: f64 = digits.parse().unwrap_or(0.0);
            let (hours, minutes) = if digits.len() <= 2 {
                (value, 0.0)
            } else {
                (floor(value / 100.0), modulo(value, 100.0))
            };
            let value = (hours * 60.0 + minutes) * MS_PER_MINUTE;
            offset = Some(if byte == b'-' { -value } else { value });
            index = end;
            continue;
        }
        if byte.is_ascii_digit() {
            let end = index
                + bytes[index..]
                    .iter()
                    .take_while(|byte| byte.is_ascii_digit())
                    .count();
            let value: f64 = lower[index..end].parse().unwrap_or(f64::NAN);
            if bytes.get(end) == Some(&b':') && time.is_none() {
                let mut parts = vec![value];
                let mut cursor = end;
                while bytes.get(cursor) == Some(&b':')
                    || (bytes.get(cursor) == Some(&b'.') && parts.len() == 3)
                {
                    let start = cursor + 1;
                    let stop = start
                        + bytes[start..]
                            .iter()
                            .take_while(|byte| byte.is_ascii_digit())
                            .count();
                    parts.push(lower[start..stop].parse().unwrap_or(f64::NAN));
                    cursor = stop;
                }
                parts.resize(4, 0.0);
                time = Some((parts[0], parts[1], parts[2], parts[3]));
                index = cursor;
            } else {
                numbers.push(value);
                index = end;
            }
            continue;
        }
        index += 1;
    }
    // Month names take the day and year in either order; numeric dates are
    // y/m/d when the first number cannot be a month or day, else m/d/y.
    let (year, month, date) = match (month_name, numbers.as_slice()) {
        (Some(month), [year]) => (*year, month, 1.0),
        (Some(month), [first, second]) if *first > 31.0 => (*first, month, *second),
        (Some(month), [first, second]) => (*second, month, *first),
        (None, [first, second, third]) if *first > 31.0 => (*first, second - 1.0, *third),
        (None, [first, second, third]) => (*third, first - 1.0, *second),
        _ => return f64::NAN,
    };
    let year = if (0.0..50.0).contains(&year) {
        year + 2000.0
    } else if (50.0..100.0).contains(&year) {
        year + 1900.0
    } else {
        year
    };
    let (mut hour, minute, second, millisecond) = time.unwrap_or((0.0, 0.0, 0.0, 0.0));
    match meridiem {
        Some(_) if hour > 12.0 => return f64::NAN,
        Some(true) if hour < 12.0 => hour += 12.0,
        Some(false) if hour == 12.0 => hour = 0.0,
        _ => {}
    }
    let late = hour > 24.0 || (hour == 24.0 && (minute, second, millisecond) != (0.0, 0.0, 0.0));
    if !(0.0..12.0).contains(&month)
        || !(1.0..=31.0).contains(&date)
        || late
        || minute > 59.0
        || second > 59.0
    {
        return f64::NAN;
    }
    let local = make_date(
        make_day(year, month, date),
        make_time(hour, minute, second, millisecond),
    );
    match offset {
        Some(offset) => local - offset,
        None => utc(realm, local),
    }
}

/// `Date.parse`.
fn parse(realm: &mut Realm, text: &JsStr) -> f64 {
    let text = text.to_rust_lossy();
    let text = text.trim();
    let parsed = parse_iso(realm, text.as_bytes()).unwrap_or_else(|| parse_legacy(realm, text));
    time_clip(parsed)
}

/// The time value of `new Date(value)`.
fn from_value(realm: &mut Realm, value: &Value) -> Result<f64, Thrown> {
    if let Some(time) = value.as_object().and_then(time_of) {
        return Ok(time);
    }
    match ops::to_primitive(realm, value, ops::Hint::Default)? {
        Value::String(text) => Ok(parse(realm, &text)),
        primitive => Ok(time_clip(ops::to_number(realm, &primitive)?)),
    }
}

/// The local time value of year, month and later components.
fn from_components(realm: &mut Realm, args: &[Value]) -> Result<f64, Thrown> {
    let mut values = Vec::with_capacity(7);
    for value in args.iter().take(7) {
        values.push(ops::to_number(realm, value)?);
    }
    let get = |index: usize, fallback: f64| values.get(index).copied().unwrap_or(fallback);
    let mut year = get(0, f64::NAN);
    if !year.is_nan() && (0.0..=99.0).contains(&integer(year)) {
        year = 1900.0 + integer(year);
    }
    Ok(make_date(
        make_day(year, get(1, 0.0), get(2, 1.0)),
        make_time(get(3, 0.0), get(4, 0.0), get(5, 0.0), get(6, 0.0)),
    ))
}

fn construct(realm: &mut Realm, target: Object, args: &[Value]) -> Result<Object, Thrown> {
    let time = match args {
        [] => now(),
        [value] => from_value(realm, value)?,
        args => {
            let local = from_components(realm, args)?;
            time_clip(utc(realm, local))
        }
    };
    let fallback = realm.date_prototype.clone();
    let prototype = match ops::get_named(realm, &Value::Object(target), "prototype")? {
        Value::Object(prototype) => prototype,
        _ => fallback,
    };
    Ok(Object::new(Some(prototype), Kind::Date(time)))
}

/// Which component a getter reads.
#[derive(Clone, Copy)]
enum Part {
    Year,
    Month,
    Date,
    Day,
    Hours,
    Minutes,
    Seconds,
    Milliseconds,
}

fn part(t: f64, part: Part) -> f64 {
    let (year, month, date) = date_parts(t);
    match part {
        Part::Year => year,
        Part::Month => month as f64,
        Part::Date => date,
        Part::Day => week_day(t) as f64,
        Part::Hours => hours(t),
        Part::Minutes => minutes(t),
        Part::Seconds => seconds(t),
        Part::Milliseconds => milliseconds(t),
    }
}

/// A setter's new value: components from `first` on are replaced by the
/// arguments, in the order of `[year, month, date, hours, minutes,
/// seconds, milliseconds]`; arguments are converted before the time value
/// is checked.
fn set_parts(
    realm: &mut Realm,
    this: &Value,
    args: &[Value],
    first: usize,
    count: usize,
    local: bool,
    method: &str,
) -> Completion {
    let time = date_value(realm, this, method)?;
    let mut given = Vec::with_capacity(count);
    for index in 0..count.min(args.len().max(1)) {
        given.push(number_arg(realm, args, index)?.unwrap_or(f64::NAN));
    }
    // setFullYear on an invalid date starts from +0.
    let base = if time.is_nan() {
        if first != 0 {
            return Ok(Value::Number(f64::NAN));
        }
        0.0
    } else if local {
        local_time(realm, time)
    } else {
        time
    };
    let (year, month, date) = date_parts(base);
    let mut components = [
        year,
        month as f64,
        date,
        hours(base),
        minutes(base),
        seconds(base),
        milliseconds(base),
    ];
    for (offset, value) in given.into_iter().enumerate() {
        components[first + offset] = value;
    }
    let [year, month, date, hour, minute, second, millisecond] = components;
    let mut new = make_date(
        make_day(year, month, date),
        make_time(hour, minute, second, millisecond),
    );
    if local {
        new = utc(realm, new);
    }
    set_value(this, time_clip(new))
}

/// A native method body.
type Native = fn(&mut Realm, Value, &[Value]) -> Completion;

fn getter(
    local: bool,
    which: Part,
    method: &'static str,
) -> impl Fn(&mut Realm, Value, &[Value]) -> Completion {
    move |realm, this, _| {
        let time = date_value(realm, &this, method)?;
        if time.is_nan() {
            return Ok(Value::Number(f64::NAN));
        }
        let time = if local { local_time(realm, time) } else { time };
        Ok(Value::Number(part(time, which)))
    }
}

fn formatter(
    format: Format,
    method: &'static str,
) -> impl Fn(&mut Realm, Value, &[Value]) -> Completion {
    move |realm, this, _| {
        let time = date_value(realm, &this, method)?;
        Ok(Value::str(&format_time(realm, time, format)))
    }
}

const GETTERS: [(&str, bool, Part); 16] = [
    ("getFullYear", true, Part::Year),
    ("getMonth", true, Part::Month),
    ("getDate", true, Part::Date),
    ("getDay", true, Part::Day),
    ("getHours", true, Part::Hours),
    ("getMinutes", true, Part::Minutes),
    ("getSeconds", true, Part::Seconds),
    ("getMilliseconds", true, Part::Milliseconds),
    ("getUTCFullYear", false, Part::Year),
    ("getUTCMonth", false, Part::Month),
    ("getUTCDate", false, Part::Date),
    ("getUTCDay", false, Part::Day),
    ("getUTCHours", false, Part::Hours),
    ("getUTCMinutes", false, Part::Minutes),
    ("getUTCSeconds", false, Part::Seconds),
    ("getUTCMilliseconds", false, Part::Milliseconds),
];

const SETTERS: [(&str, usize, usize, u32, bool); 14] = [
    ("setMilliseconds", 6, 1, 1, true),
    ("setSeconds", 5, 2, 2, true),
    ("setMinutes", 4, 3, 3, true),
    ("setHours", 3, 4, 4, true),
    ("setDate", 2, 1, 1, true),
    ("setMonth", 1, 2, 2, true),
    ("setFullYear", 0, 3, 3, true),
    ("setUTCMilliseconds", 6, 1, 1, false),
    ("setUTCSeconds", 5, 2, 2, false),
    ("setUTCMinutes", 4, 3, 3, false),
    ("setUTCHours", 3, 4, 4, false),
    ("setUTCDate", 2, 1, 1, false),
    ("setUTCMonth", 1, 2, 2, false),
    ("setUTCFullYear", 0, 3, 3, false),
];

const STATICS: [(&str, u32, Native); 3] = [
    ("now", 0, |_, _, _| Ok(Value::Number(now()))),
    ("parse", 1, |realm, _, args| {
        let text = ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?;
        Ok(Value::Number(parse(realm, &text)))
    }),
    ("UTC", 7, |realm, _, args| {
        let time = from_components(realm, args)?;
        Ok(Value::Number(time_clip(time)))
    }),
];

const METHODS: [(&str, u32, Native); 9] = [
    ("getTime", 0, |realm, this, _| {
        Ok(Value::Number(date_value(realm, &this, "getTime")?))
    }),
    ("valueOf", 0, |realm, this, _| {
        Ok(Value::Number(date_value(realm, &this, "valueOf")?))
    }),
    ("getTimezoneOffset", 0, |realm, this, _| {
        let time = date_value(realm, &this, "getTimezoneOffset")?;
        if time.is_nan() {
            return Ok(Value::Number(f64::NAN));
        }
        Ok(Value::Number(
            (time - local_time(realm, time)) / MS_PER_MINUTE,
        ))
    }),
    ("getYear", 0, |realm, this, _| {
        let time = date_value(realm, &this, "getYear")?;
        if time.is_nan() {
            return Ok(Value::Number(f64::NAN));
        }
        Ok(Value::Number(
            year_from_time(local_time(realm, time)) - 1900.0,
        ))
    }),
    ("setTime", 1, |realm, this, args| {
        date_value(realm, &this, "setTime")?;
        let time = number_arg(realm, args, 0)?.unwrap_or(f64::NAN);
        set_value(&this, time_clip(time))
    }),
    ("setYear", 1, |realm, this, args| {
        let time = date_value(realm, &this, "setYear")?;
        let year = number_arg(realm, args, 0)?.unwrap_or(f64::NAN);
        if year.is_nan() {
            return set_value(&this, f64::NAN);
        }
        let year = integer(year);
        let year = if (0.0..=99.0).contains(&year) {
            year + 1900.0
        } else {
            year
        };
        let base = if time.is_nan() {
            0.0
        } else {
            local_time(realm, time)
        };
        let (_, month, date) = date_parts(base);
        let day = make_day(year, month as f64, date);
        let local = make_date(day, modulo(base, MS_PER_DAY));
        let utc_time = utc(realm, local);
        set_value(&this, time_clip(utc_time))
    }),
    ("toISOString", 0, |realm, this, _| {
        let time = date_value(realm, &this, "toISOString")?;
        if time.is_nan() {
            return Err(realm.throw(ErrorKind::Range, "Invalid time value"));
        }
        Ok(Value::str(&iso_string(time)))
    }),
    ("toJSON", 1, |realm, this, _| {
        let object = ops::to_object(realm, &this)?;
        let object = Value::Object(object);
        let primitive = ops::to_primitive(realm, &object, ops::Hint::Number)?;
        if matches!(primitive, Value::Number(number) if !number.is_finite()) {
            return Ok(Value::Null);
        }
        let method = ops::get_named(realm, &object, "toISOString")?;
        ops::call(realm, &method, object, &[])
    }),
    ("[Symbol.toPrimitive]", 1, |realm, this, args| {
        let Value::Object(object) = &this else {
            return Err(realm.throw(
                ErrorKind::Type,
                "Date.prototype [ @@toPrimitive ] called on non-object",
            ));
        };
        let hint = match args.first() {
            Some(Value::String(hint)) => hint.to_rust_lossy(),
            _ => String::new(),
        };
        let hint = match hint.as_str() {
            "string" | "default" => ops::Hint::String,
            "number" => ops::Hint::Number,
            _ => return Err(realm.throw(ErrorKind::Type, "Invalid hint")),
        };
        ops::ordinary_to_primitive(realm, object, hint)
    }),
];

pub(crate) fn install(realm: &mut Realm) {
    let prototype = realm.date_prototype.clone();
    let constructor = realm.builtin_constructor(
        "Date",
        7,
        &prototype,
        |realm, _, _| Ok(Value::str(&format_time(realm, now(), Format::Full))),
        construct,
    );
    for (name, length, code) in STATICS {
        realm.method(&constructor, name, length, code);
    }
    for (name, length, code) in METHODS {
        if name == "[Symbol.toPrimitive]" {
            let function = realm.function(name, length, false, code);
            prototype.define(
                Key::Symbol(realm.symbols.to_primitive.clone()),
                Property {
                    slot: Slot::Data {
                        value: Value::Object(function),
                        writable: false,
                    },
                    enumerable: false,
                    configurable: true,
                },
            );
        } else {
            realm.method(&prototype, name, length, code);
        }
    }
    for (name, local, which) in GETTERS {
        realm.method(&prototype, name, 0, getter(local, which, name));
    }
    for (name, first, count, length, local) in SETTERS {
        realm.method(&prototype, name, length, move |realm, this, args| {
            set_parts(realm, &this, args, first, count, local, name)
        });
    }
    for (name, format) in [
        ("toString", Format::Full),
        ("toDateString", Format::Date),
        ("toTimeString", Format::Time),
        ("toUTCString", Format::Utc),
        ("toLocaleString", Format::LocaleFull),
        ("toLocaleDateString", Format::LocaleDate),
        ("toLocaleTimeString", Format::LocaleTime),
    ] {
        realm.method(&prototype, name, 0, formatter(format, name));
    }
    let utc_string = ops::get(
        realm,
        &Value::Object(prototype.clone()),
        &Key::name("toUTCString"),
    )
    .expect("toUTCString is installed");
    prototype.define(Key::name("toGMTString"), Property::hidden(utc_string));
    realm.set_global("Date", Value::Object(constructor));
}

/// The ISO text of a date object for inspection, or `Invalid Date`.
pub(crate) fn inspect_text(time: f64) -> String {
    if time.is_nan() {
        "Invalid Date".to_owned()
    } else {
        iso_string(time)
    }
}

#[cfg(test)]
mod tests;
