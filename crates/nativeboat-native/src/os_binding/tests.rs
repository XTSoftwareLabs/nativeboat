//! The binding against this machine, and its failure paths with mocked
//! Windows queries.

use super::*;
use crate::ErrorKind;

fn call(realm: &mut Realm, binding: &Object, name: &str, args: &[Value]) -> Completion {
    let function = ops::get(realm, &Value::Object(binding.clone()), &Key::name(name)).unwrap();
    ops::call(realm, &function, Value::Undefined, args)
}

fn show(realm: &mut Realm, value: &Value) -> String {
    crate::inspect::inspect(realm, value, 2).unwrap()
}

/// A context object; it has no prototype so it needs no realm.
fn context() -> Value {
    Value::Object(Object::new(None, crate::Kind::Ordinary))
}

#[test]
fn ip_addresses_are_written_like_libuv() {
    assert_eq!(ip4_name(&[192, 168, 0, 61]), "192.168.0.61");
    let ip6 = |words: [u16; 8]| {
        let mut bytes = [0u8; 16];
        for (index, word) in words.iter().enumerate() {
            bytes[index * 2..index * 2 + 2].copy_from_slice(&word.to_be_bytes());
        }
        ip6_name(&bytes)
    };
    for (words, text) in [
        ([0, 0, 0, 0, 0, 0, 0, 0], "::"),
        ([0, 0, 0, 0, 0, 0, 0, 1], "::1"),
        ([1, 0, 0, 0, 0, 0, 0, 0], "1::"),
        (
            [0xfe80, 0, 0, 0, 0x179e, 0xb19, 0x4b2, 0x725c],
            "fe80::179e:b19:4b2:725c",
        ),
        ([1, 0, 0, 2, 0, 0, 0, 3], "1:0:0:2::3"),
        ([1, 0, 0, 2, 0, 0, 3, 4], "1::2:0:0:3:4"),
        ([1, 0, 1, 0, 1, 0, 1, 0], "1:0:1:0:1:0:1:0"),
        ([0, 0, 0, 0, 0, 0xffff, 0x0102, 0x0304], "::ffff:1.2.3.4"),
        ([0, 0, 0, 0, 0, 0, 0x0102, 0x0304], "::1.2.3.4"),
        ([0, 0, 0, 0, 0, 0xfffe, 0x0102, 0x0304], "::fffe:102:304"),
        ([0xffff; 8], "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"),
    ] {
        assert_eq!(ip6(words), text, "{words:?}");
    }
}

/// A socket address as Windows lays it out.
fn socket_address(family: u16, address: &[u8], scope: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; 28];
    bytes[..2].copy_from_slice(&family.to_le_bytes());
    if family == 23 {
        bytes[8..24].copy_from_slice(address);
        bytes[24..28].copy_from_slice(&scope.to_le_bytes());
    } else {
        bytes[4..8].copy_from_slice(address);
    }
    bytes
}

fn unicast(address: &[u8], prefix: u8, next: *const Unicast) -> Unicast {
    Unicast {
        length: 0,
        flags: 0,
        next,
        address: address.as_ptr(),
        address_length: address.len() as i32,
        address_padding: 0,
        prefix_origin: 0,
        suffix_origin: 0,
        dad_state: 0,
        valid_lifetime: 0,
        preferred_lifetime: 0,
        lease_lifetime: 0,
        prefix_length: prefix,
    }
}

fn adapter(
    name: &[u16],
    first: *const Unicast,
    up: bool,
    mac: usize,
    next: *const Adapter,
) -> Adapter {
    Adapter {
        length: 0,
        index: 0,
        next,
        adapter_name: core::ptr::null(),
        first_unicast: first,
        first_anycast: core::ptr::null(),
        first_multicast: core::ptr::null(),
        first_dns_server: core::ptr::null(),
        dns_suffix: core::ptr::null(),
        description: core::ptr::null(),
        friendly_name: name.as_ptr(),
        physical_address: [0x80, 0x2b, 0xf9, 0xad, 0x9c, 0x33, 0, 0],
        physical_address_length: mac as u32,
        flags: 0,
        mtu: 0,
        interface_type: if mac == 6 { 6 } else { 24 },
        operational_status: if up { 1 } else { 2 },
    }
}

#[test]
fn adapters_list_unicast_addresses_of_interfaces_that_are_up() {
    let wlan: Vec<u16> = "WLAN 2\0".encode_utf16().collect();
    let loopback: Vec<u16> = "Loopback\0".encode_utf16().collect();
    let v4 = socket_address(2, &[192, 168, 0, 61], 0);
    let v4_host = socket_address(2, &[10, 0, 0, 1], 0);
    let mut link_local = [0u8; 16];
    link_local[0] = 0xfe;
    link_local[1] = 0x80;
    link_local[15] = 1;
    let v6 = socket_address(23, &link_local, 58);
    let third = unicast(&v4_host, 0, core::ptr::null());
    let second = unicast(&v6, 61, &third);
    let first = unicast(&v4, 20, &second);
    let loop_address = unicast(&v4, 32, core::ptr::null());
    let down = adapter(&wlan, &first, false, 6, core::ptr::null());
    let empty = adapter(&wlan, core::ptr::null(), true, 6, &down);
    let internal = adapter(&loopback, &loop_address, true, 0, &empty);
    let head = adapter(&wlan, &first, true, 6, &internal);
    // SAFETY: the list and everything it points to live until the end.
    let interfaces = unsafe { adapters(&head) };
    let shown: Vec<String> = interfaces
        .iter()
        .map(|interface| {
            format!(
                "{} {} {} {} {} {} {:?}",
                interface.name.to_rust_lossy(),
                interface.address,
                interface.netmask,
                interface.family,
                interface.mac,
                interface.internal,
                interface.scope
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            "WLAN 2 192.168.0.61 255.255.240.0 IPv4 80:2b:f9:ad:9c:33 false None",
            "WLAN 2 fe80::1 ffff:ffff:ffff:fff8:: IPv6 80:2b:f9:ad:9c:33 false Some(58)",
            "WLAN 2 10.0.0.1 0.0.0.0 IPv4 80:2b:f9:ad:9c:33 false None",
            "Loopback 192.168.0.61 255.255.255.255 IPv4 00:00:00:00:00:00 true None",
        ]
    );
}

#[test]
fn adapter_queries_grow_their_buffer_and_map_failures() {
    let mut session = shimforge::Session::new();
    let query = shimforge::mock!(session, query_adapters, fn(*mut c_void, &mut u32) -> u32);
    let calls = std::sync::Arc::new(std::sync::Mutex::new(0));
    let seen = calls.clone();
    query.expect().returning(move |buffer, size| {
        let mut calls = seen.lock().unwrap();
        *calls += 1;
        match *calls {
            1 => {
                assert!(buffer.is_null());
                *size = 300;
                ERROR_BUFFER_OVERFLOW
            }
            2 => {
                assert!(!buffer.is_null());
                0
            }
            3 => ERROR_NO_DATA,
            4 => ERROR_ADDRESS_NOT_ASSOCIATED,
            5 => ERROR_INVALID_PARAMETER,
            6 => u32::MAX,
            _ => 5,
        }
    });
    // A zeroed adapter is not up.
    assert_eq!(interfaces(), Ok(Vec::new()));
    assert_eq!(interfaces(), Ok(Vec::new()));
    assert_eq!(interfaces(), Err(UV_EAGAIN));
    assert_eq!(interfaces(), Err(UV_ENOBUFS));
    assert_eq!(interfaces(), Err(UV_ENOSYS));
    assert_eq!(interfaces(), Err(-4048));
}

#[test]
fn missing_system_functions_are_reported() {
    assert!(system_procedure("nativeboat-missing.dll", c"Anything").is_null());
    assert!(system_procedure("kernel32.dll", c"NativeboatMissing").is_null());
    let mut session = shimforge::Session::new();
    shimforge::mock!(
        session,
        system_procedure,
        fn(&str, &core::ffi::CStr) -> *const c_void
    )
    .expect()
    .returning(|_, _| core::ptr::null());
    assert_eq!(query_processor_times(&mut []), -1);
    assert_eq!(query_profile(core::ptr::null_mut(), &mut [], &mut 0), None);
    assert_eq!(query_user_name(&mut [0; 4], &mut 4), None);
    assert_eq!(query_adapters(core::ptr::null_mut(), &mut 0), u32::MAX);
    // SAFETY: OSVERSIONINFOW is plain data.
    let mut info: VersionInfo = unsafe { core::mem::zeroed() };
    query_version(&mut info);
    assert_eq!(info.major, 0);
    assert_eq!(profile_directory(core::ptr::null_mut()), Err(UV_ENOSYS));
    assert_eq!(user_name(), Err(UV_ENOSYS));
}

#[test]
fn machine_queries_answer_like_node() {
    let (total, free) = memory();
    assert!(total > free && free > 0);
    assert!(available_parallelism() >= 1);
    assert!(uptime() > 0.0);
    let cpus = cpus().unwrap();
    assert_eq!(cpus.len(), system_info().processors as usize);
    assert!(!cpus[0].model.is_empty() && cpus[0].speed > 0);
    let host = host_name().unwrap();
    assert!(!host.is_empty());
    let user = user().unwrap();
    assert!(!user.username.is_empty());
    assert!(!user.homedir.is_empty());
    let [system, version, release, machine] = uname();
    assert_eq!(system, "Windows_NT");
    assert!(version.starts_with("Windows"));
    assert_eq!(release.split('.').count(), 3);
    assert_eq!(machine, "x86_64");
    assert_eq!(priority(0), Ok(0));
    assert_eq!(set_priority(0, 0), Ok(()));
    assert_eq!(priority(i32::MAX), Err(UV_ESRCH));
    // Other processes are opened and closed again.
    let own = std::process::id() as i32;
    assert_eq!(priority(own), Ok(0));
    assert_eq!(set_priority(own, 0), Ok(()));
    assert_eq!(set_priority(0, 20), Err(UV_EINVAL));
    // The System process cannot be changed.
    assert_eq!(set_priority(4, 0), Err(-4048));
}

#[test]
fn failed_machine_queries_fall_back_like_libuv() {
    let mut session = shimforge::Session::new();
    shimforge::mock!(session, query_memory, fn(&mut MemoryStatus) -> i32)
        .expect()
        .returning(|_| 0);
    assert_eq!(memory(), (0, 0));
    shimforge::mock!(session, affinity, fn() -> usize)
        .expect()
        .returning(|| 0);
    assert_eq!(available_parallelism(), 1);
    shimforge::mock!(session, query_host_name, fn(&mut [u16]) -> i32)
        .expect()
        .returning(|_| 0);
    shimforge::mock!(session, last_error, fn() -> u32)
        .expect()
        .returning(|| 8);
    assert_eq!(host_name(), Err(UV_ENOMEM));
    shimforge::mock!(
        session,
        query_processor_times,
        fn(&mut [ProcessorTimes]) -> i32
    )
    .expect()
    .returning(|_| 1);
    assert!(cpus().is_none());
}

#[test]
fn processor_registry_failures_mean_no_processors() {
    let mut session = shimforge::Session::new();
    shimforge::mock!(session, registry, fn(&str, &str) -> Result<Vec<u8>, u32>)
        .expect()
        .returning(|_, name| if name == "~MHz" { Ok(vec![1]) } else { Err(2) });
    assert!(cpus().is_none());
}

#[test]
fn missing_registry_keys_and_values_are_errors() {
    assert_eq!(registry("SOFTWARE\\NativeboatMissing", "Value"), Err(2));
    assert_eq!(
        registry(
            "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
            "NativeboatMissing"
        ),
        Err(2)
    );
}

#[test]
fn speeds_are_read_from_four_bytes() {
    let mut session = shimforge::Session::new();
    shimforge::mock!(session, registry, fn(&str, &str) -> Result<Vec<u8>, u32>)
        .expect()
        .returning(|_, name| {
            Ok(if name == "~MHz" {
                vec![1]
            } else {
                "Model\0"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect()
            })
        });
    let cpus = cpus().unwrap();
    assert_eq!(cpus[0].speed, 0);
    assert_eq!(String::from_utf16_lossy(&cpus[0].model), "Model");
}

#[test]
fn versions_follow_libuv_rules() {
    let mut session = shimforge::Session::new();
    shimforge::mock!(session, query_version, fn(&mut VersionInfo))
        .expect()
        .returning(|info| {
            info.major = 10;
            info.build = 22631;
            info.service_pack[..3].copy_from_slice(&[83, 80, 49]);
        });
    let product = std::sync::Arc::new(std::sync::Mutex::new(Ok(
        b"W\0i\0n\0d\0o\0w\0s\0 \x001\x000\0 \0P\0r\0o\0".to_vec(),
    )));
    let source = product.clone();
    shimforge::mock!(session, registry, fn(&str, &str) -> Result<Vec<u8>, u32>)
        .expect()
        .returning(move |_, _| source.lock().unwrap().clone());
    assert_eq!(uname()[1], "Windows 11 Pro SP1");
    *product.lock().unwrap() = Err(2);
    assert_eq!(uname()[1], "SP1");
    let architectures = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let pending = architectures.clone();
    shimforge::mock!(session, system_info, fn() -> SystemInfo)
        .expect()
        .returning(move || {
            let (architecture, level) = pending.lock().unwrap().remove(0);
            // SAFETY: SYSTEM_INFO is plain data.
            let mut info: SystemInfo = unsafe { core::mem::zeroed() };
            info.processor_architecture = architecture;
            info.processor_level = level;
            info
        });
    for (architecture, level, machine) in [
        (9, 0, "x86_64"),
        (6, 0, "ia64"),
        (0, 3, "i386"),
        (0, 5, "i586"),
        (0, 15, "i686"),
        (10, 0, "i686"),
        (1, 0, "mips"),
        (2, 0, "alpha"),
        (7, 0, "alpha"),
        (3, 0, "powerpc"),
        (4, 0, "sh"),
        (5, 0, "arm"),
        (12, 0, "unknown"),
    ] {
        architectures.lock().unwrap().push((architecture, level));
        assert_eq!(uname()[3], machine);
    }
}

#[test]
fn user_queries_map_failures() {
    let mut session = shimforge::Session::new();
    let errors = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let pending = errors.clone();
    shimforge::mock!(session, last_error, fn() -> u32)
        .expect()
        .returning(move || pending.lock().unwrap().remove(0));
    // A profile query that does not ask for a larger buffer.
    errors.lock().unwrap().push(5);
    assert_eq!(profile_directory(core::ptr::null_mut()), Err(-4048));
    let answers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let pending = answers.clone();
    shimforge::mock!(
        session,
        query_profile,
        fn(*mut c_void, &mut [u16], &mut u32) -> Option<i32>
    )
    .expect()
    .returning(move |_, _, size| {
        *size = 4;
        pending.lock().unwrap().remove(0)
    });
    answers.lock().unwrap().extend([Some(0), Some(0)]);
    errors
        .lock()
        .unwrap()
        .extend([ERROR_INSUFFICIENT_BUFFER, 2]);
    assert_eq!(profile_directory(core::ptr::null_mut()), Err(UV_ENOENT));
    shimforge::mock!(
        session,
        query_user_name,
        fn(&mut [u16], &mut u32) -> Option<i32>
    )
    .expect()
    .returning(|_, _| Some(0));
    errors
        .lock()
        .unwrap()
        .extend([ERROR_INSUFFICIENT_BUFFER, 5]);
    assert_eq!(user_name(), Err(UV_ENOMEM));
    assert_eq!(user_name(), Err(-4048));
    shimforge::mock!(session, open_token, fn() -> Result<*mut c_void, i32>)
        .expect()
        .returning(|| Err(-4048));
    assert!(matches!(user(), Err(-4048)));
}

#[test]
fn tokens_and_process_handles_report_windows_errors() {
    let mut session = shimforge::Session::new();
    shimforge::mock!(
        session,
        OpenProcessToken,
        unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32
    )
    .expect()
    .returning(|_, _, _| 0);
    shimforge::mock!(session, last_error, fn() -> u32)
        .expect()
        .returning(|| 5);
    assert_eq!(open_token(), Err(-4048));
    shimforge::mock!(
        session,
        GetPriorityClass,
        unsafe extern "system" fn(*mut c_void) -> u32
    )
    .expect()
    .returning(|_| 0);
    assert_eq!(priority(0), Err(-4048));
    shimforge::mock!(
        session,
        SetPriorityClass,
        unsafe extern "system" fn(*mut c_void, u32) -> i32
    )
    .expect()
    .returning(|_, _| 0);
    assert_eq!(set_priority(0, 0), Err(-4048));
    shimforge::mock!(
        session,
        GetProcessAffinityMask,
        unsafe extern "system" fn(*mut c_void, *mut usize, *mut usize) -> i32
    )
    .expect()
    .returning(|_, _, _| 0);
    assert_eq!(affinity(), 0);
}

#[test]
fn priorities_map_to_windows_classes_like_libuv() {
    let mut session = shimforge::Session::new();
    let classes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = classes.clone();
    shimforge::mock!(
        session,
        SetPriorityClass,
        unsafe extern "system" fn(*mut c_void, u32) -> i32
    )
    .expect()
    .returning(move |_, class| {
        seen.lock().unwrap().push(class);
        1
    });
    for priority in [-20, -15, -14, -8, -7, -1, 0, 9, 10, 18, 19] {
        set_priority(0, priority).unwrap();
    }
    assert_eq!(
        *classes.lock().unwrap(),
        [
            0x100, 0x100, 0x80, 0x80, 0x8000, 0x8000, 0x20, 0x20, 0x4000, 0x4000, 0x40
        ]
    );
    let class = std::sync::Arc::new(std::sync::Mutex::new(0x100));
    let current = class.clone();
    shimforge::mock!(
        session,
        GetPriorityClass,
        unsafe extern "system" fn(*mut c_void) -> u32
    )
    .expect()
    .returning(move |_| *current.lock().unwrap());
    for (windows, expected) in [
        (0x100, -20),
        (0x80, -14),
        (0x8000, -7),
        (0x20, 0),
        (0x4000, 10),
        (0x40, 19),
        (7, 19),
    ] {
        *class.lock().unwrap() = windows;
        assert_eq!(priority(0), Ok(expected));
    }
}

#[test]
fn home_directories_prefer_userprofile() {
    let mut session = shimforge::Session::new();
    let values = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Option<&str>>::new()));
    let pending = values.clone();
    shimforge::mock!(
        session,
        platform::environment_value,
        fn(&JsStr) -> Option<JsStr>
    )
    .expect()
    .returning(move |_| pending.lock().unwrap().remove(0).map(JsStr::from));
    values
        .lock()
        .unwrap()
        .extend([Some("C:"), Some("D:\\home"), None]);
    assert_eq!(home_directory(), Err(UV_ENOENT));
    assert_eq!(home_directory(), Ok(JsStr::from("D:\\home")));
    assert_eq!(home_directory(), Ok(user().unwrap().homedir));
}

#[test]
fn binding_functions_report_through_their_context() {
    let mut realm = Realm::new();
    let binding = binding(&mut realm);
    for name in [
        "getUptime",
        "getTotalMem",
        "getFreeMem",
        "getAvailableParallelism",
    ] {
        let value = call(&mut realm, &binding, name, &[]).unwrap();
        assert!(
            matches!(value, Value::Number(number) if number > 0.0),
            "{name}"
        );
    }
    let host = call(&mut realm, &binding, "getHostname", &[context()]).unwrap();
    assert!(matches!(host, Value::String(_)));
    let home = call(&mut realm, &binding, "getHomeDirectory", &[context()]).unwrap();
    assert!(matches!(home, Value::String(_)));
    let information = call(&mut realm, &binding, "getOSInformation", &[]).unwrap();
    assert!(show(&mut realm, &information).starts_with("[ 'Windows_NT'"));
    let cpus = call(&mut realm, &binding, "getCPUs", &[]).unwrap();
    let length = ops::get(&mut realm, &cpus, &Key::name("length")).unwrap();
    assert!(matches!(length, Value::Number(count) if count >= 7.0));
    let interfaces = call(&mut realm, &binding, "getInterfaceAddresses", &[context()]).unwrap();
    assert!(show(&mut realm, &interfaces).contains("IPv4"));
    let averages = Value::Object(realm.array(vec![Value::Number(1.0); 3]));
    call(
        &mut realm,
        &binding,
        "getLoadAvg",
        std::slice::from_ref(&averages),
    )
    .unwrap();
    assert_eq!(show(&mut realm, &averages), "[ 0, 0, 0 ]");
    let priority = call(
        &mut realm,
        &binding,
        "getPriority",
        &[Value::Number(0.0), context()],
    )
    .unwrap();
    assert!(matches!(priority, Value::Number(0.0)));
    let set = call(
        &mut realm,
        &binding,
        "setPriority",
        &[Value::Number(0.0), Value::Number(0.0), context()],
    )
    .unwrap();
    assert!(matches!(set, Value::Number(0.0)));
    let failed = context();
    let result = call(
        &mut realm,
        &binding,
        "getPriority",
        &[Value::Number(f64::from(i32::MAX)), failed.clone()],
    )
    .unwrap();
    assert!(matches!(result, Value::Undefined));
    assert_eq!(
        show(&mut realm, &failed),
        "[Object: null prototype] {\n  errno: -4040,\n  code: 'ESRCH',\n  message: 'no such process',\n  syscall: 'uv_os_getpriority'\n}"
    );
    let failed = context();
    let args = [Value::Number(0.0), Value::Number(20.0), failed.clone()];
    let result = call(&mut realm, &binding, "setPriority", &args).unwrap();
    assert!(matches!(result, Value::Number(-4071.0)));
    assert!(show(&mut realm, &failed).contains("uv_os_setpriority"));
    let big_endian = ops::get(
        &mut realm,
        &Value::Object(binding),
        &Key::name("isBigEndian"),
    )
    .unwrap();
    assert!(matches!(big_endian, Value::Boolean(false)));
}

#[test]
fn binding_failures_set_the_context() {
    let mut realm = Realm::new();
    let binding = binding(&mut realm);
    let mut session = shimforge::Session::new();
    shimforge::mock!(session, host_name, fn() -> Result<JsStr, i32>)
        .expect()
        .returning(|| Err(UV_ENOMEM));
    shimforge::mock!(session, home_directory, fn() -> Result<JsStr, i32>)
        .expect()
        .returning(|| Err(UV_ENOENT));
    shimforge::mock!(session, user, fn() -> Result<User, i32>)
        .expect()
        .returning(|| Err(UV_ENOSYS));
    shimforge::mock!(session, cpus, fn() -> Option<Vec<Cpu>>)
        .expect()
        .returning(|| None);
    let outcomes = std::sync::Arc::new(std::sync::Mutex::new(vec![UV_ENOSYS, UV_EAGAIN]));
    let pending = outcomes.clone();
    shimforge::mock!(session, interfaces, fn() -> Result<Vec<Interface>, i32>)
        .expect()
        .returning(move || Err(pending.lock().unwrap().remove(0)));
    for (name, args, syscall) in [
        ("getHostname", vec![], "uv_os_gethostname"),
        ("getHomeDirectory", vec![], "uv_os_homedir"),
        ("getUserInfo", vec![Value::Null], "uv_os_get_passwd"),
        ("getInterfaceAddresses", vec![], "uv_interface_addresses"),
    ] {
        let failed = context();
        let mut args = args;
        args.push(failed.clone());
        if name == "getInterfaceAddresses" {
            // Without iphlpapi there is nothing to report.
            let result = call(&mut realm, &binding, name, &args).unwrap();
            assert!(matches!(result, Value::Undefined));
        }
        let result = call(&mut realm, &binding, name, &args).unwrap();
        assert!(matches!(result, Value::Undefined), "{name}");
        assert!(show(&mut realm, &failed).contains(syscall), "{name}");
    }
    let cpus = call(&mut realm, &binding, "getCPUs", &[]).unwrap();
    assert!(matches!(cpus, Value::Undefined));
}

#[test]
fn user_information_is_encoded_as_asked() {
    let mut realm = Realm::new();
    let binding = binding(&mut realm);
    let mut session = shimforge::Session::new();
    shimforge::mock!(session, user, fn() -> Result<User, i32>)
        .expect()
        .returning(|| {
            Ok(User {
                username: JsStr::from("ü"),
                homedir: JsStr::from("C:\\Users\\ü"),
            })
        });
    let options = |realm: &mut Realm, encoding: Value| {
        let object = realm.object();
        object.define(Key::name("encoding"), Property::data(encoding));
        Value::Object(object)
    };
    for (encoding, expected) in [
        (Value::Undefined, "'ü'"),
        (Value::str("hex"), "'c3bc'"),
        (Value::str("latin1"), "'Ã¼'"),
        (Value::str("nonsense"), "'ü'"),
        (Value::Number(1.0), "'ü'"),
    ] {
        let args = [options(&mut realm, encoding), context()];
        let info = call(&mut realm, &binding, "getUserInfo", &args).unwrap();
        let username = ops::get(&mut realm, &info, &Key::name("username")).unwrap();
        assert_eq!(show(&mut realm, &username), expected);
    }
    let info = call(
        &mut realm,
        &binding,
        "getUserInfo",
        &[Value::Undefined, context()],
    )
    .unwrap();
    assert!(info.as_object().unwrap().data().prototype.is_none());
    assert_eq!(
        show(&mut realm, &info),
        "[Object: null prototype] {\n  uid: -1,\n  gid: -1,\n  username: 'ü',\n  homedir: 'C:\\\\Users\\\\ü',\n  shell: null\n}"
    );
    // Buffers come from the Node.js library, which these tests do not link.
    let args = [options(&mut realm, Value::str("BUFFER")), context()];
    let thrown = call(&mut realm, &binding, "getUserInfo", &args).unwrap_err();
    assert!(show(&mut realm, &thrown.0).contains("not linked"));
    // Buffers otherwise have the registered prototype.
    fn library(realm: &mut Realm, _: &str) -> Completion {
        if realm.buffer_prototype.is_none() {
            realm.buffer_prototype = Some(realm.object());
        }
        Ok(Value::Undefined)
    }
    realm.node_library = Some(library);
    let info = call(&mut realm, &binding, "getUserInfo", &args).unwrap();
    let username = ops::get(&mut realm, &info, &Key::name("username")).unwrap();
    let prototype = username.as_object().unwrap().data().prototype.clone();
    assert_eq!(prototype, realm.buffer_prototype);
    let getter = realm.function("get", 0, false, |realm, _, _| {
        Err(realm.throw(ErrorKind::Range, "encoding"))
    });
    let throwing = realm.object();
    throwing.define(
        Key::name("encoding"),
        Property {
            slot: crate::Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: true,
            configurable: true,
        },
    );
    let args = [Value::Object(throwing), context()];
    let thrown = call(&mut realm, &binding, "getUserInfo", &args).unwrap_err();
    assert_eq!(show(&mut realm, &thrown.0), "[RangeError: encoding]");
}

#[test]
fn constants_and_credentials_match_node_on_windows() {
    let mut realm = Realm::new();
    let constants = Value::Object(constants());
    let os = ops::get(&mut realm, &constants, &Key::name("os")).unwrap();
    let shown = show(&mut realm, &os);
    assert!(
        shown.starts_with("[Object: null prototype] {\n  UV_UDP_REUSEADDR: 4,"),
        "{shown}"
    );
    let signals = ops::get(&mut realm, &os, &Key::name("signals")).unwrap();
    let sigint = ops::get(&mut realm, &signals, &Key::name("SIGINT")).unwrap();
    assert!(matches!(sigint, Value::Number(2.0)));
    assert_eq!(ERRNO.len(), 134);
    let credentials = credentials(&mut realm);
    let path = call(
        &mut realm,
        &credentials,
        "safeGetenv",
        &[Value::str("PATH")],
    )
    .unwrap();
    assert!(matches!(path, Value::String(_)));
    let missing = call(
        &mut realm,
        &credentials,
        "safeGetenv",
        &[Value::str("NATIVEBOAT_MISSING_VARIABLE")],
    )
    .unwrap();
    assert!(matches!(missing, Value::Undefined));
}

#[test]
fn filesystem_constants_match_the_pinned_windows_binding_shape() {
    let mut realm = Realm::new();
    let binding = Value::Object(constants());
    let fs = ops::get(&mut realm, &binding, &Key::name("fs")).unwrap();
    assert!(fs.as_object().unwrap().data().prototype.is_none());
    assert_eq!(fs.as_object().unwrap().own_keys().len(), 38);
    for (key, value) in [
        ("O_CREAT", 256.0),
        ("O_EXCL", 1024.0),
        ("S_IFREG", 32768.0),
        ("UV_FS_COPYFILE_FICLONE_FORCE", 4.0),
        ("UV_DIRENT_BLOCK", 7.0),
    ] {
        assert!(ops::strict_equals(
            &ops::get(&mut realm, &fs, &Key::name(key)).unwrap(),
            &Value::Number(value)
        ));
    }
    for absent in ["S_IFBLK", "S_IFSOCK", "O_SYNC"] {
        assert!(matches!(
            ops::get(&mut realm, &fs, &Key::name(absent)).unwrap(),
            Value::Undefined
        ));
    }
}
