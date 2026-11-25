use super::*;
use crate::render::tests::bitmap_file;
use crate::tests::support::{record, send};
use shimforge::{Session, mock};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    HMENU, SetForegroundWindow, TPMPARAMS, TrackPopupMenuEx, WM_MOUSEMOVE, WM_NULL,
};
use windows_sys::core::{BOOL, PCWSTR};

/// Message, flags, tooltip and balloon kind of each call.
type Calls = Arc<Mutex<Vec<(u32, u32, String, u32)>>>;

fn tip(data: &NOTIFYICONDATAW) -> String {
    crate::wide::narrow(&data.szTip)
}

#[test]
fn tray_icons_and_input() {
    let events = record();
    let calls = Calls::default();
    let seen = calls.clone();
    let mut session = Session::new();
    let shell = mock!(
        session,
        Shell_NotifyIconW,
        unsafe extern "system" fn(u32, *const NOTIFYICONDATAW) -> BOOL
    );
    shell.expect().returning(move |message, data| {
        // SAFETY: the structure is live during the call.
        let data = unsafe { &*data };
        seen.lock()
            .unwrap()
            .push((message, data.uFlags, tip(data), data.dwInfoFlags));
        1
    });
    crate::add_image("tray.bmp", &bitmap_file());
    add_tray(1, Some("tray.bmp"), "Tip").unwrap();
    set_tray_image(1, None).unwrap();
    set_tray_image(1, Some("missing.png")).unwrap();
    set_tray_tooltip(1, &"x".repeat(200)).unwrap();
    for kind in [
        MessageKind::None,
        MessageKind::Info,
        MessageKind::Warning,
        MessageKind::Error,
        MessageKind::Question,
    ] {
        tray_balloon(1, "Title", "Body", kind).unwrap();
    }
    remove_tray(1).unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].0, NIM_ADD);
    assert_eq!(calls[0].2, "Tip");
    assert_eq!(calls[3].2.len(), 127);
    let infos: Vec<u32> = calls[4..9].iter().map(|call| call.3).collect();
    assert_eq!(
        infos,
        [NIIF_NONE, NIIF_INFO, NIIF_WARNING, NIIF_ERROR, NIIF_INFO]
    );
    assert_eq!(calls[9].0, NIM_DELETE);
    let window = owner().unwrap();
    assert_eq!(owner().unwrap(), window);
    for mouse in [
        WM_LBUTTONUP,
        WM_RBUTTONUP,
        WM_LBUTTONDBLCLK,
        NIN_BALLOONUSERCLICK,
        WM_MOUSEMOVE,
    ] {
        send(window, TRAY_MESSAGE, 4, mouse as isize);
    }
    send(window, WM_NULL, 0, 0);
    let tray = |event| Event::Tray { id: 4, event };
    assert_eq!(
        events.take(),
        [
            tray(TrayEvent::Click),
            tray(TrayEvent::RightClick),
            tray(TrayEvent::DoubleClick),
            tray(TrayEvent::BalloonClick)
        ]
    );
}

#[test]
fn tray_failures() {
    let mut session = Session::new();
    let shell = mock!(
        session,
        Shell_NotifyIconW,
        unsafe extern "system" fn(u32, *const NOTIFYICONDATAW) -> BOOL
    );
    shell.expect().returning(|_, _| 0);
    assert!(
        add_tray(2, None, "Tip")
            .unwrap_err()
            .message
            .starts_with("Shell_NotifyIconW failed")
    );
}

#[test]
fn tray_windows_can_fail() {
    let mut session = Session::new();
    let create = mock!(
        session,
        CreateWindowExW,
        unsafe extern "system" fn(
            u32,
            PCWSTR,
            PCWSTR,
            u32,
            i32,
            i32,
            i32,
            i32,
            HWND,
            HMENU,
            windows_sys::Win32::Foundation::HINSTANCE,
            *const std::ffi::c_void,
        ) -> HWND
    );
    create
        .expect()
        .returning(|_, _, _, _, _, _, _, _, _, _, _, _| ptr::null_mut());
    assert!(
        add_tray(3, None, "Tip")
            .unwrap_err()
            .message
            .starts_with("CreateWindowExW failed")
    );
    assert!(tray_menu(&[]).is_err());
}

#[test]
fn tray_menus() {
    let mut session = Session::new();
    let foreground = mock!(
        session,
        SetForegroundWindow,
        unsafe extern "system" fn(HWND) -> BOOL
    );
    foreground.expect().returning(|_| 1);
    let track = mock!(
        session,
        TrackPopupMenuEx,
        unsafe extern "system" fn(HMENU, u32, i32, i32, HWND, *const TPMPARAMS) -> BOOL
    );
    track.expect().returning(|_, _, _, _, _, _| 3);
    assert_eq!(tray_menu(&crate::menu::tests::items()).unwrap(), Some(3));
}
