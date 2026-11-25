//! Live document geometry for compiled renderer pages. Platform services stay
//! behind `Page`; style, layout, hit testing and text geometry use the same
//! document and scene that the host paints.

#![no_std]

extern crate alloc;
// Tests mock platform calls, which names `std` at the crate root.
#[cfg(test)]
#[macro_use]
extern crate std;

use alloc::{collections::BTreeMap, rc::Rc, string::String, vec::Vec};
use core::{
    cell::{Cell, RefCell},
    fmt,
};
use nativeboat_css::types::{
    AppRegion, ContainerType, Cursor, Edges, LengthPercentageAuto, Overflow, Px,
};
use nativeboat_css::{
    Color, ComputedStyle, Origin, PropertyId, RuleMap, SheetRef, StyleEnvironment, UsedBox,
    media::MediaEnvironment, parse_stylesheet, user_agent_stylesheet,
};
use nativeboat_dom::{Dom, NodeId};
use nativeboat_ir::json::{self, Json};
use nativeboat_ir::{CanvasCommand, CanvasSource, DisplayItem, Font, NativeTree, PathSegment};
use nativeboat_layout::motion::{Keyframes, Motion, TransitionEvent};
use nativeboat_layout::resize::{self, Resizing};
use nativeboat_layout::scrollbars;
use nativeboat_layout::{
    Highlight, LayoutCache, LayoutOptions, LayoutResult, Scene, Styles, TextMeasurer, TextQuery,
    layout_cached, scene_over,
};
use nativeboat_native::web::{MediaSource, Page, Rect, ScrollAlign};

mod assets;
mod window;

pub use window::{PageWindow, PageWindows, electron_before_input};

/// The document's selection and focused element, which the scene paints,
/// and how many times the word at the caret was spell checked after typing
/// paused.
pub type Highlighted = (Option<[(NodeId, u32); 2]>, Option<NodeId>, u64);

/// What a rendering of a page shows: the page's revision and highlights.
pub type RenderState = ((u64, u64), Highlighted);

/// How long typing pauses before the word at the caret is spell checked, in
/// milliseconds.
const TYPING_PAUSE: f64 = 1000.0;

/// The computed styles of one version of the document, its style sheets
/// and its environment: style queries need no layout.
struct Styled {
    version: u64,
    sheets: u64,
    environment: StyleEnvironment,
    styles: Rc<Styles>,
}

/// The layout and scene of computed styles, which it shares.
#[derive(Clone)]
struct Rendered {
    /// The document version laid out, as styles are kept across versions
    /// that change no selector's match.
    version: u64,
    highlighted: Highlighted,
    styles: Rc<Styles>,
    /// The layout viewport laid out in.
    viewport: (f32, f32),
    layout: LayoutResult,
    scene: Rc<Scene>,
}

/// The rules of the current style sheets indexed for their media, kept
/// between restyles with the parsed declarations the index caches. The
/// index borrows the sheets this also holds.
struct IndexedSheets {
    // Declared before the sheets it borrows, so it is dropped first.
    map: RuleMap<'static>,
    media: MediaEnvironment,
    _sheets: Vec<Rc<nativeboat_css::Stylesheet>>,
}

impl IndexedSheets {
    fn new(sheets: &[Rc<nativeboat_css::Stylesheet>], media: MediaEnvironment) -> IndexedSheets {
        let sheets = sheets.to_vec();
        let refs: Vec<SheetRef<'static>> = core::iter::once(SheetRef {
            sheet: user_agent_stylesheet(),
            origin: Origin::UserAgent,
        })
        .chain(sheets.iter().map(|sheet| SheetRef {
            // SAFETY: the sheet is shared through an `Rc` this value holds
            // and drops after the index, so it stays in place for as long
            // as the index refers to it.
            sheet: unsafe { &*Rc::as_ptr(sheet) },
            origin: Origin::Author,
        }))
        .collect();
        IndexedSheets {
            map: RuleMap::new(&refs, &media),
            media,
            _sheets: sheets,
        }
    }
}

struct ParsedSheets {
    imported_version: u64,
    /// The document's and its shadow trees' sheets: each source with the
    /// tree it applies in (0 for the document).
    document_sources: Vec<(String, u32)>,
    /// The source and tree of each sheet, imported and document sheets
    /// alike.
    sources: Vec<(String, u32)>,
    sheets: Vec<Rc<nativeboat_css::Stylesheet>>,
    /// Whether a sheet uses container query units, whose values depend on
    /// the laid-out size of their containers.
    container_units: bool,
}

/// Whether CSS text uses container query units.
fn uses_container_units(source: &str) -> bool {
    let bytes = source.as_bytes();
    source.match_indices("cq").any(|(at, _)| {
        let digit_before = at > 0 && (bytes[at - 1].is_ascii_digit() || bytes[at - 1] == b'.');
        let rest = &source[at + 2..];
        digit_before
            && ["w", "h", "i", "b", "min", "max"]
                .iter()
                .any(|unit| rest.starts_with(unit))
    })
}

/// A text node's text when it was spell checked, and its misspelled ranges.
type Checked = (String, Vec<(u32, u32)>);

/// A renderer page whose Web API queries and painting share native geometry.
pub struct DocumentPage {
    platform: Rc<dyn Page>,
    measurer: Rc<dyn TextMeasurer>,
    /// The native window showing the page, once one does.
    window: Cell<Option<nativeboat_win32::WindowId>>,
    /// Text selections scripts made in text controls since the last render.
    selections: RefCell<BTreeMap<NodeId, (u32, u32)>>,
    /// The file that keeps the page's `localStorage`, when the application
    /// has one.
    storage: RefCell<Option<String>>,
    /// The platform bitmaps of the page's canvases.
    canvases: RefCell<BTreeMap<NodeId, u64>>,
    /// A select the page draws whose options a script or the user opened.
    select_request: Cell<Option<NodeId>>,
    document: RefCell<Option<Rc<RefCell<Dom>>>>,
    stylesheets: RefCell<Vec<String>>,
    inserted: RefCell<BTreeMap<NodeId, String>>,
    /// The text of each style sheet `<link>` elements name, by file, with
    /// its `url()`s made absolute; `None` for those that did not load.
    linked: RefCell<BTreeMap<String, Option<String>>>,
    sheet_version: Cell<u64>,
    view_version: Cell<u64>,
    environment: Cell<StyleEnvironment>,
    /// The room the window's scrollbars take from the layout viewport: the
    /// horizontal bar's height and the vertical bar's width.
    scrollbar_room: Cell<(f32, f32)>,
    /// Whether the window shows what lies behind it where the page paints
    /// nothing, as a transparent Electron window does.
    transparent: Cell<bool>,
    styled: RefCell<Option<Rc<Styled>>>,
    rendered: RefCell<Option<Rc<Rendered>>>,
    /// The last layout's tree, whose unchanged parts the next one reuses.
    layout_cache: RefCell<LayoutCache>,
    /// The content box size of each query container in the last layout,
    /// which container query units resolve against.
    container_sizes: RefCell<BTreeMap<NodeId, (f32, f32)>>,
    /// Whether container sizes changed since styles were computed.
    containers_stale: Cell<bool>,
    /// Whether a layout is being redone for new container sizes, so it is
    /// redone at most once.
    container_retry: Cell<bool>,
    parsed: RefCell<Option<ParsedSheets>>,
    indexed: RefCell<Option<IndexedSheets>>,
    /// URLs whose images did not load, so layout does not try again.
    missing: RefCell<alloc::collections::BTreeSet<String>>,
    /// Network images layout asked for, which the page downloads.
    wanted: RefCell<Vec<String>>,
    /// The `@font-face` sources loaded or tried: family, weight, style and
    /// URL.
    fonts: RefCell<alloc::collections::BTreeSet<(String, u16, bool, String)>>,
    /// The device pixels per CSS pixel the page is measured for.
    scale: Cell<f32>,
    /// Text nodes last spell checked: their text and its misspelled ranges.
    spelling: RefCell<BTreeMap<NodeId, Checked>>,
    /// When the word at the caret, in text typed into last, is checked:
    /// Chromium marks it once typing pauses.
    spelling_due: Cell<Option<f64>>,
    /// Bumped when that check is due, so the page highlights again.
    spelling_version: Cell<u64>,
    /// The element the page last focused or blurred, for the window to
    /// follow with its keyboard focus.
    focus_request: Cell<Option<Option<NodeId>>>,
    /// CSS transitions from earlier computed styles to the current ones.
    motion: RefCell<Motion>,
    /// The time of the frame being shown, in milliseconds: transitions are
    /// sampled at it, so every query in a frame sees the same styles.
    frame_time: Cell<f64>,
    /// The styles transitions present at the frame time.
    presented: RefCell<Option<(f64, Rc<Styles>)>>,
    /// The `@keyframes` rules of the style sheets for the media they were
    /// collected for, by sheet version.
    keyframes: RefCell<Option<(u64, MediaEnvironment, Rc<Keyframes>)>>,
}

impl fmt::Debug for DocumentPage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DocumentPage")
            .field("attached", &self.document.borrow().is_some())
            .field("environment", &self.environment.get())
            .finish_non_exhaustive()
    }
}

impl DocumentPage {
    /// Creates the view before the realm attaches its document. The platform
    /// supplies services unrelated to document layout.
    pub fn new(platform: Rc<dyn Page>, measurer: Rc<dyn TextMeasurer>) -> Self {
        let mut environment = StyleEnvironment::default();
        let viewport = platform.viewport();
        environment.media.width = f64::from(viewport.0);
        environment.media.height = f64::from(viewport.1);
        environment.media.resolution = f64::from(platform.device_pixel_ratio());
        environment.standard_font = measurer.standard_family();
        Self {
            platform,
            measurer,
            window: Cell::new(None),
            selections: RefCell::new(BTreeMap::new()),
            storage: RefCell::new(None),
            canvases: RefCell::new(BTreeMap::new()),
            select_request: Cell::new(None),
            document: RefCell::new(None),
            stylesheets: RefCell::new(Vec::new()),
            inserted: RefCell::new(BTreeMap::new()),
            linked: RefCell::new(BTreeMap::new()),
            sheet_version: Cell::new(0),
            view_version: Cell::new(0),
            environment: Cell::new(environment),
            scrollbar_room: Cell::new((0.0, 0.0)),
            transparent: Cell::new(false),
            styled: RefCell::new(None),
            rendered: RefCell::new(None),
            layout_cache: RefCell::default(),
            container_sizes: RefCell::new(BTreeMap::new()),
            containers_stale: Cell::new(false),
            container_retry: Cell::new(false),
            parsed: RefCell::new(None),
            indexed: RefCell::new(None),
            missing: RefCell::new(alloc::collections::BTreeSet::new()),
            wanted: RefCell::new(Vec::new()),
            fonts: RefCell::new(alloc::collections::BTreeSet::new()),
            focus_request: Cell::new(None),
            spelling: RefCell::new(BTreeMap::new()),
            spelling_due: Cell::new(None),
            spelling_version: Cell::new(0),
            scale: Cell::new(0.0),
            motion: RefCell::default(),
            frame_time: Cell::new(nativeboat_native::platform::now()),
            presented: RefCell::new(None),
            keyframes: RefCell::new(None),
        }
    }

    /// Shows the page in a native window, whose text controls and the
    /// system's clipboard scripts then use.
    pub fn attach_window(&self, window: nativeboat_win32::WindowId) {
        self.window.set(Some(window));
    }

    /// Keeps the page's `localStorage` in a file, which later runs of the
    /// application read again.
    pub fn set_storage_file(&self, path: String) {
        *self.storage.borrow_mut() = Some(path);
    }

    /// The text selections scripts made in text controls since the last
    /// call, to apply once the controls show their text.
    pub fn take_selections(&self) -> Vec<(NodeId, (u32, u32))> {
        core::mem::take(&mut *self.selections.borrow_mut())
            .into_iter()
            .collect()
    }

    /// The select whose options to show, once asked.
    pub fn take_select_request(&self) -> Option<NodeId> {
        self.select_request.take()
    }

    /// The options of a select, their labels with spaces collapsed, which
    /// one is selected, and the select's box in window coordinates.
    pub fn select_options(
        &self,
        node: NodeId,
    ) -> Option<(Vec<String>, Option<usize>, nativeboat_ir::Rect)> {
        let rect = self.scene()?.client_rect(node)?;
        let document = self.document.borrow().clone()?;
        let dom = document.borrow();
        let options = dom.options(node);
        let selected = dom.selected_options(node);
        let labels = options
            .iter()
            .map(|option| {
                let text = dom.text_content(*option).unwrap_or_default();
                text.split_whitespace().collect::<Vec<_>>().join(" ")
            })
            .collect();
        Some((
            labels,
            options.iter().position(|option| selected.contains(option)),
            rect,
        ))
    }

    /// Whether scripts selected text in controls since the last render.
    pub fn selects_text(&self) -> bool {
        !self.selections.borrow().is_empty()
    }

    /// Starts a frame at `now` (milliseconds): running transitions advance
    /// to it, and the page renders again while they run.
    pub fn advance_frame(&self, now: f64) {
        self.frame_time.set(now);
        if self.motion.borrow().active() {
            self.view_version
                .set(self.view_version.get().wrapping_add(1));
        }
    }

    /// Whether transitions run or have events to deliver, so the page needs
    /// frames.
    pub fn animating(&self) -> bool {
        self.motion.borrow().has_work()
    }

    /// The transition events since they were last taken, for the page's
    /// scripts.
    pub fn take_transition_events(&self) -> Vec<TransitionEvent> {
        core::mem::take(&mut self.motion.borrow_mut().events)
    }

    /// The `@keyframes` rules of the style sheets styles were computed from.
    fn keyframes(&self, styled: &Styled) -> Rc<Keyframes> {
        let media = styled.environment.media;
        let mut cached = self.keyframes.borrow_mut();
        if let Some((_, _, keyframes)) = cached
            .as_ref()
            .filter(|(version, cached, _)| *version == styled.sheets && *cached == media)
        {
            return keyframes.clone();
        }
        let parsed = self.parsed.borrow();
        let sheets = parsed
            .as_ref()
            .map(|parsed| parsed.sheets.as_slice())
            .unwrap_or_default();
        let keyframes = Rc::new(nativeboat_layout::motion::keyframes(
            sheets.iter().map(Rc::as_ref),
            &media,
        ));
        *cached = Some((styled.sheets, media, keyframes.clone()));
        keyframes
    }

    /// The computed styles, and the styles the page shows: while
    /// transitions run, those of the current frame.
    fn presented(&self) -> Option<(Rc<Styled>, Rc<Styles>)> {
        let styled = self.styled()?;
        let mut motion = self.motion.borrow_mut();
        if !motion
            .targets()
            .is_some_and(|targets| Rc::ptr_eq(targets, &styled.styles))
        {
            // Transitions and animations start when styles change, which
            // moves the frame to that time.
            let now = nativeboat_native::platform::now().max(self.frame_time.get());
            self.frame_time.set(now);
            motion.set_keyframes(self.keyframes(&styled));
            motion.update(styled.styles.clone(), now / 1000.0);
            self.presented.borrow_mut().take();
        }
        if !motion.affects() {
            let styles = styled.styles.clone();
            return Some((styled, styles));
        }
        let now = self.frame_time.get() / 1000.0;
        let mut presented = self.presented.borrow_mut();
        if let Some((_, styles)) = presented.as_ref().filter(|(time, _)| *time == now) {
            return Some((styled.clone(), styles.clone()));
        }
        // Descendants that inherit a transitioning value present it too.
        let document = self.document.borrow().clone();
        let styles = Rc::new(match document {
            Some(document) => motion.sample_inherited(now, &document.borrow()),
            None => motion.sample(now),
        });
        *presented = Some((now, styles.clone()));
        Some((styled, styles))
    }

    /// Measures and lays out text for a scale of device pixels per CSS
    /// pixel, as a window's DPI and zoom give it.
    pub fn set_scale(&self, scale: f32) {
        if self.scale.replace(scale) == scale {
            return;
        }
        self.measurer.set_scale(scale);
        let mut environment = self.environment.get();
        environment.media.resolution = f64::from(scale);
        self.set_environment(environment);
        self.view_version
            .set(self.view_version.get().wrapping_add(1));
    }

    /// When the word at the caret is checked, typing having paused.
    pub fn spelling_due(&self) -> Option<f64> {
        self.spelling_due.get()
    }

    /// Highlights the page again for a due check of the word at the caret;
    /// its layout stands.
    pub fn refresh_spelling(&self) {
        if self
            .spelling_due
            .get()
            .is_some_and(|at| nativeboat_native::platform::now() >= at)
        {
            self.spelling_version
                .set(self.spelling_version.get().wrapping_add(1));
        }
    }

    /// The misspelled words of the focused editing host's text where
    /// `spellcheck` allows, as Chromium checks the text being edited,
    /// except the word at the caret while it is being typed: a second
    /// after its text last changed, it is checked too.
    fn misspellings(&self, dom: &Dom) -> Vec<(NodeId, Vec<(u32, u32)>)> {
        // The cache is out of its cell while the system spell checker runs:
        // its COM calls can pump window messages, which paint the page again
        // and so check it again.
        let mut cache = core::mem::take(&mut *self.spelling.borrow_mut());
        // A host that turns spell checking off is not checked, as MarkText's
        // editor is while its spell checker is off.
        let host = dom.focused().filter(|host| {
            dom.editing_host(*host) == Some(*host) && spellchecked_element(dom, *host)
        });
        let Some(host) = host.filter(|_| nativeboat_native::spelling::enabled()) else {
            return Vec::new();
        };
        let caret = dom
            .selection()
            .filter(|[start, end]| start == end)
            .map(|[point, _]| point);
        let now = nativeboat_native::platform::now();
        let mut found = Vec::new();
        let mut checked = BTreeMap::new();
        for node in dom.descendants(host) {
            let Some(text) = dom
                .data(node)
                .filter(|_| dom.node_type(node) == nativeboat_dom::node_type::TEXT)
            else {
                continue;
            };
            if !spellchecked(dom, node) {
                continue;
            }
            let ranges = match cache.remove(&node) {
                Some((known, ranges)) if known == text => ranges,
                _ => {
                    if caret.is_some_and(|(at, _)| at == node) {
                        self.spelling_due.set(Some(now + TYPING_PAUSE));
                    }
                    let units: Vec<u16> = text.encode_utf16().collect();
                    nativeboat_native::spelling::misspellings(&units)
                }
            };
            let typing = self.spelling_due.get().is_some_and(|at| now < at);
            let shown: Vec<(u32, u32)> = ranges
                .iter()
                .copied()
                .filter(|(start, end)| {
                    !typing
                        || caret.is_none_or(|(at, offset)| {
                            at != node || offset < *start || offset > *end
                        })
                })
                .collect();
            if !shown.is_empty() {
                found.push((node, shown));
            }
            checked.insert(node, (String::from(text), ranges));
        }
        drop(cache);
        *self.spelling.borrow_mut() = checked;
        if self.spelling_due.get().is_some_and(|at| now >= at) {
            self.spelling_due.set(None);
        }
        found
    }

    /// Takes the focus change the page asked for since the last call:
    /// `Some(None)` when it blurred its element.
    pub fn take_focus_request(&self) -> Option<Option<NodeId>> {
        self.focus_request.take()
    }

    /// The natural size of an image: the platform's, else one this page
    /// loads from its URL once.
    /// The style sheet a `<link rel="stylesheet">` loads: its file's text,
    /// read once, under its `media` query.
    fn linked_sheet(&self, dom: &Dom, link: NodeId) -> Option<String> {
        let rel = dom.get_attribute(link, "rel")?.to_ascii_lowercase();
        let tokens: Vec<&str> = rel.split_ascii_whitespace().collect();
        if !tokens.contains(&"stylesheet")
            || tokens.contains(&"alternate")
            || dom.has_attribute(link, "disabled")
        {
            return None;
        }
        let href = dom.get_attribute(link, "href")?.trim();
        if href.is_empty() {
            return None;
        }
        let page = self.platform.url();
        let text = self.sheet_file(&page, href, 0)?;
        Some(under_media(text, dom.get_attribute(link, "media")))
    }

    /// The text of a style sheet file, read once: its `url()`s made
    /// absolute and its `@import`s replaced by the sheets they name.
    fn sheet_file(&self, base: &str, href: &str, depth: u8) -> Option<String> {
        let path = assets::locate(base, href)?;
        if let Some(known) = self.linked.borrow().get(&path).cloned() {
            return known;
        }
        let text = assets::fetch(base, href).map(|bytes| {
            let text = String::from_utf8_lossy(&bytes);
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            let text = assets::rebase_urls(text, &path);
            self.with_imports(&text, &path, depth)
        });
        self.linked.borrow_mut().insert(path, text.clone());
        text
    }

    /// Style sheet text with its leading `@import`s replaced by the sheets
    /// they name, from files beside `base`, as deep as browsers allow.
    fn with_imports(&self, css: &str, base: &str, depth: u8) -> String {
        let (imports, rest) = assets::leading_imports(css);
        if imports.is_empty() || depth >= 8 {
            return String::from(css);
        }
        let mut text = String::new();
        for (url, media) in imports {
            if let Some(imported) = self.sheet_file(base, &url, depth + 1) {
                text.push_str(&under_media(imported, Some(&media)));
                text.push('\n');
            }
        }
        text.push_str(&css[rest..]);
        text
    }

    /// Lays out again what a newly added image sizes.
    fn image_added(&self, url: &str) {
        self.missing.borrow_mut().remove(url);
        // Boxes the image sizes are laid out again: the layout cache does
        // not know image sizes.
        *self.layout_cache.borrow_mut() = LayoutCache::default();
        if let Some(document) = self.document.borrow().clone() {
            document.borrow_mut().resource_loaded();
        }
    }

    fn image_size_of(&self, url: &str) -> Option<(f32, f32)> {
        if let Some(size) = self
            .platform
            .image_size(url)
            .or_else(|| nativeboat_win32::image_size(url))
        {
            return Some(size);
        }
        // A video's frame is there once the page has read it; its URL names
        // the video, not an image to load.
        if url.starts_with(nativeboat_layout::VIDEO_FRAME) || self.missing.borrow().contains(url) {
            return None;
        }
        // A network image is missing until the page downloads it, which
        // adds it.
        if assets::is_network(url) {
            self.missing.borrow_mut().insert(url.into());
            self.wanted.borrow_mut().push(url.into());
            return None;
        }
        let loaded = assets::fetch(&self.platform.url(), url)
            .and_then(|bytes| nativeboat_win32::add_image(url, &bytes));
        if loaded.is_none() {
            self.missing.borrow_mut().insert(url.into());
        }
        loaded
    }

    /// Loads the fonts the `@font-face` rules of style sheets declare: for
    /// each face, its first source that loads.
    fn load_fonts(&self, sheets: &[Rc<nativeboat_css::Stylesheet>]) {
        fn faces<'a>(
            rules: &'a [nativeboat_css::Rule],
            found: &mut Vec<&'a [nativeboat_css::parser::Declaration]>,
        ) {
            for rule in rules {
                match rule {
                    nativeboat_css::Rule::FontFace(declarations) => found.push(declarations),
                    nativeboat_css::Rule::Media { rules, .. }
                    | nativeboat_css::Rule::Supports { rules, .. }
                    | nativeboat_css::Rule::Layer { rules, .. } => faces(rules, found),
                    _ => {}
                }
            }
        }
        let mut declared = Vec::new();
        for sheet in sheets {
            faces(&sheet.rules, &mut declared);
        }
        let page = self.platform.url();
        for declarations in declared {
            let Some(face) = assets::font_face(declarations) else {
                continue;
            };
            for url in &face.urls {
                let key = (
                    face.family.clone(),
                    face.weights.0,
                    face.italic,
                    url.clone(),
                );
                if !self.fonts.borrow_mut().insert(key) {
                    break;
                }
                let loaded = assets::fetch(&page, url)
                    .and_then(|file| {
                        // WOFF2 needs Brotli and table transforms, which
                        // DirectWrite undoes.
                        nativeboat_native::woff::font_data(&file)
                            .or_else(|| nativeboat_win32::unpack_font(&file))
                    })
                    .is_some_and(|font| {
                        nativeboat_win32::register_font_face(
                            &face.family,
                            face.weights,
                            face.italic,
                            &face.ranges,
                            &font,
                        )
                    });
                if loaded {
                    break;
                }
            }
        }
    }

    /// Attaches the live arena returned by `Realm::install_web`.
    pub fn set_document(&self, document: Rc<RefCell<Dom>>) {
        *self.document.borrow_mut() = Some(document);
        self.rendered.borrow_mut().take();
        self.parsed.borrow_mut().take();
        self.inserted.borrow_mut().clear();
        self.view_version
            .set(self.view_version.get().wrapping_add(1));
    }

    /// Changes to the document, stylesheet set, or viewport require painting.
    /// What a rendering of the page shows: its revision, and the selection
    /// and focus it highlights.
    pub fn render_state(&self) -> RenderState {
        let spelling = self.spelling_version.get();
        let highlighted = self
            .document
            .borrow()
            .as_ref()
            .map_or((None, None, spelling), |dom| {
                let dom = dom.borrow();
                (dom.selection(), dom.focused(), spelling)
            });
        (self.revision(), highlighted)
    }

    pub fn revision(&self) -> (u64, u64) {
        (
            self.document
                .borrow()
                .as_ref()
                .map_or(0, |dom| dom.borrow().version()),
            self.view_version.get(),
        )
    }

    /// Sets viewport and media preferences used by layout and media queries.
    pub fn set_environment(&self, environment: StyleEnvironment) {
        if self.environment.get() != environment {
            self.environment.set(environment);
            self.view_version
                .set(self.view_version.get().wrapping_add(1));
        }
    }

    /// Updates the viewport after a native window resize: the whole client
    /// area, which media queries, viewport units and `innerWidth` see, as
    /// Chromium's include the window's scrollbars.
    pub fn set_viewport(&self, width: f32, height: f32) {
        let mut environment = self.environment.get();
        environment.media.width = f64::from(width);
        environment.media.height = f64::from(height);
        self.set_environment(environment);
    }

    /// Sets the room the window's scrollbars take from the layout viewport.
    pub fn set_scrollbar_room(&self, room: (f32, f32)) {
        if self.scrollbar_room.get() != room {
            self.scrollbar_room.set(room);
            self.view_version
                .set(self.view_version.get().wrapping_add(1));
        }
    }

    /// Sets whether the window shows what lies behind it where the page
    /// paints no background, instead of white.
    pub fn set_transparent(&self, transparent: bool) {
        if self.transparent.get() != transparent {
            self.transparent.set(transparent);
            // The scene paints over the new base when next asked for.
            self.rendered.borrow_mut().take();
        }
    }

    /// The layout viewport: the viewport less the room the window's
    /// scrollbars take, which the root lays out in and its `clientWidth`
    /// and `clientHeight` report.
    fn layout_viewport(&self) -> (f32, f32) {
        let (width, height) = self.viewport();
        let (across, down) = self.scrollbar_room.get();
        ((width - down).max(0.0), (height - across).max(0.0))
    }

    /// Activates styles imported by newly evaluated renderer modules.
    pub fn extend_stylesheets(&self, stylesheets: Vec<String>) {
        if !stylesheets.is_empty() {
            self.stylesheets.borrow_mut().extend(stylesheets);
            self.sheet_version
                .set(self.sheet_version.get().wrapping_add(1));
            self.view_version
                .set(self.view_version.get().wrapping_add(1));
        }
    }

    /// The document's computed styles, brought up to date.
    fn styled(&self) -> Option<Rc<Styled>> {
        let document = self.document.borrow().clone()?;
        let version = document.borrow().version();
        let environment = self.environment.get();
        let sheet_version = self.sheet_version.get();
        let current = !self.containers_stale.get()
            && self.styled.borrow().as_ref().is_some_and(|styled| {
                styled.version == version
                    && styled.environment == environment
                    && styled.sheets == sheet_version
            });
        if current {
            return self.styled.borrow().clone();
        }
        let dom = document.borrow();
        let mut document_sources = Vec::new();
        // `<style>` elements and style sheet `<link>`s, in document order,
        // then those of each connected shadow tree, which apply in that tree
        // alone.
        let mut trees = alloc::vec![(Dom::DOCUMENT, 0)];
        trees.extend(
            dom.shadow_roots()
                .into_iter()
                .filter(|root| dom.is_connected(*root))
                .map(|root| (root, root.0)),
        );
        for (tree, scope) in trees {
            for owner in dom.get_elements_by_tag_name(tree, "*") {
                match dom.local_name(owner) {
                    Some("style") => {
                        let mut source = dom.text_content(owner).unwrap_or_default();
                        if let Some(inserted) = self.inserted.borrow().get(&owner) {
                            source.push('\n');
                            source.push_str(inserted);
                        }
                        // Imports load from beside the page.
                        if source.contains("@import") {
                            source = self.with_imports(&source, &self.platform.url(), 0);
                        }
                        document_sources.push((source, scope));
                    }
                    Some("link") => {
                        if let Some(source) = self.linked_sheet(&dom, owner) {
                            document_sources.push((source, scope));
                        }
                    }
                    _ => {}
                }
            }
        }
        let changed = self.parsed.borrow().as_ref().is_none_or(|parsed| {
            parsed.imported_version != sheet_version || parsed.document_sources != document_sources
        });
        if changed {
            // Pages add `<style>` elements one at a time; only sources not
            // parsed before are parsed.
            let sources: Vec<(String, u32)> = self
                .stylesheets
                .borrow()
                .iter()
                .map(|source| (source.clone(), 0))
                .chain(document_sources.iter().cloned())
                .collect();
            let previous = self.parsed.borrow_mut().take();
            let parsed_before: BTreeMap<(&str, u32), &Rc<nativeboat_css::Stylesheet>> = previous
                .as_ref()
                .map(|parsed| {
                    parsed
                        .sources
                        .iter()
                        .map(|(source, scope)| (source.as_str(), *scope))
                        .zip(&parsed.sheets)
                        .collect()
                })
                .unwrap_or_default();
            let sheets: Vec<Rc<nativeboat_css::Stylesheet>> = sources
                .iter()
                .map(|(source, scope)| {
                    parsed_before.get(&(source.as_str(), *scope)).map_or_else(
                        || {
                            let mut sheet = parse_stylesheet(source).0;
                            sheet.scope = *scope;
                            Rc::new(sheet)
                        },
                        |sheet| Rc::clone(sheet),
                    )
                })
                .collect();
            self.load_fonts(&sheets);
            let container_units = sources
                .iter()
                .any(|(source, _)| uses_container_units(source));
            *self.parsed.borrow_mut() = Some(ParsedSheets {
                imported_version: sheet_version,
                document_sources,
                sources,
                sheets,
                container_units,
            });
        }
        let parsed = self.parsed.borrow();
        let container_units = parsed.as_ref().is_some_and(|parsed| parsed.container_units);
        let parsed = &parsed.as_ref().expect("stylesheets were parsed").sheets;
        let mut indexed = self.indexed.borrow_mut();
        if changed
            || indexed
                .as_ref()
                .is_none_or(|indexed| indexed.media != environment.media)
        {
            *indexed = None;
            *indexed = Some(IndexedSheets::new(parsed, environment.media));
        }
        let rules = &indexed.as_ref().expect("the sheets were indexed").map;
        let previous = self.styled.borrow_mut().take();
        let started = nativeboat_native::platform::now();
        let previous = previous.as_ref().filter(|_| !changed);
        // A change that no selector sees, as typing is, keeps every style.
        let stale = self.containers_stale.replace(false);
        let styles = match previous
            .filter(|_| !stale)
            .filter(|styled| styled.styles.reusable(&dom, &environment))
        {
            Some(styled) => styled.styles.clone(),
            None if container_units => {
                let sizes = self.container_sizes.borrow();
                Rc::new(Styles::compute_with_containers(
                    &dom,
                    rules,
                    &environment,
                    previous.map(|styled| styled.styles.as_ref()),
                    &|node| sizes.get(&node).copied(),
                ))
            }
            None => Rc::new(Styles::compute_with_rules(
                &dom,
                rules,
                &environment,
                previous.map(|styled| styled.styles.as_ref()),
            )),
        };
        nativeboat_native::platform::trace_layout(|| {
            let [styled, matched, kept] = styles.stats();
            alloc::format!(
                "style {:.2} ms{}, {styled} elements, {matched} matches and {kept} styles reused",
                nativeboat_native::platform::now() - started,
                if changed { " (sheets changed)" } else { "" }
            )
        });
        let styled = Rc::new(Styled {
            version,
            sheets: sheet_version,
            environment,
            styles,
        });
        *self.styled.borrow_mut() = Some(styled.clone());
        Some(styled)
    }

    /// The box layout gave an element, for resolved style values.
    fn used_box(&self, node: NodeId, style: &ComputedStyle) -> Option<UsedBox> {
        let rendered = self.rendered()?;
        let document = self.document.borrow().clone()?;
        let dom = document.borrow();
        rendered
            .layout
            .boxes
            .get(&node)?
            .used_box(&dom, node, style)
    }

    fn rendered(&self) -> Option<Rc<Rendered>> {
        let (styled, styles) = self.presented()?;
        let document = self.document.borrow().clone()?;
        let highlighted = {
            let dom = document.borrow();
            (dom.selection(), dom.focused(), self.spelling_version.get())
        };
        let viewport = self.layout_viewport();
        let current = self.rendered.borrow().as_ref().map(|rendered| {
            (
                rendered.version == styled.version
                    && Rc::ptr_eq(&rendered.styles, &styles)
                    && rendered.viewport == viewport,
                rendered.highlighted == highlighted,
            )
        });
        if current == Some((true, true)) {
            return self.rendered.borrow().clone();
        }
        let dom = document.borrow();
        let parsed = self.parsed.borrow();
        let sheets = sheet_refs(&parsed.as_ref().expect("stylesheets were parsed").sheets);
        let previous = self.rendered.borrow_mut().take();
        let layout = match (current, previous) {
            // Only the selection or the focus moved: the layout stands.
            (Some((true, false)), Some(previous)) => Rc::unwrap_or_clone(previous).layout,
            _ => {
                let image_size = |url: &str| self.image_size_of(url);
                let started = nativeboat_native::platform::now();
                let mut cache = self.layout_cache.borrow_mut();
                let layout = layout_cached(
                    &dom,
                    &styles,
                    &LayoutOptions {
                        viewport,
                        measurer: self.measurer.as_ref(),
                        image_size: &image_size,
                    },
                    &mut cache,
                );
                nativeboat_native::platform::trace_layout(|| {
                    alloc::format!(
                        "layout {:.2} ms, {} boxes, {} reused",
                        nativeboat_native::platform::now() - started,
                        layout.boxes.len(),
                        cache.reused()
                    )
                });
                layout
            }
        };
        // Container query units resolve against the containers' laid-out
        // sizes: when those moved, styles and layout run once more.
        let container_units = parsed.as_ref().is_some_and(|parsed| parsed.container_units);
        if container_units {
            let sizes: BTreeMap<NodeId, (f32, f32)> = styles
                .iter()
                .filter(|(_, style)| style.container_type != ContainerType::Normal)
                .filter_map(|(node, _)| {
                    let content = layout.boxes.get(&node)?.content_box();
                    Some((node, (content.width, content.height)))
                })
                .collect();
            let changed = *self.container_sizes.borrow() != sizes;
            if changed {
                *self.container_sizes.borrow_mut() = sizes;
                if !self.container_retry.replace(true) {
                    self.containers_stale.set(true);
                    drop(parsed);
                    drop(dom);
                    let again = self.rendered();
                    self.container_retry.set(false);
                    return again;
                }
            }
            self.container_retry.set(false);
        }
        let started = nativeboat_native::platform::now();
        // The selection and caret paint over the text they cover.
        let mut highlight = Highlight::new(&dom, &styles, &sheets, self.measurer.as_ref());
        highlight.mark_misspellings(&dom, &styles, &sheets, self.misspellings(&dom));
        let base = if self.transparent.get() {
            Color::TRANSPARENT
        } else {
            Color::WHITE
        };
        let scene = scene_over(&dom, &styles, &layout, viewport, Some(&highlight), base);
        nativeboat_native::platform::trace_layout(|| {
            alloc::format!(
                "scene {:.2} ms",
                nativeboat_native::platform::now() - started
            )
        });
        let rendered = Rc::new(Rendered {
            version: styled.version,
            highlighted,
            styles,
            viewport,
            layout,
            scene: Rc::new(scene),
        });
        *self.rendered.borrow_mut() = Some(rendered.clone());
        Some(rendered)
    }

    /// The page laid out for printing on paper of `paper` CSS pixels: with
    /// the `print` media type, at the width between the margins. Margins
    /// left `None` come from the document's `@page` rules, else Chromium's
    /// default of a centimetre. Pages break where they would otherwise cut
    /// through a line of text or an image. The page's screen layout is
    /// restored afterwards.
    pub fn print_layout(
        &self,
        paper: (f32, f32),
        margins: [Option<f32>; 4],
    ) -> Option<PrintLayout> {
        const DEFAULT_MARGIN: f32 = 96.0 / 2.54;
        let screen = self.environment.get();
        let mut print = screen;
        print.media.print = true;
        self.set_environment(print);
        self.styled()?;
        let declared = {
            let parsed = self.parsed.borrow();
            let sheets: Vec<&nativeboat_css::Stylesheet> = parsed
                .as_ref()
                .map(|parsed| parsed.sheets.iter().map(Rc::as_ref).collect())
                .unwrap_or_default();
            nativeboat_css::page::page_margins(&sheets, &print.media)
        };
        let margins: [f32; 4] =
            core::array::from_fn(|side| margins[side].or(declared[side]).unwrap_or(DEFAULT_MARGIN));
        let content = (
            (paper.0 - margins[1] - margins[3]).max(1.0),
            (paper.1 - margins[0] - margins[2]).max(1.0),
        );
        print.media.width = f64::from(content.0);
        print.media.height = f64::from(content.1);
        self.set_environment(print);
        let rendered = self.rendered();
        self.set_environment(screen);
        let mut tree = rendered?.scene.tree.clone();
        // The caret and selection are the screen's.
        tree.root
            .items
            .retain(|item| !matches!(item, DisplayItem::Caret { .. }));
        let breaks = page_breaks(&tree, content.1);
        Some(PrintLayout {
            tree,
            breaks,
            content,
            margins,
        })
    }

    /// The current scene for native painting, retaining the DOM's node ids.
    pub fn scene(&self) -> Option<Rc<Scene>> {
        self.rendered().map(|rendered| rendered.scene.clone())
    }

    /// The document the page shows, once attached.
    /// The page for diagnosing rendering: each element with its border box
    /// and the computed styles that most affect what it shows, and each
    /// text node's text, indented by depth.
    pub fn describe(&self) -> String {
        const PROPERTIES: &[&str] = &[
            "display",
            "position",
            "visibility",
            "opacity",
            "transform",
            "font-family",
            "font-size",
            "font-weight",
            "color",
            "background-color",
            "width",
            "height",
        ];
        let (Some(document), Some(rendered), Some(styled)) =
            (self.document(), self.rendered(), self.styled())
        else {
            return String::new();
        };
        let dom = document.borrow();
        let mut text = String::new();
        let mut stack = alloc::vec![(Dom::DOCUMENT, 0usize)];
        while let Some((node, depth)) = stack.pop() {
            let indent = "  ".repeat(depth);
            match dom.local_name(node) {
                Some(name) => {
                    let _ = fmt::Write::write_fmt(&mut text, format_args!("{indent}<{name}"));
                    for (attribute, value) in dom.attributes(node) {
                        if matches!(
                            attribute.as_str(),
                            "id" | "class"
                                | "style"
                                | "src"
                                | "href"
                                | "viewBox"
                                | "transform"
                                | "width"
                                | "height"
                                | "x"
                                | "y"
                        ) {
                            let value: String = value.chars().take(80).collect();
                            let _ = fmt::Write::write_fmt(
                                &mut text,
                                format_args!(" {attribute}=\"{value}\""),
                            );
                        }
                    }
                    text.push('>');
                    if let Some(area) = rendered.scene.client_rect(node) {
                        let _ = fmt::Write::write_fmt(
                            &mut text,
                            format_args!(
                                " @{:.0},{:.0} {:.0}x{:.0}",
                                area.x, area.y, area.width, area.height
                            ),
                        );
                    }
                    if let Some(style) = styled.styles.get(node) {
                        for property in PROPERTIES {
                            if let Some(id) = PropertyId::from_name(property) {
                                let _ = fmt::Write::write_fmt(
                                    &mut text,
                                    format_args!(" {property}:{}", style.value_of(id)),
                                );
                            }
                        }
                    }
                    text.push('\n');
                }
                None => {
                    if let Some(data) = dom.data(node)
                        && !data.trim().is_empty()
                    {
                        let data: String = data.chars().take(120).collect();
                        let _ = fmt::Write::write_fmt(
                            &mut text,
                            format_args!("{indent}\"{}\"\n", data.escape_debug()),
                        );
                    }
                }
            }
            if dom
                .local_name(node)
                .is_some_and(|name| matches!(name, "style" | "script"))
            {
                continue;
            }
            let children = dom.children(node);
            stack.extend(children.into_iter().rev().map(|child| (child, depth + 1)));
        }
        // Then what each surface paints, in order, to compare painting with
        // the boxes: fills, clips, groups and where children show.
        fn describe_surface(text: &mut String, surface: &nativeboat_ir::Surface, depth: usize) {
            let indent = "  ".repeat(depth);
            let rect = surface.rect;
            let _ = fmt::Write::write_fmt(
                text,
                format_args!(
                    "{indent}surface {} @{:.0},{:.0} {:.0}x{:.0} content {:.0}x{:.0}{}\n",
                    surface.id,
                    rect.x,
                    rect.y,
                    rect.width,
                    rect.height,
                    surface.content_width,
                    surface.content_height,
                    if surface.scrollbars.is_some() {
                        " scrollbars"
                    } else {
                        ""
                    }
                ),
            );
            for item in &surface.items {
                let line = match item {
                    nativeboat_ir::DisplayItem::Text { text, rect, .. } => {
                        let text: String = text.chars().take(24).collect();
                        alloc::format!("text @{:.0},{:.0} {:?}", rect.x, rect.y, text)
                    }
                    nativeboat_ir::DisplayItem::Image { rect, .. } => {
                        alloc::format!(
                            "image @{:.0},{:.0} {:.0}x{:.0}",
                            rect.x,
                            rect.y,
                            rect.width,
                            rect.height
                        )
                    }
                    nativeboat_ir::DisplayItem::Path { .. } => String::from("path"),
                    other => {
                        let mut line = alloc::format!("{other:?}");
                        line.truncate(160);
                        line
                    }
                };
                let _ = fmt::Write::write_fmt(text, format_args!("{indent}  {line}\n"));
            }
            for child in &surface.children {
                describe_surface(text, child, depth + 1);
            }
        }
        describe_surface(&mut text, &rendered.scene.tree.root, 0);
        text
    }

    pub fn document(&self) -> Option<Rc<RefCell<Dom>>> {
        self.document.borrow().clone()
    }

    /// The mouse cursor over an element.
    pub fn cursor(&self, node: NodeId) -> Cursor {
        self.rendered()
            .and_then(|rendered| rendered.styles.get(node).map(|style| style.cursor))
            .unwrap_or(Cursor::Auto)
    }

    /// Starts resizing a box when the pointer is pressed on its resize grip.
    pub fn start_resize(&self, x: f32, y: f32) -> Option<Resizing> {
        let rendered = self.rendered()?;
        let document = self.document.borrow().clone()?;
        let dom = document.borrow();
        Resizing::start(&rendered.scene, &dom, x, y)
    }

    /// Presses the scrollbar of a scroll container painted in place at a
    /// point: `None` when none is there, else the thumb drag it starts.
    pub fn press_scrollbar(&self, x: f32, y: f32) -> Option<Option<scrollbars::Dragging>> {
        let rendered = self.rendered()?;
        let document = self.document.borrow().clone()?;
        let mut dom = document.borrow_mut();
        scrollbars::press(&rendered.scene, &mut dom, x, y)
    }

    /// Scrolls the container of a thumb drag so that the thumb follows the
    /// pointer.
    pub fn drag_scrollbar(&self, dragging: &scrollbars::Dragging, x: f32, y: f32) {
        let (Some(rendered), Some(document)) = (self.rendered(), self.document.borrow().clone())
        else {
            return;
        };
        dragging.drag(&rendered.scene, &mut document.borrow_mut(), x, y);
    }

    /// Shows the scrollbar part under the pointer, or none when it left the
    /// window; returns whether it is over one.
    pub fn hover_scrollbar(&self, point: Option<(f32, f32)>) -> bool {
        let (Some(rendered), Some(document)) = (self.rendered(), self.document.borrow().clone())
        else {
            return false;
        };
        let mut dom = document.borrow_mut();
        match point {
            Some((x, y)) => scrollbars::hover(&rendered.scene, &mut dom, x, y),
            None => {
                let (_, pressed) = dom.scrollbar_parts();
                dom.set_scrollbar_parts((None, pressed));
                false
            }
        }
    }

    /// Scrolls again for the arrow button or track held down on a scrollbar
    /// painted in place, with the pointer at a point; returns whether one is
    /// held.
    pub fn repeat_scrollbar(&self, x: f32, y: f32) -> bool {
        let (Some(rendered), Some(document)) = (self.rendered(), self.document.borrow().clone())
        else {
            return false;
        };
        scrollbars::repeat(&rendered.scene, &mut document.borrow_mut(), x, y)
    }

    /// Ends a press of a scrollbar part.
    pub fn release_scrollbar(&self) {
        if let Some(document) = self.document.borrow().clone() {
            scrollbars::release(&mut document.borrow_mut());
        }
    }

    /// Scrolls the innermost scroll container painted in place around an
    /// element by a wheel turn; returns whether one scrolled.
    pub fn wheel_in_place(&self, target: NodeId, delta: (f32, f32)) -> bool {
        let (Some(rendered), Some(document)) = (self.rendered(), self.document.borrow().clone())
        else {
            return false;
        };
        scrollbars::wheel(&rendered.scene, &mut document.borrow_mut(), target, delta)
    }

    /// Whether an element is a native control kept out of sight, which
    /// takes the pointer where its image shows.
    pub fn takes_pointer(&self, node: NodeId) -> bool {
        self.rendered()
            .is_some_and(|rendered| rendered.scene.takes_pointer(node))
    }

    /// Sizes the box of a resize drag to follow the pointer.
    pub fn resize(&self, resizing: &Resizing, x: f32, y: f32) {
        let (Some(rendered), Some(document)) = (self.rendered(), self.document.borrow().clone())
        else {
            return;
        };
        let mut dom = document.borrow_mut();
        resizing.drag(
            &rendered.scene,
            &rendered.styles,
            &rendered.layout,
            &mut dom,
            x,
            y,
        );
    }

    /// The cursor over a resize grip at a point, if one is there.
    pub fn resize_cursor(&self, x: f32, y: f32) -> Option<Cursor> {
        let rendered = self.rendered()?;
        let document = self.document.borrow().clone()?;
        let dom = document.borrow();
        resize::resizer_at(&rendered.scene, &dom, x, y)
            .map(|node| resize::cursor(&rendered.styles, node))
    }

    /// Whether page content painted above a native control takes the
    /// pointer at a point: the resize grip over a text area does. It reads
    /// only what was last rendered, as the system asks while the page's
    /// scripts may be running.
    pub fn above_control(&self, control: NodeId, x: f32, y: f32) -> bool {
        let (Ok(document), Ok(rendered)) = (self.document.try_borrow(), self.rendered.try_borrow())
        else {
            return false;
        };
        let (Some(document), Some(rendered)) = (document.as_ref(), rendered.as_ref()) else {
            return false;
        };
        let Ok(dom) = document.try_borrow() else {
            return false;
        };
        resize::resizer_at(&rendered.scene, &dom, x, y) == Some(control)
    }

    /// Whether a point of the page drags its frameless window: the nearest
    /// element at the point, or around it, with an `app-region` decides.
    pub fn drags(&self, x: f32, y: f32) -> bool {
        let (Some(document), Some(rendered)) = (self.document.borrow().clone(), self.rendered())
        else {
            return false;
        };
        let dom = document.borrow();
        core::iter::successors(rendered.scene.hit_test(x, y), |node| dom.parent(*node))
            .filter_map(|node| rendered.styles.get(node))
            .map(|style| style.app_region)
            .find(|region| *region != AppRegion::None)
            == Some(AppRegion::Drag)
    }

    fn text<T>(&self, question: impl FnOnce(&Scene, &TextQuery<'_>) -> T) -> Option<T> {
        let document = self.document.borrow().clone()?;
        let rendered = self.rendered()?;
        let dom = document.borrow();
        let query = TextQuery {
            dom: &dom,
            styles: &rendered.styles,
            layout: &rendered.layout,
            measurer: self.measurer.as_ref(),
        };
        Some(question(&rendered.scene, &query))
    }
}

impl DocumentPage {
    fn scroll_native(&self, node: NodeId, block: ScrollAlign, inline: ScrollAlign) {
        // Pages render once they have a document.
        let (Some(document), Some(rendered)) = (self.document.borrow().clone(), self.rendered())
        else {
            return;
        };
        let (rect, containers) = {
            let Some(rect) = rendered.scene.client_rect(node) else {
                return;
            };
            // `scroll-margin` keeps room around the element.
            let margin = rendered
                .styles
                .get(node)
                .map_or(Edges::all(Px(0.0)), |style| style.scroll_margin);
            let rect = nativeboat_ir::Rect::new(
                rect.x - margin.left.0,
                rect.y - margin.top.0,
                rect.width + margin.left.0 + margin.right.0,
                rect.height + margin.top.0 + margin.bottom.0,
            );
            let dom = document.borrow();
            let containers: Vec<(NodeId, nativeboat_ir::Rect, (f32, f32))> =
                core::iter::successors(dom.parent_element(node), |element| {
                    dom.parent_element(*element)
                })
                .filter_map(|element| {
                    let style = rendered.styles.get(element)?;
                    let scrolls = |overflow: Overflow| overflow.is_scroll_container();
                    let root = Some(element) == dom.document_element();
                    let (visible, content) = if root {
                        let view = self.layout_viewport();
                        (
                            nativeboat_ir::Rect::new(0.0, 0.0, view.0, view.1),
                            (rendered.layout.width, rendered.layout.height),
                        )
                    } else if scrolls(style.overflow_x) || scrolls(style.overflow_y) {
                        let geometry = rendered.layout.boxes.get(&element)?;
                        let border = rendered.scene.client_rect(element)?;
                        let padding = geometry.padding_box();
                        (
                            nativeboat_ir::Rect::new(
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
                    let visible = nativeboat_ir::Rect::new(
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
        let mut dom = document.borrow_mut();
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

/// The user agent's style sheet, then the page's.
/// Prints `page`, shown in the window with handle `window`, as `request`
/// asks: into a PDF through Windows' PDF printer, on a printer the user
/// picks in the print dialog, or on a named or the default printer.
pub fn print_page(
    page: &DocumentPage,
    window: usize,
    request: &nativeboat_native::app_host::PrintRequest,
) -> Result<nativeboat_native::app_host::PrintOutcome, String> {
    use nativeboat_native::app_host::{PrintOutcome, PrintTarget};
    use nativeboat_win32::{Paper, Printer};
    let printer = match &request.target {
        PrintTarget::Pdf => Printer::Named("Microsoft Print to PDF"),
        PrintTarget::Dialog => Printer::Dialog(window),
        PrintTarget::Printer(name) if name.is_empty() => Printer::Default,
        PrintTarget::Printer(name) => Printer::Named(name),
    };
    let paper = Paper {
        size: request.paper,
        landscape: request.landscape,
    };
    let Some(job) =
        nativeboat_win32::open_printer(printer, paper).map_err(|error| error.message)?
    else {
        return Ok(PrintOutcome::Cancelled);
    };
    let mut layout = page
        .print_layout(job.paper, request.margins)
        .ok_or_else(|| String::from("the page has no document to print"))?;
    if !request.background {
        without_backgrounds(&mut layout.tree.root);
    }
    let output =
        (request.target == PrintTarget::Pdf).then(|| nativeboat_win32::temporary_output("pdf"));
    job.print(
        &request.name,
        output.as_deref(),
        &layout.tree,
        &layout.breaks,
        layout.content,
        layout.margins,
    )
    .map_err(|error| error.message)?;
    match output {
        Some(path) => {
            let bytes = nativeboat_win32::take_output(&path)
                .ok_or_else(|| String::from("the PDF printer wrote no document"))?;
            Ok(PrintOutcome::Pdf(bytes))
        }
        None => Ok(PrintOutcome::Printed),
    }
}

/// Removes the background colors and gradients of a surface and those it
/// holds, as printing without backgrounds leaves them out.
fn without_backgrounds(surface: &mut nativeboat_ir::Surface) {
    surface.items.retain(|item| {
        !matches!(
            item,
            DisplayItem::Fill { .. } | DisplayItem::Gradient { .. }
        )
    });
    for child in &mut surface.children {
        without_backgrounds(child);
    }
}

/// A page laid out for printing.
#[derive(Debug, Clone)]
pub struct PrintLayout {
    /// The whole document; printing shows it from each break.
    pub tree: NativeTree,
    /// Where each page begins in the document, in CSS pixels.
    pub breaks: Vec<f32>,
    /// The printable size of a page in CSS pixels.
    pub content: (f32, f32),
    /// The margins of a page in CSS pixels: top, right, bottom, left.
    pub margins: [f32; 4],
}

/// Where the pages of a document `height` CSS pixels tall begin: every
/// `height`, or earlier where that would cut through a line of text or an
/// image shorter than a page.
fn page_breaks(tree: &NativeTree, height: f32) -> Vec<f32> {
    let total = tree.root.content_height.max(tree.root.rect.height);
    let spans: Vec<(f32, f32)> = tree
        .root
        .items
        .iter()
        .filter_map(|item| match item {
            DisplayItem::Text { rect, .. } | DisplayItem::Image { rect, .. } => {
                Some((rect.y, rect.y + rect.height))
            }
            _ => None,
        })
        .filter(|(top, bottom)| bottom - top <= height)
        .collect();
    let mut breaks = alloc::vec![0.0];
    let mut start = 0.0;
    while start + height < total - 0.5 {
        let end = start + height;
        let cut = spans
            .iter()
            .filter(|(top, bottom)| *top < end && *bottom > end && *top > start + 0.5)
            .map(|(top, _)| *top)
            .fold(end, f32::min);
        breaks.push(cut);
        start = cut;
    }
    breaks
}

fn sheet_refs(parsed: &[Rc<nativeboat_css::Stylesheet>]) -> Vec<SheetRef<'_>> {
    let mut sheets = Vec::with_capacity(parsed.len() + 1);
    sheets.push(SheetRef {
        sheet: user_agent_stylesheet(),
        origin: Origin::UserAgent,
    });
    sheets.extend(parsed.iter().map(|sheet| SheetRef {
        sheet,
        origin: Origin::Author,
    }));
    sheets
}

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

fn rect(value: nativeboat_ir::Rect) -> Rect {
    Rect {
        x: value.x,
        y: value.y,
        width: value.width,
        height: value.height,
    }
}

macro_rules! platform {
    ($(fn $name:ident(&self $(, $arg:ident : $ty:ty)*) $(-> $result:ty)?; )*) => {
        $(fn $name(&self $(, $arg: $ty)*) $(-> $result)? { self.platform.$name($($arg),*) })*
    };
}

/// The platform bitmaps of canvases are numbered across the process.
static NEXT_CANVAS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

/// A font as JSON, as scripts hand it back to draw and measure text.
fn font_json(font: &Font) -> Json {
    let number = |value: f64| Json::Number(value);
    let mut members = alloc::vec![
        (
            String::from("families"),
            Json::Array(
                font.families
                    .iter()
                    .map(|family| Json::String(family.clone()))
                    .collect(),
            ),
        ),
        (String::from("size"), number(f64::from(font.size))),
        (String::from("weight"), number(f64::from(font.weight))),
        (String::from("italic"), Json::Bool(font.italic)),
    ];
    if !font.features.is_empty() {
        let features = font
            .features
            .iter()
            .map(|(tag, value)| {
                Json::Array(alloc::vec![
                    number(f64::from(*tag)),
                    number(f64::from(*value))
                ])
            })
            .collect();
        members.push((String::from("features"), Json::Array(features)));
    }
    Json::Object(members)
}

/// Path segments as JSON, externally tagged as `serde` writes them.
fn path_json(segments: &[PathSegment]) -> Json {
    let tagged = |name: &str, values: &[f32]| {
        Json::Object(alloc::vec![(
            String::from(name),
            Json::Array(
                values
                    .iter()
                    .map(|value| Json::Number(f64::from(*value)))
                    .collect()
            ),
        )])
    };
    Json::Array(
        segments
            .iter()
            .map(|segment| match segment {
                PathSegment::Move(x, y) => tagged("Move", &[*x, *y]),
                PathSegment::Line(x, y) => tagged("Line", &[*x, *y]),
                PathSegment::Cubic(a, b, c, d, e, f) => tagged("Cubic", &[*a, *b, *c, *d, *e, *f]),
                PathSegment::Close => Json::String(String::from("Close")),
            })
            .collect(),
    )
}

/// Style sheet text under a media query, unless it applies to all media.
fn under_media(text: String, media: Option<&str>) -> String {
    match media.map(str::trim) {
        Some(media) if !media.is_empty() && !media.eq_ignore_ascii_case("all") => {
            alloc::format!("@media {media} {{\n{text}\n}}")
        }
        _ => text,
    }
}

impl Page for DocumentPage {
    fn focus(&self, node: Option<NodeId>) {
        self.focus_request.set(Some(node));
        self.platform.focus(node);
    }
    fn image_size(&self, url: &str) -> Option<(f32, f32)> {
        self.image_size_of(url)
    }
    fn add_image(&self, url: &str, bytes: &[u8]) -> bool {
        if nativeboat_win32::add_image(url, bytes).is_none() {
            return false;
        }
        self.image_added(url);
        true
    }
    fn media_source(&self, url: &str) -> Option<MediaSource> {
        if assets::is_network(url) {
            return Some(MediaSource::Url(url.trim().into()));
        }
        if url
            .trim_start()
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
        {
            return assets::fetch(&self.platform.url(), url).map(MediaSource::Bytes);
        }
        assets::file_path(&self.platform.url(), url).map(MediaSource::Path)
    }
    fn add_frame(&self, key: &str, width: u32, height: u32, pixels: &[u8]) -> bool {
        if width == 0
            || height == 0
            || pixels.len() != width as usize * height as usize * 4
            || nativeboat_win32::image_from_pixels(key, (width, height), pixels).is_none()
        {
            return false;
        }
        self.image_added(key);
        true
    }
    fn image_pixels(&self, url: &str) -> Option<(u32, u32, Vec<u8>)> {
        self.image_size_of(url)?;
        nativeboat_win32::image_pixels(url)
    }
    fn take_wanted_images(&self) -> Vec<String> {
        core::mem::take(&mut *self.wanted.borrow_mut())
    }
    fn scroll_into_view(&self, node: NodeId, block: ScrollAlign, inline: ScrollAlign) {
        self.scroll_native(node, block, inline);
    }
    fn rect(&self, node: NodeId) -> Option<Rect> {
        let rendered = self.rendered()?;
        if let Some(found) = rendered.scene.client_rect(node) {
            return Some(rect(found));
        }
        // An inline element with nothing to show on a line of nothing else
        // is an empty rectangle where its containing block's content
        // starts, as Chromium reports it.
        if rendered.styles.get(node)?.display != nativeboat_css::types::Display::Inline {
            return None;
        }
        let document = self.document.borrow().clone()?;
        let dom = document.borrow();
        let mut ancestor = dom.parent_element(node);
        while let Some(parent) = ancestor {
            if let (Some(border), Some(geometry)) = (
                rendered.scene.client_rect(parent),
                rendered.layout.boxes.get(&parent),
            ) {
                let content = geometry.content_box();
                return Some(rect(nativeboat_ir::Rect::new(
                    border.x + content.x - geometry.rect.x,
                    border.y + content.y - geometry.rect.y,
                    0.0,
                    0.0,
                )));
            }
            ancestor = dom.parent_element(parent);
        }
        None
    }
    fn bbox(&self, node: NodeId) -> Option<Rect> {
        let document = self.document.borrow().clone()?;
        let styled = self.styled()?;
        let dom = document.borrow();
        Some(rect(nativeboat_layout::svg::bbox(
            &dom,
            &styled.styles,
            node,
            Some(self.measurer.as_ref()),
        )))
    }
    fn client(&self, node: NodeId) -> Option<Rect> {
        let document = self.document.borrow().clone()?;
        let rendered = self.rendered()?;
        let geometry = rendered.layout.boxes.get(&node)?;
        // The root element's client area is the layout viewport (CSSOM
        // View): the viewport less the window's scrollbars.
        let (width, height) = if document.borrow().document_element() == Some(node) {
            self.layout_viewport()
        } else {
            let client = geometry.client_box();
            (client.width, client.height)
        };
        // Whole pixels, as Chromium reports them, around exact borders; none
        // of these is negative.
        let round = |value: f32| (value + 0.5) as i32 as f32;
        Some(Rect {
            x: round(geometry.border[3]),
            y: round(geometry.border[0]),
            width: round(width),
            height: round(height),
        })
    }
    fn scroll_size(&self, node: NodeId) -> Option<(f32, f32)> {
        let document = self.document.borrow().clone()?;
        let rendered = self.rendered()?;
        if document.borrow().document_element() == Some(node) {
            let (width, height) = self.layout_viewport();
            return Some((
                rendered.layout.width.max(width),
                rendered.layout.height.max(height),
            ));
        }
        let geometry = rendered.layout.boxes.get(&node)?;
        Some((geometry.content_width, geometry.content_height))
    }
    fn computed_style(&self, node: NodeId) -> Vec<(String, String)> {
        self.presented()
            .and_then(|(_, styles)| styles.get(node).cloned())
            .map_or_else(Vec::new, |style| {
                PropertyId::ALL
                    .iter()
                    .map(|property| {
                        (
                            String::from(property.name()),
                            style.resolved_value_of(*property, None),
                        )
                    })
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
        let Some(style) = self
            .presented()
            .and_then(|(_, styles)| styles.get(node).cloned())
        else {
            return String::new();
        };
        // Only these resolve against layout, which they bring up to date.
        let used = ComputedStyle::resolves_against_layout(name)
            .then(|| self.used_box(node, &style))
            .flatten();
        style
            .resolved_value(name, used.as_ref())
            .unwrap_or_default()
    }
    fn viewport(&self) -> (f32, f32) {
        let media = self.environment.get().media;
        (media.width as f32, media.height as f32)
    }
    fn device_pixel_ratio(&self) -> f32 {
        self.environment.get().media.resolution as f32
    }
    fn element_from_point(&self, x: f32, y: f32) -> Option<NodeId> {
        self.rendered()?.scene.hit_test(x, y)
    }
    fn caret_from_point(&self, x: f32, y: f32) -> Option<(NodeId, u32)> {
        self.text(|scene, query| scene.caret_from_point(query, x, y))
            .flatten()
    }
    fn text_rects(&self, node: NodeId, start: u32, end: u32) -> Vec<Rect> {
        self.text(|scene, query| {
            scene
                .text_rects(query, node, start, end)
                .into_iter()
                .map(rect)
                .collect()
        })
        .unwrap_or_default()
    }
    fn misspelling(&self, node: NodeId, offset: u32) -> Option<(u32, u32)> {
        let spelling = self.spelling.borrow();
        let (_, ranges) = spelling.get(&node)?;
        ranges
            .iter()
            .copied()
            .find(|(start, end)| *start <= offset && offset <= *end)
    }
    fn match_media(&self, query: &str) -> bool {
        let tokens = nativeboat_css::tokenizer::tokenize(query)
            .into_iter()
            .map(|token| token.token)
            .collect::<Vec<_>>();
        nativeboat_css::media::parse_media_query_list(&tokens)
            .matches(&self.environment.get().media)
    }
    fn sheet_rules(&self, owner: NodeId, rules: &str) {
        self.inserted
            .borrow_mut()
            .insert(owner, String::from(rules));
        self.sheet_version
            .set(self.sheet_version.get().wrapping_add(1));
        self.view_version
            .set(self.view_version.get().wrapping_add(1));
    }
    fn parse_color(&self, text: &str) -> Option<[f32; 4]> {
        let values = nativeboat_css::values::without_whitespace(
            &nativeboat_css::values::parse_component_values(text),
        );
        let [value] = values.as_slice() else {
            return None;
        };
        let color = match nativeboat_css::color::parse_color(value, &self.environment.get().system)?
        {
            nativeboat_css::color::SpecifiedColor::Color(color) => color,
            nativeboat_css::color::SpecifiedColor::CurrentColor => nativeboat_css::Color::BLACK,
        };
        Some([
            f32::from(color.r),
            f32::from(color.g),
            f32::from(color.b),
            color.a,
        ])
    }
    // Text controls are the window's; the clipboard is the system's.
    fn text_selection(&self, node: NodeId) -> Option<(u32, u32)> {
        if let Some(selection) = self.selections.borrow().get(&node) {
            return Some(*selection);
        }
        nativeboat_win32::text_selection(self.window.get()?, node.index())
            .map(|(start, end)| (start as u32, end as u32))
    }
    fn select_text(&self, node: NodeId, start: u32, end: u32) {
        self.selections.borrow_mut().insert(node, (start, end));
    }
    fn read_clipboard(&self) -> Option<String> {
        Some(nativeboat_win32::read_text())
    }
    fn write_clipboard(&self, text: &str) -> bool {
        self.write_clipboard_contents(Some(text), None)
    }
    fn read_clipboard_contents(&self) -> Option<Vec<(String, String)>> {
        nativeboat_win32::read_clipboard_contents().map(|contents| {
            contents
                .into_iter()
                .map(|(kind, data)| (String::from(kind), data))
                .collect()
        })
    }
    fn write_clipboard_contents(&self, text: Option<&str>, html: Option<&str>) -> bool {
        nativeboat_win32::write_clipboard(text, html).is_ok()
    }
    fn load_storage(&self) -> Option<String> {
        let path = self.storage.borrow().clone()?;
        String::from_utf8(nativeboat_native::platform::read_file(&path)?).ok()
    }
    fn save_storage(&self, data: &str) {
        if let Some(path) = self.storage.borrow().as_deref() {
            nativeboat_native::platform::write_file(path, data.as_bytes());
        }
    }
    // A composition the input method holds ends in the window's control.
    fn complete_composition(&self) -> Option<String> {
        nativeboat_win32::complete_composition(self.window.get()?)
    }
    // Canvases draw into platform bitmaps, which the scene shows.
    fn canvas_draw(&self, node: NodeId, width: u32, height: u32, commands: &str) {
        let Some(mut commands) = json::from_str::<Vec<CanvasCommand>>(commands) else {
            return;
        };
        // Images are known to the platform by their URLs once loaded, and
        // other canvases by their bitmaps.
        for command in &mut commands {
            if let CanvasCommand::Image { source, .. } = command {
                match source {
                    CanvasSource::Url(url) => {
                        self.image_size_of(url);
                    }
                    CanvasSource::Canvas(other) => {
                        *other = u32::try_from(*other)
                            .ok()
                            .and_then(|other| self.canvases.borrow().get(&NodeId(other)).copied())
                            .unwrap_or(0);
                    }
                }
            }
        }
        let key = *self
            .canvases
            .borrow_mut()
            .entry(node)
            .or_insert_with(|| NEXT_CANVAS.fetch_add(1, core::sync::atomic::Ordering::Relaxed));
        nativeboat_win32::canvas_draw(key, width, height, &commands);
        if let Some(document) = self.document.borrow().clone() {
            document.borrow_mut().canvas_drawn(node, key);
        }
    }
    fn canvas_read(&self, node: NodeId, (x, y, width, height): (i32, i32, u32, u32)) -> Vec<u8> {
        match self.canvases.borrow().get(&node) {
            Some(key) => nativeboat_win32::canvas_read(*key, x, y, width, height),
            None => alloc::vec![0; width as usize * height as usize * 4],
        }
    }
    fn canvas_write(
        &self,
        node: NodeId,
        (x, y, width, height): (i32, i32, u32, u32),
        pixels: &[u8],
    ) {
        let key = self.canvases.borrow().get(&node).copied();
        if let Some(key) = key {
            nativeboat_win32::canvas_write(key, x, y, width, height, pixels);
            if let Some(document) = self.document.borrow().clone() {
                document.borrow_mut().canvas_drawn(node, key);
            }
        }
    }
    fn canvas_url(&self, node: NodeId, jpeg: bool) -> String {
        let key = self.canvases.borrow().get(&node).copied();
        match key.and_then(|key| nativeboat_win32::canvas_encode(key, jpeg)) {
            Some(bytes) => alloc::format!(
                "data:image/{};base64,{}",
                if jpeg { "jpeg" } else { "png" },
                nativeboat_native::base64_encode(&bytes, false)
            ),
            None => String::from("data:,"),
        }
    }
    fn parse_font(&self, text: &str) -> Option<String> {
        let longhands = nativeboat_css::shorthand::expand(
            "font",
            &nativeboat_css::values::parse_component_values(text),
        )?;
        let specified = nativeboat_css::computed::SpecifiedValues {
            values: longhands
                .into_iter()
                .map(|(id, value)| (id, nativeboat_css::computed::Specified::Value(value)))
                .collect(),
            ..Default::default()
        };
        let style =
            nativeboat_css::ComputedStyle::compute(&specified, None, &self.environment.get());
        Some(font_json(&nativeboat_layout::text::font_of(&style)).text())
    }
    fn measure_text(&self, font: &str, text: &str) -> [f32; 3] {
        let font = json::from_str::<Font>(font).unwrap_or_default();
        let (width, ascent, descent) = nativeboat_win32::measure_text(&font, text);
        [width, ascent, descent]
    }
    fn path_data(&self, data: &str) -> String {
        path_json(&nativeboat_layout::svg::path_data(data)).text()
    }
    fn open_select(&self, node: NodeId) {
        self.select_request.set(Some(node));
    }
    fn drag_out(&self, items: &[(String, String)], allowed: &str) -> String {
        String::from(nativeboat_win32::drag_out(items, allowed))
    }
    platform! {
        fn url(&self) -> String;
    }
}

#[cfg(test)]
mod tests;

/// Whether an editable text node is spell checked: as its nearest
/// `spellcheck` attribute says, else, as editable text, checked.
fn spellchecked(dom: &Dom, node: NodeId) -> bool {
    dom.parent_element(node)
        .is_none_or(|element| spellchecked_element(dom, element))
}

/// Whether an editable element is spell checked.
fn spellchecked_element(dom: &Dom, element: NodeId) -> bool {
    core::iter::successors(Some(element), |element| dom.parent_element(*element))
        .find_map(|element| {
            let value = dom.get_attribute(element, "spellcheck")?;
            Some(!value.trim().eq_ignore_ascii_case("false"))
        })
        .unwrap_or(true)
}
