use super::*;
use shimforge::{Session, mock};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::{HANDLE, HWND};
use windows_sys::core::BOOL;

pub(super) type Store = Arc<Mutex<HashMap<u32, usize>>>;

pub(super) fn formats(session: &mut Session) {
    let register = mock!(
        session,
        RegisterClipboardFormatW,
        unsafe extern "system" fn(*const u16) -> u32
    );
    register.expect().returning(|name| {
        let mut length = 0;
        // SAFETY: the API receives a NUL-terminated name from the adapter.
        let name = unsafe {
            while *name.add(length) != 0 {
                length += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(name, length))
        };
        match name.as_str() {
            "FileNameW" => 0xc001,
            "HTML Format" => 0xc002,
            "custom/binary" => 0xc003,
            "PNG" => 0xc004,
            _ => 0,
        }
    });
}

#[test]
fn named_binary_formats_preserve_utf16_nuls_and_all_bytes() {
    let store = Store::default();
    let mut session = Session::new();
    fake(&mut session, &store, true);
    formats(&mut session);
    assert!(read_clipboard_buffer("FileNameW").is_empty());
    assert!(!has_clipboard_format("FileNameW"));
    let paths = utf16("C:\\资料\\笔记.md\0D:\\second.md\0");
    write_clipboard_buffer("FileNameW", &paths).unwrap();
    let read = read_clipboard_buffer("FileNameW");
    assert_eq!(&read[..paths.len()], paths);
    assert!(has_clipboard_format("FileNameW"));
    let data = [0, 255, 1, 0, 128, 13, 10, 0];
    write_clipboard_buffer("custom/binary", &data).unwrap();
    assert_eq!(&read_clipboard_buffer("custom/binary")[..data.len()], data);
    write_clipboard_buffer("custom/binary", &[]).unwrap();
    assert!(read_clipboard_buffer("custom/binary").is_empty());
    assert!(has_clipboard_format("custom/binary"));
    for name in ["", "FileNameW\0other", "unknown"] {
        assert!(read_clipboard_buffer(name).is_empty());
        assert!(!has_clipboard_format(name));
        assert!(write_clipboard_buffer(name, &data).is_err());
    }
}

#[test]
fn locked_clipboard_writes_release_untransferred_allocations() {
    let store = Store::default();
    let mut session = Session::new();
    fake(&mut session, &store, true);
    formats(&mut session);
    let alloc = mock!(
        session,
        GlobalAlloc,
        unsafe extern "system" fn(u32, usize) -> *mut c_void
    );
    alloc
        .expect()
        .once()
        .returning(|_, _| 0x1234_usize as *mut c_void);
    let lock = mock!(
        session,
        GlobalLock,
        unsafe extern "system" fn(*mut c_void) -> *mut c_void
    );
    lock.expect().returning(|_| ptr::null_mut());
    let frees = mock!(
        session,
        GlobalFree,
        unsafe extern "system" fn(*mut c_void) -> *mut c_void
    );
    frees.expect().once().returning(|handle| {
        assert_eq!(handle as usize, 0x1234);
        ptr::null_mut()
    });
    let error = write_clipboard_buffer("FileNameW", b"data").unwrap_err();
    assert!(error.message.starts_with("GlobalLock failed"));
}

fn owner_mocks(session: &mut Session, events: &Arc<Mutex<Vec<&'static str>>>) {
    let create = mock!(
        session,
        CreateWindowExW,
        unsafe extern "system" fn(
            u32,
            *const u16,
            *const u16,
            u32,
            i32,
            i32,
            i32,
            i32,
            HWND,
            *mut c_void,
            *mut c_void,
            *const c_void,
        ) -> HWND
    );
    let seen = events.clone();
    create.expect().returning(
        move |_, _, _, style, _, _, width, height, parent, _, _, _| {
            assert_eq!(parent, HWND_MESSAGE);
            assert_eq!((style, width, height), (0, 0, 0));
            seen.lock().unwrap().push("create");
            0x1111_usize as HWND
        },
    );
    let destroy = mock!(
        session,
        DestroyWindow,
        unsafe extern "system" fn(HWND) -> BOOL
    );
    let seen = events.clone();
    destroy.expect().returning(move |window| {
        assert_eq!(window as usize, 0x1111);
        assert_eq!(seen.lock().unwrap().last(), Some(&"close"));
        seen.lock().unwrap().push("destroy");
        1
    });
    let open = mock!(
        session,
        OpenClipboard,
        unsafe extern "system" fn(HWND) -> BOOL
    );
    let seen = events.clone();
    open.expect().returning(move |window| {
        assert_eq!(window as usize, 0x1111);
        seen.lock().unwrap().push("open");
        1
    });
    let close = mock!(session, CloseClipboard, unsafe extern "system" fn() -> BOOL);
    let seen = events.clone();
    close.expect().returning(move || {
        seen.lock().unwrap().push("close");
        1
    });
}

#[test]
fn clipboard_writes_have_a_scoped_window_owner_and_check_empty_failures() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut session = Session::new();
    owner_mocks(&mut session, &events);
    formats(&mut session);
    let empty = mock!(session, EmptyClipboard, unsafe extern "system" fn() -> BOOL);
    let seen = events.clone();
    empty.expect().returning(move || {
        seen.lock().unwrap().push("empty");
        1
    });
    let alloc = mock!(
        session,
        GlobalAlloc,
        unsafe extern "system" fn(u32, usize) -> *mut c_void
    );
    let seen = events.clone();
    alloc.expect().returning(move |flags, size| {
        assert_eq!((flags, size), (GMEM_MOVEABLE, 0));
        seen.lock().unwrap().push("allocate");
        0x2222_usize as *mut c_void
    });
    let set = mock!(
        session,
        SetClipboardData,
        unsafe extern "system" fn(u32, HANDLE) -> HANDLE
    );
    let seen = events.clone();
    set.expect().returning(move |format, memory| {
        assert_eq!(format, 0xc001);
        assert_eq!(memory as usize, 0x2222);
        seen.lock().unwrap().push("set");
        memory
    });
    write_clipboard_buffer("FileNameW", &[]).unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        [
            "create", "open", "empty", "allocate", "set", "close", "destroy"
        ]
    );
    drop(session);
    events.lock().unwrap().clear();
    let mut session = Session::new();
    owner_mocks(&mut session, &events);
    let empty = mock!(session, EmptyClipboard, unsafe extern "system" fn() -> BOOL);
    let seen = events.clone();
    empty.expect().returning(move || {
        seen.lock().unwrap().push("empty");
        0
    });
    assert!(
        write_clipboard(None, None)
            .unwrap_err()
            .message
            .starts_with("EmptyClipboard failed")
    );
    assert_eq!(
        *events.lock().unwrap(),
        ["create", "open", "empty", "close", "destroy"]
    );
}

#[test]
fn clipboard_owner_creation_failure_does_not_open_or_empty_clipboard() {
    let mut session = Session::new();
    let create = mock!(
        session,
        CreateWindowExW,
        unsafe extern "system" fn(
            u32,
            *const u16,
            *const u16,
            u32,
            i32,
            i32,
            i32,
            i32,
            HWND,
            *mut c_void,
            *mut c_void,
            *const c_void,
        ) -> HWND
    );
    create
        .expect()
        .returning(|_, _, _, _, _, _, _, _, _, _, _, _| ptr::null_mut());
    let open = mock!(
        session,
        OpenClipboard,
        unsafe extern "system" fn(HWND) -> BOOL
    );
    open.expect()
        .returning(|_| panic!("failed owner must not open the clipboard"));
    let empty = mock!(session, EmptyClipboard, unsafe extern "system" fn() -> BOOL);
    empty
        .expect()
        .returning(|| panic!("failed owner must not empty the clipboard"));
    assert!(
        write_clipboard(None, None)
            .unwrap_err()
            .message
            .starts_with("CreateWindowExW failed")
    );
}

/// Replaces the system clipboard with a map from formats to the global
/// memory handles put on it, so tests leave the user's clipboard alone.
pub(super) fn fake(session: &mut Session, store: &Store, opens: bool) {
    let open = mock!(
        session,
        OpenClipboard,
        unsafe extern "system" fn(HWND) -> BOOL
    );
    open.expect().returning(move |_| BOOL::from(opens));
    let close = mock!(session, CloseClipboard, unsafe extern "system" fn() -> BOOL);
    close.expect().returning(|| 1);
    fake_contents(session, store);
}

pub(super) fn fake_contents(session: &mut Session, store: &Store) {
    let emptied = store.clone();
    let empty = mock!(session, EmptyClipboard, unsafe extern "system" fn() -> BOOL);
    empty.expect().returning(move || {
        emptied.lock().unwrap().clear();
        1
    });
    let stored = store.clone();
    let set = mock!(
        session,
        SetClipboardData,
        unsafe extern "system" fn(u32, HANDLE) -> HANDLE
    );
    set.expect().returning(move |format, memory| {
        stored.lock().unwrap().insert(format, memory as usize);
        memory
    });
    let read = store.clone();
    let get = mock!(
        session,
        GetClipboardData,
        unsafe extern "system" fn(u32) -> HANDLE
    );
    get.expect()
        .returning(move |format| read.lock().unwrap().get(&format).copied().unwrap_or(0) as HANDLE);
    let present = store.clone();
    let available = mock!(
        session,
        IsClipboardFormatAvailable,
        unsafe extern "system" fn(u32) -> BOOL
    );
    available
        .expect()
        .returning(move |format| BOOL::from(present.lock().unwrap().contains_key(&format)));
}

#[test]
fn text_and_html_round_trip() {
    let store = Store::default();
    let mut session = Session::new();
    fake(&mut session, &store, true);
    assert_eq!(read_text(), "");
    assert_eq!(read_html(), "");
    assert!(clipboard_formats().is_empty());
    write_clipboard(Some("héllo"), Some("<b>x</b>")).unwrap();
    assert_eq!(read_text(), "héllo");
    assert_eq!(read_html(), "<b>x</b>");
    assert_eq!(clipboard_formats(), ["text/plain", "text/html"]);
    write_clipboard(None, None).unwrap();
    assert!(clipboard_formats().is_empty());
    // Data without fragment offsets, or with offsets past its end, is no HTML.
    for broken in [&b"plain"[..], b"StartFragment:5\r\nEndFragment:500\r\n"] {
        let memory = handle_of(broken);
        store.lock().unwrap().insert(html_format(), memory as usize);
        assert_eq!(read_html(), "");
    }
}

#[test]
fn page_formats_are_read_under_one_open_clipboard() {
    let store = Store::default();
    let mut session = Session::new();
    fake(&mut session, &store, true);
    formats(&mut session);
    write_clipboard(Some(""), Some("<b>资料😀</b>")).unwrap();
    drop(session);
    let mut session = Session::new();
    fake_contents(&mut session, &store);
    formats(&mut session);
    let open = mock!(
        session,
        OpenClipboard,
        unsafe extern "system" fn(HWND) -> BOOL
    );
    open.expect().times(4).returning(|owner| {
        assert!(owner.is_null());
        1
    });
    let close = mock!(session, CloseClipboard, unsafe extern "system" fn() -> BOOL);
    close.expect().times(4).returning(|| 1);
    assert_eq!(
        read_clipboard_contents(),
        Some(vec![
            ("text/plain", "".into()),
            ("text/html", "<b>资料😀</b>".into())
        ])
    );
    store.lock().unwrap().remove(&u32::from(CF_UNICODETEXT));
    assert_eq!(
        read_clipboard_contents(),
        Some(vec![("text/html", "<b>资料😀</b>".into())])
    );
    store
        .lock()
        .unwrap()
        .insert(html_format(), handle_of(&cf_html("")) as usize);
    assert_eq!(
        read_clipboard_contents(),
        Some(vec![("text/html", "".into())])
    );
    store.lock().unwrap().clear();
    assert_eq!(read_clipboard_contents(), Some(vec![]));
}

#[test]
fn unlockable_data_reads_as_empty() {
    let store = Store::default();
    let mut session = Session::new();
    fake(&mut session, &store, true);
    let memory = handle_of(&[104, 0, 0, 0]);
    store
        .lock()
        .unwrap()
        .insert(u32::from(CF_UNICODETEXT), memory as usize);
    let lock = mock!(
        session,
        GlobalLock,
        unsafe extern "system" fn(*mut c_void) -> *mut c_void
    );
    lock.expect().returning(|_| ptr::null_mut());
    assert_eq!(read_text(), "");
    assert_eq!(read_clipboard_contents(), Some(vec![]));
}

pub(super) fn handle_of(bytes: &[u8]) -> *mut c_void {
    // SAFETY: test memory, copied in while locked.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        let pointer = GlobalLock(memory);
        ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
        GlobalUnlock(memory);
        memory
    }
}

#[test]
fn html_carries_byte_offsets() {
    let data = String::from_utf8(cf_html("é")).unwrap();
    let offset = |name: &str| -> usize {
        data.lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap()
            .parse()
            .unwrap()
    };
    assert_eq!(&data[offset("StartFragment:")..offset("EndFragment:")], "é");
    assert!(data[offset("StartHTML:")..offset("EndHTML:")].starts_with("<html>"));
}

#[test]
fn a_busy_clipboard_is_an_error() {
    let store = Store::default();
    let mut session = Session::new();
    fake(&mut session, &store, false);
    assert_eq!(read_text(), "");
    assert_eq!(read_clipboard_contents(), None);
    assert!(
        write_clipboard(Some("x"), None)
            .unwrap_err()
            .message
            .starts_with("OpenClipboard failed")
    );
}

#[test]
fn failed_writes_free_their_memory() {
    let store = Store::default();
    let mut session = Session::new();
    fake(&mut session, &store, true);
    let alloc = mock!(
        session,
        GlobalAlloc,
        unsafe extern "system" fn(u32, usize) -> *mut c_void
    );
    alloc.expect().once().returning(|_, _| ptr::null_mut());
    let error = write_clipboard(Some("x"), None).unwrap_err();
    assert!(error.message.starts_with("GlobalAlloc failed"));
    session.verify();
    drop(session);
    let mut session = Session::new();
    let open = mock!(
        session,
        OpenClipboard,
        unsafe extern "system" fn(HWND) -> BOOL
    );
    open.expect().returning(|_| 1);
    let close = mock!(session, CloseClipboard, unsafe extern "system" fn() -> BOOL);
    close.expect().returning(|| 1);
    let empty = mock!(session, EmptyClipboard, unsafe extern "system" fn() -> BOOL);
    empty.expect().returning(|| 1);
    let set = mock!(
        session,
        SetClipboardData,
        unsafe extern "system" fn(u32, HANDLE) -> HANDLE
    );
    set.expect().returning(|_, _| ptr::null_mut());
    let error = write_clipboard(Some("x"), None).unwrap_err();
    assert!(error.message.starts_with("SetClipboardData failed"));
}
