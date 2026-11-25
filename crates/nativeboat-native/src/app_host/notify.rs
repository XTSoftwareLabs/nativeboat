//! The notification area and global shortcuts: a hidden window that receives
//! notification icons' callbacks and `WM_HOTKEY`, and owns the context menus
//! icons open. The main realm reacts through one handler, as for menus.

use super::{GetLastError, GetModuleHandleW, Hwnd, NativeMenuItem, WinRect, Window, menu_draw};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// `NOTIFYICONDATAW`.
#[repr(C)]
struct NotifyIconData {
    size: u32,
    window: Hwnd,
    id: u32,
    flags: u32,
    callback_message: u32,
    icon: *mut c_void,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    timeout_or_version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: [u8; 16],
    balloon_icon: *mut c_void,
}

/// `NOTIFYICONIDENTIFIER`.
#[repr(C)]
struct NotifyIconIdentifier {
    size: u32,
    window: Hwnd,
    id: u32,
    guid: [u8; 16],
}

/// `ICONINFO`.
#[repr(C)]
struct IconInfo {
    icon: i32,
    hotspot_x: u32,
    hotspot_y: u32,
    mask: *mut c_void,
    color: *mut c_void,
}

#[repr(C)]
struct WindowClass {
    style: u32,
    procedure: Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>,
    class_extra: i32,
    window_extra: i32,
    instance: *mut c_void,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn Shell_NotifyIconW(message: u32, data: *mut NotifyIconData) -> i32;
    fn Shell_NotifyIconGetRect(identifier: *const NotifyIconIdentifier, rect: *mut WinRect) -> i32;
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
        instance: *mut c_void,
        parameter: *mut c_void,
    ) -> Hwnd;
    fn DefWindowProcW(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn RegisterHotKey(window: Hwnd, id: i32, modifiers: u32, key: u32) -> i32;
    fn UnregisterHotKey(window: Hwnd, id: i32) -> i32;
    fn CreateIconIndirect(info: *const IconInfo) -> *mut c_void;
    fn DestroyIcon(icon: *mut c_void) -> i32;
    fn LoadIconW(instance: *mut c_void, name: *const u16) -> *mut c_void;
    fn SetForegroundWindow(window: Hwnd) -> i32;
    fn PostMessageW(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> i32;
    fn GetCursorPos(point: *mut super::frame::Point) -> i32;
    fn RegisterSuspendResumeNotification(recipient: *mut c_void, flags: u32) -> *mut c_void;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemPowerStatus(status: *mut PowerStatus) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn ProcessIdToSessionId(process: u32, session: *mut u32) -> i32;
    fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
}

/// `SYSTEM_POWER_STATUS`.
#[repr(C)]
#[derive(Default)]
struct PowerStatus {
    ac_line: u8,
    battery_flag: u8,
    battery_percent: u8,
    saver: u8,
    battery_seconds: u32,
    full_battery_seconds: u32,
}

/// Whether the computer runs on its battery: Windows reports it off mains
/// power.
pub fn on_battery() -> bool {
    let mut status = PowerStatus::default();
    // SAFETY: the status is writable and sized as Windows expects.
    let read = unsafe { GetSystemPowerStatus(&mut status) };
    read != 0 && status.ac_line == 0
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateBitmap(
        width: i32,
        height: i32,
        planes: u32,
        bits: u32,
        data: *const c_void,
    ) -> *mut c_void;
    fn DeleteObject(object: *mut c_void) -> i32;
}

const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIF_MESSAGE: u32 = 0x1;
const NIF_ICON: u32 = 0x2;
const NIF_TIP: u32 = 0x4;
const NIF_INFO: u32 = 0x10;
const NIF_SHOWTIP: u32 = 0x80;
const NIIF_NONE: u32 = 0;
const NIIF_INFO: u32 = 1;
const NIIF_WARNING: u32 = 2;
const NIIF_ERROR: u32 = 3;
const NIIF_USER: u32 = 4;
const NIIF_NOSOUND: u32 = 0x10;
const NIIF_LARGE_ICON: u32 = 0x20;
/// The callback message of every icon.
const WM_NOTIFY_ICON: u32 = 0x8000 + 1;
const WM_HOTKEY: u32 = 0x0312;
const WM_POWERBROADCAST: u32 = 0x0218;
const WM_WTSSESSION_CHANGE: u32 = 0x02b1;
const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
const WM_SYSCOLORCHANGE: u32 = 0x0015;
const WM_DRAWITEM: u32 = 0x002b;
const WM_MEASUREITEM: u32 = 0x002c;
const WM_MENUCHAR: u32 = 0x0120;
const WM_MENUSELECT: u32 = 0x011f;

/// What happened to a notification icon, or a global shortcut's press.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyEvent {
    Click = 0,
    RightClick = 1,
    DoubleClick = 2,
    MiddleClick = 3,
    MouseMove = 4,
    BalloonShow = 5,
    BalloonClick = 6,
    BalloonClosed = 7,
    /// A global shortcut; the identifier is the shortcut's.
    Hotkey = 8,
    /// The computer goes to sleep.
    Suspend = 9,
    /// The computer woke up.
    Resume = 10,
    /// The computer now runs on mains power.
    OnAc = 11,
    /// The computer now runs on its battery.
    OnBattery = 12,
    /// The user's session was locked.
    LockScreen = 13,
    /// The user's session was unlocked.
    UnlockScreen = 14,
    /// Windows' accent color changed; the identifier is the new color,
    /// `0xAARRGGBB`.
    AccentColor = 15,
    /// Windows' system colors changed.
    SystemColors = 16,
}

/// Callback for notification events: context, event, and the identifier of
/// the icon or shortcut.
pub type NotifyHandler = unsafe extern "system" fn(*mut c_void, NotifyEvent, u32);

static WINDOW: AtomicUsize = AtomicUsize::new(0);
/// The power source last seen: 0 unknown, 1 mains, 2 battery.
static POWER_SOURCE: AtomicU32 = AtomicU32::new(0);
/// Whether session and sleep notifications reach the window.
static WATCHING: AtomicU32 = AtomicU32::new(0);

/// The Remote Desktop session this process runs in.
fn session() -> u32 {
    let mut session = 0;
    // SAFETY: the session is writable.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
    session
}

/// Has the notification window take the system's power, session and
/// color notifications, as Electron's power monitor and system preferences
/// do. Power and color changes reach every top-level window; session locks
/// and connected-standby sleep need asking for.
pub fn watch_system() -> Result<(), u32> {
    let window = window()?;
    if WATCHING.swap(1, Ordering::AcqRel) == 1 {
        return Ok(());
    }
    POWER_SOURCE.store(if on_battery() { 2 } else { 1 }, Ordering::Release);
    const NOTIFY_FOR_THIS_SESSION: u32 = 0;
    const DEVICE_NOTIFY_WINDOW_HANDLE: u32 = 0;
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
    let library: Vec<u16> = "wtsapi32.dll\0".encode_utf16().collect();
    // SAFETY: the name is null-terminated; system32 is searched only.
    let module = unsafe {
        LoadLibraryExW(
            library.as_ptr(),
            core::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if !module.is_null() {
        // SAFETY: the module is loaded and the name null-terminated.
        let register =
            unsafe { GetProcAddress(module, c"WTSRegisterSessionNotification".as_ptr().cast()) };
        if !register.is_null() {
            type Register = unsafe extern "system" fn(Hwnd, u32) -> i32;
            // SAFETY: the address is WTSRegisterSessionNotification's.
            unsafe {
                let register: Register = core::mem::transmute(register);
                register(window, NOTIFY_FOR_THIS_SESSION);
            }
        }
    }
    // SAFETY: the window is live; the registration lasts the process.
    unsafe { RegisterSuspendResumeNotification(window.cast(), DEVICE_NOTIFY_WINDOW_HANDLE) };
    Ok(())
}
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);
static HANDLER: AtomicUsize = AtomicUsize::new(0);
static CONTEXT: AtomicUsize = AtomicUsize::new(0);

/// Installs the main realm's callback for notification icons and global
/// shortcuts. The context must stay valid until the message loop exits.
pub fn set_notify_handler(context: *mut c_void, handler: Option<NotifyHandler>) {
    CONTEXT.store(context as usize, Ordering::Release);
    HANDLER.store(
        handler.map_or(0, |handler| handler as usize),
        Ordering::Release,
    );
}

fn dispatch(event: NotifyEvent, id: u32) {
    let handler = HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        return;
    }
    // SAFETY: installed with this ABI by `set_notify_handler`.
    let handler: NotifyHandler = unsafe { core::mem::transmute(handler) };
    let context = CONTEXT.load(Ordering::Acquire) as *mut c_void;
    // SAFETY: the context and handler follow `set_notify_handler`'s contract.
    unsafe { handler(context, event, id) };
}

/// The icons shown, to show them again when Explorer restarts.
static ICONS: shown::Icons = shown::Icons::new();

mod shown {
    //! The icons this process shows: identifier, icon and tip, kept on the
    //! UI thread only.
    use alloc::vec::Vec;
    use core::cell::RefCell;

    pub(super) struct Icons(RefCell<Vec<(u32, usize, Vec<u16>)>>);

    // SAFETY: notification icons are created and changed on the UI thread only.
    unsafe impl Sync for Icons {}

    impl Icons {
        pub(super) const fn new() -> Self {
            Icons(RefCell::new(Vec::new()))
        }

        pub(super) fn set(&self, id: u32, icon: usize, tip: Vec<u16>) {
            let mut icons = self.0.borrow_mut();
            match icons.iter_mut().find(|entry| entry.0 == id) {
                Some(entry) => *entry = (id, icon, tip),
                None => icons.push((id, icon, tip)),
            }
        }

        pub(super) fn get(&self, id: u32) -> Option<(usize, Vec<u16>)> {
            self.0
                .borrow()
                .iter()
                .find(|entry| entry.0 == id)
                .map(|entry| (entry.1, entry.2.clone()))
        }

        pub(super) fn remove(&self, id: u32) {
            self.0.borrow_mut().retain(|entry| entry.0 != id);
        }

        pub(super) fn all(&self) -> Vec<(u32, usize, Vec<u16>)> {
            self.0.borrow().clone()
        }
    }
}

unsafe extern "system" fn procedure(
    window: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    const WM_MOUSEMOVE: u32 = 0x0200;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_MBUTTONUP: u32 = 0x0208;
    const NIN_BALLOONSHOW: u32 = 0x0402;
    const NIN_BALLOONTIMEOUT: u32 = 0x0404;
    const NIN_BALLOONUSERCLICK: u32 = 0x0405;
    match message {
        WM_NOTIFY_ICON => {
            let event = match lparam as u32 & 0xffff {
                WM_LBUTTONUP => Some(NotifyEvent::Click),
                WM_RBUTTONUP => Some(NotifyEvent::RightClick),
                WM_LBUTTONDBLCLK => Some(NotifyEvent::DoubleClick),
                WM_MBUTTONUP => Some(NotifyEvent::MiddleClick),
                WM_MOUSEMOVE => Some(NotifyEvent::MouseMove),
                NIN_BALLOONSHOW => Some(NotifyEvent::BalloonShow),
                NIN_BALLOONUSERCLICK => Some(NotifyEvent::BalloonClick),
                NIN_BALLOONTIMEOUT => Some(NotifyEvent::BalloonClosed),
                _ => None,
            };
            if let Some(event) = event {
                dispatch(event, wparam as u32);
            }
            return 0;
        }
        WM_HOTKEY => {
            dispatch(NotifyEvent::Hotkey, wparam as u32);
            return 0;
        }
        WM_POWERBROADCAST => {
            const PBT_APMSUSPEND: usize = 0x4;
            const PBT_APMPOWERSTATUSCHANGE: usize = 0xa;
            const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
            match wparam {
                PBT_APMSUSPEND => dispatch(NotifyEvent::Suspend, 0),
                PBT_APMRESUMEAUTOMATIC => dispatch(NotifyEvent::Resume, 0),
                // Only a change of power source counts.
                PBT_APMPOWERSTATUSCHANGE => {
                    let source = if on_battery() { 2 } else { 1 };
                    if POWER_SOURCE.swap(source, Ordering::AcqRel) != source {
                        dispatch(
                            if source == 2 {
                                NotifyEvent::OnBattery
                            } else {
                                NotifyEvent::OnAc
                            },
                            0,
                        );
                    }
                }
                _ => {}
            }
            return 1;
        }
        // The lock state of this process's own session.
        WM_WTSSESSION_CHANGE if lparam as u32 == session() => {
            const WTS_SESSION_LOCK: usize = 7;
            const WTS_SESSION_UNLOCK: usize = 8;
            match wparam {
                WTS_SESSION_LOCK => dispatch(NotifyEvent::LockScreen, 0),
                WTS_SESSION_UNLOCK => dispatch(NotifyEvent::UnlockScreen, 0),
                _ => {}
            }
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => dispatch(NotifyEvent::AccentColor, wparam as u32),
        WM_SYSCOLORCHANGE => dispatch(NotifyEvent::SystemColors, 0),
        // Context menus of icons are drawn as the windows' menus are.
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
    // Explorer restarted: its notification area lost every icon.
    let restarted = TASKBAR_CREATED.load(Ordering::Acquire);
    if restarted != 0 && message == restarted {
        for (id, icon, tip) in ICONS.all() {
            let _ = notify(NIM_ADD, id, Some(icon as *mut c_void), Some(&tip));
        }
        return 0;
    }
    // SAFETY: the default procedure handles every other message.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

/// The hidden window, created on first use.
fn window() -> Result<Hwnd, u32> {
    let existing = WINDOW.load(Ordering::Acquire);
    if existing != 0 {
        return Ok(existing as Hwnd);
    }
    let class_name: Vec<u16> = "NativeBoatNotify\0".encode_utf16().collect();
    // SAFETY: the module handle is process-wide.
    let instance = unsafe { GetModuleHandleW(core::ptr::null()) };
    let class = WindowClass {
        style: 0,
        procedure: Some(procedure),
        class_extra: 0,
        window_extra: 0,
        instance,
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: core::ptr::null_mut(),
        menu_name: core::ptr::null(),
        class_name: class_name.as_ptr(),
    };
    // SAFETY: the class and its name live through the call. A second
    // registration fails harmlessly.
    unsafe { RegisterClassW(&class) };
    // A hidden top-level window, not a message-only one: context menus need
    // a window that can take the foreground.
    // SAFETY: the class is registered and the strings are null-terminated.
    let window = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0x8000_0000,
            0,
            0,
            0,
            0,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            instance,
            core::ptr::null_mut(),
        )
    };
    if window.is_null() {
        // SAFETY: GetLastError has no preconditions.
        return Err(unsafe { GetLastError() });
    }
    let name: Vec<u16> = "TaskbarCreated\0".encode_utf16().collect();
    // SAFETY: the name is null-terminated.
    TASKBAR_CREATED.store(
        unsafe { RegisterWindowMessageW(name.as_ptr()) },
        Ordering::Release,
    );
    WINDOW.store(window as usize, Ordering::Release);
    Ok(window)
}

fn copy(target: &mut [u16], text: &[u16]) {
    let text: Vec<u16> = text.iter().copied().take_while(|unit| *unit != 0).collect();
    let length = text.len().min(target.len() - 1);
    target[..length].copy_from_slice(&text[..length]);
    target[length] = 0;
}

fn data(window: Hwnd, id: u32) -> NotifyIconData {
    NotifyIconData {
        size: core::mem::size_of::<NotifyIconData>() as u32,
        window,
        id,
        flags: 0,
        callback_message: WM_NOTIFY_ICON,
        icon: core::ptr::null_mut(),
        tip: [0; 128],
        state: 0,
        state_mask: 0,
        info: [0; 256],
        timeout_or_version: 0,
        info_title: [0; 64],
        info_flags: 0,
        guid: [0; 16],
        balloon_icon: core::ptr::null_mut(),
    }
}

fn notify(
    message: u32,
    id: u32,
    icon: Option<*mut c_void>,
    tip: Option<&[u16]>,
) -> Result<(), u32> {
    let mut data = data(window()?, id);
    data.flags = NIF_MESSAGE | NIF_SHOWTIP;
    if let Some(icon) = icon {
        data.flags |= NIF_ICON;
        data.icon = icon;
    }
    if let Some(tip) = tip {
        data.flags |= NIF_TIP;
        copy(&mut data.tip, tip);
    }
    // SAFETY: `data` declares its size and lives through the call.
    if unsafe { Shell_NotifyIconW(message, &mut data) } == 0 {
        // SAFETY: GetLastError has no preconditions.
        return Err(unsafe { GetLastError() });
    }
    Ok(())
}

/// Frees an icon by its handle.
///
/// # Safety
/// The handle is an icon `Icon::from_rgba` made, no longer in use.
pub(crate) unsafe fn destroy_icon(handle: usize) {
    // SAFETY: guaranteed by the caller.
    unsafe { DestroyIcon(handle as *mut c_void) };
}

/// An icon handle owned by its creator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Icon(usize);

impl Icon {
    /// An icon from straight-alpha RGBA pixels.
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Icon> {
        if width == 0 || height == 0 || rgba.len() != (width * height * 4) as usize {
            return None;
        }
        let bgra: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
            .collect();
        let mask = alloc::vec![0u8; (width.div_ceil(16) * 2 * height) as usize];
        // SAFETY: both buffers hold the pixels the bitmaps declare.
        let (color, mask) = unsafe {
            (
                CreateBitmap(width as i32, height as i32, 1, 32, bgra.as_ptr().cast()),
                CreateBitmap(width as i32, height as i32, 1, 1, mask.as_ptr().cast()),
            )
        };
        let info = IconInfo {
            icon: 1,
            hotspot_x: 0,
            hotspot_y: 0,
            mask,
            color,
        };
        // SAFETY: the bitmaps are live; the icon copies them.
        let icon = unsafe { CreateIconIndirect(&info) };
        // SAFETY: the bitmaps were created above and are no longer needed.
        unsafe {
            DeleteObject(color);
            DeleteObject(mask);
        }
        (!icon.is_null()).then_some(Icon(icon as usize))
    }

    /// The application's own icon, from its executable.
    pub fn application() -> Option<Icon> {
        // SAFETY: resource 1 is the icon group the build embeds; a missing
        // one gives a null handle. Shared icons are never destroyed.
        let icon = unsafe {
            LoadIconW(
                GetModuleHandleW(core::ptr::null()),
                core::ptr::without_provenance::<u16>(1),
            )
        };
        (!icon.is_null()).then_some(Icon(icon as usize))
    }

    /// The icon's handle.
    pub(crate) fn raw(self) -> usize {
        self.0
    }

    /// Frees an icon made by `from_rgba`.
    pub fn destroy(self) {
        // SAFETY: the icon was created by CreateIconIndirect and is unused.
        unsafe { DestroyIcon(self.0 as *mut c_void) };
    }
}

/// Shows a notification icon with an image and a tooltip.
pub fn add_icon(id: u32, icon: Option<Icon>, tip: &[u16]) -> Result<(), u32> {
    let handle = icon.map_or(core::ptr::null_mut(), |icon| icon.0 as *mut c_void);
    notify(NIM_ADD, id, Some(handle), Some(tip))?;
    ICONS.set(id, handle as usize, tip.to_vec());
    Ok(())
}

/// Changes an icon's image.
pub fn set_icon(id: u32, icon: Option<Icon>) -> Result<(), u32> {
    let handle = icon.map_or(core::ptr::null_mut(), |icon| icon.0 as *mut c_void);
    notify(NIM_MODIFY, id, Some(handle), None)?;
    if let Some((_, tip)) = ICONS.get(id) {
        ICONS.set(id, handle as usize, tip);
    }
    Ok(())
}

/// Changes an icon's tooltip.
pub fn set_tip(id: u32, tip: &[u16]) -> Result<(), u32> {
    notify(NIM_MODIFY, id, None, Some(tip))?;
    if let Some((icon, _)) = ICONS.get(id) {
        ICONS.set(id, icon, tip.to_vec());
    }
    Ok(())
}

/// Removes an icon from the notification area.
pub fn remove_icon(id: u32) -> Result<(), u32> {
    ICONS.remove(id);
    notify(NIM_DELETE, id, None, None)
}

/// The kind of balloon, which picks its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalloonKind {
    None,
    Info,
    Warning,
    Error,
    /// The given icon.
    Custom(Icon),
}

/// Shows a balloon from an icon, which Windows 10 and later show as a
/// notification banner. An empty title and text remove the balloon.
pub fn balloon(
    id: u32,
    title: &[u16],
    text: &[u16],
    kind: BalloonKind,
    silent: bool,
    large: bool,
) -> Result<(), u32> {
    let mut data = data(window()?, id);
    data.flags = NIF_INFO;
    copy(&mut data.info_title, title);
    copy(&mut data.info, text);
    data.info_flags = match kind {
        BalloonKind::None => NIIF_NONE,
        BalloonKind::Info => NIIF_INFO,
        BalloonKind::Warning => NIIF_WARNING,
        BalloonKind::Error => NIIF_ERROR,
        BalloonKind::Custom(icon) => {
            data.balloon_icon = icon.0 as *mut c_void;
            NIIF_USER
        }
    };
    if silent {
        data.info_flags |= NIIF_NOSOUND;
    }
    if large {
        data.info_flags |= NIIF_LARGE_ICON;
    }
    // SAFETY: `data` declares its size and lives through the call.
    if unsafe { Shell_NotifyIconW(NIM_MODIFY, &mut data) } == 0 {
        // SAFETY: GetLastError has no preconditions.
        return Err(unsafe { GetLastError() });
    }
    Ok(())
}

/// An icon's screen rectangle in physical pixels: left, top, right, bottom.
pub fn icon_rect(id: u32) -> Option<(i32, i32, i32, i32)> {
    let identifier = NotifyIconIdentifier {
        size: core::mem::size_of::<NotifyIconIdentifier>() as u32,
        window: window().ok()?,
        id,
        guid: [0; 16],
    };
    let mut rect = WinRect::default();
    // SAFETY: both structures are valid for the call.
    (unsafe { Shell_NotifyIconGetRect(&identifier, &mut rect) } == 0).then_some((
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
    ))
}

/// Device pixels per device-independent pixel of the notification area.
pub fn notify_scale() -> f64 {
    window().map_or(1.0, |window| super::scale(Window(window as usize)))
}

/// The cursor position in physical pixels.
pub fn cursor() -> (i32, i32) {
    let mut point = super::frame::Point { x: 0, y: 0 };
    // SAFETY: `point` is writable.
    unsafe { GetCursorPos(&mut point) };
    (point.x, point.y)
}

/// Opens an icon's context menu at a point in device-independent pixels of
/// the screen, or at the cursor, and returns the chosen command.
pub fn icon_menu(
    items: &[NativeMenuItem],
    position: Option<(i32, i32)>,
) -> Result<Option<u16>, u32> {
    let window = window()?;
    // Windows closes an icon's menu when it loses the focus only if its
    // owner is in the foreground, and needs a message after it.
    // SAFETY: the window belongs to this thread.
    unsafe { SetForegroundWindow(window) };
    let chosen = super::popup_menu(Some(Window(window as usize)), items, position);
    // SAFETY: as above; WM_NULL does nothing.
    unsafe { PostMessageW(window, 0, 0, 0) };
    chosen
}

/// Registers a global shortcut for an Electron accelerator, which `WM_HOTKEY`
/// then reports with `id`. False when the accelerator has no key, or another
/// application holds the shortcut.
pub fn register_hotkey(id: u32, accelerator: &[u16]) -> bool {
    const MOD_ALT: u32 = 0x1;
    const MOD_CONTROL: u32 = 0x2;
    const MOD_SHIFT: u32 = 0x4;
    const MOD_WIN: u32 = 0x8;
    const MOD_NOREPEAT: u32 = 0x4000;
    // The menu accelerator parser knows no Windows key, which shortcuts take.
    let text = alloc::string::String::from_utf16_lossy(accelerator);
    let windows_key = text
        .split('+')
        .any(|part| matches!(part.trim().to_ascii_lowercase().as_str(), "super" | "meta"));
    let rest: Vec<&str> = text
        .split('+')
        .filter(|part| !matches!(part.trim().to_ascii_lowercase().as_str(), "super" | "meta"))
        .collect();
    let rest: Vec<u16> = rest.join("+").encode_utf16().collect();
    let Some((flags, key)) = super::accelerator(&rest) else {
        return false;
    };
    let mut modifiers = MOD_NOREPEAT;
    if flags & 0x04 != 0 {
        modifiers |= MOD_SHIFT;
    }
    if flags & 0x08 != 0 {
        modifiers |= MOD_CONTROL;
    }
    if flags & 0x10 != 0 {
        modifiers |= MOD_ALT;
    }
    if windows_key {
        modifiers |= MOD_WIN;
    }
    let Ok(window) = window() else {
        return false;
    };
    // SAFETY: the window belongs to this thread.
    unsafe { RegisterHotKey(window, id as i32, modifiers, u32::from(key)) != 0 }
}

/// Releases a global shortcut.
pub fn unregister_hotkey(id: u32) {
    if let Ok(window) = window() {
        // SAFETY: the window belongs to this thread.
        unsafe { UnregisterHotKey(window, id as i32) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn SendMessageW(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
    }

    static SEEN: std::sync::Mutex<Vec<(NotifyEvent, u32)>> = std::sync::Mutex::new(Vec::new());

    unsafe extern "system" fn record(_: *mut c_void, event: NotifyEvent, id: u32) {
        SEEN.lock().unwrap().push((event, id));
    }

    #[test]
    fn icon_callbacks_and_hotkeys_reach_the_handler() {
        let _guard = super::super::lock_host_tests();
        set_notify_handler(core::ptr::null_mut(), Some(record));
        let window = window().unwrap();
        for (lparam, wparam) in [
            (0x0202, 7),
            (0x0205, 7),
            (0x0203, 7),
            (0x0405, 8),
            (0x0404, 8),
            (0x0201, 7),
        ] {
            // SAFETY: the window belongs to this thread.
            unsafe { SendMessageW(window, WM_NOTIFY_ICON, wparam, lparam) };
        }
        // SAFETY: as above.
        unsafe { SendMessageW(window, WM_HOTKEY, 9, 0) };
        set_notify_handler(core::ptr::null_mut(), None);
        assert_eq!(
            *SEEN.lock().unwrap(),
            [
                (NotifyEvent::Click, 7),
                (NotifyEvent::RightClick, 7),
                (NotifyEvent::DoubleClick, 7),
                (NotifyEvent::BalloonClick, 8),
                (NotifyEvent::BalloonClosed, 8),
                (NotifyEvent::Hotkey, 9),
            ]
        );
    }

    #[test]
    fn icons_come_from_pixels_and_shortcuts_need_a_key() {
        let icon = Icon::from_rgba(2, 2, &[255; 16]).unwrap();
        icon.destroy();
        assert!(Icon::from_rgba(2, 2, &[255; 15]).is_none());
        let none: Vec<u16> = "Ctrl+Alt".encode_utf16().collect();
        assert!(!register_hotkey(1, &none));
    }
}
