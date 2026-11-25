//! Node's `os` binding (`internalBinding('os')`) on Windows, following the
//! libuv in Node 22: memory, processors, uptime, the host and user names,
//! the home directory, the operating-system version, process priorities
//! and network interfaces. Functions of DLLs other than kernel32 are looked
//! up when first needed, as libuv does for some, so executables import only
//! kernel32. Also `internalBinding('constants').os` and the credentials
//! binding `os` loads.

use crate::platform::system_procedure;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::uv_error::{self, translate};
use crate::{Completion, JsStr, Key, Object, Property, Realm, Value, ops, platform};
use alloc::vec;
use core::ffi::c_void;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    fn GetTickCount64() -> u64;
    fn GetCurrentProcess() -> *mut c_void;
    fn GetProcessAffinityMask(
        process: *mut c_void,
        process_mask: *mut usize,
        system_mask: *mut usize,
    ) -> i32;
    fn GetSystemInfo(info: *mut SystemInfo);
    fn GetComputerNameExW(format: u32, buffer: *mut u16, size: *mut u32) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetPriorityClass(process: *mut c_void) -> u32;
    fn SetPriorityClass(process: *mut c_void, class: u32) -> i32;
    fn GetLastError() -> u32;
    fn RegOpenKeyExW(
        key: isize,
        path: *const u16,
        options: u32,
        access: u32,
        result: *mut isize,
    ) -> i32;
    fn RegQueryValueExW(
        key: isize,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> i32;
    fn RegCloseKey(key: isize) -> i32;
}

/// `MEMORYSTATUSEX`.
#[repr(C)]
#[derive(Default)]
struct MemoryStatus {
    length: u32,
    load: u32,
    total_physical: u64,
    available_physical: u64,
    total_page_file: u64,
    available_page_file: u64,
    total_virtual: u64,
    available_virtual: u64,
    available_extended_virtual: u64,
}

/// `SYSTEM_INFO`.
#[repr(C)]
struct SystemInfo {
    processor_architecture: u16,
    reserved: u16,
    page_size: u32,
    minimum_address: *mut c_void,
    maximum_address: *mut c_void,
    active_processors: usize,
    processors: u32,
    processor_type: u32,
    allocation_granularity: u32,
    processor_level: u16,
    processor_revision: u16,
}

/// `SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION`, in 100-nanosecond units.
#[repr(C)]
#[derive(Clone, Default)]
struct ProcessorTimes {
    idle: i64,
    kernel: i64,
    user: i64,
    dpc: i64,
    interrupt: i64,
    interrupt_count: u32,
}

/// `OSVERSIONINFOW`.
#[repr(C)]
struct VersionInfo {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    service_pack: [u16; 128],
}

const UV_EINVAL: i32 = -4071;
const UV_ENOENT: i32 = -4058;
const UV_ENOSYS: i32 = -4054;
const UV_ESRCH: i32 = -4040;
const UV_EAGAIN: i32 = -4088;
const UV_ENOBUFS: i32 = -4060;
const UV_ENOMEM: i32 = -4057;

const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_BUFFER_OVERFLOW: u32 = 111;
const ERROR_NO_DATA: u32 = 232;
const ERROR_ADDRESS_NOT_ASSOCIATED: u32 = 1228;

fn last_error() -> u32 {
    // SAFETY: GetLastError reads this thread's latest Windows error.
    unsafe { GetLastError() }
}

/// How many logical processors the system has, as
/// `navigator.hardwareConcurrency` reports.
#[cfg(feature = "dom")]
pub(crate) fn processor_count() -> u32 {
    system_info().processors.max(1)
}

fn system_info() -> SystemInfo {
    // SAFETY: SYSTEM_INFO is plain data that GetSystemInfo fills.
    unsafe {
        let mut info: SystemInfo = core::mem::zeroed();
        GetSystemInfo(&mut info);
        info
    }
}

fn query_memory(status: &mut MemoryStatus) -> i32 {
    status.length = core::mem::size_of::<MemoryStatus>() as u32;
    // SAFETY: the structure is writable and its length is set.
    unsafe { GlobalMemoryStatusEx(status) }
}

/// `uv_get_total_memory` and `uv_get_free_memory`: zero when unknown.
pub(crate) fn memory() -> (u64, u64) {
    let mut status = MemoryStatus::default();
    if query_memory(&mut status) == 0 {
        return (0, 0);
    }
    (status.total_physical, status.available_physical)
}

fn affinity() -> usize {
    let (mut process, mut system) = (0, 0);
    // SAFETY: both masks are writable; the pseudo handle needs no closing.
    let ok = unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) };
    if ok == 0 { 0 } else { process }
}

/// `uv_available_parallelism`: the processors this process may run on.
fn available_parallelism() -> u32 {
    affinity().count_ones().max(1)
}

/// `uv_uptime`: seconds since the system started.
fn uptime() -> f64 {
    // SAFETY: GetTickCount64 has no preconditions.
    (unsafe { GetTickCount64() }) as f64 / 1000.0
}

/// UTF-16 text from Windows as libuv's WTF-8 reaches a JavaScript string.
fn os_text(units: &[u16]) -> JsStr {
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    platform::os_string(&units[..end])
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// A registry value under `HKEY_LOCAL_MACHINE`, or the Windows error.
fn registry(path: &str, name: &str) -> Result<Vec<u8>, u32> {
    const HKEY_LOCAL_MACHINE: isize = 0x8000_0002u32 as i32 as isize;
    const KEY_QUERY_VALUE_64: u32 = 0x0001 | 0x0100;
    let (path, name) = (wide(path), wide(name));
    let mut key = 0;
    // SAFETY: the strings are null-terminated, the outputs are writable and
    // the key is closed after use.
    unsafe {
        let error = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_ptr(),
            0,
            KEY_QUERY_VALUE_64,
            &mut key,
        );
        if error != 0 {
            return Err(error as u32);
        }
        let mut data = vec![0u8; 512];
        let mut size = data.len() as u32;
        let error = RegQueryValueExW(
            key,
            name.as_ptr(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            data.as_mut_ptr(),
            &mut size,
        );
        RegCloseKey(key);
        if error != 0 {
            return Err(error as u32);
        }
        data.truncate(size as usize);
        Ok(data)
    }
}

fn registry_text(bytes: &[u8]) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect()
}

/// One processor as `uv_cpu_info` reports it, times in milliseconds.
struct Cpu {
    model: Vec<u16>,
    speed: u32,
    times: [f64; 5],
}

fn query_processor_times(times: &mut [ProcessorTimes]) -> i32 {
    const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION: u32 = 8;
    type Query = unsafe extern "system" fn(u32, *mut c_void, u32, *mut u32) -> i32;
    let procedure = system_procedure("ntdll.dll", c"NtQuerySystemInformation");
    if procedure.is_null() {
        return -1;
    }
    // SAFETY: the procedure is NtQuerySystemInformation, and the buffer is
    // writable for its byte length.
    unsafe {
        let query: Query = core::mem::transmute(procedure);
        let mut written = 0;
        query(
            SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION,
            times.as_mut_ptr().cast(),
            core::mem::size_of_val(times) as u32,
            &mut written,
        )
    }
}

/// `uv_cpu_info`: each processor's model, speed and times; `None` on any
/// failure, which Node reports as no processors.
fn cpus() -> Option<Vec<Cpu>> {
    let count = system_info().processors as usize;
    let mut times = vec![ProcessorTimes::default(); count];
    if query_processor_times(&mut times) != 0 {
        return None;
    }
    let mut cpus = Vec::with_capacity(count);
    for (index, times) in times.iter().enumerate() {
        let path = format!("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\{index}");
        let speed = registry(&path, "~MHz").ok()?;
        let model = registry(&path, "ProcessorNameString").ok()?;
        let speed = speed.get(..4).map_or(0, |bytes| {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        });
        let milliseconds = |value: i64| (value / 10_000) as f64;
        cpus.push(Cpu {
            model: registry_text(&model),
            speed,
            times: [
                milliseconds(times.user),
                0.0,
                milliseconds(times.kernel - times.idle),
                milliseconds(times.idle),
                milliseconds(times.interrupt),
            ],
        });
    }
    Some(cpus)
}

fn query_host_name(buffer: &mut [u16]) -> i32 {
    const COMPUTER_NAME_DNS_HOSTNAME: u32 = 1;
    let mut size = buffer.len() as u32;
    // SAFETY: the buffer is writable for `size` units.
    unsafe { GetComputerNameExW(COMPUTER_NAME_DNS_HOSTNAME, buffer.as_mut_ptr(), &mut size) }
}

/// `uv_os_gethostname`: the DNS host name.
fn host_name() -> Result<JsStr, i32> {
    let mut buffer = [0u16; 256];
    if query_host_name(&mut buffer) == 0 {
        return Err(translate(last_error()));
    }
    Ok(os_text(&buffer))
}

/// A user as `uv_os_get_passwd` reports one on Windows.
struct User {
    username: JsStr,
    homedir: JsStr,
}

fn open_token() -> Result<*mut c_void, i32> {
    const TOKEN_READ: u32 = 0x0002_0008;
    let mut token = core::ptr::null_mut();
    // SAFETY: the pseudo handle needs no closing; `token` is writable.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_READ, &mut token) } == 0 {
        return Err(translate(last_error()));
    }
    Ok(token)
}

/// `GetUserProfileDirectoryW` into `buffer`, or `None` without userenv.
fn query_profile(token: *mut c_void, buffer: &mut [u16], size: &mut u32) -> Option<i32> {
    type Query = unsafe extern "system" fn(*mut c_void, *mut u16, *mut u32) -> i32;
    let procedure = system_procedure("userenv.dll", c"GetUserProfileDirectoryW");
    if procedure.is_null() {
        return None;
    }
    let pointer = if buffer.is_empty() {
        core::ptr::null_mut()
    } else {
        buffer.as_mut_ptr()
    };
    // SAFETY: the procedure is GetUserProfileDirectoryW and the buffer is
    // null or writable for `size` units.
    Some(unsafe { core::mem::transmute::<*const c_void, Query>(procedure)(token, pointer, size) })
}

fn profile_directory(token: *mut c_void) -> Result<Vec<u16>, i32> {
    let mut size = 0;
    query_profile(token, &mut [], &mut size).ok_or(UV_ENOSYS)?;
    let error = last_error();
    if error != ERROR_INSUFFICIENT_BUFFER {
        return Err(translate(error));
    }
    let mut path = vec![0u16; size as usize];
    if query_profile(token, &mut path, &mut size) == Some(0) {
        return Err(translate(last_error()));
    }
    Ok(path)
}

/// `GetUserNameW` into `buffer`, or `None` without advapi32.
fn query_user_name(buffer: &mut [u16], size: &mut u32) -> Option<i32> {
    type Query = unsafe extern "system" fn(*mut u16, *mut u32) -> i32;
    let procedure = system_procedure("advapi32.dll", c"GetUserNameW");
    if procedure.is_null() {
        return None;
    }
    // SAFETY: the procedure is GetUserNameW and the buffer holds `size` units.
    Some(unsafe {
        core::mem::transmute::<*const c_void, Query>(procedure)(buffer.as_mut_ptr(), size)
    })
}

fn user_name() -> Result<Vec<u16>, i32> {
    let mut name = vec![0u16; 257];
    let mut size = name.len() as u32;
    if query_user_name(&mut name, &mut size).ok_or(UV_ENOSYS)? == 0 {
        let error = last_error();
        return Err(if error == ERROR_INSUFFICIENT_BUFFER {
            UV_ENOMEM
        } else {
            translate(error)
        });
    }
    Ok(name)
}

/// `uv_os_get_passwd`: the profile directory and the user name.
fn user() -> Result<User, i32> {
    let token = open_token()?;
    let path = profile_directory(token);
    // SAFETY: the token was opened above and is closed once.
    unsafe { CloseHandle(token) };
    let homedir = os_text(&path?);
    let username = os_text(&user_name()?);
    Ok(User { username, homedir })
}

/// `uv_os_homedir`: `USERPROFILE` when set, else the user's profile.
fn home_directory() -> Result<JsStr, i32> {
    match platform::environment_value(&JsStr::from("USERPROFILE")) {
        Some(home) if crate::encoding::utf8_length(home.units()) < 3 => Err(UV_ENOENT),
        Some(home) => Ok(home),
        None => user().map(|user| user.homedir),
    }
}

fn query_version(info: &mut VersionInfo) {
    type Query = unsafe extern "system" fn(*mut VersionInfo) -> i32;
    info.size = core::mem::size_of::<VersionInfo>() as u32;
    let procedure = system_procedure("ntdll.dll", c"RtlGetVersion");
    if !procedure.is_null() {
        // SAFETY: the procedure is RtlGetVersion and the structure's size is set.
        unsafe { core::mem::transmute::<*const c_void, Query>(procedure)(info) };
    }
}

/// `uv_os_uname`: system name, version, release and machine.
fn uname() -> [String; 4] {
    // SAFETY: OSVERSIONINFOW is plain data.
    let mut info: VersionInfo = unsafe { core::mem::zeroed() };
    query_version(&mut info);
    let mut version = String::new();
    if let Ok(bytes) = registry(
        "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
        "ProductName",
    ) {
        let mut product = registry_text(&bytes);
        // Windows 11 reports itself as Windows 10; its builds start at 22000.
        if info.major == 10 && info.build >= 22000 && product.starts_with(&wide("Windows 10")[..10])
        {
            product[9] = u16::from(b'1');
        }
        version = String::from_utf16_lossy(&product);
    }
    let end = info
        .service_pack
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(128);
    let service_pack = &info.service_pack[..end];
    if !service_pack.is_empty() {
        if !version.is_empty() {
            version.push(' ');
        }
        version.push_str(&String::from_utf16_lossy(service_pack));
    }
    let release = format!("{}.{}.{}", info.major, info.minor, info.build);
    let system = system_info();
    let machine = match system.processor_architecture {
        9 => "x86_64".to_owned(),
        6 => "ia64".to_owned(),
        0 => format!("i{}86", system.processor_level.clamp(3, 6)),
        10 => "i686".to_owned(),
        1 => "mips".to_owned(),
        2 | 7 => "alpha".to_owned(),
        3 => "powerpc".to_owned(),
        4 => "sh".to_owned(),
        5 => "arm".to_owned(),
        _ => "unknown".to_owned(),
    };
    ["Windows_NT".to_owned(), version, release, machine]
}

/// libuv's priorities and Windows priority classes, highest first.
const PRIORITIES: [(i32, u32); 6] = [
    (-20, 0x100), // REALTIME_PRIORITY_CLASS
    (-14, 0x80),  // HIGH_PRIORITY_CLASS
    (-7, 0x8000), // ABOVE_NORMAL_PRIORITY_CLASS
    (0, 0x20),    // NORMAL_PRIORITY_CLASS
    (10, 0x4000), // BELOW_NORMAL_PRIORITY_CLASS
    (19, 0x40),   // IDLE_PRIORITY_CLASS
];

/// libuv's `uv__get_handle`: this process for 0, else an opened process.
fn process_handle(pid: i32, access: u32) -> Result<(*mut c_void, bool), i32> {
    if pid == 0 {
        // SAFETY: GetCurrentProcess has no preconditions.
        return Ok((unsafe { GetCurrentProcess() }, false));
    }
    // SAFETY: OpenProcess has no preconditions; failure is checked.
    let handle = unsafe { OpenProcess(access, 0, pid as u32) };
    if handle.is_null() {
        let error = last_error();
        return Err(if error == ERROR_INVALID_PARAMETER {
            UV_ESRCH
        } else {
            translate(error)
        });
    }
    Ok((handle, true))
}

fn close_process(handle: *mut c_void, opened: bool) {
    if opened {
        // SAFETY: the handle was opened by `process_handle`.
        unsafe { CloseHandle(handle) };
    }
}

/// `uv_os_getpriority`.
fn priority(pid: i32) -> Result<i32, i32> {
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    let (handle, opened) = process_handle(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    // SAFETY: the handle is valid until it is closed below.
    let class = unsafe { GetPriorityClass(handle) };
    let result = if class == 0 {
        Err(translate(last_error()))
    } else {
        Ok(PRIORITIES
            .iter()
            .find(|(_, known)| *known == class)
            .map_or(19, |(priority, _)| *priority))
    };
    close_process(handle, opened);
    result
}

/// `uv_os_setpriority`.
fn set_priority(pid: i32, priority: i32) -> Result<(), i32> {
    const PROCESS_SET_INFORMATION: u32 = 0x0200;
    if !(-20..=19).contains(&priority) {
        return Err(UV_EINVAL);
    }
    // The class of the highest priority not above the requested one.
    let class = PRIORITIES
        .windows(2)
        .find(|pair| priority < pair[1].0)
        .map_or(PRIORITIES[5].1, |pair| pair[0].1);
    let (handle, opened) = process_handle(pid, PROCESS_SET_INFORMATION)?;
    // SAFETY: the handle is valid until it is closed below.
    let result = if unsafe { SetPriorityClass(handle, class) } == 0 {
        Err(translate(last_error()))
    } else {
        Ok(())
    };
    close_process(handle, opened);
    result
}

/// One unicast address of an interface that is up.
#[derive(Debug, PartialEq)]
pub(crate) struct Interface {
    name: JsStr,
    address: String,
    netmask: String,
    family: &'static str,
    mac: String,
    internal: bool,
    scope: Option<u32>,
}

/// The fields of `IP_ADAPTER_ADDRESSES_LH` that libuv reads.
#[repr(C)]
pub(crate) struct Adapter {
    length: u32,
    index: u32,
    next: *const Adapter,
    adapter_name: *const u8,
    first_unicast: *const Unicast,
    first_anycast: *const c_void,
    first_multicast: *const c_void,
    first_dns_server: *const c_void,
    dns_suffix: *const u16,
    description: *const u16,
    friendly_name: *const u16,
    physical_address: [u8; 8],
    physical_address_length: u32,
    flags: u32,
    mtu: u32,
    interface_type: u32,
    operational_status: u32,
}

/// The fields of `IP_ADAPTER_UNICAST_ADDRESS_LH` that libuv reads.
#[repr(C)]
pub(crate) struct Unicast {
    length: u32,
    flags: u32,
    next: *const Unicast,
    address: *const u8,
    address_length: i32,
    address_padding: u32,
    prefix_origin: u32,
    suffix_origin: u32,
    dad_state: u32,
    valid_lifetime: u32,
    preferred_lifetime: u32,
    lease_lifetime: u32,
    prefix_length: u8,
}

fn query_adapters(buffer: *mut c_void, size: &mut u32) -> u32 {
    const SKIP_ANYCAST_MULTICAST_DNS: u32 = 0x2 | 0x4 | 0x8;
    type Query = unsafe extern "system" fn(u32, u32, *mut c_void, *mut c_void, *mut u32) -> u32;
    let procedure = system_procedure("iphlpapi.dll", c"GetAdaptersAddresses");
    if procedure.is_null() {
        return u32::MAX;
    }
    // SAFETY: the procedure is GetAdaptersAddresses; the buffer is null or
    // writable for `size` bytes.
    unsafe {
        core::mem::transmute::<*const c_void, Query>(procedure)(
            0,
            SKIP_ANYCAST_MULTICAST_DNS,
            core::ptr::null_mut(),
            buffer,
            size,
        )
    }
}

/// `uv_interface_addresses`: every unicast address of every adapter that
/// is up.
fn interfaces() -> Result<Vec<Interface>, i32> {
    let mut size = 0;
    let mut buffer: Vec<u64> = Vec::new();
    loop {
        let pointer = if buffer.is_empty() {
            core::ptr::null_mut()
        } else {
            buffer.as_mut_ptr().cast()
        };
        match query_adapters(pointer, &mut size) {
            0 => break,
            ERROR_BUFFER_OVERFLOW => buffer = vec![0; (size as usize).div_ceil(8)],
            ERROR_NO_DATA => return Ok(Vec::new()),
            ERROR_ADDRESS_NOT_ASSOCIATED => return Err(UV_EAGAIN),
            ERROR_INVALID_PARAMETER => return Err(UV_ENOBUFS),
            u32::MAX => return Err(UV_ENOSYS),
            error => return Err(translate(error)),
        }
    }
    // SAFETY: Windows wrote a linked list of adapters into the buffer.
    Ok(unsafe { adapters(buffer.as_ptr().cast()) })
}

/// The interfaces of a list of adapters.
///
/// # Safety
/// `adapter` is null or the head of a valid `IP_ADAPTER_ADDRESSES` list.
pub(crate) unsafe fn adapters(mut adapter: *const Adapter) -> Vec<Interface> {
    let mut interfaces = Vec::new();
    // SAFETY: the caller guarantees the list and the strings and addresses
    // it points to.
    unsafe {
        while let Some(current) = adapter.as_ref() {
            adapter = current.next;
            if current.operational_status != 1 || current.first_unicast.is_null() {
                continue;
            }
            let mut length = 0;
            while *current.friendly_name.add(length) != 0 {
                length += 1;
            }
            let name = os_text(core::slice::from_raw_parts(current.friendly_name, length));
            let physical = if current.physical_address_length == 6 {
                &current.physical_address[..6]
            } else {
                &[0; 6][..]
            };
            let mac = physical
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(":");
            let mut unicast = current.first_unicast;
            while let Some(address) = unicast.as_ref() {
                unicast = address.next;
                let prefix = u32::from(address.prefix_length);
                let family = u16::from_le_bytes([*address.address, *address.address.add(1)]);
                let (text, netmask, family, scope) = if family == 23 {
                    let bytes: [u8; 16] = core::ptr::read_unaligned(address.address.add(8).cast());
                    let scope = u32::from_le_bytes(core::ptr::read_unaligned(
                        address.address.add(24).cast(),
                    ));
                    let mut mask = [0u8; 16];
                    for (index, byte) in mask.iter_mut().enumerate() {
                        let bits = prefix.saturating_sub(index as u32 * 8).min(8);
                        *byte = (0xff00u16 >> bits) as u8;
                    }
                    (ip6_name(&bytes), ip6_name(&mask), "IPv6", Some(scope))
                } else {
                    let bytes: [u8; 4] = core::ptr::read_unaligned(address.address.add(4).cast());
                    let mask = if prefix > 0 {
                        (u32::MAX << (32 - prefix.min(32))).to_be_bytes()
                    } else {
                        [0; 4]
                    };
                    (ip4_name(&bytes), ip4_name(&mask), "IPv4", None)
                };
                interfaces.push(Interface {
                    name: name.clone(),
                    address: text,
                    netmask,
                    family,
                    mac: mac.clone(),
                    internal: current.interface_type == 24,
                    scope,
                });
            }
        }
    }
    interfaces
}

/// `uv_ip4_name`.
pub(crate) fn ip4_name(bytes: &[u8; 4]) -> String {
    format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3])
}

/// `uv_ip6_name`: the text libuv's `inet_ntop6` writes, compressing the
/// first longest run of two or more zero words.
pub(crate) fn ip6_name(bytes: &[u8; 16]) -> String {
    let words: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
        .collect();
    let mut best: Option<(usize, usize)> = None;
    let mut current: Option<(usize, usize)> = None;
    for (index, word) in words.iter().chain([&1]).enumerate() {
        if *word == 0 {
            current = Some(current.map_or((index, 1), |(start, length)| (start, length + 1)));
        } else if let Some(run) = current.take()
            && best.is_none_or(|(_, length)| run.1 > length)
        {
            best = Some(run);
        }
    }
    let best = best.filter(|(_, length)| *length >= 2);
    let mut text = String::new();
    for (index, word) in words.iter().enumerate() {
        if let Some((start, length)) = best
            && (start..start + length).contains(&index)
        {
            if index == start {
                text.push(':');
            }
            continue;
        }
        if index != 0 {
            text.push(':');
        }
        // An IPv4-compatible or IPv4-mapped address ends in IPv4 text.
        if index == 6
            && best.is_some_and(|(start, length)| {
                start == 0 && (length == 6 || (length == 5 && words[5] == 0xffff))
            })
        {
            text.push_str(&ip4_name(&[bytes[12], bytes[13], bytes[14], bytes[15]]));
            return text;
        }
        text.push_str(&format!("{word:x}"));
    }
    if best.is_some_and(|(start, length)| start + length == 8) {
        text.push(':');
    }
    text
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or_default()
}

/// The context object a binding is given last, for failures.
fn context(args: &[Value]) -> Value {
    args.last().cloned().unwrap_or_default()
}

/// Records a failure on the context and returns `undefined`, as Node's
/// bindings do.
fn fail(realm: &mut Realm, args: &[Value], code: i32, syscall: &str) -> Completion {
    uv_error::collect(realm, &context(args), code, syscall)?;
    Ok(Value::Undefined)
}

/// Bytes of a string, encoded as `userInfo({ encoding })` asks: a Buffer
/// for `buffer`, else text in the encoding, UTF-8 by default.
fn encoded(realm: &mut Realm, text: &JsStr, encoding: &Option<JsStr>) -> Completion {
    let bytes = crate::encoding::encode(text.units(), crate::encoding::Encoding::Utf8);
    match encoding {
        Some(name) if name.to_rust_lossy().eq_ignore_ascii_case("buffer") => {
            // Buffers need the buffer module, which Node always has loaded.
            crate::modules::load(realm, "node:buffer")?;
            let prototype = realm.buffer_prototype.clone();
            Ok(Value::Object(crate::typed_array_builtin::new_bytes(
                realm, bytes, prototype,
            )))
        }
        _ => {
            let encoding = encoding
                .as_ref()
                .and_then(|name| crate::encoding::named(name.units()))
                .unwrap_or(crate::encoding::Encoding::Utf8);
            Ok(Value::String(JsStr::from_units(&crate::encoding::decode(
                &bytes, encoding,
            ))))
        }
    }
}

fn user_info(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let options = arg(args, 0);
    let encoding = if options.as_object().is_some() {
        match ops::get_named(realm, &options, "encoding")? {
            Value::String(name) => Some(name),
            _ => None,
        }
    } else {
        None
    };
    let user = match user() {
        Ok(user) => user,
        Err(code) => return fail(realm, args, code, "uv_os_get_passwd"),
    };
    let info = Object::new(None, crate::Kind::Ordinary);
    let username = encoded(realm, &user.username, &encoding)?;
    let homedir = encoded(realm, &user.homedir, &encoding)?;
    for (key, value) in [
        ("uid", Value::Number(-1.0)),
        ("gid", Value::Number(-1.0)),
        ("username", username),
        ("homedir", homedir),
        ("shell", Value::Null),
    ] {
        info.define(Key::name(key), Property::data(value));
    }
    Ok(Value::Object(info))
}

/// A binding function.
type Native = fn(&mut Realm, Value, &[Value]) -> Completion;

/// `internalBinding('os')`.
pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let functions: [(&str, Native); 13] = [
        ("getHostname", |realm, _, args| match host_name() {
            Ok(name) => Ok(Value::String(name)),
            Err(code) => fail(realm, args, code, "uv_os_gethostname"),
        }),
        ("getLoadAvg", |realm, _, args| {
            // Windows has no load average.
            let array = arg(args, 0);
            for index in 0..3 {
                ops::set(realm, &array, &Key::Index(index), Value::Number(0.0))?;
            }
            Ok(Value::Undefined)
        }),
        ("getUptime", |_, _, _| Ok(Value::Number(uptime()))),
        ("getTotalMem", |_, _, _| {
            Ok(Value::Number(memory().0 as f64))
        }),
        ("getFreeMem", |_, _, _| Ok(Value::Number(memory().1 as f64))),
        ("getCPUs", |realm, _, _| {
            let Some(cpus) = cpus() else {
                return Ok(Value::Undefined);
            };
            let mut values = Vec::new();
            for cpu in cpus {
                // Node reads the model's UTF-8 bytes as Latin-1.
                let bytes = crate::encoding::encode(&cpu.model, crate::encoding::Encoding::Utf8);
                let latin1 = crate::encoding::decode(&bytes, crate::encoding::Encoding::Latin1);
                values.push(Value::String(JsStr::from_units(&latin1)));
                values.push(Value::Number(f64::from(cpu.speed)));
                values.extend(cpu.times.map(Value::Number));
            }
            Ok(Value::Object(realm.array(values)))
        }),
        ("getInterfaceAddresses", |realm, _, args| {
            let interfaces = match interfaces() {
                Ok(interfaces) => interfaces,
                Err(UV_ENOSYS) => return Ok(Value::Undefined),
                Err(code) => return fail(realm, args, code, "uv_interface_addresses"),
            };
            let mut values = Vec::new();
            for interface in interfaces {
                values.extend([
                    Value::String(interface.name),
                    Value::str(&interface.address),
                    Value::str(&interface.netmask),
                    Value::str(interface.family),
                    Value::str(&interface.mac),
                    Value::Boolean(interface.internal),
                    Value::Number(interface.scope.map_or(-1.0, f64::from)),
                ]);
            }
            Ok(Value::Object(realm.array(values)))
        }),
        ("getHomeDirectory", |realm, _, args| {
            match home_directory() {
                Ok(home) => Ok(Value::String(home)),
                Err(code) => fail(realm, args, code, "uv_os_homedir"),
            }
        }),
        ("getUserInfo", user_info),
        ("setPriority", |realm, _, args| {
            let pid = ops::to_int32(realm, &arg(args, 0))?;
            let priority = ops::to_int32(realm, &arg(args, 1))?;
            match set_priority(pid, priority) {
                Ok(()) => Ok(Value::Number(0.0)),
                Err(code) => {
                    fail(realm, args, code, "uv_os_setpriority")?;
                    Ok(Value::Number(f64::from(code)))
                }
            }
        }),
        ("getPriority", |realm, _, args| {
            let pid = ops::to_int32(realm, &arg(args, 0))?;
            match priority(pid) {
                Ok(priority) => Ok(Value::Number(f64::from(priority))),
                Err(code) => fail(realm, args, code, "uv_os_getpriority"),
            }
        }),
        ("getAvailableParallelism", |_, _, _| {
            Ok(Value::Number(f64::from(available_parallelism())))
        }),
        ("getOSInformation", |realm, _, _| {
            let values = uname().map(|text| Value::str(&text)).to_vec();
            Ok(Value::Object(realm.array(values)))
        }),
    ];
    for (name, function) in functions {
        let function = realm.function(name, 0, false, function);
        exports.define(Key::name(name), Property::data(Value::Object(function)));
    }
    exports.define(
        Key::name("isBigEndian"),
        Property::data(Value::Boolean(false)),
    );
    exports
}

/// `os.constants.errno` on Windows, in Node's order.
const ERRNO: &[(&str, i32)] = &[
    ("E2BIG", 7),
    ("EACCES", 13),
    ("EADDRINUSE", 100),
    ("EADDRNOTAVAIL", 101),
    ("EAFNOSUPPORT", 102),
    ("EAGAIN", 11),
    ("EALREADY", 103),
    ("EBADF", 9),
    ("EBADMSG", 104),
    ("EBUSY", 16),
    ("ECANCELED", 105),
    ("ECHILD", 10),
    ("ECONNABORTED", 106),
    ("ECONNREFUSED", 107),
    ("ECONNRESET", 108),
    ("EDEADLK", 36),
    ("EDESTADDRREQ", 109),
    ("EDOM", 33),
    ("EEXIST", 17),
    ("EFAULT", 14),
    ("EFBIG", 27),
    ("EHOSTUNREACH", 110),
    ("EIDRM", 111),
    ("EILSEQ", 42),
    ("EINPROGRESS", 112),
    ("EINTR", 4),
    ("EINVAL", 22),
    ("EIO", 5),
    ("EISCONN", 113),
    ("EISDIR", 21),
    ("ELOOP", 114),
    ("EMFILE", 24),
    ("EMLINK", 31),
    ("EMSGSIZE", 115),
    ("ENAMETOOLONG", 38),
    ("ENETDOWN", 116),
    ("ENETRESET", 117),
    ("ENETUNREACH", 118),
    ("ENFILE", 23),
    ("ENOBUFS", 119),
    ("ENODATA", 120),
    ("ENODEV", 19),
    ("ENOENT", 2),
    ("ENOEXEC", 8),
    ("ENOLCK", 39),
    ("ENOLINK", 121),
    ("ENOMEM", 12),
    ("ENOMSG", 122),
    ("ENOPROTOOPT", 123),
    ("ENOSPC", 28),
    ("ENOSR", 124),
    ("ENOSTR", 125),
    ("ENOSYS", 40),
    ("ENOTCONN", 126),
    ("ENOTDIR", 20),
    ("ENOTEMPTY", 41),
    ("ENOTSOCK", 128),
    ("ENOTSUP", 129),
    ("ENOTTY", 25),
    ("ENXIO", 6),
    ("EOPNOTSUPP", 130),
    ("EOVERFLOW", 132),
    ("EPERM", 1),
    ("EPIPE", 32),
    ("EPROTO", 134),
    ("EPROTONOSUPPORT", 135),
    ("EPROTOTYPE", 136),
    ("ERANGE", 34),
    ("EROFS", 30),
    ("ESPIPE", 29),
    ("ESRCH", 3),
    ("ETIME", 137),
    ("ETIMEDOUT", 138),
    ("ETXTBSY", 139),
    ("EWOULDBLOCK", 140),
    ("EXDEV", 18),
    ("WSAEINTR", 10004),
    ("WSAEBADF", 10009),
    ("WSAEACCES", 10013),
    ("WSAEFAULT", 10014),
    ("WSAEINVAL", 10022),
    ("WSAEMFILE", 10024),
    ("WSAEWOULDBLOCK", 10035),
    ("WSAEINPROGRESS", 10036),
    ("WSAEALREADY", 10037),
    ("WSAENOTSOCK", 10038),
    ("WSAEDESTADDRREQ", 10039),
    ("WSAEMSGSIZE", 10040),
    ("WSAEPROTOTYPE", 10041),
    ("WSAENOPROTOOPT", 10042),
    ("WSAEPROTONOSUPPORT", 10043),
    ("WSAESOCKTNOSUPPORT", 10044),
    ("WSAEOPNOTSUPP", 10045),
    ("WSAEPFNOSUPPORT", 10046),
    ("WSAEAFNOSUPPORT", 10047),
    ("WSAEADDRINUSE", 10048),
    ("WSAEADDRNOTAVAIL", 10049),
    ("WSAENETDOWN", 10050),
    ("WSAENETUNREACH", 10051),
    ("WSAENETRESET", 10052),
    ("WSAECONNABORTED", 10053),
    ("WSAECONNRESET", 10054),
    ("WSAENOBUFS", 10055),
    ("WSAEISCONN", 10056),
    ("WSAENOTCONN", 10057),
    ("WSAESHUTDOWN", 10058),
    ("WSAETOOMANYREFS", 10059),
    ("WSAETIMEDOUT", 10060),
    ("WSAECONNREFUSED", 10061),
    ("WSAELOOP", 10062),
    ("WSAENAMETOOLONG", 10063),
    ("WSAEHOSTDOWN", 10064),
    ("WSAEHOSTUNREACH", 10065),
    ("WSAENOTEMPTY", 10066),
    ("WSAEPROCLIM", 10067),
    ("WSAEUSERS", 10068),
    ("WSAEDQUOT", 10069),
    ("WSAESTALE", 10070),
    ("WSAEREMOTE", 10071),
    ("WSASYSNOTREADY", 10091),
    ("WSAVERNOTSUPPORTED", 10092),
    ("WSANOTINITIALISED", 10093),
    ("WSAEDISCON", 10101),
    ("WSAENOMORE", 10102),
    ("WSAECANCELLED", 10103),
    ("WSAEINVALIDPROCTABLE", 10104),
    ("WSAEINVALIDPROVIDER", 10105),
    ("WSAEPROVIDERFAILEDINIT", 10106),
    ("WSASYSCALLFAILURE", 10107),
    ("WSASERVICE_NOT_FOUND", 10108),
    ("WSATYPE_NOT_FOUND", 10109),
    ("WSA_E_NO_MORE", 10110),
    ("WSA_E_CANCELLED", 10111),
    ("WSAEREFUSED", 10112),
];

/// Signals Node knows on Windows.
const SIGNALS: &[(&str, i32)] = &[
    ("SIGHUP", 1),
    ("SIGINT", 2),
    ("SIGQUIT", 3),
    ("SIGILL", 4),
    ("SIGABRT", 22),
    ("SIGFPE", 8),
    ("SIGKILL", 9),
    ("SIGSEGV", 11),
    ("SIGTERM", 15),
    ("SIGBREAK", 21),
    ("SIGWINCH", 28),
];

/// The signal number for Node's Windows signal name.
pub(crate) fn signal_number(name: &str) -> Option<i32> {
    SIGNALS
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, number)| *number)
}

/// Whether Node exposes this numeric signal on Windows.
pub(crate) fn signal_number_by_value(number: i32) -> Option<&'static str> {
    SIGNALS
        .iter()
        .find(|(_, known)| *known == number)
        .map(|(name, _)| *name)
}

/// Node's named priorities.
const PRIORITY_NAMES: &[(&str, i32)] = &[
    ("PRIORITY_LOW", 19),
    ("PRIORITY_BELOW_NORMAL", 10),
    ("PRIORITY_NORMAL", 0),
    ("PRIORITY_ABOVE_NORMAL", -7),
    ("PRIORITY_HIGH", -14),
    ("PRIORITY_HIGHEST", -20),
];

/// A null-prototype object of named numbers, as Node's constants are.
fn numbers(table: &[(&str, i32)]) -> Object {
    let object = Object::new(None, crate::Kind::Ordinary);
    for (name, value) in table {
        object.define(
            Key::name(name),
            Property::data(Value::Number(f64::from(*value))),
        );
    }
    object
}

// Node v22.15.0 constants exported by its Windows filesystem binding.
const FS_CONSTANTS: &[(&str, i32)] = &[
    ("UV_FS_SYMLINK_DIR", 1),
    ("UV_FS_SYMLINK_JUNCTION", 2),
    ("O_RDONLY", 0),
    ("O_WRONLY", 1),
    ("O_RDWR", 2),
    ("UV_DIRENT_UNKNOWN", 0),
    ("UV_DIRENT_FILE", 1),
    ("UV_DIRENT_DIR", 2),
    ("UV_DIRENT_LINK", 3),
    ("UV_DIRENT_FIFO", 4),
    ("UV_DIRENT_SOCKET", 5),
    ("UV_DIRENT_CHAR", 6),
    ("UV_DIRENT_BLOCK", 7),
    ("EXTENSIONLESS_FORMAT_JAVASCRIPT", 0),
    ("EXTENSIONLESS_FORMAT_WASM", 1),
    ("S_IFMT", 61440),
    ("S_IFREG", 32768),
    ("S_IFDIR", 16384),
    ("S_IFCHR", 8192),
    ("S_IFIFO", 4096),
    ("S_IFLNK", 40960),
    ("O_CREAT", 256),
    ("O_EXCL", 1024),
    ("UV_FS_O_FILEMAP", 536870912),
    ("O_TRUNC", 512),
    ("O_APPEND", 8),
    ("S_IRUSR", 256),
    ("S_IWUSR", 128),
    ("F_OK", 0),
    ("R_OK", 4),
    ("W_OK", 2),
    ("X_OK", 1),
    ("UV_FS_COPYFILE_EXCL", 1),
    ("COPYFILE_EXCL", 1),
    ("UV_FS_COPYFILE_FICLONE", 2),
    ("COPYFILE_FICLONE", 2),
    ("UV_FS_COPYFILE_FICLONE_FORCE", 4),
    ("COPYFILE_FICLONE_FORCE", 4),
];

/// Node 22's zlib constants, as `internalBinding('constants').zlib` gives
/// them to `zlib` and the web stream adapters.
const ZLIB: &[(&str, f64)] = &[
    ("Z_NO_FLUSH", 0.0),
    ("Z_PARTIAL_FLUSH", 1.0),
    ("Z_SYNC_FLUSH", 2.0),
    ("Z_FULL_FLUSH", 3.0),
    ("Z_FINISH", 4.0),
    ("Z_BLOCK", 5.0),
    ("Z_OK", 0.0),
    ("Z_STREAM_END", 1.0),
    ("Z_NEED_DICT", 2.0),
    ("Z_ERRNO", -1.0),
    ("Z_STREAM_ERROR", -2.0),
    ("Z_DATA_ERROR", -3.0),
    ("Z_MEM_ERROR", -4.0),
    ("Z_BUF_ERROR", -5.0),
    ("Z_VERSION_ERROR", -6.0),
    ("Z_NO_COMPRESSION", 0.0),
    ("Z_BEST_SPEED", 1.0),
    ("Z_BEST_COMPRESSION", 9.0),
    ("Z_DEFAULT_COMPRESSION", -1.0),
    ("Z_FILTERED", 1.0),
    ("Z_HUFFMAN_ONLY", 2.0),
    ("Z_RLE", 3.0),
    ("Z_FIXED", 4.0),
    ("Z_DEFAULT_STRATEGY", 0.0),
    ("ZLIB_VERNUM", 4865.0),
    ("DEFLATE", 1.0),
    ("INFLATE", 2.0),
    ("GZIP", 3.0),
    ("GUNZIP", 4.0),
    ("DEFLATERAW", 5.0),
    ("INFLATERAW", 6.0),
    ("UNZIP", 7.0),
    ("BROTLI_DECODE", 8.0),
    ("BROTLI_ENCODE", 9.0),
    ("ZSTD_DECOMPRESS", 11.0),
    ("ZSTD_COMPRESS", 10.0),
    ("Z_MIN_WINDOWBITS", 8.0),
    ("Z_MAX_WINDOWBITS", 15.0),
    ("Z_DEFAULT_WINDOWBITS", 15.0),
    ("Z_MIN_CHUNK", 64.0),
    ("Z_MAX_CHUNK", f64::INFINITY),
    ("Z_DEFAULT_CHUNK", 16384.0),
    ("Z_MIN_MEMLEVEL", 1.0),
    ("Z_MAX_MEMLEVEL", 9.0),
    ("Z_DEFAULT_MEMLEVEL", 8.0),
    ("Z_MIN_LEVEL", -1.0),
    ("Z_MAX_LEVEL", 9.0),
    ("Z_DEFAULT_LEVEL", -1.0),
    ("BROTLI_OPERATION_PROCESS", 0.0),
    ("BROTLI_OPERATION_FLUSH", 1.0),
    ("BROTLI_OPERATION_FINISH", 2.0),
    ("BROTLI_OPERATION_EMIT_METADATA", 3.0),
    ("BROTLI_PARAM_MODE", 0.0),
    ("BROTLI_MODE_GENERIC", 0.0),
    ("BROTLI_MODE_TEXT", 1.0),
    ("BROTLI_MODE_FONT", 2.0),
    ("BROTLI_DEFAULT_MODE", 0.0),
    ("BROTLI_PARAM_QUALITY", 1.0),
    ("BROTLI_MIN_QUALITY", 0.0),
    ("BROTLI_MAX_QUALITY", 11.0),
    ("BROTLI_DEFAULT_QUALITY", 11.0),
    ("BROTLI_PARAM_LGWIN", 2.0),
    ("BROTLI_MIN_WINDOW_BITS", 10.0),
    ("BROTLI_MAX_WINDOW_BITS", 24.0),
    ("BROTLI_LARGE_MAX_WINDOW_BITS", 30.0),
    ("BROTLI_DEFAULT_WINDOW", 22.0),
    ("BROTLI_PARAM_LGBLOCK", 3.0),
    ("BROTLI_MIN_INPUT_BLOCK_BITS", 16.0),
    ("BROTLI_MAX_INPUT_BLOCK_BITS", 24.0),
    ("BROTLI_PARAM_DISABLE_LITERAL_CONTEXT_MODELING", 4.0),
    ("BROTLI_PARAM_SIZE_HINT", 5.0),
    ("BROTLI_PARAM_LARGE_WINDOW", 6.0),
    ("BROTLI_PARAM_NPOSTFIX", 7.0),
    ("BROTLI_PARAM_NDIRECT", 8.0),
    ("BROTLI_DECODER_RESULT_ERROR", 0.0),
    ("BROTLI_DECODER_RESULT_SUCCESS", 1.0),
    ("BROTLI_DECODER_RESULT_NEEDS_MORE_INPUT", 2.0),
    ("BROTLI_DECODER_RESULT_NEEDS_MORE_OUTPUT", 3.0),
    ("BROTLI_DECODER_PARAM_DISABLE_RING_BUFFER_REALLOCATION", 0.0),
    ("BROTLI_DECODER_PARAM_LARGE_WINDOW", 1.0),
    ("BROTLI_DECODER_NO_ERROR", 0.0),
    ("BROTLI_DECODER_SUCCESS", 1.0),
    ("BROTLI_DECODER_NEEDS_MORE_INPUT", 2.0),
    ("BROTLI_DECODER_NEEDS_MORE_OUTPUT", 3.0),
    ("BROTLI_DECODER_ERROR_FORMAT_EXUBERANT_NIBBLE", -1.0),
    ("BROTLI_DECODER_ERROR_FORMAT_RESERVED", -2.0),
    ("BROTLI_DECODER_ERROR_FORMAT_EXUBERANT_META_NIBBLE", -3.0),
    ("BROTLI_DECODER_ERROR_FORMAT_SIMPLE_HUFFMAN_ALPHABET", -4.0),
    ("BROTLI_DECODER_ERROR_FORMAT_SIMPLE_HUFFMAN_SAME", -5.0),
    ("BROTLI_DECODER_ERROR_FORMAT_CL_SPACE", -6.0),
    ("BROTLI_DECODER_ERROR_FORMAT_HUFFMAN_SPACE", -7.0),
    ("BROTLI_DECODER_ERROR_FORMAT_CONTEXT_MAP_REPEAT", -8.0),
    ("BROTLI_DECODER_ERROR_FORMAT_BLOCK_LENGTH_1", -9.0),
    ("BROTLI_DECODER_ERROR_FORMAT_BLOCK_LENGTH_2", -10.0),
    ("BROTLI_DECODER_ERROR_FORMAT_TRANSFORM", -11.0),
    ("BROTLI_DECODER_ERROR_FORMAT_DICTIONARY", -12.0),
    ("BROTLI_DECODER_ERROR_FORMAT_WINDOW_BITS", -13.0),
    ("BROTLI_DECODER_ERROR_FORMAT_PADDING_1", -14.0),
    ("BROTLI_DECODER_ERROR_FORMAT_PADDING_2", -15.0),
    ("BROTLI_DECODER_ERROR_FORMAT_DISTANCE", -16.0),
    ("BROTLI_DECODER_ERROR_DICTIONARY_NOT_SET", -19.0),
    ("BROTLI_DECODER_ERROR_INVALID_ARGUMENTS", -20.0),
    ("BROTLI_DECODER_ERROR_ALLOC_CONTEXT_MODES", -21.0),
    ("BROTLI_DECODER_ERROR_ALLOC_TREE_GROUPS", -22.0),
    ("BROTLI_DECODER_ERROR_ALLOC_CONTEXT_MAP", -25.0),
    ("BROTLI_DECODER_ERROR_ALLOC_RING_BUFFER_1", -26.0),
    ("BROTLI_DECODER_ERROR_ALLOC_RING_BUFFER_2", -27.0),
    ("BROTLI_DECODER_ERROR_ALLOC_BLOCK_TYPE_TREES", -30.0),
    ("BROTLI_DECODER_ERROR_UNREACHABLE", -31.0),
    ("ZSTD_e_continue", 0.0),
    ("ZSTD_e_flush", 1.0),
    ("ZSTD_e_end", 2.0),
    ("ZSTD_fast", 1.0),
    ("ZSTD_dfast", 2.0),
    ("ZSTD_greedy", 3.0),
    ("ZSTD_lazy", 4.0),
    ("ZSTD_lazy2", 5.0),
    ("ZSTD_btlazy2", 6.0),
    ("ZSTD_btopt", 7.0),
    ("ZSTD_btultra", 8.0),
    ("ZSTD_btultra2", 9.0),
    ("ZSTD_c_compressionLevel", 100.0),
    ("ZSTD_c_windowLog", 101.0),
    ("ZSTD_c_hashLog", 102.0),
    ("ZSTD_c_chainLog", 103.0),
    ("ZSTD_c_searchLog", 104.0),
    ("ZSTD_c_minMatch", 105.0),
    ("ZSTD_c_targetLength", 106.0),
    ("ZSTD_c_strategy", 107.0),
    ("ZSTD_c_enableLongDistanceMatching", 160.0),
    ("ZSTD_c_ldmHashLog", 161.0),
    ("ZSTD_c_ldmMinMatch", 162.0),
    ("ZSTD_c_ldmBucketSizeLog", 163.0),
    ("ZSTD_c_ldmHashRateLog", 164.0),
    ("ZSTD_c_contentSizeFlag", 200.0),
    ("ZSTD_c_checksumFlag", 201.0),
    ("ZSTD_c_dictIDFlag", 202.0),
    ("ZSTD_c_nbWorkers", 400.0),
    ("ZSTD_c_jobSize", 401.0),
    ("ZSTD_c_overlapLog", 402.0),
    ("ZSTD_d_windowLogMax", 100.0),
    ("ZSTD_CLEVEL_DEFAULT", 3.0),
    ("ZSTD_error_no_error", 0.0),
    ("ZSTD_error_GENERIC", 1.0),
    ("ZSTD_error_prefix_unknown", 10.0),
    ("ZSTD_error_version_unsupported", 12.0),
    ("ZSTD_error_frameParameter_unsupported", 14.0),
    ("ZSTD_error_frameParameter_windowTooLarge", 16.0),
    ("ZSTD_error_corruption_detected", 20.0),
    ("ZSTD_error_checksum_wrong", 22.0),
    ("ZSTD_error_literals_headerWrong", 24.0),
    ("ZSTD_error_dictionary_corrupted", 30.0),
    ("ZSTD_error_dictionary_wrong", 32.0),
    ("ZSTD_error_dictionaryCreation_failed", 34.0),
    ("ZSTD_error_parameter_unsupported", 40.0),
    ("ZSTD_error_parameter_combination_unsupported", 41.0),
    ("ZSTD_error_parameter_outOfBound", 42.0),
    ("ZSTD_error_tableLog_tooLarge", 44.0),
    ("ZSTD_error_maxSymbolValue_tooLarge", 46.0),
    ("ZSTD_error_maxSymbolValue_tooSmall", 48.0),
    ("ZSTD_error_stabilityCondition_notRespected", 50.0),
    ("ZSTD_error_stage_wrong", 60.0),
    ("ZSTD_error_init_missing", 62.0),
    ("ZSTD_error_memory_allocation", 64.0),
    ("ZSTD_error_workSpace_tooSmall", 66.0),
    ("ZSTD_error_dstSize_tooSmall", 70.0),
    ("ZSTD_error_srcSize_wrong", 72.0),
    ("ZSTD_error_dstBuffer_null", 74.0),
    ("ZSTD_error_noForwardProgress_destFull", 80.0),
    ("ZSTD_error_noForwardProgress_inputEmpty", 82.0),
];

/// `internalBinding('constants')`: Windows OS, filesystem and zlib
/// constants.
pub(crate) fn constants() -> Object {
    let os = Object::new(None, crate::Kind::Ordinary);
    let dlopen = Object::new(None, crate::Kind::Ordinary);
    for (name, value) in [
        ("UV_UDP_REUSEADDR", Value::Number(4.0)),
        ("dlopen", Value::Object(dlopen)),
        ("errno", Value::Object(numbers(ERRNO))),
        ("signals", Value::Object(numbers(SIGNALS))),
        ("priority", Value::Object(numbers(PRIORITY_NAMES))),
    ] {
        os.define(Key::name(name), Property::data(value));
    }
    let exports = Object::new(None, crate::Kind::Ordinary);
    exports.define(Key::name("os"), Property::data(Value::Object(os)));
    exports.define(
        Key::name("fs"),
        Property::data(Value::Object(fs_constants())),
    );
    let zlib = Object::new(None, crate::Kind::Ordinary);
    for (name, value) in ZLIB {
        zlib.define(Key::name(name), Property::data(Value::Number(*value)));
    }
    exports.define(Key::name("zlib"), Property::data(Value::Object(zlib)));
    exports
}

/// The public `constants` module exposes filesystem flag values at its top
/// level, as well as under `fs` in `internalBinding('constants')`.
pub(crate) fn fs_constants() -> Object {
    numbers(FS_CONSTANTS)
}

/// Public `require('constants')` is an ordinary JavaScript object. Node's
/// `internalBinding('constants').fs` remains null-prototype, but public
/// consumers such as graceful-fs rely on Object.prototype methods.
pub(crate) fn fs_constants_module(realm: &mut Realm) -> Object {
    let exports = realm.object();
    for (name, value) in FS_CONSTANTS {
        exports.define(
            Key::name(name),
            Property::data(Value::Number(f64::from(*value))),
        );
    }
    exports
}

/// `internalBinding('credentials')` on Windows: `safeGetenv`.
pub(crate) fn credentials(realm: &mut Realm) -> Object {
    let exports = realm.object();
    let get = realm.function("safeGetenv", 1, false, |realm, _, args| {
        let name = ops::to_string(realm, &arg(args, 0))?;
        Ok(platform::environment_value(&name).map_or(Value::Undefined, Value::String))
    });
    exports.define(Key::name("safeGetenv"), Property::data(Value::Object(get)));
    exports.define(
        Key::name("implementsPosixCredentials"),
        Property::data(Value::Boolean(false)),
    );
    exports
}

#[cfg(test)]
mod tests;
