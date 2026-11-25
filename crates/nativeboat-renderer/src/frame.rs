//! The rendered state of a page: styles, layout and the native UI scene,
//! recomputed when the document, its style sheets or the viewport change.
//! The frame also answers the web APIs' layout and platform queries.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use nativeboat_css::color::{SpecifiedColor, parse_color};
use nativeboat_css::computed::{ComputedStyle, Specified, SpecifiedValues};
use nativeboat_css::media::parse_media_query_list;
use nativeboat_css::shorthand::expand;
use nativeboat_css::tokenizer::tokenize;
use nativeboat_css::types::{Edges, LengthPercentageAuto, Overflow, Px};
use nativeboat_css::values::{parse_component_values, without_whitespace};
use nativeboat_css::{
    Origin, PropertyId, SheetRef, StyleEnvironment, Stylesheet, parse_stylesheet,
    user_agent_stylesheet,
};
use nativeboat_dom::{Dom, NodeId};
use nativeboat_ir::{CanvasCommand, CanvasSource, DisplayItem, Font, Rect, Surface};
use nativeboat_layout::motion::Motion;
use nativeboat_layout::paint::scene_with;
use nativeboat_layout::{
    Highlight, LayoutOptions, LayoutResult, Scene, Styles, TextMeasurer, TextQuery, layout,
};
use nativeboat_web::{Page, ScrollAlign};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

/// The platform keys of canvas bitmaps, unique across pages.
static NEXT_CANVAS: AtomicU64 = AtomicU64::new(1);

/// Looks up the natural size of an image by asset key.
pub type Images = Rc<dyn Fn(&str) -> Option<(f32, f32)>>;

/// Loads saved `localStorage` contents.
pub type LoadStorage = Rc<dyn Fn() -> Option<String>>;

/// Saves `localStorage` contents.
pub type SaveStorage = Rc<dyn Fn(&str)>;

/// The layout and scene of one document state.
pub(crate) struct Rendered {
    version: u64,
    sheets: u64,
    viewport: (f32, f32),
    time: f64,
    /// The selection and focus the scene paints.
    highlight: (Option<[(NodeId, u32); 2]>, Option<NodeId>),
    pub styles: Styles,
    pub layout: LayoutResult,
    pub scene: Rc<Scene>,
}

/// Inputs of the authored styles, independent of layout and paint.
pub(crate) struct Styled {
    version: u64,
    sheets: u64,
    environment: StyleEnvironment,
    dynamic: Vec<Rc<Stylesheet>>,
}

pub(crate) struct Frame {
    pub motion: Motion,
    pub clock: Rc<dyn Fn() -> f64>,
    pub dom: Rc<RefCell<Dom>>,
    pub url: String,
    pub base: String,
    pub author: Vec<Stylesheet>,
    pub inserted: BTreeMap<NodeId, String>,
    pub parsed: HashMap<NodeId, (String, Rc<Stylesheet>)>,
    pub sheets: u64,
    pub environment: StyleEnvironment,
    pub measurer: Rc<dyn TextMeasurer>,
    pub images: Images,
    pub load_storage: LoadStorage,
    pub save_storage: SaveStorage,
    pub rendered: Option<Rendered>,
    pub styled: Option<Styled>,
    pub focus: Option<Option<NodeId>>,
    pub window: nativeboat_win32::WindowId,
    /// Text selections scripts made since the last render.
    pub selections: BTreeMap<NodeId, (u32, u32)>,
    /// The platform bitmaps of canvas elements.
    pub canvases: HashMap<NodeId, u64>,
    /// A drawn `<select>` whose options to show.
    pub select: Option<NodeId>,
}

impl Drop for Frame {
    fn drop(&mut self) {
        for key in self.canvases.values() {
            nativeboat_win32::canvas_release(*key);
        }
    }
}

/// Resolves `.` and `..` segments of a `/`-separated path.
fn normalize(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            _ => segments.push(segment),
        }
    }
    format!("/{}", segments.join("/"))
}

impl Frame {
    pub fn viewport(&self) -> (f32, f32) {
        (
            self.environment.media.width as f32,
            self.environment.media.height as f32,
        )
    }

    /// The keys an image URL may have in the application's assets: its path
    /// from the site root, and its path relative to the application root.
    pub fn asset_keys(&self, url: &str) -> Vec<String> {
        let url = url.split(['?', '#']).next().unwrap_or(url);
        if let Some(path) = url.strip_prefix(&self.base) {
            return vec![normalize(path)];
        }
        if let Some(path) = url.strip_prefix("file://") {
            return vec![normalize(path)];
        }
        if url.contains("://") || url.starts_with("data:") {
            return vec![url.to_owned()];
        }
        if url.starts_with('/') {
            return vec![normalize(url)];
        }
        let page = self.url.strip_prefix(&self.base).unwrap_or(&self.url);
        let directory = &page[..page.rfind('/').map_or(0, |index| index + 1)];
        vec![normalize(&format!("{directory}{url}")), normalize(url)]
    }

    /// The asset key of an image URL with a known size, if any.
    pub fn image_key(&self, url: &str) -> Option<String> {
        self.asset_keys(url)
            .into_iter()
            .find(|key| (self.images)(key).is_some())
    }

    /// Parsed sheets of the `<style>` elements in document order, with rules
    /// inserted through the CSSOM.
    /// The style sheets of the cascade: the user agent's, the page's and
    /// those of `<style>` elements.
    fn sheets<'a>(&'a self, dynamic: &'a [Rc<Stylesheet>]) -> Vec<SheetRef<'a>> {
        let mut sheets = vec![SheetRef {
            sheet: user_agent_stylesheet(),
            origin: Origin::UserAgent,
        }];
        sheets.extend(
            self.author
                .iter()
                .chain(dynamic.iter().map(Rc::as_ref))
                .map(|sheet| SheetRef {
                    sheet,
                    origin: Origin::Author,
                }),
        );
        sheets
    }

    /// The computed style of an element, also inside hidden elements.
    pub fn style_of(&mut self, node: NodeId) -> Option<Rc<ComputedStyle>> {
        self.update_styles((self.clock)());
        // Transitions keep their existing presentation/event path. Ordinary
        // CSS reads need the cascade, without document layout or painting.
        if self.motion.has_work() {
            self.rendered();
        }
        let styles = if self.motion.has_work() {
            &self
                .rendered
                .as_ref()
                .expect("the frame was just rendered")
                .styles
        } else {
            self.motion.targets().expect("styles were just computed")
        };
        if let Some(style) = styles.get(node) {
            return Some(style.clone());
        }
        let sheets = self.sheets(
            &self
                .styled
                .as_ref()
                .expect("styles were just computed")
                .dynamic,
        );
        styles.style_of(&self.dom.borrow(), &sheets, node)
    }

    fn update_styles(&mut self, now: f64) {
        let version = self.dom.borrow().version();
        if self.styled.as_ref().is_some_and(|styled| {
            styled.version == version
                && styled.sheets == self.sheets
                && styled.environment == self.environment
        }) {
            return;
        }
        let dynamic = self.style_elements();
        let sheets = self.sheets(&dynamic);
        let reusable = self.styled.as_ref().is_some_and(|styled| {
            styled.sheets == self.sheets
                && styled.dynamic.len() == dynamic.len()
                && styled
                    .dynamic
                    .iter()
                    .zip(&dynamic)
                    .all(|(old, new)| Rc::ptr_eq(old, new))
        });
        let styles = Styles::compute_reusing(
            &self.dom.borrow(),
            &sheets,
            &self.environment,
            self.motion
                .targets()
                .map(|styles| styles.as_ref())
                .filter(|_| reusable),
        );
        self.motion.update(styles, now);
        self.styled = Some(Styled {
            version,
            sheets: self.sheets,
            environment: self.environment,
            dynamic,
        });
    }

    fn style_elements(&mut self) -> Vec<Rc<Stylesheet>> {
        let elements = {
            let dom = self.dom.borrow();
            let mut found: Vec<(NodeId, u32)> = dom
                .get_elements_by_tag_name(Dom::DOCUMENT, "style")
                .into_iter()
                .map(|element| (element, 0))
                .collect();
            // The style sheets of a shadow tree apply in that tree alone.
            for root in dom.shadow_roots() {
                if dom.is_connected(root) {
                    found.extend(
                        dom.get_elements_by_tag_name(root, "style")
                            .into_iter()
                            .map(|element| (element, root.0)),
                    );
                }
            }
            found
                .into_iter()
                .map(|(element, scope)| {
                    let mut text = dom.text_content(element).unwrap_or_default();
                    if let Some(rules) = self.inserted.get(&element) {
                        text.push('\n');
                        text.push_str(rules);
                    }
                    (element, scope, text)
                })
                .collect::<Vec<_>>()
        };
        elements
            .into_iter()
            .map(|(element, scope, text)| match self.parsed.get(&element) {
                Some((cached, sheet)) if *cached == text && sheet.scope == scope => sheet.clone(),
                _ => {
                    let mut sheet = parse_stylesheet(&text).0;
                    sheet.scope = scope;
                    let sheet = Rc::new(sheet);
                    self.parsed.insert(element, (text, sheet.clone()));
                    sheet
                }
            })
            .collect()
    }

    /// The layout and scene for the current document state.
    pub fn rendered(&mut self) -> &Rendered {
        let now = (self.clock)();
        self.update_styles(now);
        let version = self.dom.borrow().version();
        let viewport = self.viewport();
        let highlight = {
            let dom = self.dom.borrow();
            (dom.selection(), dom.focused())
        };
        let stale = self.rendered.as_ref().is_none_or(|rendered| {
            rendered.version != version
                || rendered.sheets != self.sheets
                || rendered.viewport != viewport
        });
        if stale
            || (self.motion.active()
                && self
                    .rendered
                    .as_ref()
                    .is_some_and(|rendered| rendered.time != now))
        {
            let styles = self.motion.sample(now);
            let dom = self.dom.borrow();
            let image_size = |url: &str| self.image_key(url).and_then(|key| (self.images)(&key));
            let options = LayoutOptions {
                viewport,
                measurer: self.measurer.as_ref(),
                image_size: &image_size,
            };
            let layout = layout(&dom, &styles, &options);
            drop(dom);
            let scene = self.paint(&styles, &layout, viewport);
            self.rendered = Some(Rendered {
                time: now,
                version,
                sheets: self.sheets,
                viewport,
                highlight,
                styles,
                layout,
                scene: Rc::new(scene),
            });
        } else if let Some(mut rendered) = self
            .rendered
            .take_if(|rendered| rendered.highlight != highlight)
        {
            // Only the selection or the caret moved: the layout stands.
            rendered.scene = Rc::new(self.paint(&rendered.styles, &rendered.layout, viewport));
            rendered.highlight = highlight;
            self.rendered = Some(rendered);
        }
        self.rendered.as_ref().expect("the frame was just rendered")
    }

    /// The scene of a layout, with the document's selection and caret.
    fn paint(&mut self, styles: &Styles, layout: &LayoutResult, viewport: (f32, f32)) -> Scene {
        let selected = (self.dom.borrow().selection()).is_some_and(|[start, end]| start != end);
        // Only selected text needs the style sheets, for `::selection`.
        let dynamic = if selected {
            self.style_elements()
        } else {
            Vec::new()
        };
        let sheets = self.sheets(&dynamic);
        let dom = self.dom.borrow();
        let highlight = Highlight::new(&dom, styles, &sheets, self.measurer.as_ref());
        let mut scene = scene_with(&dom, styles, layout, viewport, Some(&highlight));
        drop(dom);
        self.rewrite_images(&mut scene.tree.root);
        scene
    }

    /// Points image items at asset keys, which the platform loads images by.
    fn rewrite_images(&self, surface: &mut Surface) {
        for item in &mut surface.items {
            if let DisplayItem::Image { url, .. } = item
                && let Some(key) = self.image_key(url)
            {
                *url = key;
            }
        }
        for child in &mut surface.children {
            self.rewrite_images(child);
        }
    }

    /// Scrolls the element's scroll containers so that it is visible.
    pub fn scroll_into_view(&mut self, node: NodeId, block: ScrollAlign, inline: ScrollAlign) {
        self.rendered();
        let (rect, containers) = {
            let rendered = self.rendered.as_ref().expect("the frame was just rendered");
            let Some(rect) = rendered.scene.client_rect(node) else {
                return;
            };
            // `scroll-margin` keeps room around the element.
            let margin = rendered
                .styles
                .get(node)
                .map_or(Edges::all(Px(0.0)), |style| style.scroll_margin);
            let rect = Rect::new(
                rect.x - margin.left.0,
                rect.y - margin.top.0,
                rect.width + margin.left.0 + margin.right.0,
                rect.height + margin.top.0 + margin.bottom.0,
            );
            let dom = self.dom.borrow();
            let containers: Vec<(NodeId, Rect, (f32, f32))> =
                std::iter::successors(dom.parent_element(node), |element| {
                    dom.parent_element(*element)
                })
                .filter_map(|element| {
                    let style = rendered.styles.get(element)?;
                    let scrolls = |overflow: Overflow| overflow.is_scroll_container();
                    let root = Some(element) == dom.document_element();
                    let (visible, content) = if root {
                        let view = (
                            self.environment.media.width as f32,
                            self.environment.media.height as f32,
                        );
                        (
                            Rect::new(0.0, 0.0, view.0, view.1),
                            (rendered.layout.width, rendered.layout.height),
                        )
                    } else if scrolls(style.overflow_x) || scrolls(style.overflow_y) {
                        let geometry = rendered.layout.boxes.get(&element)?;
                        let border = rendered.scene.client_rect(element)?;
                        let padding = geometry.padding_box();
                        (
                            Rect::new(
                                border.x + geometry.border[3],
                                border.y + geometry.border[0],
                                padding.width,
                                padding.height,
                            ),
                            (geometry.content_width, geometry.content_height),
                        )
                    } else {
                        return None;
                    };
                    // `scroll-padding` keeps room inside the scrollport.
                    let inset = |value: &LengthPercentageAuto, size: f32| match value {
                        LengthPercentageAuto::Length(length) => length.resolve(size),
                        LengthPercentageAuto::Auto => 0.0,
                    };
                    let limit = (
                        (content.0 - visible.width).max(0.0),
                        (content.1 - visible.height).max(0.0),
                    );
                    let padding = &style.scroll_padding;
                    let (top, left) = (
                        inset(&padding.top, visible.height),
                        inset(&padding.left, visible.width),
                    );
                    let visible = Rect::new(
                        visible.x + left,
                        visible.y + top,
                        (visible.width - left - inset(&padding.right, visible.width)).max(0.0),
                        (visible.height - top - inset(&padding.bottom, visible.height)).max(0.0),
                    );
                    Some((element, visible, limit))
                })
                .collect();
            (rect, containers)
        };
        let mut rect = rect;
        let mut dom = self.dom.borrow_mut();
        for (container, visible, limit) in containers {
            let (left, top) = dom.scroll(container);
            let dx = scroll_delta(inline, rect.x, rect.width, visible.x, visible.width);
            let dy = scroll_delta(block, rect.y, rect.height, visible.y, visible.height);
            let x = (left + dx).clamp(0.0, limit.0);
            let y = (top + dy).clamp(0.0, limit.1);
            dom.set_scroll(container, x, y);
            rect = rect.offset(left - x, top - y);
        }
    }
}

/// How far a scroll container scrolls on one axis to place an extent of a
/// rectangle, `start` and `size`, in its view.
fn scroll_delta(align: ScrollAlign, start: f32, size: f32, view_start: f32, view_size: f32) -> f32 {
    match align {
        ScrollAlign::Start => start - view_start,
        ScrollAlign::End => start + size - view_start - view_size,
        ScrollAlign::Center => start + size / 2.0 - view_start - view_size / 2.0,
        ScrollAlign::Nearest if start < view_start && start + size > view_start + view_size => 0.0,
        ScrollAlign::Nearest if start < view_start => {
            if size <= view_size {
                start - view_start
            } else {
                start + size - view_start - view_size
            }
        }
        ScrollAlign::Nearest if start + size > view_start + view_size => {
            (start + size - view_start - view_size).min(start - view_start)
        }
        ScrollAlign::Nearest => 0.0,
    }
}

#[cfg(test)]
mod tests;

/// The frame as the web APIs see it.
pub(crate) struct PageView(pub Rc<RefCell<Frame>>);

impl PageView {
    /// Answers a text geometry question about the current rendering.
    fn text<T>(&self, question: impl FnOnce(&Scene, &TextQuery<'_>) -> T) -> T {
        let mut frame = self.0.borrow_mut();
        let (dom, measurer) = (frame.dom.clone(), frame.measurer.clone());
        let rendered = frame.rendered();
        let dom = dom.borrow();
        let query = TextQuery {
            dom: &dom,
            styles: &rendered.styles,
            layout: &rendered.layout,
            measurer: measurer.as_ref(),
        };
        question(&rendered.scene, &query)
    }
}

impl Page for PageView {
    fn url(&self) -> String {
        self.0.borrow().url.clone()
    }

    fn rect(&self, node: NodeId) -> Option<Rect> {
        self.0.borrow_mut().rendered().scene.client_rect(node)
    }

    fn client(&self, node: NodeId) -> Option<Rect> {
        let mut frame = self.0.borrow_mut();
        let root = frame.dom.borrow().document_element() == Some(node);
        let viewport = frame.viewport();
        let geometry = frame.rendered().layout.boxes.get(&node).copied()?;
        // The root element's client area is the viewport (CSSOM View), where
        // scrollbars overlay the content.
        let (width, height) = if root {
            viewport
        } else {
            let client = geometry.client_box();
            (client.width, client.height)
        };
        Some(Rect::new(
            geometry.border[3],
            geometry.border[0],
            width,
            height,
        ))
    }

    fn scroll_size(&self, node: NodeId) -> Option<(f32, f32)> {
        let mut frame = self.0.borrow_mut();
        let root = frame.dom.borrow().document_element() == Some(node);
        let (width, height) = frame.viewport();
        let rendered = frame.rendered();
        if root {
            return Some((
                rendered.layout.width.max(width),
                rendered.layout.height.max(height),
            ));
        }
        let geometry = rendered.layout.boxes.get(&node)?;
        Some((geometry.content_width, geometry.content_height))
    }

    fn computed_style(&self, node: NodeId) -> Vec<(String, String)> {
        self.0
            .borrow_mut()
            .style_of(node)
            .map_or(Vec::new(), |style| {
                PropertyId::ALL
                    .iter()
                    .map(|id| (id.name().to_owned(), style.resolved_value_of(*id, None)))
                    .chain(
                        style
                            .custom
                            .iter()
                            .map(|(name, value)| (name.clone(), value.clone())),
                    )
                    .collect()
            })
    }

    fn computed_style_value(&self, node: NodeId, name: &str) -> String {
        let mut frame = self.0.borrow_mut();
        let Some(style) = frame.style_of(node) else {
            return String::new();
        };
        let used = if ComputedStyle::resolves_against_layout(name) {
            let dom = frame.dom.clone();
            let dom = dom.borrow();
            frame
                .rendered()
                .layout
                .boxes
                .get(&node)
                .and_then(|geometry| geometry.used_box(&dom, node, &style))
        } else {
            None
        };
        style
            .resolved_value(name, used.as_ref())
            .unwrap_or_default()
    }

    fn viewport(&self) -> (f32, f32) {
        self.0.borrow().viewport()
    }

    fn device_pixel_ratio(&self) -> f32 {
        self.0.borrow().environment.media.resolution as f32
    }

    fn element_from_point(&self, x: f32, y: f32) -> Option<NodeId> {
        self.0.borrow_mut().rendered().scene.hit_test(x, y)
    }

    fn caret_from_point(&self, x: f32, y: f32) -> Option<(NodeId, u32)> {
        self.text(|scene, query| scene.caret_from_point(query, x, y))
    }

    fn text_rects(&self, node: NodeId, start: u32, end: u32) -> Vec<Rect> {
        self.text(|scene, query| scene.text_rects(query, node, start, end))
    }

    fn match_media(&self, query: &str) -> bool {
        let tokens: Vec<_> = tokenize(query)
            .into_iter()
            .map(|spanned| spanned.token)
            .collect();
        parse_media_query_list(&tokens).matches(&self.0.borrow().environment.media)
    }

    fn focus(&self, node: Option<NodeId>) {
        self.0.borrow_mut().focus = Some(node);
    }

    fn complete_composition(&self) -> Option<String> {
        nativeboat_win32::complete_composition(self.0.borrow().window)
    }

    fn scroll_into_view(&self, node: NodeId, block: ScrollAlign, inline: ScrollAlign) {
        self.0.borrow_mut().scroll_into_view(node, block, inline);
    }

    fn image_size(&self, url: &str) -> Option<(f32, f32)> {
        let frame = self.0.borrow();
        frame.image_key(url).and_then(|key| (frame.images)(&key))
    }

    fn load_storage(&self) -> Option<String> {
        let load = self.0.borrow().load_storage.clone();
        load()
    }

    fn save_storage(&self, data: &str) {
        let save = self.0.borrow().save_storage.clone();
        save(data);
    }

    fn sheet_rules(&self, owner: NodeId, rules: &str) {
        let mut frame = self.0.borrow_mut();
        frame.inserted.insert(owner, rules.to_owned());
        frame.sheets += 1;
    }

    fn text_selection(&self, node: NodeId) -> Option<(u32, u32)> {
        let frame = self.0.borrow();
        if let Some(selection) = frame.selections.get(&node) {
            return Some(*selection);
        }
        nativeboat_win32::text_selection(frame.window, node.index())
            .map(|(start, end)| (start as u32, end as u32))
    }

    fn select_text(&self, node: NodeId, start: u32, end: u32) {
        self.0.borrow_mut().selections.insert(node, (start, end));
    }

    fn read_clipboard(&self) -> Option<String> {
        Some(nativeboat_win32::read_text())
    }

    fn open_select(&self, node: NodeId) {
        self.0.borrow_mut().select = Some(node);
    }

    fn write_clipboard(&self, text: &str) -> bool {
        self.write_clipboard_contents(Some(text), None)
    }

    fn read_clipboard_contents(&self) -> Option<Vec<(String, String)>> {
        nativeboat_win32::read_clipboard_contents().map(|contents| {
            contents
                .into_iter()
                .map(|(name, data)| (name.to_owned(), data))
                .collect()
        })
    }

    fn write_clipboard_contents(&self, text: Option<&str>, html: Option<&str>) -> bool {
        nativeboat_win32::write_clipboard(text, html).is_ok()
    }

    fn canvas_draw(&self, node: NodeId, width: u32, height: u32, commands: &str) {
        let mut commands: Vec<CanvasCommand> = serde_json::from_str(commands).unwrap_or_default();
        let mut frame = self.0.borrow_mut();
        // Scripts name images by URL and canvases by node; the platform
        // knows them by asset key and bitmap key.
        for command in &mut commands {
            if let CanvasCommand::Image { source, .. } = command {
                match source {
                    CanvasSource::Url(url) => {
                        if let Some(key) = frame.image_key(url) {
                            *url = key;
                        }
                    }
                    CanvasSource::Canvas(other) => {
                        *other = frame
                            .canvases
                            .get(&NodeId(*other as u32))
                            .copied()
                            .unwrap_or(0);
                    }
                }
            }
        }
        let key = *frame
            .canvases
            .entry(node)
            .or_insert_with(|| NEXT_CANVAS.fetch_add(1, Ordering::Relaxed));
        nativeboat_win32::canvas_draw(key, width, height, &commands);
        let dom = frame.dom.clone();
        drop(frame);
        dom.borrow_mut().canvas_drawn(node, key);
    }

    fn canvas_read(&self, node: NodeId, (x, y, width, height): (i32, i32, u32, u32)) -> Vec<u8> {
        match self.0.borrow().canvases.get(&node) {
            Some(key) => nativeboat_win32::canvas_read(*key, x, y, width, height),
            None => vec![0; width as usize * height as usize * 4],
        }
    }

    fn canvas_write(
        &self,
        node: NodeId,
        (x, y, width, height): (i32, i32, u32, u32),
        pixels: &[u8],
    ) {
        let key = self.0.borrow().canvases.get(&node).copied();
        if let Some(key) = key {
            nativeboat_win32::canvas_write(key, x, y, width, height, pixels);
            self.0.borrow().dom.borrow_mut().canvas_drawn(node, key);
        }
    }

    fn canvas_url(&self, node: NodeId, jpeg: bool) -> String {
        let key = self.0.borrow().canvases.get(&node).copied();
        match key.and_then(|key| nativeboat_win32::canvas_encode(key, jpeg)) {
            Some(bytes) => {
                let kind = if jpeg { "jpeg" } else { "png" };
                format!("data:image/{kind};base64,{}", STANDARD.encode(bytes))
            }
            None => "data:,".to_owned(),
        }
    }

    fn parse_color(&self, text: &str) -> Option<[f32; 4]> {
        let values = without_whitespace(&parse_component_values(text));
        let [value] = values.as_slice() else {
            return None;
        };
        let color = match parse_color(value, &self.0.borrow().environment.system)? {
            SpecifiedColor::Color(color) => color,
            SpecifiedColor::CurrentColor => nativeboat_css::Color::BLACK,
        };
        Some([
            f32::from(color.r),
            f32::from(color.g),
            f32::from(color.b),
            color.a,
        ])
    }

    fn parse_font(&self, text: &str) -> Option<String> {
        let longhands = expand("font", &parse_component_values(text))?;
        let specified = SpecifiedValues {
            values: longhands
                .into_iter()
                .map(|(id, value)| (id, Specified::Value(value)))
                .collect(),
            ..SpecifiedValues::default()
        };
        let environment = self.0.borrow().environment;
        let style = nativeboat_css::ComputedStyle::compute(&specified, None, &environment);
        serde_json::to_string(&nativeboat_layout::text::font_of(&style)).ok()
    }

    fn measure_text(&self, font: &str, text: &str) -> [f32; 3] {
        let font: Font = serde_json::from_str(font).unwrap_or_default();
        let (width, ascent, descent) = nativeboat_win32::measure_text(&font, text);
        [width, ascent, descent]
    }

    fn path_data(&self, data: &str) -> String {
        serde_json::to_string(&nativeboat_layout::svg::path_data(data)).unwrap_or_default()
    }
}
