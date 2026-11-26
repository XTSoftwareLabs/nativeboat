//! Native Windows filesystem binding for compiled Node sources.
use crate::prelude::*;
use crate::typed_array_builtin::{self, Element, View};
use crate::{
    Completion, ErrorKind, JsStr, Key, Object, Property, Realm, Symbol, Thrown, Value, ops,
};
use alloc::{rc::Rc, sync::Arc};
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU64, Ordering};

mod stat_watcher;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn AcquireSRWLockExclusive(lock: *mut usize);
    fn ReleaseSRWLockExclusive(lock: *mut usize);
    fn GetFullPathNameW(
        path: *const u16,
        length: u32,
        buffer: *mut u16,
        part: *mut *mut u16,
    ) -> u32;
    fn GetLastError() -> u32;
}

/// Descriptor operations hold one native lock for lookup, I/O and close;
/// workers never receive a borrowed handle that can be closed/reused beneath
/// them. Path-only requests do not take this lock.
#[derive(Debug)]
pub(crate) struct Files {
    lock: UnsafeCell<usize>,
    descriptors: UnsafeCell<crate::fs_win::Descriptors>,
    directories: UnsafeCell<crate::fs_win::Directories>,
}

fn files(realm: &mut Realm) -> Arc<Files> {
    realm
        .native_files
        .get_or_insert_with(|| Arc::new(Files::new()))
        .clone()
}

pub(crate) fn is_tty(realm: &mut Realm, descriptor: i32) -> bool {
    files(realm).with(|files| files.is_tty(descriptor))
}
// SAFETY: all descriptor access is protected by the SRW lock; HANDLEs can be
// used on Windows worker threads. No source objects are stored in this state.
unsafe impl Send for Files {}
// SAFETY: the only descriptor accessor acquires the exclusive SRW lock.
unsafe impl Sync for Files {}

struct Unlock<'a>(&'a Files);
impl Drop for Unlock<'_> {
    fn drop(&mut self) {
        // SAFETY: this guard owns the lock on the current thread.
        unsafe { ReleaseSRWLockExclusive(self.0.lock.get()) };
    }
}
impl Files {
    fn new() -> Files {
        Files {
            lock: UnsafeCell::new(0),
            descriptors: UnsafeCell::new(crate::fs_win::Descriptors::default()),
            directories: UnsafeCell::new(crate::fs_win::Directories::default()),
        }
    }
    fn with<T: Send>(&self, operation: impl FnOnce(&mut crate::fs_win::Descriptors) -> T) -> T {
        // SAFETY: an all-zero SRWLOCK is initialized and lives for this call.
        unsafe { AcquireSRWLockExclusive(self.lock.get()) };
        let _unlock = Unlock(self);
        // SAFETY: the exclusive lock grants sole access until the guard drops.
        operation(unsafe { &mut *self.descriptors.get() })
    }

    fn with_directories<T: Send>(
        &self,
        operation: impl FnOnce(&mut crate::fs_win::Directories) -> T,
    ) -> T {
        // SAFETY: the same SRW lock protects descriptor and directory handles.
        unsafe { AcquireSRWLockExclusive(self.lock.get()) };
        let _unlock = Unlock(self);
        // SAFETY: the exclusive lock grants sole access until the guard drops.
        operation(unsafe { &mut *self.directories.get() })
    }
}

struct Statistics {
    numeric: Object,
    numeric_view: View,
    bigint: Object,
    bigint_view: View,
    fields: usize,
}
impl Statistics {
    fn new(realm: &Realm, fields: usize, shared_length: usize) -> Statistics {
        let numeric = typed_array_builtin::new_typed_array(realm, Element::Float64, shared_length);
        let bigint = typed_array_builtin::new_typed_array(realm, Element::BigInt64, shared_length);
        let numeric_view = typed_array_builtin::view_of(&Value::Object(numeric.clone())).unwrap();
        let bigint_view = typed_array_builtin::view_of(&Value::Object(bigint.clone())).unwrap();
        Statistics {
            numeric,
            numeric_view,
            bigint,
            bigint_view,
            fields,
        }
    }
    fn publish(&self, realm: &Realm, words: &[u64], bigint: bool, separate: bool) -> Value {
        let array = if separate {
            typed_array_builtin::new_typed_array(
                realm,
                if bigint {
                    Element::BigInt64
                } else {
                    Element::Float64
                },
                self.fields,
            )
        } else if bigint {
            self.bigint.clone()
        } else {
            self.numeric.clone()
        };
        let view = if separate {
            typed_array_builtin::view_of(&Value::Object(array.clone())).unwrap()
        } else if bigint {
            self.bigint_view.clone()
        } else {
            self.numeric_view.clone()
        };
        let mut bytes = view.bytes.borrow_mut();
        for (index, word) in words.iter().enumerate() {
            let raw = if bigint {
                word.to_le_bytes()
            } else {
                (*word as f64).to_le_bytes()
            };
            let start = view.offset + index * 8;
            bytes[start..start + 8].copy_from_slice(&raw);
        }
        drop(bytes);
        Value::Object(array)
    }
}

fn stat_words(stat: crate::fs_win::Stat) -> [u64; 18] {
    let mut fields = [
        stat.dev,
        u64::from(stat.mode),
        stat.nlink,
        0,
        0,
        stat.rdev,
        stat.blksize,
        stat.ino,
        stat.size,
        stat.blocks,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    for (index, (seconds, nanos)) in stat.times.into_iter().enumerate() {
        // Node 22's Windows ABI casts both parts through unsigned long (32-bit).
        fields[10 + index * 2] = u64::from(seconds as u32);
        fields[11 + index * 2] = u64::from(nanos as u32);
    }
    fields
}

fn statfs_words(stat: crate::fs_win::StatFs) -> [u64; 7] {
    [0, stat.bsize, stat.blocks, stat.bfree, stat.bfree, 0, 0]
}

#[derive(Clone)]
struct Context {
    syscall: &'static str,
    path: Option<JsStr>,
    destination: Option<JsStr>,
    reads: Vec<(typed_array_builtin::Bytes, usize, usize)>,
    encoding: Option<crate::encoding::Encoding>,
    buffer_names: bool,
    with_types: bool,
}

fn failure(realm: &Realm, context: &Context, errno: i32) -> Value {
    let (code, detail) = crate::uv_error::describe(errno);
    let mut message = JsStr::from(format!("{code}: {detail}, {}", context.syscall).as_str());
    for (index, path) in [&context.path, &context.destination]
        .into_iter()
        .enumerate()
    {
        if let Some(path) = path {
            message = message.concat(&JsStr::from(if index == 0 { " '" } else { " -> '" }));
            message = message.concat(path).concat(&JsStr::from("'"));
        }
    }
    // Native binding errors use intrinsic Error.prototype, not UVException or
    // a mutable uvErrmapGet row. Data definitions bypass prototype setters.
    let error = realm.error(ErrorKind::Error, "");
    error.define(
        Key::name("message"),
        Property::hidden(Value::String(message)),
    );
    error.define(
        Key::name("errno"),
        Property::data(Value::Number(f64::from(errno))),
    );
    error.define(Key::name("code"), Property::data(Value::str(&code)));
    error.define(
        Key::name("syscall"),
        Property::data(Value::str(context.syscall)),
    );
    for (key, path) in [("path", &context.path), ("dest", &context.destination)] {
        if let Some(path) = path {
            error.define(Key::name(key), Property::data(Value::String(path.clone())));
        }
    }
    Value::Object(error)
}

// The filesystem WTF-8 conversions below follow Node v22.15.0's
// deps/uv/src/idna.c. Copyright libuv contributors. All rights reserved.
// Permission to use, copy, modify, and/or distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
// ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
// OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
fn wtf8_bytes(units: &[u16]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for character in char::decode_utf16(units.iter().copied()) {
        match character {
            Ok(character) => {
                let mut encoded = [0; 4];
                bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            }
            Err(error) => {
                let unit = error.unpaired_surrogate();
                bytes.extend_from_slice(&[
                    0xe0 | (unit >> 12) as u8,
                    0x80 | ((unit >> 6) & 63) as u8,
                    0x80 | (unit & 63) as u8,
                ]);
            }
        }
    }
    bytes
}

/// libuv accepts surrogate code points and its pinned decoder also accepts
/// three/four-byte overlong sequences. Preserve that native path contract.
fn wtf8_units(bytes: &[u8]) -> Option<Vec<u16>> {
    let mut units = Vec::new();
    let mut index = 0;
    while let Some(&first) = bytes.get(index) {
        index += 1;
        if first <= 0x7f {
            units.push(u16::from(first));
            continue;
        }
        if first < 0xc2 {
            return None;
        }
        let length = if first <= 0xdf {
            2
        } else if first <= 0xef {
            3
        } else {
            4
        };
        let mut code = u32::from(first);
        for _ in 1..length {
            let continuation = *bytes.get(index)?;
            if continuation & 0xc0 != 0x80 {
                return None;
            }
            code = (code << 6) | u32::from(continuation & 63);
            index += 1;
        }
        code &= match length {
            2 => 0x7ff,
            3 => 0xffff,
            _ => 0x1fffff,
        };
        if first > 0xf4 || code > 0x10ffff {
            return None;
        }
        if code <= 0xffff {
            units.push(code as u16);
        } else {
            code -= 0x10000;
            units.extend_from_slice(&[0xd800 | (code >> 10) as u16, 0xdc00 | (code & 1023) as u16]);
        }
    }
    Some(units)
}

/// BufferValue's string conversion uses UTF-8, replacing unpaired UTF-16.
/// Buffer paths preserve WTF-8, as required for arbitrary Windows filenames.
/// Resolve once on the realm thread before a worker can observe a changed cwd.
fn path_units(realm: &Realm, value: &Value, syscall: &'static str) -> Result<Vec<u16>, Thrown> {
    let (units, invalid) = decoded_path(realm, value)?;
    if invalid {
        let lossy = Value::String(JsStr::from_units(&units));
        let (_, path) = validated_path(realm, &lossy, syscall)?;
        return Err(Thrown(failure(
            realm,
            &Context {
                syscall,
                path: Some(path),
                destination: None,
                reads: Vec::new(),
                encoding: None,
                buffer_names: false,
                with_types: false,
            },
            crate::fs_win::UV_ENOENT,
        )));
    }
    Ok(units)
}

fn decoded_path(realm: &Realm, value: &Value) -> Result<(Vec<u16>, bool), Thrown> {
    let mut invalid = false;
    let units = match value {
        Value::String(value) => crate::encoding::decode(
            &crate::encoding::encode(value.units(), crate::encoding::Encoding::Utf8),
            crate::encoding::Encoding::Utf8,
        ),
        value => {
            let Some((bytes, offset, length)) = typed_array_builtin::view_range(value) else {
                return Err(realm.throw(
                    ErrorKind::Type,
                    "Filesystem binding requires a string or buffer path",
                ));
            };
            let bytes = bytes.borrow()[offset..offset + length].to_vec();
            match wtf8_units(&bytes) {
                Some(units) => units,
                None => {
                    invalid = true;
                    crate::encoding::decode(&bytes, crate::encoding::Encoding::Utf8)
                }
            }
        }
    };
    if units.contains(&0) {
        return Err(realm.throw(ErrorKind::Type, "Filesystem path contains a null byte"));
    }
    Ok((units, invalid))
}

struct CapturedPath {
    units: Vec<u16>,
    text: JsStr,
    error: Option<i32>,
}

fn captured_path(
    realm: &Realm,
    value: &Value,
    syscall: &'static str,
    raw: bool,
) -> Result<CapturedPath, Thrown> {
    let (mut units, invalid) = decoded_path(realm, value)?;
    let (units, text) = if raw {
        let text = JsStr::from_units(&crate::encoding::decode(
            &wtf8_bytes(&units),
            crate::encoding::Encoding::Utf8,
        ));
        units.push(0);
        (units, text)
    } else {
        resolve_units(realm, units, syscall, GetFullPathNameW, GetLastError)?
    };
    Ok(CapturedPath {
        units,
        text,
        error: invalid.then_some(crate::fs_win::UV_ENOENT),
    })
}

/// A path made absolute against the working directory, null-terminated,
/// and as text.
pub(crate) fn full_path(realm: &Realm, value: &Value) -> Result<(Vec<u16>, JsStr), Thrown> {
    validated_path(realm, value, "mkdir")
}

fn validated_path(
    realm: &Realm,
    value: &Value,
    syscall: &'static str,
) -> Result<(Vec<u16>, JsStr), Thrown> {
    resolve_path(realm, value, syscall, GetFullPathNameW, GetLastError)
}

type FullPath = unsafe extern "system" fn(*const u16, u32, *mut u16, *mut *mut u16) -> u32;

fn resolve_path(
    realm: &Realm,
    value: &Value,
    syscall: &'static str,
    full_path: FullPath,
    last_error: unsafe extern "system" fn() -> u32,
) -> Result<(Vec<u16>, JsStr), Thrown> {
    resolve_units(
        realm,
        path_units(realm, value, syscall)?,
        syscall,
        full_path,
        last_error,
    )
}

fn resolve_units(
    realm: &Realm,
    mut units: Vec<u16>,
    syscall: &'static str,
    full_path: FullPath,
    last_error: unsafe extern "system" fn() -> u32,
) -> Result<(Vec<u16>, JsStr), Thrown> {
    if units.is_empty() {
        return Ok((vec![0], JsStr::from("")));
    }
    units.push(0);
    let mut absolute = vec![0; 512];
    loop {
        // SAFETY: input is terminated and the destination is writable for
        // the supplied length. lpFilePart is optional and remains null.
        let length = unsafe {
            full_path(
                units.as_ptr(),
                absolute.len() as u32,
                absolute.as_mut_ptr(),
                core::ptr::null_mut(),
            )
        };
        if length == 0 {
            // SAFETY: reads this thread's immediately preceding API failure.
            let errno = crate::uv_error::translate(unsafe { last_error() });
            let context = Context {
                syscall,
                path: Some(JsStr::from_units(&units[..units.len() - 1])),
                destination: None,
                reads: Vec::new(),
                encoding: None,
                buffer_names: false,
                with_types: false,
            };
            return Err(Thrown(failure(realm, &context, errno)));
        }
        if (length as usize) >= absolute.len() {
            absolute.resize(length as usize + 1, 0);
            continue;
        }
        absolute.truncate(length as usize);
        break;
    }
    if let Some(unpacked) = crate::fs_win::without_asar(&absolute) {
        absolute = unpacked;
    }
    let prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    let unc: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    let plain = if absolute.starts_with(&unc) {
        [vec![92, 92], absolute[unc.len()..].to_vec()].concat()
    } else if absolute.starts_with(&prefix) {
        absolute[prefix.len()..].to_vec()
    } else {
        absolute.clone()
    };
    // Node's error path is a UTF-8 String, while native I/O retains WTF-8 units.
    let text = JsStr::from_units(&crate::encoding::decode(
        &wtf8_bytes(&plain),
        crate::encoding::Encoding::Utf8,
    ));
    if !absolute.starts_with(&prefix) {
        absolute = if absolute.starts_with(&[92, 92]) {
            [unc, absolute[2..].to_vec()].concat()
        } else {
            [prefix, absolute].concat()
        };
    }
    absolute.push(0);
    Ok((absolute, text))
}

enum Output {
    Void,
    Number(f64),
    FileHandle(i32),
    DirectoryHandle(i32),
    DirectoryBatch(crate::fs_win::DirentBatch),
    DirectoryEof,
    Stat([u64; 18]),
    StatFs([u64; 7]),
    Text(Vec<u16>),
    Entries(Vec<(Vec<u16>, u8)>),
    Read { buffers: Vec<Vec<u8>>, count: usize },
}

enum Operation {
    CaptureError(i32),
    Transfer {
        fd: i32,
        buffers: Vec<Vec<u8>>,
        position: i64,
        read: bool,
    },
    Stat {
        path: Vec<u16>,
        link: bool,
    },
    Fstat(i32),
    StatFs(Vec<u16>),
    Access {
        path: Vec<u16>,
        mode: i32,
    },
    Open {
        path: Vec<u16>,
        flags: i32,
        mode: i32,
    },
    OpenFileHandle {
        path: Vec<u16>,
        flags: i32,
        mode: i32,
    },
    Close(i32),
    Rename {
        from: Vec<u16>,
        to: Vec<u16>,
    },
    Remove {
        path: Vec<u16>,
        directory: bool,
    },
    Mkdir {
        path: Vec<u16>,
        recursive: bool,
    },
    Readlink(Vec<u16>),
    Realpath(Vec<u16>),
    Scandir(Vec<u16>),
    OpenDirectory(Vec<u16>),
    ReadDirectory {
        id: i32,
        buffer_size: usize,
    },
    CloseDirectory(i32),
    Copy {
        from: Vec<u16>,
        to: Vec<u16>,
        flags: i32,
    },
    Link {
        from: Vec<u16>,
        to: Vec<u16>,
    },
    Symlink {
        target: Vec<u16>,
        path: Vec<u16>,
        flags: i32,
    },
    Fsync(i32),
    Truncate {
        fd: i32,
        length: i64,
    },
    Chmod {
        path: Vec<u16>,
        mode: i32,
    },
    Fchmod {
        fd: i32,
        mode: i32,
    },
    Times {
        path: Vec<u16>,
        access: f64,
        modified: f64,
        link: bool,
    },
    Ftimes {
        fd: i32,
        access: f64,
        modified: f64,
    },
    Mkdtemp(Vec<u16>),
}

impl Operation {
    /// The worker executor owns every argument and returns only Send data.
    /// Descriptor lookups and use are atomic with respect to native close.
    fn execute(self, files: &Files) -> Result<Output, i32> {
        use crate::fs_win as fs;
        match self {
            Operation::CaptureError(error) => Err(error),
            Operation::Transfer {
                fd,
                mut buffers,
                position,
                read,
            } => files.with(|files| {
                let handle = files.handle(fd)?;
                let mut slices: Vec<_> = buffers.iter_mut().map(Vec::as_mut_slice).collect();
                let count = if read {
                    fs::read(handle, &mut slices, position)?
                } else {
                    fs::write(handle, &mut slices, position)?
                };
                drop(slices);
                Ok(if read {
                    Output::Read { buffers, count }
                } else {
                    Output::Number(count as f64)
                })
            }),
            Operation::Stat { path, link } => {
                fs::stat(&path, link).map(|stat| Output::Stat(stat_words(stat)))
            }
            Operation::Fstat(fd) => files.with(|files| {
                fs::fstat(files.handle(fd)?).map(|stat| Output::Stat(stat_words(stat)))
            }),
            Operation::StatFs(path) => {
                fs::statfs(&path).map(|stat| Output::StatFs(statfs_words(stat)))
            }
            Operation::Access { path, mode } => fs::access(&path, mode).map(|()| Output::Void),
            Operation::Open { path, flags, mode } => files.with(|files| {
                files
                    .open(&path, flags, mode)
                    .map(|fd| Output::Number(f64::from(fd)))
            }),
            Operation::OpenFileHandle { path, flags, mode } => {
                files.with(|files| files.open(&path, flags, mode).map(Output::FileHandle))
            }
            Operation::Close(fd) => files.with(|files| files.close(fd).map(|()| Output::Void)),
            Operation::Rename { from, to } => fs::rename(&from, &to).map(|()| Output::Void),
            Operation::Remove { path, directory } => {
                fs::remove(&path, directory).map(|()| Output::Void)
            }
            Operation::Mkdir { path, recursive } => mkdir(&path, recursive),
            Operation::Readlink(path) => fs::readlink(&path).map(Output::Text),
            Operation::Realpath(path) => fs::realpath(&path).map(Output::Text),
            Operation::Scandir(path) => fs::scandir(&path).map(|mut entries| {
                entries.sort_by_cached_key(|(name, _)| wtf8_bytes(name));
                Output::Entries(entries)
            }),
            Operation::OpenDirectory(path) => files.with_directories(|directories| {
                directories.open(&path).map(Output::DirectoryHandle)
            }),
            Operation::ReadDirectory { id, buffer_size } => files.with_directories(|directories| {
                directories
                    .read(id, buffer_size)
                    .map(|entries| entries.map_or(Output::DirectoryEof, Output::DirectoryBatch))
            }),
            Operation::CloseDirectory(id) => {
                files.with_directories(|directories| directories.close(id).map(|()| Output::Void))
            }
            Operation::Copy { from, to, flags } => {
                fs::copyfile(&from, &to, flags).map(|()| Output::Void)
            }
            Operation::Link { from, to } => fs::link(&from, &to).map(|()| Output::Void),
            Operation::Symlink {
                target,
                path,
                flags,
            } => fs::symlink(&target, &path, flags).map(|()| Output::Void),
            Operation::Fsync(fd) => {
                files.with(|files| fs::fsync(files.handle(fd)?).map(|()| Output::Void))
            }
            Operation::Truncate { fd, length } => {
                files.with(|files| fs::ftruncate(files.handle(fd)?, length).map(|()| Output::Void))
            }
            Operation::Chmod { path, mode } => fs::chmod(&path, mode).map(|()| Output::Void),
            Operation::Fchmod { fd, mode } => {
                files.with(|files| fs::fchmod(files.handle(fd)?, mode).map(|()| Output::Void))
            }
            Operation::Times {
                path,
                access,
                modified,
                link,
            } => fs::utime(&path, access, modified, link).map(|()| Output::Void),
            Operation::Ftimes {
                fd,
                access,
                modified,
            } => files.with(|files| {
                fs::futime(files.handle(fd)?, access, modified).map(|()| Output::Void)
            }),
            Operation::Mkdtemp(mut path) => {
                fs::mkdtemp(&mut path)?;
                path.pop();
                Ok(Output::Text(path))
            }
        }
    }
}

/// Recursive creation returns the first created ancestor. Existing files do
/// not succeed as directories, including after another request races creation.
fn mkdir(path: &[u16], recursive: bool) -> Result<Output, i32> {
    mkdir_with(path, recursive, crate::fs_win::mkdir)
}

fn mkdir_with(
    path: &[u16],
    recursive: bool,
    create: fn(&[u16]) -> Result<(), i32>,
) -> Result<Output, i32> {
    use crate::fs_win as fs;
    const EACCES: i32 = -4092;
    const ENOSPC: i32 = -4055;
    const EEXIST: i32 = -4075;
    if !recursive {
        return create(path).map(|()| Output::Void);
    }
    let mut pending = vec![path.to_vec()];
    let mut first = None;
    while let Some(current) = pending.pop() {
        match create(&current) {
            Ok(()) => {
                first.get_or_insert_with(|| current[..current.len() - 1].to_vec());
            }
            Err(error) if matches!(error, EACCES | ENOSPC | fs::UV_ENOTDIR | fs::UV_EPERM) => {
                return Err(error);
            }
            Err(fs::UV_ENOENT) => {
                let mut end = current.len() - 1;
                while end > 0 && matches!(current[end - 1], 47 | 92) {
                    end -= 1;
                }
                let Some(split) = current[..end]
                    .iter()
                    .rposition(|unit| matches!(unit, 47 | 92))
                else {
                    return Err(fs::UV_ENOENT);
                };
                // Keep a drive root's separator, including a namespace prefix.
                let parent_end = if split > 0 && current[split - 1] == 58 {
                    split + 1
                } else {
                    split
                };
                // Trailing separators were removed before the search, so the
                // separator (including a retained drive-root one) is before
                // the final component. Every retry therefore shrinks the path.
                let mut parent = current[..parent_end].to_vec();
                parent.push(0);
                pending.push(current);
                pending.push(parent);
            }
            Err(error) => {
                let stat = fs::stat(&current, false)?;
                if stat.mode & fs::S_IFMT != fs::S_IFDIR {
                    return Err(if error == EEXIST && !pending.is_empty() {
                        fs::UV_ENOTDIR
                    } else {
                        EEXIST
                    });
                }
            }
        }
    }
    Ok(first.map_or(Output::Void, Output::Text))
}

struct Binding {
    files: Arc<Files>,
    stats: Statistics,
    statfs: Statistics,
    promise: Symbol,
    request: Symbol,
    handle_fd: Symbol,
    handle_close_promise: Symbol,
    directory_id: Symbol,
    next_async_id: AtomicU64,
}

#[derive(Clone)]
enum Delivery {
    Callback {
        request: Value,
        bigint: bool,
    },
    Promise {
        resolve: Value,
        reject: Value,
        bigint: bool,
    },
}

fn named_output(realm: &Realm, context: &Context, units: Vec<u16>) -> Value {
    let prefix: Vec<u16> = "\\\\?\\".encode_utf16().collect();
    let unc: Vec<u16> = "\\\\?\\UNC\\".encode_utf16().collect();
    let units =
        if matches!(context.syscall, "mkdir" | "realpath" | "mkdtemp") && units.starts_with(&unc) {
            [vec![92, 92], units[unc.len()..].to_vec()].concat()
        } else if matches!(context.syscall, "mkdir" | "realpath" | "mkdtemp")
            && units.starts_with(&prefix)
        {
            units[prefix.len()..].to_vec()
        } else {
            units
        };
    let bytes = wtf8_bytes(&units);
    if context.buffer_names {
        let buffer = typed_array_builtin::new_typed_array(realm, Element::Uint8, bytes.len());
        let view = typed_array_builtin::view_of(&Value::Object(buffer.clone())).unwrap();
        view.bytes.borrow_mut().copy_from_slice(&bytes);
        if let Some(prototype) = &realm.buffer_prototype {
            buffer.set_prototype(Some(prototype.clone()));
        }
        Value::Object(buffer)
    } else {
        Value::String(JsStr::from_units(&crate::encoding::decode(
            &bytes,
            context.encoding.unwrap_or(crate::encoding::Encoding::Utf8),
        )))
    }
}

fn format_output(
    state: &Rc<Binding>,
    realm: &Realm,
    context: &Context,
    result: Output,
    bigint: bool,
    separate: bool,
) -> Value {
    match result {
        Output::Void => Value::Undefined,
        Output::Number(value) => Value::Number(value),
        Output::FileHandle(fd) => native_file_handle(state, realm, fd),
        Output::DirectoryHandle(id) => Value::Object(native_directory_handle(state, realm, id)),
        Output::DirectoryBatch(entries) => {
            let mut values = Vec::with_capacity(entries.len() * 2);
            for (name, kind) in entries {
                values.push(named_output(realm, context, name));
                values.push(Value::Number(f64::from(kind)));
            }
            Value::Object(realm.array(values))
        }
        Output::DirectoryEof => Value::Null,
        Output::Stat(words) => state.stats.publish(realm, &words, bigint, separate),
        Output::StatFs(words) => state.statfs.publish(realm, &words, bigint, separate),
        Output::Read { buffers, count } => {
            let mut remaining = count;
            for ((bytes, offset, length), buffer) in context.reads.iter().zip(buffers) {
                let copied = remaining.min(*length);
                bytes.borrow_mut()[*offset..*offset + copied].copy_from_slice(&buffer[..copied]);
                remaining -= copied;
            }
            Value::Number(count as f64)
        }
        Output::Text(units) => named_output(realm, context, units),
        Output::Entries(entries) => {
            let (names, types): (Vec<_>, Vec<_>) = entries
                .into_iter()
                .map(|(name, kind)| {
                    (
                        named_output(realm, context, name),
                        Value::Number(f64::from(kind)),
                    )
                })
                .unzip();
            let names = Value::Object(realm.array(names));
            if context.with_types {
                Value::Object(realm.array(vec![names, Value::Object(realm.array(types))]))
            } else {
                names
            }
        }
    }
}

fn native_file_handle(state: &Rc<Binding>, realm: &Realm, fd: i32) -> Value {
    let object = realm.object();
    object.define(
        Key::name("fd"),
        Property {
            slot: crate::Slot::Data {
                value: Value::Number(f64::from(fd)),
                writable: true,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let id = state.next_async_id.fetch_add(1, Ordering::Relaxed);
    object.define(
        Key::name("getAsyncId"),
        Property::hidden(Value::Object(realm.function(
            "getAsyncId",
            0,
            false,
            move |_, _, _| Ok(Value::Number(id as f64)),
        ))),
    );
    let fd_key = state.handle_fd.clone();
    object.set_private(&fd_key, Property::data(Value::Number(f64::from(fd))));
    let close_key = state.handle_close_promise.clone();
    let close_fd_key = fd_key.clone();
    let close_state = state.clone();
    let close = realm.function("close", 0, false, move |realm, this, _| {
        let receiver = ops::to_object(realm, &this)?;
        if let Some(Property {
            slot: crate::Slot::Data { value, .. },
            ..
        }) = receiver.private_property(&close_key)
            && !matches!(value, Value::Undefined)
        {
            return Ok(value);
        }
        let Some(Property {
            slot:
                crate::Slot::Data {
                    value: Value::Number(descriptor),
                    ..
                },
            ..
        }) = receiver.private_property(&close_fd_key)
        else {
            return Err(realm.throw(ErrorKind::Type, "Invalid filesystem FileHandle"));
        };
        if descriptor < 0.0 {
            let (promise, resolve, _) = crate::promise_builtin::capability(realm);
            ops::call(realm, &resolve, Value::Undefined, &[Value::Undefined])?;
            return Ok(promise);
        }
        let descriptor = descriptor as i32;
        receiver.set_private(&close_fd_key, Property::data(Value::Number(-1.0)));
        ops::set(realm, &this, &Key::name("fd"), Value::Number(-1.0))?;
        let context = Context {
            syscall: "close",
            path: None,
            destination: None,
            reads: Vec::new(),
            encoding: None,
            buffer_names: false,
            with_types: false,
        };
        finish_file_handle_close(
            &receiver,
            &close_key,
            dispatch(
                &close_state,
                realm,
                context,
                Operation::Close(descriptor),
                Value::Symbol(close_state.promise.clone()),
                false,
                false,
            ),
        )
    });
    object.define(Key::name("close"), Property::hidden(Value::Object(close)));
    let release_key = fd_key;
    let release = realm.function("releaseFD", 0, false, move |realm, this, _| {
        let receiver = ops::to_object(realm, &this)?;
        let Some(Property {
            slot: crate::Slot::Data { value, .. },
            ..
        }) = receiver.private_property(&release_key)
        else {
            return Err(realm.throw(ErrorKind::Type, "Invalid filesystem FileHandle"));
        };
        receiver.set_private(&release_key, Property::data(Value::Number(-1.0)));
        ops::set(realm, &this, &Key::name("fd"), Value::Number(-1.0))?;
        Ok(value)
    });
    object.define(
        Key::name("releaseFD"),
        Property::hidden(Value::Object(release)),
    );
    Value::Object(object)
}

fn native_directory_handle(state: &Rc<Binding>, realm: &Realm, id: i32) -> Object {
    let object = realm.object();
    object.set_private(
        &state.directory_id,
        Property::data(Value::Number(f64::from(id))),
    );
    let async_id = state.next_async_id.fetch_add(1, Ordering::Relaxed);
    let get_async_id = realm.function("getAsyncId", 0, false, move |_, _, _| {
        Ok(Value::Number(async_id as f64))
    });
    object.define(
        Key::name("getAsyncId"),
        Property::hidden(Value::Object(get_async_id)),
    );

    let read_state = state.clone();
    let read_id = state.directory_id.clone();
    let read = realm.function("read", 3, false, move |realm, this, args| {
        let id = directory_handle_id(realm, &this, &read_id)?;
        let buffer_size = directory_buffer_size(realm, &arg(args, 1))?;
        let (encoding, buffer_names) = directory_encoding(realm, &arg(args, 0))?;
        let context = Context {
            syscall: "readdir",
            path: None,
            destination: None,
            reads: Vec::new(),
            encoding: Some(encoding),
            buffer_names,
            with_types: false,
        };
        dispatch_directory(
            &read_state,
            realm,
            context,
            Operation::ReadDirectory { id, buffer_size },
            arg(args, 2),
        )
    });
    object.define(Key::name("read"), Property::hidden(Value::Object(read)));

    let close_state = state.clone();
    let close_id = state.directory_id.clone();
    let close = realm.function("close", 1, false, move |realm, this, args| {
        let id = directory_handle_id(realm, &this, &close_id)?;
        let context = Context {
            syscall: "closedir",
            path: None,
            destination: None,
            reads: Vec::new(),
            encoding: None,
            buffer_names: false,
            with_types: false,
        };
        dispatch_directory(
            &close_state,
            realm,
            context,
            Operation::CloseDirectory(id),
            arg(args, 0),
        )
    });
    object.define(Key::name("close"), Property::hidden(Value::Object(close)));
    object
}

fn directory_handle_id(realm: &mut Realm, this: &Value, key: &Symbol) -> Result<i32, Thrown> {
    let object = ops::to_object(realm, this)?;
    let Some(Property {
        slot: crate::Slot::Data {
            value: Value::Number(id),
            ..
        },
        ..
    }) = object.private_property(key)
    else {
        return Err(realm.throw(ErrorKind::Type, "Invalid filesystem DirHandle"));
    };
    if id.is_finite() && crate::float::trunc(id) == id && id >= -1.0 && id <= f64::from(i32::MAX) {
        Ok(id as i32)
    } else {
        Err(realm.throw(ErrorKind::Type, "Invalid filesystem DirHandle"))
    }
}

fn directory_buffer_size(realm: &mut Realm, value: &Value) -> Result<usize, Thrown> {
    match value {
        Value::Number(size)
            if size.is_finite()
                && crate::float::trunc(*size) == *size
                && *size >= 1.0
                && *size <= f64::from(u32::MAX) =>
        {
            Ok(*size as usize)
        }
        Value::Number(size) => Err(crate::node_error::out_of_range(
            realm,
            "bufferSize",
            "an integer between 1 and 4294967295",
            &Value::Number(*size),
        )),
        value => Err(crate::node_error::invalid_arg_type(
            realm,
            "bufferSize",
            &["number"],
            value,
        )),
    }
}

fn directory_encoding(
    realm: &mut Realm,
    value: &Value,
) -> Result<(crate::encoding::Encoding, bool), Thrown> {
    if matches!(value, Value::Undefined) {
        return Ok((crate::encoding::Encoding::Utf8, false));
    }
    let name = ops::to_string(realm, value)?;
    if name.to_rust_lossy().eq_ignore_ascii_case("buffer") {
        return Ok((crate::encoding::Encoding::Utf8, true));
    }
    crate::encoding::named(name.units())
        .map(|encoding| (encoding, false))
        .ok_or_else(|| realm.throw(ErrorKind::Type, "Unknown filesystem directory encoding"))
}

fn finish_file_handle_close(
    receiver: &Object,
    close_key: &Symbol,
    result: Completion,
) -> Completion {
    match result {
        Ok(promise) => {
            // Keep the close result on the native object for repeated calls.
            receiver.set_private(close_key, Property::data(promise.clone()));
            Ok(promise)
        }
        Err(error) => Err(error),
    }
}

fn complete(
    state: &Rc<Binding>,
    realm: &mut Realm,
    context: &Context,
    delivery: Delivery,
    result: Result<Output, i32>,
) -> Result<(), Thrown> {
    match delivery {
        Delivery::Callback { request, bigint } => {
            let args = match result {
                Ok(output) => {
                    let value = format_output(state, realm, context, output, bigint, false);
                    if matches!(value, Value::Undefined) {
                        vec![Value::Null]
                    } else {
                        vec![Value::Null, value]
                    }
                }
                Err(errno) => vec![failure(realm, context, errno)],
            };
            let callback = ops::get_named(realm, &request, "oncomplete")?;
            ops::call(realm, &callback, request, &args).map(drop)
        }
        Delivery::Promise {
            resolve,
            reject,
            bigint,
        } => {
            let (function, value) = match result {
                Ok(output) => (
                    resolve,
                    format_output(state, realm, context, output, bigint, true),
                ),
                Err(errno) => (reject, failure(realm, context, errno)),
            };
            ops::call(realm, &function, Value::Undefined, &[value]).map(drop)
        }
    }
}

fn dispatch(
    state: &Rc<Binding>,
    realm: &mut Realm,
    context: Context,
    operation: Operation,
    request: Value,
    bigint: bool,
    suppress_missing: bool,
) -> Completion {
    if matches!(request, Value::Undefined) {
        return match operation.execute(&state.files) {
            Ok(result) => Ok(format_output(state, realm, &context, result, bigint, false)),
            Err(errno)
                if suppress_missing
                    && (errno == crate::fs_win::UV_ENOENT
                        || (context.syscall == "stat" && errno == crate::fs_win::UV_ENOTDIR)) =>
            {
                Ok(Value::Undefined)
            }
            Err(errno) => Err(Thrown(failure(realm, &context, errno))),
        };
    }
    let (result, delivery) = if matches!(&request,Value::Symbol(symbol) if *symbol==state.promise) {
        let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
        (
            promise,
            Delivery::Promise {
                resolve,
                reject,
                bigint,
            },
        )
    } else {
        let Some(object) = request.as_object() else {
            return Err(realm.throw(ErrorKind::Type, "Invalid filesystem request"));
        };
        let Some(Property {
            slot:
                crate::Slot::Data {
                    value: Value::Boolean(bigint),
                    ..
                },
            ..
        }) = object.private_property(&state.request)
        else {
            return Err(realm.throw(ErrorKind::Type, "Invalid filesystem request"));
        };
        (Value::Undefined, Delivery::Callback { request, bigint })
    };
    if let Operation::CaptureError(error) = operation {
        // Node dispatch rejects capture errors before queuing work. Its
        // callback runs immediately; a private promise request is rejected
        // without returning that promise. req.path is absent in this phase.
        let context = Context {
            path: None,
            ..context
        };
        complete(state, realm, &context, delivery, Err(error))?;
        return Ok(Value::Undefined);
    }
    let files = state.files.clone();
    let finished = state.clone();
    let finished_context = context.clone();
    let finished_delivery = delivery.clone();
    if let Err(error) = realm.submit_io(
        move || operation.execute(&files),
        move |realm, result| {
            complete(
                &finished,
                realm,
                &finished_context,
                finished_delivery,
                result,
            )
        },
    ) {
        complete(
            state,
            realm,
            &context,
            delivery,
            Err(crate::uv_error::translate(error)),
        )?;
    }
    Ok(result)
}

fn dispatch_directory(
    state: &Rc<Binding>,
    realm: &mut Realm,
    context: Context,
    operation: Operation,
    request: Value,
) -> Completion {
    if matches!(request, Value::Undefined) {
        return match operation.execute(&state.files) {
            Ok(output) => Ok(format_output(state, realm, &context, output, false, false)),
            Err(errno) => Err(Thrown(failure(realm, &context, errno))),
        };
    }
    let Some(_) = request.as_object() else {
        return Err(realm.throw(ErrorKind::Type, "Invalid filesystem request"));
    };
    let operation = match operation {
        Operation::CaptureError(errno) => {
            let callback = ops::get_named(realm, &request, "oncomplete")?;
            let context = Context {
                path: None,
                ..context
            };
            let error = failure(realm, &context, errno);
            ops::call(realm, &callback, request, &[error]).map(drop)?;
            return Ok(Value::Undefined);
        }
        operation => operation,
    };
    let files = state.files.clone();
    let completed_state = state.clone();
    let completed_context = context.clone();
    let completed_request = request.clone();
    if let Err(error) = realm.submit_io(
        move || operation.execute(&files),
        move |realm, result| {
            let args = match result {
                Ok(output) => {
                    let value = format_output(
                        &completed_state,
                        realm,
                        &completed_context,
                        output,
                        false,
                        false,
                    );
                    if matches!(value, Value::Undefined) {
                        vec![Value::Null]
                    } else {
                        vec![Value::Null, value]
                    }
                }
                Err(errno) => vec![failure(realm, &completed_context, errno)],
            };
            let callback = ops::get_named(realm, &completed_request, "oncomplete")?;
            ops::call(realm, &callback, completed_request, &args).map(drop)
        },
    ) {
        let callback = ops::get_named(realm, &request, "oncomplete")?;
        let error = failure(realm, &context, crate::uv_error::translate(error));
        ops::call(realm, &callback, request, &[error]).map(drop)?;
    }
    Ok(Value::Undefined)
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

fn fd(realm: &mut Realm, value: &Value) -> Result<i32, Thrown> {
    match value {
        Value::Number(value)
            if value.is_finite()
                && *value >= 0.0
                && *value <= f64::from(i32::MAX)
                && crate::float::trunc(*value) == *value =>
        {
            Ok(*value as i32)
        }
        Value::Number(value) => Err(crate::node_error::out_of_range(
            realm,
            "fd",
            "an integer >= 0 and <= 2147483647",
            &Value::Number(*value),
        )),
        value => Err(crate::node_error::invalid_arg_type(
            realm,
            "fd",
            &["number"],
            value,
        )),
    }
}

/// Fast UTF-8 file operations own temporary descriptors and retain the caller's
/// descriptor/offset when given a number. All kernel operations are performed
/// while the descriptor lock is held; no source callback runs under it.
fn utf8_file(state: &Binding, realm: &mut Realm, args: &[Value], write: bool) -> Completion {
    utf8_file_with(state, realm, args, write, crate::fs_win::write)
}

type Write = fn(crate::fs_win::Handle, &mut [&mut [u8]], i64) -> Result<usize, i32>;

fn utf8_file_with(
    state: &Binding,
    realm: &mut Realm,
    args: &[Value],
    write: bool,
    write_file: Write,
) -> Completion {
    let numeric = matches!(args.first(), Some(Value::Number(_)));
    let mut context = Context {
        syscall: "open",
        path: None,
        destination: None,
        reads: Vec::new(),
        encoding: None,
        buffer_names: false,
        with_types: false,
    };
    let input = if numeric {
        None
    } else {
        let (path, text) = validated_path(realm, &arg(args, 0), "open")?;
        context.path = Some(text);
        Some(path)
    };
    let descriptor = if numeric {
        Some(fd(realm, &arg(args, 0))?)
    } else {
        None
    };
    let flag_index = if write { 2 } else { 1 };
    let flags = ops::wrap32(ops::to_number(realm, &arg(args, flag_index))?) as i32;
    let mode = if write {
        ops::wrap32(ops::to_number(realm, &arg(args, 3))?) as i32
    } else {
        0o666
    };
    let mut data = if write {
        match arg(args, 1) {
            Value::String(text) => {
                crate::encoding::encode(text.units(), crate::encoding::Encoding::Utf8)
            }
            _ => return Err(realm.throw(ErrorKind::Type, "UTF-8 write requires a string")),
        }
    } else {
        Vec::new()
    };
    let result: Result<(), (&'static str, i32)> = state.files.with(|files| {
        let fd = match descriptor {
            Some(fd) => fd,
            None => files
                .open(input.as_ref().unwrap(), flags, mode)
                .map_err(|error| ("open", error))?,
        };
        let transfer = (|| {
            let handle = files
                .handle(fd)
                .map_err(|error| (if write { "write" } else { "read" }, error))?;
            if write {
                let mut offset = 0;
                while offset < data.len() {
                    let written = write_file(handle, &mut [&mut data[offset..]], -1)
                        .map_err(|error| ("write", error))?;
                    if written == 0 {
                        return Err(("write", crate::fs_win::UV_EIO));
                    }
                    offset += written;
                }
            } else {
                let mut buffer = [0; 8192];
                loop {
                    let read = crate::fs_win::read(handle, &mut [&mut buffer], -1)
                        .map_err(|error| ("read", error))?;
                    if read == 0 {
                        break;
                    }
                    data.extend_from_slice(&buffer[..read]);
                }
            }
            Ok(())
        })();
        if !numeric {
            let _ = files.close(fd);
        }
        transfer
    });
    if let Err((syscall, errno)) = result {
        context.syscall = syscall;
        if syscall != "open" {
            context.path = None;
        }
        return Err(Thrown(failure(realm, &context, errno)));
    }
    if write {
        Ok(Value::Undefined)
    } else {
        Ok(Value::String(JsStr::from_units(&crate::encoding::decode(
            &data,
            crate::encoding::Encoding::Utf8,
        ))))
    }
}

fn transfer(
    state: &Rc<Binding>,
    realm: &mut Realm,
    args: &[Value],
    read: bool,
    vector: bool,
    string: bool,
) -> Completion {
    let descriptor = fd(realm, &arg(args, 0))?;
    let mut context = Context {
        syscall: if read { "read" } else { "write" },
        path: None,
        destination: None,
        reads: Vec::new(),
        encoding: None,
        buffer_names: false,
        with_types: false,
    };
    let mut buffers = Vec::new();
    let (position_index, request_index) = if string {
        let text = ops::to_string(realm, &arg(args, 1))?;
        let encoding = ops::to_string(realm, &arg(args, 3))?;
        let encoding = crate::encoding::named(encoding.units())
            .ok_or_else(|| realm.throw(ErrorKind::Type, "Unknown write encoding"))?;
        buffers.push(crate::encoding::encode(text.units(), encoding));
        (2, 4)
    } else {
        let values = if vector {
            crate::iter::collect(realm, &arg(args, 1))?
        } else {
            vec![arg(args, 1)]
        };
        for value in values {
            let Some((bytes, mut offset, mut length)) = typed_array_builtin::view_range(&value)
            else {
                return Err(realm.throw(ErrorKind::Type, "Filesystem transfer requires buffers"));
            };
            if !vector {
                let start = ops::to_number(realm, &arg(args, 2))?;
                let count = ops::to_number(realm, &arg(args, 3))?;
                if !start.is_finite()
                    || !count.is_finite()
                    || start < 0.0
                    || count < 0.0
                    || crate::float::trunc(start) != start
                    || crate::float::trunc(count) != count
                    || start > length as f64
                    || count > length as f64 - start
                {
                    return Err(realm.throw(
                        ErrorKind::Range,
                        "Filesystem transfer is outside the buffer",
                    ));
                }
                offset += start as usize;
                length = count as usize;
            }
            buffers.push(if read {
                vec![0; length]
            } else {
                bytes.borrow()[offset..offset + length].to_vec()
            });
            if read {
                context.reads.push((bytes, offset, length));
            }
        }
        if vector { (2, 3) } else { (4, 5) }
    };
    let position = match arg(args, position_index) {
        Value::Null | Value::Undefined => -1,
        Value::BigInt(value) if read && !vector => {
            value.as_uint_n(64).unwrap().magnitude_u64().unwrap() as i64
        }
        Value::Number(value)
            if value.is_finite()
                && crate::float::trunc(value) == value
                && value.abs() <= 9_007_199_254_740_991.0 =>
        {
            value as i64
        }
        _ => -1,
    };
    let request = arg(args, request_index);
    let operation = Operation::Transfer {
        fd: descriptor,
        buffers,
        position,
        read,
    };
    if !read && !vector && matches!(request, Value::Undefined) {
        return match operation.execute(&state.files) {
            Ok(result) => Ok(format_output(state, realm, &context, result, false, false)),
            Err(error) => {
                let context = arg(args, request_index + 1);
                ops::set(
                    realm,
                    &context,
                    &Key::name("errno"),
                    Value::Number(f64::from(error)),
                )?;
                ops::set(realm, &context, &Key::name("syscall"), Value::str("write"))?;
                Ok(Value::Number(f64::from(error)))
            }
        };
    }
    dispatch(state, realm, context, operation, request, false, false)
}

pub(crate) fn binding(realm: &mut Realm) -> Object {
    let state = Rc::new(Binding {
        files: files(realm),
        stats: Statistics::new(realm, 18, 36),
        statfs: Statistics::new(realm, 7, 7),
        promise: Symbol::new(Some(JsStr::from("fs_use_promises_symbol"))),
        request: Symbol::private("nativeboat.fs.request"),
        handle_fd: Symbol::private("nativeboat.fs.handle.fd"),
        handle_close_promise: Symbol::private("nativeboat.fs.handle.closePromise"),
        directory_id: Symbol::private("nativeboat.fs.directory.id"),
        next_async_id: AtomicU64::new(1),
    });
    let exports = realm.object();
    let watcher = stat_watcher::constructor(realm, state.clone());
    exports.define(
        Key::name("StatWatcher"),
        Property::data(Value::Object(watcher)),
    );
    for (name, value) in [
        ("statValues", Value::Object(state.stats.numeric.clone())),
        (
            "bigintStatValues",
            Value::Object(state.stats.bigint.clone()),
        ),
        ("statFsValues", Value::Object(state.statfs.numeric.clone())),
        (
            "bigintStatFsValues",
            Value::Object(state.statfs.bigint.clone()),
        ),
        ("kFsStatsFieldsNumber", Value::Number(18.0)),
        ("kUsePromises", Value::Symbol(state.promise.clone())),
    ] {
        exports.define(Key::name(name), Property::data(value));
    }
    let key = state.request.clone();
    let constructor = realm.function("FSReqCallback", 0, true, move |realm, this, args| {
        if matches!(realm.new_target(), Value::Undefined) {
            return Err(realm.throw(ErrorKind::Type, "FSReqCallback requires construction"));
        }
        let object = ops::to_object(realm, &this)?;
        object.set_private(
            &key,
            Property::data(Value::Boolean(matches!(
                args.first(),
                Some(Value::Boolean(true))
            ))),
        );
        Ok(Value::Undefined)
    });
    exports.define(
        Key::name("FSReqCallback"),
        Property::data(Value::Object(constructor)),
    );
    for (name, link) in [("stat", false), ("lstat", true)] {
        let state = state.clone();
        let function = realm.function(name, 0, false, move |realm, _, args| {
            let captured = captured_path(realm, &arg(args, 0), name, false)?;
            let path = captured.units;
            let text = captured.text;
            let context = Context {
                syscall: name,
                path: Some(text),
                destination: None,
                reads: Vec::new(),
                encoding: None,
                buffer_names: false,
                with_types: false,
            };
            dispatch(
                &state,
                realm,
                context,
                captured
                    .error
                    .map_or(Operation::Stat { path, link }, Operation::CaptureError),
                arg(args, 2),
                matches!(args.get(1), Some(Value::Boolean(true))),
                matches!(args.get(3), Some(Value::Boolean(false))),
            )
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let stats = state.clone();
    let function = realm.function("fstat", 0, false, move |realm, _, args| {
        let descriptor = fd(realm, &arg(args, 0))?;
        dispatch(
            &stats,
            realm,
            Context {
                syscall: "fstat",
                path: None,
                destination: None,
                reads: Vec::new(),
                encoding: None,
                buffer_names: false,
                with_types: false,
            },
            Operation::Fstat(descriptor),
            arg(args, 2),
            matches!(args.get(1), Some(Value::Boolean(true))),
            false,
        )
    });
    exports.define(Key::name("fstat"), Property::data(Value::Object(function)));
    let stats = state.clone();
    let function = realm.function("statfs", 0, false, move |realm, _, args| {
        let captured = captured_path(realm, &arg(args, 0), "statfs", false)?;
        let path = captured.units;
        let text = captured.text;
        dispatch(
            &stats,
            realm,
            Context {
                syscall: "statfs",
                path: Some(text),
                destination: None,
                reads: Vec::new(),
                encoding: None,
                buffer_names: false,
                with_types: false,
            },
            captured
                .error
                .map_or(Operation::StatFs(path), Operation::CaptureError),
            arg(args, 2),
            matches!(args.get(1), Some(Value::Boolean(true))),
            false,
        )
    });
    exports.define(Key::name("statfs"), Property::data(Value::Object(function)));
    for name in [
        "access",
        "open",
        "openFileHandle",
        "close",
        "rename",
        "unlink",
        "rmdir",
        "readlink",
        "realpath",
        "readdir",
        "copyFile",
        "link",
        "symlink",
        "fsync",
        "fdatasync",
        "ftruncate",
        "chmod",
        "fchmod",
        "utimes",
        "futimes",
        "lutimes",
        "mkdtemp",
    ] {
        let state = state.clone();
        let function = realm.function(name, 0, false, move |realm, _, args| {
            let mut context = Context {
                syscall: if name == "openFileHandle" {
                    "open"
                } else {
                    name
                },
                path: None,
                destination: None,
                reads: Vec::new(),
                encoding: None,
                buffer_names: false,
                with_types: false,
            };
            let mut capture_error = None;
            if matches!(name, "readlink" | "realpath" | "readdir" | "mkdtemp") {
                match arg(args, 1) {
                    Value::Undefined => {}
                    Value::String(encoding)
                        if encoding.units() == JsStr::from("buffer").units() =>
                    {
                        context.buffer_names = true
                    }
                    Value::String(encoding) => {
                        context.encoding =
                            Some(crate::encoding::named(encoding.units()).ok_or_else(|| {
                                realm.throw(ErrorKind::Type, "Unknown filesystem encoding")
                            })?)
                    }
                    _ => {
                        return Err(
                            realm.throw(ErrorKind::Type, "Filesystem encoding requires a string")
                        );
                    }
                }
                context.with_types = matches!(args.get(2), Some(Value::Boolean(true)));
            }
            let (operation, request) = match name {
                "close" => (Operation::Close(fd(realm, &arg(args, 0))?), arg(args, 1)),
                "fsync" | "fdatasync" => {
                    (Operation::Fsync(fd(realm, &arg(args, 0))?), arg(args, 1))
                }
                "ftruncate" => (
                    Operation::Truncate {
                        fd: fd(realm, &arg(args, 0))?,
                        length: ops::to_number(realm, &arg(args, 1))? as i64,
                    },
                    arg(args, 2),
                ),
                "fchmod" => (
                    Operation::Fchmod {
                        fd: fd(realm, &arg(args, 0))?,
                        mode: ops::wrap32(ops::to_number(realm, &arg(args, 1))?) as i32,
                    },
                    arg(args, 2),
                ),
                "futimes" => (
                    Operation::Ftimes {
                        fd: fd(realm, &arg(args, 0))?,
                        access: ops::to_number(realm, &arg(args, 1))?,
                        modified: ops::to_number(realm, &arg(args, 2))?,
                    },
                    arg(args, 3),
                ),
                name => {
                    let captured = captured_path(realm, &arg(args, 0), name, name == "symlink")?;
                    capture_error = captured.error;
                    let path = captured.units;
                    let text = captured.text;
                    context.path = Some(text);
                    match name {
                        "access" => (
                            Operation::Access {
                                path,
                                mode: ops::wrap32(ops::to_number(realm, &arg(args, 1))?) as i32,
                            },
                            arg(args, 2),
                        ),
                        "open" | "openFileHandle" => (
                            if name == "openFileHandle" {
                                Operation::OpenFileHandle {
                                    path,
                                    flags: ops::wrap32(ops::to_number(realm, &arg(args, 1))?)
                                        as i32,
                                    mode: ops::wrap32(ops::to_number(realm, &arg(args, 2))?) as i32,
                                }
                            } else {
                                Operation::Open {
                                    path,
                                    flags: ops::wrap32(ops::to_number(realm, &arg(args, 1))?)
                                        as i32,
                                    mode: ops::wrap32(ops::to_number(realm, &arg(args, 2))?) as i32,
                                }
                            },
                            arg(args, 3),
                        ),
                        "unlink" | "rmdir" => (
                            Operation::Remove {
                                path,
                                directory: name == "rmdir",
                            },
                            arg(args, 1),
                        ),
                        "readlink" => (Operation::Readlink(path), arg(args, 2)),
                        "realpath" => (Operation::Realpath(path), arg(args, 2)),
                        "readdir" => (Operation::Scandir(path), arg(args, 3)),
                        "mkdtemp" => {
                            // Node's binding gives libuv a template: the
                            // prefix and six characters to replace.
                            let mut template = path;
                            let terminator = template.pop();
                            template.extend("XXXXXX".encode_utf16());
                            template.extend(terminator);
                            (Operation::Mkdtemp(template), arg(args, 2))
                        }
                        "chmod" => (
                            Operation::Chmod {
                                path,
                                mode: ops::wrap32(ops::to_number(realm, &arg(args, 1))?) as i32,
                            },
                            arg(args, 2),
                        ),
                        "utimes" | "lutimes" => (
                            Operation::Times {
                                path,
                                access: ops::to_number(realm, &arg(args, 1))?,
                                modified: ops::to_number(realm, &arg(args, 2))?,
                                link: name == "lutimes",
                            },
                            arg(args, 3),
                        ),
                        name => {
                            let captured = captured_path(realm, &arg(args, 1), name, false)?;
                            capture_error = capture_error.or(captured.error);
                            let to = captured.units;
                            let text = captured.text;
                            context.destination = Some(text);
                            match name {
                                "rename" => (Operation::Rename { from: path, to }, arg(args, 2)),
                                "link" => (Operation::Link { from: path, to }, arg(args, 2)),
                                "copyFile" => (
                                    Operation::Copy {
                                        from: path,
                                        to,
                                        flags: ops::wrap32(ops::to_number(realm, &arg(args, 2))?)
                                            as i32,
                                    },
                                    arg(args, 3),
                                ),
                                _ => (
                                    Operation::Symlink {
                                        target: path,
                                        path: to,
                                        flags: ops::wrap32(ops::to_number(realm, &arg(args, 2))?)
                                            as i32,
                                    },
                                    arg(args, 3),
                                ),
                            }
                        }
                    }
                }
            };
            let operation = capture_error.map_or(operation, Operation::CaptureError);
            dispatch(&state, realm, context, operation, request, false, false)
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    for (name, read, vector, string) in [
        ("read", true, false, false),
        ("readBuffers", true, true, false),
        ("writeBuffer", false, false, false),
        ("writeBuffers", false, true, false),
        ("writeString", false, false, true),
    ] {
        let state = state.clone();
        let function = realm.function(name, 0, false, move |realm, _, args| {
            transfer(&state, realm, args, read, vector, string)
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    for (name, write) in [("readFileUtf8", false), ("writeFileUtf8", true)] {
        let state = state.clone();
        let function = realm.function(name, 0, false, move |realm, _, args| {
            utf8_file(&state, realm, args, write)
        });
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    let exists = realm.function("existsSync", 0, false, move |realm, _, args| {
        let captured = captured_path(realm, &arg(args, 0), "stat", false)?;
        Ok(Value::Boolean(
            captured.error.is_none() && crate::fs_win::stat(&captured.units, false).is_ok(),
        ))
    });
    exports.define(
        Key::name("existsSync"),
        Property::data(Value::Object(exists)),
    );
    // `internalModuleStat(binding, path)`: 1 for a directory, 0 for another
    // file, or a negative libuv error, as recursive `readdir` asks.
    let module_stat = realm.function("internalModuleStat", 0, false, |realm, _, args| {
        let captured = captured_path(realm, &arg(args, 1), "stat", false)?;
        if captured.error.is_some() {
            return Ok(Value::Number(f64::from(crate::fs_win::UV_ENOENT)));
        }
        Ok(Value::Number(
            match crate::fs_win::stat(&captured.units, false) {
                Ok(stat) => f64::from(u8::from(stat.mode & 0o170_000 == 0o040_000)),
                Err(error) => f64::from(error),
            },
        ))
    });
    exports.define(
        Key::name("internalModuleStat"),
        Property::data(Value::Object(module_stat)),
    );
    let mkdir_state = state.clone();
    let mkdir = realm.function("mkdir", 0, false, move |realm, _, args| {
        let captured = captured_path(realm, &arg(args, 0), "mkdir", false)?;
        let path = captured.units;
        let text = captured.text;
        dispatch(
            &mkdir_state,
            realm,
            Context {
                syscall: "mkdir",
                path: Some(text),
                destination: None,
                reads: Vec::new(),
                encoding: None,
                buffer_names: false,
                with_types: false,
            },
            captured.error.map_or(
                Operation::Mkdir {
                    path,
                    recursive: matches!(args.get(2), Some(Value::Boolean(true))),
                },
                Operation::CaptureError,
            ),
            arg(args, 3),
            false,
            false,
        )
    });
    exports.define(Key::name("mkdir"), Property::data(Value::Object(mkdir)));
    exports
}

pub(crate) fn directory_binding(realm: &mut Realm) -> Object {
    let state = Rc::new(Binding {
        files: files(realm),
        stats: Statistics::new(realm, 18, 36),
        statfs: Statistics::new(realm, 7, 7),
        promise: Symbol::new(Some(JsStr::from("fs_use_promises_symbol"))),
        request: Symbol::private("nativeboat.fs.request"),
        handle_fd: Symbol::private("nativeboat.fs.handle.fd"),
        handle_close_promise: Symbol::private("nativeboat.fs.handle.closePromise"),
        directory_id: Symbol::private("nativeboat.fs.directory.id"),
        next_async_id: AtomicU64::new(1),
    });
    let exports = realm.object();

    let open_state = state.clone();
    let open = realm.function("opendir", 3, false, move |realm, _, args| {
        open_directory(&open_state, realm, args, 2)
    });
    exports.define(Key::name("opendir"), Property::data(Value::Object(open)));

    let sync_state = state.clone();
    let open_sync = realm.function("opendirSync", 2, false, move |realm, _, args| {
        open_directory(&sync_state, realm, args, usize::MAX)
    });
    exports.define(
        Key::name("opendirSync"),
        Property::data(Value::Object(open_sync)),
    );

    let constructor_state = state;
    let constructor = realm.function("DirHandle", 0, true, move |realm, _, _| {
        if matches!(realm.new_target(), Value::Undefined) {
            return Err(realm.throw(ErrorKind::Type, "DirHandle requires construction"));
        }
        Ok(Value::Object(native_directory_handle(
            &constructor_state,
            realm,
            -1,
        )))
    });
    exports.define(
        Key::name("DirHandle"),
        Property::data(Value::Object(constructor)),
    );
    exports
}

fn open_directory(
    state: &Rc<Binding>,
    realm: &mut Realm,
    args: &[Value],
    request_index: usize,
) -> Completion {
    let captured = captured_path(realm, &arg(args, 0), "opendir", false)?;
    let (encoding, _) = directory_encoding(realm, &arg(args, 1))?;
    let context = Context {
        syscall: "opendir",
        path: Some(captured.text),
        destination: None,
        reads: Vec::new(),
        encoding: Some(encoding),
        buffer_names: false,
        with_types: false,
    };
    let operation = captured.error.map_or(
        Operation::OpenDirectory(captured.units),
        Operation::CaptureError,
    );
    dispatch_directory(
        state,
        realm,
        context,
        operation,
        if request_index == usize::MAX {
            Value::Undefined
        } else {
            arg(args, request_index)
        },
    )
}

#[cfg(test)]
mod tests;
