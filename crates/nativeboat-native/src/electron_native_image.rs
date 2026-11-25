//! Electron's `nativeImage` as Electron 38 behaves on Windows. An image has
//! representations, each a picture at a scale factor, kept as Skia keeps
//! pixels: premultiplied BGRA. Images come from PNG and JPEG files, buffers
//! and data URLs, raw bitmaps, `.ico` files, Explorer's thumbnails and the
//! clipboard; they are resized and cropped as Chromium does, and written as
//! PNG, JPEG, raw bitmaps and data URLs.

mod resize;
mod windows;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::prelude::*;
use crate::{Completion, ErrorKind, Key, Kind, Object, Property, Realm, Slot, Thrown, Value, ops};
use resize::Method;

/// A native function without captures.
type Native = fn(&mut Realm, Value, &[Value]) -> Completion;

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const DIBV5_HEADER_SIZE: usize = 124;
/// The size Electron loads `.ico` files at, scaling them as `LoadImage`
/// does.
const ICON_SIZE: i32 = 256;
/// The suffixes of the files holding an image's other scale factors, in
/// the order Electron looks for them.
const SCALE_SUFFIXES: [(&str, f32); 11] = [
    ("@2x", 2.0),
    ("@3x", 3.0),
    ("@1x", 1.0),
    ("@4x", 4.0),
    ("@5x", 5.0),
    ("@1.25x", 1.25),
    ("@1.33x", 1.33),
    ("@1.4x", 1.4),
    ("@1.5x", 1.5),
    ("@1.8x", 1.8),
    ("@2.5x", 2.5),
];
/// The methods every image has as its own properties, in Electron's
/// order; `isMacTemplateImage` is an accessor among them.
const METHODS: [&str; 16] = [
    "toPNG",
    "toJPEG",
    "toBitmap",
    "getBitmap",
    "getScaleFactors",
    "getNativeHandle",
    "toDataURL",
    "isEmpty",
    "getSize",
    "setTemplateImage",
    "isTemplateImage",
    "isMacTemplateImage",
    "resize",
    "crop",
    "getAspectRatio",
    "addRepresentation",
];

struct PngImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    /// Whether the file has no alpha, which Skia keeps as opaque.
    opaque: bool,
}

/// One representation of an image: a picture at a scale factor.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Rep {
    scale: f32,
    width: u32,
    height: u32,
    /// Premultiplied BGRA, row after row.
    pixels: Vec<u8>,
    /// Whether every pixel is opaque by the picture's kind, as Skia's
    /// opaque alpha type: PNG files without alpha and JPEG files.
    opaque: bool,
}

impl Rep {
    fn from_straight(scale: f32, image: PngImage) -> Rep {
        let pixels = image
            .rgba
            .chunks_exact(4)
            .flat_map(|pixel| {
                let alpha = pixel[3];
                [
                    premultiply(pixel[2], alpha),
                    premultiply(pixel[1], alpha),
                    premultiply(pixel[0], alpha),
                    alpha,
                ]
            })
            .collect();
        Rep {
            scale,
            width: image.width,
            height: image.height,
            pixels,
            opaque: image.opaque,
        }
    }

    fn from_pixels(scale: f32, (width, height, pixels): windows::Pixels, opaque: bool) -> Rep {
        Rep {
            scale,
            width,
            height,
            pixels,
            opaque,
        }
    }

    /// The picture's straight RGBA pixels, unpremultiplied as Skia's
    /// raster pipeline does.
    fn straight(&self) -> Vec<u8> {
        self.pixels
            .chunks_exact(4)
            .flat_map(|pixel| {
                let alpha = pixel[3];
                [
                    unpremultiply(pixel[2], alpha),
                    unpremultiply(pixel[1], alpha),
                    unpremultiply(pixel[0], alpha),
                    alpha,
                ]
            })
            .collect()
    }

    /// Its size in device-independent pixels, truncated.
    fn size(&self) -> (i32, i32) {
        (
            (self.width as f32 / self.scale) as i32,
            (self.height as f32 / self.scale) as i32,
        )
    }

    fn png(&self) -> Vec<u8> {
        if self.width == 0 || self.height == 0 {
            return Vec::new();
        }
        png_file(self.width, self.height, &self.straight(), self.opaque)
    }

    /// Kept in an image object: the scale, size and opacity, then the
    /// pixels.
    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(13 + self.pixels.len());
        bytes.extend(self.scale.to_le_bytes());
        bytes.extend(self.width.to_le_bytes());
        bytes.extend(self.height.to_le_bytes());
        bytes.push(u8::from(self.opaque));
        bytes.extend_from_slice(&self.pixels);
        bytes
    }

    fn decode(bytes: &[u8]) -> Option<Rep> {
        let word = |at: usize| -> Option<[u8; 4]> { bytes.get(at..at + 4)?.try_into().ok() };
        Some(Rep {
            scale: f32::from_le_bytes(word(0)?),
            width: u32::from_le_bytes(word(4)?),
            height: u32::from_le_bytes(word(8)?),
            opaque: *bytes.get(12)? == 1,
            pixels: bytes.get(13..)?.to_vec(),
        })
    }
}

/// Skia's `SkMulDiv255Round`.
fn premultiply(colour: u8, alpha: u8) -> u8 {
    let product = u32::from(colour) * u32::from(alpha) + 128;
    ((product + (product >> 8)) >> 8) as u8
}

/// Skia's raster pipeline: channels as floats in 0..1, scaled by the
/// reciprocal of alpha, clamped, and rounded half to even.
fn unpremultiply(colour: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    let scale = 1.0 / (f32::from(alpha) * (1.0 / 255.0));
    let value = (f32::from(colour) * (1.0 / 255.0) * scale).clamp(0.0, 1.0) * 255.0;
    let whole = value as u32;
    let fraction = value - whole as f32;
    let rounded = if fraction > 0.5 || (fraction == 0.5 && whole % 2 == 1) {
        whole + 1
    } else {
        whole
    };
    rounded as u8
}

/// The representation nearest a scale factor, the first of equally near
/// ones, as `ImageSkia` finds it.
fn closest(reps: &[Rep], scale: f32) -> Option<&Rep> {
    let mut best: Option<&Rep> = None;
    for rep in reps {
        let distance = (rep.scale - scale).abs();
        if best.is_none_or(|best| distance < (best.scale - scale).abs()) {
            best = Some(rep);
        }
    }
    best
}

/// The representations of an image object; `None` for an empty image, or
/// for a value that is not one.
fn reps_of(realm: &Realm, value: &Value) -> Option<Vec<Rep>> {
    let Value::Object(object) = value else {
        return None;
    };
    let Slot::Data {
        value: Value::Object(list),
        ..
    } = object.private_property(&realm.electron_image_key)?.slot
    else {
        return None;
    };
    let mut reps = Vec::new();
    let mut index = 0;
    while let Some(Property {
        slot: Slot::Data { value, .. },
        ..
    }) = list.own_property(&Key::Index(index))
    {
        reps.extend(
            crate::typed_array_builtin::view_bytes(&value).and_then(|bytes| Rep::decode(&bytes)),
        );
        index += 1;
    }
    Some(reps)
}

/// The functions and prototype images share.
#[derive(Clone, Debug)]
pub(crate) struct ImageClass {
    prototype: Object,
    methods: Vec<Object>,
    mac_template: (Object, Object),
}

fn image_class(realm: &mut Realm) -> ImageClass {
    if let Some(class) = &realm.electron_images {
        return class.clone();
    }
    let prototype = realm.object();
    realm.builtin_constructor(
        "NativeImage",
        0,
        &prototype,
        |_, _, _| Ok(Value::Undefined),
        |realm, _, _| Ok(image_object(realm, None)),
    );
    let methods = METHODS
        .iter()
        .filter(|name| **name != "isMacTemplateImage")
        .map(|name| method(realm, name))
        .collect();
    let mac_template = (
        realm.function("get isMacTemplateImage", 0, false, |_, _, _| {
            Ok(Value::Boolean(false))
        }),
        realm.function("set isMacTemplateImage", 1, false, |_, _, _| {
            Ok(Value::Undefined)
        }),
    );
    let class = ImageClass {
        prototype,
        methods,
        mac_template,
    };
    realm.electron_images = Some(class.clone());
    class
}

/// A new image object with `reps`, or an empty image.
/// An empty `NativeImage`, as `nativeImage.createEmpty()` makes.
pub(crate) fn empty(realm: &mut Realm) -> Object {
    image_object(realm, None)
}

fn image_object(realm: &mut Realm, reps: Option<Vec<Rep>>) -> Object {
    let class = image_class(realm);
    let image = Object::new(Some(class.prototype), Kind::Ordinary);
    let mut methods = class.methods.into_iter();
    for name in METHODS {
        if name == "isMacTemplateImage" {
            let (get, set) = class.mac_template.clone();
            image.define(
                Key::name(name),
                Property {
                    slot: Slot::Accessor {
                        get: Some(get),
                        set: Some(set),
                    },
                    enumerable: true,
                    configurable: true,
                },
            );
        } else if let Some(function) = methods.next() {
            image.define(Key::name(name), Property::data(Value::Object(function)));
        }
    }
    if let Some(reps) = reps {
        let list = reps
            .iter()
            .map(|rep| {
                Value::Object(crate::typed_array_builtin::new_bytes(
                    realm,
                    rep.encode(),
                    None,
                ))
            })
            .collect();
        image.set_private(
            &realm.electron_image_key,
            Property::hidden(Value::Object(realm.array(list))),
        );
    }
    image
}

fn insufficient(realm: &Realm) -> Thrown {
    realm.throw(ErrorKind::Type, "Insufficient number of arguments.")
}

/// gin's `IsInt32`: an integral number in range, not negative zero.
fn int32(value: &Value) -> Option<i32> {
    match value {
        Value::Number(number)
            if *number == (*number as i32) as f64
                && !(*number == 0.0 && number.is_sign_negative()) =>
        {
            Some(*number as i32)
        }
        _ => None,
    }
}

/// gin's `IsUint32`.
fn uint32(value: &Value) -> Option<u32> {
    match value {
        Value::Number(number)
            if *number == (*number as u32) as f64
                && !(*number == 0.0 && number.is_sign_negative()) =>
        {
            Some(*number as u32)
        }
        _ => None,
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => Some(*number),
        _ => None,
    }
}

/// An options argument, as gin converts a dictionary: an object, or a
/// conversion error naming the value.
fn dictionary(realm: &Realm, args: &[Value], index: usize) -> Result<Value, Thrown> {
    match args.get(index) {
        None => Err(insufficient(realm)),
        Some(value @ Value::Object(_)) => Ok(value.clone()),
        Some(value) => Err(crate::electron_builtin::conversion_failure(
            realm, index, value,
        )),
    }
}

/// An option of a dictionary, read as gin reads one: through getters.
fn option(realm: &mut Realm, options: &Value, name: &'static str) -> Result<Value, Thrown> {
    ops::get_named(realm, options, name)
}

/// The scale factor of a `{ scaleFactor }` options argument, 1 without
/// one.
fn scale_option(realm: &mut Realm, args: &[Value]) -> Result<f32, Thrown> {
    match args.first() {
        Some(options @ Value::Object(_)) => {
            Ok(number(&option(realm, options, "scaleFactor")?).map_or(1.0, |scale| scale as f32))
        }
        _ => Ok(1.0),
    }
}

/// A Node buffer's bytes: any `ArrayBufferView`, as `Buffer::HasInstance`
/// allows.
fn buffer_bytes(realm: &Realm, value: &Value) -> Result<Vec<u8>, Thrown> {
    crate::typed_array_builtin::view_bytes(value)
        .ok_or_else(|| realm.throw(ErrorKind::Error, "buffer must be a node Buffer"))
}

/// A representation from encoded bytes, as Electron reads buffers: PNG,
/// then JPEG, then, given a size, premultiplied BGRA pixels.
fn rep_from_buffer(bytes: &[u8], (width, height): (i32, i32), scale: f32) -> Option<Rep> {
    if let Ok(image) = decode_png(bytes) {
        return Some(Rep::from_straight(scale, image));
    }
    if let Some(pixels) = windows::decode_jpeg(bytes) {
        return Some(Rep::from_pixels(scale, pixels, true));
    }
    if width <= 0 || height <= 0 {
        return None;
    }
    let length = width as usize * height as usize * 4;
    (bytes.len() >= length).then(|| Rep {
        scale,
        width: width as u32,
        height: height as u32,
        pixels: bytes[..length].to_vec(),
        opaque: false,
    })
}

/// The bytes and media type of a `data:` URL, as Chromium's `DataURL`
/// parses it.
fn data_url(url: &str) -> Option<(String, Vec<u8>)> {
    let (scheme, rest) = url.split_once(':')?;
    if !scheme.trim_start().eq_ignore_ascii_case("data") {
        return None;
    }
    let (header, body) = rest.split_once(',')?;
    let mut parts = header.split(';');
    let media = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let base64 = parts.any(|part| part.trim().eq_ignore_ascii_case("base64"));
    let decoded = percent_decoded(body.as_bytes());
    let bytes = if base64 {
        let units: Vec<u16> = decoded
            .iter()
            .filter(|byte| !byte.is_ascii_whitespace())
            .map(|byte| u16::from(*byte))
            .collect();
        crate::encoding::forgiving_base64(&units).ok()?
    } else {
        decoded
    };
    Some((media, bytes))
}

/// Bytes with `%XX` escapes decoded; other `%` signs are kept.
fn percent_decoded(text: &[u8]) -> Vec<u8> {
    let digit = |at: usize| {
        text.get(at)
            .and_then(|digit| char::from(*digit).to_digit(16))
    };
    let mut bytes = Vec::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        if text[index] == b'%'
            && let (Some(high), Some(low)) = (digit(index + 1), digit(index + 2))
        {
            bytes.push((high * 16 + low) as u8);
            index += 3;
        } else {
            bytes.push(text[index]);
            index += 1;
        }
    }
    bytes
}

/// A representation from a data URL: PNG or JPEG by its media type.
fn rep_from_data_url(url: &str, scale: f32) -> Option<Rep> {
    let (media, bytes) = data_url(url)?;
    match media.as_str() {
        "image/png" => decode_png(&bytes)
            .ok()
            .map(|image| Rep::from_straight(scale, image)),
        "image/jpeg" => {
            windows::decode_jpeg(&bytes).map(|pixels| Rep::from_pixels(scale, pixels, true))
        }
        _ => None,
    }
}

/// A file name's scale factor suffix, such as `@2x`; whether the name has
/// one at all.
fn suffix_scale(path: &str) -> Option<f32> {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    // Electron matches `*@*x`.
    let at = stem.find('@')?;
    if !stem[at + 1..].ends_with('x') {
        return None;
    }
    Some(
        SCALE_SUFFIXES
            .iter()
            .find(|(suffix, _)| stem.ends_with(suffix))
            .map_or(1.0, |(_, scale)| *scale),
    )
}

fn with_suffix(path: &str, suffix: &str) -> String {
    let name_start = path.rfind(['\\', '/']).map_or(0, |at| at + 1);
    match path[name_start..].rfind('.') {
        Some(dot) => {
            let dot = name_start + dot;
            alloc::format!("{}{suffix}{}", &path[..dot], &path[dot..])
        }
        None => alloc::format!("{path}{suffix}"),
    }
}

fn rep_from_file(path: &str, scale: f32) -> Option<Rep> {
    rep_from_buffer(&crate::platform::read_file(path)?, (0, 0), scale)
}

/// An image file's representations, as `createFromPath` reads them: an
/// icon file at 256 pixels, or the file and its siblings with scale factor
/// suffixes.
fn reps_from_path(path: &str) -> Vec<Rep> {
    let extension = path.rsplit_once('.').map(|(_, extension)| extension);
    if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("ico")) {
        return windows::load_icon(path, ICON_SIZE)
            .map(|pixels| Rep::from_pixels(1.0, pixels, false))
            .into_iter()
            .collect();
    }
    if let Some(scale) = suffix_scale(path) {
        return rep_from_file(path, scale).into_iter().collect();
    }
    let mut reps: Vec<Rep> = rep_from_file(path, 1.0).into_iter().collect();
    for (suffix, scale) in SCALE_SUFFIXES {
        reps.extend(rep_from_file(&with_suffix(path, suffix), scale));
    }
    reps
}

pub(crate) fn module(realm: &Realm) -> Object {
    let native_image = realm.object();
    let define = |name: &'static str, code: Native| {
        let function = realm.function("", 0, false, code);
        native_image.define(Key::name(name), Property::data(Value::Object(function)));
    };
    define("createEmpty", |realm, _, _| {
        Ok(Value::Object(image_object(realm, None)))
    });
    define("createFromPath", create_from_path);
    define("createFromBitmap", create_from_bitmap);
    define("createFromBuffer", create_from_buffer);
    define("createFromDataURL", |realm, _, args| {
        let url = match args.first() {
            None => return Err(insufficient(realm)),
            Some(Value::String(url)) => url.to_rust_lossy(),
            Some(value) => {
                return Err(crate::electron_builtin::conversion_failure(realm, 0, value));
            }
        };
        let reps = rep_from_data_url(&url, 1.0).map(|rep| vec![rep]);
        Ok(Value::Object(image_object(realm, reps)))
    });
    // macOS's named images: Electron on Windows has none.
    define("createFromNamedImage", |realm, _, args| {
        match args.first() {
            None => Err(insufficient(realm)),
            Some(Value::String(_)) => Ok(Value::Object(image_object(realm, None))),
            Some(value) => Err(crate::electron_builtin::conversion_failure(realm, 0, value)),
        }
    });
    define("createThumbnailFromPath", create_thumbnail_from_path);
    native_image
}

fn create_from_path(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let path = match args.first() {
        None => return Err(insufficient(realm)),
        Some(Value::String(path)) => path.to_rust_lossy(),
        Some(value) => return Err(crate::electron_builtin::conversion_failure(realm, 0, value)),
    };
    let reps = reps_from_path(&path);
    let reps = (!reps.is_empty()).then_some(reps);
    Ok(Value::Object(image_object(realm, reps)))
}

/// `createFromBitmap(buffer, { width, height, scaleFactor })`: exactly
/// `width × height` premultiplied BGRA pixels.
fn create_from_bitmap(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let options = dictionary(realm, args, 1)?;
    let bytes = buffer_bytes(realm, &args[0])?;
    let width = uint32(&option(realm, &options, "width")?)
        .ok_or_else(|| realm.throw(ErrorKind::Error, "width is required"))?;
    let height = uint32(&option(realm, &options, "height")?)
        .ok_or_else(|| realm.throw(ErrorKind::Error, "height is required"))?;
    if bytes.len() as u64 != u64::from(width) * u64::from(height) * 4 {
        return Err(realm.throw(ErrorKind::Error, "invalid buffer size"));
    }
    let scale = number(&option(realm, &options, "scaleFactor")?).map_or(1.0, |scale| scale as f32);
    if width == 0 || height == 0 {
        return Ok(Value::Object(image_object(realm, None)));
    }
    let rep = Rep {
        scale,
        width,
        height,
        pixels: bytes,
        opaque: false,
    };
    Ok(Value::Object(image_object(realm, Some(vec![rep]))))
}

/// `createFromBuffer(buffer, { width, height, scaleFactor })`: PNG or JPEG
/// bytes, or premultiplied BGRA pixels of the size given.
fn create_from_buffer(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    let Some(buffer) = args.first() else {
        return Err(insufficient(realm));
    };
    let bytes = buffer_bytes(realm, buffer)?;
    let (mut size, mut scale) = ((0, 0), 1.0);
    if let Some(options @ Value::Object(_)) = args.get(1) {
        size.0 = int32(&option(realm, options, "width")?).unwrap_or(0);
        size.1 = int32(&option(realm, options, "height")?).unwrap_or(0);
        scale = number(&option(realm, options, "scaleFactor")?).map_or(1.0, |scale| scale as f32);
    }
    let reps = rep_from_buffer(&bytes, size, scale).map(|rep| vec![rep]);
    Ok(Value::Object(image_object(realm, reps)))
}

/// `createThumbnailFromPath(path, { width, height })`: a promise of the
/// thumbnail Explorer shows, at most `width` pixels across.
fn create_thumbnail_from_path(realm: &mut Realm, _: Value, args: &[Value]) -> Completion {
    // Electron gives nothing back for a path that is not a string.
    let Some(Value::String(path)) = args.first() else {
        return Ok(Value::Undefined);
    };
    let path = path.to_rust_lossy();
    let (promise, resolve, reject) = crate::promise_builtin::capability(realm);
    let size = match args.get(1) {
        Some(options @ Value::Object(_)) => {
            let width = int32(&option(realm, options, "width")?);
            let height = int32(&option(realm, options, "height")?);
            width.zip(height)
        }
        _ => None,
    };
    let outcome = match size {
        None => {
            let value = args.get(1).cloned().unwrap_or(Value::Undefined);
            Err(crate::electron_builtin::conversion_failure(realm, 1, &value).0)
        }
        Some((width, height)) if width <= 0 || height <= 0 => Err(Value::Object(
            realm.error(ErrorKind::Error, "size must not be empty"),
        )),
        Some((width, _)) => match windows::thumbnail(&path, width as u32) {
            Ok(pixels) => {
                let reps = (pixels.0 > 0 && pixels.1 > 0)
                    .then(|| vec![Rep::from_pixels(1.0, pixels, false)]);
                Ok(Value::Object(image_object(realm, reps)))
            }
            Err(message) => Err(Value::Object(realm.error(ErrorKind::Error, message))),
        },
    };
    match outcome {
        Ok(image) => ops::call(realm, &resolve, Value::Undefined, &[image])?,
        Err(error) => ops::call(realm, &reject, Value::Undefined, &[error])?,
    };
    Ok(promise)
}

fn method(realm: &Realm, name: &'static str) -> Object {
    let code: Native = match name {
        "toPNG" => |realm, this, args| {
            let scale = scale_option(realm, args)?;
            let reps = reps_of(realm, &this).unwrap_or_default();
            let bytes = closest(&reps, scale).map(Rep::png).unwrap_or_default();
            crate::electron_clipboard::buffer_value(realm, bytes)
        },
        "toJPEG" => |realm, this, args| {
            let quality = match args.first() {
                None => return Err(insufficient(realm)),
                Some(value) => int32(value)
                    .ok_or_else(|| crate::electron_builtin::conversion_failure(realm, 0, value))?,
            };
            let reps = reps_of(realm, &this).unwrap_or_default();
            // Only a picture at scale factor 1 is encoded.
            let bytes = closest(&reps, 1.0)
                .filter(|rep| rep.scale == 1.0 && rep.width > 0 && rep.height > 0)
                .and_then(|rep| {
                    windows::encode_jpeg(
                        rep.width,
                        rep.height,
                        &rep.pixels,
                        quality.clamp(0, 100) as u32,
                    )
                })
                .unwrap_or_default();
            crate::electron_clipboard::buffer_value(realm, bytes)
        },
        "toBitmap" => |realm, this, args| bitmap(realm, &this, args),
        "getBitmap" => |realm, this, args| {
            if !realm.electron_image_bitmap_warned {
                realm.electron_image_bitmap_warned = true;
                let process =
                    ops::get_named(realm, &Value::Object(realm.global.clone()), "process")?;
                if let Value::Object(_) = process
                    && let emit @ Value::Object(_) = ops::get_named(realm, &process, "emitWarning")?
                {
                    ops::call(
                        realm,
                        &emit,
                        process,
                        &[
                            Value::str("getBitmap() is deprecated, use toBitmap() instead."),
                            Value::str("DeprecationWarning"),
                        ],
                    )?;
                }
            }
            bitmap(realm, &this, args)
        },
        "getScaleFactors" => |realm, this, _| {
            let scales = reps_of(realm, &this)
                .unwrap_or_default()
                .iter()
                .map(|rep| Value::Number(f64::from(rep.scale)))
                .collect();
            Ok(Value::Object(realm.array(scales)))
        },
        "getNativeHandle" => |realm, _, _| Err(realm.throw(ErrorKind::Error, "Not implemented")),
        "toDataURL" => |realm, this, args| {
            let scale = scale_option(realm, args)?;
            let reps = reps_of(realm, &this).unwrap_or_default();
            let bytes = closest(&reps, scale).map(Rep::png).unwrap_or_default();
            Ok(Value::str(&alloc::format!(
                "data:image/png;base64,{}",
                crate::encoding::base64_encode(&bytes, false)
            )))
        },
        "isEmpty" => |realm, this, _| Ok(Value::Boolean(reps_of(realm, &this).is_none())),
        "getSize" => |realm, this, args| {
            let scale = args
                .first()
                .and_then(number)
                .map_or(1.0, |scale| scale as f32);
            let (width, height) = image_size(realm, &this, scale);
            let size = realm.object();
            size.define(
                Key::name("width"),
                Property::data(Value::Number(f64::from(width))),
            );
            size.define(
                Key::name("height"),
                Property::data(Value::Number(f64::from(height))),
            );
            Ok(Value::Object(size))
        },
        // macOS's template images: Electron on Windows keeps none.
        "setTemplateImage" => |realm, _, args| match args.first() {
            None => Err(insufficient(realm)),
            Some(Value::Boolean(_)) => Ok(Value::Undefined),
            Some(value) => Err(crate::electron_builtin::conversion_failure(realm, 0, value)),
        },
        "isTemplateImage" => |_, _, _| Ok(Value::Boolean(false)),
        "resize" => resize_image,
        "crop" => crop_image,
        "getAspectRatio" => |realm, this, args| {
            let scale = args
                .first()
                .and_then(number)
                .map_or(1.0, |scale| scale as f32);
            Ok(Value::Number(f64::from(aspect_ratio(image_size(
                realm, &this, scale,
            )))))
        },
        _ => add_representation,
    };
    realm.function(name, 0, false, code)
}

/// `toBitmap({ scaleFactor })`: the premultiplied BGRA pixels of the
/// representation nearest the scale factor.
fn bitmap(realm: &mut Realm, this: &Value, args: &[Value]) -> Completion {
    let scale = scale_option(realm, args)?;
    let reps = reps_of(realm, this).unwrap_or_default();
    let bytes = closest(&reps, scale)
        .map(|rep| rep.pixels.clone())
        .unwrap_or_default();
    crate::electron_clipboard::buffer_value(realm, bytes)
}

/// The size of the representation nearest a scale factor, in
/// device-independent pixels.
fn image_size(realm: &Realm, this: &Value, scale: f32) -> (i32, i32) {
    let reps = reps_of(realm, this).unwrap_or_default();
    closest(&reps, scale).map_or((0, 0), Rep::size)
}

fn aspect_ratio((width, height): (i32, i32)) -> f32 {
    if width <= 0 || height <= 0 {
        1.0
    } else {
        width as f32 / height as f32
    }
}

/// Rounded half away from zero, as `gfx::ToRoundedInt`.
fn rounded(value: f32) -> i32 {
    let whole = value as i32;
    let fraction = value - whole as f32;
    if fraction >= 0.5 {
        whole + 1
    } else if fraction <= -0.5 {
        whole - 1
    } else {
        whole
    }
}

/// `resize({ width, height, quality })`: one side alone keeps the aspect
/// ratio. The result's picture is at scale factor 1, resized from the
/// representation nearest it.
fn resize_image(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let options = dictionary(realm, args, 0)?;
    let reps = reps_of(realm, &this);
    let current = image_size(realm, &this, 1.0);
    let width = int32(&option(realm, &options, "width")?);
    let height = int32(&option(realm, &options, "height")?);
    let method = match &option(realm, &options, "quality")? {
        Value::String(quality)
            if quality.to_rust_lossy() == "good" || quality.to_rust_lossy() == "better" =>
        {
            Method::Hamming
        }
        _ => Method::Lanczos,
    };
    let (mut target_width, mut target_height) =
        (width.unwrap_or(current.0), height.unwrap_or(current.1));
    if target_width <= 0 && target_height <= 0 {
        return Ok(Value::Object(image_object(realm, None)));
    }
    let ratio = aspect_ratio(current);
    match (width, height) {
        (Some(width), None) => target_height = rounded(width as f32 * (1.0 / ratio)),
        (None, Some(height)) => target_width = rounded(height as f32 * ratio),
        _ => {}
    }
    let Some(reps) = reps else {
        return Ok(Value::Object(image_object(realm, None)));
    };
    let (target_width, target_height) = (target_width.max(0), target_height.max(0));
    let resized = closest(&reps, 1.0).and_then(|rep| {
        if rep.size() == (target_width, target_height) {
            return Some(rep.clone());
        }
        if target_width == 0 || target_height == 0 || rep.width == 0 || rep.height == 0 {
            return None;
        }
        let size = (target_width as u32, target_height as u32);
        Some(Rep {
            scale: 1.0,
            width: size.0,
            height: size.1,
            pixels: resize::resize(
                &rep.pixels,
                (rep.width, rep.height),
                size,
                method,
                rep.opaque,
            ),
            opaque: rep.opaque,
        })
    });
    Ok(Value::Object(image_object(
        realm,
        Some(resized.into_iter().collect()),
    )))
}

/// `crop({ x, y, width, height })`: the part of the image inside the
/// rectangle, in device-independent pixels, taken from the representation
/// nearest scale factor 1 at its own scale.
fn crop_image(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let options = dictionary(realm, args, 0)?;
    let mut rect = [0_f32; 4];
    for (index, name) in ["x", "y", "width", "height"].into_iter().enumerate() {
        let value = option(realm, &options, name)?;
        rect[index] = number(&value)
            .map(|value| value as f32)
            .ok_or_else(|| crate::electron_builtin::conversion_failure(realm, 0, &options))?;
    }
    // Floats whose edges go to the nearest pixel, as `gfx::ToNearestRect`.
    let (x, y) = (rounded(rect[0]), rounded(rect[1]));
    let rect = [
        x,
        y,
        rounded(rect[0] + rect[2]) - x,
        rounded(rect[1] + rect[3]) - y,
    ];
    let reps = reps_of(realm, &this).unwrap_or_default();
    let Some(first) = reps.first() else {
        return Ok(Value::Object(image_object(realm, None)));
    };
    let bounds = first.size();
    let [x, y, width, height] = [rect[0], rect[1], rect[2].max(0), rect[3].max(0)];
    let (left, top) = (x.max(0), y.max(0));
    let right = x.saturating_add(width).min(bounds.0);
    let bottom = y.saturating_add(height).min(bounds.1);
    if right <= left || bottom <= top {
        return Ok(Value::Object(image_object(realm, None)));
    }
    let rep = closest(&reps, 1.0).expect("an image with representations");
    // The enclosing rectangle in the representation's pixels, within them.
    let scaled =
        |value: i32, round: fn(f64) -> f64| (round(f64::from(value) * f64::from(rep.scale))) as i64;
    let pixel_left = scaled(left, crate::libm::floor).clamp(0, i64::from(rep.width)) as usize;
    let pixel_top = scaled(top, crate::libm::floor).clamp(0, i64::from(rep.height)) as usize;
    let pixel_right = scaled(right, crate::libm::ceil).clamp(0, i64::from(rep.width)) as usize;
    let pixel_bottom = scaled(bottom, crate::libm::ceil).clamp(0, i64::from(rep.height)) as usize;
    let row = rep.width as usize * 4;
    let pixels = (pixel_top..pixel_bottom)
        .flat_map(|line| {
            rep.pixels[line * row + pixel_left * 4..line * row + pixel_right * 4]
                .iter()
                .copied()
        })
        .collect();
    let cropped = Rep {
        scale: rep.scale,
        width: (pixel_right - pixel_left) as u32,
        height: (pixel_bottom - pixel_top) as u32,
        pixels,
        opaque: rep.opaque,
    };
    Ok(Value::Object(image_object(realm, Some(vec![cropped]))))
}

/// `addRepresentation({ scaleFactor, width, height, buffer, dataURL })`:
/// another picture, from a buffer as `createFromBuffer` reads one or a PNG
/// or JPEG data URL.
fn add_representation(realm: &mut Realm, this: Value, args: &[Value]) -> Completion {
    let options = dictionary(realm, args, 0)?;
    let scale = number(&option(realm, &options, "scaleFactor")?).map_or(1.0, |scale| scale as f32);
    let width = int32(&option(realm, &options, "width")?).unwrap_or(0);
    let height = int32(&option(realm, &options, "height")?).unwrap_or(0);
    let buffer = option(realm, &options, "buffer")?;
    let rep = match crate::typed_array_builtin::view_bytes(&buffer) {
        Some(bytes) => rep_from_buffer(&bytes, (width, height), scale),
        None => match option(realm, &options, "dataURL")? {
            Value::String(url) => rep_from_data_url(&url.to_rust_lossy(), scale),
            _ => None,
        },
    };
    if let (Some(rep), Value::Object(image)) = (rep, &this) {
        let mut reps = reps_of(realm, &this).unwrap_or_default();
        reps.push(rep);
        let list = reps
            .iter()
            .map(|rep| {
                Value::Object(crate::typed_array_builtin::new_bytes(
                    realm,
                    rep.encode(),
                    None,
                ))
            })
            .collect();
        image.set_private(
            &realm.electron_image_key,
            Property::hidden(Value::Object(realm.array(list))),
        );
    }
    Ok(Value::Undefined)
}

/// A `NativeImage` of the clipboard's picture: its PNG form when an
/// application put one there, or else its device-independent bitmap, as
/// Chromium reads images on Windows.
pub(crate) fn clipboard_image(
    realm: &mut Realm,
    png: Option<Vec<u8>>,
    dib: Option<Vec<u8>>,
) -> Object {
    let image = png
        .and_then(|bytes| decode_png(&bytes).ok())
        .or_else(|| dib.and_then(|bytes| decode_dib(&bytes).ok()));
    let reps = image.map(|image| vec![Rep::from_straight(1.0, image)]);
    image_object(realm, reps)
}

/// The straight RGBA pixels of an image argument, as icons take them: a
/// `NativeImage`, or the path of an image file. Its picture nearest scale
/// factor 1.
pub(crate) fn pixels(realm: &mut Realm, value: &Value) -> Option<(u32, u32, Vec<u8>)> {
    let reps = match value {
        Value::String(path) => reps_from_path(&path.to_rust_lossy()),
        _ => reps_of(realm, value)?,
    };
    let rep = closest(&reps, 1.0)?;
    Some((rep.width, rep.height, rep.straight()))
}

pub(crate) fn dibv5(realm: &mut Realm, value: &Value) -> Result<Vec<u8>, Thrown> {
    if !matches!(value, Value::Object(_)) {
        return Err(realm.throw(ErrorKind::Type, "Expected a nativeImage object"));
    }
    let (width, height, rgba) = pixels(realm, value)
        .ok_or_else(|| realm.throw(ErrorKind::Type, "Expected a non-empty nativeImage"))?;
    rgba_to_dibv5(width, height, &rgba).map_err(|message| realm.throw(ErrorKind::Error, &message))
}

/// Pixels of a packed device-independent bitmap (`BITMAPINFOHEADER` or a
/// later header, 24 or 32 bits per pixel).
fn decode_dib(input: &[u8]) -> Result<PngImage, String> {
    let u32_at = |offset: usize| -> Result<u32, String> {
        input
            .get(offset..offset + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .ok_or_else(|| "truncated bitmap".to_owned())
    };
    let header = u32_at(0)? as usize;
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bits = input
        .get(14..16)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or("truncated bitmap")?;
    let compression = u32_at(16)?;
    if header < 40 || width <= 0 || height == 0 || !(bits == 24 || bits == 32) {
        return Err("unsupported bitmap".to_owned());
    }
    let (width, top_down, height) = (width as u32, height < 0, height.unsigned_abs());
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("bitmap is too large".to_owned());
    }
    // Channel masks: standard BGR(A) unless the bitmap gives its own.
    let mut masks = [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0xff00_0000_u32];
    let mut offset = header;
    if compression == 3 {
        if header == 40 {
            for (index, mask) in masks.iter_mut().take(3).enumerate() {
                *mask = u32_at(40 + index * 4)?;
            }
            masks[3] = 0;
            offset += 12;
        } else {
            for (index, mask) in masks.iter_mut().enumerate() {
                *mask = u32_at(40 + index * 4)?;
            }
        }
    } else if compression != 0 {
        return Err("unsupported bitmap compression".to_owned());
    }
    let stride = (width as usize * usize::from(bits)).div_ceil(32) * 4;
    let pixels = input.get(offset..).ok_or("truncated bitmap")?;
    if pixels.len() < stride * height as usize {
        return Err("truncated bitmap".to_owned());
    }
    let channel = |value: u32, mask: u32| -> u8 {
        if mask == 0 {
            return 0;
        }
        let shift = mask.trailing_zeros();
        let maximum = mask >> shift;
        (((value & mask) >> shift) * 255 / maximum) as u8
    };
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for row in 0..height as usize {
        let source = if top_down {
            row
        } else {
            height as usize - 1 - row
        };
        let line = &pixels[source * stride..source * stride + stride];
        for column in 0..width as usize {
            if bits == 24 {
                let pixel = &line[column * 3..column * 3 + 3];
                rgba.extend([pixel[2], pixel[1], pixel[0], 255]);
            } else {
                let pixel = &line[column * 4..column * 4 + 4];
                let value = u32::from_le_bytes([pixel[0], pixel[1], pixel[2], pixel[3]]);
                rgba.extend([
                    channel(value, masks[0]),
                    channel(value, masks[1]),
                    channel(value, masks[2]),
                    if masks[3] == 0 {
                        255
                    } else {
                        channel(value, masks[3])
                    },
                ]);
            }
        }
    }
    // Plain 32-bit bitmaps often leave the fourth byte zero: opaque.
    if bits == 32 && rgba.chunks_exact(4).all(|pixel| pixel[3] == 0) {
        for pixel in rgba.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }
    let opaque = rgba.chunks_exact(4).all(|pixel| pixel[3] == 255);
    Ok(PngImage {
        width,
        height,
        rgba,
        opaque,
    })
}

/// RGBA pixels as a PNG file: 8-bit RGBA, unfiltered rows, zlib level 6.
#[cfg(test)]
pub(crate) fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    png_file(width, height, rgba, false)
}

/// RGBA pixels as a PNG file, 8-bit RGB for an opaque picture as Chromium
/// writes one, or else RGBA; unfiltered rows, zlib level 6.
fn png_file(width: u32, height: u32, rgba: &[u8], opaque: bool) -> Vec<u8> {
    let channels = if opaque { 3 } else { 4 };
    let mut rows =
        Vec::with_capacity(width as usize * height as usize * channels + height as usize);
    for row in rgba.chunks_exact((width as usize * 4).max(1)) {
        rows.push(0);
        for pixel in row.chunks_exact(4) {
            rows.extend_from_slice(&pixel[..channels]);
        }
    }
    let mut png = PNG_SIGNATURE.to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        png.extend((data.len() as u32).to_be_bytes());
        let start = png.len();
        png.extend_from_slice(kind);
        png.extend_from_slice(data);
        let crc = crc32(&png[start..]);
        png.extend(crc.to_be_bytes());
    };
    let mut header = Vec::with_capacity(13);
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, if opaque { 2 } else { 6 }, 0, 0, 0]);
    chunk(b"IHDR", &header);
    chunk(
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&rows, 6),
    );
    chunk(b"IEND", &[]);
    png
}

fn rgba_to_dibv5(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .filter(|pixels| *pixels <= MAX_PIXELS)
        .ok_or("invalid native image dimensions")?;
    if width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err("invalid native image dimensions".to_owned());
    }
    let expected = usize::try_from(pixels.checked_mul(4).ok_or("image is too large")?)
        .map_err(|_| "image is too large")?;
    if width == 0 || height == 0 || rgba.len() != expected {
        return Err("invalid native image pixel buffer".to_owned());
    }
    let pixel_size = expected;
    let mut dib = vec![0_u8; DIBV5_HEADER_SIZE + pixel_size];
    put_u32(&mut dib, 0, DIBV5_HEADER_SIZE as u32);
    put_u32(&mut dib, 4, width);
    put_u32(&mut dib, 8, (-(height as i32)) as u32);
    put_u16(&mut dib, 12, 1);
    put_u16(&mut dib, 14, 32);
    put_u32(&mut dib, 16, 3); // BI_BITFIELDS
    put_u32(&mut dib, 20, pixel_size as u32);
    put_u32(&mut dib, 40, 0x00ff_0000);
    put_u32(&mut dib, 44, 0x0000_ff00);
    put_u32(&mut dib, 48, 0x0000_00ff);
    put_u32(&mut dib, 52, 0xff00_0000);
    put_u32(&mut dib, 56, 0x7352_4742); // LCS_sRGB
    for (source, target) in rgba
        .chunks_exact(4)
        .zip(dib[DIBV5_HEADER_SIZE..].chunks_exact_mut(4))
    {
        target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
    }
    Ok(dib)
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) {
    output[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) {
    output[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn decode_png(input: &[u8]) -> Result<PngImage, String> {
    if input.len() < 33 || !input.starts_with(PNG_SIGNATURE) {
        return Err("invalid PNG signature".to_owned());
    }
    let mut offset = 8_usize;
    let mut width = 0_u32;
    let mut height = 0_u32;
    let mut bit_depth = 0_u8;
    let mut color_type = 0_u8;
    let mut interlace = 0_u8;
    let mut palette = Vec::new();
    let mut transparency = Vec::new();
    let mut compressed = Vec::new();
    let mut saw_header = false;
    let mut saw_end = false;
    while offset < input.len() {
        if input.len() - offset < 12 {
            return Err("truncated PNG chunk".to_owned());
        }
        let length = be_u32(input, offset)? as usize;
        let end = offset
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .filter(|value| *value <= input.len())
            .ok_or("truncated PNG chunk")?;
        let kind = &input[offset + 4..offset + 8];
        let data = &input[offset + 8..offset + 8 + length];
        let expected_crc = be_u32(input, offset + 8 + length)?;
        if crc32(&input[offset + 4..offset + 8 + length]) != expected_crc {
            return Err("invalid PNG chunk checksum".to_owned());
        }
        match kind {
            b"IHDR" if !saw_header && length == 13 => {
                width = be_u32(data, 0)?;
                height = be_u32(data, 4)?;
                bit_depth = data[8];
                color_type = data[9];
                if data[10] != 0 || data[11] != 0 {
                    return Err("unsupported PNG compression or filter method".to_owned());
                }
                interlace = data[12];
                saw_header = true;
            }
            b"PLTE" => palette.extend_from_slice(data),
            b"tRNS" => transparency.extend_from_slice(data),
            b"IDAT" if saw_header => compressed.extend_from_slice(data),
            b"IEND" => {
                saw_end = true;
                break;
            }
            _ if kind[0] & 0x20 == 0 => {
                return Err("unsupported critical PNG chunk".to_owned());
            }
            _ => {}
        }
        offset = end;
    }
    if !saw_header || !saw_end || width == 0 || height == 0 {
        return Err("incomplete PNG image".to_owned());
    }
    if bit_depth != 8 || interlace != 0 {
        return Err("unsupported PNG bit depth or interlace mode".to_owned());
    }
    let channels = match color_type {
        0 => 1_usize,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err("unsupported PNG color type".to_owned()),
    };
    if color_type == 3 && (palette.is_empty() || palette.len() % 3 != 0) {
        return Err("invalid PNG palette".to_owned());
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .filter(|pixels| *pixels <= MAX_PIXELS)
        .ok_or("PNG dimensions are too large")? as usize;
    let row_bytes = (width as usize)
        .checked_mul(channels)
        .ok_or("PNG row is too large")?;
    let decoded_length = row_bytes
        .checked_add(1)
        .and_then(|row| row.checked_mul(height as usize))
        .ok_or("PNG data is too large")?;
    let decoded =
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&compressed, decoded_length)
            .map_err(|_| "invalid PNG compressed data".to_owned())?;
    if decoded.len() != decoded_length {
        return Err("invalid PNG scanline data".to_owned());
    }
    let mut raw = vec![0_u8; row_bytes * height as usize];
    let bpp = channels;
    for y in 0..height as usize {
        let source_start = y * (row_bytes + 1);
        let filter = decoded[source_start];
        let row_start = y * row_bytes;
        for x in 0..row_bytes {
            let value = decoded[source_start + 1 + x];
            let left = if x >= bpp {
                raw[row_start + x - bpp]
            } else {
                0
            };
            let above = if y != 0 {
                raw[row_start + x - row_bytes]
            } else {
                0
            };
            let upper_left = if y != 0 && x >= bpp {
                raw[row_start + x - row_bytes - bpp]
            } else {
                0
            };
            raw[row_start + x] = match filter {
                0 => value,
                1 => value.wrapping_add(left),
                2 => value.wrapping_add(above),
                3 => value.wrapping_add(((u16::from(left) + u16::from(above)) / 2) as u8),
                4 => value.wrapping_add(paeth(left, above, upper_left)),
                _ => return Err("invalid PNG filter type".to_owned()),
            };
        }
    }
    let mut rgba = Vec::with_capacity(pixels * 4);
    for pixel in raw.chunks_exact(channels) {
        match color_type {
            0 => {
                let gray = pixel[0];
                let alpha = if transparency.as_slice() == [0, gray] {
                    0
                } else {
                    255
                };
                rgba.extend_from_slice(&[gray, gray, gray, alpha]);
            }
            2 => {
                let transparent_rgb = transparency.len() == 6
                    && transparency[0] == 0
                    && transparency[1] == pixel[0]
                    && transparency[2] == 0
                    && transparency[3] == pixel[1]
                    && transparency[4] == 0
                    && transparency[5] == pixel[2];
                rgba.extend_from_slice(&[
                    pixel[0],
                    pixel[1],
                    pixel[2],
                    if transparent_rgb { 0 } else { 255 },
                ]);
            }
            3 => {
                let palette_index = pixel[0] as usize;
                let base = palette_index
                    .checked_mul(3)
                    .ok_or("invalid PNG palette index")?;
                if base + 2 >= palette.len() {
                    return Err("invalid PNG palette index".to_owned());
                }
                rgba.extend_from_slice(&[
                    palette[base],
                    palette[base + 1],
                    palette[base + 2],
                    transparency.get(palette_index).copied().unwrap_or(255),
                ]);
            }
            4 => rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]),
            6 => rgba.extend_from_slice(pixel),
            _ => unreachable!(),
        }
    }
    Ok(PngImage {
        width,
        height,
        rgba,
        opaque: color_type != 4 && color_type != 6 && transparency.is_empty(),
    })
}

fn be_u32(input: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = input
        .get(offset..offset + 4)
        .ok_or("truncated PNG integer")?;
    Ok(u32::from_be_bytes(
        bytes.try_into().expect("four-byte slice"),
    ))
}

fn paeth(left: u8, above: u8, upper_left: u8) -> u8 {
    let left = i32::from(left);
    let above = i32::from(above);
    let upper_left = i32::from(upper_left);
    let predictor = left + above - upper_left;
    let left_distance = (predictor - left).abs();
    let above_distance = (predictor - above).abs();
    let upper_left_distance = (predictor - upper_left).abs();
    if left_distance <= above_distance && left_distance <= upper_left_distance {
        left as u8
    } else if above_distance <= upper_left_distance {
        above as u8
    } else {
        upper_left as u8
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests;
