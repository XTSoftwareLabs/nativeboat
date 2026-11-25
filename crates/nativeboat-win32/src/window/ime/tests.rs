use super::*;
use crate::tests::support::{record, send, window};
use nativeboat_ir::Rect;
use shimforge::{Session, mock};
use std::sync::{Arc, Mutex};

struct MockState {
    available: bool,
    preedit: Vec<u16>,
    result: Vec<u16>,
    cursor: i32,
    query: Option<i32>,
    copied: Option<i32>,
    acquired: usize,
    released: usize,
    canceled: usize,
    completed: usize,
    notify_window: Option<usize>,
    forms: Vec<(u32, i32, i32, i32, i32)>,
}
impl Default for MockState {
    fn default() -> Self {
        Self {
            available: true,
            preedit: "ni".encode_utf16().collect(),
            result: "你好😀".encode_utf16().collect(),
            cursor: 1,
            query: None,
            copied: None,
            acquired: 0,
            released: 0,
            canceled: 0,
            completed: 0,
            notify_window: None,
            forms: Vec::new(),
        }
    }
}

fn mocks() -> (Session, Arc<Mutex<MockState>>) {
    let mut session = Session::new();
    let state = Arc::new(Mutex::new(MockState::default()));
    let log = state.clone();
    mock!(
        session,
        ImmGetContext,
        unsafe extern "system" fn(HWND) -> HIMC
    )
    .expect()
    .returning(move |_| {
        let mut log = log.lock().unwrap();
        if log.available {
            log.acquired += 1;
            99 as HIMC
        } else {
            null_mut()
        }
    });
    let log = state.clone();
    mock!(
        session,
        ImmReleaseContext,
        unsafe extern "system" fn(HWND, HIMC) -> i32
    )
    .expect()
    .returning(move |_, context| {
        assert_eq!(context as usize, 99);
        log.lock().unwrap().released += 1;
        1
    });
    let log = state.clone();
    mock!(
        session,
        ImmGetCompositionStringW,
        unsafe extern "system" fn(HIMC, u32, *mut c_void, u32) -> i32
    )
    .expect()
    .returning(move |context, kind, output, bytes| {
        assert_eq!(context as usize, 99);
        let log = log.lock().unwrap();
        if kind == GCS_CURSORPOS {
            assert!(output.is_null());
            assert_eq!(bytes, 0);
            return log.cursor;
        }
        let units = if kind == GCS_COMPSTR {
            &log.preedit
        } else {
            assert_eq!(kind, GCS_RESULTSTR);
            &log.result
        };
        if output.is_null() {
            return log.query.unwrap_or((units.len() * 2) as i32);
        }
        assert_eq!(bytes as usize, units.len() * 2);
        // SAFETY: production queried the unit count and supplied that capacity.
        unsafe { std::ptr::copy_nonoverlapping(units.as_ptr(), output.cast(), units.len()) };
        log.copied.unwrap_or(bytes as i32)
    });
    let log = state.clone();
    mock!(
        session,
        ImmNotifyIME,
        unsafe extern "system" fn(HIMC, u32, u32, u32) -> i32
    )
    .expect()
    .returning(move |context, action, index, value| {
        assert_eq!(
            (context as usize, action, value),
            (99, NI_COMPOSITIONSTR, 0)
        );
        let target = {
            let mut log = log.lock().unwrap();
            if index == CPS_CANCEL {
                log.canceled += 1;
                None
            } else {
                assert_eq!(index, CPS_COMPLETE);
                log.completed += 1;
                log.notify_window
            }
        };
        if let Some(target) = target {
            send(
                target as HWND,
                WM_IME_COMPOSITION,
                0,
                GCS_RESULTSTR as isize,
            );
            send(target as HWND, WM_IME_ENDCOMPOSITION, 0, 0);
        }
        1
    });
    let log = state.clone();
    mock!(
        session,
        ImmSetCompositionWindow,
        unsafe extern "system" fn(HIMC, *const COMPOSITIONFORM) -> i32
    )
    .expect()
    .returning(move |context, form| {
        assert_eq!(context as usize, 99);
        // SAFETY: production passes a live initialized form.
        let form = unsafe { &*form };
        log.lock().unwrap().forms.push((
            form.dwStyle,
            form.ptCurrentPos.x,
            form.ptCurrentPos.y,
            0,
            0,
        ));
        1
    });
    let log = state.clone();
    mock!(
        session,
        ImmSetCandidateWindow,
        unsafe extern "system" fn(HIMC, *const CANDIDATEFORM) -> i32
    )
    .expect()
    .returning(move |context, form| {
        assert_eq!(context as usize, 99);
        // SAFETY: production passes a live initialized form.
        let form = unsafe { &*form };
        assert_eq!(form.dwIndex, 0);
        log.lock().unwrap().forms.push((
            form.dwStyle,
            form.ptCurrentPos.x,
            form.ptCurrentPos.y,
            form.rcArea.right,
            form.rcArea.bottom,
        ));
        1
    });
    (session, state)
}

fn event(kind: CompositionKind, data: &str, cursor: usize) -> Event {
    Event::Composition {
        kind,
        data: data.into(),
        cursor,
    }
}

#[test]
fn absent_windows_have_no_composition() {
    assert!(!active(u64::MAX));
    position(u64::MAX);
    assert_eq!(crate::complete_composition(u64::MAX), None);
}

#[test]
fn completing_synchronously_preserves_converted_candidates_before_dom_focus_moves() {
    let id = window();
    let root = super::super::hwnd(id).unwrap();
    let events = record();
    let (mut session, state) = mocks();
    assert_eq!(crate::complete_composition(id), None);
    assert_eq!(
        handle(u64::MAX, root, WM_IME_STARTCOMPOSITION, 0, 0),
        Some(0)
    );
    events.take();
    state.lock().unwrap().notify_window = Some(root as usize);
    for synchronous in [false, true] {
        send(root, WM_IME_STARTCOMPOSITION, 0, 0);
        send(root, WM_IME_COMPOSITION, 0, GCS_COMPSTR as isize);
        events.take();
        if synchronous {
            assert_eq!(crate::complete_composition(id).as_deref(), Some("你好😀"));
            assert!(events.take().is_empty(), "result delivered directly to DOM");
        } else {
            complete(id, root);
            assert_eq!(
                events.take(),
                [
                    event(CompositionKind::Update, "你好😀", 4),
                    event(CompositionKind::End, "你好😀", 4)
                ]
            );
        }
        assert!(!active(id));
        assert!(!STATE.with_borrow(|all| all.windows[&id].ime.resolving));
    }
    state.lock().unwrap().notify_window = None;
    send(root, WM_IME_STARTCOMPOSITION, 0, 0);
    send(root, WM_IME_COMPOSITION, 0, GCS_COMPSTR as isize);
    events.take();
    assert_eq!(crate::complete_composition(id).as_deref(), Some("ni"));
    assert!(events.take().is_empty());
    let log = state.lock().unwrap();
    assert_eq!(log.acquired, log.released);
    drop(log);
    session.restore();
    super::super::destroy(id);
}

#[test]
fn native_messages_replace_preedit_commit_once_and_suppress_duplicate_characters() {
    let id = window();
    let root = super::super::hwnd(id).unwrap();
    let events = record();
    let (mut session, state) = mocks();
    send(root, WM_IME_STARTCOMPOSITION, 0, 0);
    send(root, WM_IME_STARTCOMPOSITION, 0, 0);
    assert_eq!(events.take(), [event(CompositionKind::Start, "", 0)]);
    send(root, WM_CHAR, 65, 0);
    send(root, WM_CHAR, 0xd83d, 0);
    send(root, WM_CHAR, 0xde00, 0);
    assert!(events.take().is_empty());
    send(root, WM_IME_COMPOSITION, 0, GCS_COMPSTR as isize);
    assert_eq!(events.take(), [event(CompositionKind::Update, "ni", 1)]);
    state.lock().unwrap().cursor = 2;
    send(root, WM_IME_COMPOSITION, 0, GCS_CURSORPOS as isize);
    assert_eq!(events.take(), [event(CompositionKind::Update, "ni", 2)]);
    send(root, WM_IME_COMPOSITION, 0, GCS_RESULTSTR as isize);
    assert_eq!(
        events.take(),
        [
            event(CompositionKind::Update, "你好😀", 4),
            event(CompositionKind::End, "你好😀", 4)
        ]
    );
    assert!(!active(id));
    send(root, WM_IME_ENDCOMPOSITION, 0, 0);
    assert!(events.take().is_empty());
    send(root, WM_CHAR, 88, 0);
    assert_eq!(events.take(), [Event::Text("X".into())]);
    let log = state.lock().unwrap();
    assert_eq!(log.acquired, log.released);
    drop(log);
    session.restore();
    super::super::destroy(id);
}

#[test]
fn results_can_start_a_session_and_continue_another_preedit_in_one_message() {
    let id = window();
    let root = super::super::hwnd(id).unwrap();
    let events = record();
    let (mut session, state) = mocks();
    state.lock().unwrap().cursor = -1;
    send(
        root,
        WM_IME_COMPOSITION,
        0,
        (GCS_RESULTSTR | GCS_COMPSTR) as isize,
    );
    assert_eq!(
        events.take(),
        [
            event(CompositionKind::Start, "", 0),
            event(CompositionKind::Update, "你好😀", 4),
            event(CompositionKind::End, "你好😀", 4),
            event(CompositionKind::Start, "", 0),
            event(CompositionKind::Update, "ni", 2)
        ]
    );
    state.lock().unwrap().cursor = -1;
    send(root, WM_IME_COMPOSITION, 0, GCS_CURSORPOS as isize);
    assert!(events.take().is_empty());
    send(root, WM_IME_COMPOSITION, 0, GCS_COMPATTR as isize);
    assert!(events.take().is_empty());
    send(root, WM_IME_COMPOSITION, 0, 0);
    assert_eq!(events.take(), [event(CompositionKind::End, "", 0)]);
    send(root, WM_IME_STARTCOMPOSITION, 0, 0);
    events.take();
    send(root, WM_IME_ENDCOMPOSITION, 0, 0);
    assert_eq!(events.take(), [event(CompositionKind::End, "", 0)]);
    session.restore();
    super::super::destroy(id);
}

#[test]
fn canceled_start_and_lost_focus_end_composition_and_balance_contexts() {
    let id = window();
    let root = super::super::hwnd(id).unwrap();
    let events = record();
    let (mut session, state) = mocks();
    events.cancel.set(true);
    send(root, WM_IME_STARTCOMPOSITION, 0, 0);
    assert_eq!(
        events.take(),
        [
            event(CompositionKind::Start, "", 0),
            event(CompositionKind::End, "", 0)
        ]
    );
    send(
        root,
        WM_IME_COMPOSITION,
        0,
        (GCS_COMPSTR | GCS_RESULTSTR) as isize,
    );
    assert_eq!(state.lock().unwrap().canceled, 3);
    assert!(!active(id));
    events.take();
    events.cancel.set(false);
    send(root, WM_IME_STARTCOMPOSITION, 0, 0);
    send(root, WM_IME_COMPOSITION, 0, GCS_COMPSTR as isize);
    events.take();
    send(root, WM_KILLFOCUS, 0, 0);
    assert_eq!(events.take(), [event(CompositionKind::End, "ni", 2)]);
    assert_eq!(state.lock().unwrap().completed, 1);
    complete(id, root);
    assert!(events.take().is_empty());
    send(root, WM_IME_STARTCOMPOSITION, 0, 0);
    events.take();
    state.lock().unwrap().available = false;
    complete(id, root);
    assert_eq!(events.take(), [event(CompositionKind::End, "", 0)]);
    assert!(Context::of(root).is_none());
    assert_eq!(handle(id, root, WM_IME_STARTCOMPOSITION, 0, 0), None);
    assert_eq!(
        handle(id, root, WM_IME_COMPOSITION, 0, GCS_COMPSTR as isize),
        None
    );
    let log = state.lock().unwrap();
    assert_eq!(log.acquired, log.released);
    drop(log);
    session.restore();
    super::super::destroy(id);
}

#[test]
fn composition_buffers_use_byte_lengths_and_reject_failed_or_odd_results() {
    let id = window();
    let root = super::super::hwnd(id).unwrap();
    let (mut session, state) = mocks();
    for (query, copied, expected) in [
        (Some(-1), None, None),
        (Some(3), None, None),
        (Some(0), None, Some("")),
        (None, Some(-1), None),
        (None, Some(3), None),
        (None, Some(6), Some("ni")),
        (None, Some(2), Some("n")),
        (None, Some(0), Some("")),
    ] {
        {
            let mut state = state.lock().unwrap();
            state.query = query;
            state.copied = copied;
        }
        let context = Context::of(root).unwrap();
        assert_eq!(context.text(GCS_COMPSTR).as_deref(), expected);
    }
    state.lock().unwrap().query = Some(-1);
    assert_eq!(
        handle(id, root, WM_IME_COMPOSITION, 0, GCS_RESULTSTR as isize),
        None
    );
    assert_eq!(
        handle(id, root, WM_IME_COMPOSITION, 0, GCS_COMPSTR as isize),
        None
    );
    let log = state.lock().unwrap();
    assert_eq!(log.acquired, log.released);
    drop(log);
    session.restore();
    super::super::destroy(id);
}

#[test]
fn candidate_forms_follow_caret_scroll_dpi_and_surface_client_coordinates() {
    let id = window();
    let root = super::super::hwnd(id).unwrap();
    let (mut session, state) = mocks();
    STATE.with_borrow_mut(|all| {
        let window = all.windows.get_mut(&id).unwrap();
        window.scale = 2.0;
        window.ime.active = true;
        let surface = window.surfaces.get_mut(&0).unwrap();
        surface.caret = Some(Some(Rect::new(10.0, 20.0, 1.0, 15.0)));
        surface.scroll = (2.0, 3.0);
    });
    mock!(
        session,
        MapWindowPoints,
        unsafe extern "system" fn(HWND, HWND, *mut POINT, u32) -> i32
    )
    .expect()
    .returning(|from, to, point, count| {
        assert_eq!(from, to);
        assert_eq!(count, 1);
        // SAFETY: the caller supplies one initialized writable point.
        unsafe {
            (*point).x += 5;
            (*point).y += 7;
        };
        0
    });
    position(id);
    assert_eq!(
        state.lock().unwrap().forms,
        [(CFS_POINT, 21, 41, 0, 0), (CFS_EXCLUDE, 21, 71, 23, 71)]
    );
    state.lock().unwrap().available = false;
    position(id);
    let flags = ISC_SHOWUICOMPOSITIONWINDOW | 3;
    mock!(
        session,
        DefWindowProcW,
        unsafe extern "system" fn(HWND, u32, usize, isize) -> isize
    )
    .expect()
    .once()
    .returning(move |_, message, active, mask| {
        assert_eq!((message, active, mask), (WM_IME_SETCONTEXT, 1, 3));
        12
    });
    assert_eq!(
        handle(id, root, WM_IME_SETCONTEXT, 1, flags as isize),
        Some(12)
    );
    session.restore();
    super::super::destroy(id);
}
