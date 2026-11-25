use super::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Default)]
struct Log {
    events: RefCell<Vec<(WindowId, Event)>>,
    idles: Cell<u32>,
}

/// Records events; the first event re-enters `dispatch`; idle calls follow a
/// script.
struct Recorder {
    log: Rc<Log>,
    reenter: bool,
    cancel_input: bool,
    script: fn(u32) -> Option<Duration>,
}

impl Handler for Recorder {
    fn event(&mut self, window: WindowId, event: Event) -> bool {
        if self.reenter {
            self.reenter = false;
            assert!(!dispatch(window, Event::Activated(true)));
        }
        let cancelled = event == Event::Close
            || (self.cancel_input
                && matches!(&event, Event::BeforeInput(input) if input.down && input.code == "F5"));
        self.log.events.borrow_mut().push((window, event));
        cancelled
    }

    fn idle(&mut self) -> Option<Duration> {
        let count = self.log.idles.get() + 1;
        self.log.idles.set(count);
        (self.script)(count)
    }
}

fn recorder(reenter: bool, script: fn(u32) -> Option<Duration>) -> (Box<Recorder>, Rc<Log>) {
    let log = Rc::new(Log::default());
    (
        Box::new(Recorder {
            log: log.clone(),
            reenter,
            cancel_input: false,
            script,
        }),
        log,
    )
}

#[test]
fn events_reach_the_handler() {
    assert!(!dispatch(1, Event::Destroyed));
    let (handler, log) = recorder(true, |_| None);
    assert!(set_handler(handler).is_none());
    assert!(!dispatch(1, Event::Destroyed));
    assert!(dispatch(2, Event::Close));
    assert_eq!(
        *log.events.borrow(),
        vec![
            (1, Event::Destroyed),
            (1, Event::Activated(true)),
            (2, Event::Close)
        ]
    );
    assert!(take_handler().is_some());
    assert!(take_handler().is_none());
}

#[test]
fn native_hit_queries_are_synchronous_and_never_queued_on_reentry() {
    struct HitHandler(Rc<Cell<u32>>);
    impl Handler for HitHandler {
        fn above_control(&mut self, window: WindowId, control: u64, x: f32, y: f32) -> bool {
            self.0.set(self.0.get() + 1);
            assert!(!above_control(window, control, x, y));
            (window, control, x, y) == (1, 2, 30.0, 40.0)
        }
        fn event(&mut self, window: WindowId, _event: Event) -> bool {
            assert!(!above_control(window, 2, 30.0, 40.0));
            false
        }
        fn idle(&mut self) -> Option<Duration> {
            None
        }
    }
    assert!(!above_control(1, 2, 30.0, 40.0));
    let calls = Rc::new(Cell::new(0));
    set_handler(Box::new(HitHandler(calls.clone())));
    assert!(above_control(1, 2, 30.0, 40.0));
    assert!(!above_control(3, 2, 30.0, 40.0));
    assert!(!dispatch(1, Event::Activated(true)));
    assert_eq!(calls.get(), 2);
    assert!(QUEUE.with_borrow(|queue| queue.is_empty()));
    take_handler();
    let (handler, _) = recorder(false, |_| None);
    set_handler(handler);
    assert!(!above_control(1, 2, 30.0, 40.0));
    take_handler();
}

#[test]
fn deferred_native_focus_keeps_the_current_control_without_oscillating() {
    use crate::ControlEvent;
    use nativeboat_ir::{Change, Control, ControlKind, ControlState, Rect};
    struct FocusHandler(Rc<RefCell<Vec<Event>>>);
    impl Handler for FocusHandler {
        fn event(&mut self, window: WindowId, event: Event) -> bool {
            if event == Event::Activated(true) {
                crate::focus_control(window, 2);
                crate::focus_control(window, 1);
                dispatch(
                    window,
                    Event::Control {
                        id: 99,
                        event: ControlEvent::Focus,
                    },
                );
                dispatch(
                    window,
                    Event::Control {
                        id: 99,
                        event: ControlEvent::Blur,
                    },
                );
            } else {
                self.0.borrow_mut().push(event);
            }
            false
        }
        fn idle(&mut self) -> Option<Duration> {
            None
        }
    }
    let window = crate::tests::support::window();
    let root = crate::hwnd(window).unwrap();
    // SAFETY: show only this test's owned window off screen without activation.
    unsafe {
        SetWindowPos(
            root,
            null_mut(),
            -32000,
            -32000,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        ShowWindow(root, SW_SHOWNA);
    }
    for id in [1, 2] {
        crate::apply(
            window,
            &[Change::AddControl {
                surface: 0,
                control: Control {
                    id,
                    kind: ControlKind::TextBox,
                    rect: Rect::new(10.0, 30.0 * id as f32, 120.0, 24.0),
                    state: ControlState {
                        visible: true,
                        ..Default::default()
                    },
                },
            }],
        );
    }
    crate::focus_control(window, 1);
    let events = Rc::new(RefCell::new(Vec::new()));
    set_handler(Box::new(FocusHandler(events.clone())));
    dispatch(window, Event::Activated(true));
    assert_eq!(
        *events.borrow(),
        vec![
            Event::Control {
                id: 2,
                event: ControlEvent::Blur
            },
            Event::Control {
                id: 1,
                event: ControlEvent::Focus
            },
        ]
    );
    assert!(QUEUE.with_borrow(|queue| queue.is_empty()));
    take_handler();
    crate::destroy(window);
}

#[test]
fn the_loop_runs_until_quit() {
    let (handler, log) = recorder(false, |count| {
        if count == 1 {
            // A thread message exercises translation and dispatch.
            Waker::current().wake();
            return Some(Duration::from_micros(1500));
        }
        quit();
        Some(Duration::ZERO)
    });
    let _ = run(handler);
    assert_eq!(log.idles.get(), 2);
    assert!(take_handler().is_none());
}

#[test]
fn wakers_end_an_unbounded_wait() {
    let (handler, log) = recorder(false, |count| {
        if count == 1 {
            let waker = Waker::current();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(20));
                waker.wake();
            });
            return None;
        }
        quit();
        Some(Duration::ZERO)
    });
    let _ = run(handler);
    assert_eq!(log.idles.get(), 2);
    assert_eq!(Waker::current(), Waker::current());
}

static TARGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[test]
fn hotkeys_and_accelerators_reach_the_handler() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    use crate::Accelerator;
    use shimforge::{Session, mock};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;
    use windows_sys::core::BOOL;
    let mut session = Session::new();
    let register = mock!(
        session,
        RegisterHotKey,
        unsafe extern "system" fn(HWND, i32, HOT_KEY_MODIFIERS, u32) -> BOOL
    );
    register
        .expect()
        .once()
        .returning(|owner, id, modifiers, key| {
            assert!(owner.is_null());
            assert_eq!(
                (id, modifiers, key),
                (5, MOD_NOREPEAT | MOD_CONTROL | MOD_SHIFT | MOD_ALT, 0x41)
            );
            1
        });
    register.expect().once().returning(|_, _, modifiers, _| {
        assert_eq!(modifiers, MOD_NOREPEAT);
        0
    });
    let unregister = mock!(
        session,
        UnregisterHotKey,
        unsafe extern "system" fn(HWND, i32) -> BOOL
    );
    unregister.expect().once().returning(|_, id| {
        assert_eq!(id, 5);
        1
    });
    let all = Accelerator {
        key: 0x41,
        ctrl: true,
        shift: true,
        alt: true,
        label: String::new(),
    };
    assert!(register_hotkey(5, &all));
    assert!(!register_hotkey(
        6,
        &Accelerator {
            key: 0x42,
            ..Accelerator::default()
        }
    ));
    unregister_hotkey(5);
    let window = crate::tests::support::window();
    crate::set_menu(window, Some(&crate::menu::tests::items())).unwrap();
    TARGET.store(
        crate::hwnd(window).unwrap() as usize,
        std::sync::atomic::Ordering::SeqCst,
    );
    let (handler, log) = recorder(false, |count| {
        if count == 1 {
            let target = TARGET.load(std::sync::atomic::Ordering::SeqCst) as HWND;
            // SAFETY: posts to this thread's queue and to its own window.
            unsafe {
                PostThreadMessageW(GetCurrentThreadId(), WM_HOTKEY, 5, 0);
                PostMessageW(target, WM_KEYDOWN, 0x74, 0);
            }
            return Some(Duration::ZERO);
        }
        quit();
        Some(Duration::ZERO)
    });
    let _ = run(handler);
    let events = log.events.borrow();
    assert!(events.contains(&(0, Event::HotKey(5))), "{events:?}");
    assert!(
        events.contains(&(
            window,
            Event::Menu(crate::MenuCommand {
                id: 2,
                accelerator: true,
                modifiers: crate::Modifiers::default()
            })
        )),
        "{events:?}"
    );
}

static INPUT_TARGET: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[test]
fn before_input_cancellation_precedes_menu_and_page_defaults() {
    use crate::{Accelerator, MenuItem};
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let window = crate::tests::support::window();
    crate::set_menu(
        window,
        Some(&[
            MenuItem {
                id: 17,
                label: "Blocked".into(),
                enabled: true,
                accelerator: Some(Accelerator {
                    key: 0x74,
                    ..Accelerator::default()
                }),
                ..MenuItem::default()
            },
            MenuItem {
                id: 18,
                label: "Allowed".into(),
                enabled: true,
                accelerator: Some(Accelerator {
                    key: 0x75,
                    ..Accelerator::default()
                }),
                ..MenuItem::default()
            },
        ]),
    )
    .unwrap();
    INPUT_TARGET.store(
        crate::hwnd(window).unwrap() as usize,
        std::sync::atomic::Ordering::SeqCst,
    );
    let (mut handler, log) = recorder(false, |count| {
        if count == 1 {
            let target = INPUT_TARGET.load(std::sync::atomic::Ordering::SeqCst)
                as windows_sys::Win32::Foundation::HWND;
            // SAFETY: the target belongs to this test thread.
            unsafe {
                PostMessageW(target, WM_KEYDOWN, 0x74, 0);
                PostMessageW(target, WM_KEYDOWN, 0x75, 0);
                PostMessageW(target, WM_KEYDOWN, 0x41, 0x1E << 16);
                PostMessageW(target, WM_KEYUP, 0x41, 0x1E << 16 | 1 << 30);
            }
            return Some(Duration::ZERO);
        }
        quit();
        Some(Duration::ZERO)
    });
    handler.cancel_input = true;
    run(handler);
    let events = log.events.borrow();
    let inputs: Vec<_> = events
        .iter()
        .filter_map(|(owner, event)| {
            if let Event::BeforeInput(input) = event {
                assert_eq!(*owner, window);
                Some(input)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(inputs.len(), 4);
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.code.as_str())
            .collect::<Vec<_>>(),
        ["F5", "F6", "KeyA", "KeyA"]
    );
    assert!(!inputs[3].down);
    assert_eq!(
        events
            .iter()
            .filter(|(_, event)| matches!(event, Event::Menu(_)))
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|(_, event)| matches!(event,Event::Key {code,..} if code=="F5"))
    );
    assert!(
        events
            .iter()
            .any(|(_, event)| matches!(event,Event::Key {code,down:true,..} if code=="KeyA"))
    );
    assert!(
        events
            .iter()
            .any(|(_, event)| matches!(event,Event::Key {code,down:false,..} if code=="KeyA"))
    );
    assert!(
        events
            .iter()
            .any(|(_, event)| matches!(event,Event::Text(text) if text=="a"))
    );
    drop(events);
    crate::destroy(window);
}
