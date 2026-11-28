//! Native controls: creation, state updates and notifications.

use crate::datetime;
use crate::event_loop::dispatch;
#[allow(unused_imports)]
use crate::prelude::*;
use crate::render::{colorref, paint_area};
use crate::wide::wide;
use crate::window::{Role, STATE, instance, key, place, px, surface_hwnd, window_text};
use crate::{ControlEvent, Event, WindowId, fonts};
use core::cell::{Cell, RefCell};
use core::ptr::{null, null_mut};
use nativeboat_css::Color;
use nativeboat_ir::{Control, ControlKind, DisplayItem, NativeChild};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use windows_sys::core::{PCWSTR, w};

/// `TBM_GETPOS`, which windows-sys does not define.
const TBM_GETPOS: u32 = WM_USER;
/// Height of a combo box's drop-down list.
const DROPDOWN: f32 = 200.0;

pub(crate) mod proxy;

pub(crate) struct ControlData {
    pub hwnd: HWND,
    pub surface: u64,
    pub control: Control,
    composing: bool,
}

local! {
    /// Set while the backend changes a control, to ignore the notifications
    /// that causes.
    static UPDATING: Cell<bool> = const { Cell::new(false) };
    /// The control whose native print procedure is running. Windows can
    /// send WM_PRINTCLIENT through its subclass during WM_PRINT.
    static PRINTING: Cell<usize> = const { Cell::new(0) };
    static BRUSHES: RefCell<HashMap<COLORREF, HBRUSH>> = RefCell::new(HashMap::new());
}

/// The height added below a control's box for its drop-down list.
/// Where a control's window goes in its surface. A single-line field shows
/// its text in the middle of its line, as Chromium centres the text of a
/// taller line; an edit control draws it at its top, so the control covers
/// just the text, centred.
pub(crate) fn window_rect(control: &Control, scale: f32) -> nativeboat_ir::Rect {
    let mut rect = control.rect;
    let state = &control.state;
    if state.bare && matches!(control.kind, ControlKind::TextBox | ControlKind::Password) {
        let height = fonts::cell_height(&state.font, scale);
        if rect.height > height {
            rect.y += (rect.height - height) / 2.0;
            rect.height = height;
        }
    }
    rect
}

pub(crate) fn extra_height(kind: ControlKind) -> f32 {
    if kind == ControlKind::ComboBox {
        DROPDOWN
    } else {
        0.0
    }
}

/// Returns the native handle of a control owned by this window.
pub fn hwnd(window: WindowId, id: u64) -> Option<HWND> {
    STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)?
            .controls
            .get(&id)
            .map(|data| data.hwnd)
    })
}

pub(crate) fn composing(handle: HWND) -> Option<bool> {
    STATE.with_borrow(|state| {
        let (window, Role::Control(id)) = state.handles.get(&(handle as usize))? else {
            return None;
        };
        Some(state.windows.get(window)?.controls.get(id)?.composing)
    })
}

/// The extended style of a control: text fields have a sunken edge unless
/// the page draws their box.
fn is_picker(kind: ControlKind) -> bool {
    matches!(
        kind,
        ControlKind::Date | ControlKind::Time | ControlKind::DateTime | ControlKind::Month
    )
}

fn is_field(kind: ControlKind) -> bool {
    is_picker(kind)
        || matches!(
            kind,
            ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea
        )
}

fn extended_style(control: &Control) -> u32 {
    if is_field(control.kind) && !control.state.bare {
        WS_EX_CLIENTEDGE
    } else {
        0
    }
}

/// The edit control style that aligns text as a field's `text-align` does.
fn alignment(control: &Control) -> u32 {
    use nativeboat_css::types::TextAlign;
    match control.state.text_align {
        TextAlign::Center => ES_CENTER as u32,
        TextAlign::Right | TextAlign::End => ES_RIGHT as u32,
        _ => ES_LEFT as u32,
    }
}

fn class_of(control: &Control) -> (PCWSTR, u32, u32) {
    let edit = (ES_AUTOHSCROLL as u32) | WS_TABSTOP | alignment(control);
    let extended = extended_style(control);
    match control.kind {
        ControlKind::Button => (w!("BUTTON"), BS_PUSHBUTTON as u32 | WS_TABSTOP, 0),
        ControlKind::CheckBox => (w!("BUTTON"), BS_3STATE as u32 | WS_TABSTOP, 0),
        ControlKind::Radio => (w!("BUTTON"), BS_RADIOBUTTON as u32 | WS_TABSTOP, 0),
        ControlKind::TextBox => (w!("EDIT"), edit, extended),
        ControlKind::Password => (w!("EDIT"), edit | ES_PASSWORD as u32, extended),
        ControlKind::TextArea => (
            w!("EDIT"),
            (ES_MULTILINE | ES_WANTRETURN | ES_AUTOVSCROLL) as u32 | WS_VSCROLL | WS_TABSTOP,
            extended,
        ),
        ControlKind::ComboBox => (
            w!("COMBOBOX"),
            CBS_DROPDOWNLIST as u32 | WS_VSCROLL | WS_TABSTOP,
            0,
        ),
        ControlKind::ListBox => {
            let multiple = if control.state.multiple {
                LBS_EXTENDEDSEL as u32
            } else {
                0
            };
            (
                w!("LISTBOX"),
                (LBS_NOTIFY | LBS_NOINTEGRALHEIGHT) as u32
                    | multiple
                    | WS_VSCROLL
                    | WS_BORDER
                    | WS_TABSTOP,
                0,
            )
        }
        ControlKind::Slider => (TRACKBAR_CLASSW, TBS_NOTICKS | WS_TABSTOP, 0),
        ControlKind::ProgressBar => (PROGRESS_CLASSW, 0, 0),
        ControlKind::Time => (DATETIMEPICK_CLASSW, DTS_TIMEFORMAT | WS_TABSTOP, extended),
        ControlKind::Date | ControlKind::DateTime | ControlKind::Month => (
            DATETIMEPICK_CLASSW,
            DTS_SHORTDATECENTURYFORMAT | WS_TABSTOP,
            extended,
        ),
    }
}

/// Creates a control on a surface.
pub(crate) fn add(window: WindowId, surface: u64, control: &Control) {
    let Some(parent) = surface_hwnd(window, surface) else {
        return;
    };
    let (class, style, extended) = class_of(control);
    // SAFETY: the class exists and the parent is a live window.
    let hwnd = unsafe {
        CreateWindowExW(
            extended,
            class,
            null(),
            WS_CHILD | style,
            0,
            0,
            0,
            0,
            parent,
            null_mut(),
            instance(),
            null(),
        )
    };
    if hwnd.is_null() {
        return;
    }
    // SAFETY: the control was just created; the subclass is removed when it
    // is destroyed.
    unsafe { SetWindowSubclass(hwnd, Some(subclass_proc), 1, control.id as usize) };
    STATE.with_borrow_mut(|state| {
        state
            .handles
            .insert(hwnd as usize, (window, Role::Control(control.id)));
        if let Some(data) = state.windows.get_mut(&window) {
            data.controls.insert(
                control.id,
                ControlData {
                    hwnd,
                    surface,
                    control: control.clone(),
                    composing: false,
                },
            );
        }
    });
    configure(window, hwnd, surface, None, control);
}

/// Updates the bounds and state of a control.
pub(crate) fn update(window: WindowId, control: &Control) {
    let found = STATE.with_borrow_mut(|state| {
        let data = state
            .windows
            .get_mut(&window)?
            .controls
            .get_mut(&control.id)?;
        let old = core::mem::replace(&mut data.control, control.clone());
        Some((data.hwnd, data.surface, old))
    });
    if let Some((hwnd, surface, old)) = found {
        configure(window, hwnd, surface, Some(&old), control);
    }
}

/// Native toggles draw author CSS from their surface, so a decoration-only
/// surface update must also invalidate their client pixels.
pub(crate) fn invalidate_authored(window: WindowId, surface: u64) {
    let handles: Vec<_> = STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .into_iter()
            .flat_map(|data| data.controls.values())
            .filter(|data| {
                data.surface == surface
                    && data.control.state.bare
                    && matches!(
                        data.control.kind,
                        ControlKind::CheckBox | ControlKind::Radio
                    )
            })
            .map(|data| data.hwnd)
            .collect()
    });
    for handle in handles {
        // SAFETY: each handle belongs to a live control of this thread.
        unsafe { InvalidateRect(handle, null(), 0) };
    }
}

/// Destroys a control.
pub(crate) fn remove(window: WindowId, id: u64) {
    let removed = STATE.with_borrow_mut(|state| {
        let data = state.windows.get_mut(&window)?.controls.remove(&id)?;
        state.handles.remove(&(data.hwnd as usize));
        Some(data.hwnd)
    });
    if let Some(hwnd) = removed {
        // SAFETY: the handle belonged to the removed control.
        unsafe { DestroyWindow(hwnd) };
    }
}

/// Restacks controls; the first one is on top, so that it also comes first
/// in the tab order.
pub(crate) fn order(window: WindowId, ids: &[u64]) {
    for id in ids.iter().rev() {
        if let Some(hwnd) = hwnd(window, *id) {
            // SAFETY: the handle belongs to a live control of this thread.
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
}

/// Re-applies fonts and positions after the window's scale changed.
pub(crate) fn rescale(window: WindowId) {
    let controls: Vec<(HWND, u64, Control)> = STATE.with_borrow(|state| {
        state.windows.get(&window).map_or(Vec::new(), |data| {
            data.controls
                .values()
                .map(|data| (data.hwnd, data.surface, data.control.clone()))
                .collect()
        })
    });
    for (hwnd, surface, control) in controls {
        configure(window, hwnd, surface, Some(&control), &control);
    }
}

fn send(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: the handle belongs to a live control and the message matches
    // its class.
    unsafe { SendMessageW(hwnd, message, wparam, lparam) }
}

fn set_text(hwnd: HWND, text: &str, edit: bool) {
    if window_text(hwnd) == text {
        return;
    }
    let (mut start, mut end) = (0_u32, 0_u32);
    if edit {
        send(
            hwnd,
            EM_GETSEL,
            &mut start as *mut u32 as usize,
            &mut end as *mut u32 as isize,
        );
    }
    let units = wide(text);
    // SAFETY: the text outlives the call.
    unsafe { SetWindowTextW(hwnd, units.as_ptr()) };
    if edit {
        send(hwnd, EM_SETSEL, start as usize, end as isize);
    }
}

/// A text control and its text as UTF-16, with `\r\n` line breaks.
fn text_control(window: WindowId, id: u64) -> Option<(HWND, Vec<u16>)> {
    let (handle, kind) = STATE.with_borrow(|state| {
        let data = state.windows.get(&window)?.controls.get(&id)?;
        Some((data.hwnd, data.control.kind))
    })?;
    let text = matches!(
        kind,
        ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea
    );
    text.then(|| (handle, window_text(handle).encode_utf16().collect()))
}

/// The DOM offset, which counts a `\r\n` line break once, of a native one.
fn dom_offset(text: &[u16], native: usize) -> usize {
    let native = native.min(text.len());
    native
        - text[..native]
            .windows(2)
            .filter(|pair| *pair == [13, 10])
            .count()
}

/// The native offset of a DOM offset.
fn native_offset(text: &[u16], dom: usize) -> usize {
    let mut index = 0;
    for _ in 0..dom {
        if index >= text.len() {
            break;
        }
        index += if text[index..].starts_with(&[13, 10]) {
            2
        } else {
            1
        };
    }
    index
}

/// The selection of a text control in DOM offsets, as `(start, end)`.
pub fn text_selection(window: WindowId, control: u64) -> Option<(usize, usize)> {
    let (hwnd, text) = text_control(window, control)?;
    let (mut start, mut end) = (0_u32, 0_u32);
    send(
        hwnd,
        EM_GETSEL,
        &mut start as *mut u32 as usize,
        &mut end as *mut u32 as isize,
    );
    Some((
        dom_offset(&text, start as usize),
        dom_offset(&text, end as usize),
    ))
}

/// Selects text in a text control; offsets are DOM offsets.
pub(crate) fn select_text(window: WindowId, control: u64, start: usize, end: usize) {
    if let Some((hwnd, text)) = text_control(window, control) {
        let (start, end) = (native_offset(&text, start), native_offset(&text, end));
        send(hwnd, EM_SETSEL, start, end as isize);
    }
}

fn set_options(hwnd: HWND, reset: u32, add: u32, options: &[String]) {
    send(hwnd, reset, 0, 0);
    for option in options {
        let units = wide(option);
        send(hwnd, add, 0, units.as_ptr() as isize);
    }
}

fn configure(window: WindowId, hwnd: HWND, surface: u64, old: Option<&Control>, control: &Control) {
    let (scale, scroll) = STATE.with_borrow(|state| {
        state
            .windows
            .get(&window)
            .map_or((1.0, (0.0, 0.0)), |data| {
                (
                    data.scale,
                    data.surfaces
                        .get(&surface)
                        .map_or((0.0, 0.0), |surface| surface.scroll),
                )
            })
    });
    let state = &control.state;
    let options_changed = old.is_none_or(|old| old.state.options != state.options);
    UPDATING.set(true);
    if is_picker(control.kind) && old.is_none_or(|old| old.state.bare != state.bare) {
        // The themed picker paints its own frame inside the client area.
        // SAFETY: empty theme names disable theming; NULL restores the system theme.
        unsafe {
            SetWindowTheme(
                hwnd,
                if state.bare { w!("") } else { null() },
                if state.bare { w!("") } else { null() },
            );
        }
    }
    // Date and time pickers give themselves an edge when created.
    // SAFETY: the handle belongs to a live control of this thread.
    let current = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
    if is_field(control.kind) && current != extended_style(control) {
        // SAFETY: the handle belongs to a live control of this thread.
        unsafe {
            SetWindowLongW(hwnd, GWL_EXSTYLE, extended_style(control) as i32);
            SetWindowPos(
                hwnd,
                null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
    let rect = window_rect(control, scale);
    let shift = rect.y - control.rect.y;
    place(hwnd, rect, extra_height(control.kind), scroll, scale);
    if state.transform.is_some() {
        proxy::hide(hwnd);
    }
    // Window regions are relative to the outer control bounds, so neither
    // surface scrolling nor non-client offsets are added here.
    if let Some(clip) = state.clip.map(|clip| nativeboat_ir::Rect {
        y: clip.y - shift,
        ..clip
    }) {
        // SAFETY: the window is live. Windows takes ownership only on success.
        unsafe {
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
    } else if old.is_some_and(|old| old.state.clip.is_some()) {
        // SAFETY: NULL removes the region; Windows releases its old region.
        unsafe { SetWindowRgn(hwnd, null_mut(), 1) };
    }
    let font = fonts::control_font(&state.font, scale, state.grayscale);
    send(hwnd, WM_SETFONT, font as usize, 1);
    // A field an author styles shows its text from the edges of its content
    // box, as Chromium's does: setting a font gives an edit control margins
    // from the font.
    if state.bare
        && matches!(
            control.kind,
            ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea
        )
    {
        send(
            hwnd,
            EM_SETMARGINS,
            (EC_LEFTMARGIN | EC_RIGHTMARGIN) as usize,
            0,
        );
    }
    // SAFETY: the handle belongs to a live control of this thread.
    unsafe { EnableWindow(hwnd, i32::from(!state.disabled)) };
    match control.kind {
        ControlKind::Button => set_text(hwnd, &state.text, false),
        ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea => {
            let text = if control.kind == ControlKind::TextArea {
                state.text.replace("\r\n", "\n").replace('\n', "\r\n")
            } else {
                state.text.clone()
            };
            set_text(hwnd, &text, true);
            // The placeholder is painted over the empty field in its CSS
            // color; a cue banner would show in the system's gray.
            if old.is_none_or(|old| {
                old.state.placeholder != state.placeholder
                    || old.state.placeholder_color != state.placeholder_color
            }) {
                // SAFETY: the handle belongs to a live control of this thread.
                unsafe { InvalidateRect(hwnd, null(), 1) };
            }
            // Alignment changes after creation, too.
            // SAFETY: the handle belongs to a live control of this thread.
            unsafe {
                let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                let aligned = (style & !((ES_CENTER | ES_RIGHT) as u32)) | alignment(control);
                if aligned != style {
                    SetWindowLongW(hwnd, GWL_STYLE, aligned as i32);
                    InvalidateRect(hwnd, null(), 1);
                }
            }
            send(hwnd, EM_SETREADONLY, usize::from(state.read_only), 0);
            send(
                hwnd,
                EM_SETLIMITTEXT,
                state.max_length.unwrap_or(0) as usize,
                0,
            );
        }
        ControlKind::CheckBox | ControlKind::Radio => {
            let check = if state.indeterminate {
                BST_INDETERMINATE
            } else if state.checked {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            };
            send(hwnd, BM_SETCHECK, check as usize, 0);
        }
        ControlKind::ComboBox => {
            if options_changed {
                set_options(hwnd, CB_RESETCONTENT, CB_ADDSTRING, &state.options);
            }
            let selected = state.selected.first().map_or(usize::MAX, |index| *index);
            send(hwnd, CB_SETCURSEL, selected, 0);
        }
        ControlKind::ListBox => {
            if options_changed {
                set_options(hwnd, LB_RESETCONTENT, LB_ADDSTRING, &state.options);
            }
            if state.multiple {
                send(hwnd, LB_SETSEL, 0, -1);
                for index in &state.selected {
                    send(hwnd, LB_SETSEL, 1, *index as isize);
                }
            } else {
                let selected = state.selected.first().map_or(usize::MAX, |index| *index);
                send(hwnd, LB_SETCURSEL, selected, 0);
            }
        }
        ControlKind::Slider => {
            send(hwnd, TBM_SETRANGEMIN, 0, state.minimum.round() as isize);
            send(hwnd, TBM_SETRANGEMAX, 1, state.maximum.round() as isize);
            send(hwnd, TBM_SETPOS, 1, state.value.round() as isize);
        }
        ControlKind::ProgressBar => {
            send(
                hwnd,
                PBM_SETRANGE32,
                0,
                (state.maximum * 1000.0).round() as isize,
            );
            send(hwnd, PBM_SETPOS, (state.value * 1000.0).round() as usize, 0);
        }
        ControlKind::Date | ControlKind::Time | ControlKind::DateTime | ControlKind::Month => {
            let time = datetime::system_time(control.kind, &state.text);
            let picture = wide(&datetime::picture(control.kind, time.is_none()));
            send(hwnd, DTM_SETFORMATW, 0, picture.as_ptr() as isize);
            if let Some(time) = time {
                send(
                    hwnd,
                    DTM_SETSYSTEMTIME,
                    GDT_VALID as usize,
                    &time as *const SYSTEMTIME as isize,
                );
            }
        }
    }
    // SAFETY: the handle belongs to a live control of this thread.
    unsafe {
        ShowWindow(hwnd, if state.visible { SW_SHOWNA } else { SW_HIDE });
        InvalidateRect(hwnd, null(), 1);
    }
    UPDATING.set(false);
}

fn control_of(window: WindowId, hwnd: HWND) -> Option<(u64, Control)> {
    STATE.with_borrow(|state| {
        let (_, role) = state.handles.get(&(hwnd as usize))?;
        let Role::Control(id) = role else {
            return None;
        };
        let data = state.windows.get(&window)?.controls.get(id)?;
        Some((*id, data.control.clone()))
    })
}

fn list_selection(hwnd: HWND, multiple: bool) -> Vec<usize> {
    if !multiple {
        let index = send(hwnd, LB_GETCURSEL, 0, 0);
        return usize::try_from(index).into_iter().collect();
    }
    let count = send(hwnd, LB_GETSELCOUNT, 0, 0).max(0) as usize;
    let mut indexes = vec![0_i32; count];
    send(hwnd, LB_GETSELITEMS, count, indexes.as_mut_ptr() as isize);
    indexes.into_iter().map(|index| index as usize).collect()
}

/// Handles a `WM_COMMAND` notification from a control.
pub(crate) fn command(window: WindowId, wparam: WPARAM, hwnd: HWND) {
    let Some((id, control)) = control_of(window, hwnd) else {
        return;
    };
    let code = ((wparam >> 16) & 0xFFFF) as u32;
    let event = match control.kind {
        ControlKind::Button | ControlKind::CheckBox | ControlKind::Radio if code == BN_CLICKED => {
            ControlEvent::Click
        }
        ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea
            if code == EN_CHANGE && !UPDATING.get() =>
        {
            ControlEvent::Input(window_text(hwnd).replace("\r\n", "\n"))
        }
        ControlKind::ComboBox if code == CBN_SELCHANGE => {
            let index = send(hwnd, CB_GETCURSEL, 0, 0);
            ControlEvent::Select(usize::try_from(index).into_iter().collect())
        }
        ControlKind::ListBox if code == LBN_SELCHANGE => {
            ControlEvent::Select(list_selection(hwnd, control.state.multiple))
        }
        _ => return,
    };
    dispatch(window, Event::Control { id, event });
}

/// Handles a `WM_NOTIFY` notification from a control: a picked date or time.
pub(crate) fn notify(window: WindowId, header: &NMHDR) {
    if header.code == DTN_DROPDOWN {
        proxy::dropped(window, header.hwndFrom);
        return;
    }
    if header.code != DTN_DATETIMECHANGE || UPDATING.get() {
        return;
    }
    let Some((id, control)) = control_of(window, header.hwndFrom) else {
        return;
    };
    if !is_picker(control.kind) {
        return;
    }
    // Pickers always hold a time: empty inputs only show a placeholder.
    let mut time = SYSTEMTIME::default();
    send(
        header.hwndFrom,
        DTM_GETSYSTEMTIME,
        0,
        &mut time as *mut SYSTEMTIME as isize,
    );
    let value = datetime::value(control.kind, &time);
    dispatch(
        window,
        Event::Control {
            id,
            event: ControlEvent::Input(value),
        },
    );
}

/// Handles a trackbar's `WM_HSCROLL` notification.
pub(crate) fn slider(window: WindowId, hwnd: HWND) {
    if let Some((id, _)) = control_of(window, hwnd) {
        let value = send(hwnd, TBM_GETPOS, 0, 0) as f64;
        dispatch(
            window,
            Event::Control {
                id,
                event: ControlEvent::Value(value),
            },
        );
    }
}

fn brush(color: COLORREF) -> HBRUSH {
    if let Some(brush) = BRUSHES.with_borrow(|brushes| brushes.get(&color).copied()) {
        return brush;
    }
    // SAFETY: creates a brush kept for the thread's lifetime.
    let brush = unsafe { CreateSolidBrush(color) };
    BRUSHES.with_borrow_mut(|brushes| brushes.insert(color, brush));
    brush
}

/// A color over an opaque one.
fn over(fill: Color, under: Color) -> Color {
    let alpha = fill.a.clamp(0.0, 1.0);
    let mix = |over: u8, under: u8| {
        (f32::from(over) * alpha + f32::from(under) * (1.0 - alpha)).round() as u8
    };
    Color::rgb(
        mix(fill.r, under.r),
        mix(fill.g, under.g),
        mix(fill.b, under.b),
    )
}

/// What shows beneath a control's middle with its own background: the
/// fills painted there before it, each over the ones before, on white.
/// Translucent and transparent backgrounds thus show what they cover.
/// Also the opacity of the groups the control is in, which fade it.
fn beneath(items: &[DisplayItem], control: &Control) -> (Color, f32) {
    let area = control.rect;
    let (x, y) = (area.x + area.width / 2.0, area.y + area.height / 2.0);
    let mut color = Color::WHITE;
    let mut own = false;
    // Fills in opacity groups show as faded as the groups.
    let mut groups: Vec<f32> = Vec::new();
    let faded = |fill: Color, groups: &[f32]| Color {
        a: fill.a * groups.iter().product::<f32>(),
        ..fill
    };
    for item in items {
        match item {
            DisplayItem::Child(NativeChild::Control(id)) if *id == control.id => break,
            DisplayItem::PushOpacity(opacity) => groups.push(opacity.clamp(0.0, 1.0)),
            DisplayItem::PopOpacity => {
                groups.pop();
            }
            DisplayItem::Fill {
                rect, color: fill, ..
            } if rect.x <= x
                && x < rect.x + rect.width
                && rect.y <= y
                && y < rect.y + rect.height =>
            {
                own |= (rect.x - area.x).abs() < 0.5
                    && (rect.y - area.y).abs() < 0.5
                    && (rect.width - area.width).abs() < 0.5
                    && (rect.height - area.height).abs() < 0.5;
                color = over(faded(*fill, &groups), color);
            }
            _ => {}
        }
    }
    let opacity = groups.iter().product::<f32>();
    // A background the display list does not paint goes over the rest; the
    // display list paints the boxes of fields an author styles.
    if own || control.state.bare {
        (color, opacity)
    } else {
        (
            over(faded(control.state.background, &groups), color),
            opacity,
        )
    }
}

/// Colors a control from its CSS colors (`WM_CTLCOLOR*`).
pub(crate) fn color(window: WindowId, dc: HDC, hwnd: HWND) -> Option<LRESULT> {
    let found = STATE.with_borrow(|state| {
        let (_, Role::Control(id)) = *state.handles.get(&(hwnd as usize))? else {
            return None;
        };
        let data = state.windows.get(&window)?;
        let control = data.controls.get(&id)?;
        let items = data
            .surfaces
            .get(&control.surface)
            .map_or(&[][..], |surface| &surface.items[..]);
        let text = control.control.state.color;
        let (background, opacity) = beneath(items, &control.control);
        // Text in an opacity group shows faded into the background.
        let text = Color {
            a: text.a * opacity,
            ..text
        };
        Some((over(text, background), background))
    });
    let (text, background) = found?;
    // SAFETY: the DC belongs to the control being painted.
    unsafe {
        SetTextColor(dc, colorref(text));
        SetBkColor(dc, colorref(background));
    }
    Some(brush(colorref(background)) as LRESULT)
}

/// Paints the placeholder of an empty text field over what the EDIT
/// control painted, in the `::placeholder` color, where its text would be.
fn paint_placeholder(window: WindowId, id: u64, hwnd: HWND, dc: HDC) {
    let found = STATE.with_borrow(|state| {
        let data = state.windows.get(&window)?;
        let control = data.controls.get(&id)?;
        if !matches!(
            control.control.kind,
            ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea
        ) || control.control.state.placeholder.is_empty()
            || control.composing
        {
            return None;
        }
        let items = data
            .surfaces
            .get(&control.surface)
            .map_or(&[][..], |surface| &surface.items[..]);
        let (background, opacity) = beneath(items, &control.control);
        let state = &control.control.state;
        let color = Color {
            a: state.placeholder_color.a * opacity,
            ..state.placeholder_color
        };
        Some((
            state.placeholder.clone(),
            over(color, background),
            control.control.kind == ControlKind::TextArea,
            (state.font.clone(), state.grayscale, data.scale),
        ))
    });
    // SAFETY: reads the length of the live control's text.
    let Some((text, color, multiline, (font, grayscale, scale))) =
        found.filter(|_| unsafe { GetWindowTextLengthW(hwnd) } == 0)
    else {
        return;
    };
    let mut area = RECT::default();
    send(hwnd, EM_GETRECT, 0, (&raw mut area) as isize);
    // SAFETY: reads the style of the live control.
    let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
    let (align, position) = if style & ES_CENTER as u32 != 0 {
        (DT_CENTER, 0.5)
    } else if style & ES_RIGHT as u32 != 0 {
        (DT_RIGHT, 1.0)
    } else {
        (DT_LEFT, 0.0)
    };
    if !multiline {
        let line = Line {
            text,
            font,
            color,
            grayscale,
        };
        paint_line(dc, area, &line, position, scale);
        return;
    }
    // A textarea's placeholder wraps as the control's own text does.
    let font = send(hwnd, WM_GETFONT, 0, 0) as HGDIOBJ;
    let mut units = wide(&text);
    units.pop();
    // SAFETY: the DC and the control's font are live; the state is restored.
    unsafe {
        let saved = SaveDC(dc);
        if !font.is_null() {
            SelectObject(dc, font);
        }
        SetTextColor(dc, colorref(color));
        SetBkMode(dc, TRANSPARENT as i32);
        DrawTextW(
            dc,
            units.as_ptr(),
            units.len() as i32,
            &mut area,
            align | DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL,
        );
        RestoreDC(dc, saved);
    }
}

/// A line of text a field paints over its client area.
struct Line {
    text: String,
    font: nativeboat_ir::Font,
    color: Color,
    grayscale: bool,
}

/// Paints a line of text at the top of `area`, a rectangle of a DC in
/// device pixels, placed along it as `position` says (0 at its start, 1 at
/// its end), and clipped to it. It is painted as the page's text is: at the
/// face's advances for the exact size, as layout measures it, and smoothed
/// in grayscale as Skia smooths it.
fn paint_line(dc: HDC, area: RECT, line: &Line, position: f32, scale: f32) {
    let (width, height) = (area.right - area.left, area.bottom - area.top);
    if width <= 0 || height <= 0 {
        return;
    }
    let measurer = fonts::GdiMeasurer::new();
    measurer.set_scale(scale);
    let advance = nativeboat_layout::TextMeasurer::width(&measurer, &line.text, &line.font);
    let room = width as f32 / scale;
    let item = DisplayItem::Text {
        rect: nativeboat_ir::Rect::new(
            (room - advance).max(0.0) * position,
            0.0,
            advance,
            fonts::cell_height(&line.font, scale),
        ),
        text: line.text.clone(),
        font: line.font.clone(),
        color: line.color,
        underline: false,
        line_through: false,
        letter_spacing: 0.0,
        rtl: false,
        grayscale: line.grayscale,
        underline_offset: None,
        decorations: Vec::new(),
    };
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: the DC is live; the memory DC and its section are owned and
    // released here. Painting reads and writes only the section's pixels.
    unsafe {
        let memory = CreateCompatibleDC(dc);
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(memory);
            return;
        }
        let previous = SelectObject(memory, bitmap);
        // What the control painted lies beneath the text.
        BitBlt(
            memory, 0, 0, width, height, dc, area.left, area.top, SRCCOPY,
        );
        paint_area(memory, &[item], scale, (0.0, 0.0), None, false);
        GdiFlush();
        BitBlt(
            dc, area.left, area.top, width, height, memory, 0, 0, SRCCOPY,
        );
        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
    }
}

/// Paints what the page shows where a control is inside its client area:
/// an author's toggle box, or what lies under a see-through text field. The
/// real control keeps state, focus, input and default providers; only its
/// pixels are supplied by the native surface display list.
fn paint_toggle(
    window: WindowId,
    surface: u64,
    hwnd: HWND,
    target: HDC,
    clip: Option<nativeboat_ir::Rect>,
) {
    let mut area = RECT::default();
    // SAFETY: these handles are live. The DIB and memory DC are owned here,
    // and rendering borrows the display list without dispatching app events.
    unsafe {
        GetClientRect(hwnd, &mut area);
        if area.right <= 0 || area.bottom <= 0 {
            return;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: area.right,
                biHeight: -area.bottom,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let memory = CreateCompatibleDC(target);
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(memory);
            return;
        }
        let old = SelectObject(memory, bitmap);
        PatBlt(memory, 0, 0, area.right, area.bottom, WHITENESS);
        let mut origin = POINT::default();
        MapWindowPoints(hwnd, GetParent(hwnd), &mut origin, 1);
        STATE.with_borrow(|state| {
            if let Some(data) = state.windows.get(&window)
                && let Some(surface) = data.surfaces.get(&surface)
            {
                let scroll = (
                    surface.scroll.0 + origin.x as f32 / data.scale,
                    surface.scroll.1 + origin.y as f32 / data.scale,
                );
                paint_area(
                    memory,
                    &surface.items,
                    data.scale,
                    scroll,
                    Some(area),
                    false,
                );
            }
        });
        crate::window::stack::paint(window, NativeChild::Surface(surface), hwnd, memory, area);
        let saved = SaveDC(target);
        if let Some(clip) = clip {
            let scale = crate::window::scale(window);
            IntersectClipRect(
                target,
                px(clip.x, scale),
                px(clip.y, scale),
                px(clip.x + clip.width, scale),
                px(clip.y + clip.height, scale),
            );
        }
        BitBlt(target, 0, 0, area.right, area.bottom, memory, 0, 0, SRCCOPY);
        RestoreDC(target, saved);
        SelectObject(memory, old);
        DeleteObject(bitmap);
        DeleteDC(memory);
    }
}

/// Native controls paint their own client first. Later compiled display
/// items are composited above it while preserving native input behavior.
fn paint_control(
    window: WindowId,
    id: u64,
    hwnd: HWND,
    target: HDC,
    message: u32,
    flags: LPARAM,
    clip: Option<nativeboat_ir::Rect>,
) {
    let mut area = RECT::default();
    // SAFETY: the control and target are live; all temporary GDI objects
    // are released here. Native printing copies no caller-owned pointers.
    unsafe {
        GetClientRect(hwnd, &mut area);
        if area.right <= 0 || area.bottom <= 0 {
            return;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: area.right,
                biHeight: -area.bottom,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let memory = CreateCompatibleDC(target);
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(memory, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            DeleteDC(memory);
            return;
        }
        let previous = SelectObject(memory, bitmap);
        PatBlt(memory, 0, 0, area.right, area.bottom, WHITENESS);
        let printing = PRINTING.replace(hwnd as usize);
        DefSubclassProc(hwnd, message, memory as usize, flags);
        PRINTING.set(printing);
        paint_placeholder(window, id, hwnd, memory);
        GdiFlush();
        crate::window::stack::paint(window, NativeChild::Control(id), hwnd, memory, area);
        let saved = SaveDC(target);
        if let Some(clip) = clip {
            let scale = crate::window::scale(window);
            IntersectClipRect(
                target,
                px(clip.x, scale),
                px(clip.y, scale),
                px(clip.x + clip.width, scale),
                px(clip.y + clip.height, scale),
            );
        }
        BitBlt(target, 0, 0, area.right, area.bottom, memory, 0, 0, SRCCOPY);
        RestoreDC(target, saved);
        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
    }
}

/// Whether a text field the page styles has transparent text, as fields laid
/// over a box that shows their text highlighted have: it shows the page
/// where it is, and only its caret and placeholder paint over it.
fn see_through(hwnd: HWND, id: u64) -> bool {
    STATE.with_borrow(|state| {
        let Some((window, _)) = state.handles.get(&(hwnd as usize)) else {
            return false;
        };
        state
            .windows
            .get(window)
            .and_then(|data| data.controls.get(&id))
            .is_some_and(|data| {
                let control = &data.control;
                control.state.bare
                    && control.state.color.a <= 0.0
                    && matches!(
                        control.kind,
                        ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea
                    )
            })
    })
}

fn control_message(
    hwnd: HWND,
    id: u64,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let (window, kind, authored, surface, clip) = STATE.with_borrow(|state| {
        let (window, _) = state.handles.get(&(hwnd as usize))?;
        let data = state.windows.get(window)?.controls.get(&id)?;
        Some((
            *window,
            data.control.kind,
            data.control.state.bare,
            data.surface,
            data.control.state.clip,
        ))
    })?;
    let see_through = see_through(hwnd, id);
    // Keep native drag selection captured even when it crosses an adornment.
    // SAFETY: the HWND belongs to this thread; GetCapture has no side effects.
    if message == WM_NCHITTEST && unsafe { GetCapture() } != hwnd {
        let x = i32::from(lparam as u16 as i16);
        let y = i32::from((lparam >> 16) as u16 as i16);
        let (x, y) = crate::window::root_point(window, hwnd, x, y, true);
        if crate::event_loop::above_control(window, id, x, y) {
            return Some(HTTRANSPARENT as isize);
        }
    }
    // A field's frame and scroll bar paint with the page content above them,
    // as its client does, over its client's native look.
    if message == WM_NCPAINT
        && crate::window::stack::paint_frame(window, NativeChild::Control(id), hwnd, |dc, inset| {
            let mut previous = POINT::default();
            // SAFETY: prints the live control's client into the caller's DC,
            // whose origin is restored after.
            unsafe {
                SetViewportOrgEx(dc, inset.x, inset.y, &mut previous);
                DefSubclassProc(hwnd, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
                SetViewportOrgEx(dc, previous.x, previous.y, null_mut());
            }
        })
    {
        return Some(0);
    }
    if (authored && matches!(kind, ControlKind::CheckBox | ControlKind::Radio)) || see_through {
        let page = |dc: HDC| {
            // The caret blinks over what paints, so it hides while the page
            // paints a see-through field; the field's placeholder shows over
            // the page.
            if see_through {
                // SAFETY: the caret, when this field has it, belongs to it.
                unsafe { HideCaret(hwnd) };
            }
            paint_toggle(window, surface, hwnd, dc, clip);
            if see_through {
                paint_placeholder(window, id, hwnd, dc);
                // SAFETY: pairs the HideCaret above.
                unsafe { ShowCaret(hwnd) };
            }
        };
        match message {
            WM_ERASEBKGND => return Some(1),
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                // SAFETY: validate the owned control's update region.
                unsafe {
                    let dc = BeginPaint(hwnd, &mut paint);
                    page(dc);
                    EndPaint(hwnd, &paint);
                }
                return Some(0);
            }
            WM_PRINTCLIENT => {
                page(wparam as HDC);
                return Some(0);
            }
            WM_PRINT => {
                if lparam as i32 & PRF_CHECKVISIBLE != 0 {
                    // SAFETY: printing honors the caller's visibility flag.
                    if unsafe { IsWindowVisible(hwnd) } == 0 {
                        return Some(0);
                    }
                }
                if lparam as i32 & PRF_CLIENT != 0 {
                    page(wparam as HDC);
                }
                return Some(0);
            }
            _ => {}
        }
    }
    if PRINTING.get() != hwnd as usize
        && crate::window::stack::exists(window, NativeChild::Control(id))
    {
        match message {
            WM_ERASEBKGND => return Some(1),
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                // SAFETY: validate this control's paint region exactly once.
                unsafe {
                    let dc = BeginPaint(hwnd, &mut paint);
                    paint_control(
                        window,
                        id,
                        hwnd,
                        dc,
                        WM_PRINTCLIENT,
                        PRF_CLIENT as isize,
                        clip,
                    );
                    EndPaint(hwnd, &paint);
                }
                return Some(0);
            }
            WM_PRINTCLIENT => {
                paint_control(window, id, hwnd, wparam as HDC, message, lparam, clip);
                return Some(0);
            }
            WM_PRINT if lparam as i32 & PRF_CLIENT != 0 => {
                // SAFETY: printing honors the caller's visibility request.
                if lparam as i32 & PRF_CHECKVISIBLE == 0 || unsafe { IsWindowVisible(hwnd) } != 0 {
                    paint_control(window, id, hwnd, wparam as HDC, message, lparam, clip);
                }
                return Some(0);
            }
            _ => {}
        }
    }
    if message == WM_PAINT
        && matches!(
            kind,
            ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea
        )
        // SAFETY: reads the length of the live control's text.
        && unsafe { GetWindowTextLengthW(hwnd) } == 0
    {
        let mut paint = PAINTSTRUCT::default();
        // SAFETY: validates the control's update region once; the EDIT
        // prints its client area into the paint DC first.
        unsafe {
            let dc = BeginPaint(hwnd, &mut paint);
            DefSubclassProc(
                hwnd,
                WM_PRINTCLIENT,
                dc as usize,
                (PRF_CLIENT | PRF_ERASEBKGND) as isize,
            );
            paint_placeholder(window, id, hwnd, dc);
            EndPaint(hwnd, &paint);
        }
        return Some(0);
    }
    // IMM starts before it supplies any preedit text. Keep the session flag
    // independently of the control's value, and let the native EDIT procedure
    // continue handling every composition message.
    if message == WM_KILLFOCUS
        || (matches!(message, WM_IME_STARTCOMPOSITION | WM_IME_ENDCOMPOSITION)
            && matches!(
                kind,
                ControlKind::TextBox | ControlKind::Password | ControlKind::TextArea
            ))
    {
        STATE.with_borrow_mut(|state| {
            if let Some(data) = state
                .windows
                .get_mut(&window)
                .and_then(|data| data.controls.get_mut(&id))
            {
                data.composing = message == WM_IME_STARTCOMPOSITION;
            }
        });
    }
    match message {
        WM_SETFOCUS => {
            dispatch(
                window,
                Event::Control {
                    id,
                    event: ControlEvent::Focus,
                },
            );
            None
        }
        WM_KILLFOCUS => {
            dispatch(
                window,
                Event::Control {
                    id,
                    event: ControlEvent::Blur,
                },
            );
            None
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            if key(window, Some(id), message, wparam, lparam) {
                return Some(0);
            }
            match (message, wparam as u16) {
                (WM_KEYDOWN, VK_TAB) => {
                    // SAFETY: the handles belong to live windows of this thread.
                    unsafe {
                        let shift = i32::from(GetKeyState(i32::from(VK_SHIFT)) < 0);
                        let next = GetNextDlgTabItem(GetAncestor(hwnd, GA_ROOT), hwnd, shift);
                        if !next.is_null() {
                            SetFocus(next);
                        }
                    }
                    Some(0)
                }
                (WM_KEYDOWN, VK_RETURN) if kind == ControlKind::Button => {
                    dispatch(
                        window,
                        Event::Control {
                            id,
                            event: ControlEvent::Click,
                        },
                    );
                    Some(0)
                }
                _ => None,
            }
        }
        // Tab and Enter would beep in single-line controls.
        WM_CHAR if wparam == 9 || (wparam == 13 && kind != ControlKind::TextArea) => Some(0),
        _ => None,
    }
}

/// Gives the page the pointer over a control, which a proxied control
/// already has from the page; `Some` when the page sets the cursor.
fn page_pointer(
    hwnd: HWND,
    id: u64,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    if !matches!(message, WM_MOUSEMOVE | WM_MOUSELEAVE | WM_SETCURSOR) {
        return None;
    }
    let (window, proxied) = STATE.with_borrow(|state| {
        let (window, _) = state.handles.get(&(hwnd as usize))?;
        let data = state.windows.get(window)?.controls.get(&id)?;
        Some((*window, data.control.state.transform.is_some()))
    })?;
    if proxied {
        return None;
    }
    if message != WM_SETCURSOR {
        crate::window::control_pointer(window, hwnd, message, wparam, lparam);
        return None;
    }
    // Controls show the cursor the page's style asks for, such as
    // `cursor: pointer` on a checkbox or a text field's `cursor: text`.
    ((lparam & 0xFFFF) as u32 == HTCLIENT && crate::window::control_cursor(window)).then_some(1)
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    id: usize,
) -> LRESULT {
    if message == WM_NCDESTROY {
        // SAFETY: removes the subclass installed by `add`.
        unsafe { RemoveWindowSubclass(hwnd, Some(subclass_proc), 1) };
        // SAFETY: forwards the message unchanged.
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }
    // A control kept out of sight takes the input where its image shows,
    // and its image repaints after what changes it.
    let lparam = proxy::input(hwnd, id as u64, message, lparam);
    if let Some(result) = page_pointer(hwnd, id as u64, message, wparam, lparam) {
        return result;
    }
    let result = match control_message(hwnd, id as u64, message, wparam, lparam) {
        Some(result) => result,
        // SAFETY: forwards the message, its coordinates mapped.
        None => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
    };
    proxy::after(hwnd, id as u64, message);
    // The EDIT control draws its text at once after typing, selecting and
    // editing; a see-through field paints the page over it before it shows.
    if proxy::changes(message) && see_through(hwnd, id as u64) {
        // SAFETY: repaints this live control now.
        unsafe { RedrawWindow(hwnd, null(), null_mut(), RDW_INVALIDATE | RDW_UPDATENOW) };
    }
    result
}

#[cfg(test)]
mod tests;
