use super::*;
use crate::tests::support::{record, window as test_window};
use shimforge::{Session, mock};
use std::cell::Cell;
use std::sync::{Arc, Mutex};
use windows_sys::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongW, WS_EX_ACCEPTFILES};

fn target(window: WindowId) -> *mut c_void {
    Box::into_raw(Box::new(DropTarget {
        vtable: &VTABLE,
        references: AtomicU32::new(1),
        window,
        hwnd: crate::hwnd(window).unwrap(),
        paths: RefCell::new(Vec::new()),
        strings: RefCell::new(Vec::new()),
    }))
    .cast()
}

#[repr(C)]
struct Data {
    table: *const DataObjectVtbl,
    query: Cell<HRESULT>,
    get: Cell<HRESULT>,
    tymed: Cell<u32>,
    handle: Cell<usize>,
    reads: Cell<u32>,
}
impl Data {
    fn new() -> Self {
        Self {
            table: &DATA,
            query: Cell::new(S_OK),
            get: Cell::new(S_OK),
            tymed: Cell::new(TYMED_HGLOBAL as u32),
            handle: Cell::new(1),
            reads: Cell::new(0),
        }
    }
    fn pointer(&mut self) -> *mut c_void {
        (self as *mut Self).cast()
    }
}

unsafe extern "system" fn unused_query(
    _: *mut c_void,
    _: *const GUID,
    _: *mut *mut c_void,
) -> HRESULT {
    E_NOINTERFACE
}
unsafe extern "system" fn unused_reference(_: *mut c_void) -> u32 {
    1
}
unsafe extern "system" fn query_data(this: *mut c_void, format: *const FORMATETC) -> HRESULT {
    // SAFETY: the test's Data object and production format remain alive.
    unsafe {
        // The drag offers files only, no text.
        if (*format).cfFormat != CF_HDROP {
            return DV_E_FORMATETC;
        }
        assert_eq!((*format).dwAspect, DVASPECT_CONTENT);
        assert_eq!((*format).lindex, -1);
        (*this.cast::<Data>()).query.get()
    }
}
unsafe extern "system" fn get_data(
    this: *mut c_void,
    _: *const FORMATETC,
    medium: *mut STGMEDIUM,
) -> HRESULT {
    // SAFETY: production passes writable medium storage and a live Data object.
    unsafe {
        let data = &*this.cast::<Data>();
        data.reads.set(data.reads.get() + 1);
        (*medium).tymed = data.tymed.get();
        (*medium).u.hGlobal = data.handle.get() as _;
        data.get.get()
    }
}
static DATA: DataObjectVtbl = DataObjectVtbl {
    base: IUnknown_Vtbl {
        QueryInterface: unused_query,
        AddRef: unused_reference,
        Release: unused_reference,
    },
    get_data,
    get_data_here: 0,
    query_get_data: query_data,
};

fn file_metadata(session: &mut Session) -> Arc<Mutex<u32>> {
    let files = mock!(
        session,
        windows_sys::Win32::UI::Shell::DragQueryFileW,
        unsafe extern "system" fn(HDROP, u32, windows_sys::core::PWSTR, u32) -> u32
    );
    files.expect().returning(|_, index, buffer, capacity| {
        if index == u32::MAX {
            return 2;
        }
        if index == 1 {
            return 0;
        }
        let path: Vec<u16> = "C:\\incoming\\receipt.txt"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        if capacity != 0 {
            // SAFETY: production allocated the queried capacity including NUL.
            unsafe { std::ptr::copy_nonoverlapping(path.as_ptr(), buffer, path.len()) };
        }
        (path.len() - 1) as u32
    });
    let released = Arc::new(Mutex::new(0));
    let count = released.clone();
    let medium = mock!(
        session,
        windows_sys::Win32::System::Ole::ReleaseStgMedium,
        unsafe extern "system" fn(*mut STGMEDIUM)
    );
    medium.expect().returning(move |_| {
        *count.lock().unwrap() += 1;
    });
    let keys = mock!(
        session,
        windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState,
        unsafe extern "system" fn(i32) -> i16
    );
    keys.expect().returns(0);
    released
}

#[test]
fn com_interfaces_reference_counts_and_callback_validation() {
    let window = test_window();
    let target = target(window);
    // SAFETY: callbacks receive this test's owned interface reference.
    unsafe {
        let mut interface = target;
        let unknown = GUID::from_u128(0x1234);
        assert_eq!(query(target, null_mut(), &mut interface), E_POINTER);
        assert_eq!(query(target, &IID_IUnknown, null_mut()), E_POINTER);
        assert_eq!(query(target, &unknown, &mut interface), E_NOINTERFACE);
        assert!(interface.is_null());
        for iid in [IID_IUnknown, IID_DROP_TARGET] {
            assert_eq!(query(target, &iid, &mut interface), S_OK);
            assert_eq!(interface, target);
            assert_eq!(release(interface), 1);
        }
        assert_eq!(add_ref(target), 2);
        assert_eq!(release(target), 1);
        assert_eq!(
            enter(target, null_mut(), 0, POINTL::default(), null_mut()),
            E_INVALIDARG
        );
        assert_eq!(over(target, 0, POINTL::default(), null_mut()), E_INVALIDARG);
        assert_eq!(
            dropped(target, null_mut(), 0, POINTL::default(), null_mut()),
            E_INVALIDARG
        );
        assert_eq!(leave(target), S_OK);
        assert_eq!(release(target), 0);
    }
    crate::destroy(window);
}

#[test]
fn shell_metadata_media_failures_release_only_successful_acquisitions() {
    let mut session = Session::new();
    let released = file_metadata(&mut session);
    let mut data = Data::new();
    // SAFETY: Data exposes the test vtable; metadata handles are mocked.
    unsafe {
        assert!(paths(null_mut()).is_empty());
        assert_eq!(paths(data.pointer()), ["C:\\incoming\\receipt.txt"]);
        assert_eq!(*released.lock().unwrap(), 1);
        data.query.set(E_FAIL);
        assert!(paths(data.pointer()).is_empty());
        assert_eq!(data.reads.get(), 1);
        data.query.set(S_OK);
        data.get.set(E_FAIL);
        assert!(paths(data.pointer()).is_empty());
        assert_eq!(*released.lock().unwrap(), 1);
        data.get.set(S_OK);
        data.tymed.set(2);
        assert!(paths(data.pointer()).is_empty());
        data.tymed.set(TYMED_HGLOBAL as u32);
        data.handle.set(0);
        assert!(paths(data.pointer()).is_empty());
        assert_eq!(*released.lock().unwrap(), 3);
    }
}

#[test]
fn hover_effects_coordinates_modifiers_leave_and_final_drop() {
    let window = test_window();
    let target = target(window);
    let events = record();
    events.cancel.set(true);
    let mut session = Session::new();
    file_metadata(&mut session);
    let screen = mock!(
        &mut session,
        windows_sys::Win32::Graphics::Gdi::ScreenToClient,
        unsafe extern "system" fn(HWND, *mut POINT) -> windows_sys::core::BOOL
    );
    screen.expect().returning(|_, point| {
        // SAFETY: production passes writable POINT storage.
        unsafe { *point = POINT { x: 20, y: 40 } };
        1
    });
    window::STATE.with_borrow_mut(|state| state.windows.get_mut(&window).unwrap().scale = 2.0);
    let mut data = Data::new();
    let mut effect = 3;
    // SAFETY: the owned target remains alive; effect and Data are valid storage.
    unsafe {
        assert_eq!(
            enter(
                target,
                data.pointer(),
                MK_CONTROL | MK_SHIFT | MK_ALT,
                POINTL::default(),
                &mut effect
            ),
            S_OK
        );
        assert_eq!(effect, DROPEFFECT_COPY);
        assert_eq!(
            events.take(),
            [Event::FileDrag {
                kind: FileDragKind::Enter,
                paths: vec!["C:\\incoming\\receipt.txt".into()],
                strings: Vec::new(),
                x: 10.0,
                y: 20.0,
                modifiers: Modifiers {
                    ctrl: true,
                    shift: true,
                    alt: true,
                    meta: false
                },
            }]
        );
        effect = 3;
        assert_eq!(over(target, 0, POINTL::default(), &mut effect), S_OK);
        assert_eq!(effect, 1);
        assert_eq!(data.reads.get(), 1, "hover never reacquires data");
        assert!(matches!(
            events.take().as_slice(),
            [Event::FileDrag {
                kind: FileDragKind::Over,
                ..
            }]
        ));
        events.cancel.set(false);
        effect = 3;
        over(target, 0, POINTL::default(), &mut effect);
        assert_eq!(effect, 0);
        events.take();
        events.cancel.set(true);
        effect = 2;
        over(target, 0, POINTL::default(), &mut effect);
        assert_eq!(effect, 0, "incoming files cannot move source files");
        events.take();
        effect = 3;
        dropped(target, data.pointer(), 0, POINTL::default(), &mut effect);
        assert_eq!(effect, 1);
        assert!(matches!(
            events.take().as_slice(),
            [
                Event::FileDrag {
                    kind: FileDragKind::Over,
                    ..
                },
                Event::FilesDropped { .. }
            ]
        ));
        effect = 3;
        over(target, 0, POINTL::default(), &mut effect);
        assert_eq!(effect, 0);
        assert!(events.take().is_empty());
        effect = 3;
        enter(target, data.pointer(), 0, POINTL::default(), &mut effect);
        events.take();
        leave(target);
        assert!(matches!(
            events.take().as_slice(),
            [Event::FileDrag {
                kind: FileDragKind::Leave,
                ..
            }]
        ));
        effect = 3;
        enter(target, data.pointer(), 0, POINTL::default(), &mut effect);
        events.take();
        events.cancel.set(false);
        effect = 3;
        dropped(target, data.pointer(), 0, POINTL::default(), &mut effect);
        assert_eq!(effect, 0);
        assert!(matches!(
            events.take().as_slice(),
            [
                Event::FileDrag {
                    kind: FileDragKind::Over,
                    ..
                },
                Event::FileDrag {
                    kind: FileDragKind::Leave,
                    ..
                }
            ]
        ));
        effect = 3;
        enter(target, null_mut(), 0, POINTL::default(), &mut effect);
        assert_eq!(effect, 0);
        leave(target);
        assert!(events.take().is_empty());
        assert_eq!(release(target), 0);
    }
    session.restore();
    crate::destroy(window);
}

#[test]
fn failed_ole_registration_preserves_legacy_shell_drop_support() {
    let mut session = Session::new();
    let registration = mock!(
        &mut session,
        windows_sys::Win32::System::Ole::RegisterDragDrop,
        unsafe extern "system" fn(HWND, *mut c_void) -> HRESULT
    );
    registration.expect().returns(E_FAIL);
    let window = test_window();
    assert_ne!(
        // SAFETY: inspecting the test window's style.
        unsafe { GetWindowLongW(crate::hwnd(window).unwrap(), GWL_EXSTYLE) } as u32
            & WS_EX_ACCEPTFILES,
        0
    );
    session.restore();
    crate::destroy(window);
}

#[test]
fn a_page_can_close_its_window_during_an_ole_callback() {
    struct Closing;
    impl crate::Handler for Closing {
        fn event(&mut self, window: WindowId, event: Event) -> bool {
            if matches!(
                event,
                Event::FileDrag {
                    kind: FileDragKind::Enter,
                    ..
                }
            ) {
                crate::destroy(window);
                true
            } else {
                false
            }
        }
        fn idle(&mut self) -> Option<std::time::Duration> {
            None
        }
    }
    let mut session = Session::new();
    let registered = Arc::new(Mutex::new(0_usize));
    let captured = registered.clone();
    let registration = mock!(
        &mut session,
        windows_sys::Win32::System::Ole::RegisterDragDrop,
        unsafe extern "system" fn(HWND, *mut c_void) -> HRESULT
    );
    registration.expect().returning(move |_, target| {
        // SAFETY: model OLE's owning reference acquired during registration.
        unsafe { add_ref(target) };
        *captured.lock().unwrap() = target as usize;
        S_OK
    });
    let captured = registered.clone();
    let revocation = mock!(
        &mut session,
        windows_sys::Win32::System::Ole::RevokeDragDrop,
        unsafe extern "system" fn(HWND) -> HRESULT
    );
    revocation.expect().returning(move |_| {
        let target = std::mem::take(&mut *captured.lock().unwrap()) as *mut c_void;
        // SAFETY: release only OLE's reference, leaving the callback guard alive.
        assert_eq!(unsafe { release(target) }, 1);
        S_OK
    });
    file_metadata(&mut session);
    let window = test_window();
    let target = *registered.lock().unwrap() as *mut c_void;
    crate::set_handler(Box::new(Closing));
    let mut data = Data::new();
    let mut effect = 3;
    assert_eq!(
        // SAFETY: the callback's guard retains the target until its return,
        // despite revocation and destruction within the page's event handler.
        unsafe { enter(target, data.pointer(), 0, POINTL::default(), &mut effect) },
        S_OK
    );
    assert_eq!(effect, 0, "a destroyed window cannot accept the drag");
    assert_eq!(*registered.lock().unwrap(), 0);
    assert!(crate::hwnd(window).is_none());
}
