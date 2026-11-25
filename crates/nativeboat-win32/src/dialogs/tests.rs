use super::*;
use crate::tests::support::window;
use shimforge::{Session, mock};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::{FARPROC, HMODULE};
use windows_sys::Win32::UI::Shell::Common::ITEMIDLIST;
use windows_sys::core::{PCSTR, PCWSTR};

fn text(pointer: PCWSTR) -> String {
    // SAFETY: test pointers come from NUL-terminated buffers.
    let length = (0..)
        .take_while(|index| unsafe { *pointer.add(*index) } != 0)
        .count();
    // SAFETY: `length` units precede the terminator.
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(pointer, length) })
}

fn write(target: *mut u16, units: &[u16]) {
    // SAFETY: the dialogs' buffers are far larger than the test paths.
    unsafe { ptr::copy_nonoverlapping(units.as_ptr(), target, units.len()) };
}

#[test]
fn chosen_paths() {
    let units = |text: &str| -> Vec<u16> { text.encode_utf16().collect() };
    assert_eq!(chosen(&units("C:\\a\\b.txt\0\0")), ["C:\\a\\b.txt"]);
    assert_eq!(
        chosen(&units("C:\\dir\0a.txt\0b.txt\0\0")),
        ["C:\\dir\\a.txt", "C:\\dir\\b.txt"]
    );
    assert_eq!(
        chosen(&units("C:\\\0a.txt\0b.txt\0\0")),
        ["C:\\a.txt", "C:\\b.txt"]
    );
    assert!(chosen(&units("\0")).is_empty());
    let list = filter_list(&[FileFilter {
        name: "Images".to_owned(),
        extensions: vec!["png".to_owned(), "jpg".to_owned()],
    }]);
    assert_eq!(String::from_utf16_lossy(&list), "Images\0*.png;*.jpg\0\0");
    assert!(owner(Some(u64::MAX)).is_null() && owner(None).is_null());
    assert!(!owner(Some(window())).is_null());
}

#[test]
fn open_and_save_dialogs() {
    let mut session = Session::new();
    let open = mock!(
        session,
        GetOpenFileNameW,
        unsafe extern "system" fn(*mut OPENFILENAMEW) -> BOOL
    );
    open.expect().once().returning(|dialog| {
        // SAFETY: the dialog structure is live during the call.
        let dialog = unsafe { &*dialog };
        assert_ne!(dialog.Flags & OFN_ALLOWMULTISELECT, 0);
        assert_ne!(dialog.Flags & OFN_FORCESHOWHIDDEN, 0);
        assert_eq!(text(dialog.lpstrTitle), "Pick");
        assert!(dialog.lpstrDefExt.is_null());
        let units: Vec<u16> = "C:\\dir\0a.txt\0b.txt\0\0".encode_utf16().collect();
        write(dialog.lpstrFile, &units);
        1
    });
    open.expect().once().returning(|dialog| {
        // SAFETY: as above.
        let dialog = unsafe { &*dialog };
        // Without file types, all files, as Electron offers them.
        assert!(dialog.lpstrTitle.is_null());
        let mut end = 0;
        // SAFETY: the list ends with two nulls.
        while unsafe { *dialog.lpstrFilter.add(end) != 0 || *dialog.lpstrFilter.add(end + 1) != 0 }
        {
            end += 1;
        }
        // SAFETY: as measured.
        let list = unsafe { core::slice::from_raw_parts(dialog.lpstrFilter, end + 1) };
        assert_eq!(String::from_utf16_lossy(list), "All Files (*.*)\0*.*\0");
        assert!(!dialog.lpstrInitialDir.is_null());
        0
    });
    let options = FileDialog {
        title: "Pick".to_owned(),
        filters: vec![FileFilter {
            name: "All".to_owned(),
            extensions: vec!["*".to_owned()],
        }],
        multiple: true,
        show_hidden: true,
        ..FileDialog::default()
    };
    assert_eq!(
        file_dialog(None, &options),
        Some(vec![
            "C:\\dir\\a.txt".to_owned(),
            "C:\\dir\\b.txt".to_owned()
        ])
    );
    let folder = std::env::temp_dir().to_string_lossy().into_owned();
    let cancelled = FileDialog {
        default_path: folder,
        ..FileDialog::default()
    };
    assert_eq!(file_dialog(Some(window()), &cancelled), None);
    let save = mock!(
        session,
        GetSaveFileNameW,
        unsafe extern "system" fn(*mut OPENFILENAMEW) -> BOOL
    );
    save.expect().returning(|dialog| {
        // SAFETY: as above.
        let dialog = unsafe { &*dialog };
        assert_ne!(dialog.Flags & OFN_OVERWRITEPROMPT, 0);
        assert_eq!(text(dialog.lpstrDefExt), "png");
        assert_eq!(text(dialog.lpstrFile), "C:\\out\\pic");
        1
    });
    let saved = FileDialog {
        default_path: "C:\\out\\pic".to_owned(),
        filters: vec![FileFilter {
            name: "PNG".to_owned(),
            extensions: vec!["png".to_owned()],
        }],
        save: true,
        ..FileDialog::default()
    };
    assert_eq!(
        file_dialog(None, &saved),
        Some(vec!["C:\\out\\pic".to_owned()])
    );
}

#[test]
fn folder_dialogs() {
    let mut session = Session::new();
    let browse = mock!(
        session,
        SHBrowseForFolderW,
        unsafe extern "system" fn(*const BROWSEINFOW) -> *mut ITEMIDLIST
    );
    browse.expect().once().returning(|_| ptr::null_mut());
    browse.expect().returning(|info| {
        // SAFETY: the structure is live during the call.
        assert_eq!(text(unsafe { (*info).lpszTitle }), "Folder");
        16 as *mut ITEMIDLIST
    });
    let path = mock!(
        session,
        SHGetPathFromIDListW,
        unsafe extern "system" fn(*const ITEMIDLIST, *mut u16) -> BOOL
    );
    path.expect().once().returning(|_, buffer| {
        let units: Vec<u16> = "C:\\folder\0".encode_utf16().collect();
        write(buffer, &units);
        1
    });
    path.expect().returning(|_, _| 0);
    let free = mock!(
        session,
        CoTaskMemFree,
        unsafe extern "system" fn(*const std::ffi::c_void)
    );
    free.expect().returning(|_| {});
    let options = FileDialog {
        title: "Folder".to_owned(),
        directory: true,
        ..FileDialog::default()
    };
    assert_eq!(file_dialog(None, &options), None);
    assert_eq!(
        file_dialog(None, &options),
        Some(vec!["C:\\folder".to_owned()])
    );
    assert_eq!(file_dialog(None, &options), None);
}

unsafe extern "system" fn chooses_next(
    config: *const TASKDIALOGCONFIG,
    button: *mut i32,
    _: *mut i32,
    checked: *mut BOOL,
) -> HRESULT {
    // SAFETY: the configuration is live; it is packed, so it is copied.
    let config = unsafe { ptr::read_unaligned(config) };
    // SAFETY: the outputs are writable.
    unsafe {
        *button = config.nDefaultButton + 1;
        *checked = BOOL::from(config.dwFlags & TDF_VERIFICATION_FLAG_CHECKED != 0);
    }
    0
}

unsafe extern "system" fn fails(
    _: *const TASKDIALOGCONFIG,
    _: *mut i32,
    _: *mut i32,
    _: *mut BOOL,
) -> HRESULT {
    -1
}

fn export(function: TaskDialogIndirect) -> FARPROC {
    // SAFETY: the mocked lookup returns the function under the generic
    // export type, as GetProcAddress does.
    Some(unsafe {
        std::mem::transmute::<TaskDialogIndirect, unsafe extern "system" fn() -> isize>(function)
    })
}

#[test]
fn task_dialogs() {
    let mut session = Session::new();
    let lookup = mock!(
        session,
        GetProcAddress,
        unsafe extern "system" fn(HMODULE, PCSTR) -> FARPROC
    );
    lookup.expect().returning(|_, _| export(chooses_next));
    let options = MessageBox {
        kind: MessageKind::Warning,
        title: "Save?".to_owned(),
        message: "Save changes?".to_owned(),
        detail: "They are lost otherwise.".to_owned(),
        buttons: vec![
            "Save".to_owned(),
            "Don't Save".to_owned(),
            "Cancel".to_owned(),
        ],
        default_button: Some(0),
        cancel_button: Some(2),
        checkbox: Some("Remember".to_owned()),
        checked: true,
    };
    assert_eq!(message_box(Some(window()), &options), (1, true));
    for kind in [
        MessageKind::None,
        MessageKind::Info,
        MessageKind::Error,
        MessageKind::Question,
    ] {
        let single = MessageBox {
            kind,
            ..MessageBox::default()
        };
        // The only button is `OK`; an id past it counts as cancelling.
        assert_eq!(message_box(None, &single), (0, false));
    }
}

#[test]
fn message_boxes_without_task_dialogs() {
    let mut session = Session::new();
    let lookup = mock!(
        session,
        GetProcAddress,
        unsafe extern "system" fn(HMODULE, PCSTR) -> FARPROC
    );
    lookup.expect().once().returning(|_, _| export(fails));
    lookup.expect().returning(|_, _| None);
    let answers = [IDOK, IDNO, IDCANCEL, IDCANCEL, IDCANCEL, IDYES];
    let next = Arc::new(AtomicUsize::new(0));
    let box_mock = mock!(
        session,
        MessageBoxW,
        unsafe extern "system" fn(HWND, PCWSTR, PCWSTR, u32) -> i32
    );
    box_mock.expect().returning(move |_, text_pointer, _, _| {
        let index = next.fetch_add(1, Ordering::SeqCst);
        if index == 0 {
            assert_eq!(text(text_pointer), "Message\n\nDetail");
        }
        answers[index]
    });
    let with = |count: usize, kind: MessageKind| MessageBox {
        kind,
        message: "Message".to_owned(),
        detail: if count == 1 {
            "Detail".to_owned()
        } else {
            String::new()
        },
        buttons: (0..count).map(|index| format!("B{index}")).collect(),
        cancel_button: Some(count.saturating_sub(1)),
        checked: true,
        ..MessageBox::default()
    };
    assert_eq!(message_box(None, &with(1, MessageKind::Info)), (0, true));
    assert_eq!(message_box(None, &with(3, MessageKind::Warning)).0, 1);
    assert_eq!(message_box(None, &with(2, MessageKind::Error)).0, 1);
    assert_eq!(message_box(None, &with(3, MessageKind::Question)).0, 2);
    assert_eq!(message_box(None, &with(1, MessageKind::None)).0, 0);
    assert_eq!(message_box(None, &with(0, MessageKind::None)).0, 0);
}

#[test]
fn error_boxes() {
    let mut session = Session::new();
    let box_mock = mock!(
        session,
        MessageBoxW,
        unsafe extern "system" fn(HWND, PCWSTR, PCWSTR, u32) -> i32
    );
    box_mock
        .expect()
        .once()
        .returning(|owner, content, title, style| {
            assert!(owner.is_null());
            assert_eq!(
                (text(title), text(content)),
                ("Oops".to_owned(), "Broken".to_owned())
            );
            assert_ne!(style & MB_ICONERROR, 0);
            IDOK
        });
    error_box("Oops", "Broken");
}
