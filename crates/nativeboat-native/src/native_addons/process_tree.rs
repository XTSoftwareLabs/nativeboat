//! `@vscode/windows-process-tree`'s binding (`windows_process_tree.node`):
//! `getProcessList(callback, flags)` lists processes from a Toolhelp
//! snapshot, at most 1024 as the addon does, with their working set
//! (`flags & 1`) and command line (`flags & 2`); `getProcessCpuUsage(list,
//! callback)` samples each listed process's CPU time over a second and sets
//! its `cpu` percentage of the system's. Both run on the I/O pool and call
//! back on the realm's thread.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Value, ops};
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;

const TH32CS_SNAPPROCESS: u32 = 0x2;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_VM_READ: u32 = 0x0010;
const MEMORY: u32 = 1;
const COMMAND_LINE: u32 = 2;
const MAX_PROCESSES: usize = 1024;
/// `ProcessCommandLineInformation`, Windows 8.1 and later.
const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;

#[repr(C)]
struct ProcessEntry {
    size: u32,
    usage: u32,
    process_id: u32,
    default_heap: usize,
    module_id: u32,
    threads: u32,
    parent_process_id: u32,
    priority: i32,
    flags: u32,
    exe_file: [u16; 260],
}

#[repr(C)]
#[derive(Default)]
struct MemoryCounters {
    size: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    quota_peak_paged_pool: usize,
    quota_paged_pool: usize,
    quota_peak_non_paged_pool: usize,
    quota_non_paged_pool: usize,
    pagefile: usize,
    peak_pagefile: usize,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    fn ticks(self) -> u64 {
        (u64::from(self.high) << 32) | u64::from(self.low)
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, process: u32) -> *mut c_void;
    fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
    fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
    fn OpenProcess(access: u32, inherit: i32, process: u32) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut c_void, size: u32) -> i32;
    fn GetProcessTimes(
        process: *mut c_void,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
    fn Sleep(milliseconds: u32);
}

/// A process as the addon lists it.
struct ProcessInfo {
    pid: u32,
    ppid: u32,
    name: String,
    memory: Option<f64>,
    command_line: Option<String>,
}

/// An open process handle, closed when dropped.
struct Process(*mut c_void);

impl Process {
    fn open(access: u32, pid: u32) -> Option<Self> {
        // SAFETY: OpenProcess takes plain values; null means it failed.
        let handle = unsafe { OpenProcess(access, 0, pid) };
        (!handle.is_null()).then_some(Self(handle))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `open` and is closed once.
        unsafe { CloseHandle(self.0) };
    }
}

fn working_set(pid: u32) -> f64 {
    let Some(process) = Process::open(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, pid) else {
        return 0.0;
    };
    let mut counters = MemoryCounters {
        size: core::mem::size_of::<MemoryCounters>() as u32,
        ..MemoryCounters::default()
    };
    // SAFETY: the handle is open and the counters are sized for the call.
    if unsafe { K32GetProcessMemoryInfo(process.0, (&raw mut counters).cast(), counters.size) } == 0
    {
        return 0.0;
    }
    // The addon keeps the working set as a 32-bit DWORD.
    f64::from(counters.working_set as u32)
}

/// A process's command line, as Windows reports it, or nothing when the
/// process cannot be queried.
fn command_line(pid: u32) -> Option<String> {
    type Query = unsafe extern "system" fn(*mut c_void, u32, *mut c_void, u32, *mut u32) -> i32;
    let procedure = crate::platform::system_procedure("ntdll.dll", c"NtQueryInformationProcess");
    if procedure.is_null() {
        return None;
    }
    let process = Process::open(PROCESS_QUERY_LIMITED_INFORMATION, pid)?;
    // SAFETY: the procedure is NtQueryInformationProcess.
    let query: Query = unsafe { core::mem::transmute(procedure) };
    let mut needed = 0;
    // SAFETY: a zero-length query reports the size the information needs.
    unsafe {
        query(
            process.0,
            PROCESS_COMMAND_LINE_INFORMATION,
            core::ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if needed == 0 {
        return None;
    }
    // A UNICODE_STRING (length, maximum length, buffer) followed by the
    // text it points to.
    let mut buffer =
        alloc::vec![0_usize; (needed as usize).div_ceil(core::mem::size_of::<usize>())];
    let size = (buffer.len() * core::mem::size_of::<usize>()) as u32;
    // SAFETY: the buffer is writable and aligned for the structure.
    let status = unsafe {
        query(
            process.0,
            PROCESS_COMMAND_LINE_INFORMATION,
            buffer.as_mut_ptr().cast(),
            size,
            &mut needed,
        )
    };
    if status < 0 {
        return None;
    }
    let base = buffer.as_ptr().cast::<u8>();
    // SAFETY: the structure starts the buffer; its text pointer points into
    // the buffer for `length` bytes.
    unsafe {
        let length = usize::from(*base.cast::<u16>()) / 2;
        let text = *base.add(core::mem::size_of::<usize>()).cast::<*const u16>();
        if text.is_null() {
            return Some(String::new());
        }
        Some(String::from_utf16_lossy(core::slice::from_raw_parts(
            text, length,
        )))
    }
}

fn utf16_name(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

fn process_list(flags: u32) -> Vec<ProcessInfo> {
    let mut processes = Vec::new();
    // SAFETY: a process snapshot; an invalid handle makes the first call fail.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot as isize == -1 {
        return processes;
    }
    // SAFETY: every field is an integer or array, for which zero is valid.
    let mut entry: ProcessEntry = unsafe { core::mem::zeroed() };
    entry.size = core::mem::size_of::<ProcessEntry>() as u32;
    // SAFETY: the snapshot is open and the entry is sized for the call.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        if entry.process_id != 0 {
            processes.push(ProcessInfo {
                pid: entry.process_id,
                ppid: entry.parent_process_id,
                name: utf16_name(&entry.exe_file),
                memory: (flags & MEMORY != 0).then(|| working_set(entry.process_id)),
                command_line: (flags & COMMAND_LINE != 0)
                    .then(|| command_line(entry.process_id).unwrap_or_default()),
            });
        }
        if processes.len() >= MAX_PROCESSES {
            break;
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: the snapshot was opened above and is closed once.
    unsafe { CloseHandle(snapshot) };
    processes
}

/// A process's and the system's busy time, in 100 ns ticks.
fn cpu_times(pid: u32) -> Option<(u64, u64)> {
    let process = Process::open(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, pid)?;
    let (mut creation, mut exit, mut kernel, mut user) = Default::default();
    let (mut idle, mut system_kernel, mut system_user) = Default::default();
    // SAFETY: the handle is open and every out-pointer is valid.
    let read = unsafe {
        GetProcessTimes(process.0, &mut creation, &mut exit, &mut kernel, &mut user) != 0
            && GetSystemTimes(&mut idle, &mut system_kernel, &mut system_user) != 0
    };
    read.then(|| {
        (
            kernel.ticks() + user.ticks(),
            system_kernel.ticks() + system_user.ticks(),
        )
    })
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

pub(crate) fn module(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let list = realm.function("getProcessList", 2, false, |realm, _, args| {
        if args.len() < 2 {
            return Err(realm.throw(ErrorKind::Type, "GetProcessList expects two arguments."));
        }
        let callback = arg(args, 0);
        if !crate::array_builtin::is_callable(&callback) {
            return Err(realm.throw(
                ErrorKind::Type,
                "The first argument of GetProcessList, callback, must be a function.",
            ));
        }
        let Value::Number(flags) = arg(args, 1) else {
            return Err(realm.throw(
                ErrorKind::Type,
                "The second argument of GetProcessList, flags, must be a number.",
            ));
        };
        let flags = flags as i32 as u32;
        let submitted = realm.submit_io(
            move || {
                process_list(flags)
                    .into_iter()
                    .map(|process| {
                        (
                            process.pid,
                            process.ppid,
                            process.name,
                            process.memory,
                            process.command_line,
                        )
                    })
                    .collect::<Vec<_>>()
            },
            move |realm, processes| {
                let mut values = Vec::with_capacity(processes.len());
                for (pid, ppid, name, memory, command_line) in processes {
                    let object = realm.object();
                    object.define(Key::name("name"), Property::data(Value::str(&name)));
                    object.define(
                        Key::name("pid"),
                        Property::data(Value::Number(f64::from(pid))),
                    );
                    object.define(
                        Key::name("ppid"),
                        Property::data(Value::Number(f64::from(ppid))),
                    );
                    if let Some(memory) = memory {
                        object.define(Key::name("memory"), Property::data(Value::Number(memory)));
                    }
                    if let Some(command_line) = command_line {
                        object.define(
                            Key::name("commandLine"),
                            Property::data(Value::str(&command_line)),
                        );
                    }
                    values.push(Value::Object(object));
                }
                let list = Value::Object(realm.array(values));
                ops::call(realm, &callback, Value::Undefined, &[list]).map(drop)
            },
        );
        if submitted.is_err() {
            return Err(realm.throw(ErrorKind::Error, "Could not queue the process list"));
        }
        Ok(Value::Undefined)
    });
    exports.define(
        Key::name("getProcessList"),
        Property::data(Value::Object(list)),
    );
    let cpu = realm.function("getProcessCpuUsage", 2, false, |realm, _, args| {
        if args.len() < 2 {
            return Err(realm.throw(ErrorKind::Type, "GetProcessCpuUsage expects two arguments."));
        }
        let processes = arg(args, 0);
        let is_array = matches!(&processes, Value::Object(object) if object.is_array());
        if !is_array {
            return Err(realm.throw(
                ErrorKind::Type,
                "The first argument of GetProcessCpuUsage, processList, must be an array.",
            ));
        }
        let callback = arg(args, 1);
        if !crate::array_builtin::is_callable(&callback) {
            return Err(realm.throw(
                ErrorKind::Type,
                "The second argument of GetProcessCpuUsage, callback, must be a function.",
            ));
        }
        let length = match ops::get_named(realm, &processes, "length")? {
            Value::Number(length) => length as u32,
            _ => 0,
        };
        let mut items = Vec::with_capacity(length as usize);
        let mut pids = Vec::with_capacity(length as usize);
        for index in 0..length {
            let item = ops::get(realm, &processes, &Key::Index(index))?;
            let pid = match &item {
                Value::Object(_) => match ops::get_named(realm, &item, "pid")? {
                    Value::Number(pid) => pid as i32 as u32,
                    _ => 0,
                },
                _ => 0,
            };
            items.push(item);
            pids.push(pid);
        }
        let submitted = realm.submit_io(
            move || {
                let first: Vec<Option<(u64, u64)>> =
                    pids.iter().map(|pid| cpu_times(*pid)).collect();
                // SAFETY: Sleep takes a plain duration.
                unsafe { Sleep(1000) };
                pids.iter()
                    .zip(first)
                    .map(|(pid, first)| {
                        let (process, system) = first?;
                        let (process_end, system_end) = cpu_times(*pid)?;
                        let elapsed = system_end.saturating_sub(system);
                        (elapsed > 0).then(|| {
                            100.0 * process_end.saturating_sub(process) as f64 / elapsed as f64
                        })
                    })
                    .collect::<Vec<Option<f64>>>()
            },
            move |realm, usage| {
                for (item, cpu) in items.iter().zip(usage) {
                    if let (Value::Object(_), Some(cpu)) = (item, cpu) {
                        ops::set(realm, item, &Key::name("cpu"), Value::Number(cpu))?;
                    }
                }
                let list = Value::Object(realm.array(items));
                ops::call(realm, &callback, Value::Undefined, &[list]).map(drop)
            },
        );
        if submitted.is_err() {
            return Err(realm.throw(ErrorKind::Error, "Could not queue the CPU sample"));
        }
        Ok(Value::Undefined)
    });
    exports.define(
        Key::name("getProcessCpuUsage"),
        Property::data(Value::Object(cpu)),
    );
    exports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_listed_with_its_parent_memory_and_command_line() {
        let pid = crate::platform::process_id();
        let processes = process_list(MEMORY | COMMAND_LINE);
        let this = processes
            .iter()
            .find(|process| process.pid == pid)
            .expect("this process is listed");
        assert!(this.name.to_ascii_lowercase().ends_with(".exe"));
        assert!(this.memory.is_some_and(|memory| memory > 0.0));
        assert!(
            this.command_line
                .as_deref()
                .is_some_and(|line| !line.is_empty())
        );
        let plain = process_list(0);
        assert!(
            plain
                .iter()
                .all(|process| process.memory.is_none() && process.command_line.is_none())
        );
        assert!(plain.len() <= MAX_PROCESSES);
        assert!(cpu_times(pid).is_some());
    }
}
