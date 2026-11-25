//! Local Windows crash dumps for Electron's `crashReporter.start` surface.
//!
//! MarkText disables uploads and asks Electron to collect crashes locally.
//! Keep that behavior without linking a browser or crash-reporting runtime by
//! loading the Windows SDK's `MiniDumpWriteDump` from system32 on demand.

use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::{Key, Object, Property, Realm, Value};

type Handle = *mut c_void;
type Filter = unsafe extern "system" fn(*mut ExceptionPointers) -> i32;
type DumpWriter = unsafe extern "system" fn(
    Handle,
    u32,
    Handle,
    u32,
    *mut ExceptionInformation,
    *mut c_void,
    *mut c_void,
) -> i32;
type GetTempPath = unsafe extern "system" fn(u32, *mut u16) -> u32;
type CreateDirectory = unsafe extern "system" fn(*const u16, *mut c_void) -> i32;
type CreateFile =
    unsafe extern "system" fn(*const u16, u32, u32, *mut c_void, u32, u32, Handle) -> Handle;
type Close = unsafe extern "system" fn(Handle) -> i32;

#[derive(Clone, Copy)]
struct CrashApi {
    get_temp_path: GetTempPath,
    create_directory: CreateDirectory,
    create_file: CreateFile,
    close: Close,
    process: unsafe extern "system" fn() -> Handle,
    process_id: unsafe extern "system" fn() -> u32,
    thread_id: unsafe extern "system" fn() -> u32,
}

#[repr(C)]
struct ExceptionPointers {
    _exception_record: *mut c_void,
    _context_record: *mut c_void,
}

#[repr(C)]
struct ExceptionInformation {
    thread_id: u32,
    exception_pointers: *mut ExceptionPointers,
    client_pointers: i32,
}

static DUMP_WRITER: AtomicUsize = AtomicUsize::new(0);
static PREVIOUS_FILTER: AtomicUsize = AtomicUsize::new(0);
static INSTALLED: AtomicUsize = AtomicUsize::new(0);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const c_void;
    fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> Option<Filter>;
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetTempPathW(length: u32, path: *mut u16) -> u32;
    fn CreateDirectoryW(path: *const u16, security: *mut c_void) -> i32;
    fn CreateFileW(
        path: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        disposition: u32,
        attributes: u32,
        template: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
}

const INVALID_HANDLE: Handle = usize::MAX as Handle;
const SYSTEM32_SEARCH: u32 = 0x0000_0800;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SHARE_READ_WRITE: u32 = 0x0000_0003;
const CREATE_ALWAYS: u32 = 2;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const WINDOWS_API: CrashApi = CrashApi {
    get_temp_path: GetTempPathW,
    create_directory: CreateDirectoryW,
    create_file: CreateFileW,
    close: CloseHandle,
    process: GetCurrentProcess,
    process_id: GetCurrentProcessId,
    thread_id: GetCurrentThreadId,
};

pub(crate) fn module(realm: &Realm) -> Object {
    let reporter = realm.object();
    let start = realm.function("start", 1, false, |_, _, _| {
        install();
        Ok(Value::Undefined)
    });
    reporter.define(Key::name("start"), Property::data(Value::Object(start)));
    reporter
}

fn install() {
    if INSTALLED
        .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let library = [
        b'd' as u16,
        b'b' as u16,
        b'g' as u16,
        b'h' as u16,
        b'e' as u16,
        b'l' as u16,
        b'p' as u16,
        b'.' as u16,
        b'd' as u16,
        b'l' as u16,
        b'l' as u16,
        0,
    ];
    // SAFETY: The name is NUL-terminated and LOAD_LIBRARY_SEARCH_SYSTEM32 avoids
    // resolving a project-controlled dbghelp.dll.
    let module =
        unsafe { LoadLibraryExW(library.as_ptr(), core::ptr::null_mut(), SYSTEM32_SEARCH) };
    if module.is_null() {
        return;
    }
    let symbol = b"MiniDumpWriteDump\0";
    // SAFETY: `module` is the live system32 handle and `symbol` is NUL-terminated.
    let writer = unsafe { GetProcAddress(module, symbol.as_ptr()) };
    if writer.is_null() {
        return;
    }
    DUMP_WRITER.store(writer as usize, Ordering::Release);
    // SAFETY: The callback uses the documented Windows unhandled-filter ABI.
    let previous = unsafe { SetUnhandledExceptionFilter(Some(write_dump)) };
    PREVIOUS_FILTER.store(
        previous.map_or(0, |filter| filter as usize),
        Ordering::Release,
    );
}

unsafe extern "system" fn write_dump(exception: *mut ExceptionPointers) -> i32 {
    let writer = DUMP_WRITER.load(Ordering::Acquire);
    if writer != 0 {
        // SAFETY: GetProcAddress returned this pointer for MiniDumpWriteDump's ABI.
        let writer: DumpWriter = unsafe { core::mem::transmute(writer) };
        // SAFETY: The callback and exception pointer come from Windows; the writer
        // accepts the process/file handles constructed below.
        unsafe { write_minidump(writer, exception) };
    }
    let previous = PREVIOUS_FILTER.load(Ordering::Acquire);
    if previous != 0 && previous != write_dump as *const () as usize {
        // SAFETY: This pointer was returned by SetUnhandledExceptionFilter.
        let filter: Filter = unsafe { core::mem::transmute(previous) };
        // SAFETY: Preserve the preceding process-wide handler's Windows ABI.
        return unsafe { filter(exception) };
    }
    1 // EXCEPTION_EXECUTE_HANDLER
}

unsafe fn write_minidump(writer: DumpWriter, exception: *mut ExceptionPointers) {
    // SAFETY: The production API table points to Windows functions and the
    // exception pointer comes from the installed system exception filter.
    unsafe { write_minidump_with(writer, exception, WINDOWS_API) };
}

unsafe fn write_minidump_with(
    writer: DumpWriter,
    exception: *mut ExceptionPointers,
    api: CrashApi,
) {
    const CAPACITY: usize = 32_768;
    let mut temporary = [0_u16; CAPACITY];
    // SAFETY: The API writes at most CAPACITY UTF-16 code units to this buffer.
    let length = unsafe { (api.get_temp_path)(CAPACITY as u32, temporary.as_mut_ptr()) } as usize;
    if length == 0 || length >= CAPACITY - 64 {
        return;
    }
    let directory = [
        b'M' as u16,
        b'a' as u16,
        b'r' as u16,
        b'k' as u16,
        b'T' as u16,
        b'e' as u16,
        b'x' as u16,
        b't' as u16,
        b' ' as u16,
        b'C' as u16,
        b'r' as u16,
        b'a' as u16,
        b's' as u16,
        b'h' as u16,
        b' ' as u16,
        b'R' as u16,
        b'e' as u16,
        b'p' as u16,
        b'o' as u16,
        b'r' as u16,
        b't' as u16,
        b's' as u16,
    ];
    let mut used = length;
    if temporary[used - 1] != b'\\' as u16 && temporary[used - 1] != b'/' as u16 {
        temporary[used] = b'\\' as u16;
        used += 1;
    }
    temporary[used..used + directory.len()].copy_from_slice(&directory);
    used += directory.len();
    temporary[used] = 0;
    // SAFETY: The path is NUL-terminated and points to our local writable buffer.
    unsafe { (api.create_directory)(temporary.as_ptr(), core::ptr::null_mut()) };
    temporary[used] = b'\\' as u16;
    used += 1;
    let prefix = [
        b'M' as u16,
        b'a' as u16,
        b'r' as u16,
        b'k' as u16,
        b'T' as u16,
        b'e' as u16,
        b'x' as u16,
        b't' as u16,
        b'-' as u16,
    ];
    temporary[used..used + prefix.len()].copy_from_slice(&prefix);
    used += prefix.len();
    // SAFETY: GetCurrentProcessId has no pointer preconditions.
    used = append_decimal(&mut temporary, used, unsafe { (api.process_id)() });
    let suffix = [b'.' as u16, b'd' as u16, b'm' as u16, b'p' as u16, 0];
    temporary[used..used + suffix.len()].copy_from_slice(&suffix);

    // SAFETY: The completed path is NUL-terminated; the remaining arguments request
    // an owned file handle that is closed after the dump write.
    let file = unsafe {
        (api.create_file)(
            temporary.as_ptr(),
            GENERIC_WRITE,
            SHARE_READ_WRITE,
            core::ptr::null_mut(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        )
    };
    if file.is_null() || file == INVALID_HANDLE {
        return;
    }
    // SAFETY: GetCurrentThreadId has no pointer preconditions.
    let thread_id = unsafe { (api.thread_id)() };
    let mut information = ExceptionInformation {
        thread_id,
        exception_pointers: exception,
        client_pointers: 0,
    };
    // SAFETY: `file` is an open file handle and `information` remains live for the
    // duration of the documented MiniDumpWriteDump call.
    unsafe {
        writer(
            (api.process)(),
            (api.process_id)(),
            file,
            0, // MiniDumpNormal
            &mut information,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        );
        (api.close)(file);
    }
}

fn append_decimal(output: &mut [u16], mut offset: usize, mut value: u32) -> usize {
    let start = offset;
    loop {
        output[offset] = (b'0' + (value % 10) as u8) as u16;
        offset += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    output[start..offset].reverse();
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static DIRECTORY_PATH: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    static DUMP_PATH: Mutex<Vec<u16>> = Mutex::new(Vec::new());
    static DUMP_TEST_LOCK: Mutex<()> = Mutex::new(());
    static DUMP_WRITE_OK: AtomicUsize = AtomicUsize::new(0);
    static HANDLE_CLOSED: AtomicUsize = AtomicUsize::new(0);

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain([0]).collect()
    }

    unsafe fn read_wide(mut value: *const u16) -> Vec<u16> {
        let mut result = Vec::new();
        loop {
            // SAFETY: The fake Win32 APIs pass a valid NUL-terminated UTF-16 path.
            let unit = unsafe { *value };
            if unit == 0 {
                break;
            }
            result.push(unit);
            // SAFETY: The pointer advances within the same NUL-terminated path.
            value = unsafe { value.add(1) };
        }
        result
    }

    unsafe extern "system" fn temp_path(length: u32, path: *mut u16) -> u32 {
        let value = wide("C:\\Temp\\");
        if length < value.len() as u32 {
            return value.len() as u32;
        }
        // SAFETY: `path` points to a buffer of at least `length` UTF-16 units.
        unsafe { core::ptr::copy_nonoverlapping(value.as_ptr(), path, value.len()) };
        (value.len() - 1) as u32
    }

    unsafe extern "system" fn no_temp_path(_: u32, _: *mut u16) -> u32 {
        0
    }

    unsafe extern "system" fn long_temp_path(length: u32, path: *mut u16) -> u32 {
        for index in 0..length as usize {
            // SAFETY: The caller provides the requested writable capacity.
            unsafe { *path.add(index) = b'x' as u16 };
        }
        length
    }

    unsafe extern "system" fn create_directory(path: *const u16, _: *mut c_void) -> i32 {
        // SAFETY: `write_minidump_with` passes a NUL-terminated directory path.
        *DIRECTORY_PATH.lock().unwrap() = unsafe { read_wide(path) };
        1
    }

    unsafe extern "system" fn create_file(
        path: *const u16,
        _: u32,
        _: u32,
        _: *mut c_void,
        _: u32,
        _: u32,
        _: Handle,
    ) -> Handle {
        // SAFETY: `write_minidump_with` passes a NUL-terminated file path.
        *DUMP_PATH.lock().unwrap() = unsafe { read_wide(path) };
        core::ptr::dangling_mut()
    }

    unsafe extern "system" fn current_process() -> Handle {
        usize::MAX as Handle
    }

    unsafe extern "system" fn process_id() -> u32 {
        12_345
    }

    unsafe extern "system" fn thread_id() -> u32 {
        7
    }

    unsafe extern "system" fn close(handle: Handle) -> i32 {
        HANDLE_CLOSED.store(
            usize::from(handle == core::ptr::dangling_mut()),
            Ordering::SeqCst,
        );
        1
    }

    unsafe extern "system" fn dump_writer(
        process: Handle,
        pid: u32,
        file: Handle,
        dump_type: u32,
        information: *mut ExceptionInformation,
        _: *mut c_void,
        _: *mut c_void,
    ) -> i32 {
        // SAFETY: `write_minidump_with` keeps this structure live during the call.
        let info = unsafe { &*information };
        let valid = process == usize::MAX as Handle
            && pid == 12_345
            && file == core::ptr::dangling_mut()
            && dump_type == 0
            && info.thread_id == 7
            && info.client_pointers == 0;
        DUMP_WRITE_OK.store(usize::from(valid), Ordering::SeqCst);
        1
    }

    fn fake_api(temp_path: GetTempPath) -> CrashApi {
        CrashApi {
            get_temp_path: temp_path,
            create_directory,
            create_file,
            close,
            process: current_process,
            process_id,
            thread_id,
        }
    }

    #[test]
    fn writes_process_id_as_decimal_without_allocating() {
        let mut path = [0_u16; 24];
        let end = append_decimal(&mut path, 0, 42_701);
        assert_eq!(
            &path[..end],
            &[
                b'4' as u16,
                b'2' as u16,
                b'7' as u16,
                b'0' as u16,
                b'1' as u16
            ]
        );
        assert_eq!(append_decimal(&mut path, 0, 0), 1);
        assert_eq!(path[0], b'0' as u16);
    }

    #[test]
    fn exports_crash_reporter_start() {
        let realm = &Realm::new();
        let reporter = module(realm);
        assert!(reporter.own_property(&Key::name("start")).is_some());
    }

    #[test]
    fn writes_a_native_dump_under_the_local_temp_directory() {
        let _guard = DUMP_TEST_LOCK.lock().unwrap();
        DIRECTORY_PATH.lock().unwrap().clear();
        DUMP_PATH.lock().unwrap().clear();
        DUMP_WRITE_OK.store(0, Ordering::SeqCst);
        HANDLE_CLOSED.store(0, Ordering::SeqCst);
        let api = fake_api(temp_path);
        // SAFETY: Every function in `api` is a test double with the declared ABI.
        unsafe { write_minidump_with(dump_writer, core::ptr::null_mut(), api) };
        assert_eq!(
            *DIRECTORY_PATH.lock().unwrap(),
            "C:\\Temp\\MarkText Crash Reports"
                .encode_utf16()
                .collect::<Vec<_>>()
        );
        assert_eq!(
            *DUMP_PATH.lock().unwrap(),
            "C:\\Temp\\MarkText Crash Reports\\MarkText-12345.dmp"
                .encode_utf16()
                .collect::<Vec<_>>()
        );
        assert_eq!(DUMP_WRITE_OK.load(Ordering::SeqCst), 1);
        assert_eq!(HANDLE_CLOSED.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn rejects_missing_or_overlong_temp_paths_without_creating_a_dump() {
        let _guard = DUMP_TEST_LOCK.lock().unwrap();
        for provider in [no_temp_path as GetTempPath, long_temp_path as GetTempPath] {
            DUMP_PATH.lock().unwrap().clear();
            DUMP_WRITE_OK.store(0, Ordering::SeqCst);
            // SAFETY: Every function in `api` is a test double with the declared ABI.
            unsafe { write_minidump_with(dump_writer, core::ptr::null_mut(), fake_api(provider)) };
            assert!(DUMP_PATH.lock().unwrap().is_empty());
            assert_eq!(DUMP_WRITE_OK.load(Ordering::SeqCst), 0);
        }
    }
}
