//! Minimal no-std Win32 top-level window and message-loop primitives for the
//! generated Electron host. JavaScript realms and Electron lifecycle events
//! are layered on this boundary; this module owns only operating-system state.

use alloc::vec::Vec;
use core::ffi::c_void;
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering};

type Hwnd = *mut c_void;
type Hinstance = *mut c_void;
type Hdc = *mut c_void;
type GdiObject = *mut c_void;
type WndProc = unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize;

mod frame;
mod menu_draw;
mod menu_frame;
mod notify;
mod taskbar;

pub use frame::{
    DragHandler, WindowOptions, center, client_size, content_bounds, has_frame, is_full_screen,
    is_menu_bar_auto_hide, is_resizable, scale, set_content_size, set_drag_handler,
    set_menu_bar_auto_hide, set_resizable, set_size_limits, set_window_bounds, shows_menu_bar,
    size_limits, window_bounds,
};
pub use notify::{
    BalloonKind, Icon, NotifyEvent, NotifyHandler, add_icon, balloon, cursor, icon_menu, icon_rect,
    notify_scale, on_battery, register_hotkey, remove_icon, set_icon, set_notify_handler, set_tip,
    unregister_hotkey, watch_system,
};

#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WinRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct WinSize {
    width: i32,
    height: i32,
}

#[repr(C)]
struct PaintStruct {
    dc: Hdc,
    erase: i32,
    paint: WinRect,
    restore: i32,
    incremental: i32,
    reserved: [u8; 32],
}

#[repr(C)]
#[allow(non_snake_case)]
#[derive(Default)]
struct WinTextMetrics {
    tmHeight: i32,
    tmAscent: i32,
    tmDescent: i32,
    tmInternalLeading: i32,
    tmExternalLeading: i32,
    tmAveCharWidth: i32,
    tmMaxCharWidth: i32,
    tmWeight: i32,
    tmOverhang: i32,
    tmDigitizedAspectX: i32,
    tmDigitizedAspectY: i32,
    tmFirstChar: u16,
    tmLastChar: u16,
    tmDefaultChar: u16,
    tmBreakChar: u16,
    tmItalic: u8,
    tmUnderlined: u8,
    tmStruckOut: u8,
    tmPitchAndFamily: u8,
    tmCharSet: u8,
}

/// Font properties requested by the native layout text measurer.
#[derive(Debug, Clone, Copy)]
pub struct FontSpec<'a> {
    /// Preferred font face.
    pub family: &'a str,
    /// Size in device-independent pixels.
    pub size: f32,
    /// CSS font weight.
    pub weight: u16,
    /// Whether the face is italic.
    pub italic: bool,
}

/// Text advance and vertical metrics returned by GDI.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MeasuredText {
    /// Horizontal advance in pixels.
    pub width: f32,
    /// Baseline to font top in pixels.
    pub ascent: f32,
    /// Baseline to font bottom in pixels.
    pub descent: f32,
}

#[repr(C)]
struct WindowClass {
    style: u32,
    procedure: Option<WndProc>,
    class_extra: i32,
    window_extra: i32,
    instance: Hinstance,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Accel {
    virt: u8,
    key: u16,
    command: u16,
}

#[repr(C)]
struct Message {
    window: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point_x: i32,
    point_y: i32,
    private: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hinstance;
    fn GetLastError() -> u32;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegGetValueW(
        key: Hinstance,
        subkey: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> i32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassW(class: *const WindowClass) -> u16;
    fn CreateWindowExW(
        extended_style: u32,
        class_name: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: *mut c_void,
        instance: Hinstance,
        parameter: *mut c_void,
    ) -> Hwnd;
    fn ShowWindow(window: Hwnd, command: i32) -> i32;
    fn UpdateWindow(window: Hwnd) -> i32;
    fn DestroyWindow(window: Hwnd) -> i32;
    fn IsWindow(window: Hwnd) -> i32;
    fn IsWindowVisible(window: Hwnd) -> i32;
    fn DefWindowProcW(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
    fn PostQuitMessage(exit_code: i32);
    fn PostMessageW(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> i32;
    fn GetMessageW(message: *mut Message, window: Hwnd, first: u32, last: u32) -> i32;
    fn IsZoomed(window: Hwnd) -> i32;
    fn IsIconic(window: Hwnd) -> i32;
    fn SetForegroundWindow(window: Hwnd) -> i32;
    fn GetForegroundWindow() -> Hwnd;
    fn GetActiveWindow() -> Hwnd;
    fn SetWindowPos(
        window: Hwnd,
        after: Hwnd,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
    ) -> i32;
    fn SetWindowTextW(window: Hwnd, text: *const u16) -> i32;
    fn GetWindowTextW(window: Hwnd, text: *mut u16, length: i32) -> i32;
    fn GetWindowRect(window: Hwnd, rect: *mut WinRect) -> i32;
    fn GetWindowLongPtrW(window: Hwnd, index: i32) -> isize;
    fn PeekMessageW(message: *mut Message, window: Hwnd, first: u32, last: u32, remove: u32)
    -> i32;
    fn MsgWaitForMultipleObjectsEx(
        count: u32,
        handles: *const *mut c_void,
        milliseconds: u32,
        wake_mask: u32,
        flags: u32,
    ) -> u32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> isize;
    fn CreateMenu() -> *mut c_void;
    fn CreatePopupMenu() -> *mut c_void;
    fn AppendMenuW(menu: *mut c_void, flags: u32, item: usize, text: *const u16) -> i32;
    fn SetMenu(window: Hwnd, menu: *mut c_void) -> i32;
    fn DrawMenuBar(window: Hwnd) -> i32;
    fn DestroyMenu(menu: *mut c_void) -> i32;
    fn CreateAcceleratorTableW(entries: *const Accel, count: i32) -> *mut c_void;
    fn DestroyAcceleratorTable(table: *mut c_void) -> i32;
    fn TranslateAcceleratorW(window: Hwnd, table: *mut c_void, message: *const Message) -> i32;
    fn GetAncestor(window: Hwnd, flags: u32) -> Hwnd;
    fn GetKeyState(key: i32) -> i16;
    fn TrackPopupMenuEx(
        menu: *mut c_void,
        flags: u32,
        x: i32,
        y: i32,
        owner: Hwnd,
        parameters: *const c_void,
    ) -> u32;
    fn GetFocus() -> Hwnd;
    fn ClientToScreen(window: Hwnd, point: *mut frame::Point) -> i32;
    fn GetCursorPos(point: *mut frame::Point) -> i32;
    fn SendMessageW(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
    fn GetClientRect(window: Hwnd, rect: *mut WinRect) -> i32;
    fn GetDC(window: Hwnd) -> Hdc;
    fn ReleaseDC(window: Hwnd, dc: Hdc) -> i32;
    fn BeginPaint(window: Hwnd, paint: *mut PaintStruct) -> Hdc;
    fn EndPaint(window: Hwnd, paint: *const PaintStruct) -> i32;
    fn InvalidateRect(window: Hwnd, rect: *const WinRect, erase: i32) -> i32;
    // User32 fills rectangles with GDI brushes.
    fn FillRect(dc: Hdc, rect: *const WinRect, brush: GdiObject) -> i32;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike_out: u32,
        char_set: u32,
        output_precision: u32,
        clip_precision: u32,
        quality: u32,
        pitch_and_family: u32,
        face: *const u16,
    ) -> GdiObject;
    fn SelectObject(dc: Hdc, object: GdiObject) -> GdiObject;
    fn DeleteObject(object: GdiObject) -> i32;
    fn GetTextExtentPoint32W(dc: Hdc, text: *const u16, length: i32, size: *mut WinSize) -> i32;
    fn GetTextMetricsW(dc: Hdc, metrics: *mut WinTextMetrics) -> i32;
    fn CreateSolidBrush(color: u32) -> GdiObject;
    fn SetTextColor(dc: Hdc, color: u32) -> u32;
    fn SetBkMode(dc: Hdc, mode: i32) -> i32;
    fn TextOutW(dc: Hdc, x: i32, y: i32, text: *const u16, length: i32) -> i32;
}

const WM_DESTROY: u32 = 0x0002;
const WS_OVERLAPPEDWINDOW: u32 = 0x00cf_0000;
const SW_SHOW: i32 = 5;
const SW_HIDE: i32 = 0;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
const WM_COMMAND: u32 = 0x0111;
const WM_INPUTLANGCHANGE: u32 = 0x0051;
const WM_PAINT: u32 = 0x000f;
const MF_GRAYED: u32 = 0x0001;
const MF_CHECKED: u32 = 0x0008;
const MF_RADIOCHECK: u32 = 0x0200;
const MF_POPUP: u32 = 0x0010;
const MF_SEPARATOR: u32 = 0x0800;
const MF_STRING: u32 = 0x0000;
const TPM_RIGHTBUTTON: u32 = 0x0002;
const TPM_RETURNCMD: u32 = 0x0100;
const TPM_NOANIMATION: u32 = 0x4000;
const HKEY_CURRENT_USER: usize = 0x8000_0001;
const RRF_RT_REG_DWORD: u32 = 0x0000_0010;

static CLASS_ATOM: AtomicU16 = AtomicU16::new(0);
static WINDOWS: AtomicUsize = AtomicUsize::new(0);
static LAST_WINDOW: AtomicUsize = AtomicUsize::new(0);
static MENU_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static MENU_HANDLER: AtomicUsize = AtomicUsize::new(0);
static KEYBOARD_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static KEYBOARD_HANDLER: AtomicUsize = AtomicUsize::new(0);
static KEY_FILTER_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static KEY_FILTER_HANDLER: AtomicUsize = AtomicUsize::new(0);
static PAINT_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static PAINT_HANDLER: AtomicUsize = AtomicUsize::new(0);
static PRINT_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static PRINT_HANDLER: AtomicUsize = AtomicUsize::new(0);
static PAGE_TEXT_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static PAGE_TEXT_HANDLER: AtomicUsize = AtomicUsize::new(0);
static WINDOW_EVENT_CONTEXT: AtomicUsize = AtomicUsize::new(0);
static WINDOW_EVENT_HANDLER: AtomicUsize = AtomicUsize::new(0);
static HOST_WINDOWS: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];
static WINDOW_MENUS: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];
/// The accelerator tables of the windows' menus, by host window slot.
static WINDOW_ACCELERATORS: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];
/// The windows whose pages asked for their menu shortcuts to be ignored,
/// by host window slot.
static IGNORED_SHORTCUTS: [AtomicBool; 32] = [const { AtomicBool::new(false) }; 32];
/// The last `WM_SIZE` state of each host window: 0 normal, 1 minimized
/// and 2 maximized.
static SIZE_STATES: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];
/// Each host window's page zoom factor, as `f32` bits; zero for 1.
static ZOOMS: [AtomicU32; 32] = [const { AtomicU32::new(0) }; 32];
static CLASS_NAME: [u16; 15] = [
    b'N' as u16,
    b'a' as u16,
    b't' as u16,
    b'i' as u16,
    b'v' as u16,
    b'e' as u16,
    b'b' as u16,
    b'o' as u16,
    b'a' as u16,
    b't' as u16,
    b'A' as u16,
    b'p' as u16,
    b'p' as u16,
    b'W' as u16,
    0,
];

/// Returns the Windows application-theme preference from the current user.
pub fn dark_mode() -> bool {
    let subkey = [
        83, 111, 102, 116, 119, 97, 114, 101, 92, 77, 105, 99, 114, 111, 115, 111, 102, 116, 92,
        87, 105, 110, 100, 111, 119, 115, 92, 67, 117, 114, 114, 101, 110, 116, 86, 101, 114, 115,
        105, 111, 110, 92, 84, 104, 101, 109, 101, 115, 92, 80, 101, 114, 115, 111, 110, 97, 108,
        105, 122, 101, 0,
    ];
    let name = [
        65, 112, 112, 115, 85, 115, 101, 76, 105, 103, 104, 116, 84, 104, 101, 109, 101, 0,
    ];
    let mut light_theme = 1_u32;
    let mut bytes = size_of::<u32>() as u32;
    // SAFETY: both strings are NUL-terminated and the DWORD output is writable.
    let status = unsafe {
        RegGetValueW(
            (HKEY_CURRENT_USER as i32 as isize) as Hinstance,
            subkey.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            core::ptr::null_mut(),
            core::ptr::from_mut(&mut light_theme).cast(),
            &mut bytes,
        )
    };
    status == 0 && bytes == size_of::<u32>() as u32 && light_theme == 0
}

#[cfg(test)]
pub(crate) fn lock_host_tests() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An operating-system window handle returned by [`create_window`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window(usize);

impl Window {
    /// The opaque numeric handle used by the native host.
    pub fn raw(self) -> usize {
        self.0
    }

    /// Wraps an Electron host handle carried by a BrowserWindow object.
    pub fn from_raw(raw: usize) -> Self {
        Self(raw)
    }
}

/// Generated-app callback invoked by the host for `WM_PAINT`.
pub type PaintHandler = unsafe extern "system" fn(usize, *mut c_void, *mut c_void);

/// Installs the generated renderer's paint callback and stable context pointer.
pub fn set_paint_handler(context: *mut c_void, handler: Option<PaintHandler>) {
    PAINT_CONTEXT.store(context as usize, Ordering::Release);
    PAINT_HANDLER.store(
        handler.map_or(0, |handler| handler as usize),
        Ordering::Release,
    );
}

/// What a window's page is printed on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintTarget {
    /// A PDF document, whose bytes printing returns.
    Pdf,
    /// The printer the user picks in the print dialog.
    Dialog,
    /// A printer by name; the default printer when empty.
    Printer(alloc::string::String),
}

/// A request to print a window's page.
#[derive(Debug, Clone, PartialEq)]
pub struct PrintRequest {
    /// Where the pages go.
    pub target: PrintTarget,
    /// The paper's portrait width and height in millimetres; `None` for
    /// the printer's own.
    pub paper: Option<(f32, f32)>,
    /// Whether pages are turned sideways.
    pub landscape: bool,
    /// Margins in CSS pixels, top, right, bottom and left; sides left
    /// `None` follow the page's `@page` rules.
    pub margins: [Option<f32>; 4],
    /// The document's name, as print queues show it.
    pub name: alloc::string::String,
    /// Whether background colors and gradients are printed.
    pub background: bool,
}

/// How printing a page ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrintOutcome {
    /// The bytes of the PDF document.
    Pdf(Vec<u8>),
    /// Sent to a printer.
    Printed,
    /// The user cancelled the print dialog.
    Cancelled,
}

/// Prints the page of a window; installed by applications that show pages.
pub type PrintHandler =
    unsafe fn(*mut c_void, usize, &PrintRequest) -> Result<PrintOutcome, alloc::string::String>;

/// Installs the generated application's page printer and its context. The
/// context must stay valid until the message loop exits.
pub fn set_print_handler(context: *mut c_void, handler: Option<PrintHandler>) {
    PRINT_CONTEXT.store(context as usize, Ordering::Release);
    PRINT_HANDLER.store(
        handler.map_or(0, |handler| handler as usize),
        Ordering::Release,
    );
}

/// Edits the page of a window as `webContents` text methods do: `name` is
/// the page's method, such as `replaceMisspelling`, given the text.
pub type PageTextHandler = unsafe fn(*mut c_void, usize, &str, &[u16]) -> bool;

/// Installs the pages' text editor and its context, which must stay valid
/// until the message loop exits.
pub fn set_page_text_handler(context: *mut c_void, handler: Option<PageTextHandler>) {
    PAGE_TEXT_CONTEXT.store(context as usize, Ordering::Release);
    PAGE_TEXT_HANDLER.store(
        handler.map_or(0, |handler| handler as usize),
        Ordering::Release,
    );
}

/// Calls a text method of the page a window shows; `false` without one.
pub fn page_text(window: Window, method: &str, text: &[u16]) -> bool {
    let handler = PAGE_TEXT_HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return false;
    }
    // SAFETY: installed with this type by `set_page_text_handler`.
    let handler = unsafe { core::mem::transmute::<usize, PageTextHandler>(handler) };
    let context = PAGE_TEXT_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the page windows keep the context alive.
    unsafe { handler(context, window.0, method, text) }
}

/// Prints the page a window shows.
pub fn print_window(
    window: Window,
    request: &PrintRequest,
) -> Result<PrintOutcome, alloc::string::String> {
    let handler = PRINT_HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return Err("printing is not available".into());
    }
    // SAFETY: installed with this type by `set_print_handler`.
    let handler = unsafe { core::mem::transmute::<usize, PrintHandler>(handler) };
    let context = PRINT_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the generated application keeps the context alive.
    unsafe { handler(context, window.0, request) }
}

fn window_slot(window: Window) -> Option<usize> {
    HOST_WINDOWS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == window.0)
}

/// The zoom factor of the page a window shows, as
/// `webContents.getZoomFactor` gives it.
pub fn zoom(window: Window) -> f64 {
    window_slot(window).map_or(1.0, |slot| match ZOOMS[slot].load(Ordering::Acquire) {
        0 => 1.0,
        bits => f64::from(f32::from_bits(bits)),
    })
}

/// Zooms the page a window shows, which repaints at the new zoom.
pub fn set_zoom(window: Window, factor: f64) {
    if let Some(slot) = window_slot(window)
        && factor.is_finite()
        && factor > 0.0
    {
        ZOOMS[slot].store((factor as f32).to_bits(), Ordering::Release);
        invalidate_window(window);
    }
}

/// The user asked to zoom a window's page with the wheel, which its
/// `webContents` reports with `zoom-changed`.
pub fn zoom_changed(window: Window, zoom_in: bool) {
    let event = if zoom_in {
        WindowEvent::ZoomIn
    } else {
        WindowEvent::ZoomOut
    };
    dispatch_window_event(window.0 as Hwnd, event);
}

/// Requests a native repaint of a BrowserWindow.
pub fn invalidate_window(window: Window) {
    // SAFETY: null asks Windows to invalidate the whole client area; the
    // window handle is opaque and rejected by the OS if it has gone stale.
    unsafe { InvalidateRect(window.0 as Hwnd, core::ptr::null(), 0) };
}

/// A GDI device context valid during one `WM_PAINT` callback.
#[derive(Debug, Clone, Copy)]
pub struct PaintDc(Hdc);

impl PaintDc {
    /// Wraps the HDC Windows supplied to the paint callback.
    ///
    /// # Safety
    /// `dc` must be the live HDC from `BeginPaint` and remain valid until
    /// `EndPaint` for the matching window.
    pub unsafe fn from_raw(dc: *mut c_void) -> PaintDc {
        PaintDc(dc)
    }

    /// Paints a solid rectangle into this native paint DC.
    pub fn fill_rect(self, x: f32, y: f32, width: f32, height: f32, rgb: [u8; 3]) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let rect = WinRect {
            left: x as i32,
            top: y as i32,
            right: (x + width) as i32,
            bottom: (y + height) as i32,
        };
        let color = u32::from(rgb[0]) | (u32::from(rgb[1]) << 8) | (u32::from(rgb[2]) << 16);
        // SAFETY: `self.0` is guaranteed live by the PaintDc constructor; the
        // brush is created for this call and released below.
        let brush = unsafe { CreateSolidBrush(color) };
        if brush.is_null() {
            return;
        }
        // SAFETY: both handles are valid for the duration of the fill.
        unsafe {
            FillRect(self.0, &rect, brush);
            DeleteObject(brush);
        }
    }

    /// Draws one UTF-8 text run using a native GDI font.
    pub fn draw_text(self, text: &str, x: f32, y: f32, rgb: [u8; 3], font: FontSpec<'_>) {
        let mut face: Vec<u16> = font.family.encode_utf16().collect();
        face.push(0);
        let size = if font.size.is_finite() {
            (font.size as i32).max(1)
        } else {
            13
        };
        // SAFETY: `face` is null-terminated and remains live until the font
        // is created; other parameters are scalar font attributes.
        let native_font = unsafe {
            CreateFontW(
                -size,
                0,
                0,
                0,
                i32::from(font.weight.clamp(1, 1000)),
                u32::from(font.italic),
                0,
                0,
                1,
                0,
                0,
                0,
                0,
                face.as_ptr(),
            )
        };
        if native_font.is_null() {
            return;
        }
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        if wide.len() > i32::MAX as usize {
            // SAFETY: this font was created above and is not selected into a DC.
            unsafe { DeleteObject(native_font) };
            return;
        }
        let length = wide.len() as i32;
        wide.push(0);
        // SAFETY: the DC and font are valid; selected font is restored below.
        let previous = unsafe { SelectObject(self.0, native_font) };
        let color = u32::from(rgb[0]) | (u32::from(rgb[1]) << 8) | (u32::from(rgb[2]) << 16);
        let mut metrics = WinTextMetrics::default();
        // SAFETY: `metrics` matches TEXTMETRICW and the selected font is live.
        let ascent = if unsafe { GetTextMetricsW(self.0, &mut metrics) } != 0 {
            metrics.tmAscent
        } else {
            size
        };
        // SAFETY: GDI receives readable UTF-16 text and scalar drawing properties.
        unsafe {
            SetTextColor(self.0, color);
            SetBkMode(self.0, 1);
            TextOutW(self.0, x as i32, y as i32 + ascent, wide.as_ptr(), length);
            SelectObject(self.0, previous);
            DeleteObject(native_font);
        }
    }
}

/// Measures a string and font with the native Windows GDI font backend.
pub fn measure_text(window: Window, text: &str, font: FontSpec<'_>) -> MeasuredText {
    let fallback_size = if font.size.is_finite() {
        font.size.max(1.0)
    } else {
        13.0
    };
    let fallback = MeasuredText {
        width: text.chars().count() as f32 * fallback_size * 0.5,
        ascent: fallback_size * 0.8,
        descent: fallback_size * 0.2,
    };
    let window_handle = window.0 as Hwnd;
    // SAFETY: GetDC accepts a live HWND; a null result is handled below.
    let dc = unsafe { GetDC(window_handle) };
    if dc.is_null() {
        return fallback;
    }
    let mut face: Vec<u16> = font.family.encode_utf16().collect();
    face.push(0);
    let height = -((fallback_size as i32).max(1));
    // SAFETY: `face` is null-terminated and stays live until the font is
    // created; the remaining fields are scalar GDI font attributes.
    let native_font = unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            i32::from(font.weight.clamp(1, 1000)),
            u32::from(font.italic),
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            face.as_ptr(),
        )
    };
    if native_font.is_null() {
        // SAFETY: `dc` was acquired for `window_handle` above.
        unsafe { ReleaseDC(window_handle, dc) };
        return fallback;
    }
    // SAFETY: both GDI handles are valid and the font remains alive below.
    let previous = unsafe { SelectObject(dc, native_font) };
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    let mut size = WinSize {
        width: 0,
        height: 0,
    };
    let measured = text.encode_utf16().count() <= i32::MAX as usize
        // SAFETY: `wide` has at least `length` readable UTF-16 units and size
        // is writable for the duration of the call.
        && unsafe {
            GetTextExtentPoint32W(
                dc,
                wide.as_ptr(),
                text.encode_utf16().count() as i32,
                &mut size,
            ) != 0
        };
    let mut metrics = WinTextMetrics::default();
    // SAFETY: `metrics` matches the Windows TEXTMETRICW ABI and is writable.
    let has_metrics = unsafe { GetTextMetricsW(dc, &mut metrics) } != 0;
    // SAFETY: restore the selected object before deleting the created font.
    unsafe {
        SelectObject(dc, previous);
        DeleteObject(native_font);
        ReleaseDC(window_handle, dc);
    }
    if !measured || !has_metrics {
        return fallback;
    }
    MeasuredText {
        width: size.width as f32,
        ascent: metrics.tmAscent as f32,
        descent: metrics.tmDescent as f32,
    }
}

/// A native menu item supplied by the compiled Electron `Menu` module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeMenuItem {
    /// Command identifier used to dispatch a selected item.
    pub id: u16,
    /// Null-terminated UTF-16 label.
    pub label: Vec<u16>,
    /// Whether this entry separates neighboring groups.
    pub separator: bool,
    /// Whether this entry owns a submenu.
    pub submenu: bool,
    /// Whether this entry is enabled.
    pub enabled: bool,
    /// Whether this entry is checked.
    pub checked: bool,
    /// Whether a checked entry uses the native radio-circle mark.
    pub radio: bool,
    /// The accelerator the menu registers, as the page wrote it, without a
    /// trailing NUL.
    pub accelerator: Option<Vec<u16>>,
    /// The shortcut text shown beside the label, as Chromium shows it.
    pub shortcut: Option<Vec<u16>>,
    /// Child entries for a submenu.
    pub children: Vec<NativeMenuItem>,
}

/// Creates and shows an 800×600-style native window of the given size in
/// device-independent pixels, centered on the primary display.
pub fn create_window(title: &[u16], width: i32, height: i32) -> Result<Window, u32> {
    create(&WindowOptions {
        size: (f64::from(width), f64::from(height)),
        ..WindowOptions::new(title)
    })
}

/// Creates a native top-level window as `options` describe.
pub fn create(options: &WindowOptions<'_>) -> Result<Window, u32> {
    let valid = |value: f64| value.is_finite() && value > 0.0;
    if !valid(options.size.0) || !valid(options.size.1) {
        return Err(87);
    }
    register_window_class()?;
    let Some(slot) = HOST_WINDOWS.iter().position(|slot| {
        slot.compare_exchange(0, frame::RESERVED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }) else {
        return Err(8);
    };
    let (style, rect) = frame::prepare(options, slot);
    let extended = frame::extended_style(options);
    let mut title = options.title.to_vec();
    title.push(0);
    // The window binds to its slot at its first message, before it returns.
    frame::PENDING.store(slot, Ordering::Release);
    // SAFETY: the class is registered, both strings are null-terminated, and
    // null parent/menu/parameter handles request an ordinary top-level window.
    let window = unsafe {
        CreateWindowExW(
            extended,
            CLASS_NAME.as_ptr(),
            title.as_ptr(),
            style,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            GetModuleHandleW(core::ptr::null()),
            core::ptr::null_mut(),
        )
    };
    frame::PENDING.store(usize::MAX, Ordering::Release);
    if window.is_null() {
        // SAFETY: GetLastError has no preconditions.
        let error = unsafe { GetLastError() };
        HOST_WINDOWS[slot].store(0, Ordering::Release);
        return Err(error);
    }
    HOST_WINDOWS[slot].store(window as usize, Ordering::Release);
    frame::made(window, options);
    if options.show {
        crate::platform::trace_layout(|| alloc::format!("window {:#x} shown", window as usize));
        // SAFETY: `window` is a live HWND returned by CreateWindowExW.
        unsafe {
            ShowWindow(window, SW_SHOW);
            UpdateWindow(window);
        }
    }
    WINDOWS.fetch_add(1, Ordering::AcqRel);
    LAST_WINDOW.store(window as usize, Ordering::Release);
    Ok(Window(window as usize))
}

/// Asks a window to close, as its close button does: the application's
/// window event handler may refuse. The request is handled by the message
/// loop, after the current script returns.
pub fn request_close(window: Window) -> Result<(), u32> {
    const WM_CLOSE: u32 = 0x0010;
    if !is_window(window) {
        return Err(1400);
    }
    // SAFETY: `is_window` confirms the handle is currently valid.
    if unsafe { PostMessageW(window.0 as Hwnd, WM_CLOSE, 0, 0) } == 0 {
        // SAFETY: GetLastError has no preconditions.
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

/// Destroys a native top-level window returned by [`create_window`].
pub fn destroy_window(window: Window) -> Result<(), u32> {
    // SAFETY: this handle was returned by `create_window` and the caller owns
    // it, so it may destroy the window exactly once.
    if unsafe { DestroyWindow(window.0 as Hwnd) } == 0 {
        // SAFETY: GetLastError has no preconditions.
        Err(unsafe { GetLastError() })
    } else {
        Ok(())
    }
}

/// Shows a live host window.
pub fn show_window(window: Window) -> Result<(), u32> {
    if !is_window(window) {
        return Err(1400);
    }
    crate::platform::trace_layout(|| alloc::format!("window {:#x} shown", window.0));
    // SAFETY: `is_window` confirms the handle is currently valid.
    unsafe { ShowWindow(window.0 as Hwnd, SW_SHOW) };
    Ok(())
}

/// Hides a live host window.
pub fn hide_window(window: Window) -> Result<(), u32> {
    if !is_window(window) {
        return Err(1400);
    }
    // SAFETY: `is_window` confirms the handle is currently valid.
    unsafe { ShowWindow(window.0 as Hwnd, SW_HIDE) };
    Ok(())
}

/// Native window lifecycle and state events reported to the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum WindowEvent {
    /// The user asked to close the window; the handler may refuse.
    Close = 0,
    /// The window was destroyed.
    Closed = 1,
    /// The window gained keyboard focus.
    Focus = 2,
    /// The window lost keyboard focus.
    Blur = 3,
    /// The window's client area changed size.
    Resize = 4,
    /// The window was maximized.
    Maximize = 5,
    /// The window left its maximized state for its normal one.
    Unmaximize = 6,
    /// The window was minimized.
    Minimize = 7,
    /// The window left its minimized state.
    Restore = 8,
    /// The window entered full screen.
    EnterFullScreen = 9,
    /// The window left full screen.
    LeaveFullScreen = 10,
    /// The user turned the wheel with Ctrl to zoom in.
    ZoomIn = 11,
    /// The user turned the wheel with Ctrl to zoom out.
    ZoomOut = 12,
}

/// Callback for [`WindowEvent`]s; returns false to refuse a `Close`.
pub type WindowEventHandler = unsafe extern "system" fn(*mut c_void, usize, WindowEvent) -> bool;

/// Installs the generated application's window event callback and context.
/// The context must stay valid until the message loop exits.
pub fn set_window_event_handler(context: *mut c_void, handler: Option<WindowEventHandler>) {
    WINDOW_EVENT_CONTEXT.store(context as usize, Ordering::Release);
    WINDOW_EVENT_HANDLER.store(
        handler.map_or(0, |handler| handler as usize),
        Ordering::Release,
    );
}

/// Dispatches the events of a window's change between its normal,
/// minimized and maximized states, except those full screen causes.
fn size_state_events(window: Hwnd, state: usize) {
    if state > 2 {
        return;
    }
    let Some(slot) = HOST_WINDOWS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == window as usize)
    else {
        return;
    };
    let previous = SIZE_STATES[slot].swap(state, Ordering::AcqRel);
    if previous == state
        || frame::frame(window).is_some_and(|frame| frame.changing.load(Ordering::Acquire))
    {
        return;
    }
    if previous == 1 {
        dispatch_window_event(window, WindowEvent::Restore);
    }
    match state {
        1 => dispatch_window_event(window, WindowEvent::Minimize),
        2 => dispatch_window_event(window, WindowEvent::Maximize),
        _ if previous == 2 => dispatch_window_event(window, WindowEvent::Unmaximize),
        _ => true,
    };
}

/// Puts a window into full screen or out of it, and tells the application
/// when it changed.
pub fn set_window_full_screen(window: Window, full: bool) {
    if is_window(window) && frame::set_full_screen(window, full) {
        let event = if full {
            WindowEvent::EnterFullScreen
        } else {
            WindowEvent::LeaveFullScreen
        };
        dispatch_window_event(window.0 as Hwnd, event);
    }
}

fn dispatch_window_event(window: Hwnd, event: WindowEvent) -> bool {
    let handler = WINDOW_EVENT_HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return true;
    }
    // SAFETY: installed with this ABI by `set_window_event_handler`.
    let handler = unsafe { core::mem::transmute::<usize, WindowEventHandler>(handler) };
    let context = WINDOW_EVENT_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the generated application keeps the context alive.
    unsafe { handler(context, window as usize, event) }
}

/// Window state changes BrowserWindow methods request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowState {
    /// `maximize()`.
    Maximize,
    /// `minimize()`.
    Minimize,
    /// `restore()` and `unmaximize()`.
    Restore,
    /// `focus()`: bring to the foreground.
    Focus,
}

/// Applies a window state change.
pub fn set_window_state(window: Window, state: WindowState) {
    const SW_MAXIMIZE: i32 = 3;
    const SW_MINIMIZE: i32 = 6;
    const SW_RESTORE: i32 = 9;
    if !is_window(window) {
        return;
    }
    let hwnd = window.0 as Hwnd;
    // SAFETY: `is_window` confirms the handle is currently valid.
    unsafe {
        match state {
            WindowState::Maximize => ShowWindow(hwnd, SW_MAXIMIZE),
            WindowState::Minimize => ShowWindow(hwnd, SW_MINIMIZE),
            WindowState::Restore => ShowWindow(hwnd, SW_RESTORE),
            WindowState::Focus => SetForegroundWindow(hwnd),
        };
    }
}

/// Whether a window is maximized, minimized, focused, or always on top.
pub fn window_flags(window: Window) -> (bool, bool, bool, bool) {
    const GWL_EXSTYLE: i32 = -20;
    const WS_EX_TOPMOST: isize = 0x8;
    let hwnd = window.0 as Hwnd;
    // SAFETY: these queries accept stale handles.
    unsafe {
        (
            IsZoomed(hwnd) != 0,
            IsIconic(hwnd) != 0,
            GetForegroundWindow() == hwnd,
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST != 0,
        )
    }
}

/// Keeps a window above others, or not.
pub fn set_always_on_top(window: Window, top: bool) {
    const SWP_NOSIZE: u32 = 0x1;
    const SWP_NOMOVE: u32 = 0x2;
    const SWP_NOACTIVATE: u32 = 0x10;
    let after = if top { -1isize } else { -2isize } as Hwnd;
    // SAFETY: SetWindowPos fails harmlessly for stale handles.
    unsafe {
        SetWindowPos(
            window.0 as Hwnd,
            after,
            0,
            0,
            0,
            0,
            SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE,
        )
    };
}

/// Sets whether a window has a taskbar button, as Electron's
/// `setSkipTaskbar` does.
pub fn set_skip_taskbar(window: Window, skip: bool) {
    let hwnd = window.0 as Hwnd;
    let Some(frame) = frame::frame(hwnd) else {
        return;
    };
    frame.skip_taskbar.store(skip, Ordering::Release);
    // SAFETY: IsWindowVisible fails harmlessly for stale handles.
    if unsafe { IsWindowVisible(hwnd) } != 0 {
        taskbar::set_button(hwnd, !skip);
    }
}

/// Sets a window's title.
pub fn set_window_title(window: Window, title: &[u16]) {
    let text: Vec<u16> = title.iter().copied().chain([0]).collect();
    // SAFETY: the title is null-terminated; stale handles fail.
    unsafe { SetWindowTextW(window.0 as Hwnd, text.as_ptr()) };
}

/// A window's title.
pub fn window_title(window: Window) -> Vec<u16> {
    let mut text = alloc::vec![0u16; 512];
    // SAFETY: the buffer holds `len` units.
    let length = unsafe { GetWindowTextW(window.0 as Hwnd, text.as_mut_ptr(), text.len() as i32) };
    text.truncate(length.max(0) as usize);
    text
}

/// Whether the supplied handle still identifies a native window.
pub fn is_window(window: Window) -> bool {
    // SAFETY: IsWindow accepts any HWND value, including stale handles.
    unsafe { IsWindow(window.0 as Hwnd) != 0 }
}

/// Whether the supplied host window is currently visible.
pub fn is_visible(window: Window) -> bool {
    // SAFETY: IsWindowVisible accepts stale or arbitrary HWND values.
    unsafe { IsWindowVisible(window.0 as Hwnd) != 0 }
}

/// The number of live top-level windows created by this host.
pub fn window_count() -> usize {
    WINDOWS.load(Ordering::Acquire)
}

/// Installs the compiled Realm's menu-command dispatcher for native messages.
///
/// The context must remain valid until it is replaced or the native message
/// loop exits. Dispatch happens synchronously on the UI thread after the main
/// compiled module has returned.
pub fn set_menu_command_handler(context: *mut c_void, handler: Option<MenuCommandHandler>) {
    MENU_CONTEXT.store(context as usize, Ordering::Release);
    MENU_HANDLER.store(
        handler.map_or(0, |handler| handler as usize),
        Ordering::Release,
    );
}

/// Callback for chosen menu items: window, command, and whether an
/// accelerator chose it.
pub type MenuCommandHandler = unsafe extern "system" fn(*mut c_void, usize, u16, bool);

/// Whether Shift, Control, Alt and the Windows key are held.
pub fn held_modifiers() -> (bool, bool, bool, bool) {
    // SAFETY: GetKeyState only reads this thread's input state.
    let held = |key: i32| unsafe { GetKeyState(key) } < 0;
    (held(0x10), held(0x11), held(0x12), held(0x5b) || held(0x5c))
}

/// Installs the main Realm callback for native keyboard layout changes.
///
/// The context must remain valid until the native message loop exits.
pub fn set_keyboard_layout_change_handler(
    context: *mut c_void,
    handler: Option<unsafe extern "system" fn(*mut c_void)>,
) {
    KEYBOARD_CONTEXT.store(context as usize, Ordering::Release);
    KEYBOARD_HANDLER.store(
        handler.map_or(0, |handler| handler as usize),
        Ordering::Release,
    );
}

/// Callback offered every queued key message (window, message, wParam,
/// lParam) before translation and dispatch; returns true to drop it.
pub type KeyFilter = unsafe extern "system" fn(*mut c_void, usize, u32, usize, isize) -> bool;

/// Installs the callback that sees key messages before the window does, as
/// Electron's `before-input-event` does. The context must stay valid until
/// the message loop exits.
pub fn set_key_filter(context: *mut c_void, handler: Option<KeyFilter>) {
    KEY_FILTER_CONTEXT.store(context as usize, Ordering::Release);
    KEY_FILTER_HANDLER.store(
        handler.map_or(0, |handler| handler as usize),
        Ordering::Release,
    );
}

/// Whether the installed key filter drops a queued message.
fn filtered(message: &Message) -> bool {
    const WM_KEYDOWN: u32 = 0x0100;
    const WM_KEYUP: u32 = 0x0101;
    const WM_SYSKEYDOWN: u32 = 0x0104;
    const WM_SYSKEYUP: u32 = 0x0105;
    if !matches!(
        message.message,
        WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP
    ) {
        return false;
    }
    let handler = KEY_FILTER_HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return false;
    }
    // SAFETY: installed with this ABI by `set_key_filter`.
    let handler = unsafe { core::mem::transmute::<usize, KeyFilter>(handler) };
    let context = KEY_FILTER_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the generated application keeps the context alive.
    unsafe {
        handler(
            context,
            message.window as usize,
            message.message,
            message.wparam,
            message.lparam,
        )
    }
}

/// Installs an Electron application menu on every native host window.
pub fn set_application_menu(items: &[NativeMenuItem]) -> Result<(), u32> {
    for slot in &HOST_WINDOWS {
        let raw = slot.load(Ordering::Acquire);
        if raw != 0 {
            set_window_menu(Window(raw), items)?;
        }
    }
    Ok(())
}

/// Each host window's own icon, made from an image, freed when replaced or
/// when the window is destroyed.
static WINDOW_ICONS: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];

/// Gives a window its own icon, for its title bar and the taskbar; `None`
/// returns it to the application's icon.
pub fn set_window_icon(window: Window, icon: Option<Icon>) {
    const WM_SETICON: u32 = 0x0080;
    let Some(index) = HOST_WINDOWS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == window.0)
    else {
        if let Some(icon) = icon {
            icon.destroy();
        }
        return;
    };
    let handle = icon.map_or(0, Icon::raw);
    for kind in [0, 1] {
        // SAFETY: the window is live; Windows draws the icon, which stays
        // alive until replaced below.
        unsafe { SendMessageW(window.0 as Hwnd, WM_SETICON, kind, handle as isize) };
    }
    let old = WINDOW_ICONS[index].swap(handle, Ordering::AcqRel);
    if old != 0 && old != handle {
        // SAFETY: the old icon was made for this window and is no longer shown.
        unsafe { notify::destroy_icon(old) };
    }
}

/// The window whose hidden menu bar the Alt key showed, until its menu
/// closes.
static ALT_SHOWN_MENU: AtomicUsize = AtomicUsize::new(0);

/// Shows or hides a window's menu bar, keeping its menu and shortcuts.
pub fn set_menu_bar_visibility(window: Window, visible: bool) {
    if !frame::note_menu_hidden(window, !visible) || !has_frame(window) {
        return;
    }
    let Some(index) = HOST_WINDOWS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == window.0)
    else {
        return;
    };
    let menu = if visible {
        WINDOW_MENUS[index].load(Ordering::Acquire) as *mut c_void
    } else {
        core::ptr::null_mut()
    };
    // SAFETY: the window is live and the menu, if any, is its own.
    unsafe {
        SetMenu(window.0 as Hwnd, menu);
        DrawMenuBar(window.0 as Hwnd);
    }
}

/// Whether a window shows a menu bar: it has a menu and the bar is shown.
pub fn is_menu_bar_visible(window: Window) -> bool {
    shows_menu_bar(window)
        && HOST_WINDOWS
            .iter()
            .position(|slot| slot.load(Ordering::Acquire) == window.0)
            .is_some_and(|index| WINDOW_MENUS[index].load(Ordering::Acquire) != 0)
}

/// Replaces one BrowserWindow's native menu with its own menu tree.
pub fn set_window_menu(window: Window, items: &[NativeMenuItem]) -> Result<(), u32> {
    let raw = window.0;
    let Some(index) = HOST_WINDOWS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == raw)
    else {
        return Err(1400);
    };
    let window = raw as Hwnd;
    // SAFETY: this handle is present in the live host window table.
    if unsafe { IsWindow(window) } == 0 {
        return Err(1400);
    }
    // SAFETY: CreateMenu has no pointer preconditions.
    let menu = unsafe { CreateMenu() };
    if menu.is_null() {
        // SAFETY: GetLastError has no preconditions.
        return Err(unsafe { GetLastError() });
    }
    if let Err(error) = append_menu_items(menu, items) {
        // SAFETY: this menu was created above and is not attached to a window.
        unsafe { DestroyMenu(menu) };
        return Err(error);
    }
    // Frameless windows and hidden menu bars keep their menu but show no
    // menu bar.
    // SAFETY: both handles are live; `menu` is a completed menu tree.
    if shows_menu_bar(Window(raw)) && unsafe { SetMenu(window, menu) } == 0 {
        // SAFETY: GetLastError has no preconditions.
        let error = unsafe { GetLastError() };
        // SAFETY: this menu was created above and SetMenu rejected it.
        unsafe { DestroyMenu(menu) };
        return Err(error);
    }
    // SAFETY: `window` is a live host HWND with its menu just updated.
    unsafe { DrawMenuBar(window) };
    let old = WINDOW_MENUS[index].swap(menu as usize, Ordering::AcqRel) as *mut c_void;
    if !old.is_null() {
        // SAFETY: SetMenu detached the old menu from this window.
        unsafe { DestroyMenu(old) };
    }
    replace_accelerators(index, accelerator_table(items));
    Ok(())
}

/// Electron's accelerator for a key, as a Win32 accelerator entry: its
/// modifiers and virtual key. `Super` and unknown keys have none.
fn accelerator(text: &[u16]) -> Option<(u8, u16)> {
    const FVIRTKEY: u8 = 0x01;
    const FSHIFT: u8 = 0x04;
    const FCONTROL: u8 = 0x08;
    const FALT: u8 = 0x10;
    let text = alloc::string::String::from_utf16(text).ok()?;
    let mut parts: Vec<&str> = text.split('+').collect();
    // `Ctrl++` ends with two empty parts for the `+` key.
    if parts.len() > 1 && parts[parts.len() - 1].is_empty() {
        parts.truncate(parts.len() - 2);
        parts.push("plus");
    }
    let mut virt = FVIRTKEY;
    let mut key = None;
    for part in parts {
        let name = part.trim().to_ascii_lowercase();
        let named = match name.as_str() {
            "cmdorctrl" | "commandorcontrol" | "ctrl" | "control" | "cmd" | "command" => {
                virt |= FCONTROL;
                continue;
            }
            "shift" => {
                virt |= FSHIFT;
                continue;
            }
            "alt" | "option" | "altgr" => {
                virt |= FALT;
                continue;
            }
            "plus" | "=" => 0xbb,
            "-" => 0xbd,
            "," => 0xbc,
            "." => 0xbe,
            "/" => 0xbf,
            "`" => 0xc0,
            "[" => 0xdb,
            "\\" => 0xdc,
            "]" => 0xdd,
            "'" => 0xde,
            ";" => 0xba,
            "space" => 0x20,
            "tab" => 0x09,
            "backspace" => 0x08,
            "delete" => 0x2e,
            "insert" => 0x2d,
            "return" | "enter" => 0x0d,
            "escape" | "esc" => 0x1b,
            "up" => 0x26,
            "down" => 0x28,
            "left" => 0x25,
            "right" => 0x27,
            "home" => 0x24,
            "end" => 0x23,
            "pageup" => 0x21,
            "pagedown" => 0x22,
            "printscreen" => 0x2c,
            "volumeup" => 0xaf,
            "volumedown" => 0xae,
            "volumemute" => 0xad,
            "medianexttrack" => 0xb0,
            "mediaprevioustrack" => 0xb1,
            "mediastop" => 0xb2,
            "mediaplaypause" => 0xb3,
            "numadd" => 0x6b,
            "numsub" => 0x6d,
            "nummult" => 0x6a,
            "numdiv" => 0x6f,
            "numdec" => 0x6e,
            _ => {
                let bytes = name.as_bytes();
                if bytes.len() == 1 && bytes[0].is_ascii_alphanumeric() {
                    u16::from(bytes[0].to_ascii_uppercase())
                } else if let Some(digit) = name
                    .strip_prefix("num")
                    .and_then(|n| n.parse::<u16>().ok())
                    .filter(|n| *n <= 9)
                {
                    0x60 + digit
                } else {
                    let number = name
                        .strip_prefix('f')
                        .and_then(|n| n.parse::<u16>().ok())
                        .filter(|n| (1..=24).contains(n))?;
                    0x6f + number
                }
            }
        };
        key = Some(named);
    }
    key.map(|key| (virt, key))
}

/// The accelerator table of a menu's enabled items, or null without any.
fn accelerator_table(items: &[NativeMenuItem]) -> *mut c_void {
    fn collect(items: &[NativeMenuItem], entries: &mut Vec<Accel>) {
        for item in items {
            if item.enabled
                && !item.separator
                && item.id != 0
                && let Some((virt, key)) = item.accelerator.as_deref().and_then(accelerator)
            {
                entries.push(Accel {
                    virt,
                    key,
                    command: item.id,
                });
            }
            collect(&item.children, entries);
        }
    }
    let mut entries = Vec::new();
    collect(items, &mut entries);
    if entries.is_empty() {
        return core::ptr::null_mut();
    }
    // SAFETY: Windows copies the entries during the call.
    unsafe { CreateAcceleratorTableW(entries.as_ptr(), entries.len() as i32) }
}

/// Gives a host window slot a new accelerator table, freeing the old one.
fn replace_accelerators(index: usize, table: *mut c_void) {
    let old = WINDOW_ACCELERATORS[index].swap(table as usize, Ordering::AcqRel) as *mut c_void;
    if !old.is_null() {
        // SAFETY: the table came from CreateAcceleratorTableW and is no
        // longer reachable.
        unsafe { DestroyAcceleratorTable(old) };
    }
}

/// Turns a key message for a window whose menu has accelerators into the
/// menu command; returns whether it did.
fn translate_accelerator(message: &Message) -> bool {
    const GA_ROOT: u32 = 2;
    if message.window.is_null() {
        return false;
    }
    // SAFETY: querying the message's root window has no ownership effects.
    let root = unsafe { GetAncestor(message.window, GA_ROOT) };
    let Some(index) = HOST_WINDOWS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == root as usize)
    else {
        return false;
    };
    if IGNORED_SHORTCUTS[index].load(Ordering::Acquire) {
        return false;
    }
    let table = WINDOW_ACCELERATORS[index].load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the window and the table are live and the message is valid.
    !table.is_null() && unsafe { TranslateAcceleratorW(root, table, message) } != 0
}

/// Makes a window ignore its menu shortcuts, or heed them again, as
/// `webContents.setIgnoreMenuShortcuts` asks.
pub fn set_ignore_menu_shortcuts(window: Window, ignore: bool) {
    if let Some(index) = HOST_WINDOWS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == window.0)
    {
        IGNORED_SHORTCUTS[index].store(ignore, Ordering::Release);
    }
}

/// Removes a BrowserWindow's native menu.
pub fn clear_window_menu(window: Window) -> Result<(), u32> {
    let raw = window.0;
    let Some(index) = HOST_WINDOWS
        .iter()
        .position(|slot| slot.load(Ordering::Acquire) == raw)
    else {
        return Err(1400);
    };
    let window = raw as Hwnd;
    // SAFETY: this handle is present in the live host window table.
    if unsafe { IsWindow(window) } == 0 {
        return Err(1400);
    }
    // SAFETY: `window` is live and null removes its current menu.
    if unsafe { SetMenu(window, core::ptr::null_mut()) } == 0 {
        // SAFETY: GetLastError has no preconditions.
        return Err(unsafe { GetLastError() });
    }
    // SAFETY: `window` is live after its menu was removed.
    unsafe { DrawMenuBar(window) };
    let old = WINDOW_MENUS[index].swap(0, Ordering::AcqRel) as *mut c_void;
    if !old.is_null() {
        // SAFETY: SetMenu detached the old menu from this window.
        unsafe { DestroyMenu(old) };
    }
    replace_accelerators(index, core::ptr::null_mut());
    Ok(())
}

/// Removes Electron's application menu from all native host windows.
pub fn clear_application_menu() -> Result<(), u32> {
    for slot in &HOST_WINDOWS {
        let raw = slot.load(Ordering::Acquire);
        if raw != 0 {
            clear_window_menu(Window(raw))?;
        }
    }
    Ok(())
}

/// Where a menu opens on the screen: a point in device-independent pixels
/// of a window's client area, else the cursor.
fn menu_point(owner: Option<Window>, position: Option<(i32, i32)>) -> frame::Point {
    let mut point = frame::Point::default();
    match (owner, position) {
        (Some(window), Some((x, y))) => {
            let scale = scale(window);
            point.x = (f64::from(x) * scale) as i32;
            point.y = (f64::from(y) * scale) as i32;
            // SAFETY: converts a point of the owner's client area.
            unsafe { ClientToScreen(window.0 as Hwnd, &mut point) };
        }
        // SAFETY: reads the cursor position.
        _ => unsafe {
            GetCursorPos(&mut point);
        },
    }
    point
}

/// Opens a native context menu and returns the selected command, if any:
/// at a point in device-independent pixels of the owner's client area, as
/// Electron places menus, else at the cursor.
pub fn popup_menu(
    owner: Option<Window>,
    items: &[NativeMenuItem],
    position: Option<(i32, i32)>,
) -> Result<Option<u16>, u32> {
    // SAFETY: CreatePopupMenu has no pointer preconditions.
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        // SAFETY: GetLastError has no preconditions.
        return Err(unsafe { GetLastError() });
    }
    // A menu needs an owner window: without one given, the active window,
    // as Electron opens it over the focused window.
    let owner = owner.or_else(|| {
        // SAFETY: GetActiveWindow has no preconditions.
        let active = unsafe { GetActiveWindow() };
        let window = if active.is_null() {
            LAST_WINDOW.load(Ordering::Acquire)
        } else {
            active as usize
        };
        (window != 0).then_some(Window(window))
    });
    let point = menu_point(owner, position);
    let owner = owner
        .map(|window| window.0 as Hwnd)
        .unwrap_or(core::ptr::null_mut());
    // The owner draws the menu's rows as Chromium draws Electron's menus.
    let mut popup = (!owner.is_null()).then(|| menu_draw::Popup::new(owner));
    let appended = match &mut popup {
        Some(popup) => popup.append(owner, menu, items),
        None => append_menu_items(menu, items),
    };
    if let Err(error) = appended {
        // SAFETY: this menu was created above and is not being displayed.
        unsafe { DestroyMenu(menu) };
        return Err(error);
    }
    if let Some(popup) = &popup {
        popup.activate();
    }
    // SAFETY: `menu` is a live popup menu and `owner` is null or an HWND. The
    // call is synchronous and copies no pointers from the caller. A menu
    // closes on outside clicks only when its owner is in the foreground.
    let selected = unsafe {
        if !owner.is_null() {
            SetForegroundWindow(owner);
        }
        TrackPopupMenuEx(
            menu,
            // Chromium's menus open without the system's animation.
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NOANIMATION,
            point.x,
            point.y,
            owner,
            core::ptr::null(),
        )
    };
    // SAFETY: TrackPopupMenuEx has returned, so the popup is no longer active.
    unsafe { DestroyMenu(menu) };
    drop(popup);
    if selected == 0 {
        Ok(None)
    } else if selected <= u32::from(u16::MAX) {
        Ok(Some(selected as u16))
    } else {
        Err(87)
    }
}

fn append_menu_items(menu: *mut c_void, items: &[NativeMenuItem]) -> Result<(), u32> {
    for item in items {
        let mut flags = if item.enabled { 0 } else { MF_GRAYED };
        if item.checked {
            flags |= MF_CHECKED;
        }
        if item.radio {
            flags |= MF_RADIOCHECK;
        }
        let mut label = item.label.clone();
        if let Some(shortcut) = &item.shortcut {
            if label.last() == Some(&0) {
                label.pop();
            }
            label.push(b'\t' as u16);
            label.extend_from_slice(shortcut);
            label.push(0);
        }
        let result = if item.separator {
            // SAFETY: `menu` is a live menu; separator items have no text.
            unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, core::ptr::null()) }
        } else if item.submenu {
            // SAFETY: CreatePopupMenu has no pointer preconditions.
            let child = unsafe { CreatePopupMenu() };
            if child.is_null() {
                // SAFETY: GetLastError has no preconditions.
                return Err(unsafe { GetLastError() });
            }
            if let Err(error) = append_menu_items(child, &item.children) {
                // SAFETY: this child menu was created above and is not attached.
                unsafe { DestroyMenu(child) };
                return Err(error);
            }
            // SAFETY: the parent and child menus are live and the UTF-16 label
            // is null-terminated for the duration of this call.
            let appended =
                unsafe { AppendMenuW(menu, MF_POPUP | flags, child as usize, label.as_ptr()) };
            if appended == 0 {
                // SAFETY: AppendMenuW did not attach the child on failure.
                unsafe { DestroyMenu(child) };
            }
            appended
        } else {
            // SAFETY: `menu` is live and `item.label` is null-terminated UTF-16.
            unsafe {
                AppendMenuW(
                    menu,
                    MF_STRING | flags,
                    usize::from(item.id),
                    label.as_ptr(),
                )
            }
        };
        if result == 0 {
            // SAFETY: GetLastError has no preconditions.
            return Err(unsafe { GetLastError() });
        }
    }
    Ok(())
}

/// Runs a common Electron edit role (`copy`, `paste`, `undo`...) where the
/// keyboard focus is: a page with the focus runs it as its own editing
/// command, as Chromium's editing does for menu roles; a native control gets
/// the standard edit message.
pub fn send_edit_action(role: &str) -> bool {
    // SAFETY: GetFocus returns null or a window handle owned by this UI thread.
    let focus = unsafe { GetFocus() };
    !focus.is_null() && edit_action_in(focus, role)
}

fn edit_action_in(focus: Hwnd, role: &str) -> bool {
    let command = match role {
        "pasteAndMatchStyle" => "paste",
        other => other,
    };
    let units: Vec<u16> = command.encode_utf16().collect();
    if page_text(Window(focus as usize), "command", &units) {
        return true;
    }
    let Some((message, wparam, lparam)) = edit_action_message(role) else {
        return false;
    };
    // SAFETY: `focus` is the UI thread's current control and these standard
    // edit messages accept the supplied integer parameters.
    unsafe { SendMessageW(focus, message, wparam, lparam) };
    true
}

fn edit_action_message(role: &str) -> Option<(u32, usize, isize)> {
    Some(match role {
        "undo" => (0x0304, 0, 0),
        "cut" => (0x0300, 0, 0),
        "copy" => (0x0301, 0, 0),
        "paste" => (0x0302, 0, 0),
        "delete" => (0x0303, 0, 0),
        "selectAll" => (0x00b1, 0, -1),
        _ => return None,
    })
}

/// Requests termination of the host message loop.
pub fn request_quit(exit_code: i32) {
    crate::trace::exit(&alloc::format!("quit requested with code {exit_code}"));
    // SAFETY: PostQuitMessage posts to the current GUI thread.
    unsafe { PostQuitMessage(exit_code) };
}

/// Runs the Win32 message loop together with the JavaScript event loops of
/// the application's realms. `step` runs the work due in every realm and
/// returns what they wait for, or the exit status when an uncaught exception
/// ends the program. Between steps the thread sleeps until the next timer,
/// a native I/O completion or a window message. Ends with `WM_QUIT`'s code.
pub fn run_host_loop(
    mut step: impl FnMut() -> Result<(Option<u32>, Vec<usize>), i32>,
) -> Result<i32, u32> {
    const PM_REMOVE: u32 = 1;
    const WM_QUIT: u32 = 0x0012;
    const QS_ALLINPUT: u32 = 0x04FF;
    const MWMO_INPUTAVAILABLE: u32 = 0x4;
    const WAIT_FAILED: u32 = u32::MAX;
    loop {
        let mut message = Message {
            window: core::ptr::null_mut(),
            message: 0,
            wparam: 0,
            lparam: 0,
            time: 0,
            point_x: 0,
            point_y: 0,
            private: 0,
        };
        // SAFETY: `message` is writable; a null HWND receives every message
        // of this thread, including WM_QUIT.
        while unsafe { PeekMessageW(&mut message, core::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if message.message == WM_QUIT {
                crate::trace::exit(&alloc::format!(
                    "host loop received WM_QUIT {}",
                    message.wparam as i32
                ));
                return Ok(message.wparam as i32);
            }
            // As in Electron, `before-input-event` sees keys first, then
            // menu shortcuts.
            if filtered(&message) || translate_accelerator(&message) {
                continue;
            }
            // SAFETY: `message` was filled by PeekMessageW.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            // Each press, release, wheel turn and typed character is a task
            // of its own, as in Chromium: tasks it made due run before the
            // next input, so a click's deferred work sees the page before a
            // wheel turn right after it scrolls. A key press stays with the
            // character it produces.
            if separate_input(message.message) {
                break;
            }
        }
        let (delay, signals) = match step() {
            Ok(wait) => wait,
            Err(status) => {
                crate::trace::exit(&alloc::format!(
                    "host step ended the program with status {status}"
                ));
                return Ok(status);
            }
        };
        if delay == Some(0) {
            continue;
        }
        let handles: Vec<*mut c_void> = signals
            .iter()
            .take(63)
            .map(|signal| *signal as *mut c_void)
            .collect();
        // SAFETY: the handles are live events owned by the realms' I/O queues.
        let status = unsafe {
            MsgWaitForMultipleObjectsEx(
                handles.len() as u32,
                handles.as_ptr(),
                delay.unwrap_or(u32::MAX),
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
        if status == WAIT_FAILED {
            // SAFETY: GetLastError has no preconditions.
            return Err(unsafe { GetLastError() });
        }
    }
}

/// Whether an input message ends the messages the host loop dispatches
/// before running pages' due tasks: mouse presses, releases and wheel
/// turns, typed characters and released keys.
fn separate_input(message: u32) -> bool {
    const WM_KEYUP: u32 = 0x0101;
    const WM_CHAR: u32 = 0x0102;
    const WM_SYSKEYUP: u32 = 0x0105;
    const WM_LBUTTONDOWN: u32 = 0x0201;
    const WM_MOUSEHWHEEL: u32 = 0x020E;
    matches!(
        message,
        WM_KEYUP | WM_CHAR | WM_SYSKEYUP | WM_LBUTTONDOWN..=WM_MOUSEHWHEEL
    )
}

/// Runs the current thread's Win32 message loop until a window closes.
pub fn run_message_loop() -> Result<i32, u32> {
    loop {
        let mut message = Message {
            window: core::ptr::null_mut(),
            message: 0,
            wparam: 0,
            lparam: 0,
            time: 0,
            point_x: 0,
            point_y: 0,
            private: 0,
        };
        // SAFETY: `message` is writable and a null HWND receives thread
        // messages, including WM_QUIT.
        let status = unsafe { GetMessageW(&mut message, core::ptr::null_mut(), 0, 0) };
        if status == 0 {
            return Ok(message.wparam as i32);
        }
        if status < 0 {
            // SAFETY: GetLastError has no preconditions.
            return Err(unsafe { GetLastError() });
        }
        // SAFETY: `message` was filled by GetMessageW and remains valid.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn register_window_class() -> Result<(), u32> {
    if CLASS_ATOM.load(Ordering::Acquire) != 0 {
        return Ok(());
    }
    // SAFETY: the module handle is process-wide, the class name is static and
    // null-terminated, and the callback follows the Win32 window procedure ABI.
    let instance = unsafe { GetModuleHandleW(core::ptr::null()) };
    // Windows show the application's icon unless given their own, as
    // Electron's do.
    let icon = Icon::application().map_or(core::ptr::null_mut(), |icon| icon.raw() as *mut c_void);
    let class = WindowClass {
        style: 0,
        procedure: Some(window_procedure),
        class_extra: 0,
        window_extra: 0,
        instance,
        icon,
        cursor: core::ptr::null_mut(),
        background: 6usize as *mut c_void,
        menu_name: core::ptr::null(),
        class_name: CLASS_NAME.as_ptr(),
    };
    // SAFETY: `class` and its static strings live through the registration call.
    let atom = unsafe { RegisterClassW(&class) };
    if atom != 0 {
        CLASS_ATOM.store(atom, Ordering::Release);
        return Ok(());
    }
    // SAFETY: GetLastError has no preconditions.
    let error = unsafe { GetLastError() };
    if error == ERROR_CLASS_ALREADY_EXISTS {
        CLASS_ATOM.store(1, Ordering::Release);
        Ok(())
    } else {
        Err(error)
    }
}

unsafe extern "system" fn window_procedure(
    window: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if let Some(result) = frame::message(window, message, wparam, lparam) {
        return result;
    }
    if message == WM_PAINT {
        let mut paint = PaintStruct {
            dc: core::ptr::null_mut(),
            erase: 0,
            paint: WinRect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            restore: 0,
            incremental: 0,
            reserved: [0; 32],
        };
        // SAFETY: `paint` has the Win32 PAINTSTRUCT layout and remains live
        // through EndPaint; the window is the one whose procedure is running.
        let dc = unsafe { BeginPaint(window, &mut paint) };
        if !dc.is_null() {
            dispatch_paint(window, dc);
            // SAFETY: this balances BeginPaint for the same live window.
            unsafe { EndPaint(window, &paint) };
        }
        return 0;
    }
    const WM_CLOSE: u32 = 0x0010;
    const WM_ACTIVATE: u32 = 0x0006;
    const WM_SIZE: u32 = 0x0005;
    const WM_SYSCOMMAND: u32 = 0x0112;
    const WM_EXITMENULOOP: u32 = 0x0212;
    const SC_KEYMENU: usize = 0xf100;
    // The Alt key shows a menu bar that hides itself, which hides again
    // once its menu closes.
    if message == WM_SYSCOMMAND
        && wparam & 0xfff0 == SC_KEYMENU
        && is_menu_bar_auto_hide(Window(window as usize))
        && !shows_menu_bar(Window(window as usize))
    {
        set_menu_bar_visibility(Window(window as usize), true);
        ALT_SHOWN_MENU.store(window as usize, Ordering::Release);
    }
    if message == WM_EXITMENULOOP
        && ALT_SHOWN_MENU
            .compare_exchange(window as usize, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        && is_menu_bar_auto_hide(Window(window as usize))
    {
        set_menu_bar_visibility(Window(window as usize), false);
    }
    if message == WM_CLOSE && !dispatch_window_event(window, WindowEvent::Close) {
        return 0;
    }
    match message {
        // A window gains and loses focus as Electron's do: when it becomes
        // or stops being the active window, not when the keyboard focus
        // moves between it and the native controls of its page.
        WM_ACTIVATE => {
            // Only changes count: activating a window can deliver another
            // activation for the same window.
            static ACTIVE: AtomicUsize = AtomicUsize::new(0);
            let handle = window as usize;
            if wparam & 0xFFFF == 0 {
                if ACTIVE
                    .compare_exchange(handle, 0, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    dispatch_window_event(window, WindowEvent::Blur);
                }
            } else if ACTIVE.swap(handle, Ordering::AcqRel) != handle {
                dispatch_window_event(window, WindowEvent::Focus);
            }
        }
        WM_SIZE => {
            size_state_events(window, wparam);
            dispatch_window_event(window, WindowEvent::Resize);
        }
        _ => {}
    }
    if message == WM_DESTROY {
        dispatch_window_event(window, WindowEvent::Closed);
        dispatch_menu_command(window, 0, false);
        if LAST_WINDOW.load(Ordering::Acquire) == window as usize {
            LAST_WINDOW.store(0, Ordering::Release);
        }
        // An application with a window event handler decides itself whether
        // closing its last window quits it.
        if WINDOWS.fetch_sub(1, Ordering::AcqRel) == 1
            && WINDOW_EVENT_HANDLER.load(Ordering::Acquire) == 0
        {
            // SAFETY: PostQuitMessage posts WM_QUIT to the current GUI thread.
            unsafe { PostQuitMessage(0) };
        }
        if let Some(index) = HOST_WINDOWS
            .iter()
            .position(|slot| slot.load(Ordering::Acquire) == window as usize)
        {
            HOST_WINDOWS[index].store(0, Ordering::Release);
            IGNORED_SHORTCUTS[index].store(false, Ordering::Release);
            SIZE_STATES[index].store(0, Ordering::Release);
            ZOOMS[index].store(0, Ordering::Release);
            replace_accelerators(index, core::ptr::null_mut());
            let icon = WINDOW_ICONS[index].swap(0, Ordering::AcqRel);
            if icon != 0 {
                // SAFETY: the icon was made for this window, which is going away.
                unsafe { notify::destroy_icon(icon) };
            }
            let menu = WINDOW_MENUS[index].swap(0, Ordering::AcqRel) as *mut c_void;
            if !menu.is_null() {
                // SAFETY: this menu was attached to the window being destroyed.
                unsafe { SetMenu(window, core::ptr::null_mut()) };
                // SAFETY: the menu is detached and owned by the native host.
                unsafe { DestroyMenu(menu) };
            }
        }
        return 0;
    }
    if message == WM_COMMAND && lparam == 0 {
        let command = (wparam & 0xffff) as u16;
        // Accelerators send their commands with a high word of 1.
        if dispatch_menu_command(window, command, wparam >> 16 & 0xffff == 1) {
            return 0;
        }
    }
    if message == WM_INPUTLANGCHANGE {
        dispatch_keyboard_layout_change();
    }
    const WM_DRAWITEM: u32 = 0x002b;
    const WM_MEASUREITEM: u32 = 0x002c;
    const WM_MENUCHAR: u32 = 0x0120;
    const WM_MENUSELECT: u32 = 0x011f;
    match message {
        // Menus send their rows' messages with no control identifier.
        WM_MEASUREITEM if wparam == 0 && menu_draw::measure(lparam) => return 1,
        WM_DRAWITEM if wparam == 0 && menu_draw::draw(lparam) => return 1,
        WM_MENUCHAR => {
            if let Some(result) = menu_draw::menu_char(wparam, lparam) {
                return result;
            }
        }
        WM_MENUSELECT => menu_draw::menu_select(window, wparam, lparam),
        _ => {}
    }
    // SAFETY: this is the callback registered for the native window class.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn dispatch_paint(window: Hwnd, dc: Hdc) {
    let handler = PAINT_HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return;
    }
    // SAFETY: the callback was installed with this ABI by generated Rust code.
    let handler = unsafe { core::mem::transmute::<usize, PaintHandler>(handler) };
    let context = PAINT_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the generated app retains its context through the message loop.
    unsafe { handler(window as usize, dc, context) };
}

fn dispatch_keyboard_layout_change() {
    let handler = KEYBOARD_HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return;
    }
    // SAFETY: the generated app installs this callback and its Realm before
    // entering the UI loop, and keeps both alive until that loop exits.
    let handler =
        unsafe { core::mem::transmute::<usize, unsafe extern "system" fn(*mut c_void)>(handler) };
    let context = KEYBOARD_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: context and callback follow the contract documented above.
    unsafe { handler(context) };
}

fn dispatch_menu_command(window: Hwnd, command: u16, accelerator: bool) -> bool {
    let handler = MENU_HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return false;
    }
    let context = MENU_CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the generated host registers a live Realm pointer and a matching
    // dispatcher before entering this UI-thread message loop.
    let handler: MenuCommandHandler = unsafe { core::mem::transmute(handler) };
    // SAFETY: the callback/context pair was atomically registered by the
    // generated host and executes on the same UI thread.
    unsafe { handler(context, window as usize, command, accelerator) };
    true
}

#[cfg(test)]
#[path = "app_host/tests.rs"]
mod tests;
