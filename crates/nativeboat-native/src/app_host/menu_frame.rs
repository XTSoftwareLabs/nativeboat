//! Popup menus framed as Chromium frames Electron's menus on Windows: no
//! window border, 12-dip rounded corners and a soft two-layer shadow, both
//! antialiased. The menu window itself is clipped to the pixels its rounded
//! body covers fully; a layered window just below it shows the shadow and
//! the partly covered pixels of the corners.

use alloc::vec::Vec;

/// The corner radius of Chromium's menus, in dip.
pub(super) const RADIUS: f64 = 12.0;

/// Chromium's shadow base color, Google Grey 800.
const SHADOW: [f64; 3] = [0x3c as f64, 0x40 as f64, 0x43 as f64];

/// One layer of the shadow: the body moved down by `offset` and blurred by
/// a Gaussian of `sigma`, both in dip, at `alpha`.
struct Layer {
    offset: f64,
    sigma: f64,
    alpha: f64,
}

/// The shadow measured from Electron 38's menus: a tight layer and a wide
/// one that falls below.
const LAYERS: [Layer; 2] = [
    Layer {
        offset: 0.65,
        sigma: 2.45,
        alpha: 0.095,
    },
    Layer {
        offset: 12.4,
        sigma: 6.0,
        alpha: 0.207,
    },
];

/// How far the shadow reaches beyond the body, in device pixels: left and
/// right, above, and below.
pub(super) fn margins(scale: f64) -> (i32, i32, i32) {
    let reach =
        |layer: &Layer, offset: f64| crate::libm::ceil((layer.sigma * 3.0 + offset) * scale) as i32;
    let side = LAYERS
        .iter()
        .map(|layer| reach(layer, 0.0))
        .max()
        .unwrap_or(0);
    let above = LAYERS
        .iter()
        .map(|layer| reach(layer, -layer.offset))
        .max()
        .unwrap_or(0)
        .max(0);
    let below = LAYERS
        .iter()
        .map(|layer| reach(layer, layer.offset))
        .max()
        .unwrap_or(0);
    (side, above, below)
}

/// How much of the pixel at `x`, `y` a rounded rectangle at `left`, `top`
/// of `width` × `height` with corners of `radius` covers, as Chromium's
/// GPU raster covers it: `r - d + 0.5` at the corners.
pub(super) fn coverage(
    x: f64,
    y: f64,
    (left, top, width, height): (f64, f64, f64, f64),
    radius: f64,
) -> f64 {
    let (x, y) = (x + 0.5, y + 0.5);
    let (right, bottom) = (left + width, top + height);
    let mut covered = (x - left + 0.5)
        .min(right - x + 0.5)
        .min(y - top + 0.5)
        .min(bottom - y + 0.5);
    let centre_x = if x < left + radius {
        Some(left + radius)
    } else if x > right - radius {
        Some(right - radius)
    } else {
        None
    };
    let centre_y = if y < top + radius {
        Some(top + radius)
    } else if y > bottom - radius {
        Some(bottom - radius)
    } else {
        None
    };
    if let (Some(centre_x), Some(centre_y)) = (centre_x, centre_y) {
        let distance =
            crate::libm::sqrt((x - centre_x) * (x - centre_x) + (y - centre_y) * (y - centre_y));
        covered = covered.min(radius - distance + 0.5);
    }
    covered.clamp(0.0, 1.0)
}

/// A Gaussian kernel of `sigma` device pixels, normalized.
fn kernel(sigma: f64) -> Vec<f64> {
    let reach = crate::libm::ceil(sigma * 3.0) as i32;
    let weights: Vec<f64> = (-reach..=reach)
        .map(|offset| crate::libm::exp(-f64::from(offset * offset) / (2.0 * sigma * sigma)))
        .collect();
    let total: f64 = weights.iter().sum();
    weights.into_iter().map(|weight| weight / total).collect()
}

/// `values`, `width` across, blurred by `kernel` along rows, then columns.
fn blur(values: &[f64], width: usize, kernel: &[f64]) -> Vec<f64> {
    let height = values.len() / width;
    let reach = (kernel.len() / 2) as isize;
    let pass = |source: &[f64], across: bool| -> Vec<f64> {
        let mut out = alloc::vec![0.0; source.len()];
        for y in 0..height {
            for x in 0..width {
                let mut sum = 0.0;
                for (index, weight) in kernel.iter().enumerate() {
                    let step = index as isize - reach;
                    let (sx, sy) = if across {
                        (x as isize + step, y as isize)
                    } else {
                        (x as isize, y as isize + step)
                    };
                    if sx >= 0 && sy >= 0 && (sx as usize) < width && (sy as usize) < height {
                        sum += weight * source[sy as usize * width + sx as usize];
                    }
                }
                out[y * width + x] = sum;
            }
        }
        out
    };
    pass(&pass(values, true), false)
}

/// The picture of a menu's frame: its shadow and its rounded body in
/// `background`, premultiplied BGRA, for a body of `width` ×
/// `height` device pixels whose top left corner lies at the margins.
pub(super) fn backdrop(
    width: i32,
    height: i32,
    scale: f64,
    background: [u8; 3],
) -> (i32, i32, Vec<u8>) {
    let (side, above, below) = margins(scale);
    let (full_width, full_height) = (width + 2 * side, height + above + below);
    let count = (full_width * full_height) as usize;
    let radius = RADIUS * scale;
    let body = (
        f64::from(side),
        f64::from(above),
        f64::from(width),
        f64::from(height),
    );
    // The shadow's transparency, layer over layer.
    let mut clear = alloc::vec![1.0; count];
    for layer in &LAYERS {
        let moved = (body.0, body.1 + layer.offset * scale, body.2, body.3);
        let mask: Vec<f64> = (0..count)
            .map(|index| {
                let (x, y) = (
                    (index as i32 % full_width) as f64,
                    (index as i32 / full_width) as f64,
                );
                coverage(x, y, moved, radius)
            })
            .collect();
        let blurred = blur(&mask, full_width as usize, &kernel(layer.sigma * scale));
        for (left, value) in clear.iter_mut().zip(blurred) {
            *left *= 1.0 - layer.alpha * value;
        }
    }
    let [body_red, body_green, body_blue] = background.map(f64::from);
    let mut pixels = Vec::with_capacity(count * 4);
    for (index, left) in clear.into_iter().enumerate() {
        let (x, y) = (
            (index as i32 % full_width) as f64,
            (index as i32 / full_width) as f64,
        );
        let shadow = 1.0 - left;
        let cover = coverage(x, y, body, radius);
        // The body over the shadow, premultiplied.
        let alpha = cover + shadow * (1.0 - cover);
        let channel =
            |body: f64, shadow_color: f64| body * cover + shadow_color * shadow * (1.0 - cover);
        let [blue, green, red] = [
            channel(body_blue, SHADOW[2]),
            channel(body_green, SHADOW[1]),
            channel(body_red, SHADOW[0]),
        ];
        let byte = |value: f64| (value + 0.5).clamp(0.0, 255.0) as u8;
        pixels.extend([byte(blue), byte(green), byte(red), byte(alpha * 255.0)]);
    }
    (full_width, full_height, pixels)
}

/// The rows of the body the menu window keeps: for each row, the first and
/// last pixel past the end it covers fully, as a window region's spans.
pub(super) fn region_spans(width: i32, height: i32, scale: f64) -> Vec<(i32, i32, i32)> {
    let radius = RADIUS * scale;
    let body = (0.0, 0.0, f64::from(width), f64::from(height));
    (0..height)
        .filter_map(|y| {
            let first =
                (0..width).find(|x| coverage(f64::from(*x), f64::from(y), body, radius) >= 1.0)?;
            let last = (0..width)
                .rev()
                .find(|x| coverage(f64::from(*x), f64::from(y), body, radius) >= 1.0)?;
            Some((y, first, last + 1))
        })
        .collect()
}

// The windows: menus are subclassed as they are created, while a popup the
// app draws is open.

use super::{Hwnd, WinRect};
use core::ffi::c_void;
use core::ptr::{null, null_mut};
use core::sync::atomic::{AtomicIsize, AtomicU16, AtomicUsize, Ordering};

const WH_CALLWNDPROC: i32 = 4;
const WM_NCCREATE: u32 = 0x0081;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_NCPAINT: u32 = 0x0085;
const WM_SHOWWINDOW: u32 = 0x0018;
const WM_WINDOWPOSCHANGING: u32 = 0x0046;
const WM_WINDOWPOSCHANGED: u32 = 0x0047;
const WM_NCDESTROY: u32 = 0x0082;
const GWLP_WNDPROC: i32 = -4;
const GCL_STYLE: i32 = -26;
const CS_DROPSHADOW: usize = 0x0002_0000;
const SWP_NOSIZE: u32 = 0x0001;
const SWP_NOMOVE: u32 = 0x0002;
const SWP_NOACTIVATE: u32 = 0x0010;
const SWP_SHOWWINDOW: u32 = 0x0040;
const SWP_HIDEWINDOW: u32 = 0x0080;
const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const ULW_ALPHA: u32 = 2;
const RGN_OR: i32 = 2;
const DIB_RGB_COLORS: u32 = 0;
const SW_HIDE: i32 = 0;

/// The window properties a framed menu keeps: its original window
/// procedure, the borders it had, and its shadow window.
const PROCEDURE: &str = "NativeBoatMenuProcedure";
const BORDERS: &str = "NativeBoatMenuBorders";
const SHADOW_WINDOW: &str = "NativeBoatMenuShadow";

static HOOK: AtomicIsize = AtomicIsize::new(0);
/// The menu class's style before its drop shadow was turned off.
static CLASS_STYLE: AtomicUsize = AtomicUsize::new(0);
static SHADOW_CLASS: AtomicU16 = AtomicU16::new(0);

#[repr(C)]
struct CallWindowProcedure {
    lparam: isize,
    wparam: usize,
    message: u32,
    window: Hwnd,
}

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

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Blend {
    operation: u8,
    flags: u8,
    alpha: u8,
    format: u8,
}

/// `BITMAPV5HEADER`, as the runtime declares it elsewhere.
#[repr(C)]
#[derive(Default)]
struct BitmapHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pixels_per_meter: i32,
    y_pixels_per_meter: i32,
    colors_used: u32,
    colors_important: u32,
    red_mask: u32,
    green_mask: u32,
    blue_mask: u32,
    alpha_mask: u32,
    color_space: u32,
    endpoints: [i32; 9],
    gamma_red: u32,
    gamma_green: u32,
    gamma_blue: u32,
    intent: u32,
    profile_data: u32,
    profile_size: u32,
    reserved: u32,
}

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetPropW(window: Hwnd, name: *const u16, data: *mut c_void) -> i32;
    fn GetPropW(window: Hwnd, name: *const u16) -> *mut c_void;
    fn RemovePropW(window: Hwnd, name: *const u16) -> *mut c_void;
    fn CallWindowProcW(
        previous: isize,
        window: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize;
    fn GetClassNameW(window: Hwnd, name: *mut u16, length: i32) -> i32;
    fn SetWindowRgn(window: Hwnd, region: *mut c_void, redraw: i32) -> i32;
    fn GetClassLongPtrW(window: Hwnd, index: i32) -> usize;
    fn SetClassLongPtrW(window: Hwnd, index: i32, value: isize) -> usize;
    fn UpdateLayeredWindow(
        window: Hwnd,
        target: *mut c_void,
        position: *const Point,
        size: *const Point,
        source: *mut c_void,
        origin: *const Point,
        key: u32,
        blend: *const Blend,
        flags: u32,
    ) -> i32;
    fn SetWindowsHookExW(id: i32, procedure: HookProc, module: *mut c_void, thread: u32) -> isize;
    fn UnhookWindowsHookEx(hook: isize) -> i32;
    fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
    fn SetWindowLongPtrW(window: Hwnd, index: i32, value: isize) -> isize;
    fn GetDpiForWindow(window: Hwnd) -> u32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateRectRgn(left: i32, top: i32, right: i32, bottom: i32) -> *mut c_void;
    fn CombineRgn(target: *mut c_void, first: *mut c_void, second: *mut c_void, mode: i32) -> i32;
    fn CreateDIBSection(
        dc: *mut c_void,
        info: *const BitmapHeader,
        usage: u32,
        bits: *mut *mut c_void,
        section: *mut c_void,
        offset: u32,
    ) -> *mut c_void;
    fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
    fn DeleteDC(dc: *mut c_void) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Starts framing the menus this thread creates, until [`end`].
pub(super) fn begin() {
    // SAFETY: hooks only this thread's window procedures; `end` removes it.
    let hook =
        unsafe { SetWindowsHookExW(WH_CALLWNDPROC, created, null_mut(), GetCurrentThreadId()) };
    let previous = HOOK.swap(hook, Ordering::Relaxed);
    if previous != 0 {
        // SAFETY: the hook was installed by an earlier `begin`.
        unsafe { UnhookWindowsHookEx(previous) };
    }
}

/// Stops framing menus; the menu class gets its drop shadow back.
pub(super) fn end() {
    let hook = HOOK.swap(0, Ordering::Relaxed);
    if hook != 0 {
        // SAFETY: the hook was installed by `begin`.
        unsafe { UnhookWindowsHookEx(hook) };
    }
}

/// Subclasses each menu window as it is created.
unsafe extern "system" fn created(code: i32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: a WH_CALLWNDPROC hook's lparam is a live CWPSTRUCT.
    let call = unsafe { &*(lparam as *const CallWindowProcedure) };
    if code >= 0 && call.message == WM_NCCREATE && is_menu(call.window) {
        frame(call.window);
    }
    // SAFETY: passes the message on to the next hook.
    unsafe { CallNextHookEx(HOOK.load(Ordering::Relaxed), code, wparam, lparam) }
}

fn is_menu(window: Hwnd) -> bool {
    let mut name = [0u16; 16];
    // SAFETY: the buffer holds 16 units.
    let length = unsafe { GetClassNameW(window, name.as_mut_ptr(), 16) };
    name[..length.max(0) as usize] == *"#32768".encode_utf16().collect::<Vec<u16>>()
}

fn frame(window: Hwnd) {
    // SAFETY: a menu window of this thread, being created; its original
    // procedure is kept to pass messages on and to restore.
    unsafe {
        let style = GetClassLongPtrW(window, GCL_STYLE);
        if style & CS_DROPSHADOW != 0 {
            CLASS_STYLE.store(style, Ordering::Relaxed);
            SetClassLongPtrW(window, GCL_STYLE, (style & !CS_DROPSHADOW) as isize);
        }
        let original = SetWindowLongPtrW(
            window,
            GWLP_WNDPROC,
            framed as super::WndProc as usize as isize,
        );
        SetPropW(window, wide(PROCEDURE).as_ptr(), original as *mut c_void);
    }
}

/// Gives the menu class its drop shadow back, once no menu is framed.
pub(super) fn restore_class(window: Hwnd) {
    let style = CLASS_STYLE.swap(0, Ordering::Relaxed);
    if style != 0 {
        // SAFETY: restores the style read from this class.
        unsafe { SetClassLongPtrW(window, GCL_STYLE, style as isize) };
    }
}

fn property(window: Hwnd, name: &str) -> isize {
    // SAFETY: reads a property of a live window.
    unsafe { GetPropW(window, wide(name).as_ptr()) as isize }
}

/// A framed menu's window procedure: no border, and a shadow that follows
/// it.
unsafe extern "system" fn framed(
    window: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    let original = property(window, PROCEDURE);
    // SAFETY: the original procedure of this menu window.
    let pass = |message, wparam, lparam| unsafe {
        CallWindowProcW(original, window, message, wparam, lparam)
    };
    match message {
        // The menu's border is measured, then dropped: the whole window is
        // its client area.
        WM_NCCALCSIZE => {
            // SAFETY: the first of the RECTs NCCALCSIZE passes is writable.
            let rect = unsafe { &mut *(lparam as *mut WinRect) };
            let before = *rect;
            pass(message, wparam, lparam);
            let borders = [
                rect.left - before.left,
                rect.top - before.top,
                before.right - rect.right,
                before.bottom - rect.bottom,
            ];
            let packed = borders.into_iter().fold(0usize, |packed, border| {
                packed << 8 | border.clamp(0, 255) as usize
            });
            // SAFETY: a property of this live window.
            unsafe { SetPropW(window, wide(BORDERS).as_ptr(), packed as *mut c_void) };
            *rect = before;
            0
        }
        WM_NCPAINT => 0,
        // The menu sizes its window for its border; it is left out.
        WM_WINDOWPOSCHANGING => {
            // SAFETY: WINDOWPOSCHANGING passes a writable WINDOWPOS.
            let position = unsafe { &mut *(lparam as *mut WindowPos) };
            if position.flags & SWP_NOSIZE == 0 {
                let packed = property(window, BORDERS) as usize;
                let [left, top, right, bottom] =
                    [24, 16, 8, 0].map(|shift| (packed >> shift & 255) as i32);
                position.width -= left + right;
                position.height -= top + bottom;
            }
            pass(message, wparam, lparam)
        }
        WM_WINDOWPOSCHANGED => {
            let result = pass(message, wparam, lparam);
            // SAFETY: WINDOWPOSCHANGED passes a WINDOWPOS.
            let flags = unsafe { (*(lparam as *const WindowPos)).flags };
            if flags & SWP_HIDEWINDOW != 0 {
                hide_shadow(window);
            } else {
                place_shadow(window);
            }
            result
        }
        WM_SHOWWINDOW if wparam == 0 => {
            hide_shadow(window);
            pass(message, wparam, lparam)
        }
        WM_NCDESTROY => {
            let shadow = property(window, SHADOW_WINDOW) as Hwnd;
            // SAFETY: the shadow window and properties belong to this menu,
            // whose procedure is put back before it goes.
            unsafe {
                if !shadow.is_null() {
                    super::DestroyWindow(shadow);
                }
                for name in [SHADOW_WINDOW, BORDERS, PROCEDURE] {
                    RemovePropW(window, wide(name).as_ptr());
                }
                SetWindowLongPtrW(window, GWLP_WNDPROC, original);
            }
            restore_class(window);
            // SAFETY: as above.
            unsafe { CallWindowProcW(original, window, message, wparam, lparam) }
        }
        _ => pass(message, wparam, lparam),
    }
}

fn hide_shadow(window: Hwnd) {
    let shadow = property(window, SHADOW_WINDOW) as Hwnd;
    if !shadow.is_null() {
        // SAFETY: the menu's own shadow window.
        unsafe { super::ShowWindow(shadow, SW_HIDE) };
    }
}

/// Clips the menu to its rounded body and shows the shadow and the corners'
/// antialiased edges just below it.
fn place_shadow(window: Hwnd) {
    let mut rect = WinRect::default();
    // SAFETY: a live window; the out rect is writable.
    if unsafe {
        super::IsWindowVisible(window) == 0 || super::GetWindowRect(window, &mut rect) == 0
    } {
        return;
    }
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if width <= 0 || height <= 0 {
        return;
    }
    // SAFETY: a live window.
    let scale = f64::from(unsafe { GetDpiForWindow(window) }.max(96)) / 96.0;
    clip(window, width, height, scale);
    let mut shadow = property(window, SHADOW_WINDOW) as Hwnd;
    if shadow.is_null() {
        shadow = shadow_window();
        if shadow.is_null() {
            return;
        }
        // SAFETY: a property of this live window.
        unsafe { SetPropW(window, wide(SHADOW_WINDOW).as_ptr(), shadow) };
    }
    let (side, above, _) = margins(scale);
    let (full_width, full_height, pixels) =
        backdrop(width, height, scale, super::menu_draw::BACKGROUND_RGB);
    show_layered(
        shadow,
        (rect.left - side, rect.top - above),
        (full_width, full_height),
        &pixels,
    );
    // SAFETY: both windows are live; the shadow goes just below the menu.
    unsafe {
        super::SetWindowPos(
            shadow,
            window,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

/// Clips the menu window to the pixels its rounded body covers fully.
fn clip(window: Hwnd, width: i32, height: i32, scale: f64) {
    // SAFETY: regions are combined into one the window then owns.
    unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        let spans = region_spans(width, height, scale);
        let mut index = 0;
        while index < spans.len() {
            // Rows with the same span join into one rectangle.
            let (top, left, right) = spans[index];
            let mut bottom = top + 1;
            while index + 1 < spans.len() && spans[index + 1] == (bottom, left, right) {
                bottom += 1;
                index += 1;
            }
            let part = CreateRectRgn(left, top, right, bottom);
            CombineRgn(region, region, part, RGN_OR);
            super::DeleteObject(part);
            index += 1;
        }
        SetWindowRgn(window, region, 1);
    }
}

fn shadow_window() -> Hwnd {
    let class = wide(SHADOW_WINDOW);
    // SAFETY: the class name outlives registration and creation; the
    // class's procedure is the default one.
    unsafe {
        let instance = super::GetModuleHandleW(null());
        if SHADOW_CLASS.load(Ordering::Relaxed) == 0 {
            let definition = super::WindowClass {
                style: 0,
                procedure: Some(super::DefWindowProcW),
                class_extra: 0,
                window_extra: 0,
                instance,
                icon: null_mut(),
                cursor: null_mut(),
                background: null_mut(),
                menu_name: null(),
                class_name: class.as_ptr(),
            };
            SHADOW_CLASS.store(super::RegisterClassW(&definition), Ordering::Relaxed);
        }
        super::CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST,
            class.as_ptr(),
            null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null_mut(),
        )
    }
}

/// Shows premultiplied BGRA pixels as a layered window's picture.
fn show_layered(window: Hwnd, (x, y): (i32, i32), (width, height): (i32, i32), pixels: &[u8]) {
    let header = BitmapHeader {
        size: size_of::<BitmapHeader>() as u32,
        width,
        height: -height,
        planes: 1,
        bit_count: 32,
        ..BitmapHeader::default()
    };
    // SAFETY: the DIB holds `width × height` pixels, filled from `pixels`
    // of that size; the GDI objects are released here.
    unsafe {
        let screen = super::GetDC(null_mut());
        let context = CreateCompatibleDC(screen);
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(screen, &header, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !bitmap.is_null() && !context.is_null() {
            core::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());
            let previous = super::SelectObject(context, bitmap);
            let blend = Blend {
                operation: 0,
                flags: 0,
                alpha: 255,
                format: 1,
            };
            UpdateLayeredWindow(
                window,
                screen,
                &Point { x, y },
                &Point {
                    x: width,
                    y: height,
                },
                context,
                &Point { x: 0, y: 0 },
                0,
                &blend,
                ULW_ALPHA,
            );
            super::SelectObject(context, previous);
        }
        if !bitmap.is_null() {
            super::DeleteObject(bitmap);
        }
        if !context.is_null() {
            DeleteDC(context);
        }
        super::ReleaseDC(null_mut(), screen);
    }
}

#[cfg(test)]
mod tests;
