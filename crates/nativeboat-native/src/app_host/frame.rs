//! Electron window geometry on Win32. Electron measures windows in
//! device-independent pixels, which each window's DPI scales to device
//! pixels. Content-sized windows exclude the native frame. Frameless windows
//! have no native frame, caption or menu bar: their whole area is client
//! area, with resize borders and drag regions answered by the host.

use super::{
    GetClientRect, GetWindowLongPtrW, GetWindowRect, HOST_WINDOWS, Hwnd, InvalidateRect, IsZoomed,
    SendMessageW, SetWindowPos, WinRect, Window,
};
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Point {
    pub(super) x: i32,
    pub(super) y: i32,
}

#[repr(C)]
#[derive(Default)]
struct MinMaxInfo {
    reserved: Point,
    max_size: Point,
    max_position: Point,
    min_track: Point,
    max_track: Point,
}

#[repr(C)]
struct MonitorInfo {
    size: u32,
    monitor: WinRect,
    work: WinRect,
    flags: u32,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForWindow(window: Hwnd) -> u32;
    fn AdjustWindowRectExForDpi(
        rect: *mut WinRect,
        style: u32,
        menu: i32,
        extended: u32,
        dpi: u32,
    ) -> i32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    fn MonitorFromPoint(point: Point, flags: u32) -> *mut c_void;
    fn MonitorFromWindow(window: Hwnd, flags: u32) -> *mut c_void;
    fn GetMonitorInfoW(monitor: *mut c_void, info: *mut MonitorInfo) -> i32;
    fn ClientToScreen(window: Hwnd, point: *mut Point) -> i32;
    fn ScreenToClient(window: Hwnd, point: *mut Point) -> i32;
    fn GetMenu(window: Hwnd) -> *mut c_void;
    fn SetWindowLongPtrW(window: Hwnd, index: i32, value: isize) -> isize;
    fn SetPropW(window: Hwnd, name: *const u16, data: *mut c_void) -> i32;
}

#[repr(C)]
struct Margins {
    left: i32,
    right: i32,
    top: i32,
    bottom: i32,
}

#[link(name = "dwmapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn DwmExtendFrameIntoClientArea(window: Hwnd, margins: *const Margins) -> i32;
}

#[link(name = "shcore", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForMonitor(monitor: *mut c_void, kind: i32, x: *mut u32, y: *mut u32) -> i32;
}

const WS_POPUP: u32 = 0x8000_0000;
const WS_CAPTION: u32 = 0x00c0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const GWL_STYLE: i32 = -16;
const GWL_EXSTYLE: i32 = -20;
const WS_EX_DLGMODALFRAME: u32 = 0x1;
const WS_EX_WINDOWEDGE: u32 = 0x100;
const WS_EX_CLIENTEDGE: u32 = 0x200;
const WS_EX_STATICEDGE: u32 = 0x2_0000;
const WM_SYSCOMMAND: u32 = 0x0112;
const SC_MAXIMIZE: usize = 0xF030;
const SC_RESTORE: usize = 0xF120;
const SWP_NOSIZE: u32 = 0x1;
const SWP_NOMOVE: u32 = 0x2;
const SWP_NOZORDER: u32 = 0x4;
const SWP_NOACTIVATE: u32 = 0x10;
const SWP_FRAMECHANGED: u32 = 0x20;
const MONITOR_DEFAULTTOPRIMARY: u32 = 1;
const MONITOR_DEFAULTTONEAREST: u32 = 2;
const SM_CXSIZEFRAME: i32 = 32;
const SM_CXPADDEDBORDER: i32 = 92;
const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;
const HTLEFT: isize = 10;
const HTRIGHT: isize = 11;
const HTTOP: isize = 12;
const HTTOPLEFT: isize = 13;
const HTTOPRIGHT: isize = 14;
const HTBOTTOM: isize = 15;
const HTBOTTOMLEFT: isize = 16;
const HTBOTTOMRIGHT: isize = 17;
pub(super) const WM_NCCALCSIZE: u32 = 0x0083;
pub(super) const WM_NCHITTEST: u32 = 0x0084;
pub(super) const WM_GETMINMAXINFO: u32 = 0x0024;
pub(super) const WM_DPICHANGED: u32 = 0x02e0;
pub(super) const WM_ERASEBKGND: u32 = 0x0014;
const WM_WINDOWPOSCHANGED: u32 = 0x0047;
const SWP_SHOWWINDOW: u32 = 0x0040;

#[repr(C)]
struct WindowPos {
    window: Hwnd,
    after: Hwnd,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    flags: u32,
}

/// How a window is created, in device-independent pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowOptions<'a> {
    /// The title, without a terminating NUL.
    pub title: &'a [u16],
    /// The outer top-left corner; `None` centers the window on the primary
    /// display's work area.
    pub position: Option<(f64, f64)>,
    /// The outer size, or the content size with `content_size`.
    pub size: (f64, f64),
    /// Whether `size` excludes the native frame.
    pub content_size: bool,
    /// Whether a window without a `position` is centered again once made,
    /// as Electron's `center` option does.
    pub center: bool,
    /// Whether the window has a native frame, caption and menu bar.
    pub frame: bool,
    /// Whether the user can resize the window.
    pub resizable: bool,
    /// Whether the window is shown when created.
    pub show: bool,
    /// The minimum outer size; zero for none.
    pub minimum: (f64, f64),
    /// The maximum outer size; zero for none.
    pub maximum: (f64, f64),
    /// The color behind the page while it paints.
    pub background: [u8; 3],
    /// Whether the menu bar stays hidden until the Alt key shows it.
    pub auto_hide_menu_bar: bool,
    /// Whether a frameless window shows what lies behind it where its page
    /// paints nothing.
    pub transparent: bool,
    /// Whether the window stays above windows that are not.
    pub always_on_top: bool,
    /// Whether the window has no taskbar button.
    pub skip_taskbar: bool,
}

impl<'a> WindowOptions<'a> {
    /// Electron's defaults: an 800×600 framed, resizable, visible window.
    pub fn new(title: &'a [u16]) -> Self {
        WindowOptions {
            title,
            position: None,
            size: (800.0, 600.0),
            content_size: false,
            center: false,
            frame: true,
            resizable: true,
            show: true,
            minimum: (0.0, 0.0),
            maximum: (0.0, 0.0),
            background: [255, 255, 255],
            auto_hide_menu_bar: false,
            transparent: false,
            always_on_top: false,
            skip_taskbar: false,
        }
    }
}

/// What the host keeps about each window beyond its HWND.
pub(super) struct Frame {
    frameless: AtomicBool,
    /// Whether the menu bar hides until the Alt key shows it.
    auto_hide_menu: AtomicBool,
    /// Whether the menu bar is hidden.
    menu_hidden: AtomicBool,
    /// Minimum and maximum outer width and height, as `f32` bits.
    limits: [AtomicU32; 4],
    background: AtomicU32,
    full_screen: AtomicBool,
    /// Whether the window keeps no taskbar button, which the taskbar adds
    /// again whenever the window shows.
    pub(super) skip_taskbar: AtomicBool,
    /// What full screen replaced: the styles, the outer rectangle and
    /// whether the window was maximized.
    saved_styles: [AtomicU32; 2],
    saved_rect: [AtomicI32; 4],
    saved_maximized: AtomicBool,
    /// Set while full screen changes the window, whose size changes are
    /// not the user's.
    pub(super) changing: AtomicBool,
}

impl Frame {
    const fn new() -> Self {
        Frame {
            frameless: AtomicBool::new(false),
            auto_hide_menu: AtomicBool::new(false),
            menu_hidden: AtomicBool::new(false),
            limits: [const { AtomicU32::new(0) }; 4],
            background: AtomicU32::new(0x00ff_ffff),
            full_screen: AtomicBool::new(false),
            skip_taskbar: AtomicBool::new(false),
            saved_styles: [const { AtomicU32::new(0) }; 2],
            saved_rect: [const { AtomicI32::new(0) }; 4],
            saved_maximized: AtomicBool::new(false),
            changing: AtomicBool::new(false),
        }
    }

    fn limit(&self, index: usize) -> f64 {
        f64::from(f32::from_bits(self.limits[index].load(Ordering::Acquire)))
    }

    fn set_limits(&self, minimum: (f64, f64), maximum: (f64, f64)) {
        for (slot, value) in self
            .limits
            .iter()
            .zip([minimum.0, minimum.1, maximum.0, maximum.1])
        {
            let value = if value.is_finite() && value > 0.0 {
                value as f32
            } else {
                0.0
            };
            slot.store(value.to_bits(), Ordering::Release);
        }
    }
}

pub(super) static FRAMES: [Frame; 32] = [const { Frame::new() }; 32];

/// The slot reserved for the window being created, which binds to its HWND
/// at its first message.
pub(super) static PENDING: AtomicUsize = AtomicUsize::new(usize::MAX);

/// A reserved, not yet bound, window slot.
pub(super) const RESERVED: usize = 1;

/// The slot of a host window, binding the window being created.
pub(super) fn slot(window: Hwnd) -> Option<usize> {
    let raw = window as usize;
    if let Some(index) = HOST_WINDOWS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == raw)
    {
        return Some(index);
    }
    let pending = PENDING.swap(usize::MAX, Ordering::AcqRel);
    let slot = HOST_WINDOWS.get(pending)?;
    slot.store(raw, Ordering::Release);
    Some(pending)
}

pub(super) fn frame(window: Hwnd) -> Option<&'static Frame> {
    slot(window).map(|index| &FRAMES[index])
}

/// Rounds to the nearest integer.
fn round(value: f64) -> i32 {
    crate::float::floor(value + 0.5) as i32
}

fn dpi_scale(dpi: u32) -> f64 {
    if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 }
}

/// A window's device pixels per device-independent pixel.
pub fn scale(window: Window) -> f64 {
    // SAFETY: GetDpiForWindow returns 0 for stale handles.
    dpi_scale(unsafe { GetDpiForWindow(window.0 as Hwnd) })
}

/// A monitor's DPI and its device-pixel area and work area.
fn monitor(monitor: *mut c_void) -> (u32, WinRect, WinRect) {
    let mut info = MonitorInfo {
        size: size_of::<MonitorInfo>() as u32,
        monitor: WinRect::default(),
        work: WinRect::default(),
        flags: 0,
    };
    let (mut dpi, mut unused) = (96, 96);
    // SAFETY: both outputs are writable; null or stale monitors fail.
    unsafe {
        GetMonitorInfoW(monitor, &mut info);
        if GetDpiForMonitor(monitor, 0, &mut dpi, &mut unused) < 0 {
            dpi = 96;
        }
    }
    (dpi, info.monitor, info.work)
}

/// Whether a window is made transparent: only frameless ones are.
fn transparent(options: &WindowOptions<'_>) -> bool {
    options.transparent && !options.frame
}

/// The extended window style of a window `options` describe.
pub(super) fn extended_style(options: &WindowOptions<'_>) -> u32 {
    const WS_EX_TOPMOST: u32 = 0x8;
    if options.always_on_top {
        WS_EX_TOPMOST
    } else {
        0
    }
}

/// Finishes a window `options` describe once it exists. A transparent
/// window's frame covers its client area, which the compositor then blends
/// over what lies behind by the alpha its page paints, as Chromium extends
/// the frame of translucent windows; the window's property tells the page
/// painter so.
pub(super) fn made(window: Hwnd, options: &WindowOptions<'_>) {
    if !transparent(options) {
        return;
    }
    let name: alloc::vec::Vec<u16> = "NativeboatTransparent".encode_utf16().chain([0]).collect();
    let margins = Margins {
        left: -1,
        right: -1,
        top: -1,
        bottom: -1,
    };
    // SAFETY: the window is live, the name NUL-terminated and the margins
    // readable.
    unsafe {
        SetPropW(window, name.as_ptr(), core::ptr::without_provenance_mut(1));
        DwmExtendFrameIntoClientArea(window, &margins);
    }
}

/// Removes the taskbar button the taskbar gave a window that keeps none.
fn shown(window: Hwnd) {
    if frame(window).is_some_and(|frame| frame.skip_taskbar.load(Ordering::Acquire)) {
        super::taskbar::set_button(window, false);
    }
}

fn style(options: &WindowOptions<'_>) -> u32 {
    let mut style = if options.frame {
        super::WS_OVERLAPPEDWINDOW
    } else {
        WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX | WS_THICKFRAME | WS_MAXIMIZEBOX
    };
    if !options.resizable {
        style &= !(WS_THICKFRAME | WS_MAXIMIZEBOX);
    }
    style
}

/// Grows a content rectangle to the outer rectangle of a window style.
fn outer(content: (i32, i32), style: u32, menu: bool, dpi: u32) -> (i32, i32) {
    if style & WS_CAPTION == 0 {
        return content;
    }
    let mut rect = WinRect {
        left: 0,
        top: 0,
        right: content.0,
        bottom: content.1,
    };
    // SAFETY: `rect` is writable.
    unsafe { AdjustWindowRectExForDpi(&mut rect, style, i32::from(menu), 0, dpi) };
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// The size clamped to the minimum and maximum sizes, where set.
fn clamped(size: (f64, f64), minimum: (f64, f64), maximum: (f64, f64)) -> (f64, f64) {
    let clamp = |value: f64, low: f64, high: f64| {
        let value = if high > 0.0 { value.min(high) } else { value };
        value.max(low)
    };
    (
        clamp(size.0, minimum.0, maximum.0),
        clamp(size.1, minimum.1, maximum.1),
    )
}

/// Creates a window as `options` describe: its window style, its outer
/// rectangle in device pixels and the slot it binds to.
pub(super) fn prepare(options: &WindowOptions<'_>, slot: usize) -> (u32, WinRect) {
    let frame = &FRAMES[slot];
    frame.frameless.store(!options.frame, Ordering::Release);
    // A menu bar that hides itself starts hidden, as in Electron.
    frame
        .auto_hide_menu
        .store(options.auto_hide_menu_bar, Ordering::Release);
    frame
        .menu_hidden
        .store(options.auto_hide_menu_bar, Ordering::Release);
    frame.set_limits(options.minimum, options.maximum);
    frame
        .skip_taskbar
        .store(options.skip_taskbar, Ordering::Release);
    // A transparent window's frame shows through where it is black, which
    // is what it erases to.
    let [red, green, blue] = if transparent(options) {
        [0; 3]
    } else {
        options.background
    };
    frame.background.store(
        u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16),
        Ordering::Release,
    );
    let style = style(options);
    let size = clamped(options.size, options.minimum, options.maximum);
    // SAFETY: MonitorFromPoint has no pointer arguments.
    let (primary_dpi, _, primary_work) =
        monitor(unsafe { MonitorFromPoint(Point::default(), MONITOR_DEFAULTTOPRIMARY) });
    let (dpi, origin) = match options.position {
        Some((x, y)) => {
            // The display nearest the point, as the primary display scales it.
            let guess = dpi_scale(primary_dpi);
            let point = Point {
                x: round(x * guess),
                y: round(y * guess),
            };
            // SAFETY: MonitorFromPoint has no pointer arguments.
            let (dpi, _, _) = monitor(unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) });
            // Electron reads the position as whole numbers.
            (dpi, Some((x as i32, y as i32)))
        }
        None => (primary_dpi, None),
    };
    let scale = dpi_scale(dpi) as f32;
    let device = (ceiled(size.0 as f32, scale), ceiled(size.1 as f32, scale));
    let size = if options.content_size {
        outer(device, style, false, dpi)
    } else {
        device
    };
    (
        style,
        placed(size, primary_work, origin, options.center, scale),
    )
}

/// Where Electron puts a new window of `size` device pixels. Chromium
/// centers it on the work area; Electron then moves it to `origin`, given in
/// device-independent pixels, or centers it again when asked. Both go through
/// the window's size in device-independent pixels, which can grow it by a
/// pixel each time: Electron calls `SetPosition` twice.
fn placed(
    size: (i32, i32),
    work: WinRect,
    origin: Option<(i32, i32)>,
    center: bool,
    scale: f32,
) -> WinRect {
    let rect = centered(work, size);
    match origin {
        Some((x, y)) => (0..2).fold(rect, |rect, _| {
            let (_, _, width, height) = logical(rect, scale);
            device_rect((x, y, width, height), scale)
        }),
        None if center => recentered(rect, work, scale),
        None => rect,
    }
}

/// A length in device-independent pixels as whole device pixels, rounded
/// up, as Chromium's `ScaleToCeiledSize` scales sizes in `f32`.
fn ceiled(length: f32, scale: f32) -> i32 {
    crate::float::ceil(f64::from(length * scale)) as i32
}

/// The device pixels a span from `start` of `length` covers at `scale`: its
/// first pixel and its count, as Chromium's `ScaleToEnclosingRect` finds them
/// in `f32`.
fn enclosing(start: i32, length: i32, scale: f32) -> (i32, i32) {
    let low = crate::float::floor(f64::from(start as f32 * scale)) as i32;
    if length == 0 {
        return (low, 0);
    }
    let high = crate::float::ceil(f64::from((start + length) as f32 * scale)) as i32;
    (low, high - low)
}

/// A rectangle in device-independent pixels in device pixels, as Chromium's
/// `DIPToScreenRect` converts window bounds: its size is that of the pixels
/// it covers, so it depends on where the rectangle lies.
fn device_rect((x, y, width, height): (i32, i32, i32, i32), scale: f32) -> WinRect {
    let (left, width) = enclosing(x, width, scale);
    let (top, height) = enclosing(y, height, scale);
    WinRect {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

/// A rectangle of `size` centered on `work`, or filling it where it is
/// larger, as Chromium's `CenterAndSizeWindow` places windows.
fn centered(work: WinRect, size: (i32, i32)) -> WinRect {
    let axis = |low: i32, high: i32, length: i32| {
        if length < high - low {
            let start = low + (high - low - length) / 2;
            (start, start + length)
        } else {
            (low, high)
        }
    };
    let (left, right) = axis(work.left, work.right, size.0);
    let (top, bottom) = axis(work.top, work.bottom, size.1);
    WinRect {
        left,
        top,
        right,
        bottom,
    }
}

/// A window centered on `work` again, as Electron's `center` does: at its
/// size in device-independent pixels, scaled back up and rounded up.
fn recentered(rect: WinRect, work: WinRect, scale: f32) -> WinRect {
    let (_, _, width, height) = logical(rect, scale);
    centered(
        work,
        (ceiled(width as f32, scale), ceiled(height as f32, scale)),
    )
}

/// Whether a window has a native frame.
pub fn has_frame(window: Window) -> bool {
    frame(window.0 as Hwnd).is_none_or(|frame| !frame.frameless.load(Ordering::Acquire))
}

/// Whether a window shows its menu as a menu bar: it has a frame and the
/// bar is not hidden.
pub fn shows_menu_bar(window: Window) -> bool {
    has_frame(window)
        && frame(window.0 as Hwnd).is_none_or(|frame| !frame.menu_hidden.load(Ordering::Acquire))
}

/// Whether a window's menu bar hides until the Alt key shows it.
pub fn is_menu_bar_auto_hide(window: Window) -> bool {
    frame(window.0 as Hwnd).is_some_and(|frame| frame.auto_hide_menu.load(Ordering::Acquire))
}

/// Sets whether a window's menu bar hides until the Alt key shows it; the
/// bar stays as it is until then, as in Electron.
pub fn set_menu_bar_auto_hide(window: Window, auto_hide: bool) {
    if let Some(frame) = frame(window.0 as Hwnd) {
        frame.auto_hide_menu.store(auto_hide, Ordering::Release);
    }
}

/// Notes whether a window's menu bar is hidden; `true` when that changed.
pub(super) fn note_menu_hidden(window: Window, hidden: bool) -> bool {
    frame(window.0 as Hwnd)
        .is_some_and(|frame| frame.menu_hidden.swap(hidden, Ordering::AcqRel) != hidden)
}

/// A window's outer bounds: x, y, width and height.
pub fn window_bounds(window: Window) -> Option<(i32, i32, i32, i32)> {
    let mut rect = WinRect::default();
    // SAFETY: `rect` is writable; stale handles fail.
    if unsafe { GetWindowRect(window.0 as Hwnd, &mut rect) } == 0 {
        return None;
    }
    Some(logical(rect, scale(window) as f32))
}

/// A window's content bounds on the screen: x, y, width and height.
pub fn content_bounds(window: Window) -> Option<(i32, i32, i32, i32)> {
    let hwnd = window.0 as Hwnd;
    let mut rect = WinRect::default();
    let mut origin = Point::default();
    // SAFETY: both outputs are writable; stale handles fail.
    let found =
        unsafe { GetClientRect(hwnd, &mut rect) != 0 && ClientToScreen(hwnd, &mut origin) != 0 };
    if !found {
        return None;
    }
    Some(logical(
        WinRect {
            left: origin.x,
            top: origin.y,
            right: origin.x + rect.right,
            bottom: origin.y + rect.bottom,
        },
        scale(window) as f32,
    ))
}

/// A rectangle in device pixels in device-independent pixels, as Chromium's
/// `ScreenToDIPRect` reports window bounds: its corner rounded down, and the
/// size of the device-independent pixels it covers.
fn logical(rect: WinRect, scale: f32) -> (i32, i32, i32, i32) {
    let corner = |value: i32| crate::float::floor(f64::from(value as f32 / scale)) as i32;
    let (_, width) = enclosing(rect.left, rect.right - rect.left, 1.0 / scale);
    let (_, height) = enclosing(rect.top, rect.bottom - rect.top, 1.0 / scale);
    (corner(rect.left), corner(rect.top), width, height)
}

/// Returns a window's current client area in whole device-independent
/// pixels, each side rounded up, as Chromium gives it to the page.
pub fn client_size(window: Window) -> Option<(f32, f32)> {
    let mut rect = WinRect::default();
    // SAFETY: `rect` is writable; GetClientRect rejects stale handles.
    if unsafe { GetClientRect(window.0 as Hwnd, &mut rect) } == 0 {
        return None;
    }
    let inverse = 1.0 / scale(window) as f32;
    Some((
        ceiled((rect.right - rect.left) as f32, inverse) as f32,
        ceiled((rect.bottom - rect.top) as f32, inverse) as f32,
    ))
}

fn place(window: Window, rect: Option<(i32, i32)>, size: Option<(i32, i32)>) {
    let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
    let (x, y) = rect.unwrap_or_else(|| {
        flags |= SWP_NOMOVE;
        (0, 0)
    });
    let (width, height) = size.unwrap_or_else(|| {
        flags |= SWP_NOSIZE;
        (0, 0)
    });
    // SAFETY: SetWindowPos fails harmlessly for stale handles.
    unsafe {
        SetWindowPos(
            window.0 as Hwnd,
            core::ptr::null_mut(),
            x,
            y,
            width,
            height,
            flags,
        )
    };
}

/// Moves and sizes a window's outer bounds, given in whole
/// device-independent pixels as Electron reads them.
pub fn set_window_bounds(window: Window, x: f64, y: f64, width: f64, height: f64) {
    let rect = device_rect(
        (x as i32, y as i32, width as i32, height as i32),
        scale(window) as f32,
    );
    place(
        window,
        Some((rect.left, rect.top)),
        Some((rect.right - rect.left, rect.bottom - rect.top)),
    );
}

/// Sizes a window so that its content has the given size.
pub fn set_content_size(window: Window, width: f64, height: f64) {
    let hwnd = window.0 as Hwnd;
    // SAFETY: these queries accept stale handles.
    let (style, menu, dpi) = unsafe {
        (
            GetWindowLongPtrW(hwnd, GWL_STYLE) as u32,
            !GetMenu(hwnd).is_null(),
            GetDpiForWindow(hwnd),
        )
    };
    let scale = dpi_scale(dpi);
    let content = (round(width * scale), round(height * scale));
    place(window, None, Some(outer(content, style, menu, dpi)));
}

/// Centers a window on the work area of its display, as Electron does: at
/// its size in device-independent pixels, which may grow it by a pixel.
pub fn center(window: Window) {
    let hwnd = window.0 as Hwnd;
    // SAFETY: MonitorFromWindow accepts stale handles.
    let (_, _, work) = monitor(unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) });
    let mut rect = WinRect::default();
    // SAFETY: `rect` is writable; stale handles fail.
    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return;
    }
    let rect = recentered(rect, work, scale(window) as f32);
    place(
        window,
        Some((rect.left, rect.top)),
        Some((rect.right - rect.left, rect.bottom - rect.top)),
    );
}

/// A window's minimum and maximum outer sizes; zero where unset.
pub fn size_limits(window: Window) -> ((f64, f64), (f64, f64)) {
    frame(window.0 as Hwnd).map_or(((0.0, 0.0), (0.0, 0.0)), |frame| {
        (
            (frame.limit(0), frame.limit(1)),
            (frame.limit(2), frame.limit(3)),
        )
    })
}

/// Sets a window's minimum and maximum outer sizes; zero for none.
pub fn set_size_limits(window: Window, minimum: (f64, f64), maximum: (f64, f64)) {
    if let Some(frame) = frame(window.0 as Hwnd) {
        frame.set_limits(minimum, maximum);
    }
}

/// Whether the user can resize a window.
pub fn is_resizable(window: Window) -> bool {
    // SAFETY: GetWindowLongPtrW returns 0 for stale handles.
    let style = unsafe { GetWindowLongPtrW(window.0 as Hwnd, GWL_STYLE) } as u32;
    style & WS_THICKFRAME != 0
}

/// Lets the user resize a window, or not.
pub fn set_resizable(window: Window, resizable: bool) {
    let hwnd = window.0 as Hwnd;
    // SAFETY: style changes fail harmlessly for stale handles.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        let changed = if resizable {
            style | WS_THICKFRAME | WS_MAXIMIZEBOX
        } else {
            style & !(WS_THICKFRAME | WS_MAXIMIZEBOX)
        };
        SetWindowLongPtrW(hwnd, GWL_STYLE, changed as i32 as isize);
        SetWindowPos(
            hwnd,
            core::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

/// Callback answering whether a point of a frameless window, in
/// device-independent client coordinates, drags the window.
pub type DragHandler = unsafe extern "system" fn(*mut c_void, usize, f32, f32) -> bool;

static DRAG_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static DRAG_HANDLER: AtomicUsize = AtomicUsize::new(0);

/// Installs the callback that finds the drag regions of frameless windows.
/// The context must stay valid until the message loop exits.
pub fn set_drag_handler(context: *mut c_void, handler: Option<DragHandler>) {
    DRAG_CONTEXT.store(context as usize, Ordering::Release);
    DRAG_HANDLER.store(
        handler.map_or(0, |handler| handler as usize),
        Ordering::Release,
    );
}

fn drags(window: Hwnd, x: f32, y: f32) -> bool {
    let handler = DRAG_HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return false;
    }
    // SAFETY: installed with this ABI by `set_drag_handler`.
    let handler = unsafe { core::mem::transmute::<usize, DragHandler>(handler) };
    let context = DRAG_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the generated application keeps the context alive.
    unsafe { handler(context, window as usize, x, y) }
}

/// Puts a window into full screen, without its frame and covering its
/// monitor, or back as it was, as Chromium's full screen handler does.
/// Returns whether the window changed.
pub fn set_full_screen(window: Window, full: bool) -> bool {
    let hwnd = window.0 as Hwnd;
    let Some(frame) = frame(hwnd) else {
        return false;
    };
    if frame.full_screen.load(Ordering::Acquire) == full {
        return false;
    }
    frame.changing.store(true, Ordering::Release);
    // SAFETY: `frame` found the handle among the live host windows; the
    // rectangle is writable.
    unsafe {
        if full {
            let maximized = IsZoomed(hwnd) != 0;
            frame.saved_maximized.store(maximized, Ordering::Release);
            // Restored first, the window returns to its normal bounds.
            if maximized {
                SendMessageW(hwnd, WM_SYSCOMMAND, SC_RESTORE, 0);
            }
            let mut rect = WinRect::default();
            GetWindowRect(hwnd, &mut rect);
            for (slot, value) in
                frame
                    .saved_rect
                    .iter()
                    .zip([rect.left, rect.top, rect.right, rect.bottom])
            {
                slot.store(value, Ordering::Release);
            }
            let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
            let extended = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            frame.saved_styles[0].store(style, Ordering::Release);
            frame.saved_styles[1].store(extended, Ordering::Release);
            frame.full_screen.store(true, Ordering::Release);
            SetWindowLongPtrW(
                hwnd,
                GWL_STYLE,
                (style & !(WS_CAPTION | WS_THICKFRAME)) as isize,
            );
            let edges =
                WS_EX_DLGMODALFRAME | WS_EX_WINDOWEDGE | WS_EX_CLIENTEDGE | WS_EX_STATICEDGE;
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (extended & !edges) as isize);
            let (_, area, _) = monitor(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST));
            SetWindowPos(
                hwnd,
                core::ptr::null_mut(),
                area.left,
                area.top,
                area.right - area.left,
                area.bottom - area.top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        } else {
            frame.full_screen.store(false, Ordering::Release);
            let [style, extended] =
                [0, 1].map(|index| frame.saved_styles[index].load(Ordering::Acquire));
            SetWindowLongPtrW(hwnd, GWL_STYLE, style as isize);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended as isize);
            let [left, top, right, bottom] =
                [0, 1, 2, 3].map(|index| frame.saved_rect[index].load(Ordering::Acquire));
            SetWindowPos(
                hwnd,
                core::ptr::null_mut(),
                left,
                top,
                right - left,
                bottom - top,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
            if frame.saved_maximized.load(Ordering::Acquire) {
                SendMessageW(hwnd, WM_SYSCOMMAND, SC_MAXIMIZE, 0);
            }
        }
    }
    frame.changing.store(false, Ordering::Release);
    true
}

/// Whether a window is in full screen.
pub fn is_full_screen(window: Window) -> bool {
    frame(window.0 as Hwnd).is_some_and(|frame| frame.full_screen.load(Ordering::Acquire))
}

/// What a point of a frameless window is: a resize border, a drag region
/// or content. Full screen windows neither resize nor move.
fn hit_test(window: Hwnd, lparam: isize) -> isize {
    if frame(window).is_some_and(|frame| frame.full_screen.load(Ordering::Acquire)) {
        return HTCLIENT;
    }
    let mut point = Point {
        x: i32::from(lparam as u16 as i16),
        y: i32::from((lparam >> 16) as u16 as i16),
    };
    let mut rect = WinRect::default();
    // SAFETY: both outputs are writable; the window is live.
    unsafe {
        ScreenToClient(window, &mut point);
        GetClientRect(window, &mut rect);
    }
    // SAFETY: these queries accept any HWND.
    let (style, zoomed, dpi) = unsafe {
        (
            GetWindowLongPtrW(window, GWL_STYLE) as u32,
            IsZoomed(window) != 0,
            GetDpiForWindow(window),
        )
    };
    if style & WS_THICKFRAME != 0 && !zoomed {
        // SAFETY: GetSystemMetricsForDpi takes scalar arguments.
        let border = unsafe {
            GetSystemMetricsForDpi(SM_CXSIZEFRAME, dpi)
                + GetSystemMetricsForDpi(SM_CXPADDEDBORDER, dpi)
        };
        let column = if point.x < border {
            0
        } else if point.x >= rect.right - border {
            2
        } else {
            1
        };
        let row = if point.y < border {
            0
        } else if point.y >= rect.bottom - border {
            2
        } else {
            1
        };
        let code = [
            [HTTOPLEFT, HTTOP, HTTOPRIGHT],
            [HTLEFT, HTCLIENT, HTRIGHT],
            [HTBOTTOMLEFT, HTBOTTOM, HTBOTTOMRIGHT],
        ][row][column];
        if code != HTCLIENT {
            return code;
        }
    }
    let scale = dpi_scale(dpi);
    let (x, y) = (
        (f64::from(point.x) / scale) as f32,
        (f64::from(point.y) / scale) as f32,
    );
    if drags(window, x, y) {
        HTCAPTION
    } else {
        HTCLIENT
    }
}

/// Answers the messages that window geometry depends on.
pub(super) fn message(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> Option<isize> {
    let frame = frame(window)?;
    let frameless = frame.frameless.load(Ordering::Acquire);
    match message {
        WM_WINDOWPOSCHANGED => {
            // SAFETY: the message's lparam points at the new placement.
            let flags = unsafe { (*(lparam as *const WindowPos)).flags };
            if flags & SWP_SHOWWINDOW != 0 {
                shown(window);
            }
            // Windows still derives the size and move messages from it.
            None
        }
        // The whole window is client area.
        WM_NCCALCSIZE if frameless => Some(0),
        WM_NCHITTEST if frameless => Some(hit_test(window, lparam)),
        WM_GETMINMAXINFO => {
            // SAFETY: WM_GETMINMAXINFO passes a writable MINMAXINFO.
            let info = unsafe { &mut *(lparam as *mut MinMaxInfo) };
            // SAFETY: these queries accept any HWND.
            let (dpi, nearest) = unsafe {
                (
                    GetDpiForWindow(window),
                    MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
                )
            };
            let scale = dpi_scale(dpi);
            let px = |value: f64| round(value * scale);
            let tracks = [
                &mut info.min_track.x,
                &mut info.min_track.y,
                &mut info.max_track.x,
                &mut info.max_track.y,
            ];
            for (index, track) in tracks.into_iter().enumerate() {
                if frame.limit(index) > 0.0 {
                    *track = px(frame.limit(index));
                }
            }
            if frameless {
                // Maximized frameless windows fill the work area, not the
                // whole monitor.
                let (_, area, work) = monitor(nearest);
                info.max_position = Point {
                    x: work.left - area.left,
                    y: work.top - area.top,
                };
                info.max_size = Point {
                    x: work.right - work.left,
                    y: work.bottom - work.top,
                };
            }
            Some(0)
        }
        WM_DPICHANGED => {
            // SAFETY: WM_DPICHANGED passes the suggested window rectangle.
            let rect = unsafe { &*(lparam as *const WinRect) };
            place(
                Window(window as usize),
                Some((rect.left, rect.top)),
                Some((rect.right - rect.left, rect.bottom - rect.top)),
            );
            // SAFETY: the window is live.
            unsafe { InvalidateRect(window, core::ptr::null(), 0) };
            Some(0)
        }
        WM_ERASEBKGND => {
            let mut rect = WinRect::default();
            // SAFETY: `rect` is writable and the DC is the one being erased.
            unsafe { GetClientRect(window, &mut rect) };
            let color = frame.background.load(Ordering::Acquire);
            let dc = super::PaintDc(wparam as *mut c_void);
            dc.fill_rect(
                0.0,
                0.0,
                rect.right as f32,
                rect.bottom as f32,
                [color as u8, (color >> 8) as u8, (color >> 16) as u8],
            );
            Some(1)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
