use super::*;
use crate::tests::support::{Events, pack, record, send, window};
use crate::window::{apply, destroy, focus_control};
use nativeboat_ir::{Change, ControlState, Corner, Rect};
use shimforge::{Session, mock};
use windows_sys::Win32::System::SystemServices::MK_SHIFT;

#[test]
fn native_edit_hit_queries_forward_adornments_and_preserve_selection_and_capture() {
    use std::cell::RefCell;
    use std::rc::Rc;
    type Queries = Rc<RefCell<Vec<(WindowId, u64, f32, f32)>>>;
    struct Hits {
        queries: Queries,
        events: Rc<RefCell<Vec<Event>>>,
    }
    impl crate::Handler for Hits {
        fn above_control(&mut self, window: WindowId, id: u64, x: f32, y: f32) -> bool {
            self.queries.borrow_mut().push((window, id, x, y));
            id == 1 && (80.0..100.0).contains(&x) && (30.0..55.0).contains(&y)
        }
        fn event(&mut self, _window: WindowId, event: Event) -> bool {
            self.events.borrow_mut().push(event);
            false
        }
        fn idle(&mut self) -> Option<std::time::Duration> {
            None
        }
    }
    for scale in [1.0, 1.5, 2.0] {
        let window = window();
        crate::window::STATE.with_borrow_mut(|state| {
            state.windows.get_mut(&window).unwrap().scale = scale;
        });
        let root = crate::window::hwnd(window).unwrap();
        // SAFETY: position only the owned test window off screen.
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
        let queries = Rc::new(RefCell::new(Vec::new()));
        let events = Rc::new(RefCell::new(Vec::new()));
        crate::event_loop::set_handler(Box::new(Hits {
            queries: queries.clone(),
            events: events.clone(),
        }));
        let mut edit = control(1, ControlKind::TextBox, state("alpha"));
        edit.rect = Rect::new(20.25, 30.25, 120.0, 24.0);
        edit.state.bare = true;
        apply(
            window,
            &[Change::AddControl {
                surface: 0,
                control: edit,
            }],
        );
        let handle = get(window, 1);
        let query = |local_x: f32| {
            let mut point = POINT {
                x: px(local_x, scale),
                y: px(10.0, scale),
            };
            // SAFETY: map the test control's client point into signed screen coordinates.
            unsafe { ClientToScreen(handle, &mut point) };
            send(handle, WM_NCHITTEST, 0, pack(point.x, point.y))
        };
        focus_control(window, 1);
        send(handle, EM_SETSEL, 1, 4);
        assert_eq!(query(65.0), HTTRANSPARENT as isize);
        let (owner, id, x, y) = queries.borrow().last().copied().unwrap();
        assert_eq!((owner, id), (window, 1));
        assert!((x - 85.25).abs() <= 1.0 / scale);
        // The control covers the text, in the middle of the 24-pixel line.
        let shift = (24.0 - crate::fonts::cell_height(&state("alpha").font, scale)) / 2.0;
        assert!((y - 40.25 - shift).abs() <= 1.0 / scale, "{y} {shift}");
        assert_eq!(query(10.0), HTCLIENT as isize);
        assert_eq!(text_selection(window, 1), Some((1, 4)));
        // SAFETY: inspect and capture only the test's own native control.
        unsafe {
            assert_eq!(GetFocus(), handle);
            SetCapture(handle);
        }
        let count = queries.borrow().len();
        assert_eq!(query(65.0), HTCLIENT as isize);
        assert_eq!(queries.borrow().len(), count);
        // SAFETY: release this test thread's capture.
        unsafe { ReleaseCapture() };
        assert_eq!(query(65.0), HTTRANSPARENT as isize);
        send(handle, WM_SETTEXT, 0, wide("beta").as_ptr() as isize);
        assert!(events.borrow().contains(&Event::Control {
            id: 1,
            event: ControlEvent::Input("beta".into()),
        }));
        crate::event_loop::take_handler();
        assert_eq!(query(65.0), HTCLIENT as isize);
        destroy(window);
    }
}

#[test]
fn native_edit_prints_later_adornments_with_clip_and_preserves_input() {
    use crate::tests::support::Canvas;
    let events = record();
    let window = window();
    let scale = crate::window::scale(window);
    let mut edit = control(1, ControlKind::TextBox, state("alpha"));
    edit.state.bare = true;
    edit.state.clip = Some(Rect::new(5.0, 0.0, 50.0, 24.0));
    apply(
        window,
        &[
            Change::AddControl {
                surface: 0,
                control: edit.clone(),
            },
            Change::Paint {
                surface: 0,
                items: vec![
                    DisplayItem::Child(NativeChild::Control(1)),
                    DisplayItem::Fill {
                        rect: edit.rect,
                        color: Color::rgb(255, 0, 0),
                        radii: [Corner::SQUARE; 4],
                    },
                ],
            },
        ],
    );
    let handle = get(window, 1);
    assert_eq!(class_name(handle), "Edit");
    for message in [WM_PRINTCLIENT, WM_PRINT] {
        let canvas = Canvas::new(px(120.0, scale), px(24.0, scale));
        send(handle, message, canvas.dc as usize, PRF_CLIENT as isize);
        assert_eq!(canvas.pixel(px(10.0, scale), px(10.0, scale)), (255, 0, 0));
        assert_eq!(canvas.pixel(0, px(10.0, scale)), (255, 255, 255));
        assert_eq!(
            canvas.pixel(px(70.0, scale), px(10.0, scale)),
            (255, 255, 255)
        );
        // SAFETY: the owned DC's clip is restored after composition.
        unsafe {
            SetPixel(canvas.dc, 0, 0, colorref(Color::BLACK));
            GdiFlush();
        }
        assert_eq!(canvas.pixel(0, 0), (0, 0, 0));
    }
    let skipped = Canvas::new(160, 40);
    send(
        handle,
        WM_PRINT,
        skipped.dc as usize,
        (PRF_CHECKVISIBLE | PRF_CLIENT) as isize,
    );
    send(handle, WM_PRINT, skipped.dc as usize, 0);
    assert_eq!(skipped.pixel(10, 10), (255, 255, 255));
    assert_eq!(send(handle, WM_ERASEBKGND, skipped.dc as usize, 0), 1);
    // SAFETY: show only the test's own off-screen window without activation.
    unsafe {
        let root = crate::window::hwnd(window).unwrap();
        SetWindowPos(
            root,
            null_mut(),
            -32000,
            -32000,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        ShowWindow(root, SW_SHOWNOACTIVATE);
        InvalidateRect(handle, null(), 0);
        UpdateWindow(handle);
    }
    send(
        handle,
        WM_PRINT,
        skipped.dc as usize,
        (PRF_CHECKVISIBLE | PRF_CLIENT) as isize,
    );
    assert_eq!(skipped.pixel(px(10.0, scale), px(10.0, scale)), (255, 0, 0));
    events.take();
    let text = wide("beta");
    send(handle, WM_SETTEXT, 0, text.as_ptr() as isize);
    assert!(events.take().iter().any(|event| matches!(event,
        Event::Control { id: 1, event: ControlEvent::Input(value) } if value == "beta")));
    destroy(window);
}

#[test]
fn native_edit_empty_or_failed_composition_releases_gdi_objects() {
    use crate::tests::support::Canvas;
    let window = window();
    let mut edit = control(1, ControlKind::TextBox, state(""));
    edit.rect.width = 0.0;
    apply(
        window,
        &[
            Change::AddControl {
                surface: 0,
                control: edit.clone(),
            },
            Change::Paint {
                surface: 0,
                items: vec![
                    DisplayItem::Child(NativeChild::Control(1)),
                    DisplayItem::Fill {
                        rect: Rect::new(0.0, 0.0, 200.0, 100.0),
                        color: Color::BLACK,
                        radii: [Corner::SQUARE; 4],
                    },
                ],
            },
        ],
    );
    let canvas = Canvas::new(16, 16);
    send(get(window, 1), WM_PRINTCLIENT, canvas.dc as usize, 0);
    assert_eq!(canvas.pixel(0, 0), (255, 255, 255));
    edit.rect.width = 50.0;
    apply(window, &[Change::UpdateControl { control: edit }]);
    // Other tests create and free GDI objects in parallel, so the count is
    // retried; a leak changes it on every attempt.
    let leaked = gdi_objects_leaked(|| {
        let mut session = Session::new();
        let create = mock!(
            session,
            CreateDIBSection,
            unsafe extern "system" fn(
                HDC,
                *const BITMAPINFO,
                u32,
                *mut *mut core::ffi::c_void,
                HANDLE,
                u32,
            ) -> HBITMAP
        );
        create.expect().returning(|_, _, _, _, _, _| null_mut());
        send(get(window, 1), WM_PRINTCLIENT, canvas.dc as usize, 0);
        session.restore();
    });
    assert!(!leaked);
    assert_eq!(canvas.pixel(0, 0), (255, 255, 255));
    destroy(window);
}

#[test]
fn native_edit_print_client_and_print_composite_alpha_once() {
    use crate::tests::support::Canvas;
    let window = window();
    let scale = crate::window::scale(window);
    let mut edit = control(1, ControlKind::TextBox, state(""));
    edit.state.bare = true;
    let mut translucent = Color::rgb(255, 0, 0);
    translucent.a = 0.5;
    apply(
        window,
        &[
            Change::AddControl {
                surface: 0,
                control: edit.clone(),
            },
            Change::Paint {
                surface: 0,
                items: vec![
                    DisplayItem::Child(NativeChild::Control(1)),
                    DisplayItem::Fill {
                        rect: edit.rect,
                        color: translucent,
                        radii: [Corner::SQUARE; 4],
                    },
                ],
            },
        ],
    );
    for message in [WM_PRINTCLIENT, WM_PRINT] {
        let canvas = Canvas::new(px(120.0, scale), px(24.0, scale));
        send(
            get(window, 1),
            message,
            canvas.dc as usize,
            PRF_CLIENT as isize,
        );
        let (r, g, b) = canvas.pixel(px(60.0, scale), px(10.0, scale));
        assert!(
            r == 255 && (126..=129).contains(&g) && (126..=129).contains(&b),
            "{message}: translucent adornment applied more than once: {r},{g},{b}"
        );
    }
    destroy(window);
}

#[test]
fn authored_native_toggles_print_css_pixels_and_keep_button_semantics() {
    use crate::tests::support::Canvas;
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let events = record();
    let window = window();
    let items = vec![DisplayItem::Fill {
        rect: Rect::new(8.0, 8.0, 18.0, 18.0),
        color: Color::rgb(255, 0, 0),
        radii: [Corner::round(9.0); 4],
    }];
    for (id, kind, style) in [
        (1, ControlKind::CheckBox, BS_3STATE),
        (2, ControlKind::Radio, BS_RADIOBUTTON),
    ] {
        let mut toggle = control(id, kind, state(""));
        toggle.rect = Rect::new(10.0, 10.0, 12.0, 12.0);
        toggle.state.bare = true;
        toggle.state.checked = true;
        apply(
            window,
            &[
                Change::Paint {
                    surface: 0,
                    items: items.clone(),
                },
                Change::AddControl {
                    surface: 0,
                    control: toggle.clone(),
                },
            ],
        );
        let handle = get(window, id);
        assert_eq!(class_name(handle), "Button");
        assert_eq!(self::style(handle) & BS_TYPEMASK as u32, style as u32);
        assert_eq!(send(handle, BM_GETCHECK, 0, 0), BST_CHECKED as isize);
        let mut bounds = RECT::default();
        let mut origin = POINT::default();
        // SAFETY: both windows belong to this test.
        unsafe {
            GetClientRect(handle, &mut bounds);
            MapWindowPoints(handle, crate::window::hwnd(window).unwrap(), &mut origin, 1);
        }
        let reference = Canvas::new(64, 64);
        paint_area(
            reference.dc,
            &items,
            crate::window::scale(window),
            (0.0, 0.0),
            None,
            false,
        );
        for message in [WM_PRINTCLIENT, WM_PRINT] {
            let canvas = Canvas::new(bounds.right, bounds.bottom);
            send(handle, message, canvas.dc as usize, PRF_CLIENT as isize);
            for y in 0..bounds.bottom {
                for x in 0..bounds.right {
                    assert_eq!(
                        canvas.pixel(x, y),
                        reference.pixel(origin.x + x, origin.y + y),
                        "{message}: {x},{y}"
                    );
                }
            }
        }
        let skipped = Canvas::new(12, 12);
        send(handle, WM_PRINT, skipped.dc as usize, 0);
        send(
            handle,
            WM_PRINT,
            skipped.dc as usize,
            (PRF_CLIENT | PRF_CHECKVISIBLE) as isize,
        );
        assert_eq!(skipped.pixel(6, 6), (255, 255, 255));
        assert_eq!(send(handle, WM_ERASEBKGND, skipped.dc as usize, 0), 1);
        // Paint validates the native update region without changing class/state.
        send(handle, WM_PAINT, 0, 0);
        // SAFETY: the test window is shown off screen without activation.
        unsafe {
            let root = crate::window::hwnd(window).unwrap();
            SetWindowPos(
                root,
                null_mut(),
                -32000,
                -32000,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            ShowWindow(root, SW_SHOWNOACTIVATE);
        }
        send(
            handle,
            WM_PRINT,
            skipped.dc as usize,
            (PRF_CLIENT | PRF_CHECKVISIBLE) as isize,
        );
        assert_ne!(skipped.pixel(6, 6), (255, 255, 255));
        // SAFETY: restore the owned window's hidden state before the next control.
        unsafe {
            ShowWindow(crate::window::hwnd(window).unwrap(), SW_HIDE);
        }
        events.take();
        send(handle, BM_CLICK, 0, 0);
        assert!(events.take().iter().any(|event| matches!(event, Event::Control{id:event_id,event:ControlEvent::Click} if *event_id==id)));
        toggle.state.checked = false;
        toggle.state.indeterminate = kind == ControlKind::CheckBox;
        apply(window, &[Change::UpdateControl { control: toggle }]);
        assert_eq!(
            send(handle, BM_GETCHECK, 0, 0),
            if kind == ControlKind::CheckBox {
                BST_INDETERMINATE
            } else {
                BST_UNCHECKED
            } as isize
        );
        apply(window, &[Change::RemoveControl { id }]);
    }
    destroy(window);
}

#[test]
fn authored_toggle_surface_updates_invalidate_only_dependent_toggle_clients() {
    let window = window();
    for (id, kind, bare) in [
        (1, ControlKind::CheckBox, true),
        (2, ControlKind::CheckBox, false),
        (3, ControlKind::TextBox, true),
    ] {
        let mut widget = control(id, kind, state(""));
        widget.state.bare = bare;
        apply(
            window,
            &[Change::AddControl {
                surface: 0,
                control: widget,
            }],
        );
    }
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = calls.clone();
    let mut session = Session::new();
    let invalidate = mock!(
        session,
        InvalidateRect,
        unsafe extern "system" fn(HWND, *const RECT, i32) -> i32
    );
    invalidate.expect().returning(move |handle, _, _| {
        captured.lock().unwrap().push(handle as usize);
        1
    });
    apply(
        window,
        &[Change::Paint {
            surface: 0,
            items: vec![DisplayItem::Fill {
                rect: Rect::new(0.0, 0.0, 200.0, 200.0),
                color: Color::rgb(0, 0, 255),
                radii: [Corner::SQUARE; 4],
            }],
        }],
    );
    session.restore();
    for id in 1..=3 {
        assert_eq!(
            calls.lock().unwrap().contains(&(get(window, id) as usize)),
            id == 1
        );
    }
    destroy(window);
    invalidate_authored(window, 0);
}

#[test]
fn single_line_fields_stay_centred_when_their_surface_moves_them() {
    let window = window();
    let mut edit = control(1, ControlKind::TextBox, state(""));
    edit.rect = Rect::new(20.0, 30.0, 120.0, 30.0);
    edit.state.bare = true;
    apply(
        window,
        &[Change::AddControl {
            surface: 0,
            control: edit,
        }],
    );
    let placed = || {
        let handle = get(window, 1);
        let mut bounds = RECT::default();
        let mut origin = POINT::default();
        // SAFETY: reads the geometry of the test's own control.
        unsafe {
            GetClientRect(handle, &mut bounds);
            MapWindowPoints(handle, crate::window::hwnd(window).unwrap(), &mut origin, 1);
        }
        (origin.y, bounds.bottom)
    };
    let (top, height) = placed();
    let scale = crate::window::scale(window);
    let line = crate::fonts::cell_height(&state("").font, scale);
    assert!(height < px(30.0, scale), "{height}");
    assert!(
        (top - px(30.0 + (30.0 - line) / 2.0, scale)).abs() <= 1,
        "{top}"
    );
    // The surface's scrolling moves the field, which keeps its height and
    // its place in the middle of its line.
    let scrolled = |scroll_y| Change::UpdateSurface {
        id: 0,
        rect: Rect::new(0.0, 0.0, 800.0, 600.0),
        content_width: 800.0,
        content_height: 1200.0,
        scroll_x: 0.0,
        scroll_y,
    };
    apply(window, &[scrolled(0.0)]);
    assert_eq!(placed(), (top, height));
    apply(window, &[scrolled(10.0)]);
    let (moved, kept) = placed();
    assert_eq!(kept, height);
    assert!((top - moved - px(10.0, scale)).abs() <= 1, "{top} {moved}");
    destroy(window);
}

#[test]
fn authored_toggle_printing_keeps_device_alignment_after_scroll_scale_and_clip() {
    use crate::tests::support::Canvas;
    let window = window();
    let mut toggle = control(1, ControlKind::CheckBox, state(""));
    toggle.state.bare = true;
    toggle.rect = Rect::new(100.0, 80.0, 12.0, 12.0);
    toggle.state.clip = Some(Rect::new(3.0, 2.0, 6.0, 8.0));
    let items = vec![DisplayItem::Fill {
        rect: Rect::new(98.0, 78.0, 18.0, 18.0),
        color: Color::rgb(0, 128, 0),
        radii: [Corner::round(9.0); 4],
    }];
    apply(
        window,
        &[
            Change::Paint {
                surface: 0,
                items: items.clone(),
            },
            Change::AddControl {
                surface: 0,
                control: toggle.clone(),
            },
            Change::UpdateSurface {
                id: 0,
                rect: Rect::new(0.0, 0.0, 800.0, 600.0),
                content_width: 1600.0,
                content_height: 1200.0,
                scroll_x: 40.0,
                scroll_y: 20.0,
            },
        ],
    );
    for scale in [1.0, 1.5, 2.0] {
        STATE.with_borrow_mut(|state| state.windows.get_mut(&window).unwrap().scale = scale);
        rescale(window);
        let handle = get(window, 1);
        let mut bounds = RECT::default();
        let mut origin = POINT::default();
        // SAFETY: coordinates come from the actual owned native control.
        unsafe {
            GetClientRect(handle, &mut bounds);
            MapWindowPoints(handle, crate::window::hwnd(window).unwrap(), &mut origin, 1);
        }
        let reference = Canvas::new(200, 200);
        paint_area(reference.dc, &items, scale, (40.0, 20.0), None, false);
        let actual = Canvas::new(bounds.right, bounds.bottom);
        send(handle, WM_PRINTCLIENT, actual.dc as usize, 0);
        for y in 0..bounds.bottom {
            for x in 0..bounds.right {
                let clipped = x >= px(3.0, scale)
                    && x < px(9.0, scale)
                    && y >= px(2.0, scale)
                    && y < px(10.0, scale);
                assert_eq!(
                    actual.pixel(x, y),
                    if clipped {
                        reference.pixel(origin.x + x, origin.y + y)
                    } else {
                        (255, 255, 255)
                    },
                    "{scale}: {x},{y}"
                );
            }
        }
        // The caller's DC clip must be restored after control printing.
        // SAFETY: the owned canvas DC is live and these calls retain no pointers.
        unsafe {
            SetPixel(actual.dc, 0, 0, colorref(Color::rgb(0, 0, 255)));
            GdiFlush();
        }
        assert_eq!(actual.pixel(0, 0), (0, 0, 255));
    }
    destroy(window);
}

#[test]
fn authored_toggle_empty_bounds_and_failed_bitmap_leave_the_target_untouched() {
    use crate::tests::support::Canvas;
    let window = window();
    let mut toggle = control(1, ControlKind::Radio, state(""));
    toggle.state.bare = true;
    toggle.rect.width = 0.0;
    apply(
        window,
        &[Change::AddControl {
            surface: 0,
            control: toggle.clone(),
        }],
    );
    let canvas = Canvas::new(16, 16);
    send(get(window, 1), WM_PRINTCLIENT, canvas.dc as usize, 0);
    assert_eq!(canvas.pixel(0, 0), (255, 255, 255));
    toggle.rect.width = 12.0;
    apply(window, &[Change::UpdateControl { control: toggle }]);
    // Other tests create and free GDI objects in parallel, so the count is
    // retried; a leak changes it on every attempt.
    let leaked = gdi_objects_leaked(|| {
        let mut session = Session::new();
        let create = mock!(
            session,
            CreateDIBSection,
            unsafe extern "system" fn(
                HDC,
                *const BITMAPINFO,
                u32,
                *mut *mut core::ffi::c_void,
                HANDLE,
                u32,
            ) -> HBITMAP
        );
        create.expect().returning(|_, _, _, _, _, _| null_mut());
        send(get(window, 1), WM_PRINTCLIENT, canvas.dc as usize, 0);
        session.restore();
    });
    assert!(!leaked);
    assert_eq!(canvas.pixel(0, 0), (255, 255, 255));
    destroy(window);
}

fn state(text: &str) -> ControlState {
    ControlState {
        text: text.to_owned(),
        visible: true,
        ..ControlState::default()
    }
}

fn control(id: u64, kind: ControlKind, state: ControlState) -> Control {
    Control {
        id,
        kind,
        rect: Rect::new(10.0, 10.0 * id as f32, 120.0, 24.0),
        state,
    }
}

#[test]
fn native_text_control_ime_sessions_reach_before_input_without_preedit_text() {
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let events = record();
    let window = window();
    for (id, kind) in [
        (1, ControlKind::TextBox),
        (2, ControlKind::Password),
        (3, ControlKind::TextArea),
        (4, ControlKind::Button),
    ] {
        apply(
            window,
            &[Change::AddControl {
                surface: 0,
                control: control(id, kind, state("unchanged")),
            }],
        );
    }
    let handles: Vec<_> = (1..=4).map(|id| get(window, id)).collect();
    let input = |handle| {
        events.take();
        assert!(!crate::window::before_input(&MSG {
            hwnd: handle,
            message: WM_KEYDOWN,
            wParam: usize::from(VK_F5),
            ..MSG::default()
        }));
        let received = events.take();
        let Event::BeforeInput(input) = &received[0] else {
            panic!("{received:?}")
        };
        input.composing
    };
    assert_eq!(composing(null_mut()), None);
    assert_eq!(composing(crate::window::hwnd(window).unwrap()), None);
    {
        let mut session = Session::new();
        mock!(
            session,
            DefSubclassProc,
            unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT
        )
        .expect()
        .returning(|_, message, wparam, lparam| {
            assert!(matches!(
                message,
                WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION | WM_KILLFOCUS
            ));
            assert_eq!((wparam, lparam), (0, 0));
            17
        });
        let message = |handle, index, kind| {
            // SAFETY: the test supplies a live owned control and its subclass id.
            let result = unsafe { subclass_proc(handle, kind, 0, 0, 1, index + 1) };
            assert_eq!(result, 17);
        };
        for (index, &handle) in handles.iter().enumerate() {
            assert!(!input(handle));
            message(handle, index, WM_IME_STARTCOMPOSITION);
            assert_eq!(input(handle), index < 3);
            // Empty composition updates retain the session; the native EDIT
            // procedure receives them unchanged.
            message(handle, index, WM_IME_COMPOSITION);
            assert_eq!(input(handle), index < 3);
            for &other in &handles[index + 1..] {
                assert!(!input(other));
            }
            message(handle, index, WM_IME_ENDCOMPOSITION);
            assert!(!input(handle));
            message(handle, index, WM_IME_STARTCOMPOSITION);
            message(handle, index, WM_KILLFOCUS);
            assert!(!input(handle));
        }
    }
    assert_eq!(
        control_message(handles[0], 1, WM_IME_STARTCOMPOSITION, 0, 0),
        None
    );
    assert!(input(handles[0]));
    apply(window, &[Change::RemoveControl { id: 1 }]);
    apply(
        window,
        &[Change::AddControl {
            surface: 0,
            control: control(1, ControlKind::TextBox, state("new")),
        }],
    );
    assert!(!input(get(window, 1)));
    destroy(window);
    assert_eq!(composing(handles[1]), None);
}

#[test]
fn native_control_regions_clip_restore_and_rescale() {
    let id = window();
    let mut clipped = control(1, ControlKind::TextBox, state("test"));
    clipped.state.clip = Some(Rect::new(5.0, 2.0, 30.0, 10.0));
    apply(
        id,
        &[Change::AddControl {
            surface: 0,
            control: clipped.clone(),
        }],
    );
    let hwnd = get(id, 1);
    // SAFETY: copies this test control's region into a region owned here.
    unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        assert_eq!(GetWindowRgn(hwnd, region), SIMPLEREGION);
        let mut bounds = RECT::default();
        GetRgnBox(region, &mut bounds);
        let scale = crate::window::scale(id);
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (
                px(5.0, scale),
                px(2.0, scale),
                px(35.0, scale),
                px(12.0, scale)
            )
        );
        STATE.with_borrow_mut(|state| state.windows.get_mut(&id).unwrap().scale = 2.0);
        rescale(id);
        GetWindowRgn(hwnd, region);
        GetRgnBox(region, &mut bounds);
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (10, 4, 70, 24)
        );
        clipped.state.clip = Some(Rect::new(0.0, 0.0, 0.0, 0.0));
        apply(
            id,
            &[Change::UpdateControl {
                control: clipped.clone(),
            }],
        );
        assert_eq!(GetWindowRgn(hwnd, region), NULLREGION);
        clipped.state.clip = None;
        apply(id, &[Change::UpdateControl { control: clipped }]);
        assert_eq!(GetWindowRgn(hwnd, region), 0);
        DeleteObject(region);
    }
    destroy(id);
}

#[test]
fn control_region_failures_keep_ownership_with_the_caller() {
    let id = window();
    let mut clipped = control(1, ControlKind::TextBox, state("test"));
    clipped.state.clip = Some(Rect::new(0.0, 0.0, 10.0, 10.0));
    let mut session = Session::new();
    let allocate = mock!(
        session,
        CreateRectRgn,
        unsafe extern "system" fn(i32, i32, i32, i32) -> HRGN
    );
    allocate.expect().returning(|_, _, _, _| null_mut());
    apply(
        id,
        &[Change::AddControl {
            surface: 0,
            control: clipped.clone(),
        }],
    );
    session.restore();
    let region = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let captured = region.clone();
    let set = mock!(
        session,
        SetWindowRgn,
        unsafe extern "system" fn(HWND, HRGN, i32) -> i32
    );
    set.expect().returning(move |_, r, _| {
        captured.store(r as usize, std::sync::atomic::Ordering::SeqCst);
        0
    });
    clipped.state.text = "updated".into();
    apply(id, &[Change::UpdateControl { control: clipped }]);
    session.restore();
    let region = region.load(std::sync::atomic::Ordering::SeqCst);
    assert_ne!(region, 0);
    // SAFETY: GetObjectType checks that our failed transfer was released.
    assert_eq!(unsafe { GetObjectType(region as HRGN) }, 0);
    destroy(id);
}

fn class_name(hwnd: HWND) -> String {
    let mut buffer = [0_u16; 64];
    // SAFETY: the buffer length is passed.
    unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    crate::wide::narrow(&buffer)
}

fn get(window: WindowId, id: u64) -> HWND {
    hwnd(window, id).unwrap()
}

fn style(hwnd: HWND) -> u32 {
    // SAFETY: reads the style of a live window.
    unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 }
}

fn controls() -> Vec<Control> {
    vec![
        control(1, ControlKind::Button, state("Save")),
        control(
            2,
            ControlKind::TextBox,
            ControlState {
                placeholder: "Name".to_owned(),
                max_length: Some(5),
                read_only: true,
                ..state("hello")
            },
        ),
        control(3, ControlKind::Password, state("secret")),
        control(4, ControlKind::TextArea, state("a\nb")),
        control(
            5,
            ControlKind::CheckBox,
            ControlState {
                checked: true,
                ..state("")
            },
        ),
        control(
            6,
            ControlKind::CheckBox,
            ControlState {
                indeterminate: true,
                ..state("")
            },
        ),
        control(
            7,
            ControlKind::Radio,
            ControlState {
                disabled: true,
                ..state("")
            },
        ),
        control(
            8,
            ControlKind::ComboBox,
            ControlState {
                options: vec!["a".to_owned(), "b".to_owned()],
                selected: vec![1],
                ..state("")
            },
        ),
        control(
            9,
            ControlKind::ListBox,
            ControlState {
                options: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
                selected: vec![0, 2],
                multiple: true,
                ..state("")
            },
        ),
        control(
            10,
            ControlKind::ListBox,
            ControlState {
                options: vec!["x".to_owned(), "y".to_owned()],
                selected: vec![1],
                ..state("")
            },
        ),
        control(
            11,
            ControlKind::Slider,
            ControlState {
                value: 30.0,
                minimum: 10.0,
                maximum: 50.0,
                ..state("")
            },
        ),
        control(
            12,
            ControlKind::ProgressBar,
            ControlState {
                value: 0.5,
                maximum: 2.0,
                ..state("")
            },
        ),
        control(
            13,
            ControlKind::Button,
            ControlState {
                visible: false,
                ..state("Hidden")
            },
        ),
    ]
}

fn setup() -> (WindowId, Events) {
    let id = window();
    let changes: Vec<Change> = controls()
        .into_iter()
        .map(|control| Change::AddControl {
            surface: 0,
            control,
        })
        .collect();
    apply(id, &changes);
    (id, record())
}

#[test]
fn controls_are_created_with_their_state() {
    let (id, _) = setup();
    let classes: Vec<String> = (1..=12)
        .map(|control| class_name(get(id, control)))
        .collect();
    assert_eq!(
        classes,
        [
            "Button",
            "Edit",
            "Edit",
            "Edit",
            "Button",
            "Button",
            "Button",
            "ComboBox",
            "ListBox",
            "ListBox",
            "msctls_trackbar32",
            "msctls_progress32"
        ]
    );
    assert_eq!(window_text(get(id, 1)), "Save");
    let text = get(id, 2);
    assert_eq!(window_text(text), "hello");
    assert_ne!(style(text) & ES_READONLY as u32, 0);
    assert_eq!(send(text, EM_GETLIMITTEXT, 0, 0), 5);
    assert_ne!(style(get(id, 3)) & ES_PASSWORD as u32, 0);
    assert_eq!(window_text(get(id, 4)), "a\r\nb");
    assert_eq!(send(get(id, 5), BM_GETCHECK, 0, 0), BST_CHECKED as isize);
    assert_eq!(
        send(get(id, 6), BM_GETCHECK, 0, 0),
        BST_INDETERMINATE as isize
    );
    // SAFETY: reads the state of a live control.
    assert_eq!(unsafe { IsWindowEnabled(get(id, 7)) }, 0);
    let combo = get(id, 8);
    assert_eq!(send(combo, CB_GETCOUNT, 0, 0), 2);
    assert_eq!(send(combo, CB_GETCURSEL, 0, 0), 1);
    let multiple = get(id, 9);
    assert_ne!(style(multiple) & LBS_EXTENDEDSEL as u32, 0);
    assert_eq!(send(multiple, LB_GETSELCOUNT, 0, 0), 2);
    assert_eq!(send(get(id, 10), LB_GETCURSEL, 0, 0), 1);
    assert_eq!(send(get(id, 11), TBM_GETPOS, 0, 0), 30);
    assert_eq!(send(get(id, 12), PBM_GETPOS, 0, 0), 500);
    assert_ne!(style(get(id, 1)) & WS_VISIBLE, 0);
    assert_eq!(style(get(id, 13)) & WS_VISIBLE, 0);
    assert_eq!(extra_height(ControlKind::ComboBox), DROPDOWN);
    assert_eq!(extra_height(ControlKind::Button), 0.0);
    destroy(id);
}

#[test]
fn updates_change_only_what_differs() {
    let (id, events) = setup();
    let text = get(id, 2);
    send(text, EM_SETSEL, 1, 3);
    let mut changed = controls()[1].clone();
    apply(
        id,
        &[Change::UpdateControl {
            control: changed.clone(),
        }],
    );
    let (mut start, mut end) = (0_u32, 0_u32);
    send(
        text,
        EM_GETSEL,
        &mut start as *mut u32 as usize,
        &mut end as *mut u32 as isize,
    );
    assert_eq!((start, end), (1, 3));
    changed.state.text = "help".to_owned();
    changed.rect = Rect::new(30.0, 40.0, 100.0, 20.0);
    apply(id, &[Change::UpdateControl { control: changed }]);
    assert_eq!(window_text(text), "help");
    send(
        text,
        EM_GETSEL,
        &mut start as *mut u32 as usize,
        &mut end as *mut u32 as isize,
    );
    assert_eq!((start, end), (1, 3));
    let mut combo = controls()[7].clone();
    combo.state.selected = Vec::new();
    let mut list = controls()[9].clone();
    list.state.options.push("w".to_owned());
    list.state.selected = Vec::new();
    apply(
        id,
        &[
            Change::UpdateControl { control: combo },
            Change::UpdateControl { control: list },
            Change::UpdateControl {
                control: control(99, ControlKind::Button, state("missing")),
            },
        ],
    );
    assert_eq!(send(get(id, 8), CB_GETCURSEL, 0, 0), -1);
    assert_eq!(send(get(id, 10), LB_GETCOUNT, 0, 0), 3);
    assert_eq!(send(get(id, 10), LB_GETCURSEL, 0, 0), -1);
    assert!(events.take().is_empty());
    destroy(id);
}

#[test]
fn notifications_become_events() {
    let (id, events) = setup();
    let root = crate::window::hwnd(id).unwrap();
    let notify = |control: u64, code: u32| {
        send(
            root,
            WM_COMMAND,
            (code as usize) << 16,
            get(id, control) as isize,
        );
    };
    notify(1, BN_CLICKED);
    let typed = crate::wide::wide("typed\r\nline");
    // SAFETY: the text outlives the call. Multi-line edits do not notify
    // programmatic changes, so the notification is sent by hand.
    unsafe { SetWindowTextW(get(id, 4), typed.as_ptr()) };
    notify(4, EN_CHANGE);
    send(get(id, 8), CB_SETCURSEL, 0, 0);
    notify(8, CBN_SELCHANGE);
    send(get(id, 9), LB_SETSEL, 1, 1);
    notify(9, LBN_SELCHANGE);
    notify(10, LBN_SELCHANGE);
    notify(2, EN_SETFOCUS);
    send(root, WM_COMMAND, 0, root as isize);
    send(root, WM_HSCROLL, 0, get(id, 11) as isize);
    send(root, WM_HSCROLL, 0, root as isize);
    let control_events: Vec<(u64, ControlEvent)> = events
        .take()
        .into_iter()
        .filter_map(|event| match event {
            Event::Control { id, event } => Some((id, event)),
            _ => None,
        })
        .collect();
    assert_eq!(
        control_events,
        vec![
            (1, ControlEvent::Click),
            (4, ControlEvent::Input("typed\nline".to_owned())),
            (8, ControlEvent::Select(vec![0])),
            (9, ControlEvent::Select(vec![0, 1, 2])),
            (10, ControlEvent::Select(vec![1])),
            (11, ControlEvent::Value(30.0)),
        ]
    );
    destroy(id);
}

fn extended(hwnd: HWND) -> u32 {
    // SAFETY: reads the extended style of a live window.
    unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 }
}

fn picked(hwnd: HWND) -> Option<SYSTEMTIME> {
    let mut time = SYSTEMTIME::default();
    let result = send(
        hwnd,
        DTM_GETSYSTEMTIME,
        0,
        &mut time as *mut SYSTEMTIME as isize,
    );
    (result == GDT_VALID as isize).then_some(time)
}

#[test]
fn date_and_time_pickers_and_bare_fields() {
    let id = window();
    crate::set_date_locale("en-US");
    let picker = |id: u64, kind: ControlKind, value: &str, bare: bool| {
        control(
            id,
            kind,
            ControlState {
                bare,
                ..state(value)
            },
        )
    };
    let controls = [
        picker(20, ControlKind::Date, "2024-03-15", false),
        picker(21, ControlKind::Date, "", true),
        picker(22, ControlKind::Time, "13:45", true),
        picker(23, ControlKind::DateTime, "2024-03-15T08:30", false),
        picker(24, ControlKind::Month, "2024-07", false),
        picker(25, ControlKind::TextBox, "plain", true),
    ];
    apply(
        id,
        &controls
            .iter()
            .map(|control| Change::AddControl {
                surface: 0,
                control: control.clone(),
            })
            .collect::<Vec<_>>(),
    );
    let events = record();
    for control in 20..=24 {
        assert_eq!(class_name(get(id, control)), "SysDateTimePick32");
    }
    let date = picked(get(id, 20)).unwrap();
    assert_eq!((date.wYear, date.wMonth, date.wDay), (2024, 3, 15));
    let time = picked(get(id, 22)).unwrap();
    assert_eq!((time.wHour, time.wMinute), (13, 45));
    assert_ne!(style(get(id, 22)) & DTS_TIMEFORMAT, 0);
    assert_eq!(picked(get(id, 24)).unwrap().wMonth, 7);
    // Fields whose box the page draws have no edge of their own.
    assert_ne!(extended(get(id, 20)) & WS_EX_CLIENTEDGE, 0);
    assert_eq!(extended(get(id, 21)) & WS_EX_CLIENTEDGE, 0);
    assert_eq!(extended(get(id, 25)) & WS_EX_CLIENTEDGE, 0);
    // Pickers inside author-drawn fields do not paint a second themed frame.
    // SAFETY: reads theme handles of live controls.
    assert_eq!(unsafe { GetWindowTheme(get(id, 21)) }, 0);
    let mut themed = controls[1].clone();
    themed.state.bare = false;
    apply(id, &[Change::UpdateControl { control: themed }]);
    assert_ne!(extended(get(id, 21)) & WS_EX_CLIENTEDGE, 0);
    apply(
        id,
        &[Change::UpdateControl {
            control: controls[1].clone(),
        }],
    );
    // The page drawing the box or not changes the edge.
    let mut unstyled = controls[5].clone();
    unstyled.state.bare = false;
    let mut cleared = controls[0].clone();
    cleared.state.text = "not a date".to_owned();
    apply(
        id,
        &[
            Change::UpdateControl { control: unstyled },
            Change::UpdateControl { control: cleared },
        ],
    );
    assert_ne!(extended(get(id, 25)) & WS_EX_CLIENTEDGE, 0);
    assert!(events.take().is_empty());
    // Picking a date reports the input's new value.
    let root = crate::window::hwnd(id).unwrap();
    let notify = |control: u64, code: u32| {
        let header = NMHDR {
            hwndFrom: get(id, control),
            idFrom: 0,
            code,
        };
        send(root, WM_NOTIFY, 0, &header as *const NMHDR as isize);
    };
    let moment = SYSTEMTIME {
        wYear: 2025,
        wMonth: 12,
        wDay: 24,
        wHour: 18,
        wMinute: 5,
        ..SYSTEMTIME::default()
    };
    send(
        get(id, 23),
        DTM_SETSYSTEMTIME,
        GDT_VALID as usize,
        &moment as *const SYSTEMTIME as isize,
    );
    notify(23, DTN_DATETIMECHANGE);
    notify(22, NM_CLICK);
    notify(25, DTN_DATETIMECHANGE);
    let header = NMHDR {
        hwndFrom: root,
        idFrom: 0,
        code: DTN_DATETIMECHANGE,
    };
    send(root, WM_NOTIFY, 0, &header as *const NMHDR as isize);
    send(root, WM_NOTIFY, 0, 0);
    let control_events: Vec<(u64, ControlEvent)> = events
        .take()
        .into_iter()
        .filter_map(|event| match event {
            Event::Control { id, event } => Some((id, event)),
            _ => None,
        })
        .collect();
    assert_eq!(
        control_events,
        vec![(23, ControlEvent::Input("2025-12-24T18:05".to_owned())),]
    );
    destroy(id);
}

#[test]
fn colors_follow_css() {
    let (id, _) = setup();
    let root = crate::window::hwnd(id).unwrap();
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![
                DisplayItem::PopClip,
                DisplayItem::Fill {
                    rect: Rect::new(0.0, 0.0, 200.0, 100.0),
                    color: Color::rgb(0, 0, 255),
                    radii: [Corner::SQUARE; 4],
                },
                // A translucent background of the text box's own, under it.
                DisplayItem::Fill {
                    rect: Rect::new(10.0, 30.0, 120.0, 24.0),
                    color: Color::rgba(0, 0, 0, 0.5),
                    radii: [Corner::SQUARE; 4],
                },
                DisplayItem::Child(NativeChild::Control(3)),
                DisplayItem::Fill {
                    rect: Rect::new(0.0, 0.0, 200.0, 45.0),
                    color: Color::rgb(255, 0, 0),
                    radii: [Corner::SQUARE; 4],
                },
            ],
        }],
    );
    let mut styled = controls()[1].clone();
    styled.state.color = Color::rgb(255, 0, 0);
    styled.state.background = Color::rgb(0, 255, 0);
    apply(id, &[Change::UpdateControl { control: styled }]);
    // SAFETY: the DC belongs to a live control and is released.
    unsafe {
        let control = get(id, 5);
        let dc = GetDC(control);
        let brush = send(root, WM_CTLCOLORSTATIC, dc as usize, control as isize);
        assert_ne!(brush, 0);
        assert_eq!(GetBkColor(dc), 0x00FF0000);
        assert_eq!(
            send(root, WM_CTLCOLORSTATIC, dc as usize, control as isize),
            brush
        );
        send(root, WM_CTLCOLORLISTBOX, dc as usize, get(id, 9) as isize);
        assert_eq!(GetBkColor(dc), 0x00FFFFFF);
        send(root, WM_CTLCOLOREDIT, dc as usize, get(id, 2) as isize);
        assert_eq!(GetTextColor(dc), 0x000000FF);
        assert_eq!(GetBkColor(dc), 0x0000FF00);
        // The password box shows its translucent background over the blue,
        // not what is painted after it.
        send(root, WM_CTLCOLOREDIT, dc as usize, get(id, 3) as isize);
        assert_eq!(GetBkColor(dc), 0x00800000);
        send(root, WM_CTLCOLOREDIT, dc as usize, root as isize);
        ReleaseDC(control, dc);
    }
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: Vec::new(),
        }],
    );
    // SAFETY: as above.
    unsafe {
        let control = get(id, 5);
        let dc = GetDC(control);
        send(root, WM_CTLCOLORSTATIC, dc as usize, control as isize);
        assert_eq!(GetBkColor(dc), 0x00FFFFFF);
        ReleaseDC(control, dc);
    }
    // In a half opaque group, the green box and its red text show faded
    // into what is beneath, as a disabled form fades.
    apply(
        id,
        &[Change::Paint {
            surface: 0,
            items: vec![
                DisplayItem::Fill {
                    rect: Rect::new(0.0, 0.0, 200.0, 100.0),
                    color: Color::rgb(0, 0, 255),
                    radii: [Corner::SQUARE; 4],
                },
                DisplayItem::PushOpacity(0.5),
                DisplayItem::Child(NativeChild::Control(2)),
                DisplayItem::PopOpacity,
            ],
        }],
    );
    // SAFETY: as above.
    unsafe {
        let dc = GetDC(root);
        send(root, WM_CTLCOLOREDIT, dc as usize, get(id, 2) as isize);
        assert_eq!(GetBkColor(dc), 0x00808000);
        assert_eq!(GetTextColor(dc), 0x00404080);
        ReleaseDC(root, dc);
    }
    // A styled field's translucent box, painted around its content, shows
    // once: the display list paints it, the control does not again.
    let mut bare = controls()[1].clone();
    bare.state.bare = true;
    bare.state.background = Color::rgba(0, 0, 0, 0.5);
    apply(
        id,
        &[
            Change::UpdateControl {
                control: bare.clone(),
            },
            Change::Paint {
                surface: 0,
                items: vec![
                    DisplayItem::Fill {
                        rect: Rect::new(
                            bare.rect.x - 4.0,
                            bare.rect.y - 4.0,
                            bare.rect.width + 8.0,
                            bare.rect.height + 8.0,
                        ),
                        color: bare.state.background,
                        radii: [Corner::SQUARE; 4],
                    },
                    DisplayItem::Child(NativeChild::Control(2)),
                ],
            },
        ],
    );
    // SAFETY: as above.
    unsafe {
        let dc = GetDC(root);
        send(root, WM_CTLCOLOREDIT, dc as usize, get(id, 2) as isize);
        assert_eq!(GetBkColor(dc), 0x00808080);
        ReleaseDC(root, dc);
    }
    destroy(id);
}

#[test]
fn keyboard_and_focus() {
    let (id, events) = setup();
    // Focus changes would activate the test window.
    let mut focus = Session::new();
    let set_focus = mock!(focus, SetFocus, unsafe extern "system" fn(HWND) -> HWND);
    set_focus.expect().returning(|_| null_mut());
    let button = get(id, 1);
    let text = get(id, 2);
    send(text, WM_SETFOCUS, 0, 0);
    send(text, WM_KILLFOCUS, 0, 0);
    assert_eq!(
        events.take(),
        vec![
            Event::Control {
                id: 2,
                event: ControlEvent::Focus
            },
            Event::Control {
                id: 2,
                event: ControlEvent::Blur
            }
        ]
    );
    assert_eq!(send(text, WM_KEYDOWN, usize::from(VK_TAB), 0), 0);
    assert_eq!(send(button, WM_KEYDOWN, usize::from(VK_RETURN), 0), 0);
    send(text, WM_KEYDOWN, usize::from(VK_RETURN), 0);
    send(text, WM_KEYUP, usize::from(VK_TAB), 0);
    let received = events.take();
    assert_eq!(received.len(), 5, "{received:?}");
    assert!(matches!(&received[0], Event::Key { control: Some(2), key, .. } if key == "Tab"));
    assert_eq!(
        received[2],
        Event::Control {
            id: 1,
            event: ControlEvent::Click
        }
    );
    {
        let next = mock!(
            focus,
            GetNextDlgTabItem,
            unsafe extern "system" fn(HWND, HWND, windows_sys::core::BOOL) -> HWND
        );
        next.expect().once().returning(|_, _, _| null_mut());
        send(text, WM_KEYDOWN, usize::from(VK_TAB), 0);
    }
    events.cancel.set(true);
    assert_eq!(send(text, WM_KEYDOWN, usize::from(VK_TAB), 0), 0);
    events.cancel.set(false);
    assert_eq!(send(text, WM_CHAR, 9, 0), 0);
    assert_eq!(send(text, WM_CHAR, 13, 0), 0);
    send(get(id, 4), WM_CHAR, 13, 0);
    assert_eq!(window_text(get(id, 4)), "\r\na\r\nb");
    send(text, WM_CHAR, 'x' as usize, 0);
    focus_control(id, 2);
    destroy(id);
}

#[test]
fn keyboard_focus_follows_the_page_inside_its_focused_window() {
    let (id, _) = setup();
    let root = crate::window::hwnd(id).unwrap();
    // SAFETY: reads this thread's focus.
    let focused = || unsafe { GetFocus() };
    // SAFETY: clears and sets this thread's focus to its own windows.
    unsafe { SetFocus(null_mut()) };
    // A window without the focus keeps it where it is, and passes it to
    // the page's control when it gains it.
    crate::follow_focus(id, Some(2));
    assert!(focused().is_null());
    // SAFETY: the window belongs to this thread.
    unsafe { SetFocus(root) };
    assert_eq!(focused(), get(id, 2));
    crate::follow_focus(id, Some(2));
    assert_eq!(focused(), get(id, 2));
    // Text for the control types into it; other controls take none.
    assert!(crate::type_into_control(id, 2, "x"));
    assert!(!crate::type_into_control(id, 1, "x"));
    assert!(!crate::type_into_control(id, 4242, "x"));
    crate::follow_focus(id, Some(2));
    crate::follow_focus(id, Some(3));
    assert_eq!(focused(), get(id, 3));
    // Painted elements and blurs take the focus back to the window.
    crate::follow_focus(id, Some(4242));
    assert_eq!(focused(), root);
    focus_control(id, 2);
    crate::follow_focus(id, None);
    assert_eq!(focused(), root);
    crate::follow_focus(999, None);
    destroy(id);
}

#[test]
fn removal_and_order() {
    let (id, _) = setup();
    let root = crate::window::hwnd(id).unwrap();
    apply(
        id,
        &[Change::Order {
            surface: 0,
            controls: vec![12, 1, 99],
        }],
    );
    // SAFETY: walks the children of a live window.
    let top = unsafe { GetWindow(root, GW_CHILD) };
    assert_eq!(top, get(id, 12));
    let removed = get(id, 3);
    apply(
        id,
        &[
            Change::RemoveControl { id: 3 },
            Change::RemoveControl { id: 3 },
        ],
    );
    assert_eq!(hwnd(id, 3), None);
    // SAFETY: IsWindow accepts any handle.
    assert_eq!(unsafe { IsWindow(removed) }, 0);
    destroy(id);
}

#[test]
fn creation_problems_are_skipped() {
    let id = window();
    apply(
        id,
        &[Change::AddControl {
            surface: 42,
            control: control(1, ControlKind::Button, state("x")),
        }],
    );
    assert_eq!(hwnd(id, 1), None);
    let mut session = Session::new();
    let create = mock!(
        session,
        CreateWindowExW,
        unsafe extern "system" fn(
            WINDOW_EX_STYLE,
            PCWSTR,
            PCWSTR,
            WINDOW_STYLE,
            i32,
            i32,
            i32,
            i32,
            HWND,
            HMENU,
            HINSTANCE,
            *const core::ffi::c_void,
        ) -> HWND
    );
    create
        .expect()
        .returning(|_, _, _, _, _, _, _, _, _, _, _, _| null_mut());
    apply(
        id,
        &[Change::AddControl {
            surface: 0,
            control: control(1, ControlKind::Button, state("x")),
        }],
    );
    session.restore();
    assert_eq!(hwnd(id, 1), None);
    assert_eq!(pack(1, 2), 0x0002_0001);
    destroy(id);
}

#[test]
fn text_selection_uses_dom_offsets() {
    let (id, _events) = setup();
    // The text area holds "a\nb", which the native control keeps as "a\r\nb".
    apply(
        id,
        &[Change::Select {
            id: 4,
            start: 1,
            end: 3,
        }],
    );
    let (mut start, mut end) = (0_u32, 0_u32);
    send(
        get(id, 4),
        EM_GETSEL,
        &mut start as *mut u32 as usize,
        &mut end as *mut u32 as isize,
    );
    assert_eq!((start, end), (1, 4));
    assert_eq!(text_selection(id, 4), Some((1, 3)));
    select_text(id, 4, 0, 99);
    assert_eq!(text_selection(id, 4), Some((0, 3)));
    select_text(id, 3, 2, 4);
    assert_eq!(text_selection(id, 3), Some((2, 4)));
    select_text(id, 1, 0, 1);
    assert_eq!(text_selection(id, 1), None);
    assert_eq!(text_selection(id, 99), None);
    assert_eq!(text_selection(id + 1, 4), None);
    destroy(id);
}

/// Whether `work` leaves more or fewer GDI objects in this process, on each
/// of several attempts.
fn gdi_objects_leaked(mut work: impl FnMut()) -> bool {
    use windows_sys::Win32::System::Threading::{
        GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources,
    };
    // SAFETY: the pseudo handle refers to this live test process.
    let count = || unsafe { GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS) };
    (0..50).all(|_| {
        let before = count();
        work();
        count() != before
    })
}

#[test]
fn empty_fields_show_their_placeholder_in_its_color() {
    let window = window();
    let mut edit = control(1, ControlKind::TextBox, state(""));
    edit.state.placeholder = "Search".into();
    edit.state.color = Color::BLACK;
    edit.state.placeholder_color = Color::rgb(168, 171, 178);
    apply(
        window,
        &[Change::AddControl {
            surface: 0,
            control: edit,
        }],
    );
    // The darkest ink of each channel painted over white: the placeholder
    // in its color, not the system's gray of a cue banner.
    let ink = || {
        // SAFETY: a memory DC and bitmap receive the paint and are deleted.
        unsafe {
            let screen = GetDC(null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 200, 40);
            ReleaseDC(null_mut(), screen);
            let previous = SelectObject(dc, bitmap);
            PatBlt(dc, 0, 0, 200, 40, WHITENESS);
            paint_placeholder(window, 1, get(window, 1), dc);
            let mut darkest = [255u32; 3];
            for y in 0..40 {
                for x in 0..200 {
                    let pixel = GetPixel(dc, x, y);
                    for (channel, value) in darkest.iter_mut().enumerate() {
                        *value = (*value).min((pixel >> (channel * 8)) & 0xFF);
                    }
                }
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            darkest
        }
    };
    assert_eq!(ink(), [168, 171, 178]);
    send(
        get(window, 1),
        WM_SETTEXT,
        0,
        wide("typed").as_ptr() as isize,
    );
    assert_eq!(ink(), [255, 255, 255]);
    destroy(window);
}

#[test]
fn fields_smooth_their_text_as_the_page_and_styled_ones_have_no_margins() {
    let window = window();
    let field = |id: u64, kind: ControlKind, bare: bool, grayscale: bool| Change::AddControl {
        surface: 0,
        control: control(
            id,
            kind,
            ControlState {
                bare,
                grayscale,
                ..state("text")
            },
        ),
    };
    apply(
        window,
        &[
            field(1, ControlKind::TextBox, false, false),
            field(2, ControlKind::TextBox, true, true),
            field(3, ControlKind::TextArea, true, false),
        ],
    );
    let quality = |id: u64| {
        let font = send(get(window, id), WM_GETFONT, 0, 0) as HGDIOBJ;
        let mut logfont = LOGFONTW::default();
        // SAFETY: the control's font is live and the buffer is a LOGFONTW.
        unsafe {
            GetObjectW(
                font,
                size_of::<LOGFONTW>() as i32,
                (&raw mut logfont).cast(),
            )
        };
        logfont.lfQuality
    };
    assert_eq!(quality(1), CLEARTYPE_QUALITY);
    assert_eq!(quality(2), ANTIALIASED_QUALITY);
    assert_eq!(quality(3), CLEARTYPE_QUALITY);
    // A native field keeps the margins its font gives it; a styled one's
    // text starts at the edges of its content box.
    let margins = |id: u64| send(get(window, id), EM_GETMARGINS, 0, 0);
    assert_ne!(margins(1), 0);
    assert_eq!(margins(2), 0);
    assert_eq!(margins(3), 0);
    destroy(window);
}

#[test]
fn placeholders_are_painted_as_page_text_is() {
    let window = window();
    for (id, grayscale) in [(1, false), (2, true)] {
        let mut edit = control(id, ControlKind::TextBox, state(""));
        edit.state.placeholder = "Search plugins".into();
        edit.state.placeholder_color = Color::BLACK;
        edit.state.bare = true;
        edit.state.grayscale = grayscale;
        apply(
            window,
            &[Change::AddControl {
                surface: 0,
                control: edit,
            }],
        );
    }
    // The inked pixels over white: whether any has unequal channels, as
    // ClearType's have, and the leftmost inked column.
    let ink = |id: u64| {
        // SAFETY: a memory DC and bitmap receive the paint and are deleted.
        unsafe {
            let screen = GetDC(null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, 200, 40);
            ReleaseDC(null_mut(), screen);
            let previous = SelectObject(dc, bitmap);
            PatBlt(dc, 0, 0, 200, 40, WHITENESS);
            paint_placeholder(window, id, get(window, id), dc);
            let (mut colored, mut left) = (false, i32::MAX);
            for y in 0..40 {
                for x in 0..200 {
                    let pixel = GetPixel(dc, x, y) & 0xff_ffff;
                    if pixel != 0xff_ffff {
                        let [r, g, b, _] = pixel.to_le_bytes();
                        colored |= r != g || g != b;
                        left = left.min(x);
                    }
                }
            }
            SelectObject(dc, previous);
            DeleteObject(bitmap);
            DeleteDC(dc);
            (colored, left)
        }
    };
    let (cleartype, left) = ink(1);
    assert!(cleartype);
    assert!(left <= 1, "{left}");
    let (colored, left) = ink(2);
    assert!(!colored);
    assert!(left <= 1, "{left}");
    destroy(window);
}

#[test]
fn fields_align_their_text_as_text_align_says() {
    use nativeboat_css::types::TextAlign;
    let window = window();
    let mut edit = control(1, ControlKind::TextBox, state("4"));
    edit.state.text_align = TextAlign::Center;
    apply(
        window,
        &[Change::AddControl {
            surface: 0,
            control: edit.clone(),
        }],
    );
    let aligned = || style(get(window, 1)) & (ES_CENTER | ES_RIGHT) as u32;
    assert_eq!(aligned(), ES_CENTER as u32);
    edit.state.text_align = TextAlign::Right;
    apply(
        window,
        &[Change::UpdateControl {
            control: edit.clone(),
        }],
    );
    assert_eq!(aligned(), ES_RIGHT as u32);
    edit.state.text_align = TextAlign::Start;
    apply(window, &[Change::UpdateControl { control: edit }]);
    assert_eq!(aligned(), 0);
    destroy(window);
}

#[test]
fn see_through_fields_show_the_page_their_caret_and_placeholder() {
    use crate::tests::support::Canvas;
    let window = window();
    let items = vec![DisplayItem::Fill {
        rect: Rect::new(0.0, 0.0, 200.0, 100.0),
        color: Color::rgb(0, 0, 255),
        radii: [Corner::SQUARE; 4],
    }];
    let mut field = control(1, ControlKind::TextBox, state("typed"));
    field.rect = Rect::new(0.0, 0.0, 80.0, 30.0);
    field.state.bare = true;
    field.state.color = Color {
        a: 0.0,
        ..Color::BLACK
    };
    apply(
        window,
        &[
            Change::Paint {
                surface: 0,
                items: items.clone(),
            },
            Change::AddControl {
                surface: 0,
                control: field.clone(),
            },
        ],
    );
    let handle = get(window, 1);
    assert!(see_through(handle, 1) && !see_through(handle, 9));
    let mut bounds = RECT::default();
    // SAFETY: the control belongs to this test.
    unsafe { GetClientRect(handle, &mut bounds) };
    // Only the page shows: the field's text never paints.
    for message in [WM_PRINTCLIENT, WM_PRINT] {
        let canvas = Canvas::new(bounds.right, bounds.bottom);
        send(handle, message, canvas.dc as usize, PRF_CLIENT as isize);
        for y in 0..bounds.bottom {
            for x in 0..bounds.right {
                assert_eq!(canvas.pixel(x, y), (0, 0, 255), "{message}: {x},{y}");
            }
        }
    }
    assert_eq!(send(handle, WM_ERASEBKGND, 0, 0), 1);
    send(handle, WM_PAINT, 0, 0);
    // Typing paints the page over what the control drew at once.
    send(handle, WM_CHAR, 'x' as usize, 0);
    // An empty field shows its placeholder over the page.
    field.state.text = String::new();
    field.state.placeholder = "Name".into();
    field.state.placeholder_color = Color::rgb(255, 255, 0);
    apply(
        window,
        &[Change::UpdateControl {
            control: field.clone(),
        }],
    );
    let canvas = Canvas::new(bounds.right, bounds.bottom);
    send(
        handle,
        WM_PRINTCLIENT,
        canvas.dc as usize,
        PRF_CLIENT as isize,
    );
    let placeholder = (0..bounds.bottom).any(|y| {
        (0..bounds.right).any(|x| {
            let (r, g, b) = canvas.pixel(x, y);
            r > 128 && g > 128 && b < 128
        })
    });
    assert!(placeholder, "the placeholder shows");
    // Translucent text shows over its box as much as it covers it.
    field.state.color = Color::rgba(255, 0, 0, 0.5);
    apply(window, &[Change::UpdateControl { control: field }]);
    assert!(!see_through(handle, 1));
    // SAFETY: the DC belongs to the test window and is released.
    unsafe {
        let root = crate::window::hwnd(window).unwrap();
        let dc = GetDC(root);
        send(root, WM_CTLCOLOREDIT, dc as usize, handle as isize);
        let text = GetTextColor(dc);
        let (red, blue) = (text & 0xFF, (text >> 16) & 0xFF);
        assert!(
            (120..=135).contains(&red) && (120..=135).contains(&blue),
            "{text:#x}"
        );
        ReleaseDC(root, dc);
    }
    destroy(window);
}

#[test]
fn pointers_over_controls_reach_the_page_and_take_its_cursor() {
    use crate::PointerKind;
    use nativeboat_css::types::Cursor;
    use std::sync::{Arc, Mutex};
    let _keyboard = crate::tests::support::KeyboardState::neutral();
    let events = record();
    let window = window();
    apply(
        window,
        &[
            Change::AddControl {
                surface: 0,
                control: control(1, ControlKind::CheckBox, state("")),
            },
            Change::AddControl {
                surface: 0,
                control: control(2, ControlKind::TextBox, state("")),
            },
        ],
    );
    let toggle = get(window, 1);
    let mut origin = POINT::default();
    // SAFETY: both windows belong to this test.
    unsafe { MapWindowPoints(toggle, crate::window::hwnd(window).unwrap(), &mut origin, 1) };
    let scale = crate::window::scale(window);
    events.take();
    send(toggle, WM_MOUSEMOVE, MK_SHIFT as usize, pack(3, 4));
    let moves: Vec<_> = events
        .take()
        .into_iter()
        .filter_map(|event| match event {
            Event::Pointer {
                kind: PointerKind::Move,
                x,
                y,
                modifiers,
                ..
            } => Some((x, y, modifiers)),
            _ => None,
        })
        .collect();
    assert_eq!(moves.len(), 1, "{moves:?}");
    let (x, y, modifiers) = moves[0];
    assert!((x - (origin.x + 3) as f32 / scale).abs() < 0.01, "{x}");
    assert!((y - (origin.y + 4) as f32 / scale).abs() < 0.01, "{y}");
    assert!(modifiers.shift && !modifiers.ctrl, "{modifiers:?}");
    // Windows' repeated moves to the same point are dropped, as on the page.
    send(toggle, WM_MOUSEMOVE, MK_SHIFT as usize, pack(3, 4));
    assert!(
        !events
            .take()
            .iter()
            .any(|event| matches!(event, Event::Pointer { .. }))
    );
    let mut session = Session::new();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let seen = shown.clone();
    let set = mock!(
        session,
        SetCursor,
        unsafe extern "system" fn(HCURSOR) -> HCURSOR
    );
    set.expect().returning(move |cursor| {
        seen.lock().unwrap().push(cursor as usize);
        null_mut()
    });
    // Setting the page's cursor shows it at once when the real pointer is
    // over the window; only what the controls show counts here.
    crate::window::set_cursor(window, Cursor::Pointer);
    shown.lock().unwrap().clear();
    assert_eq!(
        send(toggle, WM_SETCURSOR, toggle as usize, HTCLIENT as isize),
        1
    );
    // A text field shows the page's cursor too, which its style makes the
    // text cursor.
    crate::window::set_cursor(window, Cursor::Text);
    let field = get(window, 2);
    send(field, WM_SETCURSOR, field as usize, HTCLIENT as isize);
    let over_controls = shown.lock().unwrap().clone();
    crate::window::set_cursor(window, Cursor::Auto);
    shown.lock().unwrap().clear();
    send(toggle, WM_SETCURSOR, toggle as usize, HTCLIENT as isize);
    let left_to_the_control = shown.lock().unwrap().clone();
    session.restore();
    // SAFETY: loads a shared system cursor.
    let hand = unsafe { LoadCursorW(null_mut(), IDC_HAND) } as usize;
    // SAFETY: loads a shared system cursor.
    let beam = unsafe { LoadCursorW(null_mut(), IDC_IBEAM) } as usize;
    assert_eq!(over_controls, [hand, beam]);
    // SAFETY: loads a shared system cursor.
    let arrow = unsafe { LoadCursorW(null_mut(), IDC_ARROW) } as usize;
    // A control the page leaves the cursor to asks the page's window, as
    // child windows do, which shows the arrow.
    assert_eq!(left_to_the_control, [arrow]);
    // Leaving the control for somewhere outside the window leaves the page.
    let mut point = POINT::default();
    // SAFETY: reads the cursor position and the window under it.
    let outside = unsafe {
        GetCursorPos(&mut point);
        let under = WindowFromPoint(point);
        under.is_null() || GetAncestor(under, GA_ROOT) != crate::window::hwnd(window).unwrap()
    };
    send(toggle, WM_MOUSELEAVE, 0, 0);
    let left = events.take().iter().any(|event| {
        matches!(
            event,
            Event::Pointer {
                kind: PointerKind::Leave,
                ..
            }
        )
    });
    assert_eq!(left, outside);
    destroy(window);
}
