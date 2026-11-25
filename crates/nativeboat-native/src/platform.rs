//! Operating-system services for compiled applications. Windows functions
//! are declared with `raw-dylib`, so rustc writes their import entries
//! itself: no C runtime, Windows SDK import library or Microsoft linker is
//! involved, and executables import only operating-system DLLs.

use crate::JsStr;
use alloc::string::String;
use alloc::vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> *mut c_void;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        disposition: u32,
        flags: u32,
        template: *mut c_void,
    ) -> *mut c_void;
    fn WriteFile(
        file: *mut c_void,
        data: *const u8,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn ReadFile(
        file: *mut c_void,
        data: *mut u8,
        length: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn QueryPerformanceCounter(count: *mut i64) -> i32;
    fn QueryPerformanceFrequency(frequency: *mut i64) -> i32;
    fn Sleep(milliseconds: u32);
    fn GetCurrentProcessId() -> u32;
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn TerminateProcess(process: *mut c_void, exit_code: u32) -> i32;
    fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetCurrentDirectoryW(length: u32, buffer: *mut u16) -> u32;
    fn SetCurrentDirectoryW(path: *const u16) -> i32;
    fn GetModuleFileNameW(module: *mut c_void, buffer: *mut u16, length: u32) -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, length: u32) -> u32;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> i32;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(block: *mut u16) -> i32;
    fn GetCommandLineW() -> *const u16;
    fn GetConsoleMode(console: *mut c_void, mode: *mut u32) -> i32;
    fn ExitProcess(code: u32);
    fn GetUserPreferredUILanguages(
        flags: u32,
        count: *mut u32,
        buffer: *mut u16,
        length: *mut u32,
    ) -> i32;
    fn GetUserDefaultLocaleName(name: *mut u16, length: i32) -> i32;
}

// Declared as `app_host` declares it: declarations of one function must agree.
#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegGetValueW(
        key: *mut c_void,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
}

/// `process.kill`'s Windows implementation. Windows terminates the target for
/// every nonzero signal; signal zero only checks whether the process exists.
pub(crate) fn kill(pid: u32, signal: i32) -> Result<(), u32> {
    const PROCESS_TERMINATE: u32 = 0x0001;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_TIMEOUT: u32 = 258;

    // SAFETY: OpenProcess has no pointer preconditions; the handle is checked.
    let handle = unsafe {
        OpenProcess(
            SYNCHRONIZE | if signal == 0 { 0 } else { PROCESS_TERMINATE },
            0,
            pid,
        )
    };
    if handle.is_null() {
        // SAFETY: GetLastError reports this thread's OpenProcess result.
        return Err(unsafe { GetLastError() });
    }

    let result = if signal == 0 {
        // SAFETY: the process handle remains open for this zero-time check.
        match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_TIMEOUT => Ok(()),
            WAIT_OBJECT_0 => Err(87), // ERROR_INVALID_PARAMETER: process exited.
            _ => {
                // SAFETY: GetLastError reports the failed wait operation.
                Err(unsafe { GetLastError() })
            }
        }
    } else {
        // SAFETY: the handle has PROCESS_TERMINATE access and is closed below.
        if unsafe { TerminateProcess(handle, 1) } != 0 {
            Ok(())
        } else {
            // SAFETY: GetLastError reports this thread's termination result.
            Err(unsafe { GetLastError() })
        }
    };
    // SAFETY: this handle was returned by OpenProcess above.
    unsafe { CloseHandle(handle) };
    result
}

#[link(name = "bcryptprimitives", kind = "raw-dylib")]
unsafe extern "system" {
    fn ProcessPrng(data: *mut u8, length: usize) -> i32;
}

const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;

/// Writes all of `bytes` to a file handle; false when the handle fails.
fn write(file: *mut c_void, mut bytes: &[u8]) -> bool {
    while !bytes.is_empty() {
        let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let mut written = 0;
        // SAFETY: `bytes` is valid for `length` bytes and `written` is a
        // valid output location; no overlapped I/O is requested.
        let ok = unsafe {
            WriteFile(
                file,
                bytes.as_ptr(),
                length,
                &mut written,
                core::ptr::null_mut(),
            )
        };
        if ok == 0 || written == 0 {
            return false;
        }
        bytes = &bytes[written as usize..];
    }
    true
}

/// Writes text to standard output.
pub fn write_stdout(text: &str) -> bool {
    // SAFETY: GetStdHandle has no preconditions.
    write(unsafe { GetStdHandle(STD_OUTPUT_HANDLE) }, text.as_bytes())
}

/// Replaces a file's contents, for diagnostics; whether it was written.
pub fn write_file(path: &str, bytes: &[u8]) -> bool {
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_SHARE_READ: u32 = 1;
    const CREATE_ALWAYS: u32 = 2;
    let name: alloc::vec::Vec<u16> = path.encode_utf16().chain([0]).collect();
    // SAFETY: a null-terminated name; the handle is closed below.
    unsafe {
        let file = CreateFileW(
            name.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ,
            core::ptr::null_mut(),
            CREATE_ALWAYS,
            0,
            core::ptr::null_mut(),
        );
        if file as isize == -1 {
            return false;
        }
        let written = write(file, bytes);
        CloseHandle(file);
        written
    }
}

/// A file's contents, for loading resources; `None` when it cannot be read.
pub fn read_file(path: &str) -> Option<alloc::vec::Vec<u8>> {
    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_SHARE_READ_WRITE: u32 = 3;
    const OPEN_EXISTING: u32 = 3;
    let name: alloc::vec::Vec<u16> = path.encode_utf16().chain([0]).collect();
    // SAFETY: a null-terminated name; the handle is closed below and reads
    // write into the vector's spare capacity, which is then marked filled.
    unsafe {
        let file = CreateFileW(
            name.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ_WRITE,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            0,
            core::ptr::null_mut(),
        );
        if file as isize == -1 {
            return None;
        }
        let mut contents = alloc::vec::Vec::new();
        let complete = loop {
            contents.reserve(64 * 1024);
            let spare = contents.spare_capacity_mut();
            let mut read = 0u32;
            let length = spare.len().min(u32::MAX as usize) as u32;
            if ReadFile(
                file,
                spare.as_mut_ptr().cast(),
                length,
                &mut read,
                core::ptr::null_mut(),
            ) == 0
            {
                break false;
            }
            if read == 0 {
                break true;
            }
            contents.set_len(contents.len() + read as usize);
        };
        CloseHandle(file);
        complete.then_some(contents)
    }
}

/// Writes text to standard error.
pub fn write_stderr(text: &str) -> bool {
    // SAFETY: GetStdHandle has no preconditions.
    write(unsafe { GetStdHandle(STD_ERROR_HANDLE) }, text.as_bytes())
}

/// Reports a module's evaluation on standard error when
/// `NATIVEBOAT_TRACE_MODULE_EVALUATION` is set. Generated programs call this
/// for every module they load, so the variable is read once per process.
pub fn trace_module_evaluation(path: &str) {
    use core::sync::atomic::{AtomicU8, Ordering};
    /// Zero until read, then one when tracing is off and two when it is on.
    static TRACING: AtomicU8 = AtomicU8::new(0);
    let mut tracing = TRACING.load(Ordering::Relaxed);
    if tracing == 0 {
        let name = JsStr::from("NATIVEBOAT_TRACE_MODULE_EVALUATION");
        tracing = if environment_value(&name).is_some() {
            2
        } else {
            1
        };
        TRACING.store(tracing, Ordering::Relaxed);
    }
    if tracing == 2 {
        write_stderr(&alloc::format!(
            "nativeboat evaluating module: {path} at {:.1}\n",
            now()
        ));
    }
}

/// Reports a page's style, layout or paint pass and what it cost on
/// standard error when `NATIVEBOAT_TRACE_LAYOUT` is set.
pub fn trace_layout(event: impl FnOnce() -> alloc::string::String) {
    use core::sync::atomic::{AtomicU8, Ordering};
    /// Zero until read, then one when tracing is off and two when it is on.
    static TRACING: AtomicU8 = AtomicU8::new(0);
    let mut tracing = TRACING.load(Ordering::Relaxed);
    if tracing == 0 {
        let name = JsStr::from("NATIVEBOAT_TRACE_LAYOUT");
        tracing = if environment_value(&name).is_some() {
            2
        } else {
            1
        };
        TRACING.store(tracing, Ordering::Relaxed);
    }
    if tracing == 2 {
        write_stderr(&alloc::format!(
            "{:.1} layout {}
",
            now(),
            event()
        ));
    }
}

/// Whether `NATIVEBOAT_TRACE_INPUT` asks for input tracing.
pub fn input_traced() -> bool {
    use core::sync::atomic::{AtomicU8, Ordering};
    /// Zero until read, then one when tracing is off and two when it is on.
    static TRACING: AtomicU8 = AtomicU8::new(0);
    let mut tracing = TRACING.load(Ordering::Relaxed);
    if tracing == 0 {
        let name = JsStr::from("NATIVEBOAT_TRACE_INPUT");
        tracing = if environment_value(&name).is_some() {
            2
        } else {
            1
        };
        TRACING.store(tracing, Ordering::Relaxed);
    }
    tracing == 2
}

/// Reports a change of the page's focused element on standard error when
/// `NATIVEBOAT_TRACE_INPUT` is set, as page input traces are.
pub fn trace_focus(event: impl FnOnce() -> alloc::string::String) {
    use core::sync::atomic::{AtomicU8, Ordering};
    /// Zero until read, then one when tracing is off and two when it is on.
    static TRACING: AtomicU8 = AtomicU8::new(0);
    let mut tracing = TRACING.load(Ordering::Relaxed);
    if tracing == 0 {
        let name = JsStr::from("NATIVEBOAT_TRACE_INPUT");
        tracing = if environment_value(&name).is_some() {
            2
        } else {
            1
        };
        TRACING.store(tracing, Ordering::Relaxed);
    }
    if tracing == 2 {
        write_stderr(&alloc::format!("nativeboat: focus {}\n", event()));
    }
}

/// Reports an Electron IPC event on standard error when
/// `NATIVEBOAT_TRACE_IPC` is set: listeners pages add, messages sent to
/// pages and delivered to their listeners, and pages that loaded.
pub fn trace_ipc(event: impl FnOnce() -> alloc::string::String) {
    use core::sync::atomic::{AtomicU8, Ordering};
    /// Zero until read, then one when tracing is off and two when it is on.
    static TRACING: AtomicU8 = AtomicU8::new(0);
    let mut tracing = TRACING.load(Ordering::Relaxed);
    if tracing == 0 {
        let name = JsStr::from("NATIVEBOAT_TRACE_IPC");
        tracing = if environment_value(&name).is_some() {
            2
        } else {
            1
        };
        TRACING.store(tracing, Ordering::Relaxed);
    }
    if tracing == 2 {
        write_stderr(&alloc::format!("{:.1} ipc {}\n", now(), event()));
    }
}

fn standard_handle(descriptor: u32) -> *mut c_void {
    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    let which = match descriptor {
        0 => STD_INPUT_HANDLE,
        2 => STD_ERROR_HANDLE,
        _ => STD_OUTPUT_HANDLE,
    };
    // SAFETY: GetStdHandle has no preconditions.
    unsafe { GetStdHandle(which) }
}

/// Writes bytes to standard output (descriptor 1) or standard error (2).
pub fn write_standard(descriptor: u32, bytes: &[u8]) -> bool {
    write(standard_handle(descriptor), bytes)
}

/// Whether standard input (0), output (1) or error (2) is a console.
pub fn is_console(descriptor: u32) -> bool {
    let mut mode = 0;
    // SAFETY: `mode` is a valid output location; other handles fail.
    unsafe { GetConsoleMode(standard_handle(descriptor), &mut mode) != 0 }
}

/// Ends this process with an exit code, without unwinding. Windows does
/// not return from it.
pub fn exit_process(code: u32) {
    // SAFETY: ExitProcess has no preconditions.
    unsafe { ExitProcess(code) }
}

/// Random bits from the operating system's generator, which cannot fail.
pub fn random() -> u64 {
    let mut bytes = [0u8; 8];
    random_fill(&mut bytes);
    u64::from_le_bytes(bytes)
}

/// Fills a buffer with random bits from the operating system's generator.
pub(crate) fn random_fill(bytes: &mut [u8]) {
    // SAFETY: the buffer is valid for its length; ProcessPrng always succeeds.
    unsafe { ProcessPrng(bytes.as_mut_ptr(), bytes.len()) };
}

/// Generates a lowercase RFC 4122 version 4 UUID from the OS CSPRNG.
pub(crate) fn random_uuid() -> String {
    let mut bytes = [0u8; 16];
    random_fill(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    use core::fmt::Write;
    let mut uuid = String::with_capacity(36);
    for (index, byte) in bytes.into_iter().enumerate() {
        if matches!(index, 4 | 6 | 8 | 10) {
            uuid.push('-');
        }
        let _ = write!(&mut uuid, "{byte:02x}");
    }
    uuid
}

/// The identifier of this generated application's process, never the compiler's.
pub fn process_id() -> u32 {
    // SAFETY: GetCurrentProcessId has no preconditions and cannot fail.
    unsafe { GetCurrentProcessId() }
}

/// `PROCESS_BASIC_INFORMATION`.
#[repr(C)]
#[derive(Default)]
struct ProcessBasicInformation {
    exit_status: isize,
    environment_block: usize,
    affinity_mask: usize,
    base_priority: isize,
    process: usize,
    parent: usize,
}

/// The identifier of the process that started this one, or 0 when it cannot
/// be read. Like libuv's `uv_os_getppid`, this asks for this process's basic
/// information, which is immediate, where listing every process takes
/// milliseconds at startup.
pub fn parent_process_id() -> u32 {
    const PROCESS_BASIC_INFORMATION: u32 = 0;
    type Query = unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32, *mut u32) -> i32;
    let procedure = system_procedure("ntdll.dll", c"NtQueryInformationProcess");
    if procedure.is_null() {
        return 0;
    }
    let mut information = ProcessBasicInformation::default();
    // SAFETY: the procedure is NtQueryInformationProcess, -1 is this process's
    // pseudo handle, and the buffer is writable for its byte length.
    let status = unsafe {
        let query: Query = core::mem::transmute(procedure);
        query(
            -1isize as *mut c_void,
            PROCESS_BASIC_INFORMATION,
            (&raw mut information).cast(),
            core::mem::size_of::<ProcessBasicInformation>() as u32,
            core::ptr::null_mut(),
        )
    };
    if status < 0 {
        return 0;
    }
    information.parent as u32
}

/// A function exported by a system DLL, loaded on first use; null when the
/// DLL or the function is missing.
pub(crate) fn system_procedure(library: &str, name: &core::ffi::CStr) -> *const c_void {
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
    let library: alloc::vec::Vec<u16> = library.encode_utf16().chain([0]).collect();
    // SAFETY: both names are null-terminated; the search is limited to
    // System32 and a null module is checked before use.
    unsafe {
        let module = LoadLibraryExW(
            library.as_ptr(),
            core::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        );
        if module.is_null() {
            return core::ptr::null();
        }
        GetProcAddress(module, name.as_ptr().cast())
    }
}

fn query_directory(buffer: &mut [u16]) -> u32 {
    // SAFETY: the buffer is writable for the supplied number of UTF-16 units.
    unsafe { GetCurrentDirectoryW(buffer.len() as u32, buffer.as_mut_ptr()) }
}

fn query_executable(buffer: &mut [u16]) -> u32 {
    // SAFETY: a null module selects this executable; the buffer is writable.
    unsafe {
        GetModuleFileNameW(
            core::ptr::null_mut(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    }
}

/// Both Windows queries return a length below capacity on success. Directory
/// queries return the required capacity on overflow; module queries return the
/// current capacity after truncation. Retry both, without accepting truncation.
fn read_path(query: fn(&mut [u16]) -> u32) -> Result<JsStr, u32> {
    const MAX_UNITS: usize = 32_768;
    let mut buffer = vec![0; 256];
    loop {
        let length = query(&mut buffer) as usize;
        if length == 0 {
            // SAFETY: GetLastError reads this thread's latest Windows error.
            return Err(unsafe { GetLastError() });
        }
        if length < buffer.len() {
            return Ok(JsStr::from_units(&buffer[..length]));
        }
        if buffer.len() == MAX_UNITS {
            return Err(206); // ERROR_FILENAME_EXCED_RANGE
        }
        let capacity = (buffer.len() * 2).max(length + 1).min(MAX_UNITS);
        buffer.resize(capacity, 0);
    }
}

fn directory_path(path: JsStr) -> JsStr {
    let units = path.units();
    if units.last() == Some(&92) && !(units.len() == 3 && units[1] == 58) {
        JsStr::from_units(&units[..units.len() - 1])
    } else {
        path
    }
}

/// The application's current directory, preserving UTF-16 code units and drive
/// root slashes. The error is the original Windows error number.
pub fn current_directory() -> Result<JsStr, u32> {
    read_path(query_directory).map(directory_path)
}

/// Change the application's current directory using the same UTF-16 boundary
/// as Node's Windows process APIs. The error is the original Windows error.
pub fn set_current_directory(path: &JsStr) -> Result<(), u32> {
    let path = environment_text(path);
    // SAFETY: path is a null-terminated UTF-16 buffer valid for this call.
    if unsafe { SetCurrentDirectoryW(path.as_ptr()) } != 0 {
        Ok(())
    } else {
        // SAFETY: read the error immediately after the failed call on this thread.
        Err(unsafe { GetLastError() })
    }
}

/// The running application's executable path, never a build-time source path.
pub fn executable_path() -> Result<JsStr, u32> {
    read_path(query_executable)
}

/// Node's string boundary replaces unpaired surrogates before passing names
/// and values to Windows; embedded nulls terminate the OS string.
fn environment_text(text: &JsStr) -> alloc::vec::Vec<u16> {
    text.to_rust_lossy()
        .encode_utf16()
        .take_while(|unit| *unit != 0)
        .chain([0])
        .collect()
}

/// libuv returns WTF-8 from an OS value; Node's UTF-8 decoder replaces each
/// byte of an encoded unpaired surrogate. Preserve that observable boundary.
pub(crate) fn os_string(units: &[u16]) -> JsStr {
    let mut text = alloc::string::String::new();
    for character in core::char::decode_utf16(units.iter().copied()) {
        match character {
            Ok(character) => text.push(character),
            Err(_) => text.push_str("���"),
        }
    }
    JsStr::from(text.as_str())
}

fn query_environment(name: &[u16], buffer: &mut [u16]) -> u32 {
    // SAFETY: name is null-terminated and buffer is writable for its length.
    // Reset the thread error to distinguish an empty value from an absent key.
    unsafe {
        SetLastError(0);
        GetEnvironmentVariableW(name.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32)
    }
}

/// A live Windows environment value. Keys are case-insensitive. Empty values
/// are distinct from missing values; failed reads have no value, as in Node.
pub fn environment_value(name: &JsStr) -> Option<JsStr> {
    let name = environment_text(name);
    let mut buffer = vec![0; 256];
    loop {
        let length = query_environment(&name, &mut buffer) as usize;
        if length == 0 {
            // SAFETY: read the error immediately after the query on this thread.
            return (unsafe { GetLastError() } == 0).then(JsStr::default);
        }
        if length < buffer.len() {
            return Some(os_string(&buffer[..length]));
        }
        if buffer.len() == 32_768 {
            return None;
        }
        buffer.resize((length + 1).max(buffer.len() * 2).min(32_768), 0);
    }
}

/// Set or delete an environment variable in this process only. Node ignores
/// Windows failures here; value conversion is performed by compiled operations.
pub fn set_environment(name: &JsStr, value: Option<&JsStr>) {
    let name = environment_text(name);
    if name.first() == Some(&61) {
        return; // Windows' hidden drive variables are read-only.
    }
    let value = value.map(environment_text);
    // SAFETY: both buffers are null-terminated; null value requests deletion.
    unsafe {
        SetEnvironmentVariableW(
            name.as_ptr(),
            value
                .as_ref()
                .map_or(core::ptr::null(), |value| value.as_ptr()),
        )
    };
}

fn environment_block() -> *mut u16 {
    // SAFETY: the API has no preconditions and returns an owned terminated block.
    unsafe { GetEnvironmentStringsW() }
}

/// Enumerate a snapshot of this process's visible environment names. The OS
/// allocation is freed after copying. Enumeration allocation failure is fatal,
/// as in Node's native environment enumeration, rather than silently empty.
pub fn environment_keys() -> alloc::vec::Vec<JsStr> {
    let block = environment_block();
    assert!(
        !block.is_null(),
        "cannot allocate Windows environment snapshot"
    );
    let mut keys = alloc::vec::Vec::new();
    let mut cursor = block;
    // SAFETY: Windows owns a double-null-terminated block of null-terminated
    // strings until FreeEnvironmentStringsW, and each traversed unit is inside
    // it. No pointers or borrowed slices escape this function.
    unsafe {
        while *cursor != 0 {
            let start = cursor;
            while *cursor != 0 {
                cursor = cursor.add(1);
            }
            let length = cursor.offset_from(start) as usize;
            let entry = core::slice::from_raw_parts(start, length);
            if entry[0] != 61
                && let Some(separator) = entry.iter().position(|unit| *unit == 61)
            {
                keys.push(os_string(&entry[..separator]));
            }
            cursor = cursor.add(1);
        }
        FreeEnvironmentStringsW(block);
    }
    keys
}

fn argument_space(unit: u16) -> bool {
    matches!(unit, 32 | 9)
}

/// Microsoft startup rules: the program name has a separate quote rule;
/// subsequent arguments interpret backslashes only immediately before quotes.
fn parse_arguments(line: &[u16]) -> alloc::vec::Vec<JsStr> {
    let line = &line[..line
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(line.len())];
    let mut position = 0;
    let mut quoted = false;
    let mut program = alloc::vec::Vec::new();
    while position < line.len() && (quoted || !argument_space(line[position])) {
        if line[position] == 34 {
            quoted = !quoted;
        } else {
            program.push(line[position]);
        }
        position += 1;
    }
    let mut arguments = vec![JsStr::from(
        alloc::string::String::from_utf16_lossy(&program).as_str(),
    )];
    while position < line.len() {
        while position < line.len() && argument_space(line[position]) {
            position += 1;
        }
        if position == line.len() {
            break;
        }
        let mut argument = alloc::vec::Vec::new();
        quoted = false;
        loop {
            let mut slashes = 0;
            while position < line.len() && line[position] == 92 {
                slashes += 1;
                position += 1;
            }
            if position < line.len() && line[position] == 34 {
                argument.extend(core::iter::repeat_n(92, slashes / 2));
                if slashes & 1 != 0 {
                    argument.push(34);
                    position += 1;
                } else if quoted && line.get(position + 1) == Some(&34) {
                    argument.push(34);
                    position += 2;
                } else {
                    quoted = !quoted;
                    position += 1;
                }
                continue;
            }
            argument.extend(core::iter::repeat_n(92, slashes));
            if position == line.len() || (!quoted && argument_space(line[position])) {
                break;
            }
            argument.push(line[position]);
            position += 1;
        }
        arguments.push(JsStr::from(
            alloc::string::String::from_utf16_lossy(&argument).as_str(),
        ));
    }
    arguments
}

/// The process command line, parsed natively without a C runtime or shell DLL.
/// The first item is the raw program-name argument; compiled process.argv
/// replaces it with the actual executable path.
pub fn command_arguments() -> alloc::vec::Vec<JsStr> {
    // SAFETY: Windows returns a valid system-managed, null-terminated UTF-16
    // string for this process. It remains valid and we neither write nor free it.
    unsafe {
        let start = GetCommandLineW();
        let mut end = start;
        while *end != 0 {
            end = end.add(1);
        }
        parse_arguments(core::slice::from_raw_parts(
            start,
            end.offset_from(start) as usize,
        ))
    }
}

/// The user's regional format, such as `en-US` or `zh-CN`, which Electron's
/// `app.getSystemLocale()` reports on Windows; empty when Windows gives none.
pub fn system_locale() -> String {
    const LOCALE_NAME_MAX_LENGTH: usize = 85;
    let mut buffer = [0u16; LOCALE_NAME_MAX_LENGTH];
    // SAFETY: the buffer is valid for the length passed, in units.
    let length = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
    // The length counts the terminating null.
    let end = usize::try_from(length).unwrap_or(0).saturating_sub(1);
    String::from_utf16_lossy(&buffer[..end.min(buffer.len())])
}

/// The locales an application keeps, as electron-builder's
/// `electronLanguages` names them; empty while it keeps all of Electron's.
static KEPT: KeptLocales = KeptLocales(AtomicPtr::new(core::ptr::null_mut()), AtomicUsize::new(0));

struct KeptLocales(AtomicPtr<&'static str>, AtomicUsize);

/// Keeps only these Electron locales, as an application packaged with
/// electron-builder's `electronLanguages` ships only their resources:
/// Chromium then takes its locale from them.
pub fn keep_locales(locales: &'static [&'static str]) {
    KEPT.1.store(0, Ordering::Release);
    KEPT.0.store(locales.as_ptr().cast_mut(), Ordering::Release);
    KEPT.1.store(locales.len(), Ordering::Release);
}

fn kept_locales() -> &'static [&'static str] {
    let length = KEPT.1.load(Ordering::Acquire);
    let start = KEPT.0.load(Ordering::Acquire);
    if start.is_null() || length == 0 {
        return &[];
    }
    // SAFETY: `keep_locales` stored a `'static` slice's start before its
    // length, which is zero while the two do not match.
    unsafe { core::slice::from_raw_parts(start, length) }
}

/// The user's display language as Chromium names the application locale,
/// which Electron's `app.getLocale()` and pages' `navigator.language` report:
/// the first preferred Windows UI language the application has resources
/// for, mapped onto Chromium's locales.
pub fn user_locale() -> String {
    application_locale(&preferred_ui_languages(), kept_locales())
}

/// The Windows UI languages the user prefers, such as `zh-CN`, first the
/// display language.
fn preferred_ui_languages() -> vec::Vec<String> {
    const MUI_LANGUAGE_NAME: u32 = 0x8;
    let mut count = 0;
    let mut buffer = [0u16; 256];
    let mut length = buffer.len() as u32;
    // SAFETY: the count, buffer and length are valid; the buffer's length
    // is passed in units.
    let found = unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            buffer.as_mut_ptr(),
            &mut length,
        )
    };
    if found == 0 || count == 0 {
        return vec::Vec::new();
    }
    strings(&buffer[..(length as usize).min(buffer.len())])
}

/// The strings of a list of null-terminated strings, after one another.
fn strings(units: &[u16]) -> vec::Vec<String> {
    units
        .split(|unit| *unit == 0)
        .filter(|language| !language.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// The languages the user lists in Windows' language settings, such as
/// `en-US` and `zh-Hans-CN`, which Electron's
/// `app.getPreferredSystemLanguages()` reports and adds to pages'
/// `navigator.languages`; the preferred UI languages when Windows keeps no
/// such list.
pub fn preferred_languages() -> vec::Vec<String> {
    const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
    const RRF_RT_REG_MULTI_SZ: u32 = 0x20;
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<vec::Vec<u16>>();
    let subkey = wide("Control Panel\\International\\User Profile");
    let name = wide("Languages");
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: both names are null-terminated, and the buffer holds `size`
    // bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER as *mut c_void,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_MULTI_SZ,
            core::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    let read = if status == 0 { size as usize / 2 } else { 0 };
    or_ui_languages(strings(&buffer[..read.min(buffer.len())]))
}

/// The languages Windows lists, else the preferred UI languages.
fn or_ui_languages(listed: vec::Vec<String>) -> vec::Vec<String> {
    if listed.is_empty() {
        preferred_ui_languages()
    } else {
        listed
    }
}

/// Pages' `navigator.languages` as Electron makes them: the application
/// locale, then the user's other preferred languages.
pub fn accept_languages() -> vec::Vec<String> {
    let locale = user_locale();
    let mut languages = vec![locale.clone()];
    languages.extend(
        preferred_languages()
            .into_iter()
            .filter(|language| *language != locale),
    );
    languages
}

/// Chromium's application locale: the first preferred language with
/// resources, else `en-US`. All of Electron's locales have resources unless
/// the application keeps only some.
pub(crate) fn application_locale(preferred: &[String], kept: &[&str]) -> String {
    let same = |locale: &str, kept: &str| {
        locale.len() == kept.len()
            && locale
                .chars()
                .zip(kept.chars())
                .all(|(a, b)| a.eq_ignore_ascii_case(&b) || (a == '-' && b == '_'))
    };
    if kept.is_empty() {
        return chromium_locale(preferred.first().map_or("", String::as_str));
    }
    preferred
        .iter()
        .map(|language| chromium_locale(language))
        .find(|locale| kept.iter().any(|kept| same(locale, kept)))
        .unwrap_or_else(|| "en-US".into())
}

/// A BCP 47 language tag as the closest Chromium application locale.
pub(crate) fn chromium_locale(tag: &str) -> String {
    let mut parts = tag.split(['-', '_']);
    let language = parts.next().unwrap_or_default().to_ascii_lowercase();
    let rest: alloc::vec::Vec<String> = parts.map(str::to_ascii_uppercase).collect();
    let has = |part: &str| rest.iter().any(|item| item == part);
    match language.as_str() {
        "" => "en-US".into(),
        "zh" if has("TW") || has("HK") || has("MO") || has("HANT") => "zh-TW".into(),
        "zh" => "zh-CN".into(),
        "pt" if has("BR") => "pt-BR".into(),
        "pt" => "pt-PT".into(),
        "en" if ["GB", "AU", "NZ", "IE", "IN", "ZA"]
            .iter()
            .any(|region| has(region)) =>
        {
            "en-GB".into()
        }
        "en" => "en-US".into(),
        "es" if rest.is_empty() || has("ES") => "es".into(),
        "es" => "es-419".into(),
        _ => language,
    }
}

/// A monotonic clock in milliseconds, for measuring elapsed time.
pub fn now() -> f64 {
    let mut count = 0;
    let mut frequency = 0;
    // SAFETY: both are valid output locations; the calls cannot fail on
    // Windows XP and later.
    unsafe {
        QueryPerformanceCounter(&mut count);
        QueryPerformanceFrequency(&mut frequency);
    }
    count as f64 * 1000.0 / frequency as f64
}

/// Monotonic nanoseconds since an arbitrary fixed point, used by
/// `process.hrtime`. The wide intermediate avoids overflowing while scaling
/// Windows' performance-counter ticks.
pub fn hrtime_ns() -> u64 {
    let mut count = 0i64;
    let mut frequency = 0i64;
    // SAFETY: both are valid output locations; Windows provides this counter
    // on every supported version.
    unsafe {
        QueryPerformanceCounter(&mut count);
        QueryPerformanceFrequency(&mut frequency);
    }
    (u128::from(count.max(0) as u64) * 1_000_000_000 / u128::from(frequency.max(1) as u64)) as u64
}

/// Waits about this many milliseconds, rounded up to whole milliseconds.
pub fn sleep(milliseconds: f64) {
    let whole = crate::float::ceil(milliseconds).clamp(0.0, f64::from(u32::MAX - 1)) as u32;
    // SAFETY: Sleep has no preconditions.
    unsafe { Sleep(whole) };
}

#[cfg(test)]
mod tests;
