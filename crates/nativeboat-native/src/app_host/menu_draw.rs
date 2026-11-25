//! Popup menus drawn as Chromium draws Electron's menus on Windows: white,
//! with 28-pixel rows, 12 pixels above the first row and below the last,
//! labels 20 pixels in, shortcuts and submenu arrows 20 pixels from the
//! right, check and radio marks in a column of their own, and light blue
//! separators. Lengths are device-independent pixels of the owner window.

use super::{GdiObject, Hdc, Hwnd, NativeMenuItem, WinRect};
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

#[repr(C)]
struct LogFont {
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u8,
    underline: u8,
    strike_out: u8,
    char_set: u8,
    output_precision: u8,
    clip_precision: u8,
    quality: u8,
    pitch_and_family: u8,
    face: [u16; 32],
}

#[repr(C)]
struct NonClientMetrics {
    size: u32,
    border_width: i32,
    scroll_width: i32,
    scroll_height: i32,
    caption_width: i32,
    caption_height: i32,
    caption_font: LogFont,
    small_caption_width: i32,
    small_caption_height: i32,
    small_caption_font: LogFont,
    menu_width: i32,
    menu_height: i32,
    menu_font: LogFont,
    status_font: LogFont,
    message_font: LogFont,
    padded_border_width: i32,
}

#[repr(C)]
struct MenuInfo {
    size: u32,
    mask: u32,
    style: u32,
    max_height: u32,
    background: GdiObject,
    help: u32,
    data: usize,
}

#[repr(C)]
pub(super) struct MeasureItem {
    kind: u32,
    control: u32,
    item: u32,
    width: u32,
    height: u32,
    data: usize,
}

#[repr(C)]
pub(super) struct DrawItem {
    kind: u32,
    control: u32,
    item: u32,
    action: u32,
    state: u32,
    window: Hwnd,
    dc: Hdc,
    rect: WinRect,
    data: usize,
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetDpiForWindow(window: Hwnd) -> u32;
    fn GetSystemMetricsForDpi(index: i32, dpi: u32) -> i32;
    fn SystemParametersInfoForDpi(
        action: u32,
        parameter: u32,
        value: *mut c_void,
        update: u32,
        dpi: u32,
    ) -> i32;
    fn SetMenuInfo(menu: *mut c_void, info: *const MenuInfo) -> i32;
    fn DrawTextW(dc: Hdc, text: *const u16, length: i32, rect: *mut WinRect, format: u32) -> i32;
    fn GetMenuItemCount(menu: *mut c_void) -> i32;
}

#[repr(C)]
struct Message {
    window: Hwnd,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    point: (i32, i32),
}

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetWindowsHookExW(id: i32, procedure: HookProc, module: *mut c_void, thread: u32) -> isize;
    fn UnhookWindowsHookEx(hook: isize) -> i32;
    fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

const WH_MSGFILTER: i32 = -1;
const MSGF_MENU: i32 = 2;
const WM_KEYDOWN: u32 = 0x0100;
const VK_UP: u32 = 0x26;
const VK_DOWN: u32 = 0x28;
const MF_DISABLED: usize = 0x0002;
const MF_MOUSESELECT: usize = 0x8000;

/// The menu loop's message hook, the arrow key that moved the selection
/// last (0 after other input) and the disabled rows passed over since.
static HOOK: AtomicUsize = AtomicUsize::new(0);
static ARROW: AtomicU32 = AtomicU32::new(0);
static PASSED: AtomicU32 = AtomicU32::new(0);

/// Notes which arrow key, if any, the menu loop handles next.
unsafe extern "system" fn menu_input(code: i32, wparam: usize, lparam: isize) -> isize {
    if code == MSGF_MENU && lparam != 0 {
        // SAFETY: a message filter hook receives the message being handled.
        let message = unsafe { &*(lparam as *const Message) };
        match message.message {
            WM_KEYDOWN => {
                let key = message.wparam as u32;
                ARROW.store(
                    if matches!(key, VK_UP | VK_DOWN) {
                        key
                    } else {
                        0
                    },
                    Ordering::Relaxed,
                );
            }
            // Mouse input selects where the pointer is.
            0x0200..=0x020E => ARROW.store(0, Ordering::Relaxed),
            _ => {}
        }
    }
    // SAFETY: passes the message on to the thread's other hooks.
    unsafe { CallNextHookEx(HOOK.load(Ordering::Relaxed) as isize, code, wparam, lparam) }
}

/// Answers `WM_MENUSELECT`: an arrow key that selected a disabled row moves
/// on past it, as Chromium's menus pass over disabled items.
pub(super) fn menu_select(owner: Hwnd, wparam: usize, lparam: isize) {
    if active().is_none() || lparam == 0 {
        return;
    }
    let flags = wparam >> 16 & 0xffff;
    let arrow = ARROW.load(Ordering::Relaxed);
    if arrow == 0
        || flags & MF_MOUSESELECT != 0
        || flags & (super::MF_GRAYED as usize | MF_DISABLED) == 0
    {
        PASSED.store(0, Ordering::Relaxed);
        return;
    }
    // SAFETY: the menu handle is live while it is shown.
    let rows = unsafe { GetMenuItemCount(lparam as *mut c_void) }.max(0) as u32;
    // A menu of only disabled rows stops somewhere.
    if PASSED.fetch_add(1, Ordering::Relaxed) >= rows {
        return;
    }
    // SAFETY: posts a key to the owner, whose menu loop takes it.
    unsafe { super::PostMessageW(owner, WM_KEYDOWN, arrow as usize, 0) };
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFontIndirectW(font: *const LogFont) -> GdiObject;
    fn ExcludeClipRect(dc: Hdc, left: i32, top: i32, right: i32, bottom: i32) -> i32;
}

#[repr(C)]
struct StartupInput {
    version: u32,
    callback: usize,
    suppress_thread: i32,
    suppress_codecs: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PointF {
    x: f32,
    y: f32,
}

#[link(name = "gdiplus", kind = "raw-dylib")]
unsafe extern "system" {
    fn GdiplusStartup(token: *mut usize, input: *const StartupInput, output: *mut c_void) -> i32;
    fn GdiplusShutdown(token: usize);
    fn GdipCreateFromHDC(dc: Hdc, graphics: *mut *mut c_void) -> i32;
    fn GdipDeleteGraphics(graphics: *mut c_void) -> i32;
    fn GdipSetSmoothingMode(graphics: *mut c_void, mode: i32) -> i32;
    fn GdipSetPixelOffsetMode(graphics: *mut c_void, mode: i32) -> i32;
    fn GdipCreatePen1(color: u32, width: f32, unit: i32, pen: *mut *mut c_void) -> i32;
    fn GdipDeletePen(pen: *mut c_void) -> i32;
    fn GdipDrawLines(
        graphics: *mut c_void,
        pen: *mut c_void,
        points: *const PointF,
        count: i32,
    ) -> i32;
    fn GdipDrawEllipse(
        graphics: *mut c_void,
        pen: *mut c_void,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> i32;
    fn GdipCreateSolidFill(color: u32, brush: *mut *mut c_void) -> i32;
    fn GdipFillEllipse(
        graphics: *mut c_void,
        brush: *mut c_void,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> i32;
    fn GdipDeleteBrush(brush: *mut c_void) -> i32;
}

const SMOOTHING_ANTI_ALIAS: i32 = 4;
const PIXEL_OFFSET_HALF: i32 = 4;
const UNIT_PIXEL: i32 = 2;

/// Chromium's menu icons: Material Design's 24-unit check, radio buttons
/// and chevron, drawn in a 16-pixel box.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Icon {
    Check,
    RadioOn,
    RadioOff,
    Arrow,
}

impl Icon {
    /// The icon as text, when GDI+ is missing.
    fn text(self) -> u16 {
        match self {
            Icon::Check => 0x2713,
            Icon::RadioOn => 0x25C9,
            Icon::RadioOff => 0x25CB,
            Icon::Arrow => 0x203A,
        }
    }

    /// The center line of a stroked icon, in the icon's 24 units; the
    /// strokes are 2 units wide.
    fn line(self) -> &'static [(f32, f32)] {
        match self {
            Icon::Check => &[(4.12, 12.71), (9.0, 17.59), (20.3, 6.3)],
            Icon::Arrow => &[(9.3, 6.71), (14.59, 12.0), (9.3, 17.29)],
            Icon::RadioOn | Icon::RadioOff => &[],
        }
    }
}

/// Draws an icon with GDI+ in a square box of a device context.
fn draw_icon(dc: Hdc, icon: Icon, left: i32, top: i32, size: i32, color: u32) {
    let argb = 0xff00_0000 | (color & 0xff) << 16 | (color & 0xff00) | (color >> 16 & 0xff);
    let unit = size as f32 / 24.0;
    let at = |x: f32, y: f32| PointF {
        x: left as f32 + x * unit,
        y: top as f32 + y * unit,
    };
    // SAFETY: the graphics, pen and brush are created and deleted here,
    // drawing on the live menu DC.
    unsafe {
        let mut graphics = core::ptr::null_mut();
        if GdipCreateFromHDC(dc, &mut graphics) != 0 {
            return;
        }
        GdipSetSmoothingMode(graphics, SMOOTHING_ANTI_ALIAS);
        GdipSetPixelOffsetMode(graphics, PIXEL_OFFSET_HALF);
        let mut pen = core::ptr::null_mut();
        if GdipCreatePen1(argb, 2.0 * unit, UNIT_PIXEL, &mut pen) == 0 {
            let line: Vec<PointF> = icon.line().iter().map(|(x, y)| at(*x, *y)).collect();
            if line.is_empty() {
                // The ring's center line has a radius of 9 units.
                let corner = at(3.0, 3.0);
                GdipDrawEllipse(graphics, pen, corner.x, corner.y, 18.0 * unit, 18.0 * unit);
            } else {
                GdipDrawLines(graphics, pen, line.as_ptr(), line.len() as i32);
            }
            GdipDeletePen(pen);
        }
        let mut brush = core::ptr::null_mut();
        if icon == Icon::RadioOn && GdipCreateSolidFill(argb, &mut brush) == 0 {
            let corner = at(7.0, 7.0);
            GdipFillEllipse(
                graphics,
                brush,
                corner.x,
                corner.y,
                10.0 * unit,
                10.0 * unit,
            );
            GdipDeleteBrush(brush);
        }
        GdipDeleteGraphics(graphics);
    }
}

const ODT_MENU: u32 = 1;
const ODS_SELECTED: u32 = 0x0001;
const ODS_GRAYED: u32 = 0x0002;
const ODS_DISABLED: u32 = 0x0004;
const MF_OWNERDRAW: u32 = 0x0100;
const MIM_STYLE: u32 = 0x0010;
const MIM_BACKGROUND: u32 = 0x0002;
const MNS_NOCHECK: u32 = 0x8000_0000;
const SPI_GETNONCLIENTMETRICS: u32 = 0x0029;
const DT_RIGHT: u32 = 0x0002;
const DT_CENTER: u32 = 0x0001;
const DT_VCENTER: u32 = 0x0004;
const DT_SINGLELINE: u32 = 0x0020;
const DT_NOCLIP: u32 = 0x0100;
const DT_CALCRECT: u32 = 0x0400;
const DT_NOPREFIX: u32 = 0x0800;
const DT_HIDEPREFIX: u32 = 0x0010_0000;
const TRANSPARENT: i32 = 1;
const MNC_EXECUTE: isize = 2;
const MNC_SELECT: isize = 3;

/// Chromium's menu colors, as GDI `COLORREF`s.
const BACKGROUND: u32 = rgb(0xff, 0xff, 0xff);
/// The menu's background as red, green and blue, for its frame.
pub(super) const BACKGROUND_RGB: [u8; 3] = [0xff, 0xff, 0xff];
const LABEL: u32 = rgb(0x1f, 0x1f, 0x1f);
const SECONDARY: u32 = rgb(0x47, 0x47, 0x47);
const DISABLED: u32 = rgb(0x5f, 0x63, 0x68);
const HOVER: u32 = rgb(0xf2, 0xf2, 0xf2);
const SEPARATOR: u32 = rgb(0xd3, 0xe3, 0xfd);
const RADIO: u32 = rgb(0x0b, 0x57, 0xd0);

const fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | (green as u32) << 8 | (blue as u32) << 16
}

/// Chromium's menu metrics in device-independent pixels.
const ROW: f64 = 28.0;
const END_PADDING: f64 = 12.0;
const SEPARATOR_HEIGHT: f64 = 17.0;
const SEPARATOR_GAP: f64 = 8.0;
const SIDE_PADDING: f64 = 20.0;
const ICON: f64 = 16.0;
const ICON_GAP: f64 = 12.0;
const SHORTCUT_GAP: f64 = 10.0;
const ARROW_GAP: f64 = 16.0;
const MINIMUM_WIDTH: f64 = 122.0;

/// The widest label and shortcut of a menu and which columns it has, in
/// device pixels, from which every row takes the menu's width.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct Columns {
    pub label: i32,
    pub shortcut: i32,
    pub marks: bool,
    pub arrows: bool,
}

/// Device pixels for device-independent ones.
fn px(scale: f64, dip: f64) -> i32 {
    // Lengths are positive: adding a half rounds them.
    (dip * scale + 0.5) as i32
}

impl Columns {
    /// The width of the menu's rows. Shortcuts and submenu arrows end at
    /// the same right edge, so the wider of their columns is kept, as in
    /// Chromium.
    pub(super) fn width(&self, scale: f64) -> i32 {
        let mut width = px(scale, SIDE_PADDING) * 2 + self.label;
        if self.marks {
            width += px(scale, ICON + ICON_GAP);
        }
        let shortcuts = if self.shortcut > 0 {
            px(scale, SHORTCUT_GAP) + self.shortcut
        } else {
            0
        };
        let arrows = if self.arrows {
            px(scale, ARROW_GAP + ICON)
        } else {
            0
        };
        width += shortcuts.max(arrows);
        width.max(px(scale, MINIMUM_WIDTH))
    }
}

/// The height of a row: separators are thinner, and the first and last rows
/// hold the menu's end padding.
pub(super) fn row_height(scale: f64, separator: bool, first: bool, last: bool) -> i32 {
    let mut height = px(scale, if separator { SEPARATOR_HEIGHT } else { ROW });
    if first {
        height += px(scale, END_PADDING);
    }
    if last {
        height += px(scale, END_PADDING);
    }
    height
}

/// The character after a label's single `&`, lowercased, as Windows menus
/// mark access keys.
pub(super) fn access_key(label: &[u16]) -> Option<u16> {
    let mut units = label.iter().copied().take_while(|unit| *unit != 0);
    while let Some(unit) = units.next() {
        if unit == u16::from(b'&') {
            match units.next() {
                Some(next) if next == u16::from(b'&') => {}
                Some(next) => return Some(lower(next)),
                None => return None,
            }
        }
    }
    None
}

fn lower(unit: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
        unit + 32
    } else {
        unit
    }
}

/// The rows a typed character picks, as Chromium's menus pick them: the
/// rows whose access key it is, else the rows whose label starts with it.
pub(super) fn rows_for_key(labels: &[(&[u16], bool)], key: u16) -> Vec<usize> {
    let key = lower(key);
    let enabled = |(index, (_, enabled)): (usize, &(&[u16], bool))| enabled.then_some(index);
    let keyed: Vec<usize> = labels
        .iter()
        .enumerate()
        .filter(|(_, (label, _))| access_key(label) == Some(key))
        .filter_map(enabled)
        .collect();
    if !keyed.is_empty() {
        return keyed;
    }
    labels
        .iter()
        .enumerate()
        .filter(|(_, (label, _))| {
            label
                .iter()
                .copied()
                .find(|unit| *unit != u16::from(b'&'))
                .is_some_and(|unit| unit != 0 && lower(unit) == key)
        })
        .filter_map(enabled)
        .collect()
}

/// A drawn row's data.
struct Row {
    label: Vec<u16>,
    shortcut: Vec<u16>,
    separator: bool,
    submenu: bool,
    enabled: bool,
    checked: bool,
    radio: bool,
    first: bool,
    last: bool,
    menu: usize,
}

/// A drawn menu of the popup: its handle, columns and rows.
struct Menu {
    handle: *mut c_void,
    columns: Columns,
    rows: Vec<usize>,
}

/// The popup being tracked: fonts, scale and the rows its menus draw.
pub(super) struct Popup {
    scale: f64,
    /// The width Windows adds to drawn rows for a check mark.
    check: i32,
    font: GdiObject,
    /// The GDI+ session that draws marks and arrows as Chromium's vector
    /// icons; without one they are text.
    gdiplus: usize,
    background: GdiObject,
    rows: Vec<Row>,
    menus: Vec<Menu>,
}

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

impl Popup {
    /// Prepares drawing for a popup owned by a window.
    pub(super) fn new(owner: Hwnd) -> Self {
        // SAFETY: the owner is a live window of this thread.
        let dpi = unsafe { GetDpiForWindow(owner) }.max(96);
        let scale = f64::from(dpi) / 96.0;
        const SM_CXMENUCHECK: i32 = 71;
        // SAFETY: reads a system metric for the owner's DPI.
        let check = unsafe { GetSystemMetricsForDpi(SM_CXMENUCHECK, dpi) };
        // SAFETY: an all-zero NONCLIENTMETRICSW is a valid buffer.
        let mut metrics: NonClientMetrics = unsafe { core::mem::zeroed() };
        metrics.size = core::mem::size_of::<NonClientMetrics>() as u32;
        // SAFETY: the buffer is sized as SPI_GETNONCLIENTMETRICS writes it.
        let found = unsafe {
            SystemParametersInfoForDpi(
                SPI_GETNONCLIENTMETRICS,
                metrics.size,
                (&raw mut metrics).cast(),
                0,
                dpi,
            )
        };
        // Chromium's menus use the default UI font, Windows' message font.
        let font = if found != 0 {
            // SAFETY: the font description was filled in above.
            unsafe { CreateFontIndirectW(&metrics.message_font) }
        } else {
            core::ptr::null_mut()
        };
        let mut gdiplus = 0;
        let input = StartupInput {
            version: 1,
            callback: 0,
            suppress_thread: 0,
            suppress_codecs: 1,
        };
        // SAFETY: the startup description is complete; the session ends
        // when the popup is dropped.
        if unsafe { GdiplusStartup(&mut gdiplus, &input, core::ptr::null_mut()) } != 0 {
            gdiplus = 0;
        }
        // SAFETY: CreateSolidBrush has no pointer preconditions.
        let background = unsafe { super::CreateSolidBrush(BACKGROUND) };
        Self {
            scale,
            check,
            font,
            gdiplus,
            background,
            rows: Vec::new(),
            menus: Vec::new(),
        }
    }

    /// Appends drawn rows for menu items to a menu.
    pub(super) fn append(
        &mut self,
        owner: Hwnd,
        menu: *mut c_void,
        items: &[NativeMenuItem],
    ) -> Result<(), u32> {
        let index = self.menus.len();
        self.menus.push(Menu {
            handle: menu,
            columns: Columns::default(),
            rows: Vec::new(),
        });
        let mut columns = Columns::default();
        let count = items.len();
        for (position, item) in items.iter().enumerate() {
            let label: Vec<u16> = item
                .label
                .iter()
                .copied()
                .take_while(|unit| *unit != 0)
                .collect();
            let shortcut = item.shortcut.clone().unwrap_or_default();
            if !item.separator {
                columns.label = columns.label.max(self.text_width(owner, &label));
                if !item.submenu {
                    columns.shortcut = columns.shortcut.max(self.text_width(owner, &shortcut));
                }
                columns.marks |= item.checked || item.radio;
                columns.arrows |= item.submenu;
            }
            // A row's item data is its number, counted from 1.
            self.rows.push(Row {
                label,
                shortcut,
                separator: item.separator,
                submenu: item.submenu,
                enabled: item.enabled,
                checked: item.checked,
                radio: item.radio,
                first: position == 0,
                last: position + 1 == count,
                menu: index,
            });
            let data = self.rows.len();
            self.menus[index].rows.push(data);
            let mut flags = MF_OWNERDRAW | if item.enabled { 0 } else { super::MF_GRAYED };
            let handle = if item.submenu {
                // SAFETY: CreatePopupMenu has no pointer preconditions.
                let child = unsafe { super::CreatePopupMenu() };
                if child.is_null() {
                    // SAFETY: GetLastError has no preconditions.
                    return Err(unsafe { super::GetLastError() });
                }
                if let Err(error) = self.append(owner, child, &item.children) {
                    // SAFETY: the child menu was created above and is not attached.
                    unsafe { super::DestroyMenu(child) };
                    return Err(error);
                }
                flags |= super::MF_POPUP;
                child as usize
            } else if item.separator {
                // A drawn separator is still one, so keys pass over it.
                flags |= super::MF_SEPARATOR;
                0
            } else {
                usize::from(item.id)
            };
            // SAFETY: the menu is live, and an owner-drawn item's "text" is
            // its item data, the row's number.
            let appended = unsafe { super::AppendMenuW(menu, flags, handle, data as *const u16) };
            if appended == 0 {
                if item.submenu {
                    // SAFETY: AppendMenuW did not attach the child on failure.
                    unsafe { super::DestroyMenu(handle as *mut c_void) };
                }
                // SAFETY: GetLastError has no preconditions.
                return Err(unsafe { super::GetLastError() });
            }
        }
        self.menus[index].columns = columns;
        let info = MenuInfo {
            size: core::mem::size_of::<MenuInfo>() as u32,
            mask: MIM_STYLE | MIM_BACKGROUND,
            style: MNS_NOCHECK,
            max_height: 0,
            background: self.background,
            help: 0,
            data: 0,
        };
        // SAFETY: the menu is live and the description is complete; the
        // brush outlives the popup.
        unsafe { SetMenuInfo(menu, &info) };
        Ok(())
    }

    fn text_width(&self, owner: Hwnd, text: &[u16]) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let mut rect = WinRect::default();
        // SAFETY: the window's DC is released below; the font is live.
        unsafe {
            let dc = super::GetDC(owner);
            if dc.is_null() {
                return 0;
            }
            let previous = super::SelectObject(dc, self.font);
            DrawTextW(
                dc,
                text.as_ptr(),
                text.len() as i32,
                &mut rect,
                DT_CALCRECT | DT_SINGLELINE | DT_HIDEPREFIX,
            );
            super::SelectObject(dc, previous);
            super::ReleaseDC(owner, dc);
        }
        rect.right - rect.left
    }

    /// Makes this popup the one the owner window's messages draw.
    pub(super) fn activate(&self) {
        ACTIVE.store(core::ptr::from_ref(self) as usize, Ordering::Release);
        ARROW.store(0, Ordering::Relaxed);
        PASSED.store(0, Ordering::Relaxed);
        // SAFETY: hooks only this thread's menu loop, until the popup drops.
        let hook = unsafe {
            SetWindowsHookExW(
                WH_MSGFILTER,
                menu_input,
                core::ptr::null_mut(),
                GetCurrentThreadId(),
            )
        };
        HOOK.store(hook as usize, Ordering::Relaxed);
        super::menu_frame::begin();
    }

    fn row(&self, data: usize) -> Option<&Row> {
        self.rows.get(data.checked_sub(1)?)
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        if ACTIVE.load(Ordering::Acquire) == core::ptr::from_ref(self) as usize {
            ACTIVE.store(0, Ordering::Release);
            let hook = HOOK.swap(0, Ordering::Relaxed);
            if hook != 0 {
                // SAFETY: the hook was installed by this popup's activation.
                unsafe { UnhookWindowsHookEx(hook as isize) };
            }
            super::menu_frame::end();
        }
        if self.gdiplus != 0 {
            // SAFETY: the session was started for this popup, whose drawing
            // has ended.
            unsafe { GdiplusShutdown(self.gdiplus) };
        }
        for object in [self.font, self.background] {
            if !object.is_null() {
                // SAFETY: these GDI objects were created for this popup and
                // its menus are no longer shown.
                unsafe { super::DeleteObject(object) };
            }
        }
    }
}

fn active() -> Option<&'static Popup> {
    let popup = ACTIVE.load(Ordering::Acquire);
    // SAFETY: the active popup lives on the stack of the call tracking it,
    // which clears it before returning, and messages arrive on its thread.
    (popup != 0).then(|| unsafe { &*(popup as *const Popup) })
}

/// Answers `WM_MEASUREITEM` for the active popup's rows.
pub(super) fn measure(lparam: isize) -> bool {
    // SAFETY: WM_MEASUREITEM passes a MEASUREITEMSTRUCT.
    let item = unsafe { &mut *(lparam as *mut MeasureItem) };
    let Some(popup) = active() else {
        return false;
    };
    if item.kind != ODT_MENU {
        return false;
    }
    let Some(row) = popup.row(item.data) else {
        return false;
    };
    let columns = popup.menus[row.menu].columns;
    // Windows widens drawn rows by a check mark's width, less a pixel.
    item.width = (columns.width(popup.scale) - (popup.check - 1)).max(0) as u32;
    item.height = row_height(popup.scale, row.separator, row.first, row.last) as u32;
    true
}

/// Answers `WM_DRAWITEM` for the active popup's rows.
pub(super) fn draw(lparam: isize) -> bool {
    // SAFETY: WM_DRAWITEM passes a DRAWITEMSTRUCT.
    let item = unsafe { &*(lparam as *const DrawItem) };
    let Some(popup) = active() else {
        return false;
    };
    if item.kind != ODT_MENU {
        return false;
    }
    let Some(row) = popup.row(item.data) else {
        return false;
    };
    let scale = popup.scale;
    let columns = popup.menus[row.menu].columns;
    let dc = item.dc;
    let rect = &item.rect;
    let fill = |left: i32, top: i32, right: i32, bottom: i32, color: u32| {
        let area = WinRect {
            left,
            top,
            right,
            bottom,
        };
        // SAFETY: the brush is deleted right after filling the menu's DC.
        unsafe {
            let brush = super::CreateSolidBrush(color);
            super::FillRect(dc, &area, brush);
            super::DeleteObject(brush);
        }
    };
    fill(rect.left, rect.top, rect.right, rect.bottom, BACKGROUND);
    let top = rect.top + if row.first { px(scale, END_PADDING) } else { 0 };
    let bottom = rect.bottom - if row.last { px(scale, END_PADDING) } else { 0 };
    if row.separator {
        // A line across the whole menu, 8 pixels below the row above.
        let y = top + px(scale, SEPARATOR_GAP);
        fill(
            rect.left,
            y,
            rect.right,
            y + px(scale, 1.0).max(1),
            SEPARATOR,
        );
        return true;
    }
    let enabled = row.enabled && item.state & (ODS_GRAYED | ODS_DISABLED) == 0;
    if enabled && item.state & ODS_SELECTED != 0 {
        fill(rect.left, top, rect.right, bottom, HOVER);
    }
    let text = |font: GdiObject, color: u32, units: &[u16], left: i32, right: i32, format: u32| {
        let mut area = WinRect {
            left,
            top,
            right,
            bottom,
        };
        // SAFETY: the font is live and the text is a valid UTF-16 slice.
        unsafe {
            let previous = super::SelectObject(dc, font);
            super::SetTextColor(dc, color);
            super::SetBkMode(dc, TRANSPARENT);
            DrawTextW(
                dc,
                units.as_ptr(),
                units.len() as i32,
                &mut area,
                format | DT_SINGLELINE | DT_VCENTER | DT_NOCLIP,
            );
            super::SelectObject(dc, previous);
        }
    };
    let mut left = rect.left + px(scale, SIDE_PADDING);
    let right = rect.right - px(scale, SIDE_PADDING);
    let icon = px(scale, ICON);
    let mark = |kind: Icon, color: u32, left: i32| {
        if popup.gdiplus != 0 {
            let middle = (top + bottom) / 2;
            draw_icon(dc, kind, left, middle - icon / 2, icon, color);
            return;
        }
        let (font, unit) = (popup.font, kind.text());
        text(
            font,
            color,
            &[unit],
            left,
            left + icon,
            DT_CENTER | DT_NOPREFIX,
        );
    };
    if columns.marks {
        let color = if enabled { SECONDARY } else { DISABLED };
        match (row.radio, row.checked) {
            // The radio button: a ring, with a dot when chosen.
            (true, checked) => {
                let color = if enabled { RADIO } else { DISABLED };
                if checked {
                    mark(Icon::RadioOn, color, left);
                } else {
                    mark(Icon::RadioOff, color, left);
                }
            }
            (false, true) => mark(Icon::Check, color, left),
            (false, false) => {}
        }
        left += px(scale, ICON + ICON_GAP);
    }
    let label = if enabled { LABEL } else { DISABLED };
    text(popup.font, label, &row.label, left, right, DT_HIDEPREFIX);
    if row.submenu {
        let color = if enabled { SECONDARY } else { DISABLED };
        mark(Icon::Arrow, color, right - icon);
    } else if !row.shortcut.is_empty() {
        let color = if enabled { SECONDARY } else { DISABLED };
        text(
            popup.font,
            color,
            &row.shortcut,
            left,
            right,
            DT_RIGHT | DT_NOPREFIX,
        );
    }
    // Keeps Windows from drawing its own submenu arrow over the row.
    // SAFETY: clips only the menu's DC for the rest of this paint.
    unsafe { ExcludeClipRect(dc, rect.left, rect.top, rect.right, rect.bottom) };
    true
}

/// Answers `WM_MENUCHAR` for the active popup: a typed character runs the
/// only row it picks, or selects the first of several.
pub(super) fn menu_char(wparam: usize, lparam: isize) -> Option<isize> {
    let popup = active()?;
    let menu = popup
        .menus
        .iter()
        .find(|menu| menu.handle as isize == lparam)?;
    let rows: Vec<&Row> = menu
        .rows
        .iter()
        .filter_map(|data| popup.row(*data))
        .collect();
    let labels: Vec<(&[u16], bool)> = rows
        .iter()
        .map(|row| (row.label.as_slice(), row.enabled && !row.separator))
        .collect();
    // SAFETY: the menu handle is live while it is shown.
    let count = unsafe { GetMenuItemCount(menu.handle) };
    let picked = rows_for_key(&labels, (wparam & 0xffff) as u16);
    let index = *picked.first()? as isize;
    if index >= count as isize {
        return None;
    }
    let action = if picked.len() == 1 {
        MNC_EXECUTE
    } else {
        MNC_SELECT
    };
    Some(index | action << 16)
}

#[cfg(test)]
#[path = "menu_draw/tests.rs"]
mod tests;
