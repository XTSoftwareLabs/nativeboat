//! Top-level windows, scroll surfaces and their window procedure.

use crate::controls::{self, ControlData};
use crate::event_loop::dispatch;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::wide::{narrow, wide};
use crate::{
    Error, Event, KeyboardInput, MenuCommand, Modifiers, PointerKind, WindowId, keys, render,
};
use core::cell::{Cell, RefCell};
use core::ptr::{null, null_mut};
use nativeboat_css::types::Cursor;
use nativeboat_ir::{Change, DisplayItem, NativeChild, Rect};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::{GdiplusStartup, GdiplusStartupInput};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::{
    MK_CONTROL, MK_LBUTTON, MK_MBUTTON, MK_RBUTTON, MK_SHIFT, MK_XBUTTON1, MK_XBUTTON2,
};
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{
    DragAcceptFiles, DragFinish, DragQueryFileW, DragQueryPoint, HDROP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::{PCWSTR, w};

/// Options of a new top-level window.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowOptions {
    /// Title bar text.
    pub title: String,
    /// Width in device-independent pixels.
    pub width: f32,
    /// Height in device-independent pixels.
    pub height: f32,
    /// Whether the size is the client area instead of the whole window.
    pub use_content_size: bool,
    /// Screen position of the window; centered on the work area when unset.
    pub position: Option<(f32, f32)>,
    /// Minimum window size.
    pub min_size: Option<(f32, f32)>,
    /// Maximum window size.
    pub max_size: Option<(f32, f32)>,
    /// Whether the user can resize the window.
    pub resizable: bool,
    /// Whether the window has a minimize button.
    pub minimizable: bool,
    /// Whether the window has a maximize button.
    pub maximizable: bool,
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions {
            title: String::new(),
            width: 800.0,
            height: 600.0,
            use_content_size: false,
            position: None,
            min_size: None,
            max_size: None,
            resizable: true,
            minimizable: true,
            maximizable: true,
        }
    }
}

const WINDOW_CLASS: PCWSTR = w!("NativeBoatWindow");
const SURFACE_CLASS: PCWSTR = w!("NativeBoatSurface");
/// Pixels scrolled by a scroll bar arrow.
const LINE: f32 = 40.0;
/// Pixels scrolled by one wheel notch.
const NOTCH: f32 = 100.0;

mod clicks;
mod damage;
mod ime;
mod scrollbar;
pub(crate) mod stack;

/// What a window handle is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// The top-level window (surface 0) or a scroll container.
    Surface(u64),
    /// A native control.
    Control(u64),
}

pub(crate) struct SurfaceData {
    pub hwnd: HWND,
    pub parent: u64,
    pub rect: Rect,
    pub content: (f32, f32),
    pub scroll: (f32, f32),
    pub items: Vec<DisplayItem>,
    scrollbar: scrollbar::State,
    /// The area of the text insertion caret in the display list, in content
    /// coordinates, or `Some(None)` when a transform moves it, so that the
    /// whole surface repaints as it blinks.
    pub caret: Option<Option<Rect>>,
}

pub(crate) struct WindowData {
    pub hwnd: HWND,
    pub scale: f32,
    /// The page zoom included in `scale`.
    pub zoom: f32,
    /// The control the page's focus is on, which takes the keyboard focus
    /// whenever the window does.
    pub focus_target: Option<u64>,
    pub cursor: Cursor,
    pub min_size: Option<(f32, f32)>,
    pub max_size: Option<(f32, f32)>,
    pub surfaces: HashMap<u64, SurfaceData>,
    pub controls: HashMap<u64, ControlData>,
    /// Whether the caret is in the visible phase of its blink.
    pub caret_on: bool,
    /// Whether the window shows what lies behind it where its page paints
    /// nothing, as its host asked with [`TRANSPARENT_PROPERTY`].
    pub transparent: bool,
    ime: ime::State,
}

#[derive(Default)]
pub(crate) struct State {
    pub initialized: bool,
    pub next_id: WindowId,
    pub windows: HashMap<WindowId, WindowData>,
    pub handles: HashMap<usize, (WindowId, Role)>,
}

/// A surface's back buffer: a memory DC with a 32-bit DIB section selected,
/// kept between paints, as creating and clearing a window-sized bitmap for
/// every caret blink and keystroke costs more than painting it.
struct Back {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    /// The section's pixels, top-down rows of `size.0` BGRA pixels.
    bits: *mut u32,
    size: (i32, i32),
}

impl Drop for Back {
    fn drop(&mut self) {
        // SAFETY: the DC and bitmap were created for this buffer alone.
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

local! {
    static BACKS: RefCell<HashMap<(WindowId, u64), Back>> = RefCell::new(HashMap::new());
}

/// Runs `paint` with a surface's back buffer at least `size` device pixels
/// large, made compatible with `dc`.
fn with_back(window: WindowId, id: u64, dc: HDC, size: (i32, i32), paint: impl FnOnce(&Back)) {
    // Out of the map while painting: painting may paint other surfaces.
    let kept = BACKS.with_borrow_mut(|backs| backs.remove(&(window, id)));
    let back = match kept {
        Some(back) if back.size.0 >= size.0 && back.size.1 >= size.1 => back,
        kept => {
            // The old buffer is freed when `kept` goes out of use here.
            let grown = kept.map_or(size, |back| {
                (back.size.0.max(size.0), back.size.1.max(size.1))
            });
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: grown.0,
                    biHeight: -grown.1,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..BITMAPINFOHEADER::default()
                },
                ..BITMAPINFO::default()
            };
            // SAFETY: the DC is live; the buffer owns what is created here.
            unsafe {
                let memory = CreateCompatibleDC(dc);
                let mut bits = null_mut();
                let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
                let previous = SelectObject(memory, bitmap);
                Back {
                    dc: memory,
                    bitmap,
                    previous,
                    bits: bits.cast(),
                    size: grown,
                }
            }
        }
    };
    paint(&back);
    BACKS.with_borrow_mut(|backs| backs.insert((window, id), back));
}

/// Frees the back buffers of a window's surfaces, or of one of them.
fn release_backs(window: WindowId, id: Option<u64>) {
    let released: Vec<Back> = BACKS.with_borrow_mut(|backs| {
        let keys: Vec<_> = backs
            .keys()
            .filter(|(owner, surface)| *owner == window && id.is_none_or(|id| id == *surface))
            .copied()
            .collect();
        keys.into_iter()
            .filter_map(|key| backs.remove(&key))
            .collect()
    });
    drop(released);
}

local! {
    pub(crate) static STATE: RefCell<State> = RefCell::new(State::default());
    /// Whether the page cancelled the key press whose characters follow.
    static CANCELLED_KEY: Cell<bool> = const { Cell::new(false) };
    /// The first half of a surrogate pair typed as two characters.
    static HIGH_SURROGATE: Cell<u16> = const { Cell::new(0) };
    /// The window procedures of the hosts of adopted windows.
    static HOSTS: RefCell<HashMap<usize, isize>> = RefCell::new(HashMap::new());
}

pub(crate) fn px(value: f32, scale: f32) -> i32 {
    (value * scale).round() as i32
}

pub(crate) fn instance() -> HINSTANCE {
    // SAFETY: the module handle of the running executable.
    unsafe { GetModuleHandleW(null()) }
}

pub(crate) fn init() -> Result<(), Error> {
    if STATE.with_borrow(|state| state.initialized) {
        return Ok(());
    }
    let classes = INITCOMMONCONTROLSEX {
        dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_STANDARD_CLASSES | ICC_BAR_CLASSES | ICC_PROGRESS_CLASS,
    };
    let input = GdiplusStartupInput {
        GdiplusVersion: 1,
        ..GdiplusStartupInput::default()
    };
    let mut token = 0;
    // SAFETY: the structures are initialised and outlive the calls. The
    // DPI awareness call fails harmlessly when it was already set; OLE,
    // which the shell's dialogs need, stays initialised for the thread.
    let status = unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        windows_sys::Win32::System::Ole::OleInitialize(null());
        InitCommonControlsEx(&classes);
        GdiplusStartup(&mut token, &input, null_mut())
    };
    if status != 0 {
        return Err(Error {
            message: format!("GDI+ failed to start with status {status}"),
        });
    }
    for (name, style) in [
        (WINDOW_CLASS, CS_DBLCLKS | CS_HREDRAW | CS_VREDRAW),
        (SURFACE_CLASS, CS_DBLCLKS),
    ] {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style,
            lpfnWndProc: Some(window_proc),
            hInstance: instance(),
            lpszClassName: name,
            ..WNDCLASSEXW::default()
        };
        // SAFETY: the class structure is initialised; the error code is read
        // right after the failed call.
        let failed = unsafe {
            RegisterClassExW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS
        };
        if failed {
            return Err(Error::last("RegisterClassExW"));
        }
    }
    STATE.with_borrow_mut(|state| state.initialized = true);
    Ok(())
}

fn style_of(options: &WindowOptions) -> WINDOW_STYLE {
    let mut style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    if !options.resizable {
        style &= !(WS_THICKFRAME | WS_MAXIMIZEBOX);
    }
    if !options.maximizable {
        style &= !WS_MAXIMIZEBOX;
    }
    if !options.minimizable {
        style &= !WS_MINIMIZEBOX;
    }
    style
}

/// Creates a hidden top-level window.
pub fn create_window(options: &WindowOptions) -> Result<WindowId, Error> {
    init()?;
    // SAFETY: reads the system DPI.
    let dpi = unsafe { GetDpiForSystem() };
    let scale = dpi as f32 / 96.0;
    let style = style_of(options);
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: px(options.width, scale),
        bottom: px(options.height, scale),
    };
    if options.use_content_size {
        // SAFETY: `outer` is writable.
        unsafe { AdjustWindowRectExForDpi(&mut outer, style, 0, 0, dpi) };
    }
    let (width, height) = (outer.right - outer.left, outer.bottom - outer.top);
    let (x, y) = match options.position {
        Some((x, y)) => (px(x, scale), px(y, scale)),
        None => {
            let mut area = RECT::default();
            // SAFETY: SPI_GETWORKAREA writes a RECT.
            unsafe {
                SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut area as *mut RECT).cast(), 0)
            };
            (
                area.left + (area.right - area.left - width).max(0) / 2,
                area.top + (area.bottom - area.top - height).max(0) / 2,
            )
        }
    };
    let title = wide(&options.title);
    // SAFETY: the class is registered and the title outlives the call.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            WINDOW_CLASS,
            title.as_ptr(),
            style,
            x,
            y,
            width,
            height,
            null_mut(),
            null_mut(),
            instance(),
            null(),
        )
    };
    if hwnd.is_null() {
        return Err(Error::last("CreateWindowExW"));
    }
    Ok(register(hwnd, options.min_size, options.max_size))
}

/// Takes over the content of a top-level window that another host created:
/// painting, scroll surfaces, controls, pointer, keyboard and input-method
/// input, as for the windows of [`create_window`]. The host's window
/// procedure keeps the frame, activation, size limits, closing and menu
/// commands, and every message this crate leaves alone.
///
/// # Safety
/// `hwnd` must be a live top-level window of this thread whose procedure
/// takes the messages passed on to it.
pub unsafe fn adopt(hwnd: HWND) -> Result<WindowId, Error> {
    init()?;
    let window = register(hwnd, None, None);
    // Scroll surfaces and controls are child windows that paint themselves.
    // Without clipping them, the window's own painting covers them until
    // they paint again, which flashes them white whenever the page under
    // them changes, as the windows of `create_window` never do.
    // SAFETY: the caller guarantees the window; only a style bit changes.
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        SetWindowLongW(hwnd, GWL_STYLE, (style | WS_CLIPCHILDREN) as i32);
    }
    // SAFETY: the caller guarantees the window; its previous procedure
    // receives the messages the adopted procedure passes on.
    let host = unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, adopted_proc as *const () as isize) };
    HOSTS.with_borrow_mut(|hosts| hosts.insert(hwnd as usize, host));
    Ok(window)
}

/// Records a window's state and accepts files dropped on it.
fn register(hwnd: HWND, min_size: Option<(f32, f32)>, max_size: Option<(f32, f32)>) -> WindowId {
    // SAFETY: the window is live.
    let scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.0;
    let root = SurfaceData {
        hwnd,
        parent: 0,
        rect: Rect::default(),
        content: (0.0, 0.0),
        scroll: (0.0, 0.0),
        items: Vec::new(),
        scrollbar: scrollbar::State::default(),
        caret: None,
    };
    let id = STATE.with_borrow_mut(|state| {
        state.next_id += 1;
        let id = state.next_id;
        state.handles.insert(hwnd as usize, (id, Role::Surface(0)));
        state.windows.insert(
            id,
            WindowData {
                hwnd,
                scale,
                zoom: 1.0,
                focus_target: None,
                cursor: Cursor::Auto,
                min_size,
                max_size,
                surfaces: HashMap::from([(0, root)]),
                controls: HashMap::new(),
                caret_on: true,
                transparent: is_marked_transparent(hwnd),
                ime: ime::State::default(),
            },
        );
        id
    });
    let registered = crate::drop_target::register(hwnd, id);
    // SAFETY: OLE delivers the complete drag lifecycle on success; retain
    // legacy shell-drop support when OLE registration is unavailable.
    unsafe { DragAcceptFiles(hwnd, i32::from(!registered)) };
    id
}

/// The handle of a window, for platform services such as dialogs.
pub fn hwnd(window: WindowId) -> Option<HWND> {
    STATE.with_borrow(|state| state.windows.get(&window).map(|data| data.hwnd))
}

/// The ids of the open windows, oldest first.
pub fn window_ids() -> Vec<WindowId> {
    let mut ids: Vec<WindowId> = STATE.with_borrow(|state| state.windows.keys().copied().collect());
    ids.sort_unstable();
    ids
}

/// A window's page zoom.
pub fn zoom(window: WindowId) -> f32 {
    STATE.with_borrow(|state| state.windows.get(&window).map_or(1.0, |data| data.zoom))
}

/// Zooms a window's content: CSS pixels become `zoom` times as many device
/// pixels as at the window's DPI, and its content repaints.
pub fn set_zoom(window: WindowId, zoom: f32) {
    let changed = STATE.with_borrow_mut(|state| {
        let data = state.windows.get_mut(&window)?;
        if zoom.is_nan() || zoom <= 0.0 || data.zoom == zoom {
            return None;
        }
        data.scale = data.scale / data.zoom * zoom;
        data.zoom = zoom;
        Some(())
    });
    if changed.is_some() {
        rescale(window);
    }
}

/// The device pixels per CSS pixel of a window: its DPI scale and zoom.
pub fn scale(window: WindowId) -> f32 {
    STATE.with_borrow(|state| state.windows.get(&window).map_or(1.0, |data| data.scale))
}

pub(crate) fn surface_hwnd(window: WindowId, surface: u64) -> Option<HWND> {
    STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)?
            .surfaces
            .get(&surface)
            .map(|data| data.hwnd)
    })
}

/// Shows or hides a window.
pub fn show(window: WindowId, visible: bool) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: the handle belongs to a live window of this thread.
        unsafe {
            ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
        }
    }
}

/// Whether a window is visible.
pub fn is_visible(window: WindowId) -> bool {
    // SAFETY: the handle belongs to a live window of this thread.
    hwnd(window).is_some_and(|hwnd| unsafe { IsWindowVisible(hwnd) } != 0)
}

/// Whether a window is minimized.
pub fn is_minimized(window: WindowId) -> bool {
    // SAFETY: the handle belongs to a live window of this thread.
    hwnd(window).is_some_and(|hwnd| unsafe { IsIconic(hwnd) } != 0)
}

/// Whether a window is maximized.
pub fn is_maximized(window: WindowId) -> bool {
    // SAFETY: the handle belongs to a live window of this thread.
    hwnd(window).is_some_and(|hwnd| unsafe { IsZoomed(hwnd) } != 0)
}

/// Sets the smallest and largest sizes the user can resize a window to.
pub fn set_size_limits(window: WindowId, min: Option<(f32, f32)>, max: Option<(f32, f32)>) {
    STATE.with_borrow_mut(|state| {
        if let Some(data) = state.windows.get_mut(&window) {
            data.min_size = min;
            data.max_size = max;
        }
    });
}

/// Brings a window to the foreground and gives it the keyboard focus.
pub fn focus(window: WindowId) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: the handle belongs to a live window of this thread.
        unsafe {
            SetForegroundWindow(hwnd);
            SetFocus(hwnd);
        }
    }
}

/// Moves the keyboard focus after the page's focused element: to its native
/// control, or from a native control to the painted content. Only a window
/// holding the focus moves it, so pages never activate their windows.
pub fn follow_focus(window: WindowId, control: Option<u64>) {
    let Some(root) = hwnd(window) else {
        return;
    };
    let control = control.filter(|control| controls::hwnd(window, *control).is_some());
    STATE.with_borrow_mut(|state| {
        if let Some(data) = state.windows.get_mut(&window) {
            data.focus_target = control;
        }
    });
    // SAFETY: reads this thread's focus; the handles are compared only.
    let focus = unsafe { GetFocus() };
    // SAFETY: both handles are windows of this thread or null.
    if focus.is_null() || (focus != root && unsafe { IsChild(root, focus) } == 0) {
        return;
    }
    let target = control
        .and_then(|control| controls::hwnd(window, control))
        .unwrap_or(root);
    if target == focus {
        return;
    }
    if target == root {
        ime::complete(window, root);
    }
    // SAFETY: the handle belongs to a live window of this thread.
    unsafe { SetFocus(target) };
}

/// Types text into a window's text control at its caret, as when keys meant
/// for the control reach the window; `false` without such a control.
pub fn type_into_control(window: WindowId, control: u64, text: &str) -> bool {
    let Some(hwnd) = controls::hwnd(window, control) else {
        return false;
    };
    let mut class = [0u16; 16];
    // SAFETY: the buffer holds its length; the handle is a live control.
    let length = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    if !String::from_utf16_lossy(&class[..length.max(0) as usize]).eq_ignore_ascii_case("edit") {
        return false;
    }
    let text: Vec<u16> = text.encode_utf16().chain([0]).collect();
    // SAFETY: EM_REPLACESEL reads the null-terminated text during the call.
    unsafe { SendMessageW(hwnd, EM_REPLACESEL, 1, text.as_ptr() as LPARAM) };
    true
}

/// Gives the keyboard focus to a window's painted content, taking it from
/// its native controls.
pub fn focus_content(window: WindowId) {
    if let Some(hwnd) = hwnd(window) {
        ime::complete(window, hwnd);
        // SAFETY: the handle belongs to a live window of this thread.
        unsafe { SetFocus(hwnd) };
    }
}

/// Completes a painted host's IME and returns its final candidate before DOM
/// focus or selection moves. Returns `None` when no composition is active.
pub fn complete_composition(window: WindowId) -> Option<String> {
    ime::resolve(window)
}

/// Creates a hidden window inside another window's client area, as the
/// document of a frame shows in a page: it paints, scrolls and takes input
/// as the windows of [`create_window`] do, and [`place_child`] places it.
pub fn create_child(parent: WindowId) -> Result<WindowId, Error> {
    init()?;
    let parent_hwnd = hwnd(parent).ok_or_else(|| Error {
        message: "the parent window is gone".to_owned(),
    })?;
    // SAFETY: the class is registered and the parent is a live window of
    // this thread.
    let child = unsafe {
        CreateWindowExW(
            0,
            WINDOW_CLASS,
            null(),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            parent_hwnd,
            null_mut(),
            instance(),
            null(),
        )
    };
    if child.is_null() {
        return Err(Error::last("CreateWindowExW"));
    }
    let window = register(child, None, None);
    set_zoom(window, zoom(parent));
    Ok(window)
}

/// Places a window of [`create_child`] at a rectangle of its parent's
/// client area, in CSS pixels, above its siblings; `None` hides it.
pub fn place_child(window: WindowId, rect: Option<(f32, f32, f32, f32)>) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    let scale = scale(window);
    // SAFETY: the handle is live; showing and placing keeps the focus.
    unsafe {
        match rect {
            Some((x, y, width, height)) => {
                SetWindowPos(
                    hwnd,
                    HWND_TOP,
                    px(x, scale),
                    px(y, scale),
                    px(width, scale).max(0),
                    px(height, scale).max(0),
                    SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
            None => {
                ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
}

/// Moves a window to the top of the stacking order without activating,
/// showing, moving or resizing it.
pub fn move_top(window: WindowId) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: the handle is live; these flags preserve geometry and focus.
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
    }
}

/// Minimizes a window.
pub fn minimize(window: WindowId) {
    show_command(window, SW_MINIMIZE);
}

/// Maximizes a window.
pub fn maximize(window: WindowId) {
    show_command(window, SW_MAXIMIZE);
}

/// Restores a minimized or maximized window.
pub fn restore(window: WindowId) {
    show_command(window, SW_RESTORE);
}

fn show_command(window: WindowId, command: SHOW_WINDOW_CMD) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: the handle belongs to a live window of this thread.
        unsafe { ShowWindow(hwnd, command) };
    }
}

/// Sets the title bar text.
pub fn set_title(window: WindowId, title: &str) {
    if let Some(hwnd) = hwnd(window) {
        let title = wide(title);
        // SAFETY: the text outlives the call.
        unsafe { SetWindowTextW(hwnd, title.as_ptr()) };
    }
}

pub(crate) fn window_text(hwnd: HWND) -> String {
    // SAFETY: the buffer holds the reported length plus the terminator.
    unsafe {
        let length = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buffer = vec![0_u16; length + 1];
        GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        narrow(&buffer)
    }
}

/// The title bar text.
pub fn title(window: WindowId) -> String {
    hwnd(window).map(window_text).unwrap_or_default()
}

/// Asks a window to close, as the close button does.
pub fn close(window: WindowId) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: the handle belongs to a live window of this thread.
        unsafe { SendMessageW(hwnd, WM_CLOSE, 0, 0) };
    }
}

/// Destroys a window without asking.
pub fn destroy(window: WindowId) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: the handle belongs to a live window of this thread.
        unsafe { DestroyWindow(hwnd) };
    }
}

fn client(hwnd: HWND) -> RECT {
    let mut rect = RECT::default();
    // SAFETY: `rect` is writable.
    unsafe { GetClientRect(hwnd, &mut rect) };
    rect
}

/// The window property, a nonzero handle-sized value, by which the host of
/// an adopted window marks it transparent: its frame is extended over its
/// client area, which shows what lies behind it where the page paints
/// nothing.
pub const TRANSPARENT_PROPERTY: &str = "NativeboatTransparent";

/// Whether the host marked a window transparent with
/// [`TRANSPARENT_PROPERTY`].
fn is_marked_transparent(hwnd: HWND) -> bool {
    let name = wide(TRANSPARENT_PROPERTY);
    // SAFETY: the name is NUL-terminated; a stale window has no properties.
    !unsafe { GetPropW(hwnd, name.as_ptr()) }.is_null()
}

/// Whether a window shows what lies behind it where its page paints
/// nothing.
pub fn is_transparent(window: WindowId) -> bool {
    STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .is_some_and(|data| data.transparent)
    })
}

/// The room the window's root scrollbars take from the viewport, in
/// device-independent pixels: the horizontal bar's height and the vertical
/// bar's width.
pub fn scrollbar_room(window: WindowId) -> (f32, f32) {
    STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .and_then(|data| data.surfaces.get(&0))
            .and_then(|surface| surface.scrollbar.room)
            .unwrap_or_default()
    })
}

/// The client area size in CSS pixels. Chromium gives the page the client
/// area in whole device-independent pixels, each side rounded up, so a page
/// may lie a fraction of a pixel past the window's bottom and right edges;
/// zoom then divides it.
pub fn client_size(window: WindowId) -> (f32, f32) {
    let Some(hwnd) = hwnd(window) else {
        return (0.0, 0.0);
    };
    view(hwnd, scale(window), zoom(window))
}

/// The client area of a window or surface in CSS pixels at `scale`, which
/// includes `zoom`. A top-level window's is in whole device-independent
/// pixels, as the page has it.
pub(crate) fn view(hwnd: HWND, scale: f32, zoom: f32) -> (f32, f32) {
    let area = client(hwnd);
    // SAFETY: reads the style of a window of this thread.
    let child = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32 & WS_CHILD != 0;
    if child {
        return (area.right as f32 / scale, area.bottom as f32 / scale);
    }
    let dip = scale / zoom;
    (
        whole_dips(area.right, dip) / zoom,
        whole_dips(area.bottom, dip) / zoom,
    )
}

/// Device pixels in whole device-independent pixels, rounded up, as
/// Chromium's `ScaleToCeiledSize` scales them in `f32`.
fn whole_dips(pixels: i32, scale: f32) -> f32 {
    (pixels as f32 * (1.0 / scale)).ceil()
}

/// Resizes a window so that its client area has the given size.
pub fn set_client_size(window: WindowId, width: f32, height: f32) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    let scale = scale(window);
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: px(width, scale),
        bottom: px(height, scale),
    };
    // SAFETY: reads the window styles and writes `outer`.
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let extended = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        // A menu bar takes its height from the frame, not the client area.
        let menu = i32::from(!GetMenu(hwnd).is_null());
        AdjustWindowRectExForDpi(&mut outer, style, menu, extended, GetDpiForWindow(hwnd));
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            outer.right - outer.left,
            outer.bottom - outer.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// Sets the mouse cursor shown over painted content.
pub fn set_cursor(window: WindowId, cursor: Cursor) {
    STATE.with_borrow_mut(|state| {
        if let Some(data) = state.windows.get_mut(&window) {
            data.cursor = cursor;
        }
    });
}

/// Gives a native control the keyboard focus.
pub fn focus_control(window: WindowId, control: u64) {
    if let Some(hwnd) = controls::hwnd(window, control) {
        // SAFETY: the handle belongs to a live control of this thread.
        unsafe { SetFocus(hwnd) };
    }
}

/// Applies changes computed by [`nativeboat_ir::diff`] to a window.
pub fn apply(window: WindowId, changes: &[Change]) {
    let tracing = crate::trace::painting();
    for change in changes {
        if tracing {
            crate::trace::line(&alloc::format!(
                "{} apply {}",
                microseconds(),
                describe(change)
            ));
        }
        match change {
            Change::Scrollbars { surface, style } => {
                scrollbar::set(window, *surface, style.clone())
            }
            Change::SurfaceView {
                surface,
                clip,
                opacity,
            } => view_surface(window, *surface, *clip, *opacity),
            Change::AddSurface { id, parent } => add_surface(window, *id, *parent),
            Change::UpdateSurface {
                id,
                rect,
                content_width,
                content_height,
                scroll_x,
                scroll_y,
            } => update_surface(
                window,
                *id,
                *rect,
                (*content_width, *content_height),
                (*scroll_x, *scroll_y),
            ),
            Change::RemoveSurface { id } => remove_surface(window, *id),
            Change::Paint { surface, items } => paint_surface(window, *surface, items.clone()),
            Change::AddControl { surface, control } => controls::add(window, *surface, control),
            Change::UpdateControl { control } => controls::update(window, control),
            Change::RemoveControl { id } => controls::remove(window, *id),
            Change::Order { controls, .. } => controls::order(window, controls),
            Change::Stack { surface, children } => stack::order(window, *surface, children),
            Change::Select { id, start, end } => {
                controls::select_text(window, *id, *start as usize, *end as usize)
            }
        }
    }
}

/// A change in a line of a paint trace.
fn describe(change: &Change) -> String {
    match change {
        Change::Scrollbars { surface, .. } => alloc::format!("scrollbars {surface}"),
        Change::SurfaceView {
            surface,
            clip,
            opacity,
        } => alloc::format!("view {surface} clip {clip:?} opacity {opacity:?}"),
        Change::AddSurface { id, parent } => alloc::format!("add surface {id} in {parent}"),
        Change::UpdateSurface { id, rect, .. } => alloc::format!(
            "update surface {id} at {},{} {}x{}",
            rect.x,
            rect.y,
            rect.width,
            rect.height
        ),
        Change::RemoveSurface { id } => alloc::format!("remove surface {id}"),
        Change::Paint { surface, items } => {
            alloc::format!("paint {surface} with {} items", items.len())
        }
        Change::AddControl { surface, .. } => alloc::format!("add control in {surface}"),
        Change::UpdateControl { .. } => "update control".into(),
        Change::RemoveControl { id } => alloc::format!("remove control {id}"),
        Change::Order { .. } => "order controls".into(),
        Change::Stack { surface, children } => {
            alloc::format!("stack {surface}: {children:?}")
        }
        Change::Select { id, .. } => alloc::format!("select in control {id}"),
    }
}

fn add_surface(window: WindowId, id: u64, parent: u64) {
    let Some(parent_hwnd) = surface_hwnd(window, parent) else {
        return;
    };
    // SAFETY: the class is registered and the parent is a live window.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_CONTROLPARENT,
            SURFACE_CLASS,
            null(),
            WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            0,
            0,
            parent_hwnd,
            null_mut(),
            instance(),
            null(),
        )
    };
    if hwnd.is_null() {
        return;
    }
    // Scroll containers stay below the controls of their parent.
    // SAFETY: the window was just created.
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
    STATE.with_borrow_mut(|state| {
        state
            .handles
            .insert(hwnd as usize, (window, Role::Surface(id)));
        if let Some(data) = state.windows.get_mut(&window) {
            data.surfaces.insert(
                id,
                SurfaceData {
                    hwnd,
                    parent,
                    rect: Rect::default(),
                    content: (0.0, 0.0),
                    scroll: (0.0, 0.0),
                    items: Vec::new(),
                    scrollbar: scrollbar::State::default(),
                    caret: None,
                },
            );
        }
    });
}

/// Shows only part of a surface, as the boxes around it clip it, or all of
/// it, as opaque as the boxes around it are.
fn view_surface(
    window: WindowId,
    id: u64,
    clip: Option<nativeboat_ir::Rect>,
    opacity: Option<f32>,
) {
    let Some(hwnd) = surface_hwnd(window, id) else {
        return;
    };
    // Child windows are layered to show through, on Windows 8 and later.
    // SAFETY: the window is live; its extended style and alpha change.
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        match opacity {
            Some(opacity) => {
                SetWindowLongW(hwnd, GWL_EXSTYLE, (style | WS_EX_LAYERED) as i32);
                SetLayeredWindowAttributes(
                    hwnd,
                    0,
                    (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
                    LWA_ALPHA,
                );
            }
            None if style & WS_EX_LAYERED != 0 => {
                SetWindowLongW(hwnd, GWL_EXSTYLE, (style & !WS_EX_LAYERED) as i32);
            }
            None => {}
        }
    }
    let scale =
        STATE.with_borrow(|state| state.windows.get(&window).map_or(1.0, |data| data.scale));
    // SAFETY: the window is live; Windows owns a region it accepts and
    // releases the one it replaces.
    unsafe {
        match clip {
            Some(clip) => {
                let region = CreateRectRgn(
                    px(clip.x, scale),
                    px(clip.y, scale),
                    px(clip.x + clip.width, scale),
                    px(clip.y + clip.height, scale),
                );
                if !region.is_null() && SetWindowRgn(hwnd, region, 1) == 0 {
                    DeleteObject(region);
                }
            }
            None => {
                SetWindowRgn(hwnd, null_mut(), 1);
            }
        }
    }
}

/// Positions a child window at a rectangle of its parent's content.
pub(crate) fn place(hwnd: HWND, rect: Rect, extra: f32, scroll: (f32, f32), scale: f32) {
    place_with(hwnd, rect, extra, scroll, scale, 0);
}

/// Positions a child window, with extra `SetWindowPos` flags.
fn place_with(hwnd: HWND, rect: Rect, extra: f32, scroll: (f32, f32), scale: f32, flags: u32) {
    // SAFETY: the handle belongs to a live child window of this thread.
    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            px(rect.x - scroll.0, scale),
            px(rect.y - scroll.1, scale),
            px(rect.width, scale),
            px(rect.height + extra, scale),
            SWP_NOZORDER | SWP_NOACTIVATE | flags,
        )
    };
}

fn update_surface(window: WindowId, id: u64, rect: Rect, content: (f32, f32), scroll: (f32, f32)) {
    let found = STATE.with_borrow_mut(|state| {
        let data = state.windows.get_mut(&window)?;
        let scale = data.scale;
        let parent = data.surfaces.get(&id)?.parent;
        let parent_scroll = data
            .surfaces
            .get(&parent)
            .map_or((0.0, 0.0), |parent| parent.scroll);
        let surface = data.surfaces.get_mut(&id)?;
        surface.rect = rect;
        surface.content = content;
        surface.scroll = scroll;
        Some((surface.hwnd, scale, parent_scroll))
    });
    let Some((hwnd, scale, parent_scroll)) = found else {
        return;
    };
    if id != 0 {
        place(hwnd, rect, 0.0, parent_scroll, scale);
    }
    refresh(window, id, hwnd);
}

/// Updates the scroll bars, child positions and painting of a surface after
/// its geometry changed.
fn refresh(window: WindowId, id: u64, hwnd: HWND) {
    update_scrollbars(window, id);
    reposition_children(window, id);
    // SAFETY: the handle belongs to a live window of this thread.
    unsafe { InvalidateRect(hwnd, null(), 0) };
    controls::invalidate_authored(window, id);
}

use nativeboat_ir::scrollbar::overflows;

/// Returns whether the page was told of a new viewport size, which the
/// window's root scrollbars changed.
fn update_scrollbars(window: WindowId, id: u64) -> bool {
    let found = STATE.with_borrow(|state| {
        let data = state.windows.get(&window)?;
        let surface = data.surfaces.get(&id)?;
        Some((
            surface.hwnd,
            data.scale,
            surface.rect,
            surface.content,
            surface.scroll,
            surface.scrollbar.style.is_some(),
        ))
    });
    let Some((hwnd, scale, rect, content, scroll, styled)) = found else {
        return false;
    };
    if styled {
        // SAFETY: styled scrollbars are painted inside the live surface.
        unsafe {
            ShowScrollBar(hwnd, SB_BOTH, 0);
        }
        // The window's root scrollbars take room from the viewport.
        let changed = id == 0
            && STATE.with_borrow_mut(|state| {
                state.windows.get_mut(&window).is_some_and(|data| {
                    let (scale, zoom) = (data.scale, data.zoom);
                    data.surfaces
                        .get_mut(&0)
                        .is_some_and(|surface| scrollbar::reserve(surface, scale, zoom))
                })
            });
        if changed {
            report_viewport(window);
        }
        return changed;
    }
    // Windows' own scrollbars take their room outside the client area.
    let dropped = id == 0
        && STATE.with_borrow_mut(|state| {
            state
                .windows
                .get_mut(&window)
                .and_then(|data| data.surfaces.get_mut(&0))
                .is_some_and(|surface| surface.scrollbar.room.take().is_some())
        });
    let area = client(hwnd);
    for (bar, content, extent, scroll, page) in [
        (SB_HORZ, content.0, rect.width, scroll.0, area.right),
        (SB_VERT, content.1, rect.height, scroll.1, area.bottom),
    ] {
        let (max, page) = if overflows(content, extent) {
            (px(content, scale) - 1, page.max(0) as u32)
        } else {
            (0, 1)
        };
        let info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: max,
            nPage: page,
            nPos: px(scroll, scale),
            nTrackPos: 0,
        };
        // SAFETY: the structure is initialised.
        unsafe { SetScrollInfo(hwnd, bar, &info, 1) };
    }
    if dropped {
        report_viewport(window);
    }
    dropped
}

fn reposition_children(window: WindowId, id: u64) {
    let found = STATE.with_borrow(|state| {
        let data = state.windows.get(&window)?;
        let scroll = data.surfaces.get(&id)?.scroll;
        let mut children: Vec<(NativeChild, HWND, Rect, f32)> = data
            .surfaces
            .iter()
            .filter(|(child, surface)| **child != 0 && surface.parent == id)
            .map(|(child, surface)| {
                (
                    NativeChild::Surface(*child),
                    surface.hwnd,
                    surface.rect,
                    0.0,
                )
            })
            .collect();
        children.extend(
            data.controls
                .iter()
                .filter(|(_, control)| control.surface == id)
                .map(|(child, control)| {
                    (
                        NativeChild::Control(*child),
                        control.hwnd,
                        controls::window_rect(&control.control, data.scale),
                        controls::extra_height(control.control.kind),
                    )
                }),
        );
        Some((data.scale, scroll, children))
    });
    if let Some((scale, scroll, children)) = found {
        for (child, hwnd, rect, extra) in children {
            // A child that ancestor content paints over moves without its
            // pixels: what lies over it changes as it moves, so all of it
            // repaints.
            if stack::exists(window, child) {
                place_with(hwnd, rect, extra, scroll, scale, SWP_NOCOPYBITS);
                // SAFETY: the handle belongs to a live child window of this
                // thread.
                unsafe { InvalidateRect(hwnd, null(), 0) };
            } else {
                place(hwnd, rect, extra, scroll, scale);
            }
        }
    }
}

fn remove_surface(window: WindowId, id: u64) {
    release_backs(window, Some(id));
    let removed = STATE.with_borrow_mut(|state| {
        let surface = state.windows.get_mut(&window)?.surfaces.remove(&id)?;
        state.handles.remove(&(surface.hwnd as usize));
        Some(surface.hwnd)
    });
    if let Some(hwnd) = removed {
        // SAFETY: the handle belonged to the removed surface.
        unsafe { DestroyWindow(hwnd) };
    }
}

/// The timer that blinks the caret of a top-level window.
const CARET_TIMER: usize = 0xCA7E;

/// Where the caret of a display list is: its rectangle, or `Some(None)`
/// when a transform moves it.
fn caret_area(items: &[DisplayItem]) -> Option<Option<Rect>> {
    let mut transforms = 0usize;
    for item in items {
        match item {
            DisplayItem::PushTransform(_) => transforms += 1,
            DisplayItem::PopTransform => transforms = transforms.saturating_sub(1),
            DisplayItem::Caret { rect, .. } => return Some((transforms == 0).then_some(*rect)),
            _ => {}
        }
    }
    None
}

/// Restarts the caret's blink, or stops it when no caret blinks.
fn blink(root: HWND, carets: bool) {
    // SAFETY: the timer belongs to a live window of this thread.
    unsafe {
        let time = GetCaretBlinkTime();
        if carets && time != windows_sys::Win32::System::Threading::INFINITE {
            SetTimer(root, CARET_TIMER, time, None);
        } else {
            KillTimer(root, CARET_TIMER);
        }
    }
}

/// Shows the carets of a window afresh when `focus` gives its focus change,
/// else flips their blink phase, and repaints them.
fn caret_phase(window: WindowId, focus: Option<bool>) {
    let found = STATE.with_borrow_mut(|state| {
        let data = state.windows.get_mut(&window)?;
        data.caret_on = focus.is_some() || !data.caret_on;
        let carets: Vec<(HWND, Option<Rect>, (f32, f32))> = data
            .surfaces
            .values()
            .filter_map(|surface| Some((surface.hwnd, surface.caret?, surface.scroll)))
            .collect();
        Some((data.hwnd, data.scale, carets))
    });
    let Some((root, scale, carets)) = found else {
        return;
    };
    for (hwnd, area, (x, y)) in &carets {
        let bounds = area.map(|rect| RECT {
            left: px(rect.x - x, scale) - 2,
            top: px(rect.y - y, scale) - 2,
            right: px(rect.x + rect.width - x, scale) + 2,
            bottom: px(rect.y + rect.height - y, scale) + 2,
        });
        // SAFETY: the handle belongs to a live window of this thread.
        unsafe { InvalidateRect(*hwnd, bounds.as_ref().map_or(null(), |bounds| bounds), 0) };
    }
    if focus.is_some() || carets.is_empty() {
        blink(root, focus.unwrap_or(false) && !carets.is_empty());
    }
}

/// Repaints the surfaces showing the image under `url`, whose pixels have
/// arrived or changed: their display lists name it as before, so comparing
/// them finds no damage. Images added while the window state is in use,
/// as when painting loads them, are not shown yet.
pub(crate) fn image_changed(url: &str) {
    let shown = |items: &[DisplayItem]| {
        items
            .iter()
            .any(|item| matches!(item, DisplayItem::Image { url: shown, .. } if shown == url))
    };
    let surfaces: Vec<HWND> = STATE.with(|state| {
        state.try_borrow().map_or_else(
            |_| Vec::new(),
            |state| {
                state
                    .windows
                    .values()
                    .flat_map(|data| data.surfaces.values())
                    .filter(|surface| shown(&surface.items))
                    .map(|surface| surface.hwnd)
                    .collect()
            },
        )
    });
    for hwnd in surfaces {
        // SAFETY: the handle belongs to a live window of this thread.
        unsafe { InvalidateRect(hwnd, null(), 0) };
        stack::invalidate(hwnd, None);
    }
}

fn paint_surface(window: WindowId, id: u64, items: Vec<DisplayItem>) {
    let caret = caret_area(&items);
    let found = STATE.with_borrow_mut(|state| {
        let data = state.windows.get_mut(&window)?;
        let scale = data.scale;
        let surface = data.surfaces.get_mut(&id)?;
        // Only the parts the new items change are painted again.
        let area = match damage::between(&surface.items, &items) {
            damage::Damage::None => Some(Vec::new()),
            damage::Damage::Areas(rects) => {
                let (x, y) = surface.scroll;
                Some(
                    rects
                        .iter()
                        .map(|rect| RECT {
                            left: px(rect.x - x, scale) - 1,
                            top: px(rect.y - y, scale) - 1,
                            right: px(rect.x + rect.width - x, scale) + 1,
                            bottom: px(rect.y + rect.height - y, scale) + 1,
                        })
                        .collect(),
                )
            }
            damage::Damage::All => None,
        };
        surface.items = items;
        let moved = surface.caret != caret;
        surface.caret = caret;
        let hwnd = surface.hwnd;
        // A caret that moves shows at once and blinks afresh.
        data.caret_on |= moved;
        let carets = data
            .surfaces
            .values()
            .any(|surface| surface.caret.is_some());
        Some((hwnd, data.hwnd, moved.then_some(carets), area))
    });
    if let Some((hwnd, root, moved, area)) = found {
        if area.as_ref().is_none_or(|areas| !areas.is_empty()) {
            // The changed areas repaint, with the native children under them,
            // whose pixels the surface's changes may cover.
            match &area {
                Some(areas) => {
                    for area in areas {
                        // SAFETY: the handle belongs to a live window of this thread.
                        unsafe { InvalidateRect(hwnd, area, 0) };
                        stack::invalidate(hwnd, Some(area));
                    }
                }
                None => {
                    // SAFETY: the handle belongs to a live window of this thread.
                    unsafe { InvalidateRect(hwnd, null(), 0) };
                    stack::invalidate(hwnd, None);
                }
            }
            controls::invalidate_authored(window, id);
        }
        if let Some(carets) = moved {
            blink(root, carets);
        }
    }
    ime::position(window);
}

/// How far beyond an area backdrop filters read painted pixels: those on
/// a surface and those its ancestors paint above it.
fn backdrop_reach(window: WindowId, id: u64) -> i32 {
    STATE
        .with_borrow(|state| {
            let data = state.windows.get(&window)?;
            let surface = data.surfaces.get(&id)?;
            let own = render::backdrop_reach(&surface.items, data.scale);
            Some(own.max(stack::backdrop_reach(data, NativeChild::Surface(id))))
        })
        .unwrap_or(0)
}

/// Where a surface's box and its content begin in the window, in CSS
/// pixels.
/// Whether an area of a surface, in its device pixels, lies wholly outside
/// its window's client area.
fn outside_window(window: WindowId, id: u64, area: RECT) -> bool {
    STATE.with_borrow(|state| {
        let Some(data) = state.windows.get(&window) else {
            return false;
        };
        let Some(root) = data.surfaces.get(&0) else {
            return false;
        };
        if id == 0 {
            return false;
        }
        let scale = data.scale;
        let (corner, _) = origins(data, id);
        let (x, y) = (corner.0 * scale, corner.1 * scale);
        let (width, height) = (root.rect.width * scale, root.rect.height * scale);
        width > 0.0
            && height > 0.0
            && (x + area.right as f32 <= 0.0
                || y + area.bottom as f32 <= 0.0
                || x + area.left as f32 >= width
                || y + area.top as f32 >= height)
    })
}

fn origins(data: &WindowData, id: u64) -> ((f32, f32), (f32, f32)) {
    let Some(surface) = data.surfaces.get(&id) else {
        return ((0.0, 0.0), (0.0, 0.0));
    };
    let corner = if id == 0 {
        (surface.rect.x, surface.rect.y)
    } else {
        let (_, content) = origins(data, surface.parent);
        (content.0 + surface.rect.x, content.1 + surface.rect.y)
    };
    (
        corner,
        (corner.0 - surface.scroll.0, corner.1 - surface.scroll.1),
    )
}

/// The surfaces a surface's display list stacks, in order.
/// Paints, into the device context of surface `id`, what lies beneath it
/// in its window: the display lists of the surfaces around it and of
/// those stacked under it, each within its own box.
fn paint_beneath(data: &WindowData, id: u64, dc: HDC, area: RECT) {
    let (corner, _) = origins(data, id);
    let mut chain = alloc::vec![id];
    while let Some(parent) = chain
        .last()
        .and_then(|last| (*last != 0).then(|| data.surfaces.get(last).map(|s| s.parent)))
        .flatten()
    {
        if chain.contains(&parent) {
            break;
        }
        chain.push(parent);
    }
    chain.reverse();
    // Each surface around this one paints up to this one's place in it.
    for pair in chain.windows(2) {
        beneath_list(data, pair[0], dc, corner, area, Some(pair[1]), 0);
    }
}

/// The item that closes a group a push item opens.
fn closing(push: &DisplayItem) -> Option<DisplayItem> {
    match push {
        DisplayItem::PushClip { .. } | DisplayItem::PushClipPath { .. } => {
            Some(DisplayItem::PopClip)
        }
        DisplayItem::PushOpacity(_) => Some(DisplayItem::PopOpacity),
        DisplayItem::PushFilter(_) => Some(DisplayItem::PopFilter),
        DisplayItem::PushMask(_) => Some(DisplayItem::PopMask),
        DisplayItem::PushTransform(_) => Some(DisplayItem::PopTransform),
        _ => None,
    }
}

/// Paints, for what lies beneath an overlay, a surface's display list up to
/// the place of the surface `stop` in it, or all of it, within `clip`. The
/// surfaces it stacks paint at their places in the list, under what the list
/// paints after them, as a dialog's overlay dims the editor beside its
/// window's title bar; its scrollbars paint over its content.
fn beneath_list(
    data: &WindowData,
    id: u64,
    dc: HDC,
    corner: (f32, f32),
    clip: RECT,
    stop: Option<u64>,
    depth: u32,
) {
    let Some(surface) = data.surfaces.get(&id) else {
        return;
    };
    let scale = data.scale;
    let (own, content) = origins(data, id);
    let scroll = (corner.0 - content.0, corner.1 - content.1);
    let items = &surface.items;
    let end = stop
        .and_then(|stop| {
            items.iter().position(|item| {
                *item == DisplayItem::Child(nativeboat_ir::NativeChild::Surface(stop))
            })
        })
        .unwrap_or(items.len());
    let places: Vec<usize> = items[..end]
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(
                item,
                DisplayItem::Child(nativeboat_ir::NativeChild::Surface(_))
            )
        })
        .map(|(index, _)| index)
        .collect();
    if places.is_empty() || depth >= 32 {
        render::paint_area(dc, &items[..end], scale, scroll, Some(clip), false);
    } else {
        // The groups open at a child's place close before it paints and
        // open again after it.
        let mut open: Vec<&DisplayItem> = Vec::new();
        let mut from = 0;
        for at in places.into_iter().chain(core::iter::once(end)) {
            let mut segment: Vec<DisplayItem> = open.iter().map(|item| (*item).clone()).collect();
            for item in &items[from..at] {
                if closing(item).is_some() {
                    open.push(item);
                } else if matches!(
                    item,
                    DisplayItem::PopClip
                        | DisplayItem::PopOpacity
                        | DisplayItem::PopFilter
                        | DisplayItem::PopMask
                        | DisplayItem::PopTransform
                ) {
                    open.pop();
                }
                segment.push(item.clone());
            }
            segment.extend(open.iter().rev().filter_map(|item| closing(item)));
            render::paint_area(dc, &segment, scale, scroll, Some(clip), false);
            if let Some(DisplayItem::Child(nativeboat_ir::NativeChild::Surface(child))) =
                items.get(at)
                && at < end
                && let Some(under) = data.surfaces.get(child)
            {
                let (place, _) = origins(data, *child);
                let boxed = RECT {
                    left: ((place.0 - corner.0) * scale).floor() as i32,
                    top: ((place.1 - corner.1) * scale).floor() as i32,
                    right: ((place.0 + under.rect.width - corner.0) * scale).ceil() as i32,
                    bottom: ((place.1 + under.rect.height - corner.1) * scale).ceil() as i32,
                };
                if let Some(inside) = crate::render::meet(clip, boxed) {
                    beneath_list(data, *child, dc, corner, inside, None, depth + 1);
                }
            }
            from = at + 1;
        }
    }
    // Its scrollbars show beneath too, dimmed with it.
    let bars = scrollbar::items(surface, scale, data.zoom);
    if !bars.is_empty() {
        let at = (corner.0 - own.0, corner.1 - own.1);
        render::paint_area(dc, &bars, scale, at, Some(clip), false);
    }
}

/// Paints the part of a surface's display list in an area of a device
/// context, in device pixels.
pub(crate) fn render_surface(window: WindowId, id: u64, dc: HDC, area: RECT) {
    render_surface_over(window, id, dc, area, WHITENESS);
}

/// Paints the root surface of a transparent window into `back` with the
/// alpha it covers what lies behind with, premultiplied, as the window's
/// extended frame composites it. The page is painted over black and over
/// white: where it paints nothing the two differ fully, and where it is
/// opaque they agree, so each pixel's alpha is what the white left showing,
/// and its color is what shows over black.
fn render_see_through(window: WindowId, dc: HDC, back: &Back, area: RECT) {
    render_surface_over(window, 0, back.dc, area, BLACKNESS);
    // The white copy lives beside the surfaces' buffers, under an id no
    // surface has.
    with_back(window, u64::MAX, dc, back.size, |white| {
        render_surface_over(window, 0, white.dc, area, WHITENESS);
        // SAFETY: GDI has finished drawing into both sections, which are
        // at least `back.size` pixels, and `area` lies within them.
        unsafe {
            GdiFlush();
            for y in area.top.max(0)..area.bottom.min(back.size.1) {
                for x in area.left.max(0)..area.right.min(back.size.0) {
                    let over_black = &mut *back.bits.add((y * back.size.0 + x) as usize);
                    let over_white = *white.bits.add((y * white.size.0 + x) as usize);
                    *over_black = see_through(*over_black, over_white);
                }
            }
        }
    });
}

/// A premultiplied BGRA pixel from what a page painted over black and over
/// white.
fn see_through(over_black: u32, over_white: u32) -> u32 {
    let channel = |pixel: u32, shift: u32| (pixel >> shift) & 0xff;
    let shown: u32 = [0, 8, 16]
        .iter()
        .map(|shift| channel(over_white, *shift).saturating_sub(channel(over_black, *shift)))
        .sum();
    let alpha = 255 - (shown + 1) / 3;
    [0, 8, 16].iter().fold(alpha << 24, |pixel, shift| {
        pixel | (channel(over_black, *shift).min(alpha) << shift)
    })
}

/// Paints an area of a surface over a cleared background, `WHITENESS` or
/// `BLACKNESS`.
fn render_surface_over(window: WindowId, id: u64, dc: HDC, area: RECT, clear: ROP_CODE) {
    // Pages park hidden popups far outside the window; nothing of them
    // shows until they move in, which paints them again.
    if outside_window(window, id, area) {
        return;
    }
    // Backdrop filters near the area read pixels beyond it, which are
    // painted too; the caller shows only the area.
    let reach = backdrop_reach(window, id);
    let painted = RECT {
        left: area.left - reach,
        top: area.top - reach,
        right: area.right + reach,
        bottom: area.bottom + reach,
    };
    let (width, height) = (painted.right - painted.left, painted.bottom - painted.top);
    // SAFETY: the DC is live; PatBlt clears the area.
    unsafe { PatBlt(dc, painted.left, painted.top, width, height, clear) };
    // Controls kept out of sight paint as their images.
    let proxies = controls::proxy::snapshots(window, id);
    render::with_proxies(proxies, || {
        STATE.with_borrow(|state| {
            if let Some(data) = state.windows.get(&window)
                && let Some(surface) = data.surfaces.get(&id)
            {
                // The caret shows while the page has the keyboard focus.
                // SAFETY: reads the keyboard focus of this thread.
                let caret = data.caret_on && unsafe { GetFocus() } == data.hwnd;
                let scroll = surface.scroll;
                if surface.items.first() == Some(&DisplayItem::Beneath) {
                    paint_beneath(data, id, dc, painted);
                }
                render::paint_area(dc, &surface.items, data.scale, scroll, Some(painted), caret);
                let bars = scrollbar::items(surface, data.scale, data.zoom);
                if !bars.is_empty() {
                    render::paint_area(dc, &bars, data.scale, (0.0, 0.0), Some(painted), false);
                }
            }
        })
    });
    if let Some(hwnd) = surface_hwnd(window, id) {
        stack::paint(
            window,
            nativeboat_ir::NativeChild::Surface(id),
            hwnd,
            dc,
            painted,
        );
    }
}

fn paint_window(window: WindowId, id: u64, hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    // SAFETY: this handles WM_PAINT for a live window.
    unsafe {
        // Areas far apart, as a line typed into and a word count, paint
        // apart rather than all between them.
        let areas = update_areas(hwnd);
        let dc = BeginPaint(hwnd, &mut paint);
        if areas.is_empty() {
            paint_into(window, id, dc, paint.rcPaint);
        } else {
            for area in areas {
                paint_into(window, id, dc, area);
            }
        }
        EndPaint(hwnd, &paint);
    }
}

/// The rectangles of a window's update region when it is made of a few,
/// else none.
fn update_areas(hwnd: HWND) -> Vec<RECT> {
    const MOST: u32 = 8;
    // SAFETY: the region is created and deleted here; the buffer holds the
    // region data GetRegionData reports, aligned for its RECTs.
    unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        if region.is_null() {
            return Vec::new();
        }
        let mut areas = Vec::new();
        if GetUpdateRgn(hwnd, region, 0) == COMPLEXREGION {
            let size = GetRegionData(region, 0, null_mut());
            let header = size_of::<RGNDATAHEADER>() as u32;
            if size > header {
                let mut buffer = vec![RECT::default(); (size as usize).div_ceil(size_of::<RECT>())];
                if GetRegionData(region, size, buffer.as_mut_ptr().cast()) == size {
                    let data = &*(buffer.as_ptr().cast::<RGNDATAHEADER>());
                    if data.nCount <= MOST {
                        let first = buffer
                            .as_ptr()
                            .cast::<u8>()
                            .add(data.dwSize as usize)
                            .cast::<RECT>();
                        areas.extend_from_slice(core::slice::from_raw_parts(
                            first,
                            data.nCount as usize,
                        ));
                    }
                }
            }
        }
        DeleteObject(region);
        areas
    }
}

/// Paints an area of a surface, in device pixels, at the origin of the
/// supplied DC, respecting its viewport and clip.
fn paint_into(window: WindowId, id: u64, dc: HDC, area: RECT) {
    let (width, height) = (area.right - area.left, area.bottom - area.top);
    if width <= 0 || height <= 0 {
        return;
    }
    let tracing = crate::trace::painting();
    let started = if tracing { microseconds() } else { 0 };
    let mut rendered = started;
    // The frame is painted into a top-down 32-bit DIB section, which GDI+
    // draws into directly. It extends as far as backdrop filters read.
    let reach = backdrop_reach(window, id);
    let size = (area.right + reach, area.bottom + reach);
    let transparent = id == 0 && is_transparent(window);
    with_back(window, id, dc, size, |back| {
        if transparent {
            render_see_through(window, dc, back, area);
        } else {
            render_surface(window, id, back.dc, area);
        }
        if tracing {
            rendered = microseconds();
        }
        // SAFETY: both DCs are live for the copy.
        unsafe {
            BitBlt(
                dc, area.left, area.top, width, height, back.dc, area.left, area.top, SRCCOPY,
            );
        }
    });
    if tracing {
        let copied = microseconds();
        crate::trace::line(&alloc::format!(
            "{started} paint surface {id} {width}x{height} at {},{}: render {:.1} ms, copy {:.1} ms",
            area.left,
            area.top,
            (rendered - started) as f64 / 1000.0,
            (copied - rendered) as f64 / 1000.0,
        ));
    }
}

/// The time in microseconds, for paint traces.
fn microseconds() -> u64 {
    let mut now = FILETIME::default();
    // SAFETY: reads the clock into a local.
    unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime(&mut now)
    };
    (u64::from(now.dwHighDateTime) << 32 | u64::from(now.dwLowDateTime)) / 10
}

/// Scrolls a surface by a delta and reports the new offsets; returns whether
/// it moved.
fn scroll_by(window: WindowId, id: u64, dx: f32, dy: f32) -> bool {
    let moved = STATE.with_borrow_mut(|state| {
        let data = state.windows.get_mut(&window)?;
        let scale = data.scale;
        let surface = data.surfaces.get_mut(&id)?;
        let area = client(surface.hwnd);
        let limit = |content: f32, extent: f32, view: i32| {
            if overflows(content, extent) {
                (content - view as f32 / scale).max(0.0)
            } else {
                0.0
            }
        };
        let max = (
            limit(surface.content.0, surface.rect.width, area.right),
            limit(surface.content.1, surface.rect.height, area.bottom),
        );
        let next = (
            (surface.scroll.0 + dx).clamp(0.0, max.0),
            (surface.scroll.1 + dy).clamp(0.0, max.1),
        );
        if next == surface.scroll {
            return None;
        }
        surface.scroll = next;
        Some((surface.hwnd, next))
    });
    let Some((hwnd, (x, y))) = moved else {
        return false;
    };
    refresh(window, id, hwnd);
    dispatch(window, Event::Scroll { surface: id, x, y });
    true
}

/// The innermost surface of a window at a point on the screen, beneath any
/// controls, whatever covers the window.
fn surface_under(window: WindowId, screen: POINT) -> Option<u64> {
    let root = hwnd(window)?;
    let mut current = root;
    loop {
        let mut point = screen;
        // SAFETY: the windows are this thread's; `point` is writable.
        let child = unsafe {
            ScreenToClient(current, &mut point);
            ChildWindowFromPointEx(current, point, CWP_SKIPINVISIBLE | CWP_SKIPTRANSPARENT)
        };
        if child.is_null() || child == current {
            break;
        }
        current = child;
    }
    STATE.with_borrow(|state| {
        let mut handle = current;
        loop {
            if let Some((owner, Role::Surface(id))) = state.handles.get(&(handle as usize))
                && *owner == window
            {
                return Some(*id);
            }
            if handle == root {
                return None;
            }
            // SAFETY: walks up this thread's window tree.
            handle = unsafe { GetParent(handle) };
            if handle.is_null() {
                return None;
            }
        }
    })
}

fn parent_surface(window: WindowId, id: u64) -> u64 {
    STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .and_then(|data| data.surfaces.get(&id))
            .map_or(0, |surface| surface.parent)
    })
}

fn scroll_bar(window: WindowId, id: u64, hwnd: HWND, message: u32, wparam: WPARAM) {
    let bar = if message == WM_VSCROLL {
        SB_VERT
    } else {
        SB_HORZ
    };
    let mut info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        ..SCROLLINFO::default()
    };
    // SAFETY: `info` is initialised and writable.
    unsafe { GetScrollInfo(hwnd, bar, &mut info) };
    let scale = scale(window);
    let position = info.nPos as f32;
    let target = match (wparam & 0xFFFF) as i32 {
        SB_LINEUP => position - LINE * scale,
        SB_LINEDOWN => position + LINE * scale,
        SB_PAGEUP => position - info.nPage as f32,
        SB_PAGEDOWN => position + info.nPage as f32,
        SB_THUMBTRACK | SB_THUMBPOSITION => info.nTrackPos as f32,
        SB_TOP => 0.0,
        SB_BOTTOM => info.nMax as f32,
        _ => position,
    };
    let delta = (target - position) / scale;
    if bar == SB_VERT {
        scroll_by(window, id, 0.0, delta);
    } else {
        scroll_by(window, id, delta, 0.0);
    }
}

pub(crate) fn root_point(window: WindowId, hwnd: HWND, x: i32, y: i32, screen: bool) -> (f32, f32) {
    let root = self::hwnd(window).unwrap_or(hwnd);
    let mut point = POINT { x, y };
    // SAFETY: both handles belong to live windows of this thread.
    unsafe {
        if screen {
            ScreenToClient(root, &mut point);
        } else {
            MapWindowPoints(hwnd, root, &mut point, 1);
        }
    }
    let scale = scale(window);
    (point.x as f32 / scale, point.y as f32 / scale)
}

fn low(value: isize) -> i32 {
    i32::from(value as u16 as i16)
}

fn high(value: isize) -> i32 {
    i32::from((value >> 16) as u16 as i16)
}

fn dropped_files(window: WindowId, hwnd: HWND, drop: HDROP) {
    let mut point = POINT::default();
    // SAFETY: WM_DROPFILES transfers a shell-owned HDROP to this procedure.
    let count = unsafe { DragQueryFileW(drop, u32::MAX, null_mut(), 0) };
    let mut paths = Vec::new();
    for index in 0..count {
        // SAFETY: queries the required size of a path in this drop handle.
        let length = unsafe { DragQueryFileW(drop, index, null_mut(), 0) };
        let mut path = vec![0_u16; length as usize + 1];
        // SAFETY: the buffer includes room for the terminating NUL.
        if unsafe { DragQueryFileW(drop, index, path.as_mut_ptr(), length + 1) } > 0 {
            paths.push(narrow(&path));
        }
    }
    // SAFETY: reads the client drop position, then releases the handle once.
    unsafe {
        DragQueryPoint(drop, &mut point);
        DragFinish(drop);
    }
    if !paths.is_empty() {
        let (x, y) = root_point(window, hwnd, point.x, point.y, false);
        dispatch(
            window,
            Event::FilesDropped {
                paths,
                strings: Vec::new(),
                x,
                y,
            },
        );
    }
}

fn wheel(window: WindowId, id: u64, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
    let notches = high(wparam as isize) as f32 / WHEEL_DELTA as f32;
    let modifiers = keys::modifiers();
    let (delta_x, delta_y) = if message == WM_MOUSEHWHEEL {
        (notches * NOTCH, 0.0)
    } else if modifiers.shift {
        (-notches * NOTCH, 0.0)
    } else {
        (0.0, -notches * NOTCH)
    };
    let (x, y) = root_point(window, hwnd, low(lparam), high(lparam), true);
    let event = Event::Wheel {
        x,
        y,
        delta_x,
        delta_y,
        modifiers,
    };
    if dispatch(window, event) {
        return;
    }
    // Wheel turns scroll the innermost scroller under the pointer, as in
    // browsers, whichever window Windows sent them to.
    let screen = POINT {
        x: i32::from(low(lparam) as i16),
        y: i32::from(high(lparam) as i16),
    };
    let mut current = surface_under(window, screen).unwrap_or(id);
    loop {
        let (dx, dy) = wheel_step(window, current, delta_x, delta_y);
        if scroll_by(window, current, dx, dy) || current == 0 {
            break;
        }
        current = parent_surface(window, current);
    }
}

/// A wheel message scrolls a surface at most its own visible size, as in
/// Chromium, however far a fast wheel turned.
fn wheel_step(window: WindowId, id: u64, dx: f32, dy: f32) -> (f32, f32) {
    STATE
        .with_borrow(|state| {
            let data = state.windows.get(&window)?;
            let surface = data.surfaces.get(&id)?;
            let area = client(surface.hwnd);
            let width = area.right as f32 / data.scale;
            let height = area.bottom as f32 / data.scale;
            Some((dx.clamp(-width, width), dy.clamp(-height, height)))
        })
        .unwrap_or((dx, dy))
}

local! {
    /// The screen point and key flags of the last mouse message, when it
    /// was a move.
    static LAST_MOVE: Cell<Option<(POINT, WPARAM)>> = const { Cell::new(None) };
}

/// Whether a mouse message is a move to where the last one already moved
/// the pointer, which Windows sends when windows change under a still
/// pointer; Chromium drops these, so pages see no mouse move.
fn repeated_move(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    if message != WM_MOUSEMOVE {
        LAST_MOVE.set(None);
        return false;
    }
    let mut point = POINT {
        x: low(lparam),
        y: high(lparam),
    };
    // SAFETY: converts a point of this live window's client area.
    unsafe { ClientToScreen(hwnd, &mut point) };
    let last = LAST_MOVE.replace(Some((point, wparam)));
    last.is_some_and(|(at, flags)| at.x == point.x && at.y == point.y && flags == wparam)
}

fn pointer(window: WindowId, hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) {
    let extra = if (wparam >> 16) as u16 == XBUTTON1 {
        3
    } else {
        4
    };
    let (kind, button, count) = match message {
        WM_LBUTTONDOWN => (PointerKind::Down, 0, 1),
        WM_LBUTTONDBLCLK => (PointerKind::Down, 0, 2),
        WM_LBUTTONUP => (PointerKind::Up, 0, 1),
        WM_MBUTTONDOWN => (PointerKind::Down, 1, 1),
        WM_MBUTTONDBLCLK => (PointerKind::Down, 1, 2),
        WM_MBUTTONUP => (PointerKind::Up, 1, 1),
        WM_RBUTTONDOWN => (PointerKind::Down, 2, 1),
        WM_RBUTTONDBLCLK => (PointerKind::Down, 2, 2),
        WM_RBUTTONUP => (PointerKind::Up, 2, 1),
        WM_XBUTTONDOWN => (PointerKind::Down, extra, 1),
        WM_XBUTTONDBLCLK => (PointerKind::Down, extra, 2),
        WM_XBUTTONUP => (PointerKind::Up, extra, 1),
        _ => (PointerKind::Move, 0, 0),
    };
    let flags = wparam as u32;
    let buttons = [
        (MK_LBUTTON, 1),
        (MK_RBUTTON, 2),
        (MK_MBUTTON, 4),
        (MK_XBUTTON1, 8),
        (MK_XBUTTON2, 16),
    ]
    .into_iter()
    .filter(|(flag, _)| flags & flag != 0)
    .map(|(_, bit)| bit)
    .sum();
    let held = keys::modifiers();
    let modifiers = Modifiers {
        shift: flags & MK_SHIFT != 0,
        ctrl: flags & MK_CONTROL != 0,
        ..held
    };
    let (x, y) = root_point(window, hwnd, low(lparam), high(lparam), false);
    // Pages see click counts as Chromium counts them, not as Windows' double
    // click messages do: a triple click's third press is a plain press.
    let count = match kind {
        PointerKind::Down => {
            let mut point = POINT {
                x: low(lparam),
                y: high(lparam),
            };
            // SAFETY: converts a point of this live window's client area.
            unsafe { ClientToScreen(hwnd, &mut point) };
            clicks::press(button, (point.x, point.y), &modifiers)
        }
        PointerKind::Up => clicks::release(button),
        _ => count,
    };
    // SAFETY: the handle belongs to a live window of this thread.
    unsafe {
        match kind {
            PointerKind::Down => {
                SetCapture(hwnd);
            }
            PointerKind::Up => {
                ReleaseCapture();
            }
            _ => {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut track);
            }
        }
    }
    let cancelled = dispatch(
        window,
        Event::Pointer {
            kind,
            button,
            buttons,
            count,
            x,
            y,
            modifiers,
        },
    );
    // Pressing on painted content takes the focus from native controls.
    if kind == PointerKind::Down
        && !cancelled
        && let Some(root) = self::hwnd(window)
    {
        STATE.with_borrow_mut(|state| {
            if let Some(data) = state.windows.get_mut(&window) {
                data.focus_target = None;
            }
        });
        // SAFETY: the handle belongs to a live window of this thread.
        unsafe { SetFocus(root) };
    }
}

/// Tells the page about the pointer moving over a native control and
/// leaving it, as over the part of the page the control shows: Chromium's
/// controls are part of the page, so `:hover`, mouse events and the cursor
/// the page asks for follow the pointer over them.
pub(crate) fn control_pointer(
    window: WindowId,
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) {
    match message {
        WM_MOUSEMOVE if !repeated_move(hwnd, message, wparam, lparam) => {
            pointer(window, hwnd, message, wparam, lparam);
        }
        WM_MOUSELEAVE => {
            LAST_MOVE.set(None);
            leave(window);
        }
        _ => {}
    }
}

/// Sets the cursor the page asks for over a native control; `false` when
/// the page leaves the cursor to the control.
pub(crate) fn control_cursor(window: WindowId) -> bool {
    let cursor = STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .map_or(Cursor::Auto, |data| data.cursor)
    });
    if cursor == Cursor::Auto {
        return false;
    }
    set_cursor_now(window);
    true
}

fn leave(window: WindowId) {
    let mut point = POINT::default();
    // SAFETY: reads the cursor position and the window under it.
    let inside = unsafe {
        GetCursorPos(&mut point);
        let under = WindowFromPoint(point);
        !under.is_null() && hwnd(window) == Some(GetAncestor(under, GA_ROOT))
    };
    if !inside {
        dispatch(
            window,
            Event::Pointer {
                kind: PointerKind::Leave,
                button: 0,
                buttons: 0,
                count: 0,
                x: 0.0,
                y: 0.0,
                modifiers: keys::modifiers(),
            },
        );
    }
}

/// Gives the owning application the first chance to cancel a queued key.
pub(crate) fn before_input(message: &MSG) -> bool {
    match keyboard_input(
        message.hwnd as usize,
        message.message,
        message.wParam,
        message.lParam,
    ) {
        Some((window, input)) => dispatch(window, Event::BeforeInput(input)),
        None => false,
    }
}

/// The window a queued key message is for, among this thread's windows,
/// and the key as an application sees it before the page does; `None`
/// for other messages and windows. `hwnd` is the message's window handle.
pub fn keyboard_input(
    hwnd: usize,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<(WindowId, KeyboardInput)> {
    let hwnd = hwnd as HWND;
    if !matches!(message, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP) || hwnd.is_null() {
        return None;
    }
    // SAFETY: querying the message's root window has no ownership effects.
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    let window = STATE.with_borrow(|state| {
        state
            .handles
            .get(&(root as usize))
            .map(|(window, _)| *window)
    })?;
    let vk = wparam as u16;
    let scan = ((lparam >> 16) & 0xFF) as u32;
    let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
    let code = keys::code(vk, scan, lparam & (1 << 24) != 0);
    let location = if code.starts_with("Numpad") {
        3
    } else if matches!(
        code.as_str(),
        "ShiftLeft" | "ControlLeft" | "AltLeft" | "MetaLeft"
    ) {
        1
    } else if matches!(
        code.as_str(),
        "ShiftRight" | "ControlRight" | "AltRight" | "MetaRight"
    ) {
        2
    } else {
        0
    };
    // SAFETY: reads this thread's lock-key toggle state.
    let (caps_lock, num_lock) = unsafe {
        (
            GetKeyState(i32::from(VK_CAPITAL)) & 1 != 0,
            GetKeyState(i32::from(VK_NUMLOCK)) & 1 != 0,
        )
    };
    Some((
        window,
        KeyboardInput {
            down,
            key: keys::key(vk, scan),
            code,
            repeat: down && lparam & (1 << 30) != 0,
            composing: controls::composing(hwnd).unwrap_or_else(|| ime::active(window)),
            modifiers: keys::modifiers(),
            location,
            caps_lock,
            num_lock,
        },
    ))
}

/// Reports a key message; returns whether the handler cancelled it.
pub(crate) fn key(
    window: WindowId,
    control: Option<u64>,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> bool {
    let vk = wparam as u16;
    let scan = ((lparam >> 16) & 0xFF) as u32;
    let down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
    dispatch(
        window,
        Event::Key {
            down,
            key: keys::key(vk, scan),
            code: keys::code(vk, scan, lparam & (1 << 24) != 0),
            repeat: down && lparam & (1 << 30) != 0,
            modifiers: keys::modifiers(),
            control,
        },
    )
}

/// Reports a key message for a surface and remembers whether the page
/// cancelled a press, which drops the characters it types.
fn surface_key(window: WindowId, message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    let cancelled = key(window, None, message, wparam, lparam);
    CANCELLED_KEY.set(cancelled && matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN));
    cancelled
}

/// Reports a typed UTF-16 unit as text, joining surrogate pairs and
/// leaving out control characters and the characters of cancelled keys.
fn typed(window: WindowId, unit: u16) {
    if ime::active(window) {
        HIGH_SURROGATE.set(0);
        return;
    }
    if (0xD800..0xDC00).contains(&unit) {
        HIGH_SURROGATE.set(unit);
        return;
    }
    let high = HIGH_SURROGATE.replace(0);
    let units: &[u16] = if (0xDC00..0xE000).contains(&unit) {
        &[high, unit]
    } else {
        &[unit]
    };
    let Ok(text) = String::from_utf16(units) else {
        return;
    };
    if !CANCELLED_KEY.get() && !text.chars().any(char::is_control) {
        dispatch(window, Event::Text(text));
    }
}

fn cursor_name(cursor: Cursor) -> PCWSTR {
    match cursor {
        Cursor::Pointer | Cursor::Grab | Cursor::Grabbing => IDC_HAND,
        Cursor::Text => IDC_IBEAM,
        Cursor::Move => IDC_SIZEALL,
        Cursor::NotAllowed => IDC_NO,
        Cursor::Wait => IDC_WAIT,
        Cursor::Progress => IDC_APPSTARTING,
        Cursor::Help => IDC_HELP,
        Cursor::Crosshair => IDC_CROSS,
        Cursor::ColResize | Cursor::EwResize => IDC_SIZEWE,
        Cursor::RowResize | Cursor::NsResize => IDC_SIZENS,
        Cursor::NeswResize => IDC_SIZENESW,
        Cursor::NwseResize => IDC_SIZENWSE,
        _ => IDC_ARROW,
    }
}

fn set_cursor_now(window: WindowId) {
    let cursor = STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .map_or(Cursor::Auto, |data| data.cursor)
    });
    // SAFETY: system cursors are shared and need no cleanup.
    unsafe {
        let handle = if cursor == Cursor::None {
            null_mut()
        } else {
            LoadCursorW(null_mut(), cursor_name(cursor))
        };
        SetCursor(handle);
    }
}

fn resized(window: WindowId) {
    if !update_scrollbars(window, 0) {
        report_viewport(window);
    }
}

/// Tells the page the client area's size, which it reports also when only
/// the room its root scrollbars take changed, for the page to lay out again.
fn report_viewport(window: WindowId) {
    let (width, height) = client_size(window);
    dispatch(window, Event::Resized { width, height });
}

fn dpi_changed(window: WindowId, hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let scale = (wparam & 0xFFFF) as f32 / 96.0;
    STATE.with_borrow_mut(|state| {
        if let Some(data) = state.windows.get_mut(&window) {
            data.scale = scale * data.zoom;
        }
    });
    // SAFETY: WM_DPICHANGED passes the suggested window rectangle.
    let suggested = unsafe { *(lparam as *const RECT) };
    // SAFETY: the handle belongs to a live window of this thread.
    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            suggested.left,
            suggested.top,
            suggested.right - suggested.left,
            suggested.bottom - suggested.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    };
    rescale(window);
}

/// Places and repaints a window's controls and surfaces at its scale, and
/// reports its size in CSS pixels.
fn rescale(window: WindowId) {
    controls::rescale(window);
    let surfaces: Vec<(u64, HWND)> = STATE.with_borrow(|state| {
        state.windows.get(&window).map_or(Vec::new(), |data| {
            data.surfaces
                .iter()
                .map(|(id, surface)| (*id, surface.hwnd))
                .collect()
        })
    });
    for (id, surface) in surfaces {
        refresh(window, id, surface);
    }
    report_viewport(window);
}

fn limits(window: WindowId, lparam: LPARAM) {
    let found = STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .map(|data| (data.scale, data.min_size, data.max_size))
    });
    // SAFETY: WM_GETMINMAXINFO passes a writable MINMAXINFO.
    let info = unsafe { &mut *(lparam as *mut MINMAXINFO) };
    let (scale, min, max) = found.unwrap_or((1.0, None, None));
    if let Some((width, height)) = min {
        info.ptMinTrackSize = POINT {
            x: px(width, scale),
            y: px(height, scale),
        };
    }
    if let Some((width, height)) = max {
        info.ptMaxTrackSize = POINT {
            x: px(width, scale),
            y: px(height, scale),
        };
    }
}

/// The length of a NUL-terminated string, at most 64 units.
///
/// # Safety
///
/// `text` points at a NUL-terminated string.
unsafe fn area_length(text: *const u16) -> usize {
    // SAFETY: reads up to the terminator, which the caller guarantees.
    (0..64)
        .find(|index| unsafe { *text.add(*index) } == 0)
        .unwrap_or(64)
}

fn destroyed(window: WindowId) {
    if let Some(hwnd) = hwnd(window) {
        // SAFETY: OLE releases its registered reference; an unregistered
        // legacy fallback window returns an error harmlessly.
        unsafe { windows_sys::Win32::System::Ole::RevokeDragDrop(hwnd) };
    }
    crate::menu::destroyed(window);
    release_backs(window, None);
    STATE.with_borrow_mut(|state| {
        state.windows.remove(&window);
        state.handles.retain(|_, (owner, _)| *owner != window);
    });
    dispatch(window, Event::Destroyed);
}

/// Handles a message for a surface; `None` leaves it to `DefWindowProcW`.
fn handle(
    window: WindowId,
    id: u64,
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    if id == 0
        && let Some(result) = ime::handle(window, hwnd, message, wparam, lparam)
    {
        return Some(result);
    }
    match message {
        WM_DROPFILES => {
            dropped_files(window, hwnd, wparam as HDROP);
            Some(0)
        }
        WM_ERASEBKGND => Some(1),
        WM_PAINT => {
            paint_window(window, id, hwnd);
            Some(0)
        }
        WM_PRINTCLIENT => {
            paint_into(window, id, wparam as HDC, client(hwnd));
            Some(0)
        }
        // A scroll container's native scroll bars paint with the page
        // content above them.
        WM_NCPAINT
            if id != 0
                && stack::paint_frame(
                    window,
                    nativeboat_ir::NativeChild::Surface(id),
                    hwnd,
                    |_, _| {},
                ) =>
        {
            Some(0)
        }
        WM_TIMER if id == 0 && wparam == CARET_TIMER => {
            caret_phase(window, None);
            Some(0)
        }
        WM_TIMER if wparam == scrollbar::REPEAT_TIMER => {
            scrollbar::repeat(window, id);
            Some(0)
        }
        WM_SETFOCUS | WM_KILLFOCUS if id == 0 => {
            if message == WM_KILLFOCUS {
                ime::complete(window, hwnd);
            }
            caret_phase(window, Some(message == WM_SETFOCUS));
            // A window gaining the focus passes it to the control its page
            // focused, as browsers restore the focused element.
            let target = STATE.with_borrow(|state| {
                state
                    .windows
                    .get(&window)
                    .and_then(|data| data.focus_target)
            });
            if message == WM_SETFOCUS
                && let Some(control) = target.and_then(|target| controls::hwnd(window, target))
            {
                // SAFETY: the control belongs to this live window.
                unsafe { SetFocus(control) };
            }
            None
        }
        WM_SIZE if id == 0 && wparam != SIZE_MINIMIZED as usize => {
            resized(window);
            Some(0)
        }
        WM_VSCROLL | WM_HSCROLL if lparam == 0 => {
            scroll_bar(window, id, hwnd, message, wparam);
            Some(0)
        }
        WM_HSCROLL => {
            controls::slider(window, lparam as HWND);
            Some(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            wheel(window, id, hwnd, message, wparam, lparam);
            Some(0)
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK | WM_MBUTTONDOWN
        | WM_MBUTTONUP | WM_MBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_RBUTTONDBLCLK
        | WM_XBUTTONDOWN | WM_XBUTTONUP | WM_XBUTTONDBLCLK => {
            if !repeated_move(hwnd, message, wparam, lparam)
                && !scrollbar::pointer(window, id, message, wparam, lparam)
            {
                pointer(window, hwnd, message, wparam, lparam);
            }
            Some(0)
        }
        WM_MOUSELEAVE => {
            LAST_MOVE.set(None);
            scrollbar::leave(window, id);
            leave(window);
            Some(0)
        }
        WM_SETCURSOR if (lparam & 0xFFFF) as u32 == HTCLIENT => {
            if scrollbar::cursor(window, id) {
                // SAFETY: installs a shared system cursor.
                unsafe {
                    SetCursor(LoadCursorW(null_mut(), IDC_ARROW));
                }
            } else {
                set_cursor_now(window);
            }
            Some(1)
        }
        WM_CAPTURECHANGED | WM_CANCELMODE => {
            scrollbar::cancel(window, id, message == WM_CANCELMODE);
            None
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            surface_key(window, message, wparam, lparam).then_some(0)
        }
        WM_CHAR => {
            typed(window, wparam as u16);
            Some(0)
        }
        WM_COMMAND if lparam != 0 => {
            controls::command(window, wparam, lparam as HWND);
            Some(0)
        }
        WM_NOTIFY if lparam != 0 => {
            // SAFETY: a notification's parameter points to its header.
            controls::notify(window, unsafe { &*(lparam as *const NMHDR) });
            None
        }
        WM_COMMAND if id == 0 => {
            dispatch(
                window,
                Event::Menu(MenuCommand {
                    id: (wparam & 0xFFFF) as u16,
                    accelerator: (wparam >> 16) & 0xFFFF == 1,
                    modifiers: keys::modifiers(),
                }),
            );
            Some(0)
        }
        WM_SETTINGCHANGE if id == 0 && lparam != 0 => {
            // SAFETY: the parameter of a setting change names the area.
            let area = unsafe {
                crate::wide::narrow(core::slice::from_raw_parts(
                    lparam as *const u16,
                    area_length(lparam as *const u16),
                ))
            };
            if area == "ImmersiveColorSet" {
                dispatch(window, Event::ThemeChanged);
            }
            None
        }
        WM_POWERBROADCAST if id == 0 => {
            match wparam as u32 {
                PBT_APMSUSPEND => {
                    dispatch(window, Event::Suspend);
                }
                PBT_APMRESUMEAUTOMATIC => {
                    dispatch(window, Event::Resume);
                }
                _ => {}
            }
            Some(1)
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX => {
            controls::color(window, wparam as HDC, lparam as HWND)
        }
        WM_ACTIVATE => {
            dispatch(
                window,
                Event::Activated((wparam & 0xFFFF) as u32 != WA_INACTIVE),
            );
            None
        }
        WM_CLOSE => {
            if !dispatch(window, Event::Close) {
                // SAFETY: the handle belongs to a live window of this thread.
                unsafe { DestroyWindow(hwnd) };
            }
            Some(0)
        }
        WM_DESTROY if id == 0 => {
            destroyed(window);
            Some(0)
        }
        WM_DPICHANGED => {
            dpi_changed(window, hwnd, wparam, lparam);
            Some(0)
        }
        WM_GETMINMAXINFO => {
            limits(window, lparam);
            Some(0)
        }
        _ => None,
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let role = STATE.with_borrow(|state| state.handles.get(&(hwnd as usize)).copied());
    if let Some((window, Role::Surface(id))) = role
        && let Some(result) = handle(window, id, hwnd, message, wparam, lparam)
    {
        return result;
    }
    // SAFETY: forwards the message unchanged.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// The procedure of adopted windows: their content's messages are this
/// crate's; the rest are their host's.
unsafe extern "system" fn adopted_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let host = HOSTS.with_borrow(|hosts| hosts.get(&(hwnd as usize)).copied());
    let window = STATE.with_borrow(|state| state.handles.get(&(hwnd as usize)).map(|(id, _)| *id));
    let forward = || {
        // SAFETY: the host procedure was the window's procedure, and takes
        // the message unchanged.
        unsafe {
            CallWindowProcW(
                core::mem::transmute::<isize, WNDPROC>(host.unwrap_or_default()),
                hwnd,
                message,
                wparam,
                lparam,
            )
        }
    };
    let Some(window) = window else {
        if message == WM_NCDESTROY {
            HOSTS.with_borrow_mut(|hosts| hosts.remove(&(hwnd as usize)));
        }
        return forward();
    };
    match message {
        // The frame, its lifecycle and the menu stay with the host.
        WM_CLOSE | WM_ACTIVATE | WM_GETMINMAXINFO | WM_NCCALCSIZE | WM_NCHITTEST
        | WM_SETTINGCHANGE | WM_POWERBROADCAST => forward(),
        WM_COMMAND if lparam == 0 => forward(),
        WM_DESTROY => {
            destroyed(window);
            forward()
        }
        // Both follow focus and size changes.
        WM_SIZE | WM_SETFOCUS | WM_KILLFOCUS => {
            let _ = handle(window, 0, hwnd, message, wparam, lparam);
            forward()
        }
        _ => handle(window, 0, hwnd, message, wparam, lparam).unwrap_or_else(forward),
    }
}

#[cfg(test)]
mod tests;
