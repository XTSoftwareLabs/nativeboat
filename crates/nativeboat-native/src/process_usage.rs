//! The process's memory and CPU use, as Node's `process.memoryUsage()`,
//! `process.resourceUsage()` and `process.cpuUsage()` and Electron's
//! `process.getProcessMemoryInfo()` report them, from the Windows process
//! counters. There is no V8 heap: `heapTotal`, `heapUsed` and `external`
//! are 0, and `rss` is the working set, as Node reports it on Windows.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::{Key, Object, Property, Realm, Thrown, Value, ops};
use core::ffi::c_void;

#[repr(C)]
#[derive(Default)]
struct MemoryCounters {
    size: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_usage: usize,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    /// Microseconds, from 100-nanosecond intervals.
    fn micros(self) -> f64 {
        ((u64::from(self.high) << 32 | u64::from(self.low)) / 10) as f64
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn K32GetProcessMemoryInfo(process: *mut c_void, counters: *mut c_void, size: u32) -> i32;
    fn GetProcessTimes(
        process: *mut c_void,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn GetSystemTimeAsFileTime(time: *mut FileTime);
}

/// The current process's memory counters, zero where Windows gives none.
fn memory() -> MemoryCounters {
    let mut counters = MemoryCounters {
        size: core::mem::size_of::<MemoryCounters>() as u32,
        ..MemoryCounters::default()
    };
    // SAFETY: the pseudo-handle names this process and the structure is
    // sized for PROCESS_MEMORY_COUNTERS_EX.
    unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&raw mut counters).cast(),
            counters.size,
        )
    };
    counters
}

/// User and kernel CPU time in microseconds.
fn cpu_times() -> (f64, f64) {
    let (mut creation, mut exit, mut kernel, mut user) = Default::default();
    // SAFETY: the pseudo-handle names this process; all out-pointers are valid.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return (0.0, 0.0);
    }
    (user.micros(), kernel.micros())
}

/// `process.uptime()`: seconds since the process started.
/// When the process started, in milliseconds since 1970.
pub(crate) fn creation_time_ms() -> f64 {
    let (mut creation, mut exit, mut kernel, mut user) = Default::default();
    // SAFETY: the pseudo-handle names this process; all out-pointers are
    // valid.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return 0.0;
    }
    // FILETIME counts from 1601; the Unix epoch is 11 644 473 600 s later.
    (FileTime::micros(creation) - 11_644_473_600_000_000.0) / 1000.0
}

/// The process's working set, peak working set and private bytes, in
/// kilobytes, as Electron's process metrics give them.
pub(crate) fn memory_kilobytes() -> (f64, f64, f64) {
    let counters = memory();
    (
        (counters.working_set_size / 1024) as f64,
        (counters.peak_working_set_size / 1024) as f64,
        (counters.private_usage / 1024) as f64,
    )
}

pub(crate) fn uptime(realm: &mut Realm) -> Object {
    realm.function("uptime", 0, false, |_, _, _| {
        let (mut creation, mut exit, mut kernel, mut user, mut now) = Default::default();
        // SAFETY: the pseudo-handle names this process; all out-pointers are
        // valid.
        let ok = unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        };
        // SAFETY: the out-pointer is valid.
        unsafe { GetSystemTimeAsFileTime(&mut now) };
        let elapsed = if ok == 0 {
            0.0
        } else {
            (FileTime::micros(now) - FileTime::micros(creation)).max(0.0) / 1e6
        };
        Ok(Value::Number(elapsed))
    })
}

/// `process.availableMemory()`: the free memory libuv reports, as no
/// constraint applies on Windows.
pub(crate) fn available_memory(realm: &mut Realm) -> Object {
    realm.function("availableMemory", 0, false, |_, _, _| {
        Ok(Value::Number(crate::os_binding::memory().1 as f64))
    })
}

/// `process.constrainedMemory()`: zero, for no constraint.
pub(crate) fn constrained_memory(realm: &mut Realm) -> Object {
    realm.function("constrainedMemory", 0, false, |_, _, _| {
        Ok(Value::Number(0.0))
    })
}

fn number(object: &Object, name: &str, value: f64) {
    object.define(Key::name(name), Property::data(Value::Number(value)));
}

/// `process.memoryUsage()`, with `memoryUsage.rss()`.
pub(crate) fn memory_usage(realm: &mut Realm) -> Object {
    let usage = realm.function("memoryUsage", 0, false, |realm, _, _| {
        let counters = memory();
        let result = realm.object();
        number(&result, "rss", counters.working_set_size as f64);
        number(&result, "heapTotal", 0.0);
        number(&result, "heapUsed", 0.0);
        number(&result, "external", 0.0);
        number(&result, "arrayBuffers", 0.0);
        Ok(Value::Object(result))
    });
    let rss = realm.function("rss", 0, false, |_, _, _| {
        Ok(Value::Number(memory().working_set_size as f64))
    });
    usage.define(Key::name("rss"), Property::data(Value::Object(rss)));
    usage
}

/// `process.resourceUsage()`: CPU times in microseconds and the peak
/// working set in kilobytes, as Node fills them from libuv on Windows.
pub(crate) fn resource_usage(realm: &mut Realm) -> Object {
    realm.function("resourceUsage", 0, false, |realm, _, _| {
        let counters = memory();
        let (user, system) = cpu_times();
        let result = realm.object();
        number(&result, "userCPUTime", user);
        number(&result, "systemCPUTime", system);
        number(
            &result,
            "maxRSS",
            (counters.peak_working_set_size / 1024) as f64,
        );
        for name in [
            "sharedMemorySize",
            "unsharedDataSize",
            "unsharedStackSize",
            "minorPageFault",
            "swappedOut",
            "fsRead",
            "fsWrite",
            "ipcSent",
            "ipcReceived",
            "signalsCount",
            "voluntaryContextSwitches",
            "involuntaryContextSwitches",
        ] {
            number(&result, name, 0.0);
        }
        number(
            &result,
            "majorPageFault",
            f64::from(counters.page_fault_count),
        );
        Ok(Value::Object(result))
    })
}

/// `process.cpuUsage(previous?)`: user and system time in microseconds,
/// less `previous` when given.
pub(crate) fn cpu_usage(realm: &mut Realm) -> Object {
    realm.function("cpuUsage", 1, false, |realm, _, args| {
        let (mut user, mut system) = cpu_times();
        if let Some(previous @ Value::Object(_)) = args.first() {
            let previous_user = ops::get_named(realm, previous, "user")?;
            user -= ops::to_number(realm, &previous_user)?;
            let previous_system = ops::get_named(realm, previous, "system")?;
            system -= ops::to_number(realm, &previous_system)?;
        }
        let result = realm.object();
        number(&result, "user", user);
        number(&result, "system", system);
        Ok(Value::Object(result))
    })
}

/// Electron's `process.getProcessMemoryInfo()`: a promise of the working set
/// and private bytes in kilobytes.
pub(crate) fn process_memory_info(realm: &mut Realm) -> Object {
    realm.function("getProcessMemoryInfo", 0, false, |realm, _, _| {
        let counters = memory();
        let result = realm.object();
        number(
            &result,
            "residentSet",
            (counters.working_set_size / 1024) as f64,
        );
        number(&result, "private", (counters.private_usage / 1024) as f64);
        number(
            &result,
            "shared",
            (counters
                .working_set_size
                .saturating_sub(counters.private_usage)
                / 1024) as f64,
        );
        let (promise, resolve, _) = crate::promise_builtin::capability(realm);
        ops::call(realm, &resolve, Value::Undefined, &[Value::Object(result)])?;
        Ok::<Value, Thrown>(promise)
    })
}
