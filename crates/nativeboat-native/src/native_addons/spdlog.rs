//! `@vscode/spdlog`'s binding (`spdlog.node`), as spdlog 1.12 writes on
//! Windows: `new Logger('rotating' | 'rotating_async', name, file, maxSize,
//! maxFiles)` appends lines to a rotating file, formatted by spdlog's
//! pattern flags and ended by `\r\n`; past `maxSize` bytes the file moves to
//! `name.1.ext`, `name.1.ext` to `name.2.ext` and so on up to `maxFiles`.
//! Loggers are kept by name, as spdlog's registry keeps them: a second
//! logger of a name writes through the first. Lines are written when
//! logged, so flushing has nothing left to write. Other logger types write
//! to standard output.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::ffi::c_void;

/// spdlog 1.12.0, as `SPDLOG_VERSION` encodes it.
const VERSION: f64 = 11200.0;
const LEVELS: [&str; 7] = [
    "trace", "debug", "info", "warning", "error", "critical", "off",
];
const SHORT_LEVELS: [&str; 7] = ["T", "D", "I", "W", "E", "C", "O"];
const DEFAULT_PATTERN: &str = "[%Y-%m-%d %H:%M:%S.%e] [%n] [%l] %v";
const INFO: u8 = 2;
const OFF: u8 = 6;

const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_APPEND_DATA: u32 = 0x0004;
const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_WRITE: u32 = 0x2;
const FILE_SHARE_DELETE: u32 = 0x4;
const OPEN_ALWAYS: u32 = 4;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;

#[repr(C)]
#[derive(Default)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    milliseconds: u16,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
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
        buffer: *const u8,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn GetFileSizeEx(file: *mut c_void, size: *mut i64) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    fn DeleteFileW(name: *const u16) -> i32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn GetLocalTime(time: *mut SystemTime);
    fn GetCurrentThreadId() -> u32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(core::iter::once(0)).collect()
}

fn exists(path: &str) -> bool {
    let path = wide(path);
    // SAFETY: the path is nul-terminated.
    unsafe { GetFileAttributesW(path.as_ptr()) != u32::MAX }
}

/// spdlog's name of rotated file `index`: the index before the extension,
/// unless the name is a hidden file's or has none.
fn rotated_name(base: &str, index: usize) -> String {
    if index == 0 {
        return base.into();
    }
    let file_start = base.rfind(['\\', '/']).map_or(0, |at| at + 1);
    match base.rfind('.') {
        Some(dot) if dot > file_start => {
            alloc::format!("{}.{index}{}", &base[..dot], &base[dot..])
        }
        _ => alloc::format!("{base}.{index}"),
    }
}

/// The rotating file a logger writes.
struct Sink {
    path: String,
    max_size: u64,
    max_files: usize,
    file: Cell<*mut c_void>,
    size: Cell<u64>,
}

impl Sink {
    fn open(&self) -> bool {
        let path = wide(&self.path);
        // SAFETY: the path is nul-terminated; an invalid handle is checked.
        let file = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_WRITE | FILE_APPEND_DATA,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                core::ptr::null_mut(),
                OPEN_ALWAYS,
                FILE_ATTRIBUTE_NORMAL,
                core::ptr::null_mut(),
            )
        };
        if file as isize == -1 {
            return false;
        }
        let mut size = 0;
        // SAFETY: the file is open and the size is writable.
        unsafe { GetFileSizeEx(file, &mut size) };
        self.file.set(file);
        self.size.set(size.max(0) as u64);
        true
    }

    fn close(&self) {
        let file = self.file.replace(core::ptr::null_mut());
        if !file.is_null() {
            // SAFETY: the file was opened by `open` and is closed once.
            unsafe { CloseHandle(file) };
        }
    }

    /// Moves each file up one index, dropping the last, and starts anew.
    fn rotate(&self) {
        self.close();
        for index in (1..=self.max_files).rev() {
            let source = rotated_name(&self.path, index - 1);
            if !exists(&source) {
                continue;
            }
            let target = wide(&rotated_name(&self.path, index));
            let source = wide(&source);
            // SAFETY: both names are nul-terminated.
            unsafe {
                DeleteFileW(target.as_ptr());
                MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_REPLACE_EXISTING);
            }
        }
        self.open();
    }

    fn write(&self, line: &[u8]) {
        if self.file.get().is_null() && !self.open() {
            return;
        }
        if self.size.get() + line.len() as u64 > self.max_size && self.size.get() > 0 {
            self.rotate();
        }
        let file = self.file.get();
        if file.is_null() {
            return;
        }
        let mut written = 0;
        // SAFETY: the file is open and the line is readable for its length.
        unsafe {
            WriteFile(
                file,
                line.as_ptr(),
                line.len() as u32,
                &mut written,
                core::ptr::null_mut(),
            )
        };
        self.size.set(self.size.get() + u64::from(written));
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.close();
    }
}

enum Format {
    Pattern(String),
    /// The message alone, as the addon's `clearFormatters` writes it.
    Message,
}

struct Logger {
    name: String,
    level: Cell<u8>,
    format: RefCell<Format>,
    /// None writes to standard output.
    sink: Option<Sink>,
}

fn two(out: &mut String, value: u16) {
    let _ = core::fmt::Write::write_fmt(out, format_args!("{value:02}"));
}

/// A line as spdlog's pattern formatter writes it.
fn format_line(pattern: &str, logger: &str, level: u8, message: &str) -> String {
    let mut time = SystemTime::default();
    // SAFETY: the structure is writable.
    unsafe { GetLocalTime(&mut time) };
    let mut out = String::with_capacity(pattern.len() + message.len() + 16);
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let Some(flag) = chars.next() else {
            out.push('%');
            break;
        };
        match flag {
            'Y' => {
                let _ = core::fmt::Write::write_fmt(&mut out, format_args!("{}", time.year));
            }
            'y' => two(&mut out, time.year % 100),
            'm' => two(&mut out, time.month),
            'd' => two(&mut out, time.day),
            'H' => two(&mut out, time.hour),
            'I' => two(
                &mut out,
                if time.hour % 12 == 0 {
                    12
                } else {
                    time.hour % 12
                },
            ),
            'M' => two(&mut out, time.minute),
            'S' => two(&mut out, time.second),
            'p' => out.push_str(if time.hour >= 12 { "PM" } else { "AM" }),
            'e' => {
                let _ =
                    core::fmt::Write::write_fmt(&mut out, format_args!("{:03}", time.milliseconds));
            }
            'f' => {
                let _ = core::fmt::Write::write_fmt(
                    &mut out,
                    format_args!("{:06}", u32::from(time.milliseconds) * 1000),
                );
            }
            'F' => {
                let _ = core::fmt::Write::write_fmt(
                    &mut out,
                    format_args!("{:09}", u64::from(time.milliseconds) * 1_000_000),
                );
            }
            'D' => {
                two(&mut out, time.month);
                out.push('/');
                two(&mut out, time.day);
                out.push('/');
                two(&mut out, time.year % 100);
            }
            'T' | 'X' => {
                two(&mut out, time.hour);
                out.push(':');
                two(&mut out, time.minute);
                out.push(':');
                two(&mut out, time.second);
            }
            'R' => {
                two(&mut out, time.hour);
                out.push(':');
                two(&mut out, time.minute);
            }
            'l' => out.push_str(LEVELS[usize::from(level.min(OFF))]),
            'L' => out.push_str(SHORT_LEVELS[usize::from(level.min(OFF))]),
            'n' => out.push_str(logger),
            'v' => out.push_str(message),
            't' => {
                // SAFETY: GetCurrentThreadId takes no arguments.
                let thread = unsafe { GetCurrentThreadId() };
                let _ = core::fmt::Write::write_fmt(&mut out, format_args!("{thread}"));
            }
            'P' => {
                let _ = core::fmt::Write::write_fmt(
                    &mut out,
                    format_args!("{}", crate::platform::process_id()),
                );
            }
            '%' => out.push('%'),
            other => {
                out.push('%');
                out.push(other);
            }
        }
    }
    out
}

impl Logger {
    fn log(&self, level: u8, message: &str) {
        if level < self.level.get() || self.level.get() == OFF {
            return;
        }
        let mut line = match &*self.format.borrow() {
            Format::Pattern(pattern) => format_line(pattern, &self.name, level, message),
            Format::Message => message.into(),
        };
        if matches!(&*self.format.borrow(), Format::Pattern(_)) {
            line.push_str("\r\n");
        }
        match &self.sink {
            Some(sink) => sink.write(line.as_bytes()),
            None => {
                crate::platform::write_standard(1, line.as_bytes());
            }
        }
    }
}

/// The loggers of a realm by name, with the global level new ones take.
#[derive(Default)]
struct Registry {
    loggers: RefCell<Vec<Rc<Logger>>>,
    level: Cell<u8>,
}

fn level_argument(realm: &mut Realm, value: &Value, missing: &str) -> Result<u8, Thrown> {
    let Value::Number(level) = value else {
        return Err(Thrown(Value::Object(
            realm.error(ErrorKind::Error, missing),
        )));
    };
    let level = *level as i64;
    if !(0..7).contains(&level) {
        return Err(Thrown(Value::Object(
            realm.error(ErrorKind::Error, "Invalid level"),
        )));
    }
    Ok(level as u8)
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

fn error(realm: &mut Realm, message: &str) -> Thrown {
    Thrown(Value::Object(realm.error(ErrorKind::Error, message)))
}

fn method(
    realm: &Realm,
    target: &Object,
    name: &'static str,
    body: impl Fn(&mut Realm, &[Value]) -> crate::Completion + 'static,
) {
    let function = realm.function(name, 0, false, move |realm, _, args| body(realm, args));
    target.define(Key::name(name), Property::data(Value::Object(function)));
}

pub(crate) fn module(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let registry = Rc::new(Registry {
        level: Cell::new(INFO),
        ..Registry::default()
    });
    exports.define(Key::name("version"), Property::data(Value::Number(VERSION)));
    let global = registry.clone();
    method(realm, &exports, "setLevel", move |realm, args| {
        let level = level_argument(realm, &arg(args, 0), "Provide level")?;
        global.level.set(level);
        for logger in global.loggers.borrow().iter() {
            logger.level.set(level);
        }
        Ok(Value::Undefined)
    });
    // Lines are written as they are logged: every level flushes.
    method(realm, &exports, "setFlushOn", |realm, args| {
        level_argument(realm, &arg(args, 0), "Provide flush level")?;
        Ok(Value::Undefined)
    });
    let prototype = realm.object();
    let instance_prototype = prototype.clone();
    let constructor = realm.function("Logger", 5, true, move |realm, _, args| {
        if matches!(realm.new_target(), Value::Undefined) {
            return Err(realm.throw(
                ErrorKind::Type,
                "Class constructor Logger cannot be invoked without 'new'",
            ));
        }
        let Value::String(kind) = arg(args, 0) else {
            return Err(error(realm, "Provide a logger name"));
        };
        let kind = kind.to_rust_lossy();
        let logger = if kind == "rotating" || kind == "rotating_async" {
            let (Value::String(name), Value::String(file)) = (arg(args, 1), arg(args, 2)) else {
                return Err(error(realm, "Provide a logger name and file name"));
            };
            let (Value::Number(size), Value::Number(files)) = (arg(args, 3), arg(args, 4)) else {
                return Err(error(realm, "Provide the file size and count"));
            };
            let name = name.to_rust_lossy();
            let known = registry
                .loggers
                .borrow()
                .iter()
                .find(|logger| logger.name == name)
                .cloned();
            match known {
                Some(logger) => logger,
                None => {
                    let sink = Sink {
                        path: file.to_rust_lossy(),
                        max_size: size.max(0.0) as u64,
                        max_files: files.max(0.0) as usize,
                        file: Cell::new(core::ptr::null_mut()),
                        size: Cell::new(0),
                    };
                    if !sink.open() {
                        return Err(error(
                            realm,
                            &alloc::format!("Failed opening file {} for writing", sink.path),
                        ));
                    }
                    let logger = Rc::new(Logger {
                        name,
                        level: Cell::new(registry.level.get()),
                        format: RefCell::new(Format::Pattern(DEFAULT_PATTERN.into())),
                        sink: Some(sink),
                    });
                    registry.loggers.borrow_mut().push(logger.clone());
                    logger
                }
            }
        } else {
            let logger = Rc::new(Logger {
                name: kind,
                level: Cell::new(registry.level.get()),
                format: RefCell::new(Format::Pattern(DEFAULT_PATTERN.into())),
                sink: None,
            });
            registry.loggers.borrow_mut().push(logger.clone());
            logger
        };
        let current = Rc::new(RefCell::new(Some(logger)));
        let object = realm.object();
        object.set_prototype(Some(instance_prototype.clone()));
        for (name, level) in [
            ("trace", 0_u8),
            ("debug", 1),
            ("info", 2),
            ("warn", 3),
            ("error", 4),
            ("critical", 5),
        ] {
            let current = current.clone();
            method(realm, &object, name, move |realm, args| {
                let Value::String(message) = arg(args, 0) else {
                    return Err(error(realm, "Provide a message"));
                };
                if let Some(logger) = &*current.borrow() {
                    logger.log(level, &message.to_rust_lossy());
                }
                Ok(Value::Undefined)
            });
        }
        let level = current.clone();
        method(realm, &object, "getLevel", move |_, _| {
            let level = level
                .borrow()
                .as_ref()
                .map_or(INFO, |logger| logger.level.get());
            Ok(Value::Number(f64::from(level)))
        });
        let set_level = current.clone();
        method(realm, &object, "setLevel", move |realm, args| {
            let level = level_argument(realm, &arg(args, 0), "Provide level")?;
            if let Some(logger) = &*set_level.borrow() {
                logger.level.set(level);
            }
            Ok(Value::Undefined)
        });
        let pattern = current.clone();
        method(realm, &object, "setPattern", move |realm, args| {
            let Value::String(text) = arg(args, 0) else {
                return Err(error(realm, "Provide pattern"));
            };
            if let Some(logger) = &*pattern.borrow() {
                *logger.format.borrow_mut() = Format::Pattern(text.to_rust_lossy());
            }
            Ok(Value::Undefined)
        });
        let clear = current.clone();
        method(realm, &object, "clearFormatters", move |_, _| {
            if let Some(logger) = &*clear.borrow() {
                *logger.format.borrow_mut() = Format::Message;
            }
            Ok(Value::Undefined)
        });
        method(realm, &object, "flush", |_, _| Ok(Value::Undefined));
        let drop_registry = registry.clone();
        method(realm, &object, "drop", move |_, _| {
            if let Some(logger) = current.borrow_mut().take() {
                drop_registry
                    .loggers
                    .borrow_mut()
                    .retain(|known| known.name != logger.name);
            }
            Ok(Value::Undefined)
        });
        Ok(Value::Object(object))
    });
    constructor.define(
        Key::name("prototype"),
        Property::hidden(Value::Object(prototype.clone())),
    );
    prototype.define(
        Key::name("constructor"),
        Property::hidden(Value::Object(constructor.clone())),
    );
    exports.define(
        Key::name("Logger"),
        Property::data(Value::Object(constructor)),
    );
    exports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_files_take_the_index_before_the_extension() {
        assert_eq!(rotated_name("C:\\logs\\main.log", 0), "C:\\logs\\main.log");
        assert_eq!(
            rotated_name("C:\\logs\\main.log", 2),
            "C:\\logs\\main.2.log"
        );
        assert_eq!(rotated_name("C:\\logs.d\\main", 1), "C:\\logs.d\\main.1");
    }

    #[test]
    fn patterns_format_as_spdlog_does() {
        let line = format_line(
            "%Y-%m-%d %H:%M:%S.%e [%l] [%n] %v %L %% %q",
            "main",
            3,
            "hello",
        );
        let (date, rest) = line.split_at(23);
        assert_eq!(date.len(), 23);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[19..20], ".");
        assert_eq!(rest, " [warning] [main] hello W % %q");
    }

    #[test]
    fn loggers_write_rotate_and_keep_levels() {
        let base = std::env::temp_dir()
            .join(alloc::format!(
                "nativeboat-spdlog-{}.log",
                crate::platform::process_id()
            ))
            .to_string_lossy()
            .into_owned();
        for index in 0..3 {
            let name = wide(&rotated_name(&base, index));
            // SAFETY: the name is nul-terminated.
            unsafe { DeleteFileW(name.as_ptr()) };
        }
        let sink = Sink {
            path: base.clone(),
            max_size: 16,
            max_files: 2,
            file: Cell::new(core::ptr::null_mut()),
            size: Cell::new(0),
        };
        assert!(sink.open());
        let logger = Logger {
            name: "test".into(),
            level: Cell::new(INFO),
            format: RefCell::new(Format::Message),
            sink: Some(sink),
        };
        logger.log(1, "skipped");
        logger.log(2, "0123456789");
        logger.log(4, "abcdefghij");
        logger.log(5, "ABCDEFGHIJ");
        drop(logger);
        let read =
            |index| crate::platform::read_file(&rotated_name(&base, index)).unwrap_or_default();
        assert_eq!(read(0), b"ABCDEFGHIJ");
        assert_eq!(read(1), b"abcdefghij");
        assert_eq!(read(2), b"0123456789");
        for index in 0..3 {
            let name = wide(&rotated_name(&base, index));
            // SAFETY: the name is nul-terminated.
            unsafe { DeleteFileW(name.as_ptr()) };
        }
    }
}
