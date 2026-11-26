//! A page shown in a native window. Pointer, wheel, keyboard and
//! input-method input of the window become DOM events of the page's realm,
//! as the prelude's renderer interface (`host.web`) defines them, and the
//! page's scene updates the window's surfaces, controls and caret.

use crate::{DocumentPage, RenderState};
use alloc::{boxed::Box, rc::Rc, vec::Vec};
use core::cell::{Cell, RefCell};
use core::ffi::c_void;
use core::time::Duration;
use nativeboat_css::types::Cursor;
use nativeboat_dom::NodeId;
use nativeboat_ir::{NativeTree, diff};
use nativeboat_layout::Scene;
use nativeboat_layout::resize::Resizing;
use nativeboat_layout::scrollbars::Dragging;
use nativeboat_native::web::Page;
use nativeboat_native::{Key, Property, Realm, Value};
use nativeboat_win32::{
    CompositionKind, ControlEvent, Event, FileDragKind, Handler, MenuItem, MenuKind, Modifiers,
    PointerKind, WindowId,
};

/// A page and the native window that shows it.
pub struct PageWindow {
    page: Rc<DocumentPage>,
    realm: *mut Realm,
    window: WindowId,
    painted: RefCell<Option<Rc<Scene>>>,
    /// The page state, window size and root scrollbars' room the last
    /// rendering showed.
    rendered: Cell<Option<(RenderState, [f32; 4])>>,
    /// The target, button and click count of the last press.
    pressed: Cell<Option<(Option<NodeId>, i16, u32)>>,
    /// The drag of a resize grip under way.
    resizing: Cell<Option<Resizing>>,
    /// Whether a scrollbar the page paints is pressed, and the drag of its
    /// thumb.
    scrollbar: Cell<Option<Option<Dragging>>>,
    /// When an arrow button or track held down on a scrollbar the page
    /// paints scrolls again, and where the pointer is.
    holding: Cell<Option<(f64, (f32, f32))>>,
    cursor: Cell<Cursor>,
    /// Whether the page's realm is running, so input waits.
    busy: Cell<bool>,
    queued: RefCell<Vec<Event>>,
    /// Whether input is reported on standard error, with
    /// `NATIVEBOAT_TRACE_INPUT` set.
    traced: bool,
    /// Where `NATIVEBOAT_DUMP_PAGE` asks for the page's description.
    dump: Option<alloc::string::String>,
    /// The revision and time of the last description written.
    dumped: Cell<Option<((u64, u64), f64)>>,
    /// The frames the page's `<iframe>` elements show, each in a window of
    /// its own inside this one.
    frames: RefCell<Vec<(NodeId, Rc<PageWindow>)>>,
    /// The window of the page whose frame this is.
    parent: Option<alloc::rc::Weak<PageWindow>>,
}

thread_local! {
    /// Every page's window and frame's window, so that a frame finds the
    /// page it is in and input finds the window it arrived at.
    static WINDOWS: RefCell<Vec<alloc::rc::Weak<PageWindow>>> = const { RefCell::new(Vec::new()) };
}

fn registered(found: impl Fn(&PageWindow) -> bool) -> Option<Rc<PageWindow>> {
    WINDOWS.with_borrow(|windows| {
        windows
            .iter()
            .filter_map(alloc::rc::Weak::upgrade)
            .find(|window| found(window))
    })
}

fn register(window: &Rc<PageWindow>) {
    WINDOWS.with_borrow_mut(|windows| {
        windows.retain(|known| known.strong_count() > 0);
        windows.push(Rc::downgrade(window));
    });
}

/// Shows a frame's page, whose scripts run in `realm`, in a window inside
/// the window of the page whose realm is `parent`, at the box of that
/// page's `<iframe>` element `node`.
///
/// # Safety
/// `realm` must stay valid until [`detach_frame`] is called with it.
pub unsafe fn attach_frame(
    parent: *mut Realm,
    node: u32,
    page: Rc<DocumentPage>,
    realm: *mut Realm,
) {
    let Some(host) = registered(|window| window.realm == parent) else {
        return;
    };
    let Ok(window) = nativeboat_win32::create_child(host.window) else {
        return;
    };
    page.attach_window(window);
    let frame = Rc::new(PageWindow::new(
        page,
        realm,
        window,
        Some(Rc::downgrade(&host)),
    ));
    frame.busy.set(host.busy.get());
    register(&frame);
    host.frames.borrow_mut().push((NodeId(node), frame));
}

/// Ends the window of the frame whose realm is `realm`, and those of its
/// own frames, before the realm goes away.
pub fn detach_frame(realm: *mut Realm) {
    let Some(frame) = registered(|window| window.realm == realm) else {
        return;
    };
    if let Some(host) = frame.parent.as_ref().and_then(alloc::rc::Weak::upgrade) {
        host.frames
            .borrow_mut()
            .retain(|(_, known)| !Rc::ptr_eq(known, &frame));
    }
    frame.close_frames();
}

impl PageWindow {
    /// Shows `page`, whose scripts run in `realm`, in the native window
    /// `hwnd`, which another host created and keeps.
    ///
    /// # Safety
    /// `realm` must stay valid for the life of the returned window, and be
    /// used by others only within [`PageWindow::run`]; `hwnd` must be a live
    /// top-level window of this thread.
    pub unsafe fn adopt(
        page: Rc<DocumentPage>,
        realm: *mut Realm,
        hwnd: *mut c_void,
    ) -> Result<PageWindow, nativeboat_win32::Error> {
        // SAFETY: the caller's window is live and kept by its host.
        let window = unsafe { nativeboat_win32::adopt(hwnd) }?;
        page.attach_window(window);
        page.set_transparent(nativeboat_win32::is_transparent(window));
        Ok(PageWindow::new(page, realm, window, None))
    }

    fn new(
        page: Rc<DocumentPage>,
        realm: *mut Realm,
        window: WindowId,
        parent: Option<alloc::rc::Weak<PageWindow>>,
    ) -> PageWindow {
        PageWindow {
            page,
            realm,
            window,
            painted: RefCell::new(None),
            rendered: Cell::new(None),
            pressed: Cell::new(None),
            resizing: Cell::new(None),
            scrollbar: Cell::new(None),
            holding: Cell::new(None),
            cursor: Cell::new(Cursor::Auto),
            busy: Cell::new(false),
            queued: RefCell::new(Vec::new()),
            traced: nativeboat_native::platform::environment_value(
                &nativeboat_native::JsStr::from("NATIVEBOAT_TRACE_INPUT"),
            )
            .is_some(),
            dump: nativeboat_native::platform::environment_value(&nativeboat_native::JsStr::from(
                "NATIVEBOAT_DUMP_PAGE",
            ))
            .map(|path| path.to_rust_lossy()),
            dumped: Cell::new(None),
            frames: RefCell::new(Vec::new()),
            parent,
        }
    }

    /// Destroys the windows of this frame and of its frames.
    fn close_frames(&self) {
        for (_, frame) in core::mem::take(&mut *self.frames.borrow_mut()) {
            frame.close_frames();
        }
        if self.parent.is_some() {
            nativeboat_win32::destroy(self.window);
        }
    }

    /// The native window.
    pub fn id(&self) -> WindowId {
        self.window
    }

    /// The page.
    pub fn page(&self) -> &Rc<DocumentPage> {
        &self.page
    }

    /// Runs the page's realm; input that arrives meanwhile, from message
    /// loops that run inside it, is delivered afterwards.
    pub fn run<R>(&self, work: impl FnOnce(&mut Realm) -> R) -> R {
        self.enter();
        // SAFETY: `adopt`'s contract keeps the realm valid and unaliased.
        let result = work(unsafe { &mut *self.realm });
        self.leave();
        result
    }

    /// Marks the page's realm as running: input waits until [`leave`].
    ///
    /// [`leave`]: PageWindow::leave
    pub fn enter(&self) {
        self.busy.set(true);
        for (_, frame) in self.frames.borrow().iter() {
            frame.enter();
        }
    }

    /// Ends [`enter`] and delivers the input that waited.
    ///
    /// [`enter`]: PageWindow::enter
    pub fn leave(&self) {
        self.busy.set(false);
        self.flush();
        let frames: Vec<_> = self
            .frames
            .borrow()
            .iter()
            .map(|(_, frame)| frame.clone())
            .collect();
        for frame in frames {
            frame.leave();
        }
    }

    /// Delivers the input that waited for the realm.
    fn flush(&self) {
        loop {
            let queued = core::mem::take(&mut *self.queued.borrow_mut());
            if queued.is_empty() {
                return;
            }
            for event in queued {
                self.handle(&event);
            }
        }
    }

    /// The zoom the window's host asks for its page.
    fn host_zoom(&self) -> f32 {
        // A frame shows at its page's zoom.
        if let Some(host) = self.parent.as_ref().and_then(alloc::rc::Weak::upgrade) {
            return host.host_zoom();
        }
        nativeboat_win32::hwnd(self.window).map_or(1.0, |hwnd| {
            nativeboat_native::app_host::zoom(nativeboat_native::app_host::Window::from_raw(
                hwnd as usize,
            )) as f32
        })
    }

    /// The window's client size and the room its root scrollbars take.
    fn size(&self) -> [f32; 4] {
        let (width, height) = nativeboat_win32::client_size(self.window);
        let (across, down) = nativeboat_win32::scrollbar_room(self.window);
        [width, height, across, down]
    }

    /// Scrolls again for an arrow button or track held down once its delay
    /// passed, as Chromium's scrollbars repeat their presses.
    fn repeat_scrollbar(&self, now: f64) {
        let Some((at, (x, y))) = self.holding.get() else {
            return;
        };
        if now < at {
            return;
        }
        let held = self.page.repeat_scrollbar(x, y);
        let next = now + f64::from(nativeboat_ir::scrollbar::REPEAT_DELAYS[1]);
        self.holding.set(held.then_some((next, (x, y))));
    }

    /// Brings the window up to date with the page.
    pub fn render(&self) {
        let now = nativeboat_native::platform::now();
        self.repeat_scrollbar(now);
        self.page.advance_frame(now);
        let zoom = self.host_zoom();
        if zoom != nativeboat_win32::zoom(self.window) {
            nativeboat_win32::set_zoom(self.window, zoom);
        }
        self.page.set_scale(nativeboat_win32::scale(self.window));
        let size = self.size();
        if size[0] > 0.0 && size[1] > 0.0 {
            self.page.set_viewport(size[0], size[1]);
        }
        self.page.set_scrollbar_room((size[2], size[3]));
        self.page.refresh_spelling();
        self.dump();
        self.rendered.set(Some((self.page.render_state(), size)));
        let Some(scene) = self.page.scene() else {
            return;
        };
        let mut painted = self.painted.borrow_mut();
        if !painted
            .as_ref()
            .is_some_and(|painted| Rc::ptr_eq(painted, &scene))
        {
            let empty = NativeTree::default();
            let before = painted.as_ref().map_or(&empty, |painted| &painted.tree);
            let changes = diff(before, &scene.tree);
            nativeboat_win32::apply(self.window, &changes);
            *painted = Some(scene);
        }
        drop(painted);
        // Selections apply after the text they select.
        let selections: Vec<_> = self
            .page
            .take_selections()
            .into_iter()
            .map(|(node, (start, end))| nativeboat_ir::Change::Select {
                id: node.index(),
                start,
                end,
            })
            .collect();
        if !selections.is_empty() {
            nativeboat_win32::apply(self.window, &selections);
        }
        // Controls exist once applied, so focus moves to them afterwards.
        if let Some(focused) = self.page.take_focus_request() {
            nativeboat_win32::follow_focus(self.window, focused.map(|node| u64::from(node.0)));
        }
        self.deliver_transition_events();
        // Frames show at the content boxes of their elements, above the
        // page; a frame whose element is not rendered is hidden.
        let frames: Vec<_> = self.frames.borrow().clone();
        for (node, frame) in frames {
            let rect = self.page.frame_rect(node);
            nativeboat_win32::place_child(frame.window, rect);
            if rect.is_some() {
                frame.render();
            }
        }
    }

    /// Fires the transition events of the frame at the page's elements.
    fn deliver_transition_events(&self) {
        let events = self.page.take_transition_events();
        if events.is_empty() || self.busy.get() {
            return;
        }
        self.run(|realm| {
            for event in events {
                let (method, name) = if event.kind.starts_with("animation") {
                    ("animation", Value::str(&event.name))
                } else {
                    ("transition", Value::str(event.property))
                };
                let _ = realm.web_call(
                    method,
                    &[
                        Value::Number(f64::from(event.node.0)),
                        Value::str(event.kind),
                        name,
                        Value::Number(event.elapsed),
                        Value::str(event.pseudo),
                    ],
                );
            }
        });
    }

    /// With `NATIVEBOAT_DUMP_PAGE` naming a file, writes the page's
    /// description there when it changed, at most once a second.
    fn dump(&self) {
        let Some(path) = &self.dump else {
            return;
        };
        let state = self.page.render_state();
        let now = nativeboat_native::platform::now();
        let mut last = self.dumped.get();
        if last.is_some_and(|(revision, time)| revision == state.0 || now - time < 1000.0) {
            return;
        }
        last = Some((state.0, now));
        self.dumped.set(last);
        nativeboat_native::platform::write_file(path, self.page.describe().as_bytes());
    }

    /// Whether the page or the window changed since the last rendering, or
    /// the page's description is out of date: it is written once its second
    /// has passed, so the last state of a page is described too.
    /// Hosts render at most once a frame, and wake for the next frame while
    /// this holds.
    pub fn needs_render(&self) -> bool {
        if self
            .frames
            .borrow()
            .iter()
            .any(|(_, frame)| frame.needs_render())
        {
            return true;
        }
        let size = self.size();
        let spelling = self
            .page
            .spelling_due()
            .is_some_and(|at| nativeboat_native::platform::now() >= at);
        spelling
            || self.holding.get().is_some()
            || self.page.animating()
            || self.page.selects_text()
            || self.rendered.get() != Some((self.page.render_state(), size))
            || self.host_zoom() != nativeboat_win32::zoom(self.window)
            || (self.dump.is_some()
                && self
                    .dumped
                    .get()
                    .is_some_and(|(revision, _)| revision != self.page.render_state().0))
    }

    /// When the page renders again without other changes: a pending spell
    /// check of the word typed last.
    pub fn wake_at(&self) -> Option<f64> {
        self.page.spelling_due()
    }

    /// Delivers platform input; returns whether the page cancelled its
    /// default action.
    pub fn handle(&self, event: &Event) -> bool {
        if self.busy.get() {
            if self.traced {
                nativeboat_native::platform::write_stderr(&alloc::format!(
                    "nativeboat: input waits for the page: {event:?}\n"
                ));
            }
            self.queued.borrow_mut().push(event.clone());
            return false;
        }
        self.busy.set(true);
        let cancelled = self.deliver(event);
        self.busy.set(false);
        self.flush();
        if let Some(select) = self.page.take_select_request() {
            self.show_options(select);
        }
        cancelled
    }

    /// Shows the options of a select the page draws below it, the selected
    /// one checked; the one chosen reaches the page as the control's
    /// choice.
    fn show_options(&self, select: NodeId) {
        let Some((labels, selected, rect)) = self.page.select_options(select) else {
            return;
        };
        let items: Vec<MenuItem> = labels
            .into_iter()
            .enumerate()
            .map(|(index, label)| MenuItem {
                id: index as u16 + 1,
                // An ampersand shows as itself, not as an access key.
                label: label.replace('&', "&&"),
                kind: MenuKind::Radio,
                enabled: true,
                checked: selected == Some(index),
                ..MenuItem::default()
            })
            .collect();
        if items.is_empty() || items.len() >= 0xEFFF {
            return;
        }
        let chosen = nativeboat_win32::popup_menu(
            Some(self.window),
            core::ptr::null_mut(),
            &items,
            Some((rect.x, rect.y + rect.height)),
        );
        if let Some(id) = chosen {
            self.handle(&Event::Control {
                id: select.index(),
                event: ControlEvent::Select(alloc::vec![usize::from(id - 1)]),
            });
        }
    }

    fn deliver(&self, event: &Event) -> bool {
        match event {
            Event::Pointer {
                kind,
                button,
                buttons,
                count,
                x,
                y,
                modifiers,
            } => self.pointer(*kind, *button, *buttons, *count, (*x, *y), modifiers),
            Event::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                modifiers,
            } => {
                let hit = self.page.element_from_point(*x, *y);
                let target = node(hit);
                let init = self.init(
                    &[
                        ("clientX", *x),
                        ("clientY", *y),
                        ("deltaX", *delta_x),
                        ("deltaY", *delta_y),
                        ("deltaMode", 0.0),
                    ],
                    &[],
                    modifiers,
                );
                let cancelled = self.call("wheel", &[target, init]);
                // Ctrl and the wheel ask to zoom instead of scrolling, which
                // the application decides on, as Electron leaves it.
                if !cancelled && modifiers.ctrl && *delta_y != 0.0 {
                    if let Some(hwnd) = nativeboat_win32::hwnd(self.window) {
                        nativeboat_native::app_host::zoom_changed(
                            nativeboat_native::app_host::Window::from_raw(hwnd as usize),
                            *delta_y < 0.0,
                        );
                    }
                    return true;
                }
                // Scroll containers painted in place scroll here; native
                // ones scroll themselves.
                if !cancelled
                    && let Some(hit) = hit
                    && self.page.wheel_in_place(hit, (*delta_x, *delta_y))
                {
                    return true;
                }
                cancelled
            }
            Event::Key {
                down,
                key,
                code,
                repeat,
                modifiers,
                control,
            } => {
                let init = self.init(
                    &[],
                    &[
                        ("key", Value::str(key)),
                        ("code", Value::str(code)),
                        ("repeat", Value::Boolean(*repeat)),
                    ],
                    modifiers,
                );
                self.call(
                    "key",
                    &[
                        Value::Boolean(*down),
                        init,
                        Value::Boolean(control.is_some()),
                    ],
                )
            }
            Event::Text(text) => {
                // Text reaching the window while the page's focus is on a
                // text control belongs to the control.
                let focused = self
                    .page
                    .document()
                    .and_then(|document| document.borrow().focused());
                if focused.is_some_and(|node| {
                    nativeboat_win32::type_into_control(self.window, u64::from(node.0), text)
                }) {
                    return true;
                }
                self.call("text", &[Value::str(text)])
            }
            Event::Composition { kind, data, cursor } => {
                let kind = match kind {
                    CompositionKind::Start => "start",
                    CompositionKind::Update => "update",
                    CompositionKind::End => "end",
                };
                self.call(
                    "composition",
                    &[
                        Value::str(kind),
                        Value::str(data),
                        Value::Number(*cursor as f64),
                    ],
                )
            }
            Event::Scroll { surface, x, y } => {
                let target = (*surface != 0).then_some(NodeId(*surface as u32));
                if let Some(document) = self.page.document() {
                    let mut dom = document.borrow_mut();
                    if let Some(scrolled) = target.or_else(|| dom.document_element()) {
                        dom.set_scroll(scrolled, *x, *y);
                    }
                }
                self.call("scroll", &[node(target)]);
                false
            }
            Event::Control { id, event } => {
                let target = Value::Number(f64::from(*id as u32));
                match event {
                    ControlEvent::Click => self.call("activate", &[target]),
                    ControlEvent::Input(text) => self.call("input", &[target, Value::str(text)]),
                    ControlEvent::Select(indexes) => {
                        // SAFETY: as in `run`; the realm only makes the array.
                        let realm = unsafe { &mut *self.realm };
                        let indexes = indexes
                            .iter()
                            .map(|index| Value::Number(*index as f64))
                            .collect();
                        let indexes = Value::Object(realm.array(indexes));
                        self.call("select", &[target, indexes])
                    }
                    ControlEvent::Value(value) => {
                        self.call("value", &[target, Value::Number(*value)])
                    }
                    ControlEvent::Focus => self.call("focus", &[target]),
                    ControlEvent::Blur => self.call("focus", &[Value::Null]),
                };
                false
            }
            Event::Resized { width, height } => {
                self.page.set_viewport(*width, *height);
                self.page
                    .set_scrollbar_room(nativeboat_win32::scrollbar_room(self.window));
                false
            }
            // Files dragged from Explorer: their paths while they hover,
            // their contents once dropped.
            Event::FileDrag {
                kind,
                paths,
                strings,
                x,
                y,
                modifiers,
            } => {
                let kind = match kind {
                    FileDragKind::Enter => "enter",
                    FileDragKind::Over => "over",
                    FileDragKind::Leave => "leave",
                };
                let target = node(self.page.element_from_point(*x, *y));
                let files = self.files(paths, false);
                let init = self.init(&[("clientX", *x), ("clientY", *y)], &[], modifiers);
                let strings = self.strings(strings);
                self.call(
                    "dragFiles",
                    &[Value::str(kind), target, files, init, strings],
                )
            }
            Event::FilesDropped {
                paths,
                strings,
                x,
                y,
            } => {
                let target = node(self.page.element_from_point(*x, *y));
                let files = self.files(paths, true);
                let init = self.init(
                    &[("clientX", *x), ("clientY", *y)],
                    &[],
                    &Modifiers::default(),
                );
                let strings = self.strings(strings);
                self.call("dropFiles", &[target, files, init, strings])
            }
            // Window, menu and system events are the main process's.
            _ => false,
        }
    }

    fn pointer(
        &self,
        kind: PointerKind,
        button: i16,
        buttons: u16,
        count: u32,
        (x, y): (f32, f32),
        modifiers: &Modifiers,
    ) -> bool {
        let target = if kind == PointerKind::Leave {
            None
        } else {
            self.page.element_from_point(x, y)
        };
        let document = self.page.document();
        if let Some(document) = &document {
            document.borrow_mut().set_hovered(target);
        }
        let numbers = [
            ("clientX", x),
            ("clientY", y),
            ("screenX", x),
            ("screenY", y),
            ("button", f32::from(button)),
            ("buttons", f32::from(buttons)),
            ("detail", count as f32),
        ];
        let init = self.init(&numbers, &[], modifiers);
        let send =
            |kind: &str, init: Value| self.call("pointer", &[Value::str(kind), node(target), init]);
        // A control in a rotated or skewed box is kept out of sight: the
        // pointer where its image shows reaches it.
        if let Some(control) = target.filter(|target| self.page.takes_pointer(*target))
            && nativeboat_win32::forward_to_control(
                self.window,
                control.index(),
                (kind, button, buttons, count),
                (x, y),
            )
        {
            return true;
        }
        match kind {
            PointerKind::Move => {
                // A pressed scrollbar keeps the pointer until it is released.
                if let Some(pressed) = self.scrollbar.get() {
                    match pressed {
                        Some(dragging) => self.page.drag_scrollbar(&dragging, x, y),
                        // A held arrow button or track repeats while the
                        // pointer is over it.
                        None => {
                            self.page.hover_scrollbar(Some((x, y)));
                            if let Some((at, _)) = self.holding.get() {
                                self.holding.set(Some((at, (x, y))));
                            }
                        }
                    }
                    return true;
                }
                match self.resizing.get() {
                    Some(resizing) => self.page.resize(&resizing, x, y),
                    None => {
                        self.page.hover_scrollbar(Some((x, y)));
                        self.update_cursor(target, (x, y));
                    }
                }
                send("move", init)
            }
            PointerKind::Leave => {
                if self.resizing.get().is_none() {
                    self.update_cursor(None, (x, y));
                }
                self.page.hover_scrollbar(None);
                send("leave", init)
            }
            PointerKind::Down => {
                // Pressing a resize grip starts resizing its box instead of
                // reaching the page, as in Chromium.
                if button == 0
                    && let Some(resizing) = self.page.start_resize(x, y)
                {
                    self.resizing.set(Some(resizing));
                    self.pressed.set(None);
                    return true;
                }
                // So does pressing the scrollbar of a scroll container the
                // page paints in place: it scrolls, or its thumb drags.
                if button == 0
                    && let Some(dragging) = self.page.press_scrollbar(x, y)
                {
                    self.scrollbar.set(Some(dragging));
                    if dragging.is_none() {
                        let delay = nativeboat_ir::scrollbar::REPEAT_DELAYS[0];
                        let at = nativeboat_native::platform::now() + f64::from(delay);
                        self.holding.set(Some((at, (x, y))));
                    }
                    self.pressed.set(None);
                    return true;
                }
                if let Some(document) = &document {
                    document.borrow_mut().set_active(target);
                }
                self.pressed.set(Some((target, button, count)));
                send("down", init)
            }
            PointerKind::Up => {
                if self.scrollbar.take().is_some() {
                    self.holding.set(None);
                    self.page.release_scrollbar();
                    return true;
                }
                if self.resizing.take().is_some() {
                    self.update_cursor(target, (x, y));
                }
                if let Some(document) = &document {
                    document.borrow_mut().set_active(None);
                }
                let cancelled = send("up", init);
                if let Some((pressed, pressed_button, clicks)) = self.pressed.take()
                    && pressed == target
                    && pressed_button == button
                {
                    let mut numbers = numbers;
                    numbers[6].1 = clicks as f32;
                    send("click", self.init(&numbers, &[], modifiers));
                    if clicks == 2 && button == 0 {
                        send("dblclick", self.init(&numbers, &[], modifiers));
                    }
                }
                cancelled
            }
        }
    }

    fn update_cursor(&self, target: Option<NodeId>, (x, y): (f32, f32)) {
        let cursor = target.map_or(Cursor::Auto, |target| {
            self.page
                .resize_cursor(x, y)
                .unwrap_or_else(|| self.page.cursor(target))
        });
        if cursor != self.cursor.replace(cursor) {
            nativeboat_win32::set_cursor(self.window, cursor);
        }
    }

    /// Dragged files for the page: their paths and, with `contents`, what
    /// they hold, in base64.
    fn files(&self, paths: &[alloc::string::String], contents: bool) -> Value {
        // SAFETY: as in `run`; the realm only makes the objects.
        let realm = unsafe { &mut *self.realm };
        let mut entries = Vec::with_capacity(paths.len());
        for path in paths {
            let entry = realm.object();
            entry.define(Key::name("path"), Property::data(Value::str(path)));
            if contents {
                let bytes = nativeboat_native::platform::read_file(path).unwrap_or_default();
                let encoded = nativeboat_native::base64_encode(&bytes, false);
                entry.define(Key::name("bytes"), Property::data(Value::str(&encoded)));
            }
            entries.push(Value::Object(entry));
        }
        Value::Object(realm.array(entries))
    }

    /// Dragged text for the page: `[type, text]` pairs.
    fn strings(&self, strings: &[(alloc::string::String, alloc::string::String)]) -> Value {
        // SAFETY: as in `run`; the realm only makes the arrays.
        let realm = unsafe { &mut *self.realm };
        let mut pairs = Vec::with_capacity(strings.len());
        for (kind, text) in strings {
            let pair = realm.array(alloc::vec![Value::str(kind), Value::str(text)]);
            pairs.push(Value::Object(pair));
        }
        Value::Object(realm.array(pairs))
    }

    /// An event init dictionary.
    fn init(
        &self,
        numbers: &[(&str, f32)],
        values: &[(&str, Value)],
        modifiers: &Modifiers,
    ) -> Value {
        // SAFETY: as in `run`; the realm only makes the object.
        let realm = unsafe { &mut *self.realm };
        let init = realm.object();
        let flags = [
            ("ctrlKey", modifiers.ctrl),
            ("shiftKey", modifiers.shift),
            ("altKey", modifiers.alt),
            ("metaKey", modifiers.meta),
        ];
        let entries = numbers
            .iter()
            .map(|(name, number)| (*name, Value::Number(f64::from(*number))))
            .chain(values.iter().cloned())
            .chain(flags.map(|(name, flag)| (name, Value::Boolean(flag))));
        for (name, value) in entries {
            init.define(Key::name(name), Property::data(value));
        }
        Value::Object(init)
    }

    /// Calls the renderer interface; uncaught errors are reported as the
    /// page's, and a true result cancels the input's default action.
    fn call(&self, method: &str, args: &[Value]) -> bool {
        // SAFETY: as in `run`.
        let realm = unsafe { &mut *self.realm };
        let outcome = realm.web_call(method, args);
        if self.traced {
            // Pointer and wheel events name their target element.
            let target = matches!(method, "pointer" | "wheel" | "contextMenu")
                .then(|| {
                    args.iter().find_map(|argument| match argument {
                        Value::Number(id) => Some(NodeId(*id as u32)),
                        _ => None,
                    })
                })
                .flatten()
                .and_then(|id| {
                    let document = self.page.document()?;
                    let dom = document.borrow();
                    let name = alloc::string::String::from(dom.local_name(id)?);
                    let mut described = alloc::format!("<{name}");
                    for attribute in ["id", "class"] {
                        if let Some(value) = dom.get_attribute(id, attribute) {
                            described.push_str(&alloc::format!(" {attribute}=\"{value}\""));
                        }
                    }
                    described.push('>');
                    Some(described)
                });
            trace(realm, method, args, &outcome);
            if let Some(target) = target {
                nativeboat_native::platform::write_stderr(&alloc::format!(
                    "nativeboat: input target {target}
"
                ));
            }
        }
        match outcome {
            Ok(value) => matches!(value, Value::Boolean(true)),
            Err(thrown) => {
                realm.queue_host_error(thrown);
                false
            }
        }
    }
}

impl core::fmt::Debug for PageWindow {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PageWindow")
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl core::fmt::Debug for PageWindows {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_tuple("PageWindows")
            .field(&self.0.borrow().len())
            .finish()
    }
}

/// Reports a call of the renderer interface: its primitive arguments, the
/// key and position of input objects, and its outcome.
fn trace(realm: &mut Realm, method: &str, args: &[Value], outcome: &nativeboat_native::Completion) {
    let mut line = alloc::format!("nativeboat: input {method}");
    for argument in args {
        line.push(' ');
        if let Value::Object(_) = argument {
            line.push('{');
            for name in ["key", "clientX", "clientY", "button", "detail"] {
                let value = nativeboat_native::ops::get(realm, argument, &Key::name(name))
                    .unwrap_or(Value::Undefined);
                if !matches!(value, Value::Undefined) {
                    line.push_str(&alloc::format!(" {name}={}", text(realm, &value)));
                }
            }
            line.push_str(" }");
        } else {
            line.push_str(&text(realm, argument));
        }
    }
    match outcome {
        Ok(value) => line.push_str(&alloc::format!(" -> {}", text(realm, value))),
        Err(thrown) => line.push_str(&alloc::format!(" threw {}", text(realm, &thrown.0))),
    }
    line.push('\n');
    nativeboat_native::platform::write_stderr(&line);
}

fn text(realm: &mut Realm, value: &Value) -> alloc::string::String {
    nativeboat_native::ops::to_string(realm, value)
        .map_or_else(|_| "?".into(), |text| text.to_rust_lossy())
}

fn node(node: Option<NodeId>) -> Value {
    node.map_or(Value::Null, |node| Value::Number(f64::from(node.0)))
}

/// The pages of a process's windows, which receive their input.
#[derive(Clone, Default)]
pub struct PageWindows(Rc<RefCell<Vec<Rc<PageWindow>>>>);

impl PageWindows {
    /// Adds a window and makes these windows receive platform input.
    pub fn add(&self, window: Rc<PageWindow>) {
        register(&window);
        self.0.borrow_mut().push(window);
        nativeboat_win32::set_handler(Box::new(self.clone()));
        // The win32 handler keeps a clone, so the list outlives the loop.
        nativeboat_native::app_host::set_page_text_handler(
            Rc::as_ptr(&self.0).cast_mut().cast(),
            Some(page_text),
        );
    }

    /// The window showing a native window.
    pub fn get(&self, window: WindowId) -> Option<Rc<PageWindow>> {
        self.0
            .borrow()
            .iter()
            .find(|candidate| candidate.window == window)
            .cloned()
            .or_else(|| registered(|candidate| candidate.window == window))
    }
}

/// Calls a text method of the page shown in a native window.
///
/// # Safety
/// `context` is the list of a live `PageWindows`, used on its UI thread.
unsafe fn page_text(context: *mut c_void, window: usize, method: &str, text: &[u16]) -> bool {
    // SAFETY: guaranteed by the caller.
    let windows = unsafe { &*context.cast::<RefCell<Vec<Rc<PageWindow>>>>() };
    let page = windows
        .borrow()
        .iter()
        .find(|page| {
            nativeboat_win32::hwnd(page.window).is_some_and(|hwnd| hwnd as usize == window)
        })
        .cloned()
        .or_else(|| {
            registered(|page| {
                nativeboat_win32::hwnd(page.window).is_some_and(|hwnd| hwnd as usize == window)
            })
        });
    page.is_some_and(|page| {
        page.call(
            method,
            &[Value::String(nativeboat_native::JsStr::from_units(text))],
        )
    })
}

impl Handler for PageWindows {
    fn event(&mut self, window: WindowId, event: Event) -> bool {
        if matches!(event, Event::Destroyed) {
            self.0
                .borrow_mut()
                .retain(|candidate| candidate.window != window);
            return false;
        }
        let Some(page) = self.get(window) else {
            if nativeboat_native::platform::environment_value(&nativeboat_native::JsStr::from(
                "NATIVEBOAT_TRACE_INPUT",
            ))
            .is_some()
            {
                nativeboat_native::platform::write_stderr(&alloc::format!(
                    "nativeboat: input for a window without a page: {event:?}\n"
                ));
            }
            return false;
        };
        page.handle(&event)
    }

    fn idle(&mut self) -> Option<Duration> {
        None
    }

    fn above_control(&mut self, window: WindowId, control: u64, x: f32, y: f32) -> bool {
        let (Some(page), Ok(control)) = (self.get(window), u32::try_from(control)) else {
            return false;
        };
        page.page.above_control(NodeId(control), x, y)
    }
}

/// The host loop's key filter: offers keys for the pages' windows to the
/// main realm's `before-input-event` listeners, which may keep them from the
/// page. `context` is the main realm.
///
/// # Safety
/// `context` is null or points to the live main `Realm`, used only on the UI
/// thread between its own runs.
pub unsafe extern "system" fn electron_before_input(
    context: *mut c_void,
    hwnd: usize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> bool {
    if context.is_null() {
        return false;
    }
    let Some((window, input)) = nativeboat_win32::keyboard_input(hwnd, message, wparam, lparam)
    else {
        return false;
    };
    let Some(root) = nativeboat_win32::hwnd(window) else {
        return false;
    };
    // SAFETY: guaranteed by the caller.
    let realm = unsafe { &mut *context.cast::<Realm>() };
    realm.electron_before_input(
        root as usize,
        &nativeboat_native::electron_window::KeyInput {
            down: input.down,
            key: &input.key,
            code: &input.code,
            repeat: input.repeat,
            composing: input.composing,
            shift: input.modifiers.shift,
            control: input.modifiers.ctrl,
            alt: input.modifiers.alt,
            meta: input.modifiers.meta,
            location: input.location,
            caps_lock: input.caps_lock,
            num_lock: input.num_lock,
        },
    )
}

#[cfg(test)]
mod tests;
