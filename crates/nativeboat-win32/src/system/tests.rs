use super::*;
use crate::Event;
use crate::tests::support::{record, send, window};
use shimforge::{Session, mock};
use windows_sys::Win32::Foundation::{HINSTANCE, HWND};
use windows_sys::Win32::System::Registry::{HKEY, REG_ROUTINE_FLAGS, REG_VALUE_TYPE};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    MESSAGEBOX_STYLE, SHOW_WINDOW_CMD, WM_POWERBROADCAST, WM_SETTINGCHANGE,
};
use windows_sys::core::PCWSTR;

#[test]
fn known_user_folders_honor_redirection_and_release_shell_memory() {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::UI::Shell::*;
    use windows_sys::core::{GUID, HRESULT, PWSTR};
    let mut session = Session::new();
    let buffer = std::sync::Arc::new(wide("D:\\Redirected\\用户文档"));
    let allocated = buffer.clone();
    let query = mock!(
        session,
        SHGetKnownFolderPath,
        unsafe extern "system" fn(*const GUID, u32, HANDLE, *mut PWSTR) -> HRESULT
    );
    query.expect().returning(move |id, flags, _, output| {
        assert_eq!(flags, KF_FLAG_DONT_VERIFY as u32);
        // SAFETY: the production caller provides readable GUID and writable output.
        unsafe {
            if (*id).data1 == FOLDERID_Downloads.data1 {
                *output = std::ptr::null_mut();
                return -1;
            }
            *output = allocated.as_ptr() as PWSTR;
            if (*id).data1 == FOLDERID_Desktop.data1 {
                -1
            } else {
                0
            }
        }
    });
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let called = count.clone();
    let free = mock!(
        session,
        windows_sys::Win32::System::Com::CoTaskMemFree,
        unsafe extern "system" fn(*const core::ffi::c_void)
    );
    free.expect().returning(move |pointer| {
        assert_eq!(pointer, buffer.as_ptr().cast());
        called.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    for name in ["documents", "music", "pictures", "videos"] {
        assert_eq!(known_folder(name), Some("D:\\Redirected\\用户文档".into()));
    }
    assert_eq!(known_folder("desktop"), None);
    assert_eq!(known_folder("downloads"), None);
    assert_eq!(known_folder("unknown"), None);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 5);
}

fn text(pointer: PCWSTR) -> String {
    // SAFETY: test pointers come from NUL-terminated buffers.
    let length = (0..)
        .take_while(|index| unsafe { *pointer.add(*index) } != 0)
        .count();
    // SAFETY: `length` units precede the terminator.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(pointer, length) })
}

#[test]
fn shell_actions() {
    let mut session = Session::new();
    let execute = mock!(
        session,
        ShellExecuteW,
        unsafe extern "system" fn(
            HWND,
            PCWSTR,
            PCWSTR,
            PCWSTR,
            PCWSTR,
            SHOW_WINDOW_CMD,
        ) -> HINSTANCE
    );
    execute
        .expect()
        .returning(|_, operation, file, parameters, _, _| {
            assert_eq!(text(operation), "open");
            let file = text(file);
            if file == "explorer.exe" {
                assert_eq!(text(parameters), "/select,\"C:\\a.txt\"");
            } else {
                assert!(parameters.is_null());
            }
            (if file == "bad" { 2_usize } else { 42 }) as HINSTANCE
        });
    open_external("https://example.com").unwrap();
    open_path("C:\\a").unwrap();
    show_item_in_folder("C:\\a.txt").unwrap();
    assert!(
        open_external("bad")
            .unwrap_err()
            .message
            .ends_with("error 2")
    );
    let operation = mock!(
        session,
        SHFileOperationW,
        unsafe extern "system" fn(*mut SHFILEOPSTRUCTW) -> i32
    );
    operation.expect().once().returning(|operation| {
        // SAFETY: the operation is live during the call.
        let operation = unsafe { &*operation };
        assert_eq!(operation.wFunc, FO_DELETE);
        assert_eq!(text(operation.pFrom), "C:\\x");
        0
    });
    operation.expect().once().returning(|_| 2);
    operation.expect().once().returning(|operation| {
        // SAFETY: as above.
        unsafe { (*operation).fAnyOperationsAborted = 1 };
        0
    });
    trash("C:\\x").unwrap();
    assert!(trash("C:\\x").unwrap_err().message.ends_with("error 2"));
    assert!(trash("C:\\x").is_err());
    let sound = mock!(
        session,
        MessageBeep,
        unsafe extern "system" fn(MESSAGEBOX_STYLE) -> windows_sys::core::BOOL
    );
    sound.expect().once().returning(|_| 1);
    beep();
}

#[test]
fn system_state() {
    let mut session = Session::new();
    let registry = mock!(
        session,
        RegGetValueW,
        unsafe extern "system" fn(
            HKEY,
            PCWSTR,
            PCWSTR,
            REG_ROUTINE_FLAGS,
            *mut REG_VALUE_TYPE,
            *mut std::ffi::c_void,
            *mut u32,
        ) -> u32
    );
    for (status, value) in [(0, 0_u32), (0, 1), (2, 0)] {
        registry
            .expect()
            .once()
            .returning(move |_, key, name, _, _, data, _| {
                assert!(text(key).ends_with("Personalize"));
                assert_eq!(text(name), "AppsUseLightTheme");
                // SAFETY: the data buffer holds a DWORD.
                unsafe { *data.cast::<u32>() = value };
                status
            });
    }
    assert!(dark_mode());
    assert!(!dark_mode());
    assert!(!dark_mode());
    let power = mock!(
        session,
        GetSystemPowerStatus,
        unsafe extern "system" fn(*mut SYSTEM_POWER_STATUS) -> windows_sys::core::BOOL
    );
    power.expect().once().returning(|status| {
        // SAFETY: the structure is writable.
        unsafe { (*status).ACLineStatus = 0 };
        1
    });
    power.expect().once().returning(|_| 0);
    assert!(on_battery());
    assert!(!on_battery());
    let list = displays();
    assert!(
        list.iter()
            .any(|display| display.primary && display.scale >= 1.0)
    );
    assert!(
        list.iter()
            .all(|display| display.bounds.2 > 0 && display.work_area.3 > 0)
    );
    let _ = cursor_position();
    assert!(idle_seconds() < 100_000_000);
}

#[test]
fn theme_and_power_messages() {
    let events = record();
    let window = window();
    let handle = crate::hwnd(window).unwrap();
    let colors = wide("ImmersiveColorSet");
    let policy = wide("Policy");
    let long = wide(&"x".repeat(80));
    for area in [&colors, &policy, &long] {
        send(handle, WM_SETTINGCHANGE, 0, area.as_ptr() as isize);
    }
    send(handle, WM_SETTINGCHANGE, 0, 0);
    for state in [4, 18, 10] {
        send(handle, WM_POWERBROADCAST, state, 0);
    }
    assert_eq!(
        events.take(),
        [Event::ThemeChanged, Event::Suspend, Event::Resume]
    );
}

#[test]
fn data_is_protected_for_the_user() {
    let secret = protect(b"secret").unwrap();
    assert_ne!(secret, b"secret");
    assert_eq!(unprotect(&secret).unwrap(), b"secret");
    let error = unprotect(b"not protected").unwrap_err();
    assert!(
        error.message.starts_with("CryptUnprotectData failed"),
        "{}",
        error.message
    );
}

#[test]
fn the_computer_is_kept_awake() {
    use std::sync::{Arc, Mutex};
    let mut session = shimforge::Session::new();
    let states = Arc::new(Mutex::new(Vec::new()));
    let record = states.clone();
    let set = shimforge::mock!(
        session,
        SetThreadExecutionState,
        unsafe extern "system" fn(u32) -> u32
    );
    set.expect().returning(move |flags| {
        record.lock().unwrap().push(flags);
        0
    });
    keep_awake(Awake::System);
    keep_awake(Awake::Display);
    keep_awake(Awake::Allow);
    assert_eq!(
        *states.lock().unwrap(),
        [
            ES_CONTINUOUS | ES_SYSTEM_REQUIRED,
            ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED,
            ES_CONTINUOUS
        ]
    );
}
