//! Paints display lists with GDI+ (shapes and images) and GDI (text).

use crate::fonts;
#[allow(unused_imports)]
use crate::prelude::*;
use core::cell::RefCell;
use core::ptr::null_mut;
use nativeboat_css::Color;
use nativeboat_css::types::{BorderStyle, FillRule, Filter, LineCap, LineJoin};
use nativeboat_ir::{Corner, DisplayItem, PathSegment, Radii, Rect, Stroke};
use nativeboat_layout::svg::multiply;
use windows_sys::Win32::Foundation::{COLORREF, RECT};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::UI::Shell::SHCreateMemStream;
use windows_sys::core::IUnknown_Vtbl;

mod bounds;
mod corners;
mod filter;
mod opacity;
mod tree;

pub use tree::paint_tree;
pub(crate) use tree::surface as paint_surface;

#[derive(Debug, Clone, Copy)]
struct Image {
    handle: *mut GpImage,
    width: u32,
    height: u32,
}

/// A vector image: its natural size and the items that draw it at that size.
type Vector = Rc<((f32, f32), Vec<DisplayItem>)>;

type ShadowKey = [u32; 17];

struct ShadowImage {
    handle: *mut GpImage,
    bytes: usize,
}

impl Drop for ShadowImage {
    fn drop(&mut self) {
        // SAFETY: this image is owned until the last cached or drawing reference drops.
        unsafe {
            GdipDisposeImage(self.handle);
        }
    }
}

struct ShadowCache {
    entries: VecDeque<(ShadowKey, Rc<ShadowImage>)>,
    bytes: usize,
    limit: usize,
}

impl ShadowCache {
    fn new(limit: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            bytes: 0,
            limit,
        }
    }

    fn get(&mut self, key: &ShadowKey) -> Option<Rc<ShadowImage>> {
        let index = self.entries.iter().position(|entry| &entry.0 == key)?;
        let entry = self.entries.remove(index).unwrap();
        let image = entry.1.clone();
        self.entries.push_back(entry);
        Some(image)
    }

    fn insert(&mut self, key: ShadowKey, image: Rc<ShadowImage>) {
        if image.bytes > self.limit {
            return;
        }
        while self.bytes + image.bytes > self.limit || self.entries.len() >= 32 {
            let entry = self.entries.pop_front().unwrap();
            self.bytes -= entry.1.bytes;
        }
        self.bytes += image.bytes;
        self.entries.push_back((key, image));
    }
}

local! {
    /// The images of controls kept out of sight, by id, while their
    /// surface paints: each paints at its control's marker, in its box.
    static PROXIES: RefCell<Vec<(u64, Image, Rect)>> = const { RefCell::new(Vec::new()) };
    static IMAGES: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
    static VECTORS: RefCell<HashMap<String, Vector>> = RefCell::new(HashMap::new());
    static SHADOWS: RefCell<ShadowCache> = RefCell::new(ShadowCache::new(8*1024*1024));
    /// Skia's coverage corrections for grayscale text, by luminance level.
    static MASK_GAMMA: RefCell<[Option<[u8; 256]>; 8]> = const { RefCell::new([None; 8]) };
}

/// Makes a vector image, such as an SVG file, available to display lists
/// under `url`; `items` draw it at its natural `size`.
pub fn add_vector_image(url: &str, size: (f32, f32), items: Vec<DisplayItem>) {
    VECTORS.with_borrow_mut(|vectors| vectors.insert(url.to_owned(), Rc::new((size, items))));
}

/// An item of a vector image drawn into `rect` instead of its natural size.
fn placed(item: &DisplayItem, rect: Rect, size: (f32, f32)) -> DisplayItem {
    let (scale_x, scale_y) = (
        rect.width / size.0.max(f32::EPSILON),
        rect.height / size.1.max(f32::EPSILON),
    );
    match item {
        DisplayItem::Path {
            segments,
            transform,
            fill,
            fill_rule,
            stroke,
        } => DisplayItem::Path {
            segments: segments.clone(),
            transform: multiply([scale_x, 0.0, 0.0, scale_y, rect.x, rect.y], *transform),
            fill: *fill,
            fill_rule: *fill_rule,
            stroke: stroke.clone(),
        },
        DisplayItem::PushClipPath {
            segments,
            fill_rule,
        } => DisplayItem::PushClipPath {
            segments: nativeboat_layout::svg::transform_segments(
                [scale_x, 0.0, 0.0, scale_y, rect.x, rect.y],
                segments,
            ),
            fill_rule: *fill_rule,
        },
        DisplayItem::PushClip { rect: clip, radii } => DisplayItem::PushClip {
            rect: Rect::new(
                rect.x + clip.x * scale_x,
                rect.y + clip.y * scale_y,
                clip.width * scale_x,
                clip.height * scale_y,
            ),
            radii: *radii,
        },
        other => other.clone(),
    }
}

/// Decodes an image (PNG, JPEG, GIF, BMP, ICO, TIFF or WebP, and formats a
/// codec installed on the system reads, or an SVG file, which becomes a
/// vector image) and makes it available to display lists under `url`;
/// returns its natural size.
pub fn add_image(url: &str, bytes: &[u8]) -> Option<(f32, f32)> {
    if let Some(source) = svg_source(bytes) {
        let (size, items) = nativeboat_layout::svg::image(source)?;
        if let Some(old) = IMAGES.with_borrow_mut(|images| images.remove(url)) {
            // SAFETY: the replaced image is no longer referenced.
            unsafe { GdipDisposeImage(old.handle) };
        }
        add_vector_image(url, size, items);
        return Some(size);
    }
    crate::init().ok()?;
    VECTORS.with_borrow_mut(|vectors| vectors.remove(url));
    // WebP decodes here, as Chromium decodes it, with or without a codec.
    if crate::webp::is_webp(bytes) {
        let (size, pixels) = crate::webp::decode(bytes)?;
        return crate::image_from_pixels(url, size, &pixels);
    }
    let mut bitmap: *mut GpBitmap = null_mut();
    // SAFETY: `stream` is a live IStream; GDI+ keeps its own reference.
    let status = with_stream(bytes, |stream| unsafe {
        GdipCreateBitmapFromStream(stream, &mut bitmap)
    })?;
    if status == Ok {
        return Some(insert_image(url, bitmap as *mut GpImage));
    }
    let (size, pixels) = with_stream(bytes, crate::wic::decode)??;
    crate::image_from_pixels(url, size, &pixels)
}

/// The text of an SVG file: UTF-8 whose root element, after any byte order
/// mark, XML declaration, comments and doctype, is `<svg>`.
fn svg_source(bytes: &[u8]) -> Option<&str> {
    let text = core::str::from_utf8(bytes).ok()?;
    let mut rest = text.trim_start_matches('\u{feff}');
    loop {
        rest = rest.trim_start();
        let end = if rest.starts_with("<?") {
            rest.find("?>")? + 2
        } else if rest.starts_with("<!--") {
            rest.find("-->")? + 3
        } else if rest.starts_with("<!") {
            rest.find('>')? + 1
        } else {
            break;
        };
        rest = &rest[end..];
    }
    let after = rest.strip_prefix("<svg")?;
    after
        .starts_with(|next: char| next.is_ascii_whitespace() || next == '>' || next == '/')
        .then_some(text)
}

/// Runs `read` on a stream over a copy of `bytes`.
fn with_stream<T>(bytes: &[u8], read: impl FnOnce(*mut core::ffi::c_void) -> T) -> Option<T> {
    // SAFETY: the bytes are copied into the new stream.
    let stream = unsafe { SHCreateMemStream(bytes.as_ptr(), bytes.len() as u32) };
    if stream.is_null() {
        return None;
    }
    let result = read(stream);
    // SAFETY: a COM object starts with its vtable; this drops our reference.
    unsafe { ((**(stream as *mut *const IUnknown_Vtbl)).Release)(stream) };
    Some(result)
}

/// Makes a GDI+ image, which the cache then owns, available under `url`;
/// returns its size in pixels.
pub(crate) fn insert_image(url: &str, handle: *mut GpImage) -> (f32, f32) {
    let (mut width, mut height) = (0, 0);
    // SAFETY: `handle` is a live image.
    unsafe {
        GdipGetImageWidth(handle, &mut width);
        GdipGetImageHeight(handle, &mut height);
    }
    let image = Image {
        handle,
        width,
        height,
    };
    if let Some(old) = IMAGES.with_borrow_mut(|images| images.insert(url.to_owned(), image)) {
        // SAFETY: the replaced image is no longer referenced.
        unsafe { GdipDisposeImage(old.handle) };
    }
    crate::window::image_changed(url);
    (width as f32, height as f32)
}

/// The GDI+ image added with [`add_image`] under `url`, with its size.
pub(crate) fn image_handle(url: &str) -> Option<(*mut GpImage, u32, u32)> {
    IMAGES.with_borrow(|images| {
        images
            .get(url)
            .map(|image| (image.handle, image.width, image.height))
    })
}

/// An icon made from an image added with [`add_image`].
pub(crate) fn icon(url: &str) -> Option<windows_sys::Win32::UI::WindowsAndMessaging::HICON> {
    let image = IMAGES.with_borrow(|images| images.get(url).copied())?;
    let mut icon = null_mut();
    // SAFETY: images from `add_image` are bitmaps; the icon is new.
    let status = unsafe { GdipCreateHICONFromBitmap(image.handle as *mut GpBitmap, &mut icon) };
    (status == Ok).then_some(icon)
}

/// The natural size of an image added with [`add_image`] or
/// [`add_vector_image`].
pub fn image_size(url: &str) -> Option<(f32, f32)> {
    IMAGES
        .with_borrow(|images| {
            images
                .get(url)
                .map(|image| (image.width as f32, image.height as f32))
        })
        .or_else(|| VECTORS.with_borrow(|vectors| vectors.get(url).map(|vector| vector.0)))
}

/// Image attributes that draw images with an opacity, or null for opaque
/// drawing; the caller disposes them.
pub(crate) fn faded(opacity: f32) -> *mut GpImageAttributes {
    let mut attributes = null_mut();
    if opacity < 1.0 {
        let mut matrix = ColorMatrix::default();
        for index in [0, 6, 12, 24] {
            matrix.m[index] = 1.0;
        }
        matrix.m[18] = opacity;
        // SAFETY: the attributes are new and receive a copy of the matrix.
        unsafe {
            GdipCreateImageAttributes(&mut attributes);
            GdipSetImageAttributesColorMatrix(
                attributes,
                ColorAdjustTypeBitmap,
                1,
                &matrix,
                core::ptr::null(),
                ColorMatrixFlagsDefault,
            );
        }
    }
    attributes
}

/// A color as GDI+ ARGB with an extra opacity factor.
pub(crate) fn argb(color: Color, opacity: f32) -> u32 {
    let alpha = (color.a * opacity * 255.0).round().clamp(0.0, 255.0) as u32;
    alpha << 24 | u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
}

/// A color as a GDI COLORREF, ignoring alpha.
pub(crate) fn colorref(color: Color) -> COLORREF {
    u32::from(color.r) | u32::from(color.g) << 8 | u32::from(color.b) << 16
}

struct Painter {
    graphics: *mut GpGraphics,
    dc: HDC,
    /// The DIB section painted into.
    target: BITMAP,
    scale: f32,
    scroll: (f32, f32),
    opacity: Vec<f32>,
    groups: opacity::Groups,
    /// Rounded shapes being painted, whose corners are blended by their
    /// coverage when they end.
    corners: RefCell<corners::Coverage>,
    states: Vec<u32>,
    /// Whether each open clip is rounded, ending a coverage group.
    rounded_clips: Vec<bool>,
    /// The combined transforms of the enclosing transformed elements.
    transforms: Vec<[f32; 6]>,
    /// Whether the text insertion caret shows, in its blink phase.
    caret: bool,
    /// The device pixels being painted, when not all.
    area: Option<RECT>,
}

const IDENTITY: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// How far an inset shadow's outer edge reaches beyond its box, so that the
/// box is covered whatever the blur.
const BEYOND: f32 = 10_000.0;

use corners::Corners;

/// Device radii that fit their box. Corners that no longer fit, as when
/// snapping to device pixels narrows a circle's box, shrink together as CSS
/// fits radii and as Chromium constrains them: arcs that overlapped would be
/// joined by lines drawn back across the box.
fn fitted(rect: RectF, radii: Radii) -> Corners {
    let fit = |length: f32, a: f32, b: f32| {
        if a + b > length {
            length / (a + b)
        } else {
            1.0
        }
    };
    let radii = radii.map(|corner| {
        if corner.is_square() {
            Corner::SQUARE
        } else {
            corner
        }
    });
    let [top_left, top_right, bottom_right, bottom_left] = radii;
    let factor = fit(rect.Width, top_left.x, top_right.x)
        .min(fit(rect.Width, bottom_left.x, bottom_right.x))
        .min(fit(rect.Height, top_left.y, bottom_left.y))
        .min(fit(rect.Height, top_right.y, bottom_right.y));
    radii.map(|corner| (corner.x * factor, corner.y * factor))
}

/// Radii multiplied by `factor`.
fn scaled(radii: Radii, factor: f32) -> Radii {
    radii.map(|corner| corner.scaled(factor))
}

/// A rectangle path with elliptical corners in device pixels; the caller
/// deletes it.
fn elliptical_path(rect: RectF, corners: Corners) -> *mut GpPath {
    let mut path = null_mut();
    let [top_left, top_right, bottom_right, bottom_left] = corners;
    let (right, bottom) = (rect.X + rect.Width, rect.Y + rect.Height);
    let arcs = [
        (rect.X, rect.Y, top_left, 180.0),
        (right - 2.0 * top_right.0, rect.Y, top_right, 270.0),
        (
            right - 2.0 * bottom_right.0,
            bottom - 2.0 * bottom_right.1,
            bottom_right,
            0.0,
        ),
        (rect.X, bottom - 2.0 * bottom_left.1, bottom_left, 90.0),
    ];
    let points = [
        (rect.X, rect.Y),
        (right, rect.Y),
        (right, bottom),
        (rect.X, bottom),
    ];
    // SAFETY: the path is created here and only receives figures.
    unsafe {
        GdipCreatePath(FillModeAlternate, &mut path);
        for ((x, y, (width, height), start), (corner_x, corner_y)) in arcs.into_iter().zip(points) {
            if width > 0.0 && height > 0.0 {
                GdipAddPathArc(path, x, y, 2.0 * width, 2.0 * height, start, 90.0);
            } else {
                GdipAddPathLine(path, corner_x, corner_y, corner_x, corner_y);
            }
        }
        GdipClosePathFigure(path);
    }
    path
}

/// Where the line through `a` and `b` meets the line through `c` and `d`.
fn meeting(a: PointF, b: PointF, c: PointF, d: PointF) -> Option<PointF> {
    let cross = |u: (f32, f32), v: (f32, f32)| u.0 * v.1 - u.1 * v.0;
    let (ab, cd) = ((b.X - a.X, b.Y - a.Y), (d.X - c.X, d.Y - c.Y));
    let denominator = cross(ab, cd);
    if denominator.abs() < 1e-6 {
        return None;
    }
    let t = cross((c.X - a.X, c.Y - a.Y), cd) / denominator;
    Some(PointF {
        X: a.X + t * ab.0,
        Y: a.Y + t * ab.1,
    })
}

impl Painter {
    fn opacity(&self) -> f32 {
        self.opacity.last().copied().unwrap_or(1.0)
    }

    fn push_opacity(&mut self, opacity: f32) {
        // Only the pixels being painted are saved and blended: a caret's
        // blink does not copy the window.
        self.push_opacity_in(opacity, self.area);
    }

    fn push_opacity_in(&mut self, opacity: f32, region: Option<RECT>) {
        let opacity = opacity.clamp(0.0, 1.0);
        // SAFETY: drawing targets the live DIB. Flush before reading it.
        unsafe {
            GdipFlush(self.graphics, FlushIntentionFlush);
            GdiFlush();
        }
        let isolated = match region {
            Some(region) => self.groups.push_in(opacity, Some(region)),
            None => self.groups.push(opacity),
        };
        self.opacity
            .push(self.opacity() * if isolated { 1.0 } else { opacity });
    }

    fn pop_opacity(&mut self) {
        // SAFETY: flush pending drawing before changing the DIB pixels.
        unsafe {
            GdipFlush(self.graphics, FlushIntentionFlush);
            GdiFlush();
        }
        self.groups.pop();
        self.opacity.pop();
    }

    /// A rectangle in content coordinates as device pixels.
    fn rect(&self, rect: Rect) -> RectF {
        RectF {
            X: (rect.x - self.scroll.0) * self.scale,
            Y: (rect.y - self.scroll.1) * self.scale,
            Width: rect.width * self.scale,
            Height: rect.height * self.scale,
        }
    }

    /// A box's rectangle in content coordinates with its edges moved to
    /// whole device pixels, as Chromium paints backgrounds, borders and
    /// shadows: each edge to the nearest pixel, halves rounding up, and a
    /// box with any size at least a pixel across. Boxes that meet or
    /// overlap at a fraction of a pixel then meet in the same pixel.
    fn snapped(&self, rect: Rect) -> Rect {
        let snap = |start: f32, size: f32, scroll: f32| {
            let (first, last) = (
                (start - scroll) * self.scale,
                (start + size - scroll) * self.scale,
            );
            if !first.is_finite() || !last.is_finite() {
                return (start, size);
            }
            let near = |edge: f32| (edge + 0.5).floor();
            let (from, mut to) = (near(first), near(last));
            // Blink keeps sizes over four sixty-fourths of a pixel.
            if to == from && (last - first).abs() > 4.0 / 64.0 {
                to = from + (last - first).signum();
            }
            (from / self.scale + scroll, (to - from) / self.scale)
        };
        let (x, width) = snap(rect.x, rect.width, self.scroll.0);
        let (y, height) = snap(rect.y, rect.height, self.scroll.1);
        Rect::new(x, y, width, height)
    }

    /// A rounded rectangle path in device pixels; the caller deletes it.
    fn path(&self, rect: RectF, radii: Radii) -> *mut GpPath {
        elliptical_path(rect, fitted(rect, scaled(radii, self.scale)))
    }

    /// A box with rounded corners in device pixels as Chromium's coverage
    /// paints it, with an inner edge `inset` by whole pixels for a ring:
    /// when its edges are whole pixels and drawing is not transformed.
    /// Otherwise GDI+ paints the shape.
    fn shape(&self, rect: RectF, radii: Radii, inset: Option<f32>) -> Option<corners::Shape> {
        let edges = [rect.X, rect.Y, rect.X + rect.Width, rect.Y + rect.Height];
        let whole = |value: f32| (value - value.round()).abs() < 0.01;
        if self.current() != IDENTITY
            || radii.iter().all(|corner| corner.is_square())
            || !edges.into_iter().all(whole)
            || !inset.is_none_or(whole)
        {
            return None;
        }
        let [left, top, right, bottom] = edges.map(|edge| edge.round() as i32);
        let radii = fitted(rect, scaled(radii, self.scale));
        let inner = inset.map(|inset| {
            let inset = inset.round();
            let edges = [
                left as f32 + inset,
                top as f32 + inset,
                right as f32 - inset,
                bottom as f32 - inset,
            ];
            let less = |radius: f32| (radius - inset).max(0.0);
            (edges, radii.map(|(x, y)| (less(x), less(y))))
        });
        Some(corners::Shape {
            rect: [left, top, right, bottom],
            radii,
            inner,
        })
    }

    /// Starts painting a rounded shape, keeping what is beneath its corners.
    fn begin_corners(&self, shape: corners::Shape) {
        // SAFETY: drawing targets the live DIB. Flush before reading it.
        unsafe {
            GdipFlush(self.graphics, FlushIntentionFlush);
            GdiFlush();
        }
        self.corners.borrow_mut().push(shape);
    }

    /// Ends the last rounded shape: its corners are blended by coverage.
    fn end_corners(&self) {
        // SAFETY: flush pending drawing before changing the DIB pixels.
        unsafe {
            GdipFlush(self.graphics, FlushIntentionFlush);
            GdiFlush();
        }
        self.corners.borrow_mut().pop();
    }

    fn fill(&self, brush: *mut GpBrush, rect: RectF, radii: Radii) {
        if let Some(shape) = self.shape(rect, radii, None) {
            self.begin_corners(shape);
            // SAFETY: the graphics and brush are live.
            unsafe {
                GdipFillRectangle(
                    self.graphics,
                    brush,
                    rect.X,
                    rect.Y,
                    rect.Width,
                    rect.Height,
                )
            };
            self.end_corners();
            return;
        }
        // SAFETY: the graphics and brush are live; the path is deleted.
        unsafe {
            if radii.iter().all(|corner| corner.is_square()) {
                GdipFillRectangle(
                    self.graphics,
                    brush,
                    rect.X,
                    rect.Y,
                    rect.Width,
                    rect.Height,
                );
            } else {
                let path = self.path(rect, radii);
                GdipFillPath(self.graphics, brush, path);
                GdipDeletePath(path);
            }
        }
    }

    fn fill_color(&self, rect: RectF, color: Color, radii: Radii) {
        let mut brush: *mut GpSolidFill = null_mut();
        // SAFETY: the brush is created and deleted here.
        unsafe {
            GdipCreateSolidFill(argb(color, self.opacity()), &mut brush);
            self.fill(brush as *mut GpBrush, rect, radii);
            GdipDeleteBrush(brush as *mut GpBrush);
        }
    }

    fn gradient(&self, rect: Rect, angle: f32, stops: &[(Color, f32)], radii: Radii) {
        let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
            return;
        };
        let mut stops = stops.to_vec();
        let low = first.1.min(0.0);
        let high = last.1.max(1.0);
        if first.1 > low {
            stops.insert(0, (first.0, low));
        }
        if last.1 < high {
            stops.push((last.0, high));
        }
        let opacity = self.opacity();
        let colors: Vec<u32> = stops
            .iter()
            .map(|(color, _)| argb(*color, opacity))
            .collect();
        // GDI+ accepts stops only in [0, 1]. Extend the physical line to
        // the full CSS stop range, then normalize the brush's positions.
        let positions: Vec<f32> = stops
            .iter()
            .map(|(_, position)| (*position - low) / (high - low))
            .collect();
        let area = self.rect(rect);
        // CSS angles are clockwise from up. The gradient line runs through
        // the center and reaches the perpendicular projections of the
        // corners, preserving the angle even in a non-square box.
        let (dx, dy) = angle.to_radians().sin_cos();
        let dy = -dy;
        let half = (area.Width * dx.abs() + area.Height * dy.abs()) / 2.0;
        let center = PointF {
            X: area.X + area.Width / 2.0,
            Y: area.Y + area.Height / 2.0,
        };
        let start = PointF {
            X: center.X + dx * half * (2.0 * low - 1.0),
            Y: center.Y + dy * half * (2.0 * low - 1.0),
        };
        let end = PointF {
            X: center.X + dx * half * (2.0 * high - 1.0),
            Y: center.Y + dy * half * (2.0 * high - 1.0),
        };
        let mut brush: *mut GpLineGradient = null_mut();
        // SAFETY: the brush is created and deleted here; the color and
        // position arrays have `count` entries.
        unsafe {
            GdipCreateLineBrush(
                &start,
                &end,
                colors[0],
                colors[colors.len() - 1],
                WrapModeTileFlipXY,
                &mut brush,
            );
            GdipSetLinePresetBlend(
                brush,
                colors.as_ptr(),
                positions.as_ptr(),
                colors.len() as i32,
            );
            self.fill(brush as *mut GpBrush, area, radii);
            GdipDeleteBrush(brush as *mut GpBrush);
        }
    }

    fn border(
        &self,
        rect: Rect,
        widths: [f32; 4],
        colors: [Color; 4],
        styles: [BorderStyle; 4],
        radii: Radii,
    ) {
        let visible =
            |style: BorderStyle| !matches!(style, BorderStyle::None | BorderStyle::Hidden);
        let uniform = widths.iter().all(|width| *width == widths[0])
            && colors.iter().all(|color| *color == colors[0])
            && styles.iter().all(|style| *style == styles[0]);
        if uniform {
            if widths[0] <= 0.0 || !visible(styles[0]) {
                return;
            }
            if matches!(styles[0], BorderStyle::Solid)
                && let Some(shape) =
                    self.shape(self.rect(rect), radii, Some(widths[0] * self.scale))
            {
                self.ring(shape, colors[0]);
                return;
            }
            let half = widths[0] / 2.0;
            let inset = Rect::new(
                rect.x + half,
                rect.y + half,
                (rect.width - widths[0]).max(0.0),
                (rect.height - widths[0]).max(0.0),
            );
            let area = self.rect(inset);
            let mut pen = null_mut();
            // SAFETY: the pen and path are created and deleted here.
            unsafe {
                GdipCreatePen1(
                    argb(colors[0], self.opacity()),
                    widths[0] * self.scale,
                    UnitPixel,
                    &mut pen,
                );
                match styles[0] {
                    BorderStyle::Dashed => GdipSetPenDashStyle(pen, DashStyleDash),
                    BorderStyle::Dotted => GdipSetPenDashStyle(pen, DashStyleDot),
                    _ => Ok,
                };
                let path = self.path(area, radii.map(|corner| corner.less(half, half)));
                GdipDrawPath(self.graphics, pen, path);
                GdipDeletePath(path);
                GdipDeletePen(pen);
            }
            return;
        }
        let patterned =
            |style: BorderStyle| matches!(style, BorderStyle::Dashed | BorderStyle::Dotted);
        if radii.iter().any(|corner| !corner.is_square()) && !styles.into_iter().any(patterned) {
            self.rounded_border(rect, widths, colors, styles.map(visible), radii);
            return;
        }
        let [top, right, bottom, left] = widths;
        let sides = [
            Rect::new(rect.x, rect.y, rect.width, top),
            Rect::new(rect.x + rect.width - right, rect.y, right, rect.height),
            Rect::new(rect.x, rect.y + rect.height - bottom, rect.width, bottom),
            Rect::new(rect.x, rect.y, left, rect.height),
        ];
        for (edge, ((side, color), style)) in sides.into_iter().zip(colors).zip(styles).enumerate()
        {
            if side.width > 0.0 && side.height > 0.0 && visible(style) {
                let area = self.rect(side);
                if patterned(style) {
                    self.patterned_border(
                        area,
                        widths[edge] * self.scale,
                        edge % 2 == 0,
                        color,
                        style,
                    );
                } else {
                    self.fill_color(area, color, [Corner::SQUARE; 4]);
                }
            }
        }
    }

    /// Paints a uniform rounded border as Chromium covers it: its straight
    /// sides as whole pixels, then its corner squares, once each, blended by
    /// the ring's coverage.
    fn ring(&self, shape: corners::Shape, color: Color) {
        let Some(([inner_left, inner_top, inner_right, inner_bottom], _)) = shape.inner else {
            return;
        };
        let [left, top, right, bottom] = shape.rect.map(|edge| edge as f32);
        let sides = [
            (left, top, right - left, inner_top - top),
            (left, inner_bottom, right - left, bottom - inner_bottom),
            (left, inner_top, inner_left - left, inner_bottom - inner_top),
            (
                inner_right,
                inner_top,
                right - inner_right,
                inner_bottom - inner_top,
            ),
        ];
        let tiles = shape.tiles();
        self.begin_corners(shape);
        let mut brush: *mut GpSolidFill = null_mut();
        let mut state = 0;
        // SAFETY: the brush is created and deleted here; the saved state
        // restores the clip the tiles were excluded from.
        unsafe {
            GdipCreateSolidFill(argb(color, self.opacity()), &mut brush);
            GdipSaveGraphics(self.graphics, &mut state);
            for [left, top, right, bottom] in &tiles {
                GdipSetClipRectI(
                    self.graphics,
                    *left,
                    *top,
                    right - left,
                    bottom - top,
                    CombineModeExclude,
                );
            }
            for (x, y, width, height) in sides {
                if width > 0.0 && height > 0.0 {
                    GdipFillRectangle(self.graphics, brush.cast(), x, y, width, height);
                }
            }
            GdipRestoreGraphics(self.graphics, state);
            for [left, top, right, bottom] in &tiles {
                GdipFillRectangleI(
                    self.graphics,
                    brush.cast(),
                    *left,
                    *top,
                    right - left,
                    bottom - top,
                );
            }
            GdipDeleteBrush(brush.cast());
        }
        self.end_corners();
    }

    /// Draws rounded borders whose sides differ as Chromium does: the ring
    /// between the outer edge and an inner edge whose corners are the outer
    /// radii less the widths beside them, with each side's color filling the
    /// part of the ring between the lines from its outer corners toward its
    /// inner corners.
    fn rounded_border(
        &self,
        rect: Rect,
        widths: [f32; 4],
        colors: [Color; 4],
        visible: [bool; 4],
        radii: Radii,
    ) {
        let area = self.rect(rect);
        let outer = fitted(area, scaled(radii, self.scale));
        let [top, right, bottom, left] = widths.map(|width| width * self.scale);
        let (x, y) = (area.X + left, area.Y + top);
        let width = (area.Width - left - right).max(0.0);
        let height = (area.Height - top - bottom).max(0.0);
        let inner_area = RectF {
            X: x,
            Y: y,
            Width: width,
            Height: height,
        };
        // The widths beside each corner, across and down.
        let beside = [(left, top), (right, top), (right, bottom), (left, bottom)];
        let mut inner = outer;
        for (corner, (across, down)) in inner.iter_mut().zip(beside) {
            *corner = ((corner.0 - across).max(0.0), (corner.1 - down).max(0.0));
        }
        let point = |x: f32, y: f32| PointF { X: x, Y: y };
        let (right_edge, bottom_edge) = (area.X + area.Width, area.Y + area.Height);
        let outer_points = [
            point(area.X, area.Y),
            point(right_edge, area.Y),
            point(right_edge, bottom_edge),
            point(area.X, bottom_edge),
        ];
        let inner_points = [
            point(x, y),
            point(x + width, y),
            point(x + width, y + height),
            point(x, y + height),
        ];
        // Each corner's split line runs from its outer corner through its
        // inner corner, and past a rounded inner corner to the chord of its
        // arc, so that each side's part covers all of its ring.
        let inward = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)];
        let mut ends = inner_points;
        for (corner, end) in ends.iter_mut().enumerate() {
            let (across, down) = inner[corner];
            if across > 0.0 || down > 0.0 {
                let (sign_x, sign_y) = inward[corner];
                let at = inner_points[corner];
                let chord = (
                    point(at.X + sign_x * across, at.Y),
                    point(at.X, at.Y + sign_y * down),
                );
                if let Some(meet) = meeting(outer_points[corner], at, chord.0, chord.1) {
                    *end = meet;
                }
            }
        }
        let painted: Vec<usize> = (0..4)
            .filter(|side| widths[*side] > 0.0 && visible[*side])
            .collect();
        let Some(first) = painted.first() else {
            return;
        };
        // SAFETY: the paths and brushes are created and deleted here; the
        // clip is restored from the saved graphics state.
        unsafe {
            let ring = elliptical_path(area, outer);
            let hole = elliptical_path(inner_area, inner);
            GdipAddPathPath(ring, hole, 0);
            GdipDeletePath(hole);
            let fill = |color: Color| {
                let mut brush: *mut GpSolidFill = null_mut();
                GdipCreateSolidFill(argb(color, self.opacity()), &mut brush);
                GdipFillPath(self.graphics, brush as *mut GpBrush, ring);
                GdipDeleteBrush(brush as *mut GpBrush);
            };
            if painted.iter().all(|side| colors[*side] == colors[*first]) {
                fill(colors[*first]);
            } else {
                for side in painted {
                    let next = (side + 1) % 4;
                    let quad = [
                        outer_points[side],
                        ends[side],
                        ends[next],
                        outer_points[next],
                    ];
                    let mut clip = null_mut();
                    let mut state = 0;
                    GdipCreatePath(FillModeAlternate, &mut clip);
                    GdipAddPathPolygon(clip, quad.as_ptr(), 4);
                    GdipSaveGraphics(self.graphics, &mut state);
                    GdipSetClipPath(self.graphics, clip, CombineModeIntersect);
                    fill(colors[side]);
                    GdipRestoreGraphics(self.graphics, state);
                    GdipDeletePath(clip);
                }
            }
            GdipDeletePath(ring);
        }
    }

    /// Draws one straight border side in device coordinates. A single native
    /// pen draws dashes; dotted sides use circles clipped to the side's box.
    fn patterned_border(
        &self,
        area: RectF,
        width: f32,
        horizontal: bool,
        color: Color,
        style: BorderStyle,
    ) {
        if style == BorderStyle::Dotted {
            let length = if horizontal { area.Width } else { area.Height };
            let mut brush = null_mut();
            let mut state = 0;
            // SAFETY: save/restore this graphics object's clip, and own the
            // brush until the dots have been filled.
            unsafe {
                GdipCreateSolidFill(argb(color, self.opacity()), &mut brush);
                GdipSaveGraphics(self.graphics, &mut state);
                GdipSetClipRect(
                    self.graphics,
                    area.X,
                    area.Y,
                    area.Width,
                    area.Height,
                    CombineModeIntersect,
                );
                let mut visible = area;
                if GdipGetVisibleClipBounds(self.graphics, &mut visible) != Ok {
                    visible = area;
                }
                let (first, last) = if horizontal {
                    (visible.X - area.X, visible.X + visible.Width - area.X)
                } else {
                    (visible.Y - area.Y, visible.Y + visible.Height - area.Y)
                };
                let step = width * 2.0;
                // Keep the phase anchored to the original side when scrolling
                // or clipping; generate only dots in the visible interval.
                let mut offset = (first.max(0.0) / step).floor() * step;
                while offset < length.min(last) {
                    let (x, y) = if horizontal {
                        (area.X + offset, area.Y)
                    } else {
                        (area.X, area.Y + offset)
                    };
                    GdipFillEllipse(self.graphics, brush.cast(), x, y, width, width);
                    offset += step;
                }
                GdipRestoreGraphics(self.graphics, state);
                GdipDeleteBrush(brush.cast());
            }
            return;
        }
        let (x1, y1, x2, y2) = if horizontal {
            (
                area.X,
                area.Y + width / 2.0,
                area.X + area.Width,
                area.Y + width / 2.0,
            )
        } else {
            (
                area.X + width / 2.0,
                area.Y,
                area.X + width / 2.0,
                area.Y + area.Height,
            )
        };
        let mut pen = null_mut();
        // SAFETY: the graphics object is live; own and release this pen.
        unsafe {
            GdipCreatePen1(argb(color, self.opacity()), width, UnitPixel, &mut pen);
            GdipSetPenDashStyle(pen, DashStyleDash);
            GdipDrawLine(self.graphics, pen, x1, y1, x2, y2);
            GdipDeletePen(pen);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        rect: Rect,
        text: &str,
        font: &nativeboat_ir::Font,
        color: Color,
        (underline, line_through, underline_offset): (bool, bool, Option<f32>),
        (letter_spacing, rtl, grayscale): (f32, bool, bool),
    ) {
        // Letter spacing follows each character, in device pixels.
        let spacing = letter_spacing * self.scale;
        let alpha = color.a * self.opacity();
        if alpha <= 0.0 {
            return;
        }
        // Grayscale text is rasterized as Skia has DirectWrite do it, where
        // it can be. Elsewhere it is drawn with ClearType, then each pixel
        // takes the mean of its three channels' coverage, as Skia turns
        // DirectWrite's subpixel coverage into grayscale. What lay beneath
        // tells the coverage apart from the background.
        let paint = color;
        let mut by_gdi = false;
        let gray = if grayscale {
            self.text_region(rect, font.size)
        } else {
            None
        };
        let beneath = gray.map(|region| self.pixels(region));
        let mut colored = false;
        let area = self.rect(rect);
        // An underline at an offset the style sets is drawn here, beneath
        // the glyphs; GDI draws others with the font.
        let offset = underline_offset.filter(|_| underline);
        let underline = underline && offset.is_none();
        let handle = fonts::font(font, self.scale, underline, line_through);
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut metrics = TEXTMETRICW::default();
        // SAFETY: the DC, font and buffers are live for the calls.
        unsafe {
            GdipFlush(self.graphics, FlushIntentionFlush);
            SelectObject(self.dc, handle);
            GetTextMetricsW(self.dc, &mut metrics);
            let x = area.X.round() as i32;
            let (ascent, descent) = fonts::content_metrics(self.dc, font.size * self.scale)
                .unwrap_or((metrics.tmAscent as f32, metrics.tmDescent as f32));
            let leading = ((area.Height - (ascent + descent)) / 2.0 + 1e-3).floor();
            let top = (rect.y - self.scroll.1) * self.scale;
            let y = text_row(top + leading);
            // Glyphs go where layout measured them, at the face's advances
            // for the exact size.
            let size = font.size * self.scale;
            // GDI text has no alpha: blend with what is already painted.
            let color = if alpha < 1.0 {
                let under = GetPixel(self.dc, x, y + metrics.tmAscent / 2);
                let mix = |front: u8, back: u32| {
                    (f32::from(front) * alpha + (back & 0xFF) as f32 * (1.0 - alpha)).round() as u8
                };
                Color::rgb(
                    mix(color.r, under),
                    mix(color.g, under >> 8),
                    mix(color.b, under >> 16),
                )
            } else {
                color
            };
            SetTextColor(self.dc, colorref(color));
            SetBkMode(self.dc, TRANSPARENT as i32);
            // Pieces in fallback fonts share the requested font's baseline.
            let baseline = y + ascent as i32;
            if let Some(offset) = offset {
                let thickness = fonts::underline_thickness(self.dc).unwrap_or(1).max(1);
                let top = baseline + (offset * self.scale).round() as i32;
                let line = RECT {
                    left: area.X.round() as i32,
                    top,
                    right: (area.X + area.Width).round() as i32,
                    bottom: top + thickness,
                };
                let brush = CreateSolidBrush(colorref(color));
                FillRect(self.dc, &line, brush);
                DeleteObject(brush);
            }
            let previous = SetTextAlign(self.dc, TA_BASELINE);
            let mut left = area.X;
            for run in fonts::runs(self.dc, font, self.scale, underline, line_through, &units) {
                let piece = &units[run.range];
                // Emoji fonts draw in color.
                if let Some(width) = run.direct.and_then(|family| {
                    crate::dwrite::draw(
                        self.dc,
                        left.round() as i32,
                        baseline,
                        piece,
                        &family,
                        fonts::pixel_size(font, self.scale),
                        font.weight,
                        font.italic,
                        &font.features,
                        colorref(color),
                    )
                }) {
                    colored = true;
                    left += width;
                    continue;
                }
                SelectObject(self.dc, run.font);
                // Glyphs, when the text needs no shaping, stay in this font.
                // Right-to-left text is shaped, which mirrors its brackets.
                let glyphs = if rtl {
                    None
                } else {
                    fonts::simple_glyphs(self.dc, piece)
                };
                let advances = glyphs
                    .as_ref()
                    .and_then(|glyphs| fonts::advances(self.dc, glyphs, size, fonts::kerns(font)));
                // Grayscale glyphs without decorations are drawn as
                // Chromium draws them, where they can be.
                if let (Some(_), Some(glyphs), Some(advances), false) = (
                    gray,
                    &glyphs,
                    &advances,
                    underline || line_through || offset.is_some(),
                ) && self
                    .gray_glyphs(
                        (glyphs, advances, spacing),
                        size,
                        (left, baseline),
                        paint,
                        alpha,
                    )
                    .is_some()
                {
                    left += advances
                        .iter()
                        .map(|advance| advance + spacing)
                        .sum::<f32>();
                    continue;
                }
                by_gdi = true;
                match (glyphs, advances) {
                    (Some(glyphs), Some(advances)) => {
                        // Each glyph starts at its rounded position.
                        let mut pen = left;
                        let steps: Vec<i32> = advances
                            .iter()
                            .map(|advance| {
                                let next = pen + advance + spacing;
                                let step = next.round() as i32 - pen.round() as i32;
                                pen = next;
                                step
                            })
                            .collect();
                        ExtTextOutW(
                            self.dc,
                            left.round() as i32,
                            baseline,
                            ETO_GLYPH_INDEX,
                            core::ptr::null(),
                            glyphs.as_ptr(),
                            glyphs.len() as u32,
                            steps.as_ptr(),
                        );
                        left = pen;
                    }
                    (Some(glyphs), None) => {
                        let steps = spaced_steps(self.dc, piece, spacing);
                        ExtTextOutW(
                            self.dc,
                            left.round() as i32,
                            baseline,
                            ETO_GLYPH_INDEX,
                            core::ptr::null(),
                            glyphs.as_ptr(),
                            glyphs.len() as u32,
                            steps
                                .as_ref()
                                .map_or(core::ptr::null(), |steps| steps.as_ptr()),
                        );
                        left += fonts::extent(self.dc, piece) as f32 + spacing * piece.len() as f32;
                    }
                    (None, _) => {
                        let steps = spaced_steps(self.dc, piece, spacing);
                        ExtTextOutW(
                            self.dc,
                            left.round() as i32,
                            baseline,
                            if rtl { ETO_RTLREADING } else { 0 },
                            core::ptr::null(),
                            piece.as_ptr(),
                            piece.len() as u32,
                            steps
                                .as_ref()
                                .map_or(core::ptr::null(), |steps| steps.as_ptr()),
                        );
                        left += fonts::extent(self.dc, piece) as f32 + spacing * piece.len() as f32;
                    }
                };
            }
            SetTextAlign(self.dc, previous);
            // GDI+ reads the pixels next.
            GdiFlush();
            // Emoji keep their colors.
            if let (Some(region), Some(beneath), false, true) = (gray, beneath, colored, by_gdi) {
                self.grayscale(region, &beneath, color);
            }
        }
    }

    /// Draws grayscale glyphs of the DC's font, with their advances and
    /// the letter spacing after each, at a size in device pixels from a
    /// point on the baseline, as Chromium draws them: DirectWrite
    /// rasterizes them with ClearType, Skia takes the mean of each pixel's
    /// thirds, corrects it for the text's luminance and blends the color by
    /// it. `false` leaves the target untouched, for GDI to draw the glyphs:
    /// under a transform that scales or turns them, or without DirectWrite.
    fn gray_glyphs(
        &self,
        (glyphs, advances, spacing): (&[u16], &[f32], f32),
        size: f32,
        (x, baseline): (f32, i32),
        color: Color,
        alpha: f32,
    ) -> Option<()> {
        let mut transform = XFORM::default();
        // SAFETY: the DC is live and `transform` writable.
        unsafe { GetWorldTransform(self.dc, &mut transform) };
        let [a, b, c, d] = [
            transform.eM11,
            transform.eM12,
            transform.eM21,
            transform.eM22,
        ];
        (a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0).then_some(())?;
        let (size, symmetric) = fonts::smoothing(self.dc, size);
        let advances: Vec<f32> = advances.iter().map(|advance| advance + spacing).collect();
        let origin = (x + transform.eDx, baseline as f32 + transform.eDy);
        let coverage =
            crate::dwrite::coverage(self.dc, glyphs, &advances, size, origin, symmetric)?;
        let bounds = coverage.bounds;
        let width = (bounds.right - bounds.left) as usize;
        let table = mask_gamma(color);
        let front = [color.b, color.g, color.r].map(f32::from);
        let whole = RECT {
            left: 0,
            top: 0,
            right: self.target.bmWidth,
            bottom: self.target.bmHeight.abs(),
        };
        let stride = self.target.bmWidthBytes as usize / 4;
        // SAFETY: the DC is live; the region is deleted here. Pending
        // drawing lands in the target before its pixels are blended, and
        // only pixels inside it and inside the DC's clip are touched.
        unsafe {
            GdipFlush(self.graphics, FlushIntentionFlush);
            GdiFlush();
            let clip = CreateRectRgn(0, 0, 0, 0);
            let clipped = GetClipRgn(self.dc, clip) == 1;
            let mut box_ = whole;
            let complex = clipped && GetRgnBox(clip, &mut box_) == COMPLEXREGION;
            // Nothing is touched where the glyphs fall outside the clip.
            let area = [whole, self.clip_bounds(), box_]
                .into_iter()
                .try_fold(bounds, meet)
                .unwrap_or_default();
            {
                for y in area.top..area.bottom {
                    let row = self.target.bmBits.cast::<u32>().add(y as usize * stride);
                    for x in area.left..area.right {
                        let at =
                            ((y - bounds.top) as usize * width + (x - bounds.left) as usize) * 3;
                        let thirds = &coverage.thirds[at..at + 3];
                        let mean = thirds.iter().map(|third| u32::from(*third)).sum::<u32>() / 3;
                        let cover = f32::from(table[mean as usize]) / 255.0 * alpha;
                        if cover <= 0.0 || (complex && PtInRegion(clip, x, y) == 0) {
                            continue;
                        }
                        let pixel = row.add(x as usize);
                        let mut blended = *pixel & 0xFF00_0000;
                        for (front, shift) in front.iter().zip([0, 8, 16]) {
                            let back = ((*pixel >> shift) & 0xFF) as f32;
                            let value = (back + (front - back) * cover).round().clamp(0.0, 255.0);
                            blended |= (value as u32) << shift;
                        }
                        *pixel = blended;
                    }
                }
            }
            DeleteObject(clip);
        }
        Some(())
    }

    /// The device pixels text in a rectangle can change, within the clip
    /// and the target, when the target can be read.
    fn text_region(&self, rect: Rect, size: f32) -> Option<RECT> {
        let target = self.target;
        if target.bmBitsPixel != 32 || target.bmBits.is_null() {
            return None;
        }
        // Glyphs reach past their line box by up to about their size.
        let content = bounds::area(self.current(), rect, size);
        let device = RECT {
            left: ((content[0] - self.scroll.0) * self.scale).floor() as i32,
            top: ((content[1] - self.scroll.1) * self.scale).floor() as i32,
            right: ((content[2] - self.scroll.0) * self.scale).ceil() as i32,
            bottom: ((content[3] - self.scroll.1) * self.scale).ceil() as i32,
        };
        let whole = RECT {
            left: 0,
            top: 0,
            right: target.bmWidth,
            bottom: target.bmHeight.abs(),
        };
        meet(device, whole).and_then(|region| meet(region, self.clip_bounds()))
    }

    /// A copy of the target's pixels in a region inside it.
    fn pixels(&self, region: RECT) -> Vec<u32> {
        // SAFETY: flush pending drawing before reading the DIB.
        unsafe {
            GdipFlush(self.graphics, FlushIntentionFlush);
            GdiFlush();
        }
        let width = (region.right - region.left) as usize;
        let stride = self.target.bmWidthBytes as usize / 4;
        let mut pixels = Vec::with_capacity(width * (region.bottom - region.top) as usize);
        for row in region.top..region.bottom {
            let start = row as usize * stride + region.left as usize;
            // SAFETY: the region lies inside the live 32-bit section.
            pixels.extend_from_slice(unsafe {
                core::slice::from_raw_parts(self.target.bmBits.cast::<u32>().add(start), width)
            });
        }
        pixels
    }

    /// Turns text drawn with ClearType in a region into grayscale text of a
    /// color, given the pixels beneath it.
    fn grayscale(&self, region: RECT, beneath: &[u32], color: Color) {
        let width = (region.right - region.left) as usize;
        let stride = self.target.bmWidthBytes as usize / 4;
        for (row, line) in (region.top..region.bottom).zip(beneath.chunks(width)) {
            let start = row as usize * stride + region.left as usize;
            // SAFETY: the region lies inside the live 32-bit section, which
            // nothing else touches while this runs.
            let pixels = unsafe {
                core::slice::from_raw_parts_mut(self.target.bmBits.cast::<u32>().add(start), width)
            };
            for (pixel, under) in pixels.iter_mut().zip(line) {
                *pixel = gray_pixel(*under, *pixel, color);
            }
        }
    }

    fn image(&mut self, rect: Rect, url: &str) {
        if let Some(vector) = VECTORS.with_borrow(|vectors| vectors.get(url).cloned()) {
            let (size, items) = &*vector;
            for item in items {
                self.item(&placed(item, rect, *size));
            }
            return;
        }
        let Some(image) = IMAGES.with_borrow(|images| images.get(url).copied()) else {
            return;
        };
        if !url.starts_with(nativeboat_layout::VIDEO_FRAME) {
            self.bitmap(image, rect);
            return;
        }
        // Chromium's compositor samples video frames bilinearly, without
        // the filtering it gives images it scales down.
        let mut state = 0;
        // SAFETY: the graphics is live; the saved state restores its mode.
        unsafe {
            GdipSaveGraphics(self.graphics, &mut state);
            GdipSetInterpolationMode(self.graphics, InterpolationModeBilinear);
        }
        self.bitmap(image, rect);
        // SAFETY: as above.
        unsafe { GdipRestoreGraphics(self.graphics, state) };
    }

    /// Draws a whole bitmap into a rectangle.
    fn bitmap(&self, image: Image, rect: Rect) {
        let area = self.rect(rect);
        let attributes = faded(self.opacity());
        // SAFETY: the image is live; the attributes are disposed here.
        unsafe {
            GdipDrawImageRectRect(
                self.graphics,
                image.handle,
                area.X,
                area.Y,
                area.Width,
                area.Height,
                0.0,
                0.0,
                image.width as f32,
                image.height as f32,
                UnitPixel,
                attributes,
                0,
                null_mut(),
            );
            if !attributes.is_null() {
                GdipDisposeImageAttributes(attributes);
            }
        }
    }

    /// Fills and outlines a vector path.
    fn vector(
        &self,
        segments: &[PathSegment],
        transform: [f32; 6],
        fill: Option<Color>,
        fill_rule: FillRule,
        stroke: Option<&Stroke>,
    ) {
        let mode = match fill_rule {
            FillRule::NonZero => FillModeWinding,
            FillRule::EvenOdd => FillModeAlternate,
        };
        // Path units map to content coordinates, then scroll and scale.
        let [a, b, c, d, e, f] = transform.map(|value| value * self.scale);
        let (dx, dy) = (
            e - self.scroll.0 * self.scale,
            f - self.scroll.1 * self.scale,
        );
        let (mut path, mut matrix) = (null_mut(), null_mut());
        // SAFETY: the path, matrix, brush and pen are created and deleted
        // here; the graphics object is live.
        unsafe {
            GdipCreatePath(mode, &mut path);
            let (mut current, mut start) = ((0.0, 0.0), (0.0, 0.0));
            for segment in segments {
                match *segment {
                    PathSegment::Move(x, y) => {
                        GdipStartPathFigure(path);
                        (current, start) = ((x, y), (x, y));
                    }
                    PathSegment::Line(x, y) => {
                        GdipAddPathLine(path, current.0, current.1, x, y);
                        current = (x, y);
                    }
                    PathSegment::Cubic(x1, y1, x2, y2, x, y) => {
                        GdipAddPathBezier(path, current.0, current.1, x1, y1, x2, y2, x, y);
                        current = (x, y);
                    }
                    PathSegment::Close => {
                        GdipClosePathFigure(path);
                        current = start;
                    }
                }
            }
            GdipCreateMatrix2(a, b, c, d, dx, dy, &mut matrix);
            // The path's own transform applies before any enclosing one.
            let mut state = 0;
            GdipSaveGraphics(self.graphics, &mut state);
            GdipMultiplyWorldTransform(self.graphics, matrix, MatrixOrderPrepend);
            if let Some(color) = fill {
                let mut brush: *mut GpSolidFill = null_mut();
                GdipCreateSolidFill(argb(color, self.opacity()), &mut brush);
                GdipFillPath(self.graphics, brush as *mut GpBrush, path);
                GdipDeleteBrush(brush as *mut GpBrush);
            }
            if let Some(stroke) = stroke {
                let mut pen = null_mut();
                GdipCreatePen1(
                    argb(stroke.color, self.opacity()),
                    stroke.width,
                    UnitWorld,
                    &mut pen,
                );
                let cap = match stroke.cap {
                    LineCap::Butt => LineCapFlat,
                    LineCap::Round => LineCapRound,
                    LineCap::Square => LineCapSquare,
                };
                // Dashes end as the line does; GDI+ has no square dash end.
                let dash_cap = if stroke.cap == LineCap::Round {
                    DashCapRound
                } else {
                    DashCapFlat
                };
                GdipSetPenLineCap197819(pen, cap, cap, dash_cap);
                let join = match stroke.join {
                    LineJoin::Miter => LineJoinMiter,
                    LineJoin::Round => LineJoinRound,
                    LineJoin::Bevel => LineJoinBevel,
                };
                GdipSetPenLineJoin(pen, join);
                // GDI+ measures dashes in pen widths, and takes none empty.
                if !stroke.dashes.is_empty() && stroke.width > 0.0 {
                    let relative: Vec<f32> = stroke
                        .dashes
                        .iter()
                        .map(|length| (length / stroke.width).max(0.001))
                        .collect();
                    GdipSetPenDashArray(pen, relative.as_ptr(), relative.len() as i32);
                    GdipSetPenDashOffset(pen, stroke.dash_offset / stroke.width);
                }
                // GDI+ draws pens under about one and a half device pixels
                // too thin: an icon's 1.46 pixel stroke comes out 1 pixel
                // wide. Its fills keep their area, so the stroke is widened
                // to its outline, dashes and caps included, and filled; the
                // outline overlaps itself where strokes cross.
                let (mut outline, mut brush) = (null_mut(), null_mut());
                let device = (a * d - b * c).abs().sqrt();
                let flatness = if device > 0.0 { 0.1 / device } else { 0.1 };
                GdipClonePath(path, &mut outline);
                if GdipWidenPath(outline, pen, null_mut(), flatness) == Ok {
                    GdipSetPathFillMode(outline, FillModeWinding);
                    GdipCreateSolidFill(argb(stroke.color, self.opacity()), &mut brush);
                    GdipFillPath(self.graphics, brush as *mut GpBrush, outline);
                    GdipDeleteBrush(brush as *mut GpBrush);
                } else {
                    GdipDrawPath(self.graphics, pen, path);
                }
                GdipDeletePath(outline);
                GdipDeletePen(pen);
            }
            GdipRestoreGraphics(self.graphics, state);
            GdipDeleteMatrix(matrix);
            GdipDeletePath(path);
        }
    }

    /// Draws the shadow's opaque shape, then blurs it before clipping to
    /// the inside or outside of the box. The blur therefore crosses edges
    /// and rounded corners without bands or a cut-off at the box boundary.
    fn shadow(
        &self,
        shape: Rect,
        radii: Radii,
        color: Color,
        blur: f32,
        inset: bool,
        clip: (Rect, Radii),
    ) {
        let mut state = 0;
        // SAFETY: the paths are created and deleted here; clipping and any
        // enclosing transform are preserved by the saved graphics state.
        unsafe {
            GdipSaveGraphics(self.graphics, &mut state);
            let box_path = self.path(self.rect(clip.0), clip.1);
            let mode = if inset {
                CombineModeIntersect
            } else {
                CombineModeExclude
            };
            GdipSetClipPath(self.graphics, box_path, mode);
            GdipDeletePath(box_path);
            if blur > 0.0 {
                self.blurred_shadow(shape, radii, color, blur, inset, clip.0);
            } else {
                let mut brush = null_mut();
                GdipCreateSolidFill(argb(color, self.opacity()), &mut brush);
                let outer = self.rect(Rect::new(
                    clip.0.x - BEYOND,
                    clip.0.y - BEYOND,
                    clip.0.width + 2.0 * BEYOND,
                    clip.0.height + 2.0 * BEYOND,
                ));
                let path = self.shadow_path(self.rect(shape), radii, inset, outer);
                GdipFillPath(self.graphics, brush as *mut GpBrush, path);
                GdipDeletePath(path);
                GdipDeleteBrush(brush as *mut GpBrush);
            }
            GdipRestoreGraphics(self.graphics, state);
        }
    }

    /// An outer shadow's shape, or the opaque area outside an inset hole.
    fn shadow_path(&self, shape: RectF, radii: Radii, inset: bool, outer: RectF) -> *mut GpPath {
        let path = self.path(shape, radii);
        if inset {
            // SAFETY: the path is owned by the caller. Alternate fill leaves
            // the inner rounded rectangle transparent.
            unsafe {
                GdipAddPathRectangle(path, outer.X, outer.Y, outer.Width, outer.Height);
            }
        }
        path
    }

    fn blurred_shadow(
        &self,
        shape: Rect,
        radii: Radii,
        color: Color,
        blur: f32,
        inset: bool,
        clip: Rect,
    ) {
        let area = self.rect(if inset { clip } else { shape });
        let radius = blur * self.scale;
        let pad = radius.ceil() + 2.0;
        let (left, top) = ((area.X - pad).floor(), (area.Y - pad).floor());
        // GDI+ accepts a kernel radius up to 255. Larger CSS blurs use a
        // smaller bitmap and are scaled back when drawn, without clamping.
        let factor = (radius / 250.0).max(1.0);
        let width = ((area.X + area.Width + pad - left) / factor).ceil() as i32;
        let height = ((area.Y + area.Height + pad - top) / factor).ceil() as i32;
        let shape = self.rect(shape);
        let local = RectF {
            X: (shape.X - left) / factor,
            Y: (shape.Y - top) / factor,
            Width: shape.Width / factor,
            Height: shape.Height / factor,
        };
        let radii = scaled(radii, 1.0 / factor);
        // Integer translations and scrolling reuse the same raster. Fractional
        // origins, DPI, dimensions, corners, color and blur all affect the key.
        let key = [
            width as u32,
            height as u32,
            local.X.to_bits(),
            local.Y.to_bits(),
            local.Width.to_bits(),
            local.Height.to_bits(),
            (radii[0].x * self.scale).to_bits(),
            (radii[0].y * self.scale).to_bits(),
            (radii[1].x * self.scale).to_bits(),
            (radii[1].y * self.scale).to_bits(),
            (radii[2].x * self.scale).to_bits(),
            (radii[2].y * self.scale).to_bits(),
            (radii[3].x * self.scale).to_bits(),
            (radii[3].y * self.scale).to_bits(),
            (radius / factor).to_bits(),
            argb(color, self.opacity()),
            inset as u32,
        ];
        let image = match SHADOWS.with_borrow_mut(|cache| cache.get(&key)) {
            Some(image) => image,
            None => {
                let Some(image) = self.shadow_bitmap(
                    local,
                    radii,
                    color,
                    radius / factor,
                    inset,
                    (width, height),
                ) else {
                    return;
                };
                SHADOWS.with_borrow_mut(|cache| cache.insert(key, image.clone()));
                image
            }
        };
        // SAFETY: image remains live through the draw; the destination graphics
        // applies the enclosing clipping and transforms to the cached bitmap.
        unsafe {
            GdipDrawImageRectRect(
                self.graphics,
                image.handle,
                left,
                top,
                width as f32 * factor,
                height as f32 * factor,
                0.0,
                0.0,
                width as f32,
                height as f32,
                UnitPixel,
                null_mut(),
                0,
                null_mut(),
            );
        }
    }

    fn shadow_bitmap(
        &self,
        local: RectF,
        radii: Radii,
        color: Color,
        radius: f32,
        inset: bool,
        size: (i32, i32),
    ) -> Option<Rc<ShadowImage>> {
        let (width, height) = size;
        let mut bitmap = null_mut();
        let mut graphics = null_mut();
        // SAFETY: the bitmap owns its pixels; graphics is released before
        // applying the effect, and the image is disposed on every exit.
        unsafe {
            // Effects operate on premultiplied pixels. Keep that format when
            // drawing the result, so translucent colors are not darkened twice.
            if GdipCreateBitmapFromScan0(width, height, 0, 0x000E_200B, null_mut(), &mut bitmap)
                != Ok
            {
                return None;
            }
            if GdipGetImageGraphicsContext(bitmap.cast(), &mut graphics) != Ok {
                GdipDisposeImage(bitmap.cast());
                return None;
            }
            GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
            GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
            let outer = RectF {
                X: 0.0,
                Y: 0.0,
                Width: width as f32,
                Height: height as f32,
            };
            let path = self.shadow_path(local, radii, inset, outer);
            let mut brush = null_mut();
            // Blur alpha alone, then apply the uniform shadow color. GDI+
            // effects otherwise introduce dark RGB fringes at transparent edges.
            GdipCreateSolidFill(
                argb(
                    Color {
                        r: 0,
                        g: 0,
                        b: 0,
                        ..color
                    },
                    self.opacity(),
                ),
                &mut brush,
            );
            GdipFillPath(graphics, brush.cast(), path);
            GdipDeleteBrush(brush.cast());
            GdipDeletePath(path);
            GdipDeleteGraphics(graphics);
            if blur_bitmap(bitmap, radius) && tint_bitmap(bitmap, width, height, color) {
                return Some(Rc::new(ShadowImage {
                    handle: bitmap.cast(),
                    bytes: width as usize * height as usize * 4,
                }));
            }
            GdipDisposeImage(bitmap.cast());
        }
        None
    }

    /// Applies a transform, combined with the enclosing ones, to GDI+ and
    /// GDI drawing.
    fn transform(&mut self, push: Option<[f32; 6]>) {
        match push {
            Some(transform) => {
                let outer = self.current();
                self.transforms
                    .push(nativeboat_layout::svg::multiply(outer, transform));
            }
            None => {
                self.transforms.pop();
            }
        }
        self.apply_transform();
    }

    /// The combined transform of the enclosing transformed elements.
    fn current(&self) -> [f32; 6] {
        self.transforms.last().copied().unwrap_or(IDENTITY)
    }

    fn apply_transform(&self) {
        let [a, b, c, d, e, f] = self.current();
        // Drawing is already in scaled device pixels, so the transform is
        // moved into that space: scroll, then scale.
        let (x, y) = self.scroll;
        let dx = self.scale * (a * x + c * y + e - x);
        let dy = self.scale * (b * x + d * y + f - y);
        let mut matrix = null_mut();
        let form = XFORM {
            eM11: a,
            eM12: b,
            eM21: c,
            eM22: d,
            eDx: dx,
            eDy: dy,
        };
        // SAFETY: the matrix is created and deleted here; the DC and graphics
        // are live.
        unsafe {
            GdipCreateMatrix2(a, b, c, d, dx, dy, &mut matrix);
            GdipSetWorldTransform(self.graphics, matrix);
            GdipDeleteMatrix(matrix);
            SetGraphicsMode(self.dc, GM_ADVANCED);
            SetWorldTransform(self.dc, &form);
        }
    }

    fn push_clip(&mut self, rect: Rect, radii: Radii) {
        // A rounded clip clips to its snapped box; its corners are blended
        // by coverage when it ends, as Chromium antialiases them.
        let snapped = self.rect(self.snapped(rect));
        if let Some(shape) = self.shape(snapped, radii, None) {
            let mut state = 0;
            // SAFETY: the graphics and DC are live.
            unsafe {
                GdipSaveGraphics(self.graphics, &mut state);
                GdipSetClipRect(
                    self.graphics,
                    snapped.X,
                    snapped.Y,
                    snapped.Width,
                    snapped.Height,
                    CombineModeIntersect,
                );
                SaveDC(self.dc);
                let [left, top, right, bottom] = shape.rect;
                IntersectClipRect(self.dc, left, top, right, bottom);
            }
            self.states.push(state);
            self.rounded_clips.push(true);
            self.begin_corners(shape);
            return;
        }
        let area = self.rect(rect);
        let mut state = 0;
        // SAFETY: the graphics and DC are live; the path is deleted.
        unsafe {
            GdipSaveGraphics(self.graphics, &mut state);
            let path = self.path(area, radii);
            GdipSetClipPath(self.graphics, path, CombineModeIntersect);
            GdipDeletePath(path);
            SaveDC(self.dc);
            IntersectClipRect(
                self.dc,
                area.X.floor() as i32,
                area.Y.floor() as i32,
                (area.X + area.Width).ceil() as i32,
                (area.Y + area.Height).ceil() as i32,
            );
        }
        self.states.push(state);
        self.rounded_clips.push(false);
    }

    /// Clips to a path in content coordinates: GDI+ to the path, GDI to the
    /// region it covers.
    fn push_clip_path(&mut self, segments: &[PathSegment], fill_rule: FillRule) {
        let mode = match fill_rule {
            FillRule::NonZero => FillModeWinding,
            FillRule::EvenOdd => FillModeAlternate,
        };
        let point = |x: f32, y: f32| {
            (
                (x - self.scroll.0) * self.scale,
                (y - self.scroll.1) * self.scale,
            )
        };
        let mut state = 0;
        // SAFETY: the graphics and DC are live; the path, region and GDI
        // region are deleted here.
        unsafe {
            GdipSaveGraphics(self.graphics, &mut state);
            let mut path = null_mut();
            GdipCreatePath(mode, &mut path);
            let (mut current, mut start) = ((0.0, 0.0), (0.0, 0.0));
            for segment in segments {
                match *segment {
                    PathSegment::Move(x, y) => {
                        GdipStartPathFigure(path);
                        current = point(x, y);
                        start = current;
                    }
                    PathSegment::Line(x, y) => {
                        let to = point(x, y);
                        GdipAddPathLine(path, current.0, current.1, to.0, to.1);
                        current = to;
                    }
                    PathSegment::Cubic(x1, y1, x2, y2, x, y) => {
                        let ((x1, y1), (x2, y2), to) = (point(x1, y1), point(x2, y2), point(x, y));
                        GdipAddPathBezier(path, current.0, current.1, x1, y1, x2, y2, to.0, to.1);
                        current = to;
                    }
                    PathSegment::Close => {
                        GdipClosePathFigure(path);
                        current = start;
                    }
                }
            }
            GdipSetClipPath(self.graphics, path, CombineModeIntersect);
            SaveDC(self.dc);
            let mut region = null_mut();
            GdipCreateRegionPath(path, &mut region);
            let mut gdi = null_mut();
            GdipGetRegionHRgn(region, self.graphics, &mut gdi);
            ExtSelectClipRgn(self.dc, gdi, RGN_AND);
            DeleteObject(gdi);
            GdipDeleteRegion(region);
            GdipDeletePath(path);
        }
        self.states.push(state);
        self.rounded_clips.push(false);
    }

    fn pop_clip(&mut self) {
        if self.rounded_clips.pop() == Some(true) {
            self.end_corners();
        }
        if let Some(state) = self.states.pop() {
            // SAFETY: the state was saved by `push_clip` on these objects.
            unsafe {
                GdipRestoreGraphics(self.graphics, state);
                RestoreDC(self.dc, -1);
            }
        }
    }

    /// Paints items in order; a filter group paints off-screen first.
    fn items(&mut self, items: &[DisplayItem]) {
        let mut index = 0;
        while index < items.len() {
            if let DisplayItem::PushFilter(filters) = &items[index] {
                let end = bounds::group_end(items, index);
                let started = crate::window::microseconds();
                self.filter_group(filters, &items[index + 1..end]);
                if crate::trace::painting() {
                    record_item(17, started);
                }
                index = end;
            } else if let DisplayItem::PushMask(mask) = &items[index] {
                let end = bounds::group_end(items, index);
                let started = crate::window::microseconds();
                self.mask_group(mask, &items[index + 1..end]);
                if crate::trace::painting() {
                    record_item(17, started);
                }
                index = end;
            } else if let DisplayItem::PushOpacity(opacity) = &items[index] {
                // Fully transparent content draws nothing, as a closed menu
                // faded out to opacity 0 does: skip it, unless it leaves
                // clipping or transforms changed after the group.
                if *opacity <= 0.0 {
                    let end = bounds::group_end(items, index);
                    if bounds::balanced(&items[index + 1..end]) {
                        index = end + 1;
                        continue;
                    }
                }
                if !(*opacity > 0.0 && *opacity < 1.0) {
                    self.item(&items[index]);
                    index += 1;
                    continue;
                }
                let end = bounds::group_end(items, index);
                let region = match bounds::opacity_extent(&items[index + 1..end], self.current()) {
                    bounds::Extent::Unknown => None,
                    bounds::Extent::Empty => Some(RECT::default()),
                    bounds::Extent::Known(content) => {
                        let edges = [
                            (content[0] - self.scroll.0) * self.scale,
                            (content[1] - self.scroll.1) * self.scale,
                            (content[2] - self.scroll.0) * self.scale,
                            (content[3] - self.scroll.1) * self.scale,
                        ];
                        if edges.iter().all(|edge| edge.is_finite()) {
                            let device = RECT {
                                left: (edges[0].floor() as i32).saturating_sub(1),
                                top: (edges[1].floor() as i32).saturating_sub(1),
                                right: (edges[2].ceil() as i32).saturating_add(1),
                                bottom: (edges[3].ceil() as i32).saturating_add(1),
                            };
                            // Bitmap clamping happens in Groups. Keep the whole
                            // content extent: integer clip queries may round
                            // inward at fractional DPI/transform boundaries.
                            Some(device)
                        } else {
                            None
                        }
                    }
                };
                // Drawing is clipped to the area being painted, so only its
                // pixels are saved and blended, as a caret's blink in a
                // translucent editor would otherwise copy the editor.
                let region = match (region, self.area) {
                    (Some(region), Some(area)) => Some(RECT {
                        left: region.left.max(area.left),
                        top: region.top.max(area.top),
                        right: region.right.min(area.right).max(region.left.max(area.left)),
                        bottom: region.bottom.min(area.bottom).max(region.top.max(area.top)),
                    }),
                    (None, area) => area,
                    (region, None) => region,
                };
                // No visible content needs no backdrop, but scopes still
                // execute to preserve the display list's state transitions.
                let started = crate::window::microseconds();
                self.push_opacity_in(*opacity, region);
                if crate::trace::painting() {
                    record_item(14, started);
                }
            } else if !self.outside_area(&items[index]) {
                if crate::trace::painting() {
                    let started = crate::window::microseconds();
                    self.item(&items[index]);
                    record_item(item_kind(&items[index]), started);
                } else {
                    self.item(&items[index]);
                }
            }
            index += 1;
        }
    }

    /// Whether an item draws only outside the area being painted, so that
    /// drawing it changes nothing: a caret's blink in an editor repaints a
    /// few pixels, which shadows, images and text elsewhere need not cost.
    fn outside_area(&self, item: &DisplayItem) -> bool {
        let Some(area) = self.area else {
            return false;
        };
        if !matches!(
            item,
            DisplayItem::Fill { .. }
                | DisplayItem::Gradient { .. }
                | DisplayItem::Border { .. }
                | DisplayItem::Text { .. }
                | DisplayItem::Path { .. }
                | DisplayItem::Canvas { .. }
                | DisplayItem::Image { .. }
                | DisplayItem::Shadow { .. }
                | DisplayItem::Backdrop { .. }
        ) {
            return false;
        }
        let Some(content) = bounds::bounds(core::slice::from_ref(item), self.current()) else {
            return true;
        };
        let edges = [
            (content[0] - self.scroll.0) * self.scale,
            (content[1] - self.scroll.1) * self.scale,
            (content[2] - self.scroll.0) * self.scale,
            (content[3] - self.scroll.1) * self.scale,
        ];
        if !edges.iter().all(|edge| edge.is_finite()) {
            return false;
        }
        // A pixel of slack on each side, as rounding at fractional scales
        // may reach it.
        edges[2] + 1.0 < area.left as f32
            || edges[0] - 1.0 > area.right as f32
            || edges[3] + 1.0 < area.top as f32
            || edges[1] - 1.0 > area.bottom as f32
    }

    /// The device pixels drawing can change: the clip within the target.
    fn clip_bounds(&self) -> RECT {
        let mut bounds = windows_sys::Win32::Graphics::GdiPlus::Rect::default();
        let mut state = 0;
        // SAFETY: the graphics is live. Without a world transform or the
        // half-pixel offset, the bounds are in device pixels; the saved
        // state restores both.
        unsafe {
            GdipSaveGraphics(self.graphics, &mut state);
            GdipResetWorldTransform(self.graphics);
            GdipSetPixelOffsetMode(self.graphics, PixelOffsetModeNone);
            GdipGetVisibleClipBoundsI(self.graphics, &mut bounds);
            GdipRestoreGraphics(self.graphics, state);
        }
        RECT {
            left: bounds.X,
            top: bounds.Y,
            right: bounds.X + bounds.Width,
            bottom: bounds.Y + bounds.Height,
        }
    }

    /// The CSS-to-device linear map at this point of the display list.
    fn matrix(&self) -> [f32; 4] {
        let [a, b, c, d, ..] = self.current();
        [a, b, c, d].map(|value| value * self.scale)
    }

    /// Paints items as an isolated layer, then draws it through filter
    /// functions. The layer is painted over black and over white to recover
    /// its transparency, because GDI text has no alpha channel.
    fn filter_group(&mut self, filters: &[Filter], items: &[DisplayItem]) {
        let transform = self.current();
        let Some(content) = bounds::bounds(items, transform) else {
            return;
        };
        let (x, y) = filter::reach(filters, self.matrix());
        let (x, y) = (x as i32, y as i32);
        // Device pixels with a pixel of room for antialiasing.
        let device = RECT {
            left: ((content[0] - self.scroll.0) * self.scale).floor() as i32 - 1,
            top: ((content[1] - self.scroll.1) * self.scale).floor() as i32 - 1,
            right: ((content[2] - self.scroll.0) * self.scale).ceil() as i32 + 1,
            bottom: ((content[3] - self.scroll.1) * self.scale).ceil() as i32 + 1,
        };
        let spread = grown(device, x, y);
        let Some(shown) = meet(spread, self.clip_bounds()) else {
            return;
        };
        // What shows depends on content within the reach of it.
        let region = meet(grown(shown, x, y), spread).unwrap_or(shown);
        let layers = [0, 0x00FF_FFFF].map(|fill| self.layer(items, region, fill));
        let [Some(black), Some(white)] = layers else {
            self.items(items);
            return;
        };
        let mut image =
            filter::Image::isolate(black.pixels(), white.pixels(), black.width, black.height);
        filter::apply(&mut image, filters, self.matrix());
        self.composite(&image, (region.left, region.top), None);
    }

    /// Paints items off-screen, then keeps of them only what the alpha of
    /// `mask`, painted over the same region, covers.
    fn mask_group(&mut self, mask: &[DisplayItem], items: &[DisplayItem]) {
        let transform = self.current();
        let Some(content) = bounds::bounds(items, transform) else {
            return;
        };
        // Device pixels with a pixel of room for antialiasing.
        let device = RECT {
            left: ((content[0] - self.scroll.0) * self.scale).floor() as i32 - 1,
            top: ((content[1] - self.scroll.1) * self.scale).floor() as i32 - 1,
            right: ((content[2] - self.scroll.0) * self.scale).ceil() as i32 + 1,
            bottom: ((content[3] - self.scroll.1) * self.scale).ceil() as i32 + 1,
        };
        let Some(region) = meet(device, self.clip_bounds()) else {
            return;
        };
        let [Some(black), Some(white)] =
            [0, 0x00FF_FFFF].map(|fill| self.layer(items, region, fill))
        else {
            self.items(items);
            return;
        };
        let [Some(mask_black), Some(mask_white)] =
            [0, 0x00FF_FFFF].map(|fill| self.layer(mask, region, fill))
        else {
            self.items(items);
            return;
        };
        let mut image =
            filter::Image::isolate(black.pixels(), white.pixels(), black.width, black.height);
        let coverage = filter::Image::isolate(
            mask_black.pixels(),
            mask_white.pixels(),
            mask_black.width,
            mask_black.height,
        );
        for (pixel, cover) in image.pixels.iter_mut().zip(&coverage.pixels) {
            *pixel = pixel.map(|channel| channel * cover[3]);
        }
        self.composite(&image, (region.left, region.top), None);
    }

    /// Paints items off-screen into a device region over a uniform color.
    fn layer(&self, items: &[DisplayItem], region: RECT, fill: u32) -> Option<Layer> {
        let layer = Layer::new(region.right - region.left, region.bottom - region.top, fill)?;
        let scroll = (
            self.scroll.0 + region.left as f32 / self.scale,
            self.scroll.1 + region.top as f32 / self.scale,
        );
        paint_with(
            layer.dc,
            items,
            Setup {
                scale: self.scale,
                scroll,
                area: None,
                caret: self.caret,
                transform: self.current(),
            },
        );
        Some(layer)
    }

    /// Draws premultiplied pixels at a device position, optionally only
    /// inside a rounded box in content coordinates.
    fn composite(&self, image: &filter::Image, at: (i32, i32), shape: Option<(Rect, Radii)>) {
        let (width, height) = (image.width as i32, image.height as i32);
        let mut pixels = image.pargb();
        let mut bitmap = null_mut();
        let mut state = 0;
        let attributes = faded(self.opacity());
        // SAFETY: the bitmap wraps `pixels`, which outlive it; the path and
        // attributes are released here and the saved state restores the
        // transform, clip and interpolation.
        unsafe {
            GdipCreateBitmapFromScan0(
                width,
                height,
                width * 4,
                PARGB,
                pixels.as_mut_ptr().cast(),
                &mut bitmap,
            );
            GdipSaveGraphics(self.graphics, &mut state);
            if let Some((rect, radii)) = shape {
                let path = self.path(self.rect(rect), radii);
                GdipSetClipPath(self.graphics, path, CombineModeIntersect);
                GdipDeletePath(path);
            }
            GdipResetWorldTransform(self.graphics);
            GdipSetInterpolationMode(self.graphics, InterpolationModeNearestNeighbor);
            GdipDrawImageRectRectI(
                self.graphics,
                bitmap.cast(),
                at.0,
                at.1,
                width,
                height,
                0,
                0,
                width,
                height,
                UnitPixel,
                attributes,
                0,
                null_mut(),
            );
            GdipRestoreGraphics(self.graphics, state);
            GdipDisposeImage(bitmap.cast());
            if !attributes.is_null() {
                GdipDisposeImageAttributes(attributes);
            }
        }
    }

    /// Filters what is already painted inside a box. Like Chrome, the box's
    /// edges are mirrored, so content outside it does not bleed in.
    fn backdrop(&mut self, rect: Rect, radii: Radii, filters: &[Filter]) {
        // SAFETY: drawing targets the live DIB. Flush before reading it.
        unsafe {
            GdipFlush(self.graphics, FlushIntentionFlush);
            GdiFlush();
        }
        let target = self.target;
        if target.bmBitsPixel != 32 || target.bmBits.is_null() {
            return;
        }
        let content = bounds::area(self.current(), rect, 0.0);
        let device = RECT {
            left: ((content[0] - self.scroll.0) * self.scale).floor() as i32,
            top: ((content[1] - self.scroll.1) * self.scale).floor() as i32,
            right: ((content[2] - self.scroll.0) * self.scale).ceil() as i32,
            bottom: ((content[3] - self.scroll.1) * self.scale).ceil() as i32,
        };
        let whole = RECT {
            left: 0,
            top: 0,
            right: target.bmWidth,
            bottom: target.bmHeight.abs(),
        };
        let (x, y) = filter::reach(filters, self.matrix());
        // Pixels beyond the reach of what shows cannot change it; mirroring
        // there instead of at the box edge changes only hidden pixels.
        let visible = grown(self.clip_bounds(), x as i32, y as i32);
        let Some(region) = meet(device, whole).and_then(|region| meet(region, visible)) else {
            return;
        };
        let (width, height) = (
            (region.right - region.left) as usize,
            (region.bottom - region.top) as usize,
        );
        let stride = target.bmWidthBytes as usize / 4;
        let mut pixels = Vec::with_capacity(width * height);
        for row in region.top..region.bottom {
            let start = row as usize * stride + region.left as usize;
            // SAFETY: the region lies inside the live 32-bit section.
            pixels.extend_from_slice(unsafe {
                core::slice::from_raw_parts(target.bmBits.cast::<u32>().add(start), width)
            });
        }
        let mut image = filter::Image::opaque(&pixels, width, height).mirrored(x, y);
        filter::apply(&mut image, filters, self.matrix());
        let image = image.crop(x, y, width, height);
        self.composite(&image, (region.left, region.top), Some((rect, radii)));
    }

    fn item(&mut self, item: &DisplayItem) {
        match item {
            DisplayItem::Fill { rect, color, radii } => {
                self.fill_color(self.rect(self.snapped(*rect)), *color, *radii)
            }
            DisplayItem::Gradient {
                rect,
                angle,
                stops,
                radii,
            } => self.gradient(self.snapped(*rect), *angle, stops, *radii),
            DisplayItem::Border {
                rect,
                widths,
                colors,
                styles,
                radii,
            } => self.border(self.snapped(*rect), *widths, *colors, *styles, *radii),
            DisplayItem::Text {
                rect,
                text,
                font,
                color,
                underline,
                line_through,
                letter_spacing,
                rtl,
                grayscale,
                underline_offset,
            } => self.text(
                *rect,
                text,
                font,
                *color,
                (*underline, *line_through, *underline_offset),
                (*letter_spacing, *rtl, *grayscale),
            ),
            DisplayItem::Path {
                segments,
                transform,
                fill,
                fill_rule,
                stroke,
            } => self.vector(segments, *transform, *fill, *fill_rule, stroke.as_ref()),
            DisplayItem::Canvas { rect, key, .. } => {
                if let Some((handle, width, height)) = crate::canvas::bitmap(*key) {
                    let image = Image {
                        handle,
                        width,
                        height,
                    };
                    self.bitmap(image, *rect);
                }
            }
            DisplayItem::Image { rect, url } => self.image(*rect, url),
            DisplayItem::PushClip { rect, radii } => self.push_clip(*rect, *radii),
            DisplayItem::PushClipPath {
                segments,
                fill_rule,
            } => self.push_clip_path(segments, *fill_rule),
            DisplayItem::PopClip => self.pop_clip(),
            DisplayItem::Shadow {
                rect,
                radii,
                color,
                blur,
                inset,
                clip,
                clip_radii,
            } => {
                // The shadow keeps its place against its box.
                let box_ = self.snapped(*clip);
                let shape = Rect::new(
                    rect.x + box_.x - clip.x,
                    rect.y + box_.y - clip.y,
                    rect.width + box_.width - clip.width,
                    rect.height + box_.height - clip.height,
                );
                self.shadow(shape, *radii, *color, *blur, *inset, (box_, *clip_radii))
            }
            DisplayItem::PushTransform(transform) => self.transform(Some(*transform)),
            DisplayItem::PopTransform => self.transform(None),
            DisplayItem::PushOpacity(opacity) => {
                self.push_opacity(*opacity);
            }
            DisplayItem::PopOpacity => {
                self.pop_opacity();
            }
            DisplayItem::Backdrop {
                rect,
                radii,
                filters,
            } => self.backdrop(self.snapped(*rect), *radii, filters),
            // `items` paints filter groups; vector images hold none. What
            // lies beneath a surface is painted by its window.
            DisplayItem::PushFilter(_)
            | DisplayItem::PopFilter
            | DisplayItem::PushMask(_)
            | DisplayItem::PopMask
            | DisplayItem::Beneath => {}
            DisplayItem::Caret { rect, color } if self.caret => {
                // A caret snaps to whole device pixels to stay sharp.
                let area = self.rect(*rect);
                let snapped = RectF {
                    X: area.X.round(),
                    Y: area.Y.round(),
                    Width: area.Width.round().max(1.0),
                    Height: area.Height.round(),
                };
                self.fill_color(snapped, *color, [Corner::SQUARE; 4]);
            }
            DisplayItem::Child(nativeboat_ir::NativeChild::Control(id)) => {
                let found = PROXIES.with_borrow(|proxies| {
                    proxies
                        .iter()
                        .find(|(proxy, ..)| proxy == id)
                        .map(|(_, image, rect)| (*image, *rect))
                });
                if let Some((image, rect)) = found {
                    self.bitmap(image, rect);
                }
            }
            DisplayItem::Caret { .. } | DisplayItem::Child(_) => {}
        }
    }
}

/// Paints with the images of controls kept out of sight, each in its box,
/// at their markers; the images are disposed after.
pub(crate) fn with_proxies<T>(
    proxies: Vec<(crate::controls::proxy::Snapshot, Rect)>,
    paint: impl FnOnce() -> T,
) -> T {
    if proxies.is_empty() {
        return paint();
    }
    let images = proxies
        .into_iter()
        .map(|(snapshot, rect)| {
            let image = Image {
                handle: snapshot.image,
                width: snapshot.size.0,
                height: snapshot.size.1,
            };
            (snapshot.id, image, rect)
        })
        .collect();
    let previous = PROXIES.with_borrow_mut(|proxies| core::mem::replace(proxies, images));
    let painted = paint();
    let painted_with = PROXIES.with_borrow_mut(|proxies| core::mem::replace(proxies, previous));
    for (_, image, _) in painted_with {
        // SAFETY: the snapshot's bitmap was handed over to be disposed here.
        unsafe { GdipDisposeImage(image.handle) };
    }
    painted
}

/// Paints a display list into a device context; `scroll` is the content
/// offset shown at the top left, in device-independent pixels.
/// `PixelFormat32bppRGB`, which windows-sys does not define.
const RGB32: i32 = 0x0002_2009;

/// Applies GDI+'s Gaussian kernel in place. Its radius produces an edge
/// profile within CSS's 5% tolerance for sigma = half the CSS blur radius.
fn blur_bitmap(bitmap: *mut GpBitmap, radius: f32) -> bool {
    let mut effect = null_mut();
    let parameters = BlurParams {
        radius,
        expandEdge: 0,
    };
    // SAFETY: bitmap is live; the new effect and its copied parameters are
    // used only here. No auxiliary allocation is requested from GDI+.
    unsafe {
        if GdipCreateEffect(BlurEffectGuid, &mut effect) != Ok {
            return false;
        }
        let success = GdipSetEffectParameters(
            effect,
            (&parameters as *const BlurParams).cast(),
            size_of::<BlurParams>() as u32,
        ) == Ok
            && GdipBitmapApplyEffect(bitmap, effect, null_mut(), 0, null_mut(), null_mut()) == Ok;
        GdipDeleteEffect(effect);
        success
    }
}

/// Colors an alpha mask in premultiplied form, avoiding color bleeding.
fn tint_bitmap(bitmap: *mut GpBitmap, width: i32, height: i32, color: Color) -> bool {
    let rect = windows_sys::Win32::Graphics::GdiPlus::Rect {
        X: 0,
        Y: 0,
        Width: width,
        Height: height,
    };
    let mut data = BitmapData::default();
    // SAFETY: the rectangle covers the live bitmap. Each pixel is accessed
    // within the returned stride; unlocking releases the temporary pixel view.
    unsafe {
        if GdipBitmapLockBits(
            bitmap,
            &rect,
            (ImageLockModeRead | ImageLockModeWrite) as u32,
            0x000E_200B,
            &mut data,
        ) != Ok
        {
            return false;
        }
        for row in 0..height {
            let line = (data.Scan0 as *mut u8).offset(row as isize * data.Stride as isize);
            for column in 0..width {
                let pixel = line.add(column as usize * 4);
                let alpha = *pixel.add(3) as u32;
                *pixel = ((color.b as u32 * alpha + 127) / 255) as u8;
                *pixel.add(1) = ((color.g as u32 * alpha + 127) / 255) as u8;
                *pixel.add(2) = ((color.r as u32 * alpha + 127) / 255) as u8;
            }
        }
        GdipBitmapUnlockBits(bitmap, &mut data);
    }
    true
}

/// Paints display items into the 32-bit top-down DIB section selected into
/// `dc`, only inside `area` when one is given, in device pixels, and with
/// the caret when `caret` is set. GDI+ draws straight into the section's
/// pixels, which is much faster than drawing through the DC, and GDI draws
/// text into the same pixels.
pub(crate) fn paint_area(
    dc: HDC,
    items: &[DisplayItem],
    scale: f32,
    scroll: (f32, f32),
    area: Option<RECT>,
    caret: bool,
) {
    paint_with(
        dc,
        items,
        Setup {
            scale,
            scroll,
            area,
            caret,
            transform: IDENTITY,
        },
    );
}

/// How a display list is painted.
struct Setup {
    scale: f32,
    scroll: (f32, f32),
    area: Option<RECT>,
    caret: bool,
    /// The transform of the elements enclosing the items.
    transform: [f32; 6],
}

fn paint_with(dc: HDC, items: &[DisplayItem], setup: Setup) {
    let Setup {
        scale,
        scroll,
        area,
        caret,
        transform,
    } = setup;
    let mut section = DIBSECTION::default();
    let mut bitmap = null_mut();
    let mut graphics = null_mut();
    // SAFETY: the DC and its DIB section are live for the duration of
    // painting; the GDI+ bitmap wraps the section's pixels until then.
    unsafe {
        GetObjectW(
            GetCurrentObject(dc, OBJ_BITMAP as u32),
            size_of::<DIBSECTION>() as i32,
            (&mut section as *mut DIBSECTION).cast(),
        );
        let pixels = section.dsBm;
        if pixels.bmBits.is_null() {
            // A printer has no pixels to wrap: GDI+ draws on it directly,
            // in its device pixels.
            GdipCreateFromHDC(dc, &mut graphics);
            GdipSetPageUnit(graphics, UnitPixel);
        } else {
            GdipCreateBitmapFromScan0(
                pixels.bmWidth,
                pixels.bmHeight,
                pixels.bmWidthBytes,
                RGB32,
                pixels.bmBits.cast(),
                &mut bitmap,
            );
            GdipGetImageGraphicsContext(bitmap.cast(), &mut graphics);
        }
        GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
        GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);
        SaveDC(dc);
        if let Some(area) = area {
            GdipSetClipRectI(
                graphics,
                area.left,
                area.top,
                area.right - area.left,
                area.bottom - area.top,
                CombineModeReplace,
            );
            IntersectClipRect(dc, area.left, area.top, area.right, area.bottom);
        }
    }
    let mut painter = Painter {
        graphics,
        dc,
        target: section.dsBm,
        scale,
        scroll,
        opacity: Vec::new(),
        // SAFETY: this section stays live for the entire paint operation.
        groups: unsafe { opacity::Groups::new(section.dsBm) },
        // SAFETY: as above.
        corners: RefCell::new(unsafe { corners::Coverage::new(section.dsBm) }),
        states: Vec::new(),
        rounded_clips: Vec::new(),
        transforms: vec![transform],
        caret,
        area,
    };
    if transform != IDENTITY {
        painter.apply_transform();
    }
    painter.items(items);
    while !painter.groups.is_empty() {
        painter.pop_opacity();
    }
    while !painter.states.is_empty() {
        painter.pop_clip();
    }
    let identity = XFORM {
        eM11: 1.0,
        eM22: 1.0,
        ..XFORM::default()
    };
    // SAFETY: the DC is live; it may be painted again later.
    unsafe {
        SetWorldTransform(dc, &identity);
        RestoreDC(dc, -1);
    }
    // SAFETY: the graphics object and bitmap were created above.
    unsafe {
        GdipDeleteGraphics(graphics);
        if !bitmap.is_null() {
            GdipDisposeImage(bitmap.cast());
        }
    }
}

/// `PixelFormat32bppPARGB`, which windows-sys does not define.
const PARGB: i32 = 0x000E_200B;

/// The device pixel row text whose top lies at `y` starts on. Chromium
/// places text in layout units, 64ths of a device pixel, so `y` snaps to
/// them first and rounding noise from fractional line heights cannot move
/// some lines a row. Its baseline, a whole number of pixels below, then
/// goes to the nearest row, and one halfway between rows to the lower, as
/// Skia places glyphs.
fn text_row(y: f32) -> i32 {
    let snapped = (y * 64.0).round() / 64.0;
    (snapped + 0.5).floor() as i32
}

/// A pixel of text drawn with ClearType in `color` over `under`, as
/// grayscale text: the mean coverage of the channels that tell text from
/// background, applied to all three.
fn gray_pixel(under: u32, drawn: u32, color: Color) -> u32 {
    if under == drawn {
        return drawn;
    }
    let channel = |value: u32, shift: u32| ((value >> shift) & 0xFF) as f32;
    let front = [color.b, color.g, color.r].map(f32::from);
    let shifts = [0, 8, 16];
    let (mut sum, mut count) = (0.0, 0.0);
    for (front, shift) in front.iter().zip(shifts) {
        let span = front - channel(under, shift);
        // A channel where text and background nearly match shows little
        // of the coverage.
        if span.abs() >= 16.0 {
            sum += ((channel(drawn, shift) - channel(under, shift)) / span).clamp(0.0, 1.0);
            count += 1.0;
        }
    }
    if count == 0.0 {
        return drawn;
    }
    let coverage = sum / count;
    let mut pixel = drawn & 0xFF00_0000;
    for (front, shift) in front.iter().zip(shifts) {
        let back = channel(under, shift);
        let value = (back + (front - back) * coverage).round().clamp(0.0, 255.0) as u32;
        pixel |= value << shift;
    }
    pixel
}

/// Skia's correction of grayscale text's coverage for a color, with
/// Chromium's settings on Windows, sRGB and full contrast
/// (`SkTMaskGamma_build_correcting_lut`): blended in sRGB values, the
/// coverage gives what blending in linear light would, with contrast added
/// as the background is lighter. Text is judged by its luminance in eight
/// levels.
fn mask_gamma(color: Color) -> [u8; 256] {
    let luminance =
        (u32::from(color.r) * 54 + u32::from(color.g) * 183 + u32::from(color.b) * 19) >> 8;
    let level = (luminance >> 5) as usize;
    MASK_GAMMA.with_borrow_mut(|tables| *tables[level].get_or_insert_with(|| correcting(level)))
}

/// The coverage correction for text at a luminance level of eight.
fn correcting(level: usize) -> [u8; 256] {
    const CONTRAST: f32 = 1.0;
    let to_linear = |value: f32| {
        if value <= 0.04045 {
            value / 12.92
        } else {
            libm::powf((value + 0.055) / 1.055, 2.4)
        }
    };
    let from_linear = |value: f32| {
        if value <= 0.003_130_8 {
            value * 12.92
        } else {
            1.055 * libm::powf(value, 1.0 / 2.4) - 0.055
        }
    };
    // Three bits widened back to a byte.
    let source = ((level << 5) | (level << 2) | (level >> 1)) as f32 / 255.0;
    // The background is guessed to be the text's opposite.
    let back = 1.0 - source;
    let (linear_source, linear_back) = (to_linear(source), to_linear(back));
    // Contrast fades as the background darkens. No level makes the text
    // and its guessed background alike, where Skia corrects nothing.
    let contrast = CONTRAST * linear_back;
    let mut table = [0u8; 256];
    for (index, entry) in table.iter_mut().enumerate() {
        let raw = index as f32 / 255.0;
        let covered = raw + (1.0 - raw) * contrast * raw;
        let out = from_linear(linear_source * covered + (1.0 - covered) * linear_back);
        let corrected = (out - back) / (source - back);
        *entry = libm::floorf(255.0 * corrected + 0.5).clamp(0.0, 255.0) as u8;
    }
    table
}

/// A rectangle grown on each side.
fn grown(rect: RECT, x: i32, y: i32) -> RECT {
    RECT {
        left: rect.left - x,
        top: rect.top - y,
        right: rect.right + x,
        bottom: rect.bottom + y,
    }
}

/// The overlap of two rectangles, if any.
pub(crate) fn meet(a: RECT, b: RECT) -> Option<RECT> {
    let rect = RECT {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    (rect.left < rect.right && rect.top < rect.bottom).then_some(rect)
}

/// An off-screen top-down 32-bit DIB section with its own DC.
struct Layer {
    dc: HDC,
    bitmap: HBITMAP,
    bits: *const u32,
    width: usize,
    height: usize,
}

impl Layer {
    /// A layer filled with a 32-bit RGB value, unless GDI is out of memory.
    fn new(width: i32, height: i32, fill: u32) -> Option<Layer> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..BITMAPINFOHEADER::default()
            },
            ..BITMAPINFO::default()
        };
        let mut bits = null_mut();
        // SAFETY: the DC and section are released in `drop`; the pixels are
        // written within the section's size.
        unsafe {
            let bitmap =
                CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                return None;
            }
            let dc = CreateCompatibleDC(null_mut());
            SelectObject(dc, bitmap);
            let (width, height) = (width as usize, height as usize);
            core::slice::from_raw_parts_mut(bits.cast::<u32>(), width * height).fill(fill);
            Some(Layer {
                dc,
                bitmap,
                bits: bits.cast(),
                width,
                height,
            })
        }
    }

    /// The painted pixels; GDI drawing must be flushed first.
    fn pixels(&self) -> &[u32] {
        // SAFETY: the section lives as long as the layer.
        unsafe {
            GdiFlush();
            core::slice::from_raw_parts(self.bits, self.width * self.height)
        }
    }
}

impl Drop for Layer {
    fn drop(&mut self) {
        // SAFETY: created in `new`; deleting the DC releases the selection.
        unsafe {
            DeleteDC(self.dc);
            DeleteObject(self.bitmap);
        }
    }
}

/// How far, in device pixels, backdrop filters in a display list read
/// beyond what they change, so that partial repaints can paint that far.
pub(crate) fn backdrop_reach<'a>(
    items: impl IntoIterator<Item = &'a DisplayItem>,
    scale: f32,
) -> i32 {
    let mut transforms = vec![IDENTITY];
    let mut reach = 0;
    for item in items {
        let current = *transforms.last().unwrap_or(&IDENTITY);
        match item {
            DisplayItem::PushTransform(transform) => transforms.push(multiply(current, *transform)),
            DisplayItem::PopTransform => {
                transforms.pop();
            }
            DisplayItem::Backdrop { filters, .. } => {
                let [a, b, c, d, ..] = current;
                let (x, y) = filter::reach(filters, [a, b, c, d].map(|value| value * scale));
                reach = reach.max(x.max(y) as i32);
            }
            _ => {}
        }
    }
    reach
}

#[cfg(test)]
pub(crate) mod tests;

/// The advance of each UTF-16 unit of a piece in the DC's font, with
/// letter spacing added, for `ExtTextOutW`; `None` without spacing.
fn spaced_steps(dc: HDC, piece: &[u16], spacing: f32) -> Option<Vec<i32>> {
    if spacing == 0.0 || piece.is_empty() {
        return None;
    }
    let mut ends = vec![0i32; piece.len()];
    let mut size = windows_sys::Win32::Foundation::SIZE::default();
    // SAFETY: the buffers are sized for the piece.
    unsafe {
        GetTextExtentExPointW(
            dc,
            piece.as_ptr(),
            piece.len() as i32,
            0,
            core::ptr::null_mut(),
            ends.as_mut_ptr(),
            &mut size,
        );
    }
    let mut previous = 0;
    let mut pen = 0.0f32;
    let mut placed = 0i32;
    Some(
        ends.iter()
            .map(|end| {
                pen += (end - previous) as f32 + spacing;
                previous = *end;
                let step = pen.round() as i32 - placed;
                placed += step;
                step
            })
            .collect(),
    )
}

/// The kinds of display items the paint trace times, by name.
const ITEM_KINDS: [&str; 19] = [
    "fill",
    "gradient",
    "border",
    "text",
    "path",
    "canvas",
    "image",
    "shadow",
    "backdrop",
    "child",
    "clip",
    "clip-path",
    "pop-clip",
    "transform",
    "opacity",
    "pop-opacity",
    "caret",
    "group",
    "other",
];

/// Microseconds each kind of item took to draw since the paint trace last
/// reported them.
static ITEM_TIMES: [core::sync::atomic::AtomicU64; 19] =
    [const { core::sync::atomic::AtomicU64::new(0) }; 19];

/// How many items of each kind were drawn since the last report.
static ITEM_COUNTS: [core::sync::atomic::AtomicU64; 19] =
    [const { core::sync::atomic::AtomicU64::new(0) }; 19];

fn item_kind(item: &DisplayItem) -> usize {
    match item {
        DisplayItem::Fill { .. } => 0,
        DisplayItem::Gradient { .. } => 1,
        DisplayItem::Border { .. } => 2,
        DisplayItem::Text { .. } => 3,
        DisplayItem::Path { .. } => 4,
        DisplayItem::Canvas { .. } => 5,
        DisplayItem::Image { .. } => 6,
        DisplayItem::Shadow { .. } => 7,
        DisplayItem::Backdrop { .. } => 8,
        DisplayItem::Child(_) => 9,
        DisplayItem::PushClip { .. } => 10,
        DisplayItem::PushClipPath { .. } => 11,
        DisplayItem::PopClip => 12,
        DisplayItem::PushTransform(_) | DisplayItem::PopTransform => 13,
        DisplayItem::PushOpacity(_) => 14,
        DisplayItem::PopOpacity => 15,
        DisplayItem::Caret { .. } => 16,
        DisplayItem::PushFilter(_) | DisplayItem::PushMask(_) => 17,
        _ => 18,
    }
}

/// Adds an item's drawing time to the paint trace's report.
fn record_item(kind: usize, started: u64) {
    ITEM_TIMES[kind].fetch_add(
        crate::window::microseconds().saturating_sub(started),
        core::sync::atomic::Ordering::Relaxed,
    );
    ITEM_COUNTS[kind].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

/// The paint trace's report of what each kind of item took to draw since
/// the last report, slowest first, and the counts reset.
pub(crate) fn item_times() -> alloc::string::String {
    let mut times: alloc::vec::Vec<(f64, &str, u64)> = ITEM_TIMES
        .iter()
        .zip(ITEM_KINDS)
        .zip(&ITEM_COUNTS)
        .map(|((time, kind), count)| {
            (
                time.swap(0, core::sync::atomic::Ordering::Relaxed) as f64 / 1000.0,
                kind,
                count.swap(0, core::sync::atomic::Ordering::Relaxed),
            )
        })
        .filter(|(time, ..)| *time >= 0.1)
        .collect();
    times.sort_by(|a, b| b.0.total_cmp(&a.0));
    times
        .iter()
        .map(|(time, kind, count)| alloc::format!(", {kind} {time:.1} ms/{count}"))
        .collect()
}
