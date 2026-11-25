//! Win32 backend: top-level windows, scroll surfaces, native controls,
//! GDI+ painting of display lists, input and the message loop.
//!
//! Everything here is bound to the thread that calls [`init`]; windows,
//! fonts and images live in thread-local state. Input reaches the
//! application through the [`Handler`] installed by [`run`] (or
//! [`set_handler`]); the handler drives the platform through the free
//! functions of this crate.
//!
//! Without the `std` feature the crate links no Rust `std`, so that
//! engine-free applications built with NativeBoat's bundled toolchain use
//! the same platform layer; its state then lives in statics that only the
//! UI thread touches.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub(crate) mod prelude {
    #[cfg(not(feature = "std"))]
    pub use crate::float::Float;
    pub use alloc::borrow::ToOwned;
    pub use alloc::boxed::Box;
    pub use alloc::collections::{BTreeMap as HashMap, BTreeSet as HashSet, VecDeque};
    pub use alloc::format;
    pub use alloc::rc::Rc;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

/// Per-thread state: `thread_local!` with `std`, otherwise a static that
/// only the UI thread uses.
#[cfg(feature = "std")]
macro_rules! local {
    ($($(#[$attribute:meta])* $visibility:vis static $name:ident: $kind:ty = $value:expr;)*) => {
        std::thread_local! {
            $($(#[$attribute])* $visibility static $name: $kind = $value;)*
        }
    };
}

#[cfg(not(feature = "std"))]
macro_rules! local {
    ($($(#[$attribute:meta])* $visibility:vis static $name:ident: $kind:ty = $value:expr;)*) => {
        $($(#[$attribute])* $visibility static $name: crate::local::Local<$kind> =
            crate::local::Local::new(|| $value);)*
    };
}

#[cfg(not(feature = "std"))]
mod float;
#[cfg(not(feature = "std"))]
mod local;

#[allow(unused_imports)]
use prelude::*;

mod canvas;
mod clipboard;
mod controls;
mod datetime;
mod dialogs;
mod drag_source;
mod drop_target;
mod dwrite;
mod event_loop;
mod fonts;
mod keys;
mod menu;
mod print;
mod render;
mod system;
mod trace;
mod tray;
mod webp;
mod wic;
mod wide;
mod window;

pub use canvas::{
    canvas_draw, canvas_encode, canvas_read, canvas_release, canvas_write, image_draw,
    image_encode, image_from_pixels, image_pixels, measure_text,
};
pub use clipboard::{
    clipboard_formats, has_clipboard_format, read_clipboard_buffer, read_clipboard_contents,
    read_clipboard_image, read_html, read_text, write_clipboard, write_clipboard_buffer,
    write_clipboard_image,
};
pub use controls::proxy::forward as forward_to_control;
pub use controls::{hwnd as control_hwnd, text_selection};
pub use datetime::set_date_locale;
pub use dialogs::{
    FileDialog, FileFilter, MessageBox, MessageKind, error_box, file_dialog, message_box,
};
pub use drag_source::drag_out;
pub use dwrite::unpack_font;
pub use event_loop::{
    Waker, dispatch, quit, register_hotkey, run, set_handler, take_handler, unregister_hotkey,
};
pub use fonts::{GdiMeasurer, follow_chromium, follow_locale, register_font, register_font_face};
pub use menu::{
    Accelerator, MenuItem, MenuKind, edit_command, menu_modifiers, popup_menu,
    set_ignore_menu_shortcuts, set_menu,
};
pub use print::{Paper, PrintJob, Printer, open_printer, take_output, temporary_output};
pub use render::{add_image, add_vector_image, image_size, paint_tree};
pub use system::{
    Awake, Display, beep, cursor_position, dark_mode, displays, idle_seconds, keep_awake,
    known_folder, on_battery, open_external, open_path, protect, show_item_in_folder, trash,
    unprotect,
};
pub use tray::{
    TrayEvent, add_tray, remove_tray, set_tray_image, set_tray_tooltip, tray_balloon, tray_menu,
};
pub use window::{
    WindowOptions, adopt, apply, client_size, close, complete_composition, create_window, destroy,
    focus, focus_content, focus_control, follow_focus, hwnd, is_maximized, is_minimized,
    is_transparent, is_visible, keyboard_input, maximize, minimize, move_top, restore, scale,
    scrollbar_room, set_client_size, set_cursor, set_size_limits, set_title, set_zoom, show, title,
    type_into_control, window_ids, zoom,
};

use core::fmt;
use core::time::Duration;
use windows_sys::Win32::Foundation::GetLastError;

/// Identifies a top-level window.
pub type WindowId = u64;

/// A platform failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// What failed.
    pub message: String,
}

impl Error {
    /// An error naming a failed call and the thread's last error code.
    fn last(call: &str) -> Error {
        // SAFETY: GetLastError only reads thread state.
        let code = unsafe { GetLastError() };
        Error {
            message: format!("{call} failed with error {code}"),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl core::error::Error for Error {}

/// Modifier keys held during an input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    /// Shift.
    pub shift: bool,
    /// Control.
    pub ctrl: bool,
    /// Alt.
    pub alt: bool,
    /// The Windows key.
    pub meta: bool,
}

/// One menu command, with the input state captured when Windows sent it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuCommand {
    /// The item's command id.
    pub id: u16,
    /// Whether WM_COMMAND came from a translated accelerator.
    pub accelerator: bool,
    /// Modifier keys held when the command arrived.
    pub modifiers: Modifiers,
}

impl From<u16> for MenuCommand {
    fn from(id: u16) -> Self {
        Self {
            id,
            ..Self::default()
        }
    }
}

/// What happened to the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerKind {
    /// The pointer moved.
    Move,
    /// A button was pressed.
    Down,
    /// A button was released.
    Up,
    /// The pointer left the window.
    Leave,
}

/// Phase of an incoming Windows file drag, before the final drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileDragKind {
    /// The drag entered this native window.
    Enter,
    /// The pointer moved while dragging over this native window.
    Over,
    /// The drag left or was canceled.
    Leave,
}

/// Events of native controls.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlEvent {
    /// A button, checkbox or radio button was activated.
    Click,
    /// The text of a text control changed.
    Input(String),
    /// The selected options of a combo or list box changed.
    Select(Vec<usize>),
    /// A slider moved.
    Value(f64),
    /// The control received the keyboard focus.
    Focus,
    /// The control lost the keyboard focus.
    Blur,
}

/// Phases of native input-method composition over painted content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionKind {
    /// A new preedit replacement begins.
    Start,
    /// The preedit text or its insertion position changed.
    Update,
    /// The final text was committed, or empty text canceled it.
    End,
}

/// Keyboard input captured before menu accelerators and page/native defaults.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyboardInput {
    /// Whether the key was pressed rather than released.
    pub down: bool,
    /// The DOM key value.
    pub key: String,
    /// The physical DOM code.
    pub code: String,
    /// Whether this press repeats.
    pub repeat: bool,
    /// Whether a painted editing host has active IMM preedit.
    pub composing: bool,
    /// Held modifier keys.
    pub modifiers: Modifiers,
    /// Standard, left, right or numeric-keypad location (0 to 3).
    pub location: u8,
    /// Caps Lock toggle state.
    pub caps_lock: bool,
    /// Num Lock toggle state.
    pub num_lock: bool,
}

/// Input and window events delivered to the [`Handler`].
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Mouse input over painted content, in window coordinates.
    Pointer {
        /// What happened.
        kind: PointerKind,
        /// The DOM button number: 0 left, 1 middle, 2 right, 3 back, 4 forward.
        button: i16,
        /// The DOM bit mask of held buttons.
        buttons: u16,
        /// Click count: 2 for the second press of a double click.
        count: u32,
        /// Horizontal position in device-independent pixels.
        x: f32,
        /// Vertical position in device-independent pixels.
        y: f32,
        /// Held modifier keys.
        modifiers: Modifiers,
    },
    /// The mouse wheel turned; deltas are in pixels, positive scrolls down
    /// or right.
    Wheel {
        /// Horizontal position.
        x: f32,
        /// Vertical position.
        y: f32,
        /// Horizontal delta.
        delta_x: f32,
        /// Vertical delta.
        delta_y: f32,
        /// Held modifier keys.
        modifiers: Modifiers,
    },
    /// Incoming file drag metadata; file contents are read only on drop.
    FileDrag {
        /// Drag phase.
        kind: FileDragKind,
        /// Paths used internally to expose protected file-kind/type metadata.
        paths: Vec<String>,
        /// Text dragged along, by web type: `text/plain`, `text/uri-list`
        /// and `text/html`.
        strings: Vec<(String, String)>,
        /// Horizontal position in CSS pixels.
        x: f32,
        /// Vertical position in CSS pixels.
        y: f32,
        /// Held modifier keys.
        modifiers: Modifiers,
    },
    /// Files dropped from Windows Explorer, in window coordinates.
    FilesDropped {
        /// Absolute file paths supplied by the operating system.
        paths: Vec<String>,
        /// Text dropped along, by web type.
        strings: Vec<(String, String)>,
        /// Horizontal drop position in CSS pixels.
        x: f32,
        /// Vertical drop position in CSS pixels.
        y: f32,
    },
    /// Cancellable keyboard input before accelerator and page defaults.
    BeforeInput(KeyboardInput),
    /// A key was pressed or released.
    Key {
        /// Whether the key went down.
        down: bool,
        /// The DOM `key` value.
        key: String,
        /// The DOM `code` value.
        code: String,
        /// Whether the press repeats.
        repeat: bool,
        /// Held modifier keys.
        modifiers: Modifiers,
        /// The focused control, if any.
        control: Option<u64>,
    },
    /// The user typed text while the window's content had the keyboard
    /// focus.
    Text(String),
    /// A Unicode input method changed the focused painted editing host.
    Composition {
        /// Composition phase.
        kind: CompositionKind,
        /// Entire preedit or final text, never an incremental append.
        data: String,
        /// Position of the insertion point in UTF-16 code units.
        cursor: usize,
    },
    /// The user scrolled a surface; offsets are in device-independent pixels.
    Scroll {
        /// Surface id.
        surface: u64,
        /// Horizontal offset.
        x: f32,
        /// Vertical offset.
        y: f32,
    },
    /// A native control changed.
    Control {
        /// Control id.
        id: u64,
        /// What happened.
        event: ControlEvent,
    },
    /// The client area changed size.
    Resized {
        /// Width in device-independent pixels.
        width: f32,
        /// Height in device-independent pixels.
        height: f32,
    },
    /// The window was activated or deactivated.
    Activated(bool),
    /// The user asked to close the window; cancel to keep it open.
    Close,
    /// The window was destroyed.
    Destroyed,
    /// A menu item of the window, or its accelerator, was chosen.
    Menu(MenuCommand),
    /// A global shortcut registered with [`register_hotkey`] was pressed;
    /// sent to window `0`.
    HotKey(u32),
    /// Input on a notification area icon; sent to window `0`.
    Tray {
        /// The icon's id.
        id: u32,
        /// What happened.
        event: TrayEvent,
    },
    /// The user switched between the light and dark themes.
    ThemeChanged,
    /// The computer is going to sleep.
    Suspend,
    /// The computer woke up.
    Resume,
}

/// Receives platform events.
pub trait Handler {
    /// Handles an event; returns `true` to cancel its default action.
    fn event(&mut self, window: WindowId, event: Event) -> bool;

    /// Synchronous native geometry query, in window CSS pixels. Return true
    /// when another painted owner covers this control. Run no application
    /// behavior here; reentry retains native control handling.
    fn above_control(&mut self, _window: WindowId, _control: u64, _x: f32, _y: f32) -> bool {
        false
    }

    /// Runs pending work between messages and returns how long the loop may
    /// wait for input before calling again; `None` waits for input.
    fn idle(&mut self) -> Option<Duration>;
}

/// Prepares the calling thread: DPI awareness, common controls, GDI+ and
/// the window classes.
pub fn init() -> Result<(), Error> {
    window::init()
}

#[cfg(test)]
mod tests;
