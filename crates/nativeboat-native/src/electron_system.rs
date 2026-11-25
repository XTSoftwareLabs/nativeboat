//! Electron's `powerMonitor` and `systemPreferences` on Windows: idle time,
//! lock state, power source, accent and system colors and animation
//! settings, and the events the notification window hears for them.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;

use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, app_host, ops};

/// The notification targets the two modules' events go to.
pub(crate) const POWER_MONITOR: u32 = u32::MAX;
pub(crate) const SYSTEM_PREFERENCES: u32 = u32::MAX - 1;

#[repr(C)]
struct LastInput {
    size: u32,
    time: u32,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastInputInfo(input: *mut LastInput) -> i32;
    fn OpenInputDesktop(flags: u32, inherit: i32, access: u32) -> *mut c_void;
    fn CloseDesktop(desktop: *mut c_void) -> i32;
    fn GetUserObjectInformationW(
        object: *mut c_void,
        index: i32,
        info: *mut c_void,
        size: u32,
        needed: *mut u32,
    ) -> i32;
    fn SystemParametersInfoW(action: u32, parameter: u32, value: *mut c_void, flags: u32) -> i32;
    fn GetSysColor(index: i32) -> u32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetTickCount() -> u32;
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
}

/// Seconds since the user last used the mouse or keyboard.
fn idle_seconds() -> u32 {
    let mut input = LastInput {
        size: core::mem::size_of::<LastInput>() as u32,
        time: 0,
    };
    // SAFETY: the structure is sized as Windows expects.
    if unsafe { GetLastInputInfo(&mut input) } == 0 {
        return 0;
    }
    // SAFETY: GetTickCount has no preconditions; both counts wrap alike.
    unsafe { GetTickCount() }.wrapping_sub(input.time) / 1000
}

/// Whether the workstation is locked, as Chromium tells: the input desktop
/// is not the user's default one, or a screen saver runs.
fn locked() -> bool {
    const GENERIC_READ: u32 = 0x8000_0000;
    const UOI_NAME: i32 = 2;
    const SPI_GETSCREENSAVERRUNNING: u32 = 0x0072;
    let mut locked = true;
    // SAFETY: opens the input desktop for reading; it is closed below.
    let desktop = unsafe { OpenInputDesktop(0, 0, GENERIC_READ) };
    if !desktop.is_null() {
        let mut name = [0u16; 256];
        let mut needed = 0;
        // SAFETY: the buffer holds the size passed, in bytes.
        let read = unsafe {
            GetUserObjectInformationW(
                desktop,
                UOI_NAME,
                name.as_mut_ptr().cast(),
                (name.len() * 2) as u32,
                &mut needed,
            )
        };
        if read != 0 {
            let end = name
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(name.len());
            locked = !String::from_utf16_lossy(&name[..end]).eq_ignore_ascii_case("default");
        }
        // SAFETY: the desktop was opened above.
        unsafe { CloseDesktop(desktop) };
    }
    let mut saver = 0i32;
    // SAFETY: the value is a writable BOOL.
    let read =
        unsafe { SystemParametersInfoW(SPI_GETSCREENSAVERRUNNING, 0, (&raw mut saver).cast(), 0) };
    locked || (read != 0 && saver != 0)
}

/// Windows' accent (DWM colorization) color, `0xAARRGGBB`.
fn colorization() -> Option<u32> {
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
    let library: Vec<u16> = "dwmapi.dll\0".encode_utf16().collect();
    // SAFETY: the name is null-terminated; system32 is searched only.
    let module = unsafe {
        LoadLibraryExW(
            library.as_ptr(),
            core::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return None;
    }
    // SAFETY: the module is loaded and the name null-terminated.
    let function = unsafe { GetProcAddress(module, c"DwmGetColorizationColor".as_ptr().cast()) };
    if function.is_null() {
        return None;
    }
    type Colorization = unsafe extern "system" fn(*mut u32, *mut i32) -> i32;
    let (mut color, mut opaque) = (0u32, 0i32);
    // SAFETY: the address is DwmGetColorizationColor's; both outputs are
    // writable.
    let result = unsafe {
        let function: Colorization = core::mem::transmute(function);
        function(&mut color, &mut opaque)
    };
    (result >= 0).then_some(color)
}

/// The application's Electron major version; the latest when unknown.
fn electron_major(realm: &Realm) -> u32 {
    realm
        .electron_version
        .and_then(|version| version.split('.').next()?.parse().ok())
        .unwrap_or(u32::MAX)
}

/// The accent color as an Electron release reads it: from Electron 38 the
/// `AccentColor` Windows keeps for the user, before it the DWM
/// colorization color.
fn accent_color(major: u32) -> Option<u32> {
    if major >= 38 {
        crate::electron_login_item::registry_dword(
            "SOFTWARE\\Microsoft\\Windows\\DWM",
            "AccentColor",
        )
    } else {
        colorization()
    }
}

/// A color DWORD as Electron writes the accent color: its bytes turned so
/// the first comes last, in lowercase hex.
pub(crate) fn accent_hex(color: u32) -> String {
    alloc::format!("{:08x}", color.rotate_left(8))
}

/// Electron's names for Windows' system colors.
const SYSTEM_COLORS: [(&str, i32); 30] = [
    ("3d-dark-shadow", 21),
    ("3d-face", 15),
    ("3d-highlight", 20),
    ("3d-light", 22),
    ("3d-shadow", 16),
    ("active-border", 10),
    ("active-caption", 2),
    ("active-caption-gradient", 27),
    ("app-workspace", 12),
    ("button-text", 18),
    ("caption-text", 9),
    ("desktop", 1),
    ("disabled-text", 17),
    ("highlight", 13),
    ("highlight-text", 14),
    ("hotlight", 26),
    ("inactive-border", 11),
    ("inactive-caption", 3),
    ("inactive-caption-gradient", 28),
    ("inactive-caption-text", 19),
    ("info-background", 24),
    ("info-text", 23),
    ("menu", 4),
    ("menu-highlight", 29),
    ("menu-text", 7),
    ("menubar", 30),
    ("scrollbar", 0),
    ("window", 5),
    ("window-frame", 6),
    ("window-text", 8),
];

/// A `COLORREF` as Electron writes system colors: `#RRGGBBAA`, opaque.
fn system_color_hex(colorref: u32) -> String {
    let (red, green, blue) = (colorref & 0xff, colorref >> 8 & 0xff, colorref >> 16 & 0xff);
    alloc::format!("#{red:02X}{green:02X}{blue:02X}FF")
}

/// Whether Windows animates controls and elements inside windows; on when
/// the setting cannot be read, as Chromium takes it.
fn animates() -> bool {
    const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
    let mut enabled = 1i32;
    // SAFETY: the value is a writable BOOL.
    let read = unsafe {
        SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut enabled).cast(), 0)
    };
    read == 0 || enabled != 0
}

fn number_argument(realm: &mut Realm, args: &[Value]) -> Result<f64, Thrown> {
    ops::to_number(realm, args.first().unwrap_or(&Value::Undefined))
}

fn string_argument(realm: &mut Realm, args: &[Value]) -> Result<String, Thrown> {
    Ok(ops::to_string(realm, args.first().unwrap_or(&Value::Undefined))?.to_rust_lossy())
}

fn define(target: &Object, functions: Vec<(&str, Object)>) {
    for (name, function) in functions {
        target.define(Key::name(name), Property::data(Value::Object(function)));
    }
}

/// Makes a module an event emitter whose events the notification window
/// sends, from now on.
fn listen(realm: &mut Realm, emitter: Option<&Value>, module: &Object, target: u32) {
    match emitter {
        Some(emitter) => {
            let _ = crate::electron_window::make_emitter(realm, emitter, module, None);
        }
        None => crate::electron_window::inert_listeners(realm, module),
    }
    if app_host::watch_system().is_ok() {
        realm.notify_targets.insert(
            target,
            crate::electron_notify::Target::System {
                object: module.clone(),
                accent: colorization(),
            },
        );
    }
}

/// Electron's `powerMonitor`.
pub(crate) fn power_monitor(realm: &mut Realm, emitter: Option<&Value>) -> Object {
    let monitor = realm.object();
    let idle_state = realm.function("getSystemIdleState", 1, false, |realm, _, args| {
        let threshold = number_argument(realm, args)?;
        if threshold.is_nan() || threshold < 1.0 {
            return Err(realm.throw(
                ErrorKind::Type,
                "Invalid idle threshold, must be greater than 0",
            ));
        }
        let state = if locked() {
            "locked"
        } else if f64::from(idle_seconds()) >= crate::float::trunc(threshold) {
            "idle"
        } else {
            "active"
        };
        Ok(Value::str(state))
    });
    let idle_time = realm.function("getSystemIdleTime", 0, false, |_, _, _| {
        Ok(Value::Number(f64::from(idle_seconds())))
    });
    // Windows tells no thermal state.
    let thermal = realm.function("getCurrentThermalState", 0, false, |_, _, _| {
        Ok(Value::str("unknown"))
    });
    let battery = realm.function("isOnBatteryPower", 0, false, |_, _, _| {
        Ok(Value::Boolean(app_host::on_battery()))
    });
    define(
        &monitor,
        alloc::vec![
            ("getSystemIdleState", idle_state),
            ("getSystemIdleTime", idle_time),
            ("getCurrentThermalState", thermal),
            ("isOnBatteryPower", battery),
        ],
    );
    let on_battery = realm.function("get onBatteryPower", 0, false, |_, _, _| {
        Ok(Value::Boolean(app_host::on_battery()))
    });
    ops::define_accessor(
        &monitor,
        Key::name("onBatteryPower"),
        Some(on_battery),
        None,
    );
    listen(realm, emitter, &monitor, POWER_MONITOR);
    monitor
}

/// Electron's `systemPreferences`, with the methods it has on Windows.
pub(crate) fn system_preferences(realm: &mut Realm, emitter: Option<&Value>) -> Object {
    let preferences = realm.object();
    let major = electron_major(realm);
    let accent = realm.function("getAccentColor", 0, false, move |_, _, _| {
        Ok(Value::str(
            &accent_color(major).map(accent_hex).unwrap_or_default(),
        ))
    });
    let color = realm.function("getColor", 1, false, |realm, _, args| {
        let name = string_argument(realm, args)?;
        match SYSTEM_COLORS.iter().find(|(known, _)| *known == name) {
            // SAFETY: GetSysColor takes any index.
            Some((_, index)) => Ok(Value::str(&system_color_hex(unsafe {
                GetSysColor(*index)
            }))),
            None => Err(realm.throw(ErrorKind::Error, &alloc::format!("Unknown color: {name}"))),
        }
    });
    // Desktop applications are allowed what Windows' privacy settings do not
    // deny them, as Electron finds when Windows cannot tell.
    let media = realm.function("getMediaAccessStatus", 1, false, |realm, _, args| {
        let kind = string_argument(realm, args)?;
        let store = match kind.as_str() {
            "camera" => "webcam",
            "microphone" => "microphone",
            "screen" => return Ok(Value::str("granted")),
            _ => return Err(realm.throw(ErrorKind::Error, "Invalid media type")),
        };
        let denied = crate::electron_login_item::registry_string(
            &alloc::format!(
                "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\{store}"
            ),
            "Value",
        )
        .is_some_and(|value| value.eq_ignore_ascii_case("deny"));
        Ok(Value::str(if denied { "denied" } else { "granted" }))
    });
    let aero = realm.function("isAeroGlassEnabled", 0, false, |_, _, _| {
        Ok(Value::Boolean(true))
    });
    let animation = realm.function("getAnimationSettings", 0, false, |realm, _, _| {
        let animates = animates();
        let settings = realm.object();
        for (name, value) in [
            ("shouldRenderRichAnimation", animates),
            ("scrollAnimationsEnabledBySystem", animates),
            ("prefersReducedMotion", !animates),
        ] {
            settings.define(Key::name(name), Property::data(Value::Boolean(value)));
        }
        Ok(Value::Object(settings))
    });
    let mut methods = alloc::vec![
        ("getAccentColor", accent),
        ("getColor", color),
        ("getMediaAccessStatus", media),
        ("getAnimationSettings", animation),
    ];
    // Electron 35 removed it.
    if major <= 34 {
        methods.push(("isAeroGlassEnabled", aero));
    }
    define(&preferences, methods);
    listen(realm, emitter, &preferences, SYSTEM_PREFERENCES);
    preferences
}

#[cfg(test)]
#[path = "electron_system/tests.rs"]
mod tests;
