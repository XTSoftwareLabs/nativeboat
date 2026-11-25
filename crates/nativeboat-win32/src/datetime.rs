//! Date and time inputs as date and time picker controls: the pictures they
//! show in the application's locale, and their values as `<input>` element
//! values (`yyyy-mm-dd`, `HH:MM`, `yyyy-mm-ddTHH:MM`, `yyyy-mm`).

#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::{narrow, wide};
use core::cell::RefCell;
use nativeboat_ir::ControlKind;
use windows_sys::Win32::Foundation::SYSTEMTIME;
use windows_sys::Win32::Globalization::{
    GetLocaleInfoEx, LOCALE_SSHORTDATE, LOCALE_SSHORTTIME, LOCALE_SYEARMONTH,
};

local! {
    static LOCALE: RefCell<String> = RefCell::new(String::from("en-US"));
}

/// Sets the locale date and time pickers show their values in.
pub fn set_date_locale(locale: &str) {
    LOCALE.with_borrow_mut(|current| {
        if current != locale {
            locale.clone_into(current);
        }
    });
}

fn locale_picture(kind: u32, fallback: &str) -> String {
    let locale = LOCALE.with_borrow(|locale| wide(locale));
    let mut buffer = [0u16; 128];
    // SAFETY: the name is NUL-terminated and the buffer holds
    // `buffer.len()` characters.
    let length = unsafe {
        GetLocaleInfoEx(
            locale.as_ptr(),
            kind,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    if length > 1 {
        narrow(&buffer)
    } else {
        fallback.to_owned()
    }
}

/// Pads the fields of a picture to their full width, as date inputs show
/// them: `M/d/yyyy` becomes `MM/dd/yyyy`.
fn padded(picture: &str) -> String {
    let mut result = String::new();
    let characters: Vec<char> = picture.chars().collect();
    let mut index = 0;
    let mut quoted = false;
    while index < characters.len() {
        let character = characters[index];
        if character == '\'' {
            quoted = !quoted;
            result.push(character);
            index += 1;
            continue;
        }
        let mut count = 1;
        while characters.get(index + count) == Some(&character) {
            count += 1;
        }
        let width = match character {
            _ if quoted => count,
            'd' | 'M' | 'h' | 'H' | 'm' if count == 1 => 2,
            'y' if count < 4 => 4,
            't' if count == 1 => 2,
            _ => count,
        };
        result.extend(core::iter::repeat_n(character, width));
        index += count;
    }
    result
}

/// What an empty input shows: the picture's fields as placeholders, such as
/// `mm/dd/yyyy` and `--:-- --`.
fn placeholder(picture: &str, time: bool) -> String {
    let mut text = String::new();
    let mut quoted = false;
    for character in picture.chars() {
        if character == '\'' {
            quoted = !quoted;
            continue;
        }
        text.push(match character {
            _ if quoted => character,
            'd' | 'M' | 'y' if !time => character.to_ascii_lowercase(),
            'd' | 'M' | 'y' | 'h' | 'H' | 'm' | 's' | 't' => '-',
            other => other,
        });
    }
    format!("'{text}'")
}

/// The picture a picker shows, for a value or for an empty input.
pub(crate) fn picture(kind: ControlKind, empty: bool) -> String {
    let date = || padded(&locale_picture(LOCALE_SSHORTDATE, "MM/dd/yyyy"));
    let time = || padded(&locale_picture(LOCALE_SSHORTTIME, "hh:mm tt"));
    match kind {
        ControlKind::Time if empty => placeholder(&time(), true),
        ControlKind::Time => time(),
        ControlKind::DateTime if empty => {
            format!(
                "{}', '{}",
                placeholder(&date(), false).trim_end_matches('\''),
                placeholder(&time(), true).trim_start_matches('\'')
            )
        }
        ControlKind::DateTime => format!("{}', '{}", date(), time()),
        ControlKind::Month => {
            let month = locale_picture(LOCALE_SYEARMONTH, "MMMM yyyy");
            if empty {
                placeholder(&month, true)
            } else {
                month
            }
        }
        _ if empty => placeholder(&date(), false),
        _ => date(),
    }
}

fn numbers(text: &str, separators: &[char]) -> Option<Vec<u16>> {
    text.split(separators)
        .map(|part| part.parse::<u16>().ok())
        .collect()
}

/// The time an input value names, if it is valid for the kind of input.
pub(crate) fn system_time(kind: ControlKind, value: &str) -> Option<SYSTEMTIME> {
    let mut time = SYSTEMTIME {
        wYear: 2000,
        wMonth: 1,
        wDay: 1,
        ..SYSTEMTIME::default()
    };
    let (date, clock) = match kind {
        ControlKind::Time => (None, Some(value)),
        ControlKind::DateTime => {
            let (date, clock) = value.split_once(['T', ' '])?;
            (Some(date), Some(clock))
        }
        _ => (Some(value), None),
    };
    if let Some(date) = date {
        let parts = numbers(date, &['-'])?;
        let expected = if kind == ControlKind::Month { 2 } else { 3 };
        if parts.len() != expected {
            return None;
        }
        time.wYear = parts[0];
        time.wMonth = parts[1];
        time.wDay = parts.get(2).copied().unwrap_or(1);
        let days = match time.wMonth {
            2 if time.wYear.is_multiple_of(4)
                && (!time.wYear.is_multiple_of(100) || time.wYear.is_multiple_of(400)) =>
            {
                29
            }
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            1..=12 => 31,
            _ => return None,
        };
        if !(1601..=9999).contains(&time.wYear) || time.wDay == 0 || time.wDay > days {
            return None;
        }
    }
    if let Some(clock) = clock {
        let clock = clock.split('.').next().unwrap_or_default();
        let parts = numbers(clock, &[':'])?;
        if !(2..=3).contains(&parts.len()) || parts[0] > 23 || parts[1] > 59 {
            return None;
        }
        let second = parts.get(2).copied().unwrap_or(0);
        if second > 59 {
            return None;
        }
        time.wHour = parts[0];
        time.wMinute = parts[1];
        time.wSecond = second;
    }
    Some(time)
}

/// The input value of a picked time.
pub(crate) fn value(kind: ControlKind, time: &SYSTEMTIME) -> String {
    let date = format!("{:04}-{:02}-{:02}", time.wYear, time.wMonth, time.wDay);
    let clock = format!("{:02}:{:02}", time.wHour, time.wMinute);
    match kind {
        ControlKind::Time => clock,
        ControlKind::DateTime => format!("{date}T{clock}"),
        ControlKind::Month => format!("{:04}-{:02}", time.wYear, time.wMonth),
        _ => date,
    }
}

#[cfg(test)]
mod tests;
