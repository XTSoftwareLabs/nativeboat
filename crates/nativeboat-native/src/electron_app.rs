//! `app` methods beyond readiness, paths and switches, as Electron gives
//! them on Windows:
//!
//! - `enableSandbox()` and `setAppLogsPath([path])`, which programs call
//!   before the app is ready: native pages run in the application's realms,
//!   so there is no renderer sandbox to enable; the logs path is
//!   `<userData>\logs` unless one is given.
//! - `focus()`, which brings the application's first window forward.
//! - `getAppMetrics()`: the application's one process, which runs its pages
//!   too, with its memory and creation time.
//! - `getGPUFeatureStatus()` and `getGPUInfo()`: pages are painted without
//!   Chromium's GPU process, so features are reported as Chromium reports
//!   them when software rendering is used.
//! - `getJumpListSettings()`: Windows' default of ten slots, with no item
//!   the user removed.
//! - `isAccessibilitySupportEnabled()` and `accessibilitySupportEnabled`:
//!   whether a screen reader runs, as Chromium enables its accessibility
//!   support then, or as the program set it.
//! - `setProxy` and `resolveProxy`, which use the default session's.
//! - `getFileIcon(path)`: a file's icon is not read; the promise resolves
//!   with an empty image.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::Cell;
use core::ffi::c_void;

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Key, Object, Property, Realm, Value, ops};

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemParametersInfoW(action: u32, parameter: u32, value: *mut c_void, flags: u32) -> i32;
}

const SPI_GETSCREENREADER: u32 = 0x0046;

/// Whether a screen reader is running.
fn screen_reader() -> bool {
    let mut running = 0u32;
    // SAFETY: SPI_GETSCREENREADER writes a BOOL to the given address.
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETSCREENREADER, 0, (&mut running as *mut u32).cast(), 0)
    };
    ok != 0 && running != 0
}

fn define(
    realm: &Realm,
    object: &Object,
    name: &str,
    length: u32,
    code: impl Fn(&mut Realm, Value, &[Value]) -> crate::Completion + 'static,
) {
    crate::electron_window::define(realm, object, name, length, code);
}

fn resolved(realm: &mut Realm, value: Value) -> crate::Completion {
    let (promise, resolve, _) = crate::promise_builtin::capability(realm);
    ops::call(realm, &resolve, Value::Undefined, &[value])?;
    Ok(promise)
}

fn number(object: &Object, name: &str, value: f64) {
    object.define(Key::name(name), Property::data(Value::Number(value)));
}

/// Chromium's feature status names when the GPU process is not used.
const GPU_FEATURES: &[(&str, &str)] = &[
    ("2d_canvas", "unavailable_software"),
    ("canvas_oop_rasterization", "disabled_off"),
    ("direct_rendering_display_compositor", "disabled_off_ok"),
    ("gpu_compositing", "disabled_software"),
    ("multiple_raster_threads", "enabled_on"),
    ("opengl", "disabled_off"),
    ("rasterization", "disabled_software"),
    ("raw_draw", "disabled_off_ok"),
    ("skia_graphite", "disabled_off"),
    ("video_decode", "disabled_software"),
    ("video_encode", "disabled_software"),
    ("vulkan", "disabled_off"),
    ("webgl", "unavailable_software"),
    ("webgl2", "unavailable_software"),
    ("webgpu", "disabled_off"),
    ("webnn", "disabled_off"),
];

/// Installs the methods on `app`.
pub(crate) fn install(realm: &Realm, app: &Object) {
    define(realm, app, "enableSandbox", 0, |_, _, _| {
        Ok(Value::Undefined)
    });
    define(realm, app, "setAppLogsPath", 1, |realm, _, args| {
        let path = match args.first() {
            Some(Value::String(path)) => path.to_rust_lossy(),
            Some(Value::Undefined) | None => {
                match crate::electron_builtin::app_path(realm, "userData")? {
                    Value::String(data) => alloc::format!("{}\\logs", data.to_rust_lossy()),
                    _ => return Ok(Value::Undefined),
                }
            }
            Some(_) => {
                return Err(realm.throw(
                    crate::ErrorKind::Type,
                    "setAppLogsPath expects an absolute path",
                ));
            }
        };
        realm.electron_app_paths.retain(|(name, _)| name != "logs");
        realm.electron_app_paths.push(("logs".into(), path));
        Ok(Value::Undefined)
    });
    define(realm, app, "focus", 1, |realm, _, _| {
        let first = realm
            .electron_windows
            .iter()
            .map(|(window, _, _)| crate::app_host::Window::from_raw(*window))
            .find(|window| crate::app_host::is_window(*window));
        if let Some(window) = first {
            crate::app_host::set_window_state(window, crate::app_host::WindowState::Focus);
        }
        Ok(Value::Undefined)
    });
    define(realm, app, "getAppMetrics", 0, |realm, _, _| {
        let metric = realm.object();
        number(&metric, "pid", f64::from(crate::platform::process_id()));
        metric.define(Key::name("type"), Property::data(Value::str("Browser")));
        number(
            &metric,
            "creationTime",
            crate::process_usage::creation_time_ms(),
        );
        let (working, peak, private) = crate::process_usage::memory_kilobytes();
        let memory = realm.object();
        number(&memory, "workingSetSize", working);
        number(&memory, "peakWorkingSetSize", peak);
        number(&memory, "privateBytes", private);
        metric.define(Key::name("memory"), Property::data(Value::Object(memory)));
        let cpu = realm.object();
        number(&cpu, "percentCPUUsage", 0.0);
        number(&cpu, "idleWakeupsPerSecond", 0.0);
        metric.define(Key::name("cpu"), Property::data(Value::Object(cpu)));
        metric.define(
            Key::name("sandboxed"),
            Property::data(Value::Boolean(false)),
        );
        metric.define(
            Key::name("integrityLevel"),
            Property::data(Value::str("medium")),
        );
        Ok(Value::Object(
            realm.array(alloc::vec![Value::Object(metric)]),
        ))
    });
    define(realm, app, "getGPUFeatureStatus", 0, |realm, _, _| {
        let status = realm.object();
        for (feature, value) in GPU_FEATURES {
            status.define(Key::name(feature), Property::data(Value::str(value)));
        }
        Ok(Value::Object(status))
    });
    define(realm, app, "getGPUInfo", 1, |realm, _, _| {
        let info = realm.object();
        info.define(
            Key::name("gpuDevice"),
            Property::data(Value::Object(realm.array(Vec::new()))),
        );
        info.define(
            Key::name("auxAttributes"),
            Property::data(Value::Object(realm.object())),
        );
        resolved(realm, Value::Object(info))
    });
    define(realm, app, "getJumpListSettings", 0, |realm, _, _| {
        let settings = realm.object();
        number(&settings, "minItems", 10.0);
        settings.define(
            Key::name("removedItems"),
            Property::data(Value::Object(realm.array(Vec::new()))),
        );
        Ok(Value::Object(settings))
    });
    // Accessibility support: a screen reader's, or the program's choice.
    let chosen: Rc<Cell<Option<bool>>> = Rc::default();
    let enabled = move |chosen: &Cell<Option<bool>>| chosen.get().unwrap_or_else(screen_reader);
    let read = chosen.clone();
    define(
        realm,
        app,
        "isAccessibilitySupportEnabled",
        0,
        move |_, _, _| Ok(Value::Boolean(enabled(&read))),
    );
    let write = chosen.clone();
    define(
        realm,
        app,
        "setAccessibilitySupportEnabled",
        1,
        move |_, _, args| {
            write.set(Some(ops::to_boolean(
                args.first().unwrap_or(&Value::Undefined),
            )));
            Ok(Value::Undefined)
        },
    );
    let read = chosen.clone();
    let getter = realm.function(
        "get accessibilitySupportEnabled",
        0,
        false,
        move |_, _, _| Ok(Value::Boolean(enabled(&read))),
    );
    let setter = realm.function(
        "set accessibilitySupportEnabled",
        1,
        false,
        move |_, _, args| {
            chosen.set(Some(ops::to_boolean(
                args.first().unwrap_or(&Value::Undefined),
            )));
            Ok(Value::Undefined)
        },
    );
    ops::define_accessor(
        app,
        Key::name("accessibilitySupportEnabled"),
        Some(getter),
        Some(setter),
    );
    // The default session's proxy settings, as `app`'s are.
    for name in ["setProxy", "resolveProxy"] {
        define(realm, app, name, 1, move |realm, _, args| {
            let Some(session) = realm.electron_session.clone() else {
                return Err(realm.throw(
                    crate::ErrorKind::Error,
                    "the default session is not available",
                ));
            };
            let session = Value::Object(session);
            let method = ops::get_named(realm, &session, name)?;
            ops::call(realm, &method, session, args)
        });
    }
    define(realm, app, "getFileIcon", 2, |realm, _, _| {
        let image = crate::electron_native_image::empty(realm);
        resolved(realm, Value::Object(image))
    });
}
