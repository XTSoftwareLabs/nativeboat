//! The shell and system state: opening URLs and files, the recycle bin,
//! the color theme, displays, user idle time, power and data protection.

use crate::Error;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::wide;
use core::ptr;
use windows_sys::Win32::Foundation::{HLOCAL, LocalFree};
use windows_sys::Win32::Foundation::{LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::Security::Cryptography::{
    CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, GetSystemPowerStatus,
    SYSTEM_POWER_STATUS, SetThreadExecutionState,
};
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
use windows_sys::Win32::System::SystemInformation::GetTickCount;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
    SHFileOperationW, ShellExecuteW,
};

/// Resolves user folders through Windows, honoring folder redirection.
pub fn known_folder(name: &str) -> Option<String> {
    use windows_sys::Win32::UI::Shell::*;
    let folder = match name {
        "desktop" => FOLDERID_Desktop,
        "documents" => FOLDERID_Documents,
        "downloads" => FOLDERID_Downloads,
        "music" => FOLDERID_Music,
        "pictures" => FOLDERID_Pictures,
        "videos" => FOLDERID_Videos,
        _ => return None,
    };
    let mut path = ptr::null_mut();
    // SAFETY: the shell allocates a NUL-terminated string, released below.
    let result = unsafe {
        SHGetKnownFolderPath(
            &folder,
            KF_FLAG_DONT_VERIFY as u32,
            ptr::null_mut(),
            &mut path,
        )
    };
    if path.is_null() {
        return None;
    }
    let value = if result >= 0 {
        let mut length = 0;
        // SAFETY: a successful shell result is a NUL-terminated UTF-16 string.
        unsafe {
            while *path.add(length) != 0 {
                length += 1;
            }
            Some(String::from_utf16_lossy(core::slice::from_raw_parts(
                path, length,
            )))
        }
    } else {
        None
    };
    // SAFETY: the shell allocated the buffer with the COM task allocator.
    unsafe { windows_sys::Win32::System::Com::CoTaskMemFree(path.cast()) };
    value
}
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, MB_OK, MONITORINFOF_PRIMARY, SW_SHOWNORMAL,
};
use windows_sys::core::BOOL;

fn execute(file: &str, parameters: Option<&str>) -> Result<(), Error> {
    let operation = wide("open");
    let file = wide(file);
    let parameters = parameters.map(wide);
    // SAFETY: the strings are NUL-terminated and outlive the call.
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            parameters
                .as_ref()
                .map_or(ptr::null(), |text| text.as_ptr()),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Values above 32 mean success.
    if result as usize <= 32 {
        return Err(Error {
            message: format!("ShellExecuteW failed with error {}", result as usize),
        });
    }
    Ok(())
}

/// Opens a URL with its default application, such as the browser.
pub fn open_external(url: &str) -> Result<(), Error> {
    execute(url, None)
}

/// Opens a file or folder with its default application.
pub fn open_path(path: &str) -> Result<(), Error> {
    execute(path, None)
}

/// Opens the folder of a file in Explorer with the file selected.
pub fn show_item_in_folder(path: &str) -> Result<(), Error> {
    execute("explorer.exe", Some(&format!("/select,\"{path}\"")))
}

/// Moves a file or folder to the recycle bin.
pub fn trash(path: &str) -> Result<(), Error> {
    // The list of paths ends with an empty one.
    let mut from = wide(path);
    from.push(0);
    let mut operation = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16,
        ..Default::default()
    };
    // SAFETY: the operation and its path list outlive the call.
    let result = unsafe { SHFileOperationW(&mut operation) };
    if result != 0 || operation.fAnyOperationsAborted != 0 {
        return Err(Error {
            message: format!("SHFileOperationW failed with error {result}"),
        });
    }
    Ok(())
}

/// Plays the default system sound.
pub fn beep() {
    // SAFETY: plays a sound asynchronously.
    unsafe { MessageBeep(MB_OK) };
}

/// Whether the user chose the dark theme for applications.
pub fn dark_mode() -> bool {
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let name = wide("AppsUseLightTheme");
    let mut value = 1_u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: reads a DWORD into `value`, whose size goes with it.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            ptr::from_mut(&mut value).cast(),
            &mut size,
        )
    };
    status == 0 && value == 0
}

/// A display, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Display {
    /// The monitor handle as a number, unique while it is connected.
    pub id: u64,
    /// The whole display.
    pub bounds: (i32, i32, i32, i32),
    /// The display without the taskbar.
    pub work_area: (i32, i32, i32, i32),
    /// Device pixels per device-independent pixel.
    pub scale: f32,
    /// Whether it is the primary display.
    pub primary: bool,
}

fn rectangle(rect: RECT) -> (i32, i32, i32, i32) {
    (
        rect.left,
        rect.top,
        rect.right - rect.left,
        rect.bottom - rect.top,
    )
}

unsafe extern "system" fn monitor(handle: HMONITOR, _: HDC, _: *mut RECT, list: LPARAM) -> BOOL {
    // SAFETY: `displays` passes a pointer to its vector.
    let list = unsafe { &mut *(list as *mut Vec<Display>) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let (mut dpi, mut unused) = (96, 96);
    // SAFETY: the monitor handle comes from the enumeration and the
    // outputs are writable.
    unsafe {
        GetMonitorInfoW(handle, &mut info);
        GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi, &mut unused);
    }
    list.push(Display {
        id: handle as u64,
        bounds: rectangle(info.rcMonitor),
        work_area: rectangle(info.rcWork),
        scale: dpi as f32 / 96.0,
        primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
    });
    1
}

/// The connected displays.
pub fn displays() -> Vec<Display> {
    let mut list: Vec<Display> = Vec::new();
    // SAFETY: the callback only runs during the call and receives a
    // pointer to `list`.
    unsafe {
        EnumDisplayMonitors(
            ptr::null_mut(),
            ptr::null(),
            Some(monitor),
            ptr::from_mut(&mut list) as LPARAM,
        )
    };
    list
}

/// The cursor position on the screen, in physical pixels.
pub fn cursor_position() -> (i32, i32) {
    let mut point = Default::default();
    // SAFETY: writes the position into `point`.
    unsafe { GetCursorPos(&mut point) };
    (point.x, point.y)
}

/// Seconds since the user last used the keyboard or mouse.
pub fn idle_seconds() -> u32 {
    let mut info = LASTINPUTINFO {
        cbSize: size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: the structure is writable and sized.
    unsafe { GetLastInputInfo(&mut info) };
    // SAFETY: reads the tick count.
    let now = unsafe { GetTickCount() };
    now.wrapping_sub(info.dwTime) / 1000
}

/// Whether the computer runs on battery power.
pub fn on_battery() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    // SAFETY: the structure is writable.
    let ok = unsafe { GetSystemPowerStatus(&mut status) };
    ok != 0 && status.ACLineStatus == 0
}

/// Runs DPAPI on data and copies the result out of the memory it returns.
fn dpapi(data: &[u8], call: &str, protect: bool) -> Result<Vec<u8>, Error> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    // SAFETY: the input outlives the call, which only reads it; the output
    // is allocated by DPAPI and freed below.
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(Error::last(call));
    }
    // SAFETY: DPAPI returned `cbData` bytes at `pbData`, freed after copying.
    unsafe {
        let bytes = core::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as HLOCAL);
        Ok(bytes)
    }
}

/// Encrypts data so that only the current Windows user can decrypt it.
pub fn protect(data: &[u8]) -> Result<Vec<u8>, Error> {
    dpapi(data, "CryptProtectData", true)
}

/// Decrypts data that [`protect`] encrypted for the current user.
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, Error> {
    dpapi(data, "CryptUnprotectData", false)
}

/// What the computer must not do while the application asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Awake {
    /// Sleeping is allowed again.
    Allow,
    /// The computer stays awake.
    System,
    /// The computer and the display stay on.
    Display,
}

/// Keeps the computer, or also the display, from sleeping. The state
/// belongs to the calling thread, the application's UI thread.
pub fn keep_awake(awake: Awake) {
    let flags = match awake {
        Awake::Allow => ES_CONTINUOUS,
        Awake::System => ES_CONTINUOUS | ES_SYSTEM_REQUIRED,
        Awake::Display => ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED,
    };
    // SAFETY: only sets the calling thread's execution state.
    unsafe { SetThreadExecutionState(flags) };
}

#[cfg(test)]
mod tests;
