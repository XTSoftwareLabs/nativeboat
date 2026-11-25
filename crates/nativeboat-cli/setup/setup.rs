//! The Windows installer nativeboat makes for an application, in place of
//! electron-builder's NSIS installer: the application's files, compressed
//! after this program's image, installed for the current user with
//! shortcuts, an uninstaller and an Add or Remove Programs entry. It takes
//! the NSIS options electron-updater passes: `/S` installs silently,
//! `--updated` installs into the folder of the application that started it
//! once that application exits, `--force-run` starts the application
//! afterwards and `/D=` names the folder. The uninstaller is this program
//! with the list of installed files in place of the files.
//!
//! With `NATIVEBOAT_SETUP_STATE` set to a folder, the registry, the known
//! folders and the shortcuts are kept in that folder instead.
//!
//! The runtime libraries include this crate precompiled; each installer
//! links it with the application's icon and version information.

#![no_std]
// Windows structures are declared whole, including fields this program
// leaves alone.
#![allow(dead_code)]

extern crate alloc;
extern crate nativeboat_start;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;
use miniz_oxide::inflate::stream::{InflateState, inflate};
use miniz_oxide::{DataFormat, MZFlush, MZStatus};

type Handle = *mut c_void;

const INVALID_HANDLE: Handle = -1isize as Handle;
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_DELETE: u32 = 4;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const SYNCHRONIZE: u32 = 0x0010_0000;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const HKEY_CURRENT_USER: Handle = 0x8000_0001usize as Handle;
const KEY_ALL_ACCESS: u32 = 0xf003f;
const REG_SZ: u32 = 1;
const REG_DWORD: u32 = 4;
const MB_YESNO: u32 = 4;
const MB_ICONERROR: u32 = 0x10;
const MB_ICONQUESTION: u32 = 0x20;
const IDYES: i32 = 6;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const MAGIC: &[u8; 8] = b"NBSETUP\0";
const END_MAGIC: &[u8; 8] = b"NBSETUP\x01";

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);

const FOLDERID_PROGRAMS: Guid = Guid(0xa77f5d77, 0x2e2b, 0x44c3, [0xa6, 0xa2, 0xab, 0xa6, 0x01, 0x05, 0x4a, 0x51]);
const FOLDERID_DESKTOP: Guid = Guid(0xb4bfcc3a, 0xdb2c, 0x424c, [0xb0, 0x29, 0x7f, 0xe9, 0x9a, 0x87, 0xc6, 0x41]);
const FOLDERID_LOCAL_APP_DATA: Guid = Guid(0xf1b32785, 0x6fba, 0x4fcf, [0x9d, 0x55, 0x7b, 0x8e, 0x7f, 0x15, 0x70, 0x91]);
const FOLDERID_ROAMING_APP_DATA: Guid = Guid(0x3eb685db, 0x65f9, 0x4cf6, [0xa0, 0x3a, 0xe3, 0xef, 0x65, 0x72, 0x9f, 0x3d]);
const CLSID_SHELL_LINK: Guid = Guid(0x00021401, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_SHELL_LINK: Guid = Guid(0x000214f9, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_PERSIST_FILE: Guid = Guid(0x0000010b, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);

#[repr(C)]
struct StartupInfo {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    x_size: u32,
    y_size: u32,
    x_count_chars: u32,
    y_count_chars: u32,
    fill_attribute: u32,
    flags: u32,
    show_window: u16,
    reserved2_count: u16,
    reserved2: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}

#[repr(C)]
struct ProcessInformation {
    process: Handle,
    thread: Handle,
    process_id: u32,
    thread_id: u32,
}

#[repr(C)]
struct ProcessBasicInformation {
    exit_status: isize,
    peb: *mut c_void,
    affinity: usize,
    priority: isize,
    process_id: usize,
    parent_id: usize,
}

#[repr(C)]
struct FindData {
    attributes: u32,
    times: [u32; 6],
    size_high: u32,
    size_low: u32,
    reserved: [u32; 2],
    name: [u16; 260],
    alternate: [u16; 14],
    obsolete: [u32; 2],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCommandLineW() -> *const u16;
    fn GetModuleFileNameW(module: Handle, name: *mut u16, size: u32) -> u32;
    fn CreateFileW(name: *const u16, access: u32, share: u32, security: *mut c_void, disposition: u32, flags: u32, template: Handle) -> Handle;
    fn ReadFile(file: Handle, buffer: *mut u8, length: u32, read: *mut u32, overlapped: *mut c_void) -> i32;
    fn WriteFile(file: Handle, buffer: *const u8, length: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
    fn SetFilePointerEx(file: Handle, distance: i64, position: *mut i64, method: u32) -> i32;
    fn GetFileSizeEx(file: Handle, size: *mut i64) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn CreateDirectoryW(name: *const u16, security: *mut c_void) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn RemoveDirectoryW(name: *const u16) -> i32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn FindFirstFileW(name: *const u16, data: *mut FindData) -> Handle;
    fn FindNextFileW(find: Handle, data: *mut FindData) -> i32;
    fn FindClose(find: Handle) -> i32;
    fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: i32) -> i32;
    fn GetTempPathW(size: u32, buffer: *mut u16) -> u32;
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
    fn GetCurrentProcess() -> Handle;
    fn OpenProcess(access: u32, inherit: i32, id: u32) -> Handle;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn QueryFullProcessImageNameW(process: Handle, flags: u32, name: *mut u16, size: *mut u32) -> i32;
    fn CreateProcessW(application: *const u16, command: *mut u16, process: *mut c_void, thread: *mut c_void, inherit: i32, flags: u32, environment: *mut c_void, directory: *const u16, startup: *mut StartupInfo, information: *mut ProcessInformation) -> i32;
    fn Sleep(milliseconds: u32);
    fn GetModuleHandleW(name: *const u16) -> Handle;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationProcess(process: Handle, class: u32, information: *mut c_void, length: u32, returned: *mut u32) -> i32;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegCreateKeyExW(key: Handle, subkey: *const u16, reserved: u32, class: *const u16, options: u32, access: u32, security: *mut c_void, result: *mut Handle, disposition: *mut u32) -> i32;
    fn RegSetValueExW(key: Handle, name: *const u16, reserved: u32, kind: u32, data: *const u8, size: u32) -> i32;
    fn RegGetValueW(key: Handle, subkey: *const u16, name: *const u16, flags: u32, kind: *mut u32, data: *mut c_void, size: *mut u32) -> i32;
    fn RegDeleteTreeW(key: Handle, subkey: *const u16) -> i32;
    fn RegCloseKey(key: Handle) -> i32;
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(id: *const Guid, flags: u32, token: Handle, path: *mut *mut u16) -> i32;
    fn CommandLineToArgvW(command: *const u16, count: *mut i32) -> *mut *mut u16;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, model: u32) -> i32;
    fn CoCreateInstance(class: *const Guid, outer: *mut c_void, context: u32, interface: *const Guid, object: *mut *mut c_void) -> i32;
    fn CoTaskMemFree(memory: *mut c_void);
}

#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct PaintStruct {
    dc: Handle,
    erase: i32,
    paint: Rect,
    restore: i32,
    update: i32,
    reserved: [u8; 32],
}

#[repr(C)]
struct Message {
    window: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: [i32; 2],
    private: u32,
}

type WindowProcedure = unsafe extern "system" fn(Handle, u32, usize, isize) -> isize;

#[repr(C)]
struct WindowClass {
    size: u32,
    style: u32,
    procedure: WindowProcedure,
    class_extra: i32,
    window_extra: i32,
    instance: Handle,
    icon: Handle,
    cursor: Handle,
    background: Handle,
    menu_name: *const u16,
    class_name: *const u16,
    icon_small: Handle,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MessageBoxW(window: Handle, text: *const u16, caption: *const u16, kind: u32) -> i32;
    fn RegisterClassExW(class: *const WindowClass) -> u16;
    fn CreateWindowExW(ex_style: u32, class: *const u16, title: *const u16, style: u32, x: i32, y: i32, width: i32, height: i32, parent: Handle, menu: Handle, instance: Handle, parameter: *mut c_void) -> Handle;
    fn DefWindowProcW(window: Handle, message: u32, wparam: usize, lparam: isize) -> isize;
    fn DestroyWindow(window: Handle) -> i32;
    fn PeekMessageW(message: *mut Message, window: Handle, first: u32, last: u32, remove: u32) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
    fn InvalidateRect(window: Handle, rect: *const Rect, erase: i32) -> i32;
    fn UpdateWindow(window: Handle) -> i32;
    fn BeginPaint(window: Handle, paint: *mut PaintStruct) -> Handle;
    fn EndPaint(window: Handle, paint: *const PaintStruct) -> i32;
    fn GetClientRect(window: Handle, rect: *mut Rect) -> i32;
    fn FillRect(dc: Handle, rect: *const Rect, brush: Handle) -> i32;
    fn DrawTextW(dc: Handle, text: *const u16, length: i32, rect: *mut Rect, format: u32) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetSysColorBrush(index: i32) -> Handle;
    fn GetDpiForSystem() -> u32;
    fn LoadCursorW(instance: Handle, name: *const u16) -> Handle;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFontW(height: i32, width: i32, escapement: i32, orientation: i32, weight: i32, italic: u32, underline: u32, strike: u32, charset: u32, precision: u32, clip: u32, quality: u32, pitch: u32, face: *const u16) -> Handle;
    fn SelectObject(dc: Handle, object: Handle) -> Handle;
    fn DeleteObject(object: Handle) -> i32;
    fn SetBkMode(dc: Handle, mode: i32) -> i32;
}

/// How far the install is, in thousandths, and the line the window shows.
static PROGRESS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static LABEL: core::sync::atomic::AtomicPtr<u16> = core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

unsafe extern "system" fn progress_procedure(window: Handle, message: u32, wparam: usize, lparam: isize) -> isize {
    const WM_PAINT: u32 = 0x000f;
    const WM_CLOSE: u32 = 0x0010;
    match message {
        WM_PAINT => {
            let mut paint: PaintStruct = unsafe { core::mem::zeroed() };
            let mut client = Rect { left: 0, top: 0, right: 0, bottom: 0 };
            // SAFETY: the window is live and painting.
            unsafe {
                let dc = BeginPaint(window, &mut paint);
                GetClientRect(window, &mut client);
                FillRect(dc, &client, GetSysColorBrush(5));
                let scale = |value: i32| value * GetDpiForSystem() as i32 / 96;
                let font = CreateFontW(-scale(12), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide("Segoe UI").as_ptr());
                let previous = SelectObject(dc, font);
                SetBkMode(dc, 1);
                let mut text = Rect { left: scale(20), top: scale(18), right: client.right - scale(20), bottom: scale(44) };
                let label = LABEL.load(core::sync::atomic::Ordering::Relaxed);
                if !label.is_null() {
                    DrawTextW(dc, label, -1, &mut text, 0x20 | 0x8000);
                }
                let bar = Rect { left: scale(20), top: scale(52), right: client.right - scale(20), bottom: scale(66) };
                FillRect(dc, &bar, GetSysColorBrush(16));
                let inner = Rect { left: bar.left + 1, top: bar.top + 1, right: bar.right - 1, bottom: bar.bottom - 1 };
                FillRect(dc, &inner, GetSysColorBrush(15));
                let done = PROGRESS.load(core::sync::atomic::Ordering::Relaxed) as i32;
                let filled = Rect { right: inner.left + (inner.right - inner.left) * done / 1000, ..inner };
                FillRect(dc, &filled, GetSysColorBrush(13));
                SelectObject(dc, previous);
                DeleteObject(font);
                EndPaint(window, &paint);
            }
            0
        }
        // The install finishes before the window goes.
        WM_CLOSE => 0,
        // SAFETY: the arguments are the message's own.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

/// The window an interactive install shows while it copies files.
struct Progress(Handle);

impl Progress {
    fn open(title: &str, label: &str) -> Option<Progress> {
        LABEL.store(Box::leak(wide(label).into_boxed_slice()).as_mut_ptr(), core::sync::atomic::Ordering::Relaxed);
        let class_name = wide("NativeboatSetup");
        // SAFETY: the class and window names are null-terminated and live for
        // the calls; IDC_ARROW is a system cursor.
        unsafe {
            let class = WindowClass {
                size: core::mem::size_of::<WindowClass>() as u32,
                style: 0,
                procedure: progress_procedure,
                class_extra: 0,
                window_extra: 0,
                instance: GetModuleHandleW(core::ptr::null()),
                icon: core::ptr::null_mut(),
                cursor: LoadCursorW(core::ptr::null_mut(), 32512 as *const u16),
                background: core::ptr::null_mut(),
                menu_name: core::ptr::null(),
                class_name: class_name.as_ptr(),
                icon_small: core::ptr::null_mut(),
            };
            RegisterClassExW(&class);
            let scale = |value: i32| value * GetDpiForSystem() as i32 / 96;
            let (width, height) = (scale(460), scale(130));
            // WS_CAPTION | WS_VISIBLE, centered on the primary screen.
            let window = CreateWindowExW(0, class_name.as_ptr(), wide(title).as_ptr(), 0x00c0_0000 | 0x1000_0000, (GetSystemMetrics(0) - width) / 2, (GetSystemMetrics(1) - height) / 2, width, height, core::ptr::null_mut(), core::ptr::null_mut(), class.instance, core::ptr::null_mut());
            if window.is_null() {
                return None;
            }
            UpdateWindow(window);
            let progress = Progress(window);
            progress.pump();
            Some(progress)
        }
    }

    fn set(&self, done: u64, total: u64) {
        let permille = if total == 0 { 1000 } else { (done.saturating_mul(1000) / total) as u32 };
        PROGRESS.store(permille.min(1000), core::sync::atomic::Ordering::Relaxed);
        // SAFETY: the window is live.
        unsafe { InvalidateRect(self.0, core::ptr::null(), 0) };
        self.pump();
    }

    fn pump(&self) {
        let mut message: Message = unsafe { core::mem::zeroed() };
        // SAFETY: PM_REMOVE; the message is this thread's.
        while unsafe { PeekMessageW(&mut message, core::ptr::null_mut(), 0, 0, 1) } != 0 {
            // SAFETY: as above.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        // SAFETY: the window is this thread's.
        unsafe { DestroyWindow(self.0) };
    }
}

/// A null-terminated UTF-16 copy of `text`.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The text of a null-terminated UTF-16 string.
fn narrow(text: *const u16) -> String {
    let mut length = 0;
    // SAFETY: callers pass null-terminated strings.
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    // SAFETY: `length` units were just read.
    String::from_utf16_lossy(unsafe { core::slice::from_raw_parts(text, length) })
}

fn join(directory: &str, name: &str) -> String {
    if directory.ends_with('\\') {
        format!("{directory}{name}")
    } else {
        format!("{directory}\\{name}")
    }
}

fn parent(path: &str) -> Option<&str> {
    path.rfind('\\').map(|index| &path[..index])
}

fn file_name(path: &str) -> &str {
    path.rfind('\\').map_or(path, |index| &path[index + 1..])
}

fn environment(name: &str) -> Option<String> {
    let mut buffer = vec![0u16; 32768];
    // SAFETY: the buffer holds `len` units.
    let length = unsafe { GetEnvironmentVariableW(wide(name).as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) };
    (length != 0 && (length as usize) < buffer.len()).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
}

/// The folder test runs keep the registry, known folders and shortcuts in.
fn state() -> Option<String> {
    environment("NATIVEBOAT_SETUP_STATE").filter(|state| !state.is_empty())
}

fn known_folder(id: &Guid, test_name: &str) -> String {
    if let Some(state) = state() {
        return join(&state, test_name);
    }
    let mut path = core::ptr::null_mut();
    // SAFETY: the shell writes a CoTaskMem string on success.
    if unsafe { SHGetKnownFolderPath(id, 0, core::ptr::null_mut(), &mut path) } != 0 {
        return String::new();
    }
    let text = narrow(path);
    // SAFETY: the shell allocated the string.
    unsafe { CoTaskMemFree(path.cast()) };
    text
}

fn exists(path: &str) -> bool {
    // SAFETY: the name is null-terminated.
    (unsafe { GetFileAttributesW(wide(path).as_ptr()) }) != u32::MAX
}

fn is_directory(path: &str) -> bool {
    // SAFETY: the name is null-terminated.
    let attributes = unsafe { GetFileAttributesW(wide(path).as_ptr()) };
    attributes != u32::MAX && attributes & FILE_ATTRIBUTE_DIRECTORY != 0
}

fn create_directories(path: &str) {
    if path.is_empty() || is_directory(path) {
        return;
    }
    if let Some(above) = parent(path) {
        create_directories(above);
    }
    // SAFETY: the name is null-terminated.
    unsafe { CreateDirectoryW(wide(path).as_ptr(), core::ptr::null_mut()) };
}

/// Retries `step` while a file stays in use, for up to half a minute.
fn retried(mut step: impl FnMut() -> bool) -> bool {
    for _ in 0..150 {
        if step() {
            return true;
        }
        // SAFETY: Sleep has no preconditions.
        unsafe { Sleep(200) };
    }
    false
}

fn delete_file(path: &str) -> bool {
    let name = wide(path);
    // SAFETY: the name is null-terminated.
    !exists(path) || retried(|| unsafe { DeleteFileW(name.as_ptr()) } != 0)
}

/// Deletes a folder and everything in it.
fn delete_tree(path: &str) {
    let mut data: FindData = unsafe { core::mem::zeroed() };
    // SAFETY: the pattern is null-terminated and `data` is writable.
    let find = unsafe { FindFirstFileW(wide(&join(path, "*")).as_ptr(), &mut data) };
    if find != INVALID_HANDLE {
        loop {
            let name = String::from_utf16_lossy(&data.name[..data.name.iter().position(|unit| *unit == 0).unwrap_or(260)]);
            if name != "." && name != ".." {
                let child = join(path, &name);
                if data.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    delete_tree(&child);
                } else {
                    delete_file(&child);
                }
            }
            // SAFETY: the search handle is live.
            if unsafe { FindNextFileW(find, &mut data) } == 0 {
                break;
            }
        }
        // SAFETY: the search handle is live.
        unsafe { FindClose(find) };
    }
    // SAFETY: the name is null-terminated.
    unsafe { RemoveDirectoryW(wide(path).as_ptr()) };
}

/// Removes the empty folders on the way from `path` up to `root`.
fn remove_empty_parents(path: &str, root: &str) {
    let mut current = parent(path);
    while let Some(directory) = current {
        if directory.len() <= root.len() {
            break;
        }
        // SAFETY: the name is null-terminated; non-empty folders stay.
        if unsafe { RemoveDirectoryW(wide(directory).as_ptr()) } == 0 {
            break;
        }
        current = parent(directory);
    }
}

struct File(Handle);

impl File {
    fn open(path: &str) -> Option<File> {
        // SAFETY: the name is null-terminated.
        let handle = unsafe { CreateFileW(wide(path).as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_DELETE, core::ptr::null_mut(), OPEN_EXISTING, 0, core::ptr::null_mut()) };
        (handle != INVALID_HANDLE).then_some(File(handle))
    }

    fn create(path: &str) -> Option<File> {
        let name = wide(path);
        let mut handle = INVALID_HANDLE;
        retried(|| {
            // SAFETY: the name is null-terminated.
            handle = unsafe { CreateFileW(name.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), CREATE_ALWAYS, 0, core::ptr::null_mut()) };
            handle != INVALID_HANDLE
        });
        (handle != INVALID_HANDLE).then_some(File(handle))
    }

    fn size(&self) -> u64 {
        let mut size = 0i64;
        // SAFETY: the handle is live.
        unsafe { GetFileSizeEx(self.0, &mut size) };
        size as u64
    }

    fn seek(&self, offset: u64) -> bool {
        // SAFETY: the handle is live.
        unsafe { SetFilePointerEx(self.0, offset as i64, core::ptr::null_mut(), 0) != 0 }
    }

    fn read(&self, buffer: &mut [u8]) -> usize {
        let mut read = 0u32;
        // SAFETY: the buffer holds `len` bytes.
        unsafe { ReadFile(self.0, buffer.as_mut_ptr(), buffer.len() as u32, &mut read, core::ptr::null_mut()) };
        read as usize
    }

    fn read_exact(&self, buffer: &mut [u8]) -> bool {
        let mut done = 0;
        while done < buffer.len() {
            let read = self.read(&mut buffer[done..]);
            if read == 0 {
                return false;
            }
            done += read;
        }
        true
    }

    fn write(&self, data: &[u8]) -> bool {
        let mut done = 0;
        while done < data.len() {
            let mut written = 0u32;
            let chunk = (data.len() - done).min(1 << 20);
            // SAFETY: `chunk` bytes are readable.
            if unsafe { WriteFile(self.0, data[done..].as_ptr(), chunk as u32, &mut written, core::ptr::null_mut()) } == 0 || written == 0 {
                return false;
            }
            done += written as usize;
        }
        true
    }
}

impl Drop for File {
    fn drop(&mut self) {
        // SAFETY: the handle is owned.
        unsafe { CloseHandle(self.0) };
    }
}

/// An entry of the payload: a file's path inside the application folder,
/// its size and where its compressed bytes are.
struct Entry {
    path: String,
    size: u64,
    offset: u64,
    compressed: u64,
}

/// This program's payload: settings and files.
struct Payload {
    settings: Vec<(String, String)>,
    entries: Vec<Entry>,
    /// Where the payload starts, which is where this program's image ends.
    start: u64,
}

impl Payload {
    fn setting(&self, key: &str) -> &str {
        self.settings
            .iter()
            .find(|(name, _)| name == key)
            .map_or("", |(_, value)| value)
    }

    fn flag(&self, key: &str) -> bool {
        self.setting(key) == "1"
    }
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from(u32_at(bytes, at)) | (u64::from(u32_at(bytes, at + 4)) << 32)
}

/// Where an executable's signature starts, or its size: the payload ends
/// there, since signing appends the certificate after it.
fn payload_end(file: &File) -> u64 {
    let size = file.size();
    let mut header = [0u8; 4096];
    let length = header.len().min(size as usize);
    if !file.seek(0) || !file.read_exact(&mut header[..length]) || length < 0x40 {
        return size;
    }
    // The PE32+ optional header's security directory: a file offset.
    let security = u32_at(&header, 0x3c) as usize + 0x18 + 0x90;
    if security + 8 > length {
        return size;
    }
    let offset = u64::from(u32_at(&header, security));
    if offset != 0 && offset <= size { offset } else { size }
}

fn read_payload(path: &str) -> Option<Payload> {
    let file = File::open(path)?;
    let end = payload_end(&file);
    let mut trailer = [0u8; 16];
    if end < 16 || !file.seek(end - 16) || !file.read_exact(&mut trailer) || &trailer[8..] != END_MAGIC {
        return None;
    }
    let start = u64_at(&trailer, 0);
    let mut head = [0u8; 16];
    if !file.seek(start) || !file.read_exact(&mut head) || &head[..8] != MAGIC {
        return None;
    }
    let mut settings_bytes = vec![0u8; u32_at(&head, 12) as usize];
    if !file.read_exact(&mut settings_bytes) {
        return None;
    }
    let settings = String::from_utf8_lossy(&settings_bytes)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
    let mut count = [0u8; 4];
    if !file.read_exact(&mut count) {
        return None;
    }
    let mut position = start + 16 + settings_bytes.len() as u64 + 4;
    let mut entries = Vec::new();
    for _ in 0..u32_at(&count, 0) {
        let mut length = [0u8; 4];
        if !file.seek(position) || !file.read_exact(&mut length) {
            return None;
        }
        let mut name = vec![0u8; u32_at(&length, 0) as usize];
        let mut sizes = [0u8; 16];
        if !file.read_exact(&mut name) || !file.read_exact(&mut sizes) {
            return None;
        }
        let offset = position + 4 + name.len() as u64 + 16;
        let compressed = u64_at(&sizes, 8);
        entries.push(Entry {
            path: String::from_utf8_lossy(&name).into_owned(),
            size: u64_at(&sizes, 0),
            offset,
            compressed,
        });
        position = offset + compressed;
    }
    Some(Payload { settings, entries, start })
}

/// Inflates an entry from `source` into the file at `path`.
fn extract(source: &File, entry: &Entry, path: &str) -> bool {
    let Some(target) = File::create(path) else {
        return false;
    };
    if entry.size == 0 {
        return true;
    }
    if !source.seek(entry.offset) {
        return false;
    }
    let mut state: Box<InflateState> = InflateState::new_boxed(DataFormat::Raw);
    let mut input = vec![0u8; 1 << 16];
    let mut output = vec![0u8; 1 << 18];
    let mut remaining = entry.compressed;
    let (mut filled, mut used) = (0usize, 0usize);
    let mut written = 0u64;
    loop {
        if used == filled && remaining > 0 {
            let wanted = (input.len() as u64).min(remaining) as usize;
            if !source.read_exact(&mut input[..wanted]) {
                return false;
            }
            remaining -= wanted as u64;
            (filled, used) = (wanted, 0);
        }
        let result = inflate(&mut state, &input[used..filled], &mut output, MZFlush::None);
        used += result.bytes_consumed;
        if !target.write(&output[..result.bytes_written]) {
            return false;
        }
        written += result.bytes_written as u64;
        let progress = result.bytes_consumed > 0 || result.bytes_written > 0;
        match result.status {
            Ok(MZStatus::StreamEnd) => return written == entry.size,
            _ if progress || (used == filled && remaining > 0) => {}
            // The stream ended early or is damaged.
            _ => return false,
        }
    }
}

/// The registry, or in test runs a text file of `key\name=value` lines.
fn registry_set(key: &str, name: &str, value: &str) {
    if let Some(state) = state() {
        let path = join(&state, "registry.txt");
        let mut text = read_text(&path);
        text.push_str(&format!("{key}\\{name}={value}\n"));
        write_text(&path, &text);
        return;
    }
    let mut handle = core::ptr::null_mut();
    // SAFETY: the key name is null-terminated and `handle` is writable.
    if unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, wide(key).as_ptr(), 0, core::ptr::null(), 0, KEY_ALL_ACCESS, core::ptr::null_mut(), &mut handle, core::ptr::null_mut()) } != 0 {
        return;
    }
    if let Some(number) = value.strip_prefix("dword:") {
        let number: u32 = number.parse().unwrap_or(0);
        // SAFETY: four bytes are readable.
        unsafe { RegSetValueExW(handle, wide(name).as_ptr(), 0, REG_DWORD, (&number as *const u32).cast(), 4) };
    } else {
        let data = wide(value);
        // SAFETY: the string's bytes are readable.
        unsafe { RegSetValueExW(handle, wide(name).as_ptr(), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32) };
    }
    // SAFETY: the key is open.
    unsafe { RegCloseKey(handle) };
}

fn registry_get(key: &str, name: &str) -> Option<String> {
    if let Some(state) = state() {
        let prefix = format!("{key}\\{name}=");
        return read_text(&join(&state, "registry.txt"))
            .lines()
            .rev()
            .find_map(|line| line.strip_prefix(&prefix).map(String::from));
    }
    let mut buffer = vec![0u16; 32768];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: the buffer holds `size` bytes; RRF_RT_REG_SZ.
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, wide(key).as_ptr(), wide(name).as_ptr(), 2, core::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size) };
    (status == 0).then(|| narrow(buffer.as_ptr()))
}

fn registry_delete(key: &str) {
    if let Some(state) = state() {
        let path = join(&state, "registry.txt");
        let prefix = format!("{key}\\");
        let text: String = read_text(&path)
            .lines()
            .filter(|line| !line.starts_with(&prefix))
            .map(|line| format!("{line}\n"))
            .collect();
        write_text(&path, &text);
        return;
    }
    // SAFETY: the key name is null-terminated.
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, wide(key).as_ptr()) };
}

fn read_text(path: &str) -> String {
    let Some(file) = File::open(path) else {
        return String::new();
    };
    let mut bytes = vec![0u8; file.size() as usize];
    file.read_exact(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn write_text(path: &str, text: &str) {
    if let Some(file) = File::create(path) {
        file.write(text.as_bytes());
    }
}

/// Calls slot `index` of a COM object's table.
///
/// # Safety
/// `object` is a live COM object whose slot has signature `F`.
unsafe fn slot<F: Copy>(object: *mut c_void, index: usize) -> F {
    // SAFETY: per the contract, the object's first field is its table.
    unsafe {
        let table = *(object as *const *const usize);
        core::mem::transmute_copy(&*table.add(index))
    }
}

/// Writes a shortcut to `target` at `path`.
fn shortcut(path: &str, target: &str, directory: &str) {
    let mut link = core::ptr::null_mut();
    // SAFETY: CLSCTX_INPROC_SERVER; `link` is writable.
    if unsafe { CoCreateInstance(&CLSID_SHELL_LINK, core::ptr::null_mut(), 1, &IID_SHELL_LINK, &mut link) } != 0 {
        return;
    }
    let target = wide(target);
    let directory = wide(directory);
    // SAFETY: IShellLinkW's slots: SetWorkingDirectory 9, SetIconLocation
    // 17, SetPath 20; IUnknown's QueryInterface 0 and Release 2;
    // IPersistFile's Save 6.
    unsafe {
        slot::<unsafe extern "system" fn(*mut c_void, *const u16) -> i32>(link, 20)(link, target.as_ptr());
        slot::<unsafe extern "system" fn(*mut c_void, *const u16) -> i32>(link, 9)(link, directory.as_ptr());
        slot::<unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32>(link, 17)(link, target.as_ptr(), 0);
        let mut file = core::ptr::null_mut();
        if slot::<unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32>(link, 0)(link, &IID_PERSIST_FILE, &mut file) == 0 {
            slot::<unsafe extern "system" fn(*mut c_void, *const u16, i32) -> i32>(file, 6)(file, wide(path).as_ptr(), 1);
            slot::<unsafe extern "system" fn(*mut c_void) -> u32>(file, 2)(file);
        }
        slot::<unsafe extern "system" fn(*mut c_void) -> u32>(link, 2)(link);
    }
}

/// Starts a program with a command line, in a folder; returns the process.
fn start(command: &str, directory: Option<&str>, flags: u32) -> Option<Handle> {
    let mut command = wide(command);
    let directory = directory.map(wide);
    let mut startup: StartupInfo = unsafe { core::mem::zeroed() };
    startup.cb = core::mem::size_of::<StartupInfo>() as u32;
    let mut information: ProcessInformation = unsafe { core::mem::zeroed() };
    // SAFETY: the command line is writable and null-terminated.
    let started = unsafe {
        CreateProcessW(core::ptr::null(), command.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), 0, flags, core::ptr::null_mut(), directory.as_ref().map_or(core::ptr::null(), |directory| directory.as_ptr()), &mut startup, &mut information)
    };
    if started == 0 {
        return None;
    }
    // SAFETY: the thread handle is owned.
    unsafe { CloseHandle(information.thread) };
    Some(information.process)
}

/// The process that started this one, and its executable.
fn parent_process() -> Option<(Handle, String)> {
    let mut information: ProcessBasicInformation = unsafe { core::mem::zeroed() };
    // SAFETY: ProcessBasicInformation fills the structure.
    let status = unsafe { NtQueryInformationProcess(GetCurrentProcess(), 0, (&mut information as *mut ProcessBasicInformation).cast(), core::mem::size_of::<ProcessBasicInformation>() as u32, core::ptr::null_mut()) };
    if status != 0 {
        return None;
    }
    // SAFETY: OpenProcess accepts any process id.
    let process = unsafe { OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, information.parent_id as u32) };
    if process.is_null() {
        return None;
    }
    let mut name = vec![0u16; 32768];
    let mut size = name.len() as u32;
    // SAFETY: the buffer holds `size` units.
    if unsafe { QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut size) } == 0 {
        // SAFETY: the handle is owned.
        unsafe { CloseHandle(process) };
        return None;
    }
    Some((process, String::from_utf16_lossy(&name[..size as usize])))
}

fn own_path() -> String {
    let mut name = vec![0u16; 32768];
    // SAFETY: the buffer holds `len` units.
    let length = unsafe { GetModuleFileNameW(core::ptr::null_mut(), name.as_mut_ptr(), name.len() as u32) };
    String::from_utf16_lossy(&name[..length as usize])
}

/// The command line's options.
struct Options {
    silent: bool,
    updated: bool,
    force_run: bool,
    directory: Option<String>,
    /// The folder an uninstaller copied out of removes.
    uninstall: Option<String>,
}

fn options() -> Options {
    // SAFETY: the process command line is null-terminated.
    let line = narrow(unsafe { GetCommandLineW() });
    let mut count = 0;
    // SAFETY: the command line is null-terminated; the array stays allocated.
    let argv = unsafe { CommandLineToArgvW(GetCommandLineW(), &mut count) };
    let mut arguments = Vec::new();
    for index in 1..count.max(0) as usize {
        // SAFETY: the shell returned `count` strings.
        arguments.push(narrow(unsafe { *argv.add(index) }));
    }
    let has = |flag: &str| arguments.iter().any(|argument| argument.eq_ignore_ascii_case(flag));
    // NSIS takes the rest of the command line after `/D=`, spaces and all.
    let directory = line
        .find("/D=")
        .map(|index| line[index + 3..].trim().trim_matches('"').trim_end_matches('\\').into())
        .filter(|directory: &String| !directory.is_empty());
    Options {
        silent: has("/S"),
        updated: has("--updated"),
        force_run: has("--force-run"),
        directory,
        uninstall: arguments
            .iter()
            .find_map(|argument| argument.strip_prefix("--uninstall-from="))
            .map(String::from),
    }
}

fn message(payload: &Payload, text: &str, kind: u32) -> i32 {
    let caption = wide(payload.setting("product"));
    // SAFETY: both strings are null-terminated.
    unsafe { MessageBoxW(core::ptr::null_mut(), wide(text).as_ptr(), caption.as_ptr(), kind) }
}

fn uninstall_key(payload: &Payload) -> String {
    format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}",
        payload.setting("appid")
    )
}

fn shortcuts(payload: &Payload) -> [String; 2] {
    let name = format!("{}.lnk", payload.setting("shortcut"));
    [
        join(&known_folder(&FOLDERID_PROGRAMS, "Programs"), &name),
        join(&known_folder(&FOLDERID_DESKTOP, "Desktop"), &name),
    ]
}

/// The paths a previous install's uninstaller lists.
fn installed_files(directory: &str, payload: &Payload) -> Vec<String> {
    let uninstaller = join(directory, &format!("Uninstall {}.exe", payload.setting("product")));
    read_payload(&uninstaller).map_or_else(Vec::new, |old| {
        old.entries.into_iter().map(|entry| entry.path).collect()
    })
}

/// Writes the uninstaller: this program's image, then the settings and the
/// list of installed files.
fn write_uninstaller(own: &str, payload: &Payload, path: &str) -> bool {
    let Some(source) = File::open(own) else {
        return false;
    };
    let mut image = vec![0u8; payload.start as usize];
    if !source.seek(0) || !source.read_exact(&mut image) {
        return false;
    }
    let settings: String = payload
        .settings
        .iter()
        .map(|(key, value)| match key.as_str() {
            "mode" => String::from("mode=uninstall\n"),
            _ => format!("{key}={value}\n"),
        })
        .collect();
    let mut overlay = Vec::new();
    overlay.extend_from_slice(MAGIC);
    overlay.extend_from_slice(&1u32.to_le_bytes());
    overlay.extend_from_slice(&(settings.len() as u32).to_le_bytes());
    overlay.extend_from_slice(settings.as_bytes());
    overlay.extend_from_slice(&(payload.entries.len() as u32).to_le_bytes());
    for entry in &payload.entries {
        overlay.extend_from_slice(&(entry.path.len() as u32).to_le_bytes());
        overlay.extend_from_slice(entry.path.as_bytes());
        overlay.extend_from_slice(&0u64.to_le_bytes());
        overlay.extend_from_slice(&0u64.to_le_bytes());
    }
    overlay.extend_from_slice(&payload.start.to_le_bytes());
    overlay.extend_from_slice(END_MAGIC);
    File::create(path).is_some_and(|file| file.write(&image) && file.write(&overlay))
}

fn install(payload: &Payload, options: &Options, own: &str) -> i32 {
    let product = payload.setting("product");
    let exe = payload.setting("exe");
    let app_key = format!("Software\\{}", payload.setting("appid"));
    // The application that started an update, which this waits for.
    let starter = parent_process().filter(|(_, path)| file_name(path).eq_ignore_ascii_case(exe));
    let directory = options
        .directory
        .clone()
        .or_else(|| {
            options
                .updated
                .then(|| starter.as_ref().and_then(|(_, path)| parent(path).map(String::from)))
                .flatten()
        })
        .or_else(|| registry_get(&app_key, "InstallLocation"))
        .unwrap_or_else(|| {
            join(
                &join(&known_folder(&FOLDERID_LOCAL_APP_DATA, "LocalAppData"), "Programs"),
                payload.setting("folder"),
            )
        });
    if let Some((process, _)) = &starter {
        if options.updated {
            // SAFETY: the process handle is owned.
            unsafe { WaitForSingleObject(*process, 30_000) };
        }
        // SAFETY: the process handle is owned.
        unsafe { CloseHandle(*process) };
    }
    create_directories(&directory);
    let previous = installed_files(&directory, payload);
    let Some(source) = File::open(own) else {
        return 2;
    };
    let progress = if options.silent {
        None
    } else {
        Progress::open(&format!("{product} Setup"), &format!("Installing {product}..."))
    };
    let total: u64 = payload.entries.iter().map(|entry| entry.size).sum();
    let mut done = 0;
    for entry in &payload.entries {
        let path = join(&directory, &entry.path);
        if let Some(folder) = parent(&path) {
            create_directories(folder);
        }
        if let Some(progress) = &progress {
            progress.set(done, total);
        }
        done += entry.size;
        if !extract(&source, entry, &path) {
            if !options.silent {
                message(payload, &format!("{product} could not be installed: {path} could not be written."), MB_ICONERROR);
            }
            return 1;
        }
    }
    // Files of the previous version that this one no longer has.
    for old in previous {
        if !payload.entries.iter().any(|entry| entry.path.eq_ignore_ascii_case(&old)) {
            let path = join(&directory, &old);
            delete_file(&path);
            remove_empty_parents(&path, &directory);
        }
    }
    let uninstaller = join(&directory, &format!("Uninstall {product}.exe"));
    write_uninstaller(own, payload, &uninstaller);
    let executable = join(&directory, exe);
    let size = total;
    let key = uninstall_key(payload);
    registry_set(&app_key, "InstallLocation", &directory);
    for (name, value) in [
        ("DisplayName", format!("{product} {}", payload.setting("version"))),
        ("DisplayVersion", payload.setting("version").into()),
        ("Publisher", payload.setting("publisher").into()),
        ("DisplayIcon", format!("{executable},0")),
        ("InstallLocation", directory.clone()),
        ("UninstallString", format!("\"{uninstaller}\" /currentuser")),
        ("QuietUninstallString", format!("\"{uninstaller}\" /currentuser /S")),
        ("EstimatedSize", format!("dword:{}", size / 1024)),
        ("NoModify", "dword:1".into()),
        ("NoRepair", "dword:1".into()),
    ] {
        registry_set(&key, name, &value);
    }
    // SAFETY: COM is initialized once for this thread's shortcuts.
    unsafe { CoInitializeEx(core::ptr::null_mut(), 2) };
    let [start_menu, desktop] = shortcuts(payload);
    let refresh = !options.updated;
    if payload.flag("startmenu") && (refresh || exists(&start_menu)) {
        create_directories(parent(&start_menu).unwrap_or(""));
        shortcut(&start_menu, &executable, &directory);
    }
    if payload.flag("desktop") && (refresh || payload.flag("desktopalways") || exists(&desktop)) {
        create_directories(parent(&desktop).unwrap_or(""));
        shortcut(&desktop, &executable, &directory);
    }
    drop(progress);
    if options.force_run || (!options.silent && payload.flag("run")) {
        if let Some(process) = start(&format!("\"{executable}\""), Some(&directory), 0) {
            // SAFETY: the process handle is owned.
            unsafe { CloseHandle(process) };
        }
    }
    0
}

fn uninstall(payload: &Payload, options: &Options, own: &str) -> i32 {
    let product = payload.setting("product");
    let Some(directory) = options.uninstall.clone() else {
        // The uninstaller lives in the folder it removes, so a copy of it
        // outside does the work once this one exits.
        if !options.silent && message(payload, &format!("Uninstall {product}?"), MB_YESNO | MB_ICONQUESTION) != IDYES {
            return 1;
        }
        let Some(directory) = parent(own) else {
            return 2;
        };
        let mut temp = vec![0u16; 1024];
        // SAFETY: the buffer holds `len` units.
        let length = unsafe { GetTempPathW(temp.len() as u32, temp.as_mut_ptr()) } as usize;
        let copy = join(&String::from_utf16_lossy(&temp[..length]), &format!("Un_{}.exe", file_name(directory).replace(' ', "_")));
        // SAFETY: both names are null-terminated.
        if unsafe { CopyFileW(wide(own).as_ptr(), wide(&copy).as_ptr(), 0) } == 0 {
            return 2;
        }
        let silent = if options.silent { " /S" } else { "" };
        return match start(&format!("\"{copy}\" --uninstall-from=\"{directory}\"{silent}"), None, 0) {
            Some(process) => {
                // SAFETY: the process handle is owned.
                unsafe { CloseHandle(process) };
                0
            }
            None => 2,
        };
    };
    // The uninstaller this copy came from exits first.
    if let Some((process, _)) = parent_process() {
        // SAFETY: the process handle is owned.
        unsafe {
            WaitForSingleObject(process, 30_000);
            CloseHandle(process);
        }
    }
    let mut failed = false;
    for entry in &payload.entries {
        let path = join(&directory, &entry.path);
        failed |= !delete_file(&path);
        remove_empty_parents(&path, &directory);
    }
    failed |= !delete_file(&join(&directory, &format!("Uninstall {product}.exe")));
    // SAFETY: the name is null-terminated; a folder with other files stays.
    unsafe { RemoveDirectoryW(wide(&directory).as_ptr()) };
    for link in shortcuts(payload) {
        delete_file(&link);
    }
    registry_delete(&uninstall_key(payload));
    registry_delete(&format!("Software\\{}", payload.setting("appid")));
    if payload.flag("appdata") && !options.updated {
        let data = join(&known_folder(&FOLDERID_ROAMING_APP_DATA, "RoamingAppData"), payload.setting("data"));
        if is_directory(&data) {
            delete_tree(&data);
        }
    }
    if failed && !options.silent {
        message(payload, &format!("Some files of {product} could not be removed from {directory}."), MB_ICONERROR);
    }
    // This copy deletes itself once it exits.
    let own_name = own.replace('"', "");
    if let Some(process) = start(&format!("cmd.exe /d /c ping -n 3 127.0.0.1 > nul & del /f /q \"{own_name}\""), None, CREATE_NO_WINDOW) {
        // SAFETY: the process handle is owned.
        unsafe { CloseHandle(process) };
    }
    i32::from(failed)
}

#[unsafe(no_mangle)]
pub fn nativeboat_main() -> i32 {
    let own = own_path();
    let options = options();
    let Some(payload) = read_payload(&own) else {
        return 2;
    };
    if payload.setting("mode") == "uninstall" {
        uninstall(&payload, &options, &own)
    } else {
        install(&payload, &options, &own)
    }
}
