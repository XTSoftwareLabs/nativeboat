//! Web APIs for renderer pages: the DOM bindings, events, the window object,
//! URLs, history, storage, observers and encoding.
//!
//! The APIs are written in JavaScript (`src/js`) over native functions that
//! operate on the Rust [`Dom`] by node id and ask the [`Page`] for anything
//! that needs layout or the platform. [`install`] adds them to a
//! [`nativeboat_js::Host`]; the renderer then drives input through the
//! `host.web` object.

mod natives;

use boa_engine::object::FunctionObjectBuilder;
use boa_engine::{Context, Finalize, JsObject, JsResult, JsString, JsValue, NativeFunction, Trace};
use nativeboat_dom::{Dom, NodeId};
use nativeboat_ir::Rect;
use nativeboat_js::Host;
use natives::{DOM, DomOp, PAGE, PageOp};
use std::cell::RefCell;
use std::rc::Rc;

/// Where `scrollIntoView` places an element in a scroll container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAlign {
    /// At the start edge.
    Start,
    /// In the middle.
    Center,
    /// At the end edge.
    End,
    /// Wherever it needs the least scrolling; not at all when it is visible.
    Nearest,
}

impl ScrollAlign {
    /// Parses a `ScrollLogicalPosition`; anything else is `nearest`.
    pub fn parse(text: &str) -> ScrollAlign {
        match text {
            "start" => ScrollAlign::Start,
            "center" => ScrollAlign::Center,
            "end" => ScrollAlign::End,
            _ => ScrollAlign::Nearest,
        }
    }
}

/// Media as a media element loads it: its duration and, for video, the
/// width, height and RGBA pixels of its first frame.
pub type LoadedMedia = (f64, Option<(u32, u32, Vec<u8>)>);

/// What the web APIs need from the renderer and the platform.
pub trait Page {
    /// The page URL.
    fn url(&self) -> String;
    /// The border box of an element in viewport coordinates, if rendered.
    fn rect(&self, node: NodeId) -> Option<Rect>;
    /// The border widths (left, top) and padding box size of an element.
    fn client(&self, node: NodeId) -> Option<Rect>;
    /// The bounding box of an SVG element's geometry in its user space, as
    /// `getBBox()` gives it.
    fn bbox(&self, _node: NodeId) -> Option<Rect> {
        None
    }
    /// The scrollable size of an element's content.
    fn scroll_size(&self, node: NodeId) -> Option<(f32, f32)>;
    /// The computed style of an element as property names and values.
    fn computed_style(&self, node: NodeId) -> Vec<(String, String)>;
    /// One computed property, without materializing the full declaration.
    fn computed_style_value(&self, node: NodeId, name: &str) -> String {
        self.computed_style(node)
            .into_iter()
            .find(|(property, _)| property == name)
            .map_or(String::new(), |(_, value)| value)
    }
    /// The viewport size.
    fn viewport(&self) -> (f32, f32);
    /// Device pixels per CSS pixel.
    fn device_pixel_ratio(&self) -> f32;
    /// The topmost element at a viewport point.
    fn element_from_point(&self, x: f32, y: f32) -> Option<NodeId>;
    /// The caret position nearest a viewport point, as a text node or
    /// element and an offset, as `caretPositionFromPoint` finds it.
    fn caret_from_point(&self, x: f32, y: f32) -> Option<(NodeId, u32)>;
    /// The viewport rectangles that the text from offset `start` to `end` of
    /// a text node covers, one per line; a collapsed range gives the caret.
    fn text_rects(&self, node: NodeId, start: u32, end: u32) -> Vec<Rect>;
    /// Whether a media query matches.
    fn match_media(&self, query: &str) -> bool;
    /// The focused element changed.
    fn focus(&self, node: Option<NodeId>);
    /// Completes the native IME before a DOM focus or caret change, returning
    /// its final candidate; `None` when no native composition is active.
    fn complete_composition(&self) -> Option<String>;
    /// Scrolls an element into view, aligned in each direction as
    /// `scrollIntoView` aligns it.
    fn scroll_into_view(&self, node: NodeId, block: ScrollAlign, inline: ScrollAlign);
    /// The natural size of an image.
    fn image_size(&self, url: &str) -> Option<(f32, f32)>;
    /// The media at a URL as a media element loads it; `None` when it
    /// cannot be read.
    fn load_media(&self, _url: &str) -> Option<LoadedMedia> {
        None
    }
    /// Takes a video's first frame as the image the page shows under `key`.
    fn add_frame(&self, _key: &str, _width: u32, _height: u32, _pixels: &[u8]) -> bool {
        false
    }
    /// The saved `localStorage` contents.
    fn load_storage(&self) -> Option<String>;
    /// Saves the `localStorage` contents.
    fn save_storage(&self, data: &str);
    /// The rules inserted into the style sheet of a `<style>` element
    /// through the CSSOM, which add to its text.
    fn sheet_rules(&self, owner: NodeId, rules: &str);
    /// The selection the platform shows in a text control, as offsets into
    /// its value; `None` when the element is not shown as a text control.
    fn text_selection(&self, node: NodeId) -> Option<(u32, u32)>;
    /// Selects text in the text control showing an element.
    fn select_text(&self, node: NodeId, start: u32, end: u32);
    /// The text on the clipboard; `None` when the clipboard cannot be read.
    fn read_clipboard(&self) -> Option<String>;
    /// Puts text on the clipboard; returns whether that worked.
    fn write_clipboard(&self, text: &str) -> bool;
    /// A snapshot of available page clipboard formats, or `None` when busy.
    fn read_clipboard_contents(&self) -> Option<Vec<(String, String)>>;
    /// Writes the text and HTML supplied by a copy or cut event together.
    fn write_clipboard_contents(&self, text: Option<&str>, html: Option<&str>) -> bool;
    /// Shows the options of a `<select>` the page draws for the user to
    /// choose one.
    fn open_select(&self, node: NodeId);
    /// Runs 2D drawing commands, serialized as JSON, on the bitmap of a
    /// canvas element, which is recreated transparent when its size differs.
    fn canvas_draw(&self, node: NodeId, width: u32, height: u32, commands: &str);
    /// The RGBA pixels of a rectangle `(x, y, width, height)` of a canvas.
    fn canvas_read(&self, node: NodeId, area: (i32, i32, u32, u32)) -> Vec<u8>;
    /// Replaces the RGBA pixels of a rectangle of a canvas.
    fn canvas_write(&self, node: NodeId, area: (i32, i32, u32, u32), pixels: &[u8]);
    /// The canvas bitmap as a PNG or JPEG data URL.
    fn canvas_url(&self, node: NodeId, jpeg: bool) -> String;
    /// A CSS color as RGBA components, with alpha between 0 and 1.
    fn parse_color(&self, text: &str) -> Option<[f32; 4]>;
    /// A CSS `font` value as a font, serialized as JSON.
    fn parse_font(&self, text: &str) -> Option<String>;
    /// The width, ascent and descent of text in a font serialized as JSON.
    fn measure_text(&self, font: &str, text: &str) -> [f32; 3];
    /// SVG path data as path segments, serialized as JSON.
    fn path_data(&self, data: &str) -> String;
}

const SCRIPT: &str = concat!(
    "(function (host, native, page) {\n'use strict';\n",
    include_str!("js/core.js"),
    include_str!("js/events.js"),
    include_str!("js/nodes.js"),
    include_str!("js/custom-elements.js"),
    include_str!("js/ranges.js"),
    include_str!("js/elements.js"),
    include_str!("js/shadow.js"),
    include_str!("js/canvas.js"),
    include_str!("js/xhr.js"),
    include_str!("js/window.js"),
    include_str!("js/editing.js"),
    include_str!("js/composition.js"),
    include_str!("js/editing-history.js"),
    "\n})"
);

#[derive(Clone, Trace, Finalize)]
struct DomNative {
    #[unsafe_ignore_trace]
    dom: Rc<RefCell<Dom>>,
    #[unsafe_ignore_trace]
    op: DomOp,
}

#[derive(Clone, Trace, Finalize)]
struct PageNative {
    #[unsafe_ignore_trace]
    page: Rc<dyn Page>,
    #[unsafe_ignore_trace]
    op: PageOp,
}

fn call_dom(
    _: &JsValue,
    args: &[JsValue],
    native: &DomNative,
    context: &mut Context,
) -> JsResult<JsValue> {
    (native.op)(&native.dom, args, context)
}

fn call_page(
    _: &JsValue,
    args: &[JsValue],
    native: &PageNative,
    context: &mut Context,
) -> JsResult<JsValue> {
    (native.op)(native.page.as_ref(), args, context)
}

fn object(functions: Vec<(&str, NativeFunction)>, context: &mut Context) -> JsObject {
    let object = JsObject::with_null_proto();
    for (name, function) in functions {
        let function = FunctionObjectBuilder::new(context.realm(), function)
            .name(JsString::from(name))
            .build();
        object
            .set(JsString::from(name), function, false, context)
            .expect("native objects are extensible");
    }
    object
}

/// Installs the web APIs over a document in a host.
pub fn install(host: &mut Host, dom: Rc<RefCell<Dom>>, page: Rc<dyn Page>) {
    let host_object = host.host_object();
    let context = host.context();
    let dom_natives = DOM
        .iter()
        .map(|(name, op)| {
            let native = DomNative {
                dom: dom.clone(),
                op: *op,
            };
            (
                *name,
                NativeFunction::from_copy_closure_with_captures(call_dom, native),
            )
        })
        .collect();
    let page_natives = PAGE
        .iter()
        .map(|(name, op)| {
            let native = PageNative {
                page: page.clone(),
                op: *op,
            };
            (
                *name,
                NativeFunction::from_copy_closure_with_captures(call_page, native),
            )
        })
        .collect();
    let native = object(dom_natives, context);
    let page = object(page_natives, context);
    let script = host
        .eval(SCRIPT, "nativeboat:web")
        .expect("the web prelude compiles");
    script
        .as_callable()
        .expect("the web prelude is a function")
        .call(
            &JsValue::undefined(),
            &[host_object.into(), native.into(), page.into()],
            host.context(),
        )
        .expect("the web prelude runs");
}

#[cfg(test)]
mod tests;
