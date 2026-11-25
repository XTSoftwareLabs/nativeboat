//! Bitmaps of `<canvas>` elements, drawn with GDI+.
//!
//! Each canvas is a premultiplied 32-bit bitmap with a GDI+ graphics object
//! that keeps the clip region between batches of commands. Display lists
//! show a canvas with [`DisplayItem::Canvas`](nativeboat_ir::DisplayItem).

#[allow(unused_imports)]
use crate::prelude::*;
use crate::render::{argb, faded, image_handle, insert_image};
use crate::wide::wide;
use core::cell::RefCell;
use core::ptr::{null, null_mut};
use nativeboat_css::Color;
use nativeboat_css::types::{FillRule, LineCap, LineJoin};
use nativeboat_ir::{
    CanvasCommand, CanvasPaint, CanvasSource, Font, PathSegment, Rect, TextAnchor,
};
use windows_sys::Win32::Foundation::HGLOBAL;
use windows_sys::Win32::Graphics::GdiPlus::*;
use windows_sys::Win32::System::Com::StructuredStorage::{
    CreateStreamOnHGlobal, GetHGlobalFromStream,
};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
use windows_sys::core::{GUID, HRESULT, IUnknown_Vtbl};

/// `PixelFormat32bppARGB`, which windows-sys does not define.
const ARGB: i32 = 0x0026_200A;
/// `PixelFormat32bppPARGB`.
const PARGB: i32 = 0x000E_200B;
/// The GDI+ PNG encoder.
const PNG: GUID = GUID::from_u128(0x557cf406_1a04_11d3_9a73_0000f81ef32e);
/// The GDI+ JPEG encoder.
const JPEG: GUID = GUID::from_u128(0x557cf401_1a04_11d3_9a73_0000f81ef32e);

struct Canvas {
    bitmap: *mut GpBitmap,
    graphics: *mut GpGraphics,
    width: u32,
    height: u32,
    /// Saved graphics states, for `save` and `restore`.
    states: Vec<u32>,
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: the canvas owns both objects.
        unsafe {
            GdipDeleteGraphics(self.graphics);
            GdipDisposeImage(self.bitmap as *mut GpImage);
        }
    }
}

local! {
    static CANVASES: RefCell<HashMap<u64, Canvas>> = RefCell::new(HashMap::new());
}

/// A transparent bitmap of the given size, or `None` for an empty one.
fn create(width: u32, height: u32) -> Option<Canvas> {
    if width == 0 || height == 0 {
        return None;
    }
    crate::init().ok()?;
    let (mut bitmap, mut graphics) = (null_mut(), null_mut());
    // SAFETY: GDI+ allocates the pixels; the graphics object draws on them.
    unsafe {
        if GdipCreateBitmapFromScan0(width as i32, height as i32, 0, PARGB, null(), &mut bitmap)
            != Ok
        {
            return None;
        }
        GdipGetImageGraphicsContext(bitmap as *mut GpImage, &mut graphics);
        GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
        GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAlias);
        GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);
    }
    Some(Canvas {
        bitmap,
        graphics,
        width,
        height,
        states: Vec::new(),
    })
}

/// The bitmap of canvas `key` with its size, for painting display lists.
pub(crate) fn bitmap(key: u64) -> Option<(*mut GpImage, u32, u32)> {
    CANVASES.with_borrow(|canvases| {
        canvases
            .get(&key)
            .map(|canvas| (canvas.bitmap as *mut GpImage, canvas.width, canvas.height))
    })
}

/// Builds a GDI+ path; the caller deletes it.
fn path(segments: &[PathSegment], rule: FillRule) -> *mut GpPath {
    let mode = match rule {
        FillRule::NonZero => FillModeWinding,
        FillRule::EvenOdd => FillModeAlternate,
    };
    let mut path = null_mut();
    let (mut current, mut start) = ((0.0, 0.0), (0.0, 0.0));
    // SAFETY: the path is created here and only receives figures.
    unsafe {
        GdipCreatePath(mode, &mut path);
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
    }
    path
}

/// Gradient stops as GDI+ blend colors and positions, padded so that the
/// colors before the first and after the last stop extend outwards.
fn blend(stops: &[(f32, Color)], alpha: f32, reverse: bool) -> (Vec<u32>, Vec<f32>) {
    let mut stops: Vec<(f32, Color)> = stops
        .iter()
        .map(|(offset, color)| (offset.clamp(0.0, 1.0), *color))
        .collect();
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    let first = stops.first().map_or(Color::TRANSPARENT, |stop| stop.1);
    let last = stops.last().map_or(Color::TRANSPARENT, |stop| stop.1);
    stops.insert(0, (0.0, first));
    stops.push((1.0, last));
    if reverse {
        stops = stops
            .into_iter()
            .rev()
            .map(|(offset, color)| (1.0 - offset, color))
            .collect();
    }
    stops
        .into_iter()
        .map(|(offset, color)| (argb(color, alpha), offset))
        .unzip()
}

/// How far linear gradient lines are extended, in multiples of their
/// length, so that tiling never shows.
const PAD: f32 = 64.0;

/// A brush for a paint; the caller deletes it.
fn brush(paint: &CanvasPaint, alpha: f32) -> *mut GpBrush {
    // SAFETY: each brush is created here and returned to the caller.
    unsafe {
        match paint {
            CanvasPaint::Color(color) => {
                let mut solid = null_mut();
                GdipCreateSolidFill(argb(*color, alpha), &mut solid);
                solid as *mut GpBrush
            }
            CanvasPaint::Linear { start, end, stops } => {
                let (dx, dy) = (end.0 - start.0, end.1 - start.1);
                let from = PointF {
                    X: start.0 - dx * PAD,
                    Y: start.1 - dy * PAD,
                };
                let to = PointF {
                    X: end.0 + dx * PAD,
                    Y: end.1 + dy * PAD,
                };
                let (colors, positions) = blend(stops, alpha, false);
                let span = 2.0 * PAD + 1.0;
                let positions: Vec<f32> = positions
                    .iter()
                    .enumerate()
                    .map(|(index, offset)| match index {
                        0 => 0.0,
                        _ if index + 1 == positions.len() => 1.0,
                        _ => (PAD + offset) / span,
                    })
                    .collect();
                let mut line = null_mut();
                GdipCreateLineBrush(
                    &from,
                    &to,
                    colors[0],
                    colors[0],
                    WrapModeTileFlipXY,
                    &mut line,
                );
                GdipSetLinePresetBlend(
                    line,
                    colors.as_ptr(),
                    positions.as_ptr(),
                    colors.len() as i32,
                );
                line as *mut GpBrush
            }
            CanvasPaint::Radial { start, end, stops } => {
                let ellipse = self::path(&[], FillRule::NonZero);
                let radius = end.2.max(0.001);
                GdipAddPathEllipse(
                    ellipse,
                    end.0 - radius,
                    end.1 - radius,
                    2.0 * radius,
                    2.0 * radius,
                );
                let mut radial = null_mut();
                GdipCreatePathGradientFromPath(ellipse, &mut radial);
                GdipDeletePath(ellipse);
                let center = PointF {
                    X: start.0,
                    Y: start.1,
                };
                GdipSetPathGradientCenterPoint(radial, &center);
                let (colors, positions) = blend(stops, alpha, true);
                GdipSetPathGradientPresetBlend(
                    radial,
                    colors.as_ptr(),
                    positions.as_ptr(),
                    colors.len() as i32,
                );
                radial as *mut GpBrush
            }
        }
    }
}

/// The color a radial gradient shows outside its end circle.
fn outside(paint: &CanvasPaint) -> Option<Color> {
    match paint {
        CanvasPaint::Radial { stops, .. } => stops
            .iter()
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|stop| stop.1),
        _ => None,
    }
}

/// A GDI+ font family for CSS family names, generic ones standing for the
/// fonts text gets for them; the caller deletes it.
fn family(families: &[String]) -> *mut GpFontFamily {
    let mut family = null_mut();
    let name = crate::fonts::family(families);
    // SAFETY: the name outlives the call; the generic sans-serif family is a
    // shared object that may also be deleted.
    unsafe {
        if GdipCreateFontFamilyFromName(wide(&name).as_ptr(), null_mut(), &mut family) != Ok {
            GdipGetGenericFontFamilySansSerif(&mut family);
        }
    }
    family
}

fn style(font: &Font) -> i32 {
    let mut style = FontStyleRegular;
    if font.weight >= 600 {
        style |= FontStyleBold;
    }
    if font.italic {
        style |= FontStyleItalic;
    }
    style
}

/// A typographic string format that measures trailing spaces; the caller
/// deletes it.
fn format() -> *mut GpStringFormat {
    let (mut generic, mut format) = (null_mut(), null_mut());
    // SAFETY: the clone is new; the generic format is shared.
    unsafe {
        GdipStringFormatGetGenericTypographic(&mut generic);
        GdipCloneStringFormat(generic, &mut format);
        GdipSetStringFormatFlags(format, StringFormatFlagsMeasureTrailingSpaces);
    }
    format
}

/// Text metrics in pixels: width, ascent and descent.
fn measure(
    graphics: *mut GpGraphics,
    family: *mut GpFontFamily,
    font: &Font,
    text: &str,
) -> (f32, f32, f32) {
    let style = style(font);
    let units = wide(text);
    let (mut handle, mut bounds) = (null_mut(), RectF::default());
    let (mut ascent, mut descent, mut em) = (0_u16, 0_u16, 0_u16);
    let format = format();
    // SAFETY: the font and format are created and deleted here.
    unsafe {
        GdipCreateFont(family, font.size, style, UnitPixel, &mut handle);
        GdipMeasureString(
            graphics,
            units.as_ptr(),
            -1,
            handle,
            &RectF::default(),
            format,
            &mut bounds,
            null_mut(),
            null_mut(),
        );
        GdipGetCellAscent(family, style, &mut ascent);
        GdipGetCellDescent(family, style, &mut descent);
        GdipGetEmHeight(family, style, &mut em);
        GdipDeleteFont(handle);
        GdipDeleteStringFormat(format);
    }
    let scale = font.size / f32::from(em.max(1));
    (
        bounds.Width,
        f32::from(ascent) * scale,
        f32::from(descent) * scale,
    )
}

/// Measures canvas text: its width, ascent and descent in pixels.
pub fn measure_text(font: &Font, text: &str) -> (f32, f32, f32) {
    let Some(canvas) = create(1, 1) else {
        return (0.0, 0.0, 0.0);
    };
    let family = family(&font.families);
    let metrics = measure(canvas.graphics, family, font, text);
    // SAFETY: the family was created above.
    unsafe { GdipDeleteFontFamily(family) };
    metrics
}

fn matrix(transform: [f32; 6]) -> *mut Matrix {
    let [a, b, c, d, e, f] = transform;
    let mut matrix = null_mut();
    // SAFETY: the matrix is returned to the caller, who deletes it.
    unsafe { GdipCreateMatrix2(a, b, c, d, e, f, &mut matrix) };
    matrix
}

impl Canvas {
    /// Turns the canvas into a plain image, which the caller then owns.
    fn into_image(self) -> *mut GpImage {
        let canvas = core::mem::ManuallyDrop::new(self);
        // SAFETY: the graphics object is deleted once here; the bitmap goes
        // to the caller.
        unsafe { GdipDeleteGraphics(canvas.graphics) };
        canvas.bitmap as *mut GpImage
    }

    fn compositing(&self, copy: bool) {
        let mode = if copy {
            CompositingModeSourceCopy
        } else {
            CompositingModeSourceOver
        };
        // SAFETY: the graphics object is live.
        unsafe { GdipSetCompositingMode(self.graphics, mode) };
    }

    /// Applies `transform` to the following drawing until it is reset.
    fn transformed(&self, transform: [f32; 6]) {
        let matrix = matrix(transform);
        // SAFETY: the matrix is deleted after GDI+ copied it.
        unsafe {
            GdipSetWorldTransform(self.graphics, matrix);
            GdipDeleteMatrix(matrix);
        }
    }

    fn reset(&self) {
        // SAFETY: the graphics object is live.
        unsafe { GdipResetWorldTransform(self.graphics) };
        self.compositing(false);
    }

    fn fill(
        &self,
        segments: &[PathSegment],
        paint: &CanvasPaint,
        rule: FillRule,
        alpha: f32,
        copy: bool,
    ) {
        let path = path(segments, rule);
        self.compositing(copy);
        // SAFETY: the path and brushes are created and deleted here.
        unsafe {
            if let Some(color) = outside(paint) {
                let pad = brush(&CanvasPaint::Color(color), alpha);
                GdipFillPath(self.graphics, pad, path);
                GdipDeleteBrush(pad);
            }
            let brush = brush(paint, alpha);
            GdipFillPath(self.graphics, brush, path);
            GdipDeleteBrush(brush);
            GdipDeletePath(path);
        }
        self.reset();
    }

    fn stroke(
        &self,
        segments: &[PathSegment],
        paint: &CanvasPaint,
        (width, cap, join, miter): (f32, LineCap, LineJoin, f32),
        (dash, offset): (&[f32], f32),
        (transform, alpha, copy): ([f32; 6], f32, bool),
    ) {
        // The path is in canvas pixels; drawing it in the stroke's own
        // space shapes the pen with the transform.
        let inverse = matrix(transform);
        let path = path(segments, FillRule::NonZero);
        let paint = outside(paint).map_or_else(|| paint.clone(), CanvasPaint::Color);
        // SAFETY: the path, matrix, brush and pen are created and deleted
        // here.
        unsafe {
            if GdipInvertMatrix(inverse) == Ok {
                GdipTransformPath(path, inverse);
                self.transformed(transform);
                self.compositing(copy);
                let brush = brush(&paint, alpha);
                let mut pen = null_mut();
                GdipCreatePen2(brush, width, UnitWorld, &mut pen);
                let cap = match cap {
                    LineCap::Butt => LineCapFlat,
                    LineCap::Round => LineCapRound,
                    LineCap::Square => LineCapSquare,
                };
                GdipSetPenLineCap197819(pen, cap, cap, DashCapFlat);
                let join = match join {
                    LineJoin::Miter => LineJoinMiter,
                    LineJoin::Round => LineJoinRound,
                    LineJoin::Bevel => LineJoinBevel,
                };
                GdipSetPenLineJoin(pen, join);
                GdipSetPenMiterLimit(pen, miter);
                if !dash.is_empty() && width > 0.0 {
                    let relative: Vec<f32> = dash
                        .iter()
                        .map(|length| (length / width).max(0.001))
                        .collect();
                    GdipSetPenDashArray(pen, relative.as_ptr(), relative.len() as i32);
                    GdipSetPenDashOffset(pen, offset / width);
                }
                GdipDrawPath(self.graphics, pen, path);
                GdipDeletePen(pen);
                GdipDeleteBrush(brush);
            }
            GdipDeletePath(path);
            GdipDeleteMatrix(inverse);
        }
        self.reset();
    }

    fn text(
        &self,
        text: &str,
        (x, y, anchor): (f32, f32, TextAnchor),
        font: &Font,
        paint: &CanvasPaint,
        stroke: Option<f32>,
        (transform, alpha): ([f32; 6], f32),
    ) {
        let family = family(&font.families);
        let (width, ascent, _) = measure(self.graphics, family, font, text);
        let left = match anchor {
            TextAnchor::Left => x,
            TextAnchor::Center => x - width / 2.0,
            TextAnchor::Right => x - width,
        };
        let origin = RectF {
            X: left,
            Y: y - ascent,
            Width: 0.0,
            Height: 0.0,
        };
        let units = wide(text);
        let format = format();
        self.transformed(transform);
        let brush = brush(paint, alpha);
        // SAFETY: the font, family, format, path, brush and pen are
        // created and deleted here.
        unsafe {
            match stroke {
                None => {
                    let mut handle = null_mut();
                    GdipCreateFont(family, font.size, style(font), UnitPixel, &mut handle);
                    GdipDrawString(
                        self.graphics,
                        units.as_ptr(),
                        -1,
                        handle,
                        &origin,
                        format,
                        brush,
                    );
                    GdipDeleteFont(handle);
                }
                Some(width) => {
                    let outline = path(&[], FillRule::NonZero);
                    GdipAddPathString(
                        outline,
                        units.as_ptr(),
                        -1,
                        family,
                        style(font),
                        font.size,
                        &origin,
                        format,
                    );
                    let mut pen = null_mut();
                    GdipCreatePen2(brush, width, UnitWorld, &mut pen);
                    GdipDrawPath(self.graphics, pen, outline);
                    GdipDeletePen(pen);
                    GdipDeletePath(outline);
                }
            }
            GdipDeleteBrush(brush);
            GdipDeleteStringFormat(format);
            GdipDeleteFontFamily(family);
        }
        self.reset();
    }

    fn image(&self, image: *mut GpImage, from: Rect, to: Rect, transform: [f32; 6], alpha: f32) {
        self.transformed(transform);
        let mut attributes = faded(alpha);
        // SAFETY: the image is live; the attributes are disposed here.
        unsafe {
            if attributes.is_null() {
                GdipCreateImageAttributes(&mut attributes);
            }
            // Scaled images keep their edge pixels instead of fading out.
            GdipSetImageAttributesWrapMode(attributes, WrapModeTileFlipXY, 0, 0);
            GdipDrawImageRectRect(
                self.graphics,
                image,
                to.x,
                to.y,
                to.width,
                to.height,
                from.x,
                from.y,
                from.width,
                from.height,
                UnitPixel,
                attributes,
                0,
                null_mut(),
            );
            GdipDisposeImageAttributes(attributes);
        }
        self.reset();
    }

    fn clip(&self, segments: &[PathSegment], rule: FillRule) {
        let path = path(segments, rule);
        // SAFETY: the path is created and deleted here.
        unsafe {
            GdipSetClipPath(self.graphics, path, CombineModeIntersect);
            GdipDeletePath(path);
        }
    }

    fn command(&mut self, key: u64, command: &CanvasCommand) {
        match command {
            CanvasCommand::Fill {
                path,
                paint,
                rule,
                alpha,
                copy,
            } => self.fill(path, paint, *rule, *alpha, *copy),
            CanvasCommand::Stroke {
                path,
                paint,
                width,
                cap,
                join,
                miter,
                dash,
                offset,
                transform,
                alpha,
                copy,
            } => self.stroke(
                path,
                paint,
                (*width, *cap, *join, *miter),
                (dash, *offset),
                (*transform, *alpha, *copy),
            ),
            CanvasCommand::Clear { path } => self.fill(
                path,
                &CanvasPaint::Color(Color::TRANSPARENT),
                FillRule::NonZero,
                1.0,
                true,
            ),
            CanvasCommand::Text {
                text,
                x,
                y,
                anchor,
                font,
                paint,
                stroke,
                transform,
                alpha,
            } => self.text(
                text,
                (*x, *y, *anchor),
                font,
                paint,
                *stroke,
                (*transform, *alpha),
            ),
            CanvasCommand::Image {
                source,
                from,
                to,
                transform,
                alpha,
            } => {
                let image = match source {
                    CanvasSource::Url(url) => image_handle(url).map(|(image, _, _)| image),
                    CanvasSource::Canvas(other) if *other != key => {
                        bitmap(*other).map(|(image, _, _)| image)
                    }
                    CanvasSource::Canvas(_) => None,
                };
                if let Some(image) = image {
                    self.image(image, *from, *to, *transform, *alpha);
                }
            }
            CanvasCommand::Save => {
                let mut state = 0;
                // SAFETY: the graphics object is live.
                unsafe { GdipSaveGraphics(self.graphics, &mut state) };
                self.states.push(state);
            }
            CanvasCommand::Restore => {
                if let Some(state) = self.states.pop() {
                    // SAFETY: the state was saved on this graphics object.
                    unsafe { GdipRestoreGraphics(self.graphics, state) };
                }
            }
            CanvasCommand::Clip { path, rule } => self.clip(path, *rule),
        }
    }
}

/// Draws commands into the bitmap of canvas `key`. The bitmap is created,
/// or recreated transparent, when it does not have the given size.
pub fn canvas_draw(key: u64, width: u32, height: u32, commands: &[CanvasCommand]) {
    let current = bitmap(key).map(|(_, width, height)| (width, height));
    if current != Some((width, height)) {
        let canvas = create(width, height);
        CANVASES.with_borrow_mut(|canvases| match canvas {
            Some(canvas) => canvases.insert(key, canvas),
            None => canvases.remove(&key),
        });
    }
    // Drawing another canvas reads the map, so take this one out meanwhile.
    let Some(mut canvas) = CANVASES.with_borrow_mut(|canvases| canvases.remove(&key)) else {
        return;
    };
    for command in commands {
        canvas.command(key, command);
    }
    // SAFETY: the graphics object is live.
    unsafe { GdipFlush(canvas.graphics, FlushIntentionFlush) };
    CANVASES.with_borrow_mut(|canvases| canvases.insert(key, canvas));
}

/// Copies pixels between a canvas and RGBA bytes (unpremultiplied, row by
/// row): reads them into `pixels`, or writes `pixels` into the canvas.
fn transfer(key: u64, area: (i32, i32, u32, u32), pixels: &mut [u8], write: bool) {
    if let Some((image, width, height)) = bitmap(key) {
        transfer_image(image, (width, height), area, pixels, write);
    }
}

/// Copies pixels between a bitmap of the given size and RGBA bytes.
fn transfer_image(
    image: *mut GpImage,
    (width, height): (u32, u32),
    area: (i32, i32, u32, u32),
    pixels: &mut [u8],
    write: bool,
) {
    let (x, y, area_width, area_height) = area;
    let left = x.max(0);
    let top = y.max(0);
    let right = (x + area_width as i32).min(width as i32);
    let bottom = (y + area_height as i32).min(height as i32);
    if left >= right || top >= bottom {
        return;
    }
    let rect = windows_sys::Win32::Graphics::GdiPlus::Rect {
        X: left,
        Y: top,
        Width: right - left,
        Height: bottom - top,
    };
    let mut data = BitmapData::default();
    let mode = if write {
        ImageLockModeWrite
    } else {
        ImageLockModeRead
    };
    // SAFETY: the locked rectangle lies inside the bitmap; rows are read
    // and written within the stride GDI+ reports.
    unsafe {
        if GdipBitmapLockBits(image as *mut GpBitmap, &rect, mode as u32, ARGB, &mut data) != Ok {
            return;
        }
        for row in 0..rect.Height {
            let line = (data.Scan0 as *mut u8).offset(row as isize * data.Stride as isize);
            for column in 0..rect.Width {
                let source = line.add(column as usize * 4);
                let target = (((top + row - y) as usize) * area_width as usize
                    + (left + column - x) as usize)
                    * 4;
                let pixel = &mut pixels[target..target + 4];
                if write {
                    *source = pixel[2];
                    *source.add(1) = pixel[1];
                    *source.add(2) = pixel[0];
                    *source.add(3) = pixel[3];
                } else {
                    pixel.copy_from_slice(&[
                        *source.add(2),
                        *source.add(1),
                        *source,
                        *source.add(3),
                    ]);
                }
            }
        }
        GdipBitmapUnlockBits(image as *mut GpBitmap, &mut data);
    }
}

/// The RGBA pixels of a rectangle of canvas `key`; pixels outside the
/// bitmap are transparent black.
pub fn canvas_read(key: u64, x: i32, y: i32, width: u32, height: u32) -> Vec<u8> {
    let mut pixels = vec![0; width as usize * height as usize * 4];
    transfer(key, (x, y, width, height), &mut pixels, false);
    pixels
}

/// Replaces the pixels of a rectangle of canvas `key` with RGBA bytes.
pub fn canvas_write(key: u64, x: i32, y: i32, width: u32, height: u32, pixels: &[u8]) {
    let mut pixels = pixels.to_vec();
    pixels.resize(width as usize * height as usize * 4, 0);
    transfer(key, (x, y, width, height), &mut pixels, true);
}

/// The start of `IStream`'s method table, up to `Seek`.
#[repr(C)]
struct StreamVtbl {
    base: IUnknown_Vtbl,
    read: usize,
    write: usize,
    seek: unsafe extern "system" fn(*mut core::ffi::c_void, i64, u32, *mut u64) -> HRESULT,
}

/// `STREAM_SEEK_END`.
const SEEK_END: u32 = 2;

/// Encodes canvas `key` as a PNG file, or a JPEG file when `jpeg` is set.
pub fn canvas_encode(key: u64, jpeg: bool) -> Option<Vec<u8>> {
    encode(bitmap(key)?.0, jpeg)
}

/// Encodes an image added with [`add_image`](crate::add_image) as a PNG or
/// JPEG file.
pub fn image_encode(url: &str, jpeg: bool) -> Option<Vec<u8>> {
    encode(image_handle(url)?.0, jpeg)
}

/// The width, height and RGBA pixels of an image.
pub fn image_pixels(url: &str) -> Option<(u32, u32, Vec<u8>)> {
    let (image, width, height) = image_handle(url)?;
    let mut pixels = vec![0; width as usize * height as usize * 4];
    transfer_image(
        image,
        (width, height),
        (0, 0, width, height),
        &mut pixels,
        false,
    );
    Some((width, height, pixels))
}

/// Draws the `from` part of image `source` scaled to `size` as a new image
/// under `url`; returns its size.
pub fn image_draw(source: &str, from: Rect, size: (u32, u32), url: &str) -> Option<(f32, f32)> {
    let (image, _, _) = image_handle(source)?;
    let canvas = create(size.0, size.1)?;
    let to = Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32);
    if (from.width, from.height) == (to.width, to.height) {
        // A crop copies pixels exactly.
        // SAFETY: the graphics object is live.
        unsafe { GdipSetInterpolationMode(canvas.graphics, InterpolationModeNearestNeighbor) };
    }
    canvas.image(image, from, to, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 1.0);
    Some(insert_image(url, canvas.into_image()))
}

/// Makes an image of the given size from RGBA pixels under `url`.
pub fn image_from_pixels(
    url: &str,
    (width, height): (u32, u32),
    pixels: &[u8],
) -> Option<(f32, f32)> {
    let canvas = create(width, height)?;
    let mut data = pixels.to_vec();
    data.resize(width as usize * height as usize * 4, 0);
    let image = canvas.bitmap as *mut GpImage;
    transfer_image(
        image,
        (width, height),
        (0, 0, width, height),
        &mut data,
        true,
    );
    Some(insert_image(url, canvas.into_image()))
}

/// Encodes an image as a PNG or JPEG file.
fn encode(image: *mut GpImage, jpeg: bool) -> Option<Vec<u8>> {
    let mut stream = null_mut();
    // SAFETY: the stream owns its memory, which is copied out before the
    // stream is released.
    unsafe {
        if CreateStreamOnHGlobal(null_mut(), 1, &mut stream) < 0 {
            return None;
        }
        let encoder = if jpeg { JPEG } else { PNG };
        let saved = GdipSaveImageToStream(image, stream, &encoder, null());
        let table = *(stream as *mut *const StreamVtbl);
        let mut size = 0_u64;
        ((*table).seek)(stream, 0, SEEK_END, &mut size);
        let mut memory: HGLOBAL = null_mut();
        GetHGlobalFromStream(stream, &mut memory);
        let bytes = (saved == Ok).then(|| {
            let data = GlobalLock(memory) as *const u8;
            let bytes = core::slice::from_raw_parts(data, size as usize).to_vec();
            GlobalUnlock(memory);
            bytes
        });
        ((*table).base.Release)(stream);
        bytes
    }
}

/// Frees the bitmap of canvas `key`. Pages can be dropped while the thread
/// exits, after the bitmaps are gone.
pub fn canvas_release(key: u64) {
    let _ = CANVASES.try_with(|canvases| canvases.borrow_mut().remove(&key));
}

#[cfg(test)]
mod tests;
