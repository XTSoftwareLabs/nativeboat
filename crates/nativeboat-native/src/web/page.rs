//! What the Web APIs ask of the renderer and the platform: geometry, styles,
//! focus, clipboard, storage and canvas bitmaps. The prelude reaches a
//! [`Page`] through the `page` natives.

use super::MediaSource;
use super::{PageOp, arg, flag, number, numbers, optional_string, pairs, string, text, unit};
#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Realm, Value};
use nativeboat_dom::NodeId;

/// A rectangle in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    /// The left edge.
    pub x: f32,
    /// The top edge.
    pub y: f32,
    /// The width.
    pub width: f32,
    /// The height.
    pub height: f32,
}

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

/// What the Web APIs need from the renderer and the platform.
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
    /// The caret position nearest a viewport point, as a text node or element
    /// and an offset, as `caretPositionFromPoint` finds it.
    fn caret_from_point(&self, x: f32, y: f32) -> Option<(NodeId, u32)>;
    /// The viewport rectangles that the text from offset `start` to `end` of a
    /// text node covers, one per line; a collapsed range gives the caret.
    fn text_rects(&self, node: NodeId, start: u32, end: u32) -> Vec<Rect>;
    /// Whether a media query matches.
    fn match_media(&self, query: &str) -> bool;
    /// The misspelled word of a text node around an offset, as painted
    /// spelling marks show it: its start and end.
    fn misspelling(&self, _node: NodeId, _offset: u32) -> Option<(u32, u32)> {
        None
    }
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
    /// Takes the bytes of an image downloaded for a URL, so that the page
    /// lays it out and paints it; returns whether they are an image.
    fn add_image(&self, _url: &str, _bytes: &[u8]) -> bool {
        false
    }
    /// Where the media at a URL comes from, for a `<video>` or `<audio>`
    /// element to read; `None` when it cannot be read.
    fn media_source(&self, _url: &str) -> Option<MediaSource> {
        None
    }
    /// Takes a video's first frame as straight RGBA pixels, as the image the
    /// page lays out and paints under `key`; returns whether it was taken.
    fn add_frame(&self, _key: &str, _width: u32, _height: u32, _pixels: &[u8]) -> bool {
        false
    }
    /// The width, height and RGBA pixels of an image the page has or loads
    /// from a file, as a notification's icon shows them.
    fn image_pixels(&self, _url: &str) -> Option<(u32, u32, Vec<u8>)> {
        None
    }
    /// The network images layout and painting asked for since the last
    /// call, such as style sheets' backgrounds, which the page downloads.
    fn take_wanted_images(&self) -> Vec<String> {
        Vec::new()
    }
    /// The saved `localStorage` contents.
    fn load_storage(&self) -> Option<String>;
    /// Saves the `localStorage` contents.
    fn save_storage(&self, data: &str);
    /// The rules inserted into the style sheet of a `<style>` element through
    /// the CSSOM, which add to its text.
    fn sheet_rules(&self, owner: NodeId, rules: &str);
    /// The selection the platform shows in a text control, as offsets into its
    /// value; `None` when the element is not shown as a text control.
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
    /// Shows the options of a `<select>` the page draws for the user to choose
    /// one.
    fn open_select(&self, node: NodeId);
    /// Runs 2D drawing commands, serialized as JSON, on the bitmap of a canvas
    /// element, which is recreated transparent when its size differs.
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
    /// Continues a drag in other applications once it leaves the window:
    /// `items` are its `[type, data]` pairs, `allowed` its `effectAllowed`;
    /// returns the drop's effect, `none` when nothing took it.
    fn drag_out(&self, _items: &[(String, String)], _allowed: &str) -> String {
        "none".to_owned()
    }
}

/// A page that is not laid out yet: nothing is rendered, the viewport has a
/// fixed size, and no platform services are attached. Renderer windows use
/// it until native layout answers geometry queries.
#[derive(Debug, Clone)]
pub struct BlankPage {
    /// The page URL.
    pub url: String,
    /// The viewport size in CSS pixels.
    pub viewport: (f32, f32),
}

impl Page for BlankPage {
    fn url(&self) -> String {
        self.url.clone()
    }
    fn rect(&self, _: NodeId) -> Option<Rect> {
        None
    }
    fn client(&self, _: NodeId) -> Option<Rect> {
        None
    }
    fn scroll_size(&self, _: NodeId) -> Option<(f32, f32)> {
        None
    }
    fn computed_style(&self, _: NodeId) -> Vec<(String, String)> {
        Vec::new()
    }
    fn viewport(&self) -> (f32, f32) {
        self.viewport
    }
    fn device_pixel_ratio(&self) -> f32 {
        1.0
    }
    fn element_from_point(&self, _: f32, _: f32) -> Option<NodeId> {
        None
    }
    fn caret_from_point(&self, _: f32, _: f32) -> Option<(NodeId, u32)> {
        None
    }
    fn text_rects(&self, _: NodeId, _: u32, _: u32) -> Vec<Rect> {
        Vec::new()
    }
    fn match_media(&self, _: &str) -> bool {
        false
    }
    fn focus(&self, _: Option<NodeId>) {}
    fn complete_composition(&self) -> Option<String> {
        None
    }
    fn scroll_into_view(&self, _: NodeId, _: ScrollAlign, _: ScrollAlign) {}
    fn image_size(&self, _: &str) -> Option<(f32, f32)> {
        None
    }
    fn load_storage(&self) -> Option<String> {
        None
    }
    fn save_storage(&self, _: &str) {}
    fn sheet_rules(&self, _: NodeId, _: &str) {}
    fn text_selection(&self, _: NodeId) -> Option<(u32, u32)> {
        None
    }
    fn select_text(&self, _: NodeId, _: u32, _: u32) {}
    fn read_clipboard(&self) -> Option<String> {
        None
    }
    fn write_clipboard(&self, _: &str) -> bool {
        false
    }
    fn read_clipboard_contents(&self) -> Option<Vec<(String, String)>> {
        None
    }
    fn write_clipboard_contents(&self, _: Option<&str>, _: Option<&str>) -> bool {
        false
    }
    fn open_select(&self, _: NodeId) {}
    fn canvas_draw(&self, _: NodeId, _: u32, _: u32, _: &str) {}
    fn canvas_read(&self, _: NodeId, area: (i32, i32, u32, u32)) -> Vec<u8> {
        vec![0; area.2 as usize * area.3 as usize * 4]
    }
    fn canvas_write(&self, _: NodeId, _: (i32, i32, u32, u32), _: &[u8]) {}
    fn canvas_url(&self, _: NodeId, _: bool) -> String {
        "data:,".to_owned()
    }
    fn parse_color(&self, _: &str) -> Option<[f32; 4]> {
        None
    }
    fn parse_font(&self, _: &str) -> Option<String> {
        None
    }
    fn measure_text(&self, _: &str, _: &str) -> [f32; 3] {
        [0.0; 3]
    }
    fn path_data(&self, _: &str) -> String {
        "[]".to_owned()
    }
}

fn node(realm: &mut Realm, args: &[Value], index: usize) -> Result<NodeId, crate::Thrown> {
    Ok(NodeId(number(realm, args, index)? as u32))
}

fn rect(realm: &Realm, value: Option<Rect>) -> Value {
    value.map_or(Value::Null, |rect| {
        numbers(realm, &[rect.x, rect.y, rect.width, rect.height])
    })
}

fn size(realm: &Realm, value: Option<(f32, f32)>) -> Value {
    value.map_or(Value::Null, |(width, height)| {
        numbers(realm, &[width, height])
    })
}

/// The `x, y, width, height` arguments of canvas pixel natives.
fn canvas_area(realm: &mut Realm, args: &[Value]) -> Result<(i32, i32, u32, u32), crate::Thrown> {
    Ok((
        number(realm, args, 1)? as i32,
        number(realm, args, 2)? as i32,
        number(realm, args, 3)? as u32,
        number(realm, args, 4)? as u32,
    ))
}

/// An array argument of `[name, value]` string pairs.
fn string_pairs(
    realm: &mut Realm,
    args: &[Value],
    index: usize,
) -> Result<Vec<(String, String)>, crate::Thrown> {
    let list = arg(args, index);
    if !matches!(list, Value::Object(_)) {
        return Ok(Vec::new());
    }
    let length = crate::ops::get(realm, list, &crate::Key::name("length"))?;
    let length = crate::ops::to_number(realm, &length)? as usize;
    let mut found = Vec::with_capacity(length.min(64));
    for index in 0..length.min(64) {
        let pair = crate::ops::get(realm, list, &crate::Key::Index(index as u32))?;
        let part = |realm: &mut Realm, at: u32| -> Result<String, crate::Thrown> {
            let value = crate::ops::get(realm, &pair, &crate::Key::Index(at))?;
            Ok(crate::ops::to_string(realm, &value)?.to_rust_lossy())
        };
        let name = part(realm, 0)?;
        let value = part(realm, 1)?;
        found.push((name, value));
    }
    Ok(found)
}

/// Text, or `None` for `null` and `undefined`.
fn optional_text(
    realm: &mut Realm,
    args: &[Value],
    index: usize,
) -> Result<Option<String>, crate::Thrown> {
    if matches!(arg(args, index), Value::Undefined | Value::Null) {
        return Ok(None);
    }
    text(realm, args, index).map(Some)
}

/// The page natives by name.
pub(super) const OPERATIONS: &[(&str, PageOp)] = &[
    ("url", |page, _, _| Ok(string(&page.url()))),
    // A package script the page inserts, by URL: a promise of its run.
    ("loadScript", |_, realm, args| {
        realm.load_script(arg(args, 0))
    }),
    ("rect", |page, realm, args| {
        let node = node(realm, args, 0)?;
        Ok(rect(realm, page.rect(node)))
    }),
    ("client", |page, realm, args| {
        let node = node(realm, args, 0)?;
        Ok(rect(realm, page.client(node)))
    }),
    ("bbox", |page, realm, args| {
        let node = node(realm, args, 0)?;
        Ok(rect(realm, page.bbox(node)))
    }),
    ("scrollSize", |page, realm, args| {
        let node = node(realm, args, 0)?;
        Ok(size(realm, page.scroll_size(node)))
    }),
    ("computedStyleValue", |page, realm, args| {
        let node = node(realm, args, 0)?;
        let name = text(realm, args, 1)?;
        Ok(string(&page.computed_style_value(node, &name)))
    }),
    ("computedStyle", |page, realm, args| {
        let node = node(realm, args, 0)?;
        Ok(pairs(realm, page.computed_style(node)))
    }),
    ("viewport", |page, realm, _| {
        let (width, height) = page.viewport();
        Ok(numbers(realm, &[width, height, page.device_pixel_ratio()]))
    }),
    ("elementFromPoint", |page, realm, args| {
        let x = number(realm, args, 0)? as f32;
        let y = number(realm, args, 1)? as f32;
        Ok(super::id(page.element_from_point(x, y)))
    }),
    ("caretFromPoint", |page, realm, args| {
        let x = number(realm, args, 0)? as f32;
        let y = number(realm, args, 1)? as f32;
        Ok(page
            .caret_from_point(x, y)
            .map_or(Value::Null, |(node, offset)| {
                let values = vec![
                    Value::Number(f64::from(node.0)),
                    Value::Number(f64::from(offset)),
                ];
                Value::Object(realm.array(values))
            }))
    }),
    ("textRects", |page, realm, args| {
        let node = node(realm, args, 0)?;
        let start = number(realm, args, 1)? as u32;
        let end = number(realm, args, 2)? as u32;
        let rects = page
            .text_rects(node, start, end)
            .into_iter()
            .map(|value| rect(realm, Some(value)))
            .collect();
        Ok(Value::Object(realm.array(rects)))
    }),
    ("misspelling", |page, realm, args| {
        let node = node(realm, args, 0)?;
        let offset = number(realm, args, 1)? as u32;
        Ok(match page.misspelling(node, offset) {
            Some((start, end)) => Value::Object(realm.array(vec![
                Value::Number(f64::from(start)),
                Value::Number(f64::from(end)),
            ])),
            None => Value::Null,
        })
    }),
    ("spellingSuggestions", |_, realm, args| {
        let word = text(realm, args, 0)?;
        let suggestions = crate::spelling::suggestions(&word)
            .iter()
            .map(|suggestion| Value::str(suggestion))
            .collect();
        Ok(Value::Object(realm.array(suggestions)))
    }),
    ("matchMedia", |page, realm, args| {
        let query = text(realm, args, 0)?;
        Ok(Value::Boolean(page.match_media(&query)))
    }),
    ("completeComposition", |page, _, _| {
        Ok(optional_string(page.complete_composition().as_deref()))
    }),
    ("focus", |page, _, args| {
        let target = match arg(args, 0) {
            Value::Number(number) => Some(NodeId(*number as u32)),
            _ => None,
        };
        page.focus(target);
        unit()
    }),
    ("scrollIntoView", |page, realm, args| {
        let node = node(realm, args, 0)?;
        let block = ScrollAlign::parse(&text(realm, args, 1)?);
        let inline = ScrollAlign::parse(&text(realm, args, 2)?);
        page.scroll_into_view(node, block, inline);
        unit()
    }),
    ("imageSize", |page, realm, args| {
        let url = text(realm, args, 0)?;
        Ok(size(realm, page.image_size(&url)))
    }),
    // Downloads an http(s) URL off the page's thread, then calls back with
    // its bytes, or `null` when the download failed.
    ("fetchImage", |_, realm, args| {
        let url = text(realm, args, 0)?;
        let callback = arg(args, 1).clone();
        let agent = crate::web::user_agent(realm);
        realm
            .submit_io(
                move || crate::http_binding::download(&url, agent),
                move |realm, bytes| {
                    let value = match bytes {
                        Some(bytes) => {
                            Value::Object(crate::typed_array_builtin::new_bytes(realm, bytes, None))
                        }
                        None => Value::Null,
                    };
                    crate::ops::call(realm, &callback, Value::Undefined, &[value])?;
                    Ok(())
                },
            )
            .map_err(|_| {
                realm.throw(crate::ErrorKind::Error, "could not queue an image download")
            })?;
        unit()
    }),
    // Reads a media element's media off the page's thread, then calls back
    // with its duration followed, when it has video, by the width, height
    // and RGBA pixels of its first frame; or with `null` when it cannot be
    // read.
    ("loadMedia", |page, realm, args| {
        let url = text(realm, args, 0)?;
        let callback = arg(args, 1).clone();
        let source = page.media_source(&url);
        realm
            .submit_io(
                move || source.as_ref().and_then(super::media::read),
                move |realm, media| {
                    let value = match media {
                        Some(media) => {
                            let mut items = vec![Value::Number(media.duration)];
                            if let Some(frame) = media.frame {
                                items.push(Value::Number(f64::from(frame.width)));
                                items.push(Value::Number(f64::from(frame.height)));
                                items.push(Value::Object(crate::typed_array_builtin::new_bytes(
                                    realm,
                                    frame.pixels,
                                    None,
                                )));
                            }
                            Value::Object(realm.array(items))
                        }
                        None => Value::Null,
                    };
                    crate::ops::call(realm, &callback, Value::Undefined, &[value])?;
                    Ok(())
                },
            )
            .map_err(|_| realm.throw(crate::ErrorKind::Error, "could not queue reading media"))?;
        unit()
    }),
    ("addFrame", |page, realm, args| {
        let key = text(realm, args, 0)?;
        let width = number(realm, args, 1)? as u32;
        let height = number(realm, args, 2)? as u32;
        let pixels = crate::typed_array_builtin::view_bytes(arg(args, 3)).unwrap_or_default();
        Ok(Value::Boolean(page.add_frame(&key, width, height, &pixels)))
    }),
    ("imagePixels", |page, realm, args| {
        let url = text(realm, args, 0)?;
        Ok(match page.image_pixels(&url) {
            Some((width, height, pixels)) => Value::Object(realm.array(vec![
                Value::Number(f64::from(width)),
                Value::Number(f64::from(height)),
                Value::Object(crate::typed_array_builtin::new_bytes(realm, pixels, None)),
            ])),
            None => Value::Null,
        })
    }),
    ("takeWantedImages", |page, realm, _| {
        let wanted = page.take_wanted_images();
        Ok(if wanted.is_empty() {
            Value::Null
        } else {
            Value::Object(realm.array(wanted.iter().map(|url| Value::str(url)).collect()))
        })
    }),
    ("addImage", |page, realm, args| {
        let url = text(realm, args, 0)?;
        let bytes = crate::typed_array_builtin::view_bytes(arg(args, 1)).unwrap_or_default();
        Ok(Value::Boolean(page.add_image(&url, &bytes)))
    }),
    ("loadStorage", |page, _, _| {
        Ok(optional_string(page.load_storage().as_deref()))
    }),
    ("saveStorage", |page, realm, args| {
        let data = text(realm, args, 0)?;
        page.save_storage(&data);
        unit()
    }),
    ("sheetRules", |page, realm, args| {
        let owner = node(realm, args, 0)?;
        let rules = text(realm, args, 1)?;
        page.sheet_rules(owner, &rules);
        unit()
    }),
    ("textSelection", |page, realm, args| {
        let node = node(realm, args, 0)?;
        Ok(page
            .text_selection(node)
            .map_or(Value::Null, |(start, end)| {
                numbers(realm, &[start as f32, end as f32])
            }))
    }),
    ("selectText", |page, realm, args| {
        let node = node(realm, args, 0)?;
        let start = number(realm, args, 1)? as u32;
        let end = number(realm, args, 2)? as u32;
        page.select_text(node, start, end);
        unit()
    }),
    ("readClipboard", |page, _, _| {
        Ok(optional_string(page.read_clipboard().as_deref()))
    }),
    ("readClipboardContents", |page, realm, _| {
        Ok(page
            .read_clipboard_contents()
            .map_or(Value::Null, |contents| pairs(realm, contents)))
    }),
    ("writeClipboardContents", |page, realm, args| {
        let plain = optional_text(realm, args, 0)?;
        let html = optional_text(realm, args, 1)?;
        Ok(Value::Boolean(page.write_clipboard_contents(
            plain.as_deref(),
            html.as_deref(),
        )))
    }),
    ("openSelect", |page, realm, args| {
        page.open_select(node(realm, args, 0)?);
        unit()
    }),
    ("dragOut", |page, realm, args| {
        let items = string_pairs(realm, args, 0)?;
        let allowed = text(realm, args, 1)?;
        Ok(string(&page.drag_out(&items, &allowed)))
    }),
    ("writeClipboard", |page, realm, args| {
        let data = text(realm, args, 0)?;
        Ok(Value::Boolean(page.write_clipboard(&data)))
    }),
    ("canvasDraw", |page, realm, args| {
        let node = node(realm, args, 0)?;
        let width = number(realm, args, 1)? as u32;
        let height = number(realm, args, 2)? as u32;
        let commands = text(realm, args, 3)?;
        page.canvas_draw(node, width, height, &commands);
        unit()
    }),
    ("canvasRead", |page, realm, args| {
        let node = node(realm, args, 0)?;
        let area = canvas_area(realm, args)?;
        let pixels = page.canvas_read(node, area);
        Ok(Value::Object(crate::typed_array_builtin::new_bytes(
            realm, pixels, None,
        )))
    }),
    ("canvasWrite", |page, realm, args| {
        let node = node(realm, args, 0)?;
        let area = canvas_area(realm, args)?;
        let pixels = crate::typed_array_builtin::view_bytes(arg(args, 5)).ok_or_else(|| {
            realm.throw(
                crate::ErrorKind::Type,
                "canvas pixels must be a typed array",
            )
        })?;
        page.canvas_write(node, area, &pixels);
        unit()
    }),
    ("canvasURL", |page, realm, args| {
        let node = node(realm, args, 0)?;
        Ok(string(&page.canvas_url(node, flag(args, 1))))
    }),
    ("parseColor", |page, realm, args| {
        let color = text(realm, args, 0)?;
        Ok(page
            .parse_color(&color)
            .map_or(Value::Null, |rgba| numbers(realm, &rgba)))
    }),
    ("parseFont", |page, realm, args| {
        let font = text(realm, args, 0)?;
        Ok(optional_string(page.parse_font(&font).as_deref()))
    }),
    ("measureText", |page, realm, args| {
        let font = text(realm, args, 0)?;
        let content = text(realm, args, 1)?;
        Ok(numbers(realm, &page.measure_text(&font, &content)))
    }),
    ("pathData", |page, realm, args| {
        let data = text(realm, args, 0)?;
        Ok(string(&page.path_data(&data)))
    }),
];
