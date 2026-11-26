//! File system operations on Windows, ported from the libuv in Node 22
//! (`src/win/fs.c`) with kernel32 functions only. Paths are null-terminated
//! UTF-16; failures are libuv error codes. File descriptors are numbered as
//! the C runtime numbers them for libuv: 0 to 2 are the standard handles and
//! opened files take the lowest free number from 3.

// Ported from Node v22.15.0's deps/uv/src/win/fs.c.
// Copyright Joyent, Inc. and other Node contributors. All rights reserved.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to
// deal in the Software without restriction, including without limitation the
// rights to use, copy, modify, merge, publish, distribute, sublicense, and/or
// sell copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS
// IN THE SOFTWARE.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::uv_error::translate;
use alloc::boxed::Box;
use alloc::{collections::VecDeque, sync::Arc, vec};
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

pub type Handle = *mut c_void;

/// An open watcher handle and, for file watches, the filename to filter in
/// notifications from its parent directory.
#[derive(Debug)]
pub struct WatchHandle {
    pub handle: Handle,
    pub filename: Option<Vec<u16>>,
}

/// A Windows directory entry name and its `uv_dirent_type_t` value.
pub type Dirent = (Vec<u16>, u8);

/// A batch of entries returned from a native directory cursor.
pub type DirentBatch = Vec<Dirent>;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(
        path: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        disposition: u32,
        attributes: u32,
        template: Handle,
    ) -> Handle;
    fn CreateEventW(
        security: *mut c_void,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn ReadDirectoryChangesW(
        handle: Handle,
        buffer: *mut u8,
        buffer_length: u32,
        watch_subtree: i32,
        notify_filter: u32,
        bytes_returned: *mut u32,
        overlapped: *mut Overlapped,
        completion: Option<unsafe extern "system" fn(u32, u32, *mut Overlapped)>,
    ) -> i32;
    fn GetOverlappedResult(
        handle: Handle,
        overlapped: *mut Overlapped,
        bytes_transferred: *mut u32,
        wait: i32,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn CancelIoEx(handle: Handle, overlapped: *mut Overlapped) -> i32;
    fn ReadFile(
        handle: Handle,
        buffer: *mut u8,
        length: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn WriteFile(
        handle: Handle,
        buffer: *const u8,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn SetFilePointerEx(handle: Handle, distance: i64, position: *mut i64, method: u32) -> i32;
    fn GetFileInformationByHandle(handle: Handle, information: *mut HandleInformation) -> i32;
    fn GetFileInformationByHandleEx(
        handle: Handle,
        class: u32,
        information: *mut c_void,
        size: u32,
    ) -> i32;
    fn SetFileInformationByHandle(
        handle: Handle,
        class: u32,
        information: *const c_void,
        size: u32,
    ) -> i32;
    fn GetFileType(handle: Handle) -> u32;
    fn GetConsoleMode(handle: Handle, mode: *mut u32) -> i32;
    fn GetStdHandle(which: u32) -> Handle;
    fn DeviceIoControl(
        handle: Handle,
        code: u32,
        input: *const c_void,
        input_size: u32,
        output: *mut c_void,
        output_size: u32,
        returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CreateDirectoryW(path: *const u16, security: *mut c_void) -> i32;
    fn RemoveDirectoryW(path: *const u16) -> i32;
    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    fn CopyFileW(from: *const u16, to: *const u16, fail_if_exists: i32) -> i32;
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn CompareStringOrdinal(
        left: *const u16,
        left_length: i32,
        right: *const u16,
        right_length: i32,
        ignore_case: i32,
    ) -> i32;
    fn SetFileAttributesW(path: *const u16, attributes: u32) -> i32;
    fn FlushFileBuffers(handle: Handle) -> i32;
    fn SetFileTime(
        handle: Handle,
        creation: *const u64,
        access: *const u64,
        write: *const u64,
    ) -> i32;
    fn CreateHardLinkW(link: *const u16, target: *const u16, security: *mut c_void) -> i32;
    fn CreateSymbolicLinkW(link: *const u16, target: *const u16, flags: u32) -> u8;
    fn GetFinalPathNameByHandleW(handle: Handle, path: *mut u16, length: u32, flags: u32) -> u32;
    fn GetDiskFreeSpaceW(
        root: *const u16,
        sectors_per_cluster: *mut u32,
        bytes_per_sector: *mut u32,
        free_clusters: *mut u32,
        total_clusters: *mut u32,
    ) -> i32;
    fn GetFullPathNameW(
        path: *const u16,
        length: u32,
        buffer: *mut u16,
        file: *mut *mut u16,
    ) -> u32;
    fn GetLastError() -> u32;
}

/// `OVERLAPPED`, used for directory notification requests.
#[repr(C)]
#[derive(Debug, Default)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: Handle,
}

/// `FILETIME`: two DWORDs, with 4-byte alignment.
#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

/// `BY_HANDLE_FILE_INFORMATION`.
#[repr(C)]
#[derive(Default)]
struct HandleInformation {
    attributes: u32,
    creation: FileTime,
    access: FileTime,
    write: FileTime,
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

/// `FILE_BASIC_INFO`.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct BasicInformation {
    creation: i64,
    access: i64,
    write: i64,
    change: i64,
    attributes: u32,
}

/// `FILE_STANDARD_INFO`.
#[repr(C)]
#[derive(Default)]
struct StandardInformation {
    allocation: i64,
    end_of_file: i64,
    links: u32,
    delete_pending: u8,
    directory: u8,
}

const INVALID_HANDLE: Handle = -1isize as Handle;

const GENERIC_WRITE: u32 = 0x4000_0000;
const DELETE: u32 = 0x0001_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x80;
const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
const FILE_LIST_DIRECTORY: u32 = 0x1;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_GENERIC_READ: u32 = 0x0012_0089;
const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
const FILE_WRITE_DATA: u32 = 0x2;
const FILE_APPEND_DATA: u32 = 0x4;
const SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;

const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const TRUNCATE_EXISTING: u32 = 5;

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
const FILE_ATTRIBUTE_DEVICE: u32 = 0x40;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const FILE_ATTRIBUTE_TEMPORARY: u32 = 0x100;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
const FILE_FLAG_RANDOM_ACCESS: u32 = 0x1000_0000;
const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x0800_0000;
const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_NOTIFY_CHANGE_FILE_NAME: u32 = 0x0000_0001;
const FILE_NOTIFY_CHANGE_DIR_NAME: u32 = 0x0000_0002;
const FILE_NOTIFY_CHANGE_ATTRIBUTES: u32 = 0x0000_0004;
const FILE_NOTIFY_CHANGE_SIZE: u32 = 0x0000_0008;
const FILE_NOTIFY_CHANGE_LAST_WRITE: u32 = 0x0000_0010;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const WAIT_FAILED: u32 = u32::MAX;

const ERROR_FILE_EXISTS: u32 = 80;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_ALREADY_EXISTS: u32 = 183;
const ERROR_INVALID_NAME: u32 = 123;
const ERROR_DIRECTORY: u32 = 267;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_BROKEN_PIPE: u32 = 109;
const ERROR_NO_DATA: u32 = 232;
const ERROR_NO_MORE_FILES: u32 = 18;
const ERROR_SYMLINK_NOT_SUPPORTED: u32 = 1464;
const ERROR_NOT_A_REPARSE_POINT: u32 = 4390;
const ERROR_INVALID_REPARSE_DATA: u32 = 4392;
const ERROR_INVALID_FLAGS: u32 = 1004;

pub const UV_EINVAL: i32 = -4071;
pub const UV_EISDIR: i32 = -4068;
pub const UV_EBADF: i32 = -4083;
pub const UV_ENOENT: i32 = -4058;
pub const UV_ENOTDIR: i32 = -4052;
pub const UV_EPERM: i32 = -4048;
pub const UV_EBUSY: i32 = -4082;
pub const UV_ENOSYS: i32 = -4054;
pub const UV_EPIPE: i32 = -4047;
pub const UV_EIO: i32 = -4070;

// Node's `fs.constants` on Windows.
pub const O_WRONLY: i32 = 1;
pub const O_RDWR: i32 = 2;
pub const O_APPEND: i32 = 8;
pub const O_CREAT: i32 = 0x100;
pub const O_TRUNC: i32 = 0x200;
pub const O_EXCL: i32 = 0x400;
const O_SEQUENTIAL: i32 = 0x20;
const O_RANDOM: i32 = 0x10;
const O_SHORT_LIVED: i32 = 0x1000;
const O_TEMPORARY: i32 = 0x40;
const O_EXLOCK: i32 = 0x1000_0000;
const O_DIRECT: i32 = 0x0200_0000;
const O_DSYNC: i32 = 0x0400_0000;
const O_SYNC: i32 = 0x0800_0000;
pub const S_IFMT: u32 = 0xF000;
pub const S_IFDIR: u32 = 0x4000;
pub const S_IFCHR: u32 = 0x2000;
pub const S_IFIFO: u32 = 0x1000;
pub const S_IFREG: u32 = 0x8000;
pub const S_IFLNK: u32 = 0xA000;
const S_IWRITE: u32 = 0x80;
pub const W_OK: i32 = 2;
pub const COPYFILE_EXCL: i32 = 1;
pub const COPYFILE_FICLONE_FORCE: i32 = 4;
pub const SYMLINK_DIR: i32 = 1;
pub const SYMLINK_JUNCTION: i32 = 2;

/// Directory entry types, `uv_dirent_type_t`.
pub const DIRENT_FILE: u8 = 1;
pub const DIRENT_DIR: u8 = 2;
pub const DIRENT_LINK: u8 = 3;
pub const DIRENT_CHAR: u8 = 6;

fn last_error() -> u32 {
    // SAFETY: GetLastError reads this thread's latest Windows error.
    unsafe { GetLastError() }
}

fn failure() -> i32 {
    translate(last_error())
}

fn close_handle(handle: Handle) {
    // SAFETY: the handle was opened by this module and is closed once.
    unsafe { CloseHandle(handle) };
}

fn validate_path(path: &[u16]) -> Result<(), i32> {
    match path.split_last() {
        Some((&0, name)) if !name.contains(&0) => Ok(()),
        _ => Err(UV_EINVAL),
    }
}

/// `CreateFileW` with every sharing mode.
fn create(path: &[u16], access: u32, share: u32, disposition: u32, flags: u32) -> Handle {
    // SAFETY: the path is null-terminated; no security or template is used.
    unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            share,
            core::ptr::null_mut(),
            disposition,
            flags,
            core::ptr::null_mut(),
        )
    }
}

/// Open file handles by descriptor.
#[derive(Default)]
pub struct Descriptors {
    handles: Vec<Option<Handle>>,
}

impl core::fmt::Debug for Descriptors {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "Descriptors({})", self.handles.len())
    }
}

impl Drop for Descriptors {
    fn drop(&mut self) {
        for handle in self.handles.iter_mut().filter_map(Option::take) {
            close_handle(handle);
        }
    }
}

impl Descriptors {
    /// Whether an open descriptor refers to a Windows console handle.
    pub fn is_tty(&self, fd: i32) -> bool {
        let Ok(handle) = self.handle(fd) else {
            return false;
        };
        let mut mode = 0;
        // SAFETY: this table retains the handle during the query; mode is writable.
        unsafe { GetConsoleMode(handle, &mut mode) != 0 }
    }

    /// The handle of a descriptor, or `EBADF`.
    pub fn handle(&self, fd: i32) -> Result<Handle, i32> {
        if (0..=2).contains(&fd) {
            const STANDARD: [u32; 3] = [-10i32 as u32, -11i32 as u32, -12i32 as u32];
            // SAFETY: GetStdHandle has no preconditions.
            let handle = unsafe { GetStdHandle(STANDARD[fd as usize]) };
            return if handle.is_null() || handle == INVALID_HANDLE {
                Err(UV_EBADF)
            } else {
                Ok(handle)
            };
        }
        usize::try_from(fd)
            .ok()
            .and_then(|index| self.handles.get(index).copied().flatten())
            .ok_or(UV_EBADF)
    }

    /// Numbers a handle with the lowest free descriptor from 3.
    fn add(&mut self, handle: Handle) -> i32 {
        if self.handles.len() < 3 {
            self.handles.resize(3, None);
        }
        let index = (3..self.handles.len())
            .find(|index| self.handles[*index].is_none())
            .unwrap_or(self.handles.len());
        if index == self.handles.len() {
            self.handles.push(None);
        }
        self.handles[index] = Some(handle);
        index as i32
    }

    /// `uv_fs_open`.
    pub fn open(&mut self, path: &[u16], flags: i32, mode: i32) -> Result<i32, i32> {
        let handle = open_handle(path, flags, mode)?;
        Ok(self.add(handle))
    }

    /// `uv_fs_close`: the standard descriptors stay open.
    pub fn close(&mut self, fd: i32) -> Result<(), i32> {
        if (0..=2).contains(&fd) {
            return Ok(());
        }
        let handle = usize::try_from(fd)
            .ok()
            .and_then(|index| self.handles.get_mut(index))
            .and_then(Option::take)
            .ok_or(UV_EBADF)?;
        close_handle(handle);
        Ok(())
    }
}

/// libuv's mapping of `open` flags to `CreateFileW`.
fn open_handle(path: &[u16], flags: i32, mode: i32) -> Result<Handle, i32> {
    validate_path(path)?;
    let mut access = match flags & (O_WRONLY | O_RDWR) {
        0 => FILE_GENERIC_READ,
        O_WRONLY => FILE_GENERIC_WRITE,
        O_RDWR => FILE_GENERIC_READ | FILE_GENERIC_WRITE,
        _ => return Err(UV_EINVAL),
    };
    if flags & O_APPEND != 0 {
        access = (access & !FILE_WRITE_DATA) | FILE_APPEND_DATA;
    }
    let share = if flags & O_EXLOCK != 0 { 0 } else { SHARE_ALL };
    let disposition = match flags & (O_CREAT | O_EXCL | O_TRUNC) {
        0 | O_EXCL => OPEN_EXISTING,
        O_CREAT => OPEN_ALWAYS,
        x if x == O_CREAT | O_EXCL || x == O_CREAT | O_TRUNC | O_EXCL => CREATE_NEW,
        x if x == O_TRUNC || x == O_TRUNC | O_EXCL => TRUNCATE_EXISTING,
        _ => CREATE_ALWAYS,
    };
    let mut attributes = FILE_ATTRIBUTE_NORMAL;
    // The C runtime's default umask clears no permission bits.
    if flags & O_CREAT != 0 && (mode as u32) & S_IWRITE == 0 {
        attributes |= FILE_ATTRIBUTE_READONLY;
    }
    if flags & O_TEMPORARY != 0 {
        attributes |= FILE_FLAG_DELETE_ON_CLOSE | FILE_ATTRIBUTE_TEMPORARY;
        access |= DELETE;
    }
    if flags & O_SHORT_LIVED != 0 {
        attributes |= FILE_ATTRIBUTE_TEMPORARY;
    }
    attributes |= match flags & (O_SEQUENTIAL | O_RANDOM) {
        0 => 0,
        O_SEQUENTIAL => FILE_FLAG_SEQUENTIAL_SCAN,
        O_RANDOM => FILE_FLAG_RANDOM_ACCESS,
        _ => return Err(UV_EINVAL),
    };
    if flags & O_DIRECT != 0 {
        if access & FILE_APPEND_DATA != 0 {
            if access & FILE_WRITE_DATA == 0 {
                return Err(UV_EINVAL);
            }
            access &= !FILE_APPEND_DATA;
        }
        attributes |= FILE_FLAG_NO_BUFFERING;
    }
    attributes |= match flags & (O_DSYNC | O_SYNC) {
        0 => 0,
        O_DSYNC | O_SYNC => FILE_FLAG_WRITE_THROUGH,
        _ => return Err(UV_EINVAL),
    };
    // Directories can be opened too.
    attributes |= FILE_FLAG_BACKUP_SEMANTICS;
    let handle = create(path, access, share, disposition, attributes);
    if handle == INVALID_HANDLE {
        let error = last_error();
        // An existing path that cannot be created over is a directory.
        if error == ERROR_FILE_EXISTS && flags & O_CREAT != 0 && flags & O_EXCL == 0 {
            return Err(UV_EISDIR);
        }
        return Err(translate(error));
    }
    Ok(handle)
}

/// libuv's positioned reads and writes keep the file pointer where it was.
fn transfer(handle: Handle, position: i64, buffers: &mut [&mut [u8]], read: bool) -> (usize, bool) {
    let mut original = 0;
    // SAFETY: the handle is valid and `original` is writable.
    let restore = position != -1 && unsafe { SetFilePointerEx(handle, 0, &mut original, 1) } != 0;
    let mut total = 0usize;
    let mut ok = true;
    for buffer in buffers.iter_mut() {
        let mut overlapped = Overlapped::default();
        let overlapped = if position != -1 {
            let offset = (position + total as i64) as u64;
            overlapped.offset = offset as u32;
            overlapped.offset_high = (offset >> 32) as u32;
            &mut overlapped as *mut Overlapped
        } else {
            core::ptr::null_mut()
        };
        let mut done = 0;
        let length = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        // SAFETY: the buffer is valid for `length` bytes.
        ok = unsafe {
            if read {
                ReadFile(
                    handle,
                    buffer.as_mut_ptr(),
                    length,
                    &mut done,
                    overlapped.cast(),
                )
            } else {
                WriteFile(
                    handle,
                    buffer.as_ptr(),
                    length,
                    &mut done,
                    overlapped.cast(),
                )
            }
        } != 0;
        total += done as usize;
        if !ok {
            break;
        }
    }
    if restore {
        // SAFETY: the handle is valid.
        unsafe { SetFilePointerEx(handle, original, core::ptr::null_mut(), 0) };
    }
    (total, ok)
}

/// `uv_fs_read`: `position` -1 reads from the file pointer.
pub fn read(handle: Handle, buffers: &mut [&mut [u8]], position: i64) -> Result<usize, i32> {
    let (total, ok) = transfer(handle, position, buffers, true);
    if ok || total > 0 {
        return Ok(total);
    }
    match last_error() {
        ERROR_HANDLE_EOF | ERROR_BROKEN_PIPE => Ok(total),
        ERROR_ACCESS_DENIED => Err(translate(ERROR_INVALID_FLAGS)),
        error => Err(translate(error)),
    }
}

/// `uv_fs_write`.
pub fn write(handle: Handle, buffers: &mut [&mut [u8]], position: i64) -> Result<usize, i32> {
    let (total, ok) = transfer(handle, position, buffers, false);
    if ok || total > 0 {
        return Ok(total);
    }
    Err(match last_error() {
        ERROR_ACCESS_DENIED => translate(ERROR_INVALID_FLAGS),
        ERROR_BROKEN_PIPE | ERROR_NO_DATA => UV_EPIPE,
        error => translate(error),
    })
}

fn set_information(handle: Handle, class: u32, data: &[u8]) -> Result<(), u32> {
    // SAFETY: the data is valid for its length.
    if unsafe { SetFileInformationByHandle(handle, class, data.as_ptr().cast(), data.len() as u32) }
        == 0
    {
        return Err(last_error());
    }
    Ok(())
}

fn basic_bytes(basic: &BasicInformation) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(40);
    for time in [basic.creation, basic.access, basic.write, basic.change] {
        bytes.extend_from_slice(&time.to_le_bytes());
    }
    bytes.extend_from_slice(&basic.attributes.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes
}

fn query_basic(handle: Handle) -> Result<BasicInformation, u32> {
    let mut basic = BasicInformation::default();
    // SAFETY: FILE_BASIC_INFO is plain data of this size.
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            0,
            (&mut basic as *mut BasicInformation).cast(),
            core::mem::size_of::<BasicInformation>() as u32,
        )
    };
    if ok == 0 {
        return Err(last_error());
    }
    Ok(basic)
}

/// `uv_fs_unlink` and `uv_fs_rmdir`: POSIX deletion, falling back to
/// clearing the read-only attribute and deleting on close.
pub fn remove(path: &[u16], directory: bool) -> Result<(), i32> {
    validate_path(path)?;
    let handle = create(
        path,
        FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES | DELETE,
        SHARE_ALL,
        OPEN_EXISTING,
        FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
    );
    if handle == INVALID_HANDLE {
        return Err(failure());
    }
    let result = remove_handle(handle, directory);
    close_handle(handle);
    result
}

fn remove_handle(handle: Handle, directory: bool) -> Result<(), i32> {
    let mut info = HandleInformation::default();
    // SAFETY: the structure is writable.
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(failure());
    }
    let is_directory = info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if directory && !is_directory {
        return Err(UV_ENOENT);
    }
    if !directory && is_directory {
        // Only links to directories may be unlinked.
        if info.attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
            return Err(translate(ERROR_ACCESS_DENIED));
        }
        if let Err(error) = readlink_handle(handle) {
            let error = if error == ERROR_SYMLINK_NOT_SUPPORTED {
                ERROR_ACCESS_DENIED
            } else {
                error
            };
            return Err(translate(error));
        }
    }
    // FILE_DISPOSITION_FLAG_DELETE, POSIX_SEMANTICS and IGNORE_READONLY_ATTRIBUTE.
    let posix = (0x1u32 | 0x2 | 0x10).to_le_bytes();
    let error = match set_information(handle, 21, &posix) {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    if !matches!(
        error,
        ERROR_NOT_SUPPORTED | ERROR_INVALID_PARAMETER | ERROR_INVALID_FUNCTION
    ) {
        return Err(translate(error));
    }
    if info.attributes & FILE_ATTRIBUTE_READONLY != 0 {
        let basic = BasicInformation {
            attributes: (info.attributes & !FILE_ATTRIBUTE_READONLY) | FILE_ATTRIBUTE_ARCHIVE,
            ..BasicInformation::default()
        };
        set_information(handle, 0, &basic_bytes(&basic)).map_err(translate)?;
    }
    set_information(handle, 4, &[1]).map_err(translate)
}

/// `uv_fs_mkdir`.
pub fn mkdir(path: &[u16]) -> Result<(), i32> {
    validate_path(path)?;
    // SAFETY: the path is null-terminated.
    if unsafe { CreateDirectoryW(path.as_ptr(), core::ptr::null_mut()) } != 0 {
        return Ok(());
    }
    match last_error() {
        ERROR_INVALID_NAME | ERROR_DIRECTORY => Err(UV_EINVAL),
        error => Err(translate(error)),
    }
}

/// `uv_fs_mkdtemp`: replaces the trailing `XXXXXX` with random characters
/// until a new directory is created.
pub fn mkdtemp(path: &mut [u16]) -> Result<(), i32> {
    validate_path(path)?;
    const CHARACTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let end = path.len() - 1;
    if end < 6 || path[end - 6..end] != *"XXXXXX".encode_utf16().collect::<Vec<_>>() {
        return Err(UV_EINVAL);
    }
    // TMP_MAX in the C runtime.
    for _ in 0..32_767 {
        let mut value = crate::platform::random();
        for unit in &mut path[end - 6..end] {
            *unit = u16::from(CHARACTERS[(value % 62) as usize]);
            value /= 62;
        }
        // SAFETY: the path is null-terminated.
        if unsafe { CreateDirectoryW(path.as_ptr(), core::ptr::null_mut()) } != 0 {
            return Ok(());
        }
        let error = last_error();
        if error != ERROR_ALREADY_EXISTS {
            return Err(translate(error));
        }
    }
    Err(translate(ERROR_ALREADY_EXISTS))
}

/// `uv_fs_scandir`: names and entry types, without `.` and `..`.
pub fn scandir(path: &[u16]) -> Result<Vec<(Vec<u16>, u8)>, i32> {
    validate_path(path)?;
    let handle = create(
        path,
        FILE_LIST_DIRECTORY | SYNCHRONIZE,
        SHARE_ALL,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS,
    );
    if handle == INVALID_HANDLE {
        return Err(failure());
    }
    let result = scan(handle);
    close_handle(handle);
    result
}

/// A native directory cursor. `read` advances the open Windows directory
/// handle and retains any entries read beyond the caller's requested batch.
#[derive(Debug)]
pub struct Directory {
    handle: Handle,
    pending: VecDeque<Dirent>,
    first: bool,
    ended: bool,
}

impl Directory {
    fn open(path: &[u16]) -> Result<Directory, i32> {
        validate_path(path)?;
        let handle = create(
            path,
            FILE_LIST_DIRECTORY | SYNCHRONIZE,
            SHARE_ALL,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
        );
        if handle == INVALID_HANDLE {
            return Err(failure());
        }
        let mut information = HandleInformation::default();
        // SAFETY: the new handle is open and the output struct is writable.
        if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0 {
            let error = failure();
            close_handle(handle);
            return Err(error);
        }
        if information.attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            close_handle(handle);
            return Err(UV_ENOTDIR);
        }
        Ok(Directory {
            handle,
            pending: VecDeque::new(),
            first: true,
            ended: false,
        })
    }

    fn read_batch(&mut self) -> Result<(), i32> {
        const FILE_FULL_DIRECTORY_INFO: u32 = 14;
        let mut buffer = vec![0u64; 1024];
        // SAFETY: buffer is writable and 8-byte aligned; the open directory
        // handle advances its native cursor as records are returned.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                self.handle,
                FILE_FULL_DIRECTORY_INFO,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
            )
        };
        if ok == 0 {
            return match last_error() {
                ERROR_NO_MORE_FILES => {
                    self.ended = true;
                    Ok(())
                }
                ERROR_INVALID_PARAMETER if self.first => Err(UV_ENOTDIR),
                error => Err(translate(error)),
            };
        }
        self.first = false;
        let bytes: &[u8] =
            // SAFETY: GetFileInformationByHandleEx initialized the returned
            // directory record bytes in this writable buffer.
            unsafe { core::slice::from_raw_parts(buffer.as_ptr().cast(), buffer.len() * 8) };
        let mut offset = 0;
        loop {
            let field = |at: usize| {
                u32::from_le_bytes([
                    bytes[offset + at],
                    bytes[offset + at + 1],
                    bytes[offset + at + 2],
                    bytes[offset + at + 3],
                ])
            };
            let next = field(0) as usize;
            let attributes = field(56);
            let length = field(60) as usize / 2;
            let mut name: Vec<u16> = bytes[offset + 68..offset + 68 + length * 2]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            while name.last() == Some(&0) {
                name.pop();
            }
            let dot = u16::from(b'.');
            if !name.is_empty() && name != [dot] && name != [dot, dot] {
                let kind = if attributes & FILE_ATTRIBUTE_DEVICE != 0 {
                    DIRENT_CHAR
                } else if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    DIRENT_LINK
                } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    DIRENT_DIR
                } else {
                    DIRENT_FILE
                };
                self.pending.push_back((name, kind));
            }
            if next == 0 {
                break;
            }
            offset += next;
        }
        Ok(())
    }

    /// Read up to `buffer_size` entries, returning `None` only at end of stream.
    pub fn read(&mut self, buffer_size: usize) -> Result<Option<DirentBatch>, i32> {
        if buffer_size == 0 {
            return Err(UV_EINVAL);
        }
        while self.pending.len() < buffer_size && !self.ended {
            self.read_batch()?;
        }
        if self.pending.is_empty() {
            return Ok(None);
        }
        let count = buffer_size.min(self.pending.len());
        Ok(Some(self.pending.drain(..count).collect()))
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        close_handle(self.handle);
    }
}

/// Open a directory handle for asynchronous notifications. Windows reports
/// file changes through the containing directory, so file watches retain the
/// basename for filtering at the binding boundary.
pub fn watch_open(path: &[u16]) -> Result<WatchHandle, i32> {
    validate_path(path)?;
    // SAFETY: `path` is null-terminated and valid for this read-only query.
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return Err(translate(last_error()));
    }
    let (directory, filename) = if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        (path.to_vec(), None)
    } else {
        let (parent, filename) = watch_parent_and_name(path);
        (parent, Some(filename))
    };
    // SAFETY: `directory` is a caller-provided null-terminated UTF-16 path or
    // the parent path derived above.
    let handle = unsafe {
        CreateFileW(
            directory.as_ptr(),
            FILE_LIST_DIRECTORY,
            SHARE_ALL,
            core::ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            core::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle == INVALID_HANDLE {
        Err(translate(last_error()))
    } else {
        Ok(WatchHandle { handle, filename })
    }
}

/// A native directory notification batch: Windows action code and UTF-16 name.
pub type WatchChanges = Vec<(u32, Vec<u16>)>;

/// Compare notification names using Windows' case-insensitive ordinal path
/// comparison, preserving Unicode and unpaired UTF-16 code units.
pub fn watch_name_matches(expected: &[u16], actual: &[u16]) -> bool {
    // Windows file-name components are limited to 255 UTF-16 units.
    let expected_length = expected.len() as i32;
    let actual_length = actual.len() as i32;
    // SAFETY: both pointers refer to initialized UTF-16 slices for their
    // supplied lengths; CompareStringOrdinal does not retain either pointer.
    unsafe {
        CompareStringOrdinal(
            expected.as_ptr(),
            expected_length,
            actual.as_ptr(),
            actual_length,
            1,
        ) == 2
    }
}

fn watch_parent_and_name(path: &[u16]) -> (Vec<u16>, Vec<u16>) {
    let end = path.len() - 1;
    match path[..end]
        .iter()
        .rposition(|unit| *unit == u16::from(b'\\') || *unit == u16::from(b'/'))
    {
        Some(separator) => {
            let mut parent = path[..separator + 1].to_vec();
            parent.push(0);
            (parent, path[separator + 1..end].to_vec())
        }
        None => (vec![u16::from(b'.'), 0], path[..end].to_vec()),
    }
}

/// A directory notification request Windows is filling. Windows records a
/// directory's changes from its first request on, so watchers begin the
/// request on the thread that starts watching, before it reports that the
/// watch exists, and wait for it on a worker.
#[derive(Debug)]
pub struct WatchRequest {
    event: Handle,
    // Boxed: Windows writes to the OVERLAPPED and the buffer until the
    // request completes, wherever the request moves meanwhile.
    overlapped: Box<Overlapped>,
    buffer: Vec<u8>,
}

// SAFETY: the request owns its heap buffers and event handle; Windows
// completes it whichever thread waits for it.
unsafe impl Send for WatchRequest {}

/// A begun notification read: a pending request, or the cancellation or
/// error that kept it from starting.
#[derive(Debug)]
pub struct WatchPending(Result<Option<WatchRequest>, i32>);

/// Begin one directory notification read, unless the watch was cancelled.
/// # Safety
/// `handle` must be an open directory handle owned by the watcher until the
/// read is completed with [`watch_complete`].
pub unsafe fn watch_begin(
    handle: Handle,
    recursive: bool,
    cancelled: &Arc<AtomicBool>,
) -> WatchPending {
    if cancelled.load(Ordering::Acquire) {
        return WatchPending(Ok(None));
    }
    // SAFETY: a manual-reset event is owned until this request completes.
    let event = unsafe { CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null()) };
    if event.is_null() {
        return WatchPending(Err(translate(last_error())));
    }
    let mut request = WatchRequest {
        event,
        overlapped: Box::new(Overlapped {
            event,
            ..Overlapped::default()
        }),
        buffer: vec![0_u8; 64 * 1024],
    };
    let filter = FILE_NOTIFY_CHANGE_FILE_NAME
        | FILE_NOTIFY_CHANGE_DIR_NAME
        | FILE_NOTIFY_CHANGE_ATTRIBUTES
        | FILE_NOTIFY_CHANGE_SIZE
        | FILE_NOTIFY_CHANGE_LAST_WRITE;
    // SAFETY: the open directory handle, the heap buffer and the boxed
    // OVERLAPPED all remain alive until the event signals or cancellation
    // completes in `watch_complete`.
    let started = unsafe {
        ReadDirectoryChangesW(
            handle,
            request.buffer.as_mut_ptr(),
            request.buffer.len() as u32,
            i32::from(recursive),
            filter,
            core::ptr::null_mut(),
            &mut *request.overlapped,
            None,
        )
    };
    if started == 0 {
        let error = last_error();
        close_handle(event);
        return WatchPending(Err(translate(error)));
    }
    WatchPending(Ok(Some(request)))
}

/// Wait for a begun notification read. Cancellation is checked between short
/// waits, and the overlapped request is canceled before its event is released.
/// # Safety
/// `handle` must be the open directory handle `pending` was begun on; the
/// same handle must not be closed concurrently.
pub unsafe fn watch_complete(
    handle: Handle,
    pending: WatchPending,
    cancelled: &Arc<AtomicBool>,
) -> Result<Option<WatchChanges>, i32> {
    let mut request = match pending.0 {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(None),
        Err(error) => return Err(error),
    };
    let event = request.event;
    loop {
        if cancelled.load(Ordering::Acquire) {
            // SAFETY: this handle is open and this request belongs to it.
            unsafe {
                CancelIoEx(handle, &mut *request.overlapped);
            }
        }
        // SAFETY: the event remains open through completion below.
        match unsafe { WaitForSingleObject(event, 100) } {
            WAIT_OBJECT_0 => break,
            WAIT_TIMEOUT => continue,
            WAIT_FAILED => {
                let error = last_error();
                // SAFETY: cancel before the request is dropped.
                unsafe {
                    CancelIoEx(handle, &mut *request.overlapped);
                    WaitForSingleObject(event, u32::MAX);
                }
                close_handle(event);
                return Err(translate(error));
            }
            _ => {
                // SAFETY: cancel before the request is dropped.
                unsafe {
                    CancelIoEx(handle, &mut *request.overlapped);
                    WaitForSingleObject(event, u32::MAX);
                }
                close_handle(event);
                return Err(UV_EINVAL);
            }
        }
    }

    if cancelled.load(Ordering::Acquire) {
        close_handle(event);
        return Ok(None);
    }
    let mut returned = 0_u32;
    // SAFETY: the signaled event completed the request and `returned` is writable.
    let succeeded =
        unsafe { GetOverlappedResult(handle, &mut *request.overlapped, &mut returned, 0) };
    if succeeded == 0 {
        let error = last_error();
        close_handle(event);
        if cancelled.load(Ordering::Acquire) {
            return Ok(None);
        }
        return Err(translate(error));
    }
    close_handle(event);
    parse_watch_changes(&request.buffer, returned as usize).map(Some)
}

/// Wait for one directory notification: [`watch_begin`] then
/// [`watch_complete`] on the calling thread.
/// # Safety
/// `handle` must be an open directory handle owned by the watcher until this
/// call returns; the same handle must not be closed concurrently.
pub unsafe fn watch_read(
    handle: Handle,
    recursive: bool,
    cancelled: &Arc<AtomicBool>,
) -> Result<Option<WatchChanges>, i32> {
    // SAFETY: the caller's guarantees cover both halves of the read.
    unsafe {
        let pending = watch_begin(handle, recursive, cancelled);
        watch_complete(handle, pending, cancelled)
    }
}

/// Cancel a pending request without releasing its handle before completion.
///
/// # Safety
/// `handle` must still be an open directory handle owned by the caller.
pub unsafe fn watch_cancel(handle: Handle) {
    // SAFETY: a null OVERLAPPED cancels all requests associated with the handle.
    unsafe {
        CancelIoEx(handle, core::ptr::null_mut());
    }
}

/// Release a watcher handle after its pending request has completed.
pub fn watch_close(handle: Handle) {
    close_handle(handle);
}

fn parse_watch_changes(buffer: &[u8], used: usize) -> Result<WatchChanges, i32> {
    if used > buffer.len() {
        return Err(UV_EINVAL);
    }
    let mut changes = Vec::new();
    let mut offset = 0_usize;
    while offset < used {
        if used - offset < 12 {
            return Err(UV_EINVAL);
        }
        let field = |at: usize| {
            u32::from_le_bytes(buffer[offset + at..offset + at + 4].try_into().unwrap())
        };
        let next = field(0) as usize;
        let action = field(4);
        let name_bytes = field(8) as usize;
        if !name_bytes.is_multiple_of(2) || name_bytes > used - offset - 12 {
            return Err(UV_EINVAL);
        }
        let name = buffer[offset + 12..offset + 12 + name_bytes]
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        changes.push((action, name));
        if next == 0 {
            break;
        }
        if next < 12 || next > used - offset {
            return Err(UV_EINVAL);
        }
        offset += next;
    }
    Ok(changes)
}

/// Open directory cursors addressed by small native IDs for the private binding.
#[derive(Debug, Default)]
pub struct Directories {
    handles: Vec<Option<Directory>>,
}

impl Directories {
    /// Open a real Windows directory cursor.
    pub fn open(&mut self, path: &[u16]) -> Result<i32, i32> {
        let directory = Directory::open(path)?;
        if self.handles.len() < 3 {
            self.handles.resize_with(3, || None);
        }
        let index = (3..self.handles.len())
            .find(|index| self.handles[*index].is_none())
            .unwrap_or(self.handles.len());
        if index == self.handles.len() {
            self.handles.push(None);
        }
        self.handles[index] = Some(directory);
        Ok(index as i32)
    }

    /// Read a batch from an open cursor, or `EBADF` for a closed/unknown ID.
    pub fn read(&mut self, id: i32, buffer_size: usize) -> Result<Option<DirentBatch>, i32> {
        let directory = usize::try_from(id)
            .ok()
            .and_then(|index| self.handles.get_mut(index))
            .and_then(Option::as_mut)
            .ok_or(UV_EBADF)?;
        directory.read(buffer_size)
    }

    /// Close a cursor and release its kernel handle.
    pub fn close(&mut self, id: i32) -> Result<(), i32> {
        let directory = usize::try_from(id)
            .ok()
            .and_then(|index| self.handles.get_mut(index))
            .and_then(Option::take)
            .ok_or(UV_EBADF)?;
        drop(directory);
        Ok(())
    }
}

fn scan(handle: Handle) -> Result<Vec<(Vec<u16>, u8)>, i32> {
    const FILE_FULL_DIRECTORY_INFO: u32 = 14;
    let mut buffer = vec![0u64; 1024];
    let mut entries = Vec::new();
    let mut first = true;
    loop {
        // SAFETY: the buffer is writable and 8-byte aligned.
        let ok = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FILE_FULL_DIRECTORY_INFO,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
            )
        };
        if ok == 0 {
            return match last_error() {
                ERROR_NO_MORE_FILES => Ok(entries),
                ERROR_INVALID_PARAMETER if first => Err(UV_ENOTDIR),
                error => Err(translate(error)),
            };
        }
        first = false;
        let bytes: &[u8] =
            // SAFETY: the buffer's bytes are initialized.
            unsafe { core::slice::from_raw_parts(buffer.as_ptr().cast(), buffer.len() * 8) };
        let mut offset = 0;
        loop {
            let field = |at: usize| {
                u32::from_le_bytes([
                    bytes[offset + at],
                    bytes[offset + at + 1],
                    bytes[offset + at + 2],
                    bytes[offset + at + 3],
                ])
            };
            let next = field(0) as usize;
            let attributes = field(56);
            let length = field(60) as usize / 2;
            let mut name: Vec<u16> = bytes[offset + 68..offset + 68 + length * 2]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            while name.last() == Some(&0) {
                name.pop();
            }
            let dot = u16::from(b'.');
            let skip = name.is_empty() || name == [dot] || name == [dot, dot];
            if !skip {
                let kind = if attributes & FILE_ATTRIBUTE_DEVICE != 0 {
                    DIRENT_CHAR
                } else if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    DIRENT_LINK
                } else if attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    DIRENT_DIR
                } else {
                    DIRENT_FILE
                };
                entries.push((name, kind));
            }
            if next == 0 {
                break;
            }
            offset += next;
        }
    }
}

/// `uv_stat_t` on Windows.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Stat {
    pub dev: u64,
    pub mode: u32,
    pub nlink: u64,
    pub rdev: u64,
    pub blksize: u64,
    pub ino: u64,
    pub size: u64,
    pub blocks: u64,
    /// Access, modification, change and birth times as (seconds, nanoseconds).
    pub times: [(i64, i64); 4],
}

/// `uv__filetime_to_timespec`.
fn timespec(filetime: i64) -> (i64, i64) {
    let since_epoch = filetime - 116_444_736_000_000_000;
    let mut seconds = since_epoch / 10_000_000;
    let mut nanoseconds = (since_epoch - seconds * 10_000_000) * 100;
    if nanoseconds < 0 {
        seconds -= 1;
        nanoseconds += 1_000_000_000;
    }
    (seconds, nanoseconds)
}

/// libuv's statistics for a character device or pipe.
fn device_stat(mode: u32, device: u64, ino: u64) -> Stat {
    Stat {
        mode,
        nlink: 1,
        rdev: device << 16,
        ino,
        ..Stat::default()
    }
}

/// `fs__stat_handle`.
fn stat_handle(handle: Handle, link: bool) -> Result<Stat, u32> {
    // SAFETY: GetFileType has no preconditions.
    if unsafe { GetFileType(handle) } == 2 {
        // FILE_TYPE_CHAR: libuv's statistics for the NUL device.
        let mut stat = device_stat(S_IFCHR | 0o666, 0x15, 0);
        stat.blksize = 4096;
        return Ok(stat);
    }
    let mut info = HandleInformation::default();
    // SAFETY: the structure is writable.
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(last_error());
    }
    let basic = query_basic(handle)?;
    let mut standard = StandardInformation::default();
    // SAFETY: FILE_STANDARD_INFO is plain data of this size.
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            1,
            (&mut standard as *mut StandardInformation).cast(),
            core::mem::size_of::<StandardInformation>() as u32,
        )
    } == 0
    {
        return Err(last_error());
    }
    let mut size = standard.end_of_file as u64;
    let mut mode = 0;
    if link && basic.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        // A reparse point that is not a link is reported as the caller retries.
        // libuv counts the converted WTF-8 bytes, including three bytes for
        // an unpaired surrogate. utf8_length has the same length convention.
        size = crate::encoding::utf8_length(&readlink_handle(handle)?) as u64;
        mode = S_IFLNK;
    }
    if mode == 0 {
        if basic.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            mode = S_IFDIR;
            size = 0;
        } else {
            mode = S_IFREG;
        }
    }
    mode |= if basic.attributes & FILE_ATTRIBUTE_READONLY != 0 {
        0o444
    } else {
        0o666
    };
    Ok(Stat {
        dev: u64::from(info.volume_serial),
        mode,
        nlink: u64::from(standard.links),
        rdev: 0,
        blksize: 4096,
        ino: (u64::from(info.index_high) << 32) | u64::from(info.index_low),
        size,
        blocks: (standard.allocation as u64) >> 9,
        times: [
            timespec(basic.access),
            timespec(basic.write),
            timespec(basic.change),
            timespec(basic.creation),
        ],
    })
}

/// `uv_fs_stat` and `uv_fs_lstat`.
pub fn stat(path: &[u16], link: bool) -> Result<Stat, i32> {
    validate_path(path)?;
    let mut path = path.to_vec();
    // A trailing separator is dropped, except after a drive.
    let length = path.len() - 1;
    if length > 1 && path[length - 2] != u16::from(b':') && matches!(path[length - 1], 0x5c | 0x2f)
    {
        path[length - 1] = 0;
        path.truncate(length);
    }
    stat_path(&path, link)
}

fn stat_path(path: &[u16], link: bool) -> Result<Stat, i32> {
    let flags = if link {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    } else {
        FILE_FLAG_BACKUP_SEMANTICS
    };
    let handle = create(path, FILE_READ_ATTRIBUTES, SHARE_ALL, OPEN_EXISTING, flags);
    if handle == INVALID_HANDLE {
        return Err(failure());
    }
    let result = stat_handle(handle, link);
    close_handle(handle);
    match result {
        Ok(stat) => Ok(stat),
        // A reparse point that is not a link: stat its target instead.
        Err(ERROR_SYMLINK_NOT_SUPPORTED | ERROR_NOT_A_REPARSE_POINT) if link => {
            stat_path(path, false)
        }
        Err(error) => Err(translate(error)),
    }
}

/// `uv_fs_fstat`, which describes consoles and pipes too.
// Windows validates this opaque kernel identifier; it is not a Rust pointer.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn fstat(handle: Handle) -> Result<Stat, i32> {
    // SAFETY: GetFileType has no preconditions.
    match unsafe { GetFileType(handle) } {
        2 => {
            let mut mode = 0;
            // SAFETY: `mode` is writable.
            if unsafe { GetConsoleMode(handle, &mut mode) } != 0 {
                // FILE_DEVICE_CONSOLE.
                return Ok(device_stat(S_IFCHR, 0x50, handle as u64));
            }
            stat_handle(handle, false).map_err(translate)
        }
        // FILE_TYPE_PIPE and FILE_DEVICE_NAMED_PIPE.
        3 => Ok(device_stat(S_IFIFO, 0x11, handle as u64)),
        1 => stat_handle(handle, false).map_err(translate),
        _ => Err(translate(ERROR_INVALID_HANDLE)),
    }
}

/// `uv_fs_rename`, replacing an existing destination.
pub fn rename(from: &[u16], to: &[u16]) -> Result<(), i32> {
    validate_path(from)?;
    validate_path(to)?;
    // SAFETY: both paths are null-terminated.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 1) } == 0 {
        return Err(failure());
    }
    Ok(())
}

/// `uv_fs_fsync` and `uv_fs_fdatasync`.
// Windows validates this opaque kernel identifier; it is not a Rust pointer.
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub fn fsync(handle: Handle) -> Result<(), i32> {
    // SAFETY: the handle is valid.
    if unsafe { FlushFileBuffers(handle) } == 0 {
        return Err(failure());
    }
    Ok(())
}

/// `uv_fs_ftruncate`.
pub fn ftruncate(handle: Handle, length: i64) -> Result<(), i32> {
    set_information(handle, 6, &length.to_le_bytes()).map_err(translate)
}

/// `uv_fs_copyfile`: copying a file onto itself succeeds.
pub fn copyfile(from: &[u16], to: &[u16], flags: i32) -> Result<(), i32> {
    validate_path(from)?;
    validate_path(to)?;
    if flags & COPYFILE_FICLONE_FORCE != 0 {
        return Err(UV_ENOSYS);
    }
    let exclusive = i32::from(flags & COPYFILE_EXCL != 0);
    // SAFETY: both paths are null-terminated.
    if unsafe { CopyFileW(from.as_ptr(), to.as_ptr(), exclusive) } != 0 {
        return Ok(());
    }
    let error = failure();
    if error == UV_EBUSY
        && let (Ok(source), Ok(target)) = (stat_path(from, false), stat_path(to, false))
        && source.dev == target.dev
        && source.ino == target.ino
    {
        return Ok(());
    }
    Err(error)
}

/// `uv_fs_access`: only write access to a read-only file is refused.
pub fn access(path: &[u16], mode: i32) -> Result<(), i32> {
    validate_path(path)?;
    // SAFETY: the path is null-terminated.
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    if attributes == u32::MAX {
        return Err(failure());
    }
    if mode & W_OK == 0
        || attributes & FILE_ATTRIBUTE_READONLY == 0
        || attributes & FILE_ATTRIBUTE_DIRECTORY != 0
    {
        return Ok(());
    }
    Err(UV_EPERM)
}

/// `uv_fs_chmod`, as the C runtime's `_wchmod`: only the write bit counts.
pub fn chmod(path: &[u16], mode: i32) -> Result<(), i32> {
    validate_path(path)?;
    // SAFETY: the path is null-terminated.
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    if attributes == u32::MAX {
        return Err(failure());
    }
    let attributes = if (mode as u32) & S_IWRITE != 0 {
        attributes & !FILE_ATTRIBUTE_READONLY
    } else {
        attributes | FILE_ATTRIBUTE_READONLY
    };
    // SAFETY: the path is null-terminated.
    if unsafe { SetFileAttributesW(path.as_ptr(), attributes) } == 0 {
        return Err(failure());
    }
    Ok(())
}

/// `uv_fs_fchmod`: sets the read-only attribute through the handle, with
/// the archive attribute set meanwhile as Windows requires.
pub fn fchmod(handle: Handle, mode: i32) -> Result<(), i32> {
    let mut basic = query_basic(handle).map_err(translate)?;
    let archive = basic.attributes & FILE_ATTRIBUTE_ARCHIVE == 0;
    if archive {
        basic.attributes |= FILE_ATTRIBUTE_ARCHIVE;
        set_information(handle, 0, &basic_bytes(&basic)).map_err(translate)?;
    }
    if (mode as u32) & S_IWRITE != 0 {
        basic.attributes &= !FILE_ATTRIBUTE_READONLY;
    } else {
        basic.attributes |= FILE_ATTRIBUTE_READONLY;
    }
    set_information(handle, 0, &basic_bytes(&basic)).map_err(translate)?;
    if archive {
        basic.attributes &= !FILE_ATTRIBUTE_ARCHIVE;
        if basic.attributes == 0 {
            basic.attributes = FILE_ATTRIBUTE_NORMAL;
        }
        set_information(handle, 0, &basic_bytes(&basic)).map_err(translate)?;
    }
    Ok(())
}

/// libuv's `TIME_T_TO_FILETIME`.
fn filetime(seconds: f64) -> u64 {
    (seconds * 10_000_000.0 + 116_444_736_000_000_000.0) as i64 as u64
}

fn utime_handle(handle: Handle, atime: f64, mtime: f64) -> Result<(), u32> {
    let (access, write) = (filetime(atime), filetime(mtime));
    // SAFETY: both times are valid for reading.
    if unsafe { SetFileTime(handle, core::ptr::null(), &access, &write) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

/// `uv_fs_utime` and `uv_fs_lutime`.
pub fn utime(path: &[u16], atime: f64, mtime: f64, link: bool) -> Result<(), i32> {
    validate_path(path)?;
    let flags = if link {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    } else {
        FILE_FLAG_BACKUP_SEMANTICS
    };
    let handle = create(path, FILE_WRITE_ATTRIBUTES, SHARE_ALL, OPEN_EXISTING, flags);
    if handle == INVALID_HANDLE {
        return Err(failure());
    }
    let result = utime_handle(handle, atime, mtime);
    close_handle(handle);
    result.map_err(translate)
}

/// `uv_fs_futime`.
pub fn futime(handle: Handle, atime: f64, mtime: f64) -> Result<(), i32> {
    utime_handle(handle, atime, mtime).map_err(translate)
}

/// `uv_fs_link`: a hard link at `to` for the file at `from`.
pub fn link(from: &[u16], to: &[u16]) -> Result<(), i32> {
    validate_path(from)?;
    validate_path(to)?;
    // SAFETY: both paths are null-terminated.
    if unsafe { CreateHardLinkW(to.as_ptr(), from.as_ptr(), core::ptr::null_mut()) } == 0 {
        return Err(failure());
    }
    Ok(())
}

/// `uv_fs_symlink`: a link at `path` to `target`; directory links and
/// junctions as `flags` asks.
pub fn symlink(target: &[u16], path: &[u16], flags: i32) -> Result<(), i32> {
    validate_path(target)?;
    validate_path(path)?;
    // SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE.
    symlink_with(target, path, flags, 0x2)
}

fn symlink_with(target: &[u16], path: &[u16], flags: i32, unprivileged: u32) -> Result<(), i32> {
    if flags & SYMLINK_JUNCTION != 0 {
        return junction(target, path);
    }
    let directory = if flags & SYMLINK_DIR != 0 { 0x1 } else { 0 };
    // SAFETY: both paths are null-terminated.
    if unsafe { CreateSymbolicLinkW(path.as_ptr(), target.as_ptr(), directory | unprivileged) } != 0
    {
        return Ok(());
    }
    let error = last_error();
    // Systems without unprivileged links refuse the flag.
    if error == ERROR_INVALID_PARAMETER && unprivileged != 0 {
        return symlink_with(target, path, flags, 0);
    }
    Err(translate(error))
}

const LONG_PREFIX: [u16; 4] = [0x5c, 0x5c, 0x3f, 0x5c];

fn is_slash(unit: u16) -> bool {
    unit == 0x5c || unit == 0x2f
}

/// libuv's `fs__create_junction`: a mount point at `path` to the absolute
/// directory `target`.
fn junction(target: &[u16], path: &[u16]) -> Result<(), i32> {
    let target = &target[..target.len() - 1];
    let long = target.starts_with(&LONG_PREFIX);
    let absolute = long
        || (target.len() >= 3
            && (target[0] as u8).is_ascii_alphabetic()
            && target[0] < 0x80
            && target[1] == u16::from(b':')
            && is_slash(target[2]));
    if !absolute {
        return Err(UV_EINVAL);
    }
    let skip = if long { 4 } else { 0 };
    // Collapses separators to single backslashes.
    let normalized = |trailing: bool| {
        let mut units = Vec::new();
        let mut slash = false;
        for unit in &target[skip..] {
            if is_slash(*unit) {
                slash = true;
                continue;
            }
            if slash {
                units.push(0x5c);
                slash = false;
            }
            units.push(*unit);
        }
        if slash && trailing {
            units.push(0x5c);
        }
        (units, slash)
    };
    let mut substitute: Vec<u16> = vec![0x5c, 0x3f, 0x3f, 0x5c];
    substitute.extend(normalized(true).0);
    let (mut print, slash) = normalized(false);
    if print.len() == 2 || slash {
        print.push(0x5c);
    }
    let mut names = substitute.clone();
    names.push(0);
    let print_offset = names.len() * 2;
    names.extend(&print);
    names.push(0);
    // Windows limits a reparse point to 16 KiB, including its header.
    // Reject before narrowing the offsets and data length to u16.
    if 16 + names.len() * 2 > 16 * 1024 {
        return Err(translate(206)); // ERROR_FILENAME_EXCED_RANGE.
    }
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&0xA000_0003u32.to_le_bytes());
    buffer.extend_from_slice(&((8 + names.len() * 2) as u16).to_le_bytes());
    buffer.extend_from_slice(&[0, 0]);
    for value in [0, substitute.len() * 2, print_offset, print.len() * 2] {
        buffer.extend_from_slice(&(value as u16).to_le_bytes());
    }
    for unit in names {
        buffer.extend_from_slice(&unit.to_le_bytes());
    }
    // SAFETY: the path is null-terminated.
    if unsafe { CreateDirectoryW(path.as_ptr(), core::ptr::null_mut()) } == 0 {
        return Err(failure());
    }
    let handle = create(
        path,
        GENERIC_WRITE,
        0,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
    );
    let result = if handle == INVALID_HANDLE {
        Err(failure())
    } else {
        const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00A4;
        let mut returned = 0;
        // SAFETY: the buffer holds the reparse data for its length.
        let ok = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_SET_REPARSE_POINT,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                core::ptr::null_mut(),
                0,
                &mut returned,
                core::ptr::null_mut(),
            )
        };
        let result = if ok == 0 { Err(failure()) } else { Ok(()) };
        close_handle(handle);
        result
    };
    if result.is_err() {
        // SAFETY: the path is null-terminated.
        unsafe { RemoveDirectoryW(path.as_ptr()) };
    }
    result
}

/// `fs__readlink_handle`: the target of a symbolic link, a junction to a
/// drive path, or an app execution alias; a Windows error otherwise.
fn readlink_handle(handle: Handle) -> Result<Vec<u16>, u32> {
    const FSCTL_GET_REPARSE_POINT: u32 = 0x0009_00A8;
    let mut buffer = vec![0u8; 16 * 1024];
    let mut returned = 0;
    // SAFETY: the buffer is writable for its length.
    if unsafe {
        DeviceIoControl(
            handle,
            FSCTL_GET_REPARSE_POINT,
            core::ptr::null(),
            0,
            buffer.as_mut_ptr().cast(),
            buffer.len() as u32,
            &mut returned,
            core::ptr::null_mut(),
        )
    } == 0
    {
        return Err(last_error());
    }
    let returned = returned as usize;
    let buffer = buffer.get(..returned).ok_or(ERROR_INVALID_REPARSE_DATA)?;
    reparse_target(buffer)
}

/// The link target in reparse data, as libuv reads it.
pub fn reparse_target(buffer: &[u8]) -> Result<Vec<u16>, u32> {
    let header = buffer.get(..8).ok_or(ERROR_INVALID_REPARSE_DATA)?;
    let length = usize::from(u16::from_le_bytes([header[4], header[5]]));
    let buffer = buffer.get(..8 + length).ok_or(ERROR_INVALID_REPARSE_DATA)?;
    let word = |at: usize| -> Result<usize, u32> {
        let bytes = buffer.get(at..at + 2).ok_or(ERROR_INVALID_REPARSE_DATA)?;
        Ok(usize::from(u16::from_le_bytes([bytes[0], bytes[1]])))
    };
    let units = |start: usize, bytes: usize| -> Result<Vec<u16>, u32> {
        if !start.is_multiple_of(2) || !bytes.is_multiple_of(2) {
            return Err(ERROR_INVALID_REPARSE_DATA);
        }
        let bytes = buffer
            .get(start..start + bytes)
            .ok_or(ERROR_INVALID_REPARSE_DATA)?;
        Ok(bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect())
    };
    let drive = |target: &[u16]| {
        target.len() >= 6
            && target[..4] == [0x5c, 0x3f, 0x3f, 0x5c]
            && target[4] < 0x80
            && (target[4] as u8).is_ascii_alphabetic()
            && target[5] == u16::from(b':')
            && (target.len() == 6 || target[6] == 0x5c)
    };
    match u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) {
        // IO_REPARSE_TAG_SYMLINK.
        0xA000_000C => {
            buffer.get(..20).ok_or(ERROR_INVALID_REPARSE_DATA)?;
            let target = units(20 + word(8)?, word(10)?)?;
            if drive(&target) {
                return Ok(target[4..].to_vec());
            }
            let unc = target.len() >= 8
                && target[..4] == [0x5c, 0x3f, 0x3f, 0x5c]
                && String::from_utf16_lossy(&target[4..7]).eq_ignore_ascii_case("UNC")
                && target[7] == 0x5c;
            if unc {
                let mut path = target[6..].to_vec();
                path[0] = 0x5c;
                return Ok(path);
            }
            Ok(target)
        }
        // IO_REPARSE_TAG_MOUNT_POINT: junctions to drive paths only.
        0xA000_0003 => {
            buffer.get(..16).ok_or(ERROR_INVALID_REPARSE_DATA)?;
            let target = units(16 + word(8)?, word(10)?)?;
            if !drive(&target) {
                return Err(ERROR_SYMLINK_NOT_SUPPORTED);
            }
            Ok(target[4..].to_vec())
        }
        // IO_REPARSE_TAG_APPEXECLINK: the third string, an absolute path.
        0x8000_001B => {
            buffer.get(..12).ok_or(ERROR_INVALID_REPARSE_DATA)?;
            let count = u32::from_le_bytes([buffer[8], buffer[9], buffer[10], buffer[11]]);
            let strings: Vec<&[u16]> = Vec::new();
            let all = units(12, buffer.len() - 12)?;
            let mut strings = strings;
            for string in all.split_inclusive(|unit| *unit == 0).take(3) {
                let Some((&0, string)) = string.split_last() else {
                    return Err(ERROR_INVALID_REPARSE_DATA);
                };
                strings.push(string);
            }
            let target = match strings.as_slice() {
                [first, second, third] if count >= 3 && !first.is_empty() && !second.is_empty() => {
                    *third
                }
                _ => return Err(ERROR_SYMLINK_NOT_SUPPORTED),
            };
            let absolute = target.len() >= 3
                && target[0] < 0x80
                && (target[0] as u8).is_ascii_alphabetic()
                && target[1] == u16::from(b':')
                && target[2] == 0x5c;
            if !absolute {
                return Err(ERROR_SYMLINK_NOT_SUPPORTED);
            }
            Ok(target.to_vec())
        }
        _ => Err(ERROR_SYMLINK_NOT_SUPPORTED),
    }
}

/// `uv_fs_readlink`: not being a link is `EINVAL`.
pub fn readlink(path: &[u16]) -> Result<Vec<u16>, i32> {
    validate_path(path)?;
    let handle = create(
        path,
        0,
        0,
        OPEN_EXISTING,
        FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
    );
    if handle == INVALID_HANDLE {
        return Err(failure());
    }
    let result = readlink_handle(handle);
    close_handle(handle);
    result.map_err(|error| match error {
        ERROR_NOT_A_REPARSE_POINT => UV_EINVAL,
        error => translate(error),
    })
}

/// `uv_fs_realpath`: the final path, without the `\\?\` prefix.
pub fn realpath(path: &[u16]) -> Result<Vec<u16>, i32> {
    validate_path(path)?;
    let handle = create(
        path,
        0,
        0,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL | FILE_FLAG_BACKUP_SEMANTICS,
    );
    if handle == INVALID_HANDLE {
        return Err(failure());
    }
    let result = final_path(handle);
    close_handle(handle);
    result.map_err(translate)
}

fn final_path(handle: Handle) -> Result<Vec<u16>, u32> {
    // SAFETY: a null buffer measures the path.
    let length = unsafe { GetFinalPathNameByHandleW(handle, core::ptr::null_mut(), 0, 0) };
    if length == 0 {
        return Err(last_error());
    }
    let mut path = vec![0u16; length as usize + 1];
    // SAFETY: the buffer holds `length` units.
    let written = unsafe { GetFinalPathNameByHandleW(handle, path.as_mut_ptr(), length, 0) };
    // A concurrent rename may grow the path between the two queries.
    // In that case Windows reports the required size, not a written path.
    if written == 0 || written >= length {
        return Err(ERROR_INVALID_HANDLE);
    }
    path.truncate(written as usize);
    let unc: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    if path.starts_with(&unc) {
        let mut path = path[6..].to_vec();
        path[0] = 0x5c;
        return Ok(path);
    }
    if path.starts_with(&LONG_PREFIX) {
        return Ok(path[4..].to_vec());
    }
    Err(ERROR_INVALID_HANDLE)
}

/// `uv_statfs_t` on Windows.
#[derive(Debug, Default, PartialEq)]
pub struct StatFs {
    pub bsize: u64,
    pub blocks: u64,
    pub bfree: u64,
}

/// `uv_fs_statfs`: retried with the path's directory when it names a file.
pub fn statfs(path: &[u16]) -> Result<StatFs, i32> {
    validate_path(path)?;
    let (mut sectors, mut bytes, mut free, mut total) = (0, 0, 0, 0);
    // SAFETY: the path is null-terminated and the outputs are writable.
    let query = |path: &[u16],
                 sectors: &mut u32,
                 bytes: &mut u32,
                 free: &mut u32,
                 total: &mut u32| unsafe {
        GetDiskFreeSpaceW(path.as_ptr(), sectors, bytes, free, total)
    };
    if query(path, &mut sectors, &mut bytes, &mut free, &mut total) == 0 {
        let error = last_error();
        if error != ERROR_DIRECTORY {
            return Err(translate(error));
        }
        let mut full = vec![0u16; 32_768];
        let mut file = core::ptr::null_mut();
        // SAFETY: the buffer holds its length; `file` points into it.
        let length = unsafe {
            GetFullPathNameW(
                path.as_ptr(),
                full.len() as u32,
                full.as_mut_ptr(),
                &mut file,
            )
        };
        if length == 0 {
            return Err(translate(error));
        }
        if length as usize >= full.len() {
            return Err(translate(206)); // ERROR_FILENAME_EXCED_RANGE.
        }
        if !file.is_null() {
            // SAFETY: `file` points into `full`.
            unsafe { *file = 0 };
        }
        if query(&full, &mut sectors, &mut bytes, &mut free, &mut total) == 0 {
            return Err(failure());
        }
    }
    let block = u64::from(sectors) * u64::from(bytes);
    Ok(StatFs {
        bsize: block,
        blocks: u64::from(total),
        bfree: u64::from(free),
    })
}

/// Electron applications reach files in their packed `app.asar` archives
/// and the `.asar.unpacked` folders beside them; a native build has neither,
/// its files being in the folders those archives were packed from. `path`
/// (absolute, without a terminator) with each missing `NAME.asar` or
/// `NAME.asar.unpacked` component replaced by an existing `NAME`, or `None`
/// when none is.
pub fn without_asar(path: &[u16]) -> Option<Vec<u16>> {
    without_asar_in(path, |candidate| {
        let name: Vec<u16> = candidate.iter().copied().chain([0]).collect();
        // SAFETY: a null-terminated name.
        unsafe { GetFileAttributesW(name.as_ptr()) != u32::MAX }
    })
}

fn without_asar_in(path: &[u16], exists: impl Fn(&[u16]) -> bool) -> Option<Vec<u16>> {
    const SUFFIXES: [&str; 2] = [".asar.unpacked", ".asar"];
    let lower: Vec<u16> = path
        .iter()
        .map(|&unit| {
            if (65..=90).contains(&unit) {
                unit + 32
            } else {
                unit
            }
        })
        .collect();
    let marker: Vec<u16> = ".asar".encode_utf16().collect();
    if !lower
        .windows(marker.len())
        .any(|window| window == marker.as_slice())
    {
        return None;
    }
    let separator = |unit: u16| unit == u16::from(b'\\') || unit == u16::from(b'/');
    let mut mapped: Vec<u16> = Vec::with_capacity(path.len());
    let mut changed = false;
    let mut start = 0;
    while start <= path.len() {
        let end = path[start..]
            .iter()
            .position(|&unit| separator(unit))
            .map_or(path.len(), |offset| start + offset);
        let component = &path[start..end];
        let lower_component = &lower[start..end];
        let suffix = SUFFIXES.iter().find_map(|suffix| {
            let suffix: Vec<u16> = suffix.encode_utf16().collect();
            (lower_component.len() > suffix.len() && lower_component.ends_with(&suffix))
                .then_some(suffix.len())
        });
        let mut kept = component;
        if let Some(length) = suffix {
            let whole = [mapped.as_slice(), component].concat();
            if !exists(&whole) {
                let stripped = &component[..component.len() - length];
                if exists(&[mapped.as_slice(), stripped].concat()) {
                    kept = stripped;
                    changed = true;
                }
            }
        }
        mapped.extend_from_slice(kept);
        if end < path.len() {
            mapped.push(path[end]);
        }
        start = end + 1;
    }
    changed.then_some(mapped)
}

#[cfg(test)]
mod tests;
