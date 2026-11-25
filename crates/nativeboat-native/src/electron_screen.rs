//! Electron screen queries backed by the Windows monitor and cursor APIs.

use alloc::vec::Vec;
use core::ffi::c_void;

use crate::{ErrorKind, Key, Object, Property, Realm, Value, ops};

type Monitor = *mut c_void;
type DeviceContext = *mut c_void;
type MonitorCallback = unsafe extern "system" fn(Monitor, DeviceContext, *mut Rect, isize) -> i32;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[repr(C)]
#[derive(Default)]
struct Point {
    x: i32,
    y: i32,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn EnumDisplayMonitors(
        dc: DeviceContext,
        clip: *const Rect,
        callback: Option<MonitorCallback>,
        data: isize,
    ) -> i32;
    fn GetMonitorInfoW(monitor: Monitor, info: *mut MonitorInfo) -> i32;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "shcore", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForMonitor(monitor: Monitor, kind: i32, x: *mut u32, y: *mut u32) -> i32;
}

const MONITORINFOF_PRIMARY: u32 = 1;
const MDT_EFFECTIVE_DPI: i32 = 0;

#[derive(Clone, Copy)]
struct Display {
    id: u64,
    bounds: (i32, i32, i32, i32),
    work_area: (i32, i32, i32, i32),
    scale: f64,
    primary: bool,
}

fn logical_rect(rect: Rect, scale: f64) -> (i32, i32, i32, i32) {
    let scale = scale.max(0.5);
    let logical = |value: i32| crate::float::floor(f64::from(value) / scale + 0.5) as i32;
    (
        logical(rect.left),
        logical(rect.top),
        logical(rect.right - rect.left),
        logical(rect.bottom - rect.top),
    )
}

unsafe extern "system" fn collect_monitor(
    monitor: Monitor,
    _: DeviceContext,
    _: *mut Rect,
    data: isize,
) -> i32 {
    // SAFETY: `displays` passes a live vector pointer for the duration of enumeration.
    let displays = unsafe { &mut *(data as *mut Vec<Display>) };
    let mut info = MonitorInfo {
        size: size_of::<MonitorInfo>() as u32,
        monitor: Rect::default(),
        work: Rect::default(),
        flags: 0,
    };
    // SAFETY: the handle comes from EnumDisplayMonitors and info is writable.
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return 1;
    }
    let mut dpi_x = 96;
    let mut dpi_y = 96;
    // SAFETY: monitor is live and both DPI outputs are writable.
    let dpi_result =
        unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    let scale = if dpi_result >= 0 && dpi_x > 0 {
        f64::from(dpi_x) / 96.0
    } else {
        1.0
    };
    displays.push(Display {
        id: monitor as usize as u64,
        bounds: logical_rect(info.monitor, scale),
        work_area: logical_rect(info.work, scale),
        scale,
        primary: info.flags & MONITORINFOF_PRIMARY != 0,
    });
    1
}

fn displays() -> Vec<Display> {
    let mut displays = Vec::new();
    // SAFETY: callback and vector remain live for the whole synchronous enumeration.
    unsafe {
        EnumDisplayMonitors(
            core::ptr::null_mut(),
            core::ptr::null(),
            Some(collect_monitor),
            core::ptr::from_mut(&mut displays) as isize,
        )
    };
    ensure_display_fallback(&mut displays);
    displays
}

fn ensure_display_fallback(displays: &mut Vec<Display>) {
    if displays.is_empty() {
        // Windows always has a primary display, but keep the Electron API
        // well-formed if monitor enumeration fails in a restricted session.
        // SAFETY: GetSystemMetrics takes integer indexes and owns no pointers.
        let width = unsafe { GetSystemMetrics(0) }.max(1);
        // SAFETY: GetSystemMetrics takes integer indexes and owns no pointers.
        let height = unsafe { GetSystemMetrics(1) }.max(1);
        displays.push(Display {
            id: 0,
            bounds: (0, 0, width, height),
            work_area: (0, 0, width, height),
            scale: 1.0,
            primary: true,
        });
    }
}

fn rect_object(realm: &Realm, rect: (i32, i32, i32, i32)) -> Value {
    let value = realm.object();
    for (name, number) in [
        ("x", rect.0),
        ("y", rect.1),
        ("width", rect.2),
        ("height", rect.3),
    ] {
        value.define(
            Key::name(name),
            Property::data(Value::Number(f64::from(number))),
        );
    }
    Value::Object(value)
}

fn display_object(realm: &Realm, display: Display) -> Object {
    let object = realm.object();
    object.define(
        Key::name("id"),
        Property::data(Value::Number(display.id as f64)),
    );
    object.define(
        Key::name("bounds"),
        Property::data(rect_object(realm, display.bounds)),
    );
    object.define(
        Key::name("workArea"),
        Property::data(rect_object(realm, display.work_area)),
    );
    object.define(
        Key::name("size"),
        Property::data(rect_object(
            realm,
            (0, 0, display.bounds.2, display.bounds.3),
        )),
    );
    object.define(
        Key::name("workAreaSize"),
        Property::data(rect_object(
            realm,
            (0, 0, display.work_area.2, display.work_area.3),
        )),
    );
    object.define(
        Key::name("scaleFactor"),
        Property::data(Value::Number(display.scale)),
    );
    object.define(Key::name("rotation"), Property::data(Value::Number(0.0)));
    object.define(
        Key::name("internal"),
        // Electron reports built-in panels separately from the primary
        // desktop monitor. Windows' primary flag cannot identify that property.
        Property::data(Value::Boolean(false)),
    );
    object
}

fn point(realm: &mut Realm, value: Option<&Value>) -> Result<(f64, f64), crate::Thrown> {
    let value = value.ok_or_else(|| realm.throw(ErrorKind::Type, "A point object is required"))?;
    if !matches!(value, Value::Object(_)) {
        return Err(realm.throw(ErrorKind::Type, "A point object is required"));
    }
    let x = ops::get_named(realm, value, "x")?;
    let y = ops::get_named(realm, value, "y")?;
    let x = ops::to_number(realm, &x)?;
    let y = ops::to_number(realm, &y)?;
    Ok((x, y))
}

fn nearest(displays: &[Display], x: f64, y: f64) -> Display {
    displays
        .iter()
        .copied()
        .min_by(|left, right| distance(left.bounds, x, y).total_cmp(&distance(right.bounds, x, y)))
        .unwrap_or(displays[0])
}

fn distance(rect: (i32, i32, i32, i32), x: f64, y: f64) -> f64 {
    let dx = if x < f64::from(rect.0) {
        f64::from(rect.0) - x
    } else if x > f64::from(rect.0 + rect.2) {
        x - f64::from(rect.0 + rect.2)
    } else {
        0.0
    };
    let dy = if y < f64::from(rect.1) {
        f64::from(rect.1) - y
    } else if y > f64::from(rect.1 + rect.3) {
        y - f64::from(rect.1 + rect.3)
    } else {
        0.0
    };
    dx * dx + dy * dy
}

fn matching(displays: &[Display], rect: (f64, f64, f64, f64)) -> Display {
    displays
        .iter()
        .copied()
        .max_by(|left, right| {
            intersection(left.bounds, rect).total_cmp(&intersection(right.bounds, rect))
        })
        .unwrap_or(displays[0])
}

fn intersection(bounds: (i32, i32, i32, i32), rect: (f64, f64, f64, f64)) -> f64 {
    let width = (f64::from(bounds.0 + bounds.2).min(rect.0 + rect.2)
        - f64::from(bounds.0).max(rect.0))
    .max(0.0);
    let height = (f64::from(bounds.1 + bounds.3).min(rect.1 + rect.3)
        - f64::from(bounds.1).max(rect.1))
    .max(0.0);
    width * height
}

/// The native Windows displays consumed by Electron's window-placement helpers.
pub fn module(realm: &Realm) -> Object {
    let screen = realm.object();
    let primary = realm.function("getPrimaryDisplay", 0, false, |realm, _, _| {
        let displays = displays();
        let display = displays
            .iter()
            .copied()
            .find(|display| display.primary)
            .unwrap_or_else(|| displays[0]);
        Ok(Value::Object(display_object(realm, display)))
    });
    screen.define(
        Key::name("getPrimaryDisplay"),
        Property::data(Value::Object(primary)),
    );
    let all = realm.function("getAllDisplays", 0, false, |realm, _, _| {
        Ok(Value::Object(
            realm.array(
                displays()
                    .into_iter()
                    .map(|display| Value::Object(display_object(realm, display)))
                    .collect(),
            ),
        ))
    });
    screen.define(
        Key::name("getAllDisplays"),
        Property::data(Value::Object(all)),
    );
    let cursor = realm.function("getCursorScreenPoint", 0, false, |realm, _, _| {
        let mut point = Point::default();
        // SAFETY: the Windows API writes the cursor position to this structure.
        unsafe { GetCursorPos(&mut point) };
        let value = realm.object();
        value.define(
            Key::name("x"),
            Property::data(Value::Number(f64::from(point.x))),
        );
        value.define(
            Key::name("y"),
            Property::data(Value::Number(f64::from(point.y))),
        );
        Ok(Value::Object(value))
    });
    screen.define(
        Key::name("getCursorScreenPoint"),
        Property::data(Value::Object(cursor)),
    );
    let nearest_display = realm.function("getDisplayNearestPoint", 1, false, |realm, _, args| {
        let (x, y) = point(realm, args.first())?;
        let displays = displays();
        Ok(Value::Object(display_object(
            realm,
            nearest(&displays, x, y),
        )))
    });
    screen.define(
        Key::name("getDisplayNearestPoint"),
        Property::data(Value::Object(nearest_display)),
    );
    let matching_display = realm.function("getDisplayMatching", 1, false, |realm, _, args| {
        let (x, y) = point(realm, args.first())?;
        let value = args.first().expect("point validated");
        let width_value = ops::get_named(realm, value, "width")?;
        let height_value = ops::get_named(realm, value, "height")?;
        let width = ops::to_number(realm, &width_value)?;
        let height = ops::to_number(realm, &height_value)?;
        let displays = displays();
        Ok(Value::Object(display_object(
            realm,
            matching(&displays, (x, y, width, height)),
        )))
    });
    screen.define(
        Key::name("getDisplayMatching"),
        Property::data(Value::Object(matching_display)),
    );
    screen
}

#[cfg(test)]
#[path = "electron_screen/tests.rs"]
mod tests;
