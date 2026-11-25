//! The renderer of a page: runs its scripts with the web APIs, renders the
//! document into native UI tree changes and turns platform input into DOM
//! events.
//!
//! The renderer is driven by its owner: [`Renderer::start`] runs the page's
//! scripts, [`Renderer::handle`] delivers input, [`Renderer::tick`] runs
//! timers and animation frames, [`Renderer::render`] returns the changes to
//! apply to the platform window and [`Renderer::take_requests`] what else the
//! platform should do (focus, cursor).

mod frame;
mod hit;
use nativeboat_layout::motion;
use nativeboat_layout::resize::{self, Resizing};
use nativeboat_layout::scrollbars::{self, Dragging};

pub use frame::{Images, LoadStorage, SaveStorage};
pub use hit::NativeHitTest;

use base64::Engine;
use boa_engine::object::builtins::JsArray;
use boa_engine::{JsObject, JsString, JsValue, js_string};
use frame::{Frame, PageView};
use nativeboat_css::SystemColors;
use nativeboat_css::types::Cursor;
use nativeboat_dom::{Dom, NodeId};
use nativeboat_ir::{Change, NativeTree, Page, Script, diff};
use nativeboat_js::{Host, Level, Sink};
use nativeboat_layout::{Scene, TextMeasurer};
use nativeboat_win32::{CompositionKind, ControlEvent, Event, Modifiers, PointerKind};
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The interval between animation frames.
const FRAME: Duration = Duration::from_millis(16);

/// How a renderer is set up.
pub struct Options {
    /// The page URL.
    pub url: String,
    /// The URL of the application root; asset paths are relative to it.
    pub base: String,
    /// The viewport size in CSS pixels.
    pub viewport: (f32, f32),
    /// Device pixels per CSS pixel.
    pub scale: f32,
    /// Whether the system uses a dark color scheme.
    pub dark: bool,
    /// Measures text with the platform's fonts.
    pub measurer: Rc<dyn TextMeasurer>,
    /// Natural sizes of the application's images by asset key.
    pub images: Images,
    /// Loads saved `localStorage` contents.
    pub load_storage: LoadStorage,
    /// Saves `localStorage` contents.
    pub save_storage: SaveStorage,
    /// Receives console output.
    pub sink: Sink,
    /// The platform window showing the page.
    pub window: nativeboat_win32::WindowId,
}

impl fmt::Debug for Options {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Options")
            .field("url", &self.url)
            .field("viewport", &self.viewport)
            .finish_non_exhaustive()
    }
}

/// What the platform should do after the renderer handled input or ran
/// scripts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Requests {
    /// Moves the keyboard focus to a control, or to the window itself.
    pub focus: Option<Option<u64>>,
    /// Shows a cursor over the window's content.
    pub cursor: Option<Cursor>,
    /// Shows the options of a `<select>` the page draws.
    pub select: Option<SelectRequest>,
}

/// The options of a drawn `<select>`, to show below it; the choice comes
/// back as a [`ControlEvent::Select`](nativeboat_win32::ControlEvent) of its id.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectRequest {
    /// The element's id.
    pub id: u64,
    /// Its border box in window coordinates.
    pub rect: nativeboat_ir::Rect,
    /// The option labels.
    pub options: Vec<String>,
    /// The selected option.
    pub selected: Option<usize>,
}

/// A page with its document, scripts and rendering state.
pub struct Renderer {
    host: Host,
    frame: Rc<RefCell<Frame>>,
    sink: Sink,
    empty_tree: NativeTree,
    /// The immutable scene last submitted to the platform. Its tree is also
    /// the diff baseline; retaining it avoids duplicating every display item.
    submitted: Option<Rc<Scene>>,
    hits: Rc<RefCell<hit::State>>,
    pressed: Option<(Option<NodeId>, i16, u32)>,
    /// The drag of a resize grip under way.
    resizing: Option<Resizing>,
    /// Whether a scrollbar the page paints is pressed, and the drag of its
    /// thumb.
    scrollbar: Option<Option<Dragging>>,
    cursor: Cursor,
    cursor_changed: bool,
    started: Instant,
    last_frame: Option<Instant>,
    /// The title and the document version it was read at; every update
    /// asks for it, and finding it walks the document.
    title: RefCell<(u64, String)>,
}

impl fmt::Debug for Renderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Renderer").finish_non_exhaustive()
    }
}

fn node_value(node: Option<NodeId>) -> JsValue {
    node.map_or(JsValue::null(), |node| JsValue::from(node.0))
}

fn modifier_fields(modifiers: &Modifiers) -> serde_json::Value {
    json!({
        "ctrlKey": modifiers.ctrl,
        "shiftKey": modifiers.shift,
        "altKey": modifiers.alt,
        "metaKey": modifiers.meta,
    })
}

fn merge(mut base: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    if let (Some(base), serde_json::Value::Object(extra)) = (base.as_object_mut(), extra) {
        base.extend(extra);
    }
    base
}

impl Renderer {
    /// Creates the renderer of a page; scripts run with [`Renderer::start`].
    pub fn new(page: &Page, options: Options) -> Renderer {
        let started = Instant::now();
        let dom = Rc::new(RefCell::new(Dom::from_document(&page.document)));
        let mut environment = nativeboat_css::StyleEnvironment::default();
        environment.media.width = f64::from(options.viewport.0);
        environment.media.height = f64::from(options.viewport.1);
        environment.media.resolution = f64::from(options.scale);
        environment.media.dark = options.dark;
        environment.standard_font = options.measurer.standard_family();
        environment.system = if options.dark {
            SystemColors::DARK
        } else {
            SystemColors::LIGHT
        };
        let frame = Rc::new(RefCell::new(Frame {
            dom: dom.clone(),
            url: options.url,
            base: options.base,
            author: page
                .styles
                .iter()
                .map(|style| style.sheet.clone())
                .collect(),
            inserted: BTreeMap::new(),
            parsed: HashMap::new(),
            sheets: 0,
            environment,
            measurer: options.measurer,
            images: options.images,
            load_storage: options.load_storage,
            save_storage: options.save_storage,
            rendered: None,
            styled: None,
            motion: motion::Motion::default(),
            clock: Rc::new(move || started.elapsed().as_secs_f64()),
            focus: None,
            window: options.window,
            selections: BTreeMap::new(),
            canvases: HashMap::new(),
            select: None,
        }));
        let mut host = Host::new(false, options.sink.clone());
        let hits = Rc::new(RefCell::new(hit::State {
            scene: None,
            dom: dom.clone(),
        }));
        nativeboat_web::install(&mut host, dom, Rc::new(PageView(frame.clone())));
        Renderer {
            host,
            frame,
            sink: options.sink,
            empty_tree: NativeTree::default(),
            submitted: None,
            title: RefCell::new((u64::MAX, String::new())),
            hits,
            pressed: None,
            resizing: None,
            scrollbar: None,
            cursor: Cursor::Auto,
            cursor_changed: false,
            started,
            last_frame: None,
        }
    }

    /// The page's JavaScript host, for defining builtins before scripts run.
    pub fn host(&mut self) -> &mut Host {
        &mut self.host
    }

    /// The page's document.
    pub fn dom(&self) -> Rc<RefCell<Dom>> {
        self.frame.borrow().dom.clone()
    }

    /// What the renderer last put on its window.
    pub fn tree(&self) -> &NativeTree {
        self.submitted
            .as_deref()
            .map_or(&self.empty_tree, |scene| &scene.tree)
    }

    /// A live probe of the last native scene submitted by this renderer.
    pub fn native_hit_test(&self) -> NativeHitTest {
        NativeHitTest(Rc::downgrade(&self.hits))
    }

    /// Calls a method of the web APIs' renderer interface and runs the
    /// microtasks it queued.
    fn web(&mut self, method: &str, args: &[JsValue]) -> JsValue {
        let web = self
            .host
            .host_object()
            .get(js_string!("web"), self.host.context())
            .expect("the web APIs are installed");
        let function = web
            .as_object()
            .expect("the renderer interface is an object")
            .get(JsString::from(method), self.host.context())
            .expect("renderer interface methods are plain properties");
        let result = self.host.call(&function, &web, args).unwrap_or_default();
        self.host.run_microtasks();
        result
    }

    /// Runs the preload script, whose modules also see the properties of
    /// `scope`, and the page's scripts, then fires the document's load
    /// events. Script errors are reported, not returned.
    pub fn start(&mut self, page: &Page, preload: Option<&Script>, scope: Option<&JsObject>) {
        if let Some(preload) = preload
            && let Err(error) = self.host.load_scoped(preload, &[], "/", scope)
        {
            (self.sink)(Level::Error, &format!("Uncaught {error}"));
        }
        if let Err(error) = self.host.load(&page.scripts, &page.module_entries, "/") {
            (self.sink)(Level::Error, &format!("Uncaught {error}"));
        }
        self.web("ready", &[]);
    }

    /// The changes that remove everything the renderer put on its window.
    pub fn clear(&mut self) -> Vec<Change> {
        self.hits.borrow_mut().scene = None;
        let changes = diff(self.tree(), &self.empty_tree);
        self.submitted = None;
        changes
    }

    /// The changes that bring the platform window up to date.
    pub fn render(&mut self) -> Vec<Change> {
        let scene = self.frame.borrow_mut().rendered().scene.clone();
        let mut changes = diff(self.tree(), &scene.tree);
        self.submitted = Some(scene.clone());
        self.hits.borrow_mut().scene = Some(scene);
        // Selections apply after the text they select.
        let selections = std::mem::take(&mut self.frame.borrow_mut().selections);
        changes.extend(
            selections
                .into_iter()
                .map(|(node, (start, end))| Change::Select {
                    id: node.index(),
                    start,
                    end,
                }),
        );
        changes
    }

    /// The document title.
    pub fn title(&self) -> String {
        let dom = self.dom();
        let dom = dom.borrow();
        let mut cached = self.title.borrow_mut();
        if cached.0 != dom.version() {
            let title = dom
                .get_elements_by_tag_name(Dom::DOCUMENT, "title")
                .first()
                .and_then(|title| dom.text_content(*title))
                .unwrap_or_default();
            *cached = (
                dom.version(),
                title.split_whitespace().collect::<Vec<_>>().join(" "),
            );
        }
        cached.1.clone()
    }

    /// Takes what the platform should do next.
    pub fn take_requests(&mut self) -> Requests {
        let focus = self.frame.borrow_mut().focus.take();
        let dom = self.dom();
        let dom = dom.borrow();
        let mut frame = self.frame.borrow_mut();
        let styles = &frame.rendered().styles;
        let focus = focus.map(|node| {
            node.filter(|node| {
                nativeboat_layout::native_control(&dom, *node, styles.get(*node).map(Rc::as_ref))
                    .is_some()
            })
            .map(NodeId::index)
        });
        let select = frame.select.take().and_then(|node| {
            let rect = frame.rendered().scene.client_rect(node)?;
            let options = dom.options(node);
            let selected = dom.selected_options(node);
            Some(SelectRequest {
                id: node.index(),
                rect,
                options: options
                    .iter()
                    .map(|option| {
                        let text = dom.text_content(*option).unwrap_or_default();
                        text.split_whitespace().collect::<Vec<_>>().join(" ")
                    })
                    .collect(),
                selected: options.iter().position(|option| selected.contains(option)),
            })
        });
        let cursor = self.cursor_changed.then_some(self.cursor);
        self.cursor_changed = false;
        Requests {
            focus,
            cursor,
            select,
        }
    }

    /// Delivers finished background work, runs due timers and, at most every
    /// frame interval, animation frames.
    pub fn tick(&mut self) {
        self.host.run_background();
        self.host.run_timers();
        let now = Instant::now();
        let (active, events) = {
            let mut frame = self.frame.borrow_mut();
            frame.rendered();
            (
                frame.motion.active(),
                std::mem::take(&mut frame.motion.events),
            )
        };
        for event in events {
            self.web(
                "transition",
                &[
                    JsValue::from(event.node.0),
                    JsValue::from(JsString::from(event.kind)),
                    JsValue::from(JsString::from(event.property)),
                    JsValue::from(event.elapsed),
                ],
            );
        }
        if (self.web("hasFrames", &[]).to_boolean() || active)
            && self.last_frame.is_none_or(|last| now >= last + FRAME)
        {
            self.last_frame = Some(now);
            let time = (now - self.started).as_secs_f64() * 1000.0;
            self.web("frame", &[JsValue::from(time)]);
        }
    }

    /// How long until the renderer has work to do, if it has any.
    pub fn next_wake(&mut self) -> Option<Duration> {
        let css_work = {
            let mut frame = self.frame.borrow_mut();
            frame.rendered();
            frame.motion.has_work()
        };
        let timer = self.host.next_timer();
        let frame = (self.web("hasFrames", &[]).to_boolean() || css_work).then(|| {
            self.last_frame
                .map_or(Duration::ZERO, |last| FRAME.saturating_sub(last.elapsed()))
        });
        timer.into_iter().chain(frame).min()
    }

    /// Changes the scale of the window, as when it moves to a display with
    /// another resolution: text is measured again and media queries see the
    /// new resolution.
    pub fn set_scale(&mut self, scale: f32) {
        let mut frame = self.frame.borrow_mut();
        if frame.environment.media.resolution == f64::from(scale) {
            return;
        }
        frame.environment.media.resolution = f64::from(scale);
        frame.rendered = None;
    }

    /// Changes the viewport size.
    pub fn resize(&mut self, width: f32, height: f32) {
        {
            let mut frame = self.frame.borrow_mut();
            frame.environment.media.width = f64::from(width);
            frame.environment.media.height = f64::from(height);
        }
        self.web("resize", &[]);
    }

    fn hit(&mut self, x: f32, y: f32) -> Option<NodeId> {
        self.frame.borrow_mut().rendered().scene.hit_test(x, y)
    }

    fn update_cursor(&mut self, target: Option<NodeId>, (x, y): (f32, f32)) {
        let dom = self.dom();
        let cursor = target
            .and_then(|target| {
                let mut frame = self.frame.borrow_mut();
                let rendered = frame.rendered();
                resize::resizer_at(&rendered.scene, &dom.borrow(), x, y)
                    .map(|node| resize::cursor(&rendered.styles, node))
                    .or_else(|| rendered.styles.get(target).map(|style| style.cursor))
            })
            .unwrap_or(Cursor::Auto);
        if cursor != self.cursor {
            self.cursor = cursor;
            self.cursor_changed = true;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pointer(
        &mut self,
        kind: PointerKind,
        button: i16,
        buttons: u16,
        count: u32,
        x: f32,
        y: f32,
        modifiers: &Modifiers,
    ) -> bool {
        let target = if kind == PointerKind::Leave {
            None
        } else {
            self.hit(x, y)
        };
        let dom = self.dom();
        dom.borrow_mut().set_hovered(target);
        let fields = merge(
            json!({
                "clientX": x,
                "clientY": y,
                "screenX": x,
                "screenY": y,
                "button": button,
                "buttons": buttons,
                "detail": count,
            }),
            modifier_fields(modifiers),
        );
        let init = self.host.from_json(&fields);
        let call = |renderer: &mut Renderer, kind: &str, init: &JsValue| {
            renderer
                .web(
                    "pointer",
                    &[
                        JsValue::from(JsString::from(kind)),
                        node_value(target),
                        init.clone(),
                    ],
                )
                .to_boolean()
        };
        // A control in a rotated or skewed box is kept out of sight: the
        // pointer where its image shows reaches it.
        let proxied = target.filter(|target| {
            self.frame
                .borrow_mut()
                .rendered()
                .scene
                .takes_pointer(*target)
        });
        let window = self.frame.borrow().window;
        if let Some(control) = proxied
            && nativeboat_win32::forward_to_control(
                window,
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
                if let Some(pressed) = self.scrollbar {
                    if let Some(dragging) = pressed {
                        let mut frame = self.frame.borrow_mut();
                        let rendered = frame.rendered();
                        dragging.drag(&rendered.scene, &mut dom.borrow_mut(), x, y);
                    }
                    return true;
                }
                match self.resizing {
                    Some(resizing) => {
                        let mut frame = self.frame.borrow_mut();
                        let rendered = frame.rendered();
                        resizing.drag(
                            &rendered.scene,
                            &rendered.styles,
                            &rendered.layout,
                            &mut dom.borrow_mut(),
                            x,
                            y,
                        );
                    }
                    None => {
                        let mut frame = self.frame.borrow_mut();
                        let rendered = frame.rendered();
                        scrollbars::hover(&rendered.scene, &mut dom.borrow_mut(), x, y);
                        drop(frame);
                        self.update_cursor(target, (x, y));
                    }
                }
                call(self, "move", &init)
            }
            PointerKind::Leave => {
                if self.resizing.is_none() {
                    self.update_cursor(None, (x, y));
                }
                let (_, pressed) = dom.borrow().scrollbar_parts();
                dom.borrow_mut().set_scrollbar_parts((None, pressed));
                call(self, "leave", &init)
            }
            PointerKind::Down => {
                // Pressing a resize grip starts resizing its box instead of
                // reaching the page, as in Chromium.
                if button == 0 {
                    let mut frame = self.frame.borrow_mut();
                    let rendered = frame.rendered();
                    let resizing = Resizing::start(&rendered.scene, &dom.borrow(), x, y);
                    // So does pressing the scrollbar of a scroll container
                    // the page paints in place: it scrolls, or its thumb
                    // drags.
                    let scrollbar = resizing
                        .is_none()
                        .then(|| scrollbars::press(&rendered.scene, &mut dom.borrow_mut(), x, y))
                        .flatten();
                    drop(frame);
                    if resizing.is_some() || scrollbar.is_some() {
                        self.resizing = resizing;
                        self.scrollbar = scrollbar;
                        self.pressed = None;
                        return true;
                    }
                }
                dom.borrow_mut().set_active(target);
                self.pressed = Some((target, button, count));
                call(self, "down", &init)
            }
            PointerKind::Up => {
                if self.scrollbar.take().is_some() {
                    scrollbars::release(&mut dom.borrow_mut());
                    return true;
                }
                if self.resizing.take().is_some() {
                    self.update_cursor(target, (x, y));
                }
                dom.borrow_mut().set_active(None);
                let cancelled = call(self, "up", &init);
                if let Some((pressed, pressed_button, clicks)) = self.pressed.take()
                    && pressed == target
                    && pressed_button == button
                {
                    let fields = merge(fields, json!({ "detail": clicks }));
                    let init = self.host.from_json(&fields);
                    call(self, "click", &init);
                    if clicks == 2 && button == 0 {
                        call(self, "dblclick", &init);
                    }
                }
                cancelled
            }
        }
    }

    /// Runs an Electron editing command on the focused DOM element.
    pub fn edit_command(&mut self, command: &str) -> bool {
        self.web("command", &[JsString::from(command).into()])
            .to_boolean()
    }

    fn control(&mut self, id: NodeId, event: &ControlEvent) {
        let node = JsValue::from(id.0);
        match event {
            ControlEvent::Click => {
                self.web("activate", &[node]);
            }
            ControlEvent::Input(text) => {
                self.web(
                    "input",
                    &[node, JsValue::from(JsString::from(text.as_str()))],
                );
            }
            ControlEvent::Select(indexes) => {
                let indexes = JsArray::from_iter(
                    indexes.iter().map(|index| JsValue::from(*index as u32)),
                    self.host.context(),
                );
                self.web("select", &[node, indexes.into()]);
            }
            ControlEvent::Value(value) => {
                self.web("value", &[node, JsValue::from(*value)]);
            }
            ControlEvent::Focus => {
                self.web("focus", &[node]);
            }
            ControlEvent::Blur => {
                if self.dom().borrow().focused() == Some(id) {
                    self.web("focus", &[JsValue::null()]);
                }
            }
        }
    }

    /// Delivers platform input; returns whether its default action was
    /// cancelled.
    pub fn handle(&mut self, event: &Event) -> bool {
        match event {
            Event::Pointer {
                kind,
                button,
                buttons,
                count,
                x,
                y,
                modifiers,
            } => self.pointer(*kind, *button, *buttons, *count, *x, *y, modifiers),
            Event::FileDrag {
                kind,
                paths,
                strings,
                x,
                y,
                modifiers,
            } => {
                let kind = match kind {
                    nativeboat_win32::FileDragKind::Enter => "enter",
                    nativeboat_win32::FileDragKind::Over => "over",
                    nativeboat_win32::FileDragKind::Leave => "leave",
                };
                let target = self.hit(*x, *y);
                let metadata: Vec<_> = paths.iter().map(|path| json!({ "path": path })).collect();
                let files = self.host.from_json(&json!(metadata));
                let init = self.host.from_json(&merge(
                    json!({ "clientX": x, "clientY": y }),
                    modifier_fields(modifiers),
                ));
                let strings = self.host.from_json(&json!(strings));
                self.web(
                    "dragFiles",
                    &[
                        JsString::from(kind).into(),
                        node_value(target),
                        files,
                        init,
                        strings,
                    ],
                )
                .to_boolean()
            }
            Event::FilesDropped {
                paths,
                strings,
                x,
                y,
            } => {
                let target = self.hit(*x, *y);
                let files: Vec<_> = paths.iter().map(|path| {
                    let bytes = std::fs::read(path).unwrap_or_default();
                    json!({ "path": path, "bytes": base64::engine::general_purpose::STANDARD.encode(bytes) })
                }).collect();
                let files = self.host.from_json(&json!(files));
                let init = self.host.from_json(&json!({ "clientX": x, "clientY": y }));
                let strings = self.host.from_json(&json!(strings));
                self.web("dropFiles", &[node_value(target), files, init, strings])
                    .to_boolean()
            }
            Event::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                modifiers,
            } => {
                let target = self.hit(*x, *y);
                let fields = merge(
                    json!({ "clientX": x, "clientY": y, "deltaX": delta_x, "deltaY": delta_y, "deltaMode": 0 }),
                    modifier_fields(modifiers),
                );
                let init = self.host.from_json(&fields);
                if self.web("wheel", &[node_value(target), init]).to_boolean() {
                    return true;
                }
                // Scroll containers painted in place scroll here; native
                // ones scroll themselves.
                let Some(target) = target else {
                    return false;
                };
                let dom = self.dom();
                let mut frame = self.frame.borrow_mut();
                let rendered = frame.rendered();
                scrollbars::wheel(
                    &rendered.scene,
                    &mut dom.borrow_mut(),
                    target,
                    (*delta_x, *delta_y),
                )
            }
            Event::Key {
                down,
                key,
                code,
                repeat,
                modifiers,
                control,
            } => {
                let fields = merge(
                    json!({ "key": key, "code": code, "repeat": repeat }),
                    modifier_fields(modifiers),
                );
                let init = self.host.from_json(&fields);
                let native = JsValue::from(control.is_some());
                self.web("key", &[JsValue::from(*down), init, native])
                    .to_boolean()
            }
            Event::Text(text) => self
                .web("text", &[JsString::from(text.as_str()).into()])
                .to_boolean(),
            Event::Composition { kind, data, cursor } => {
                let kind = match kind {
                    CompositionKind::Start => "start",
                    CompositionKind::Update => "update",
                    CompositionKind::End => "end",
                };
                self.web(
                    "composition",
                    &[
                        JsString::from(kind).into(),
                        JsString::from(data.as_str()).into(),
                        JsValue::from(*cursor as f64),
                    ],
                )
                .to_boolean()
            }
            Event::Scroll { surface, x, y } => {
                let dom = self.dom();
                let node = if *surface == 0 {
                    dom.borrow().document_element()
                } else {
                    Some(NodeId(*surface as u32))
                };
                if let Some(node) = node {
                    dom.borrow_mut().set_scroll(node, *x, *y);
                }
                let target = if *surface == 0 { None } else { node };
                self.web("scroll", &[node_value(target)]);
                false
            }
            Event::Control { id, event } => {
                self.control(NodeId(*id as u32), event);
                false
            }
            Event::Resized { width, height } => {
                self.resize(*width, *height);
                false
            }
            // Window, menu and system events are the main process's.
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
