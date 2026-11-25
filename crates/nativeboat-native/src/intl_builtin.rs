//! `Intl`, over the ICU library Windows ships (`icu.dll`, Windows 10 1903
//! and later), which the program loads when first needed: segmentation,
//! collation, number, date, plural, relative time and list formatting with
//! the same data browsers use. Without that library, `Intl` keeps working
//! with simple fallbacks: code point segments, code unit order and the
//! runtime's own number and date text.

#[allow(unused_imports)]
use crate::prelude::*;

use crate::{ErrorKind, Key, Kind, Object, Property, Realm, Thrown, Value, ops};
use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::Any;
use core::cell::RefCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicPtr, AtomicU8, Ordering};

type Handle = *mut c_void;
type Status = i32;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryExW(name: *const u16, file: Handle, flags: u32) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const c_void;
}

/// The ICU functions `Intl` uses, with their C signatures.
#[allow(clippy::type_complexity)]
struct Icu {
    uloc_for_language_tag:
        unsafe extern "C" fn(*const u8, *mut u8, i32, *mut i32, *mut Status) -> i32,
    uloc_to_language_tag: unsafe extern "C" fn(*const u8, *mut u8, i32, i8, *mut Status) -> i32,
    ubrk_open: unsafe extern "C" fn(i32, *const u8, *const u16, i32, *mut Status) -> Handle,
    ubrk_first: unsafe extern "C" fn(Handle) -> i32,
    ubrk_next: unsafe extern "C" fn(Handle) -> i32,
    ubrk_get_rule_status: unsafe extern "C" fn(Handle) -> i32,
    ubrk_close: unsafe extern "C" fn(Handle),
    ucol_open: unsafe extern "C" fn(*const u8, *mut Status) -> Handle,
    ucol_set_attribute: unsafe extern "C" fn(Handle, i32, i32, *mut Status),
    ucol_strcoll: unsafe extern "C" fn(Handle, *const u16, i32, *const u16, i32) -> i32,
    ucol_close: unsafe extern "C" fn(Handle),
    unumf_open: unsafe extern "C" fn(*const u16, i32, *const u8, *mut Status) -> Handle,
    unumf_open_result: unsafe extern "C" fn(*mut Status) -> Handle,
    unumf_format_double: unsafe extern "C" fn(Handle, f64, Handle, *mut Status),
    unumf_result_to_string: unsafe extern "C" fn(Handle, *mut u16, i32, *mut Status) -> i32,
    unumf_close: unsafe extern "C" fn(Handle),
    unumf_close_result: unsafe extern "C" fn(Handle),
    udatpg_open: unsafe extern "C" fn(*const u8, *mut Status) -> Handle,
    udatpg_get_best_pattern:
        unsafe extern "C" fn(Handle, *const u16, i32, *mut u16, i32, *mut Status) -> i32,
    udatpg_close: unsafe extern "C" fn(Handle),
    udat_open: unsafe extern "C" fn(
        i32,
        i32,
        *const u8,
        *const u16,
        i32,
        *const u16,
        i32,
        *mut Status,
    ) -> Handle,
    udat_format: unsafe extern "C" fn(Handle, f64, *mut u16, i32, Handle, *mut Status) -> i32,
    udat_close: unsafe extern "C" fn(Handle),
    udat_to_pattern: unsafe extern "C" fn(Handle, i8, *mut u16, i32, *mut Status) -> i32,
    uplrules_open_for_type: unsafe extern "C" fn(*const u8, i32, *mut Status) -> Handle,
    uplrules_select: unsafe extern "C" fn(Handle, f64, *mut u16, i32, *mut Status) -> i32,
    uplrules_close: unsafe extern "C" fn(Handle),
    ureldatefmt_open: unsafe extern "C" fn(*const u8, Handle, i32, i32, *mut Status) -> Handle,
    ureldatefmt_format: unsafe extern "C" fn(Handle, f64, i32, *mut u16, i32, *mut Status) -> i32,
    ureldatefmt_format_numeric:
        unsafe extern "C" fn(Handle, f64, i32, *mut u16, i32, *mut Status) -> i32,
    ureldatefmt_close: unsafe extern "C" fn(Handle),
    /// Missing from older ICU builds, which have only the `and` list.
    ulistfmt_open_for_type:
        Option<unsafe extern "C" fn(*const u8, i32, i32, *mut Status) -> Handle>,
    ulistfmt_open: unsafe extern "C" fn(*const u8, *mut Status) -> Handle,
    ulistfmt_format: unsafe extern "C" fn(
        Handle,
        *const *const u16,
        *const i32,
        i32,
        *mut u16,
        i32,
        *mut Status,
    ) -> i32,
    ulistfmt_close: unsafe extern "C" fn(Handle),
    ucal_get_default_time_zone: unsafe extern "C" fn(*mut u16, i32, *mut Status) -> i32,
    uldn_open_for_context: unsafe extern "C" fn(*const u8, *const i32, i32, *mut Status) -> Handle,
    uldn_locale_display_name:
        unsafe extern "C" fn(Handle, *const u8, *mut u16, i32, *mut Status) -> i32,
    uldn_region_display_name:
        unsafe extern "C" fn(Handle, *const u8, *mut u16, i32, *mut Status) -> i32,
    uldn_script_display_name:
        unsafe extern "C" fn(Handle, *const u8, *mut u16, i32, *mut Status) -> i32,
    uldn_key_value_display_name:
        unsafe extern "C" fn(Handle, *const u8, *const u8, *mut u16, i32, *mut Status) -> i32,
    uldn_close: unsafe extern "C" fn(Handle),
    udatpg_get_field_display_name:
        unsafe extern "C" fn(Handle, i32, i32, *mut u16, i32, *mut Status) -> i32,
    uloc_add_likely_subtags: unsafe extern "C" fn(*const u8, *mut u8, i32, *mut Status) -> i32,
    uloc_minimize_subtags: unsafe extern "C" fn(*const u8, *mut u8, i32, *mut Status) -> i32,
    uloc_is_right_to_left: unsafe extern "C" fn(*const u8) -> i8,
    ucal_open: unsafe extern "C" fn(*const u16, i32, *const u8, i32, *mut Status) -> Handle,
    ucal_get_attribute: unsafe extern "C" fn(Handle, i32) -> i32,
    ucal_get_day_of_week_type: unsafe extern "C" fn(Handle, i32, *mut Status) -> i32,
    ucal_close: unsafe extern "C" fn(Handle),
    ucal_get_keyword_values_for_locale:
        unsafe extern "C" fn(*const u8, *const u8, i8, *mut Status) -> Handle,
    ucal_open_time_zone_id_enumeration:
        unsafe extern "C" fn(i32, *const u8, *const i32, *mut Status) -> Handle,
    ucol_get_keyword_values_for_locale:
        unsafe extern "C" fn(*const u8, *const u8, i8, *mut Status) -> Handle,
    unumsys_open: unsafe extern "C" fn(*const u8, *mut Status) -> Handle,
    unumsys_get_name: unsafe extern "C" fn(Handle) -> *const u8,
    unumsys_close: unsafe extern "C" fn(Handle),
    uenum_next: unsafe extern "C" fn(Handle, *mut i32, *mut Status) -> *const u8,
    uenum_close: unsafe extern "C" fn(Handle),
}

static ICU: AtomicPtr<Icu> = AtomicPtr::new(core::ptr::null_mut());
/// Zero until ICU was looked for, then one when it is missing.
static ICU_MISSING: AtomicU8 = AtomicU8::new(0);

/// The Windows ICU library, loaded once; `None` where Windows has none.
fn icu() -> Option<&'static Icu> {
    let loaded = ICU.load(Ordering::Acquire);
    if !loaded.is_null() {
        // SAFETY: a leaked, never-freed table published below.
        return Some(unsafe { &*loaded });
    }
    if ICU_MISSING.load(Ordering::Acquire) != 0 {
        return None;
    }
    let table = load_icu();
    match table {
        Some(table) => {
            let table = Box::into_raw(Box::new(table));
            ICU.store(table, Ordering::Release);
            // SAFETY: just leaked; it lives for the rest of the process.
            Some(unsafe { &*table })
        }
        None => {
            ICU_MISSING.store(1, Ordering::Release);
            None
        }
    }
}

// Each transmute's target is the type of the field it fills.
#[allow(clippy::missing_transmute_annotations)]
fn load_icu() -> Option<Icu> {
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x800;
    let name: Vec<u16> = "icu.dll".encode_utf16().chain([0]).collect();
    // SAFETY: a null-terminated name; System32 only.
    let module = unsafe {
        LoadLibraryExW(
            name.as_ptr(),
            core::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return None;
    }
    macro_rules! symbol {
        ($name:literal) => {{
            // SAFETY: a null-terminated export name of a loaded module.
            let address = unsafe { GetProcAddress(module, concat!($name, "\0").as_ptr()) };
            if address.is_null() {
                return None;
            }
            // SAFETY: the export has the C signature of the field it fills,
            // as ICU's headers declare it.
            unsafe { core::mem::transmute::<*const c_void, _>(address) }
        }};
    }
    Some(Icu {
        uloc_for_language_tag: symbol!("uloc_forLanguageTag"),
        uloc_to_language_tag: symbol!("uloc_toLanguageTag"),
        ubrk_open: symbol!("ubrk_open"),
        ubrk_first: symbol!("ubrk_first"),
        ubrk_next: symbol!("ubrk_next"),
        ubrk_get_rule_status: symbol!("ubrk_getRuleStatus"),
        ubrk_close: symbol!("ubrk_close"),
        ucol_open: symbol!("ucol_open"),
        ucol_set_attribute: symbol!("ucol_setAttribute"),
        ucol_strcoll: symbol!("ucol_strcoll"),
        ucol_close: symbol!("ucol_close"),
        unumf_open: symbol!("unumf_openForSkeletonAndLocale"),
        unumf_open_result: symbol!("unumf_openResult"),
        unumf_format_double: symbol!("unumf_formatDouble"),
        unumf_result_to_string: symbol!("unumf_resultToString"),
        unumf_close: symbol!("unumf_close"),
        unumf_close_result: symbol!("unumf_closeResult"),
        udatpg_open: symbol!("udatpg_open"),
        udatpg_get_best_pattern: symbol!("udatpg_getBestPattern"),
        udatpg_close: symbol!("udatpg_close"),
        udat_open: symbol!("udat_open"),
        udat_format: symbol!("udat_format"),
        udat_close: symbol!("udat_close"),
        udat_to_pattern: symbol!("udat_toPattern"),
        uplrules_open_for_type: symbol!("uplrules_openForType"),
        uplrules_select: symbol!("uplrules_select"),
        uplrules_close: symbol!("uplrules_close"),
        ureldatefmt_open: symbol!("ureldatefmt_open"),
        ureldatefmt_format: symbol!("ureldatefmt_format"),
        ureldatefmt_format_numeric: symbol!("ureldatefmt_formatNumeric"),
        ureldatefmt_close: symbol!("ureldatefmt_close"),
        // SAFETY: as for `symbol!`; a missing export leaves `None`.
        ulistfmt_open_for_type: unsafe {
            let address = GetProcAddress(module, c"ulistfmt_openForType".as_ptr().cast());
            (!address.is_null()).then(|| core::mem::transmute::<*const c_void, _>(address))
        },
        ulistfmt_open: symbol!("ulistfmt_open"),
        ulistfmt_format: symbol!("ulistfmt_format"),
        ulistfmt_close: symbol!("ulistfmt_close"),
        ucal_get_default_time_zone: symbol!("ucal_getDefaultTimeZone"),
        uldn_open_for_context: symbol!("uldn_openForContext"),
        uldn_locale_display_name: symbol!("uldn_localeDisplayName"),
        uldn_region_display_name: symbol!("uldn_regionDisplayName"),
        uldn_script_display_name: symbol!("uldn_scriptDisplayName"),
        uldn_key_value_display_name: symbol!("uldn_keyValueDisplayName"),
        uldn_close: symbol!("uldn_close"),
        udatpg_get_field_display_name: symbol!("udatpg_getFieldDisplayName"),
        uloc_add_likely_subtags: symbol!("uloc_addLikelySubtags"),
        uloc_minimize_subtags: symbol!("uloc_minimizeSubtags"),
        uloc_is_right_to_left: symbol!("uloc_isRightToLeft"),
        ucal_open: symbol!("ucal_open"),
        ucal_get_attribute: symbol!("ucal_getAttribute"),
        ucal_get_day_of_week_type: symbol!("ucal_getDayOfWeekType"),
        ucal_close: symbol!("ucal_close"),
        ucal_get_keyword_values_for_locale: symbol!("ucal_getKeywordValuesForLocale"),
        ucal_open_time_zone_id_enumeration: symbol!("ucal_openTimeZoneIDEnumeration"),
        ucol_get_keyword_values_for_locale: symbol!("ucol_getKeywordValuesForLocale"),
        unumsys_open: symbol!("unumsys_open"),
        unumsys_get_name: symbol!("unumsys_getName"),
        unumsys_close: symbol!("unumsys_close"),
        uenum_next: symbol!("uenum_next"),
        uenum_close: symbol!("uenum_close"),
    })
}

fn failed(status: Status) -> bool {
    status > 0
}

/// Text an ICU function writes into a buffer it sizes by returning the
/// length it needs.
fn text_from(mut write: impl FnMut(*mut u16, i32, *mut Status) -> i32) -> Option<Vec<u16>> {
    const BUFFER_OVERFLOW: Status = 15;
    let mut buffer = alloc::vec![0u16; 64];
    loop {
        let mut status = 0;
        let length = write(buffer.as_mut_ptr(), buffer.len() as i32, &mut status);
        if status == BUFFER_OVERFLOW && length as usize >= buffer.len() {
            buffer.resize(length as usize + 1, 0);
            continue;
        }
        if failed(status) || length < 0 {
            return None;
        }
        buffer.truncate(length as usize);
        return Some(buffer);
    }
}

/// An ICU locale ID for a BCP 47 tag, null-terminated.
fn icu_locale(icu: &Icu, tag: &str) -> Option<Vec<u8>> {
    let tag: Vec<u8> = tag.bytes().chain([0]).collect();
    let mut id = alloc::vec![0u8; 160];
    let mut status = 0;
    let mut parsed = 0;
    // SAFETY: null-terminated input and a writable buffer of its length.
    let length = unsafe {
        (icu.uloc_for_language_tag)(
            tag.as_ptr(),
            id.as_mut_ptr(),
            id.len() as i32 - 1,
            &mut parsed,
            &mut status,
        )
    };
    if failed(status) || length <= 0 || parsed <= 0 {
        return None;
    }
    id.truncate(length as usize);
    id.push(0);
    Some(id)
}

/// The BCP 47 tag of an ICU locale ID.
fn language_tag(icu: &Icu, id: &[u8]) -> Option<String> {
    let mut tag = alloc::vec![0u8; 160];
    let mut status = 0;
    // SAFETY: a null-terminated ID and a writable buffer of its length.
    let length = unsafe {
        (icu.uloc_to_language_tag)(
            id.as_ptr(),
            tag.as_mut_ptr(),
            tag.len() as i32 - 1,
            1,
            &mut status,
        )
    };
    if failed(status) || length <= 0 {
        return None;
    }
    tag.truncate(length as usize);
    String::from_utf8(tag).ok()
}

/// The locales a `locales` argument requests, in order.
fn requested_locales(realm: &mut Realm, locales: &Value) -> Result<Vec<String>, Thrown> {
    let tag = |realm: &mut Realm, value: &Value| -> Result<String, Thrown> {
        match value {
            Value::String(text) => Ok(text.to_rust_lossy()),
            Value::Object(_) => Ok(ops::to_string(realm, value)?.to_rust_lossy()),
            _ => Err(realm.throw(ErrorKind::Type, "Language ID should be string or object.")),
        }
    };
    let requested = match locales {
        Value::Undefined => Vec::new(),
        Value::String(_) => vec![tag(realm, locales)?],
        // An `Intl.Locale` is one locale, not a list.
        Value::Object(object) if matches!(&object.data().kind, Kind::Native(data) if data.is::<State<LocaleTag>>()) =>
        {
            vec![tag(realm, locales)?]
        }
        Value::Object(_) => {
            let length = ops::get(realm, locales, &Key::name("length"))?;
            let length = ops::to_number(realm, &length)?.max(0.0) as u32;
            let mut tags = Vec::new();
            for index in 0..length {
                let value = ops::get(realm, locales, &Key::Index(index))?;
                tags.push(tag(realm, &value)?);
            }
            tags
        }
        _ => return Err(realm.throw(ErrorKind::Type, "Incorrect locale information provided")),
    };
    let mut canonical = Vec::new();
    for tag in requested {
        let tag = canonical_tag(realm, &tag)?;
        if !canonical.contains(&tag) {
            canonical.push(tag);
        }
    }
    Ok(canonical)
}

/// A locale tag in canonical form; a malformed tag is a `RangeError`.
fn canonical_tag(realm: &mut Realm, tag: &str) -> Result<String, Thrown> {
    let valid = !tag.is_empty()
        && tag.split('-').all(|part| {
            !part.is_empty()
                && part.len() <= 8
                && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        });
    if !valid {
        return Err(realm.throw(ErrorKind::Range, "Incorrect locale information provided"));
    }
    Ok(match icu() {
        Some(icu) => icu_locale(icu, tag)
            .and_then(|id| language_tag(icu, &id))
            .unwrap_or_else(|| tag.to_owned()),
        None => tag.to_owned(),
    })
}

/// The locale a formatter uses: the first requested one, else the user's.
fn resolve_locale(realm: &mut Realm, locales: &Value) -> Result<String, Thrown> {
    let requested = requested_locales(realm, locales)?;
    Ok(requested
        .into_iter()
        .next()
        .unwrap_or_else(crate::platform::user_locale))
}

/// A string option, `None` when undefined; values outside `allowed` are a
/// `RangeError`.
fn string_option(
    realm: &mut Realm,
    options: &Value,
    name: &str,
    allowed: &[&str],
) -> Result<Option<String>, Thrown> {
    if !matches!(options, Value::Object(_)) {
        return Ok(None);
    }
    let value = ops::get(realm, options, &Key::name(name))?;
    if matches!(value, Value::Undefined) {
        return Ok(None);
    }
    let value = ops::to_string(realm, &value)?.to_rust_lossy();
    if !allowed.is_empty() && !allowed.contains(&value.as_str()) {
        return Err(realm.throw(
            ErrorKind::Range,
            &format!("Value {value} out of range for Intl options property {name}"),
        ));
    }
    Ok(Some(value))
}

fn boolean_option(realm: &mut Realm, options: &Value, name: &str) -> Result<Option<bool>, Thrown> {
    if !matches!(options, Value::Object(_)) {
        return Ok(None);
    }
    let value = ops::get(realm, options, &Key::name(name))?;
    Ok((!matches!(value, Value::Undefined)).then(|| ops::to_boolean(&value)))
}

fn number_option(
    realm: &mut Realm,
    options: &Value,
    name: &str,
    low: f64,
    high: f64,
) -> Result<Option<u32>, Thrown> {
    if !matches!(options, Value::Object(_)) {
        return Ok(None);
    }
    let value = ops::get(realm, options, &Key::name(name))?;
    if matches!(value, Value::Undefined) {
        return Ok(None);
    }
    let number = ops::to_number(realm, &value)?;
    if !(low..=high).contains(&number) || number.is_nan() {
        return Err(realm.throw(ErrorKind::Range, &format!("{name} value is out of range.")));
    }
    Ok(Some(number as u32))
}

fn arg(args: &[Value], index: usize) -> Value {
    args.get(index).cloned().unwrap_or(Value::Undefined)
}

/// A plain object with string data properties, as `resolvedOptions` gives.
fn record(realm: &Realm, fields: &[(&str, Value)]) -> Value {
    let object = realm.object();
    for (name, value) in fields {
        object.define(Key::name(name), Property::data(value.clone()));
    }
    Value::Object(object)
}

/// The state of an `Intl` object, kept in its native slot.
struct State<T>(T);

fn state<T: 'static>(realm: &Realm, this: &Value, what: &str) -> Result<Rc<dyn Any>, Thrown> {
    if let Value::Object(object) = this
        && let Kind::Native(data) = &object.data().kind
        && data.is::<State<T>>()
    {
        return Ok(data.clone());
    }
    Err(realm.throw(
        ErrorKind::Type,
        &format!("Method {what} called on incompatible receiver"),
    ))
}

fn native<T: 'static>(data: &Rc<dyn Any>) -> &T {
    &data
        .downcast_ref::<State<T>>()
        .expect("a checked Intl state")
        .0
}

/// Defines an `Intl` constructor with its `supportedLocalesOf`.
fn constructor<T: 'static>(
    realm: &Realm,
    intl: &Object,
    name: &'static str,
    prototype: &Object,
    create: impl Fn(&mut Realm, &Value, &Value) -> Result<T, Thrown> + 'static,
) {
    let instances = prototype.clone();
    let create = Rc::new(create);
    let construct = create.clone();
    let call_prototype = prototype.clone();
    let function = realm.builtin_constructor(
        name,
        0,
        prototype,
        move |realm, _, args| {
            // Collator, NumberFormat and DateTimeFormat may be called
            // without `new`.
            let state = create(realm, &arg(args, 0), &arg(args, 1))?;
            let data: Rc<dyn Any> = Rc::new(State(state));
            Ok(Value::Object(Object::new(
                Some(call_prototype.clone()),
                Kind::Native(data),
            )))
        },
        move |realm, _, args| {
            let state = construct(realm, &arg(args, 0), &arg(args, 1))?;
            let data: Rc<dyn Any> = Rc::new(State(state));
            Ok(Object::new(Some(instances.clone()), Kind::Native(data)))
        },
    );
    realm.method(&function, "supportedLocalesOf", 1, |realm, _, args| {
        let locales = requested_locales(realm, &arg(args, 0))?;
        let values = locales.into_iter().map(|tag| Value::str(&tag)).collect();
        Ok(Value::Object(realm.array(values)))
    });
    prototype.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property::hidden(Value::str(&format!("Intl.{name}"))),
    );
    intl.define(Key::name(name), Property::hidden(Value::Object(function)));
}

/// A getter that returns a function bound to the instance, made once, as
/// `format` and `compare` are.
fn bound_getter(
    realm: &Realm,
    prototype: &Object,
    name: &'static str,
    method: fn(&mut Realm, &Value, &[Value]) -> crate::Completion,
) {
    let cache = Rc::new(RefCell::new(Vec::<(Object, Object)>::new()));
    let getter = realm.function(&format!("get {name}"), 0, false, move |realm, this, _| {
        let Value::Object(instance) = &this else {
            return Err(realm.throw(
                ErrorKind::Type,
                &format!("Method get {name} called on incompatible receiver"),
            ));
        };
        if let Some((_, bound)) = cache.borrow().iter().find(|(known, _)| known == instance) {
            return Ok(Value::Object(bound.clone()));
        }
        let receiver = this.clone();
        let bound = realm.function("", 1, false, move |realm, _, args| {
            method(realm, &receiver, args)
        });
        cache.borrow_mut().push((instance.clone(), bound.clone()));
        Ok(Value::Object(bound))
    });
    prototype.define(
        Key::name(name),
        Property {
            slot: crate::Slot::Accessor {
                get: Some(getter),
                set: None,
            },
            enumerable: false,
            configurable: true,
        },
    );
}

pub(crate) fn install(realm: &mut Realm) {
    let intl = realm.object();
    intl.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property::hidden(Value::str("Intl")),
    );
    realm.method(&intl, "getCanonicalLocales", 1, |realm, _, args| {
        let locales = requested_locales(realm, &arg(args, 0))?;
        let values = locales.into_iter().map(|tag| Value::str(&tag)).collect();
        Ok(Value::Object(realm.array(values)))
    });
    install_segmenter(realm, &intl);
    install_collator(realm, &intl);
    install_number_format(realm, &intl);
    install_date_time_format(realm, &intl);
    install_plural_rules(realm, &intl);
    install_relative_time_format(realm, &intl);
    install_list_format(realm, &intl);
    install_display_names(realm, &intl);
    install_locale(realm, &intl);
    realm.set_global("Intl", Value::Object(intl));
}

// Segmenter -----------------------------------------------------------------

struct Segmenter {
    locale: String,
    granularity: &'static str,
}

/// One segment: where it starts and ends, and for words whether it is one.
#[derive(Clone, Copy)]
struct Piece {
    start: usize,
    end: usize,
    word: bool,
}

struct Segments {
    text: crate::JsStr,
    pieces: Vec<Piece>,
    word: bool,
}

/// The segments of text at a granularity.
fn segment_text(locale: &str, granularity: &str, units: &[u16]) -> Vec<Piece> {
    if units.is_empty() {
        return Vec::new();
    }
    if let Some(icu) = icu()
        && let Some(id) = icu_locale(icu, locale).or_else(|| Some(b"\0".to_vec()))
    {
        let kind = match granularity {
            "word" => 1,
            "sentence" => 3,
            _ => 0,
        };
        let mut status = 0;
        // SAFETY: the text outlives the iterator, which is closed below.
        let iterator = unsafe {
            (icu.ubrk_open)(
                kind,
                id.as_ptr(),
                units.as_ptr(),
                units.len() as i32,
                &mut status,
            )
        };
        if !failed(status) && !iterator.is_null() {
            let mut pieces = Vec::new();
            // SAFETY: the iterator is live until closed.
            unsafe {
                let mut start = (icu.ubrk_first)(iterator);
                loop {
                    let end = (icu.ubrk_next)(iterator);
                    if end < 0 {
                        break;
                    }
                    // Word rule statuses of 100 and above mark words.
                    let word = kind == 1 && (icu.ubrk_get_rule_status)(iterator) >= 100;
                    pieces.push(Piece {
                        start: start as usize,
                        end: end as usize,
                        word,
                    });
                    start = end;
                }
                (icu.ubrk_close)(iterator);
            }
            return pieces;
        }
    }
    // Without ICU: code points, or runs of letters and digits as words.
    let mut pieces = Vec::new();
    let mut index = 0;
    while index < units.len() {
        let width = if (0xD800..0xDC00).contains(&units[index])
            && units
                .get(index + 1)
                .is_some_and(|next| (0xDC00..0xE000).contains(next))
        {
            2
        } else {
            1
        };
        pieces.push(Piece {
            start: index,
            end: index + width,
            word: false,
        });
        index += width;
    }
    if granularity == "grapheme" {
        return pieces;
    }
    let letter = |piece: &Piece| {
        char::decode_utf16(units[piece.start..piece.end].iter().copied())
            .next()
            .and_then(Result::ok)
            .is_some_and(|character| character.is_alphanumeric())
    };
    let mut merged: Vec<Piece> = Vec::new();
    for piece in pieces {
        let word = letter(&piece);
        match merged.last_mut() {
            Some(last) if granularity == "sentence" || (last.word && word) => last.end = piece.end,
            _ => merged.push(Piece { word, ..piece }),
        }
    }
    merged
}

fn segment_record(realm: &Realm, segments: &Segments, piece: Piece) -> Value {
    let object = realm.object();
    object.define(
        Key::name("segment"),
        Property::data(Value::String(segments.text.slice(piece.start, piece.end))),
    );
    object.define(
        Key::name("index"),
        Property::data(Value::Number(piece.start as f64)),
    );
    object.define(
        Key::name("input"),
        Property::data(Value::String(segments.text.clone())),
    );
    if segments.word {
        object.define(
            Key::name("isWordLike"),
            Property::data(Value::Boolean(piece.word)),
        );
    }
    Value::Object(object)
}

fn install_segmenter(realm: &mut Realm, intl: &Object) {
    let prototype = realm.object();
    let segments_prototype = realm.object();
    let iterator_prototype = realm.object();
    realm.method(&segments_prototype, "containing", 1, |realm, this, args| {
        let data = state::<Segments>(realm, &this, "containing")?;
        let segments = native::<Segments>(&data);
        let index = ops::to_number(realm, &arg(args, 0))?;
        let index = if index.is_nan() {
            0.0
        } else {
            crate::float::trunc(index)
        };
        if index < 0.0 || index >= segments.text.len() as f64 {
            return Ok(Value::Undefined);
        }
        let index = index as usize;
        Ok(segments
            .pieces
            .iter()
            .find(|piece| piece.start <= index && index < piece.end)
            .map_or(Value::Undefined, |piece| {
                segment_record(realm, segments, *piece)
            }))
    });
    let iterator_for = iterator_prototype.clone();
    let iterate = realm.function("[Symbol.iterator]", 0, false, move |realm, this, _| {
        let data = state::<Segments>(realm, &this, "[Symbol.iterator]")?;
        let position: Rc<dyn Any> = Rc::new(State((data, core::cell::Cell::new(0usize))));
        Ok(Value::Object(Object::new(
            Some(iterator_for.clone()),
            Kind::Native(position),
        )))
    });
    segments_prototype.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::hidden(Value::Object(iterate)),
    );
    realm.method(&iterator_prototype, "next", 0, |realm, this, _| {
        let data = state::<(Rc<dyn Any>, core::cell::Cell<usize>)>(realm, &this, "next")?;
        let (segments_data, position) = native::<(Rc<dyn Any>, core::cell::Cell<usize>)>(&data);
        let segments = native::<Segments>(segments_data);
        let index = position.get();
        let (value, done) = match segments.pieces.get(index) {
            Some(piece) => {
                position.set(index + 1);
                (segment_record(realm, segments, *piece), false)
            }
            None => (Value::Undefined, true),
        };
        Ok(record(
            realm,
            &[("value", value), ("done", Value::Boolean(done))],
        ))
    });
    let self_iterator = realm.function("[Symbol.iterator]", 0, false, |_, this, _| Ok(this));
    iterator_prototype.define(
        Key::Symbol(realm.symbols.iterator.clone()),
        Property::hidden(Value::Object(self_iterator)),
    );
    iterator_prototype.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property::hidden(Value::str("Segmenter String Iterator")),
    );
    let for_segments = segments_prototype.clone();
    let segment = realm.function("segment", 1, false, move |realm, this, args| {
        let data = state::<Segmenter>(realm, &this, "Intl.Segmenter.prototype.segment")?;
        let segmenter = native::<Segmenter>(&data);
        let text = ops::to_string(realm, &arg(args, 0))?;
        let pieces = segment_text(&segmenter.locale, segmenter.granularity, text.units());
        let segments: Rc<dyn Any> = Rc::new(State(Segments {
            text,
            pieces,
            word: segmenter.granularity == "word",
        }));
        Ok(Value::Object(Object::new(
            Some(for_segments.clone()),
            Kind::Native(segments),
        )))
    });
    prototype.define(
        Key::name("segment"),
        Property::hidden(Value::Object(segment)),
    );
    realm.method(&prototype, "resolvedOptions", 0, |realm, this, _| {
        let data = state::<Segmenter>(realm, &this, "Intl.Segmenter.prototype.resolvedOptions")?;
        let segmenter = native::<Segmenter>(&data);
        Ok(record(
            realm,
            &[
                ("locale", Value::str(&segmenter.locale)),
                ("granularity", Value::str(segmenter.granularity)),
            ],
        ))
    });
    constructor(
        realm,
        intl,
        "Segmenter",
        &prototype,
        |realm, locales, options| {
            let locale = resolve_locale(realm, locales)?;
            let granularity = match string_option(
                realm,
                options,
                "granularity",
                &["grapheme", "word", "sentence"],
            )?
            .as_deref()
            {
                Some("word") => "word",
                Some("sentence") => "sentence",
                _ => "grapheme",
            };
            Ok(Segmenter {
                locale,
                granularity,
            })
        },
    );
}

// Collator ------------------------------------------------------------------

struct Collator {
    locale: String,
    handle: Option<Handle>,
    sensitivity: String,
    numeric: bool,
    case_first: String,
    ignore_punctuation: bool,
    usage: String,
}

impl Drop for Collator {
    fn drop(&mut self) {
        if let (Some(icu), Some(handle)) = (icu(), self.handle) {
            // SAFETY: opened by `ucol_open`, closed once.
            unsafe { (icu.ucol_close)(handle) };
        }
    }
}

fn compare(realm: &mut Realm, this: &Value, args: &[Value]) -> crate::Completion {
    let data = state::<Collator>(realm, this, "compare")?;
    let collator = native::<Collator>(&data);
    let left = ops::to_string(realm, &arg(args, 0))?;
    let right = ops::to_string(realm, &arg(args, 1))?;
    let order = match (icu(), collator.handle) {
        (Some(icu), Some(handle)) => {
            // SAFETY: the collator and both texts are live.
            unsafe {
                (icu.ucol_strcoll)(
                    handle,
                    left.units().as_ptr(),
                    left.units().len() as i32,
                    right.units().as_ptr(),
                    right.units().len() as i32,
                )
            }
        }
        _ => match left.units().cmp(right.units()) {
            core::cmp::Ordering::Less => -1,
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        },
    };
    Ok(Value::Number(f64::from(order)))
}

fn install_collator(realm: &mut Realm, intl: &Object) {
    let prototype = realm.object();
    bound_getter(realm, &prototype, "compare", compare);
    realm.method(&prototype, "resolvedOptions", 0, |realm, this, _| {
        let data = state::<Collator>(realm, &this, "Intl.Collator.prototype.resolvedOptions")?;
        let collator = native::<Collator>(&data);
        Ok(record(
            realm,
            &[
                ("locale", Value::str(&collator.locale)),
                ("usage", Value::str(&collator.usage)),
                ("sensitivity", Value::str(&collator.sensitivity)),
                (
                    "ignorePunctuation",
                    Value::Boolean(collator.ignore_punctuation),
                ),
                ("collation", Value::str("default")),
                ("numeric", Value::Boolean(collator.numeric)),
                ("caseFirst", Value::str(&collator.case_first)),
            ],
        ))
    });
    constructor(realm, intl, "Collator", &prototype, new_collator);
}

/// `new Intl.Collator(locales, options)`'s state.
fn new_collator(realm: &mut Realm, locales: &Value, options: &Value) -> Result<Collator, Thrown> {
    let locale = resolve_locale(realm, locales)?;
    let usage = string_option(realm, options, "usage", &["sort", "search"])?
        .unwrap_or_else(|| "sort".into());
    let sensitivity = string_option(
        realm,
        options,
        "sensitivity",
        &["base", "accent", "case", "variant"],
    )?
    .unwrap_or_else(|| "variant".into());
    let ignore_punctuation = boolean_option(realm, options, "ignorePunctuation")?.unwrap_or(false);
    let numeric = boolean_option(realm, options, "numeric")?.unwrap_or(false);
    let case_first = string_option(realm, options, "caseFirst", &["upper", "lower", "false"])?
        .unwrap_or_else(|| "false".into());
    let handle = icu().and_then(|icu| {
        let id = icu_locale(icu, &locale).unwrap_or_else(|| b"\0".to_vec());
        let mut status = 0;
        // SAFETY: a null-terminated locale ID.
        let handle = unsafe { (icu.ucol_open)(id.as_ptr(), &mut status) };
        if failed(status) || handle.is_null() {
            return None;
        }
        const STRENGTH: i32 = 5;
        const CASE_LEVEL: i32 = 3;
        const CASE_FIRST: i32 = 2;
        const NUMERIC: i32 = 7;
        const ALTERNATE: i32 = 1;
        const ON: i32 = 17;
        let set = |attribute: i32, value: i32| {
            let mut status = 0;
            // SAFETY: the collator is live.
            unsafe { (icu.ucol_set_attribute)(handle, attribute, value, &mut status) };
        };
        match sensitivity.as_str() {
            "base" => set(STRENGTH, 0),
            "accent" => set(STRENGTH, 1),
            "case" => {
                set(STRENGTH, 0);
                set(CASE_LEVEL, ON);
            }
            _ => set(STRENGTH, 2),
        }
        if numeric {
            set(NUMERIC, ON);
        }
        match case_first.as_str() {
            "upper" => set(CASE_FIRST, 25),
            "lower" => set(CASE_FIRST, 24),
            _ => {}
        }
        if ignore_punctuation {
            set(ALTERNATE, 20);
        }
        Some(handle)
    });
    Ok(Collator {
        locale,
        handle,
        sensitivity,
        numeric,
        case_first,
        ignore_punctuation,
        usage,
    })
}

/// `String.prototype.localeCompare` as ECMA-402 defines it: the order of
/// `new Intl.Collator(locales, options).compare(left, right)`. `None` when
/// ICU is unavailable. The default collator, which sorts use, is kept.
pub(crate) fn locale_compare(
    realm: &mut Realm,
    left: &[u16],
    right: &[u16],
    locales: &Value,
    options: &Value,
) -> Result<Option<i32>, Thrown> {
    let Some(icu) = icu() else {
        return Ok(None);
    };
    let defaults = matches!(locales, Value::Undefined) && matches!(options, Value::Undefined);
    // A collator made for other locales or options closes when dropped.
    let mut _owned = None;
    let handle = match realm.default_collator {
        Some(handle) if defaults => handle as Handle,
        _ => {
            let mut collator = new_collator(realm, locales, options)?;
            let Some(handle) = collator.handle else {
                return Ok(None);
            };
            if defaults {
                // Kept open for the rest of the process.
                collator.handle = None;
                realm.default_collator = Some(handle as usize);
            }
            _owned = Some(collator);
            handle
        }
    };
    // SAFETY: the collator and both texts are live.
    let order = unsafe {
        (icu.ucol_strcoll)(
            handle,
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
        )
    };
    Ok(Some(order))
}

// NumberFormat --------------------------------------------------------------

struct NumberFormat {
    locale: String,
    handle: Option<Handle>,
    resolved: Vec<(&'static str, Value)>,
    /// Fraction digits for the fallback formatting.
    maximum_fraction: u32,
    percent: bool,
}

impl Drop for NumberFormat {
    fn drop(&mut self) {
        if let (Some(icu), Some(handle)) = (icu(), self.handle) {
            // SAFETY: opened by `unumf_openForSkeletonAndLocale`, closed once.
            unsafe { (icu.unumf_close)(handle) };
        }
    }
}

/// The ICU measure type of a unit ECMA-402 sanctions.
fn unit_type(unit: &str) -> Option<&'static str> {
    Some(match unit {
        "acre" | "hectare" => "area",
        "bit" | "byte" | "gigabit" | "gigabyte" | "kilobit" | "kilobyte" | "megabit"
        | "megabyte" | "petabyte" | "terabit" | "terabyte" => "digital",
        "celsius" | "fahrenheit" => "temperature",
        "centimeter" | "foot" | "inch" | "kilometer" | "meter" | "mile" | "mile-scandinavian"
        | "millimeter" | "yard" => "length",
        "day" | "hour" | "microsecond" | "millisecond" | "minute" | "month" | "nanosecond"
        | "second" | "week" | "year" => "duration",
        "degree" => "angle",
        "fluid-ounce" | "gallon" | "liter" | "milliliter" => "volume",
        "gram" | "kilogram" | "ounce" | "pound" | "stone" => "mass",
        "percent" => "concentr",
        _ => return None,
    })
}

/// Digits of a currency's minor unit, as ISO 4217 gives them.
fn currency_digits(code: &str) -> u32 {
    match code {
        "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF" | "UGX"
        | "UYI" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => 0,
        "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => 3,
        _ => 2,
    }
}

fn format_number(realm: &mut Realm, this: &Value, args: &[Value]) -> crate::Completion {
    let data = state::<NumberFormat>(realm, this, "format")?;
    let format = native::<NumberFormat>(&data);
    let value = arg(args, 0);
    let number = match &value {
        Value::BigInt(_) => {
            let text = Value::String(ops::to_string(realm, &value)?);
            ops::to_number(realm, &text)?
        }
        _ => ops::to_number(realm, &value)?,
    };
    if let (Some(icu), Some(handle)) = (icu(), format.handle) {
        let mut status = 0;
        // SAFETY: the result is closed below; the formatter is live.
        let result = unsafe { (icu.unumf_open_result)(&mut status) };
        if !failed(status) && !result.is_null() {
            let text = {
                let mut status = 0;
                // SAFETY: the formatter and result are live.
                unsafe { (icu.unumf_format_double)(handle, number, result, &mut status) };
                if failed(status) {
                    None
                } else {
                    // SAFETY: the result is live; the buffer is writable.
                    text_from(|buffer, capacity, status| unsafe {
                        (icu.unumf_result_to_string)(result, buffer, capacity, status)
                    })
                }
            };
            // SAFETY: opened above.
            unsafe { (icu.unumf_close_result)(result) };
            if let Some(text) = text {
                return Ok(Value::String(crate::JsStr::from_units(&text)));
            }
        }
    }
    let shown = if format.percent {
        number * 100.0
    } else {
        number
    };
    let digits = format.maximum_fraction.min(20);
    let text = format!("{:.*}", digits as usize, shown);
    let text = text.trim_end_matches('0').trim_end_matches('.').to_owned();
    Ok(Value::str(&if format.percent {
        format!("{text}%")
    } else {
        text
    }))
}

fn install_number_format(realm: &mut Realm, intl: &Object) {
    let prototype = realm.object();
    bound_getter(realm, &prototype, "format", format_number);
    realm.method(&prototype, "resolvedOptions", 0, |realm, this, _| {
        let data =
            state::<NumberFormat>(realm, &this, "Intl.NumberFormat.prototype.resolvedOptions")?;
        let format = native::<NumberFormat>(&data);
        let mut fields = vec![
            ("locale", Value::str(&format.locale)),
            ("numberingSystem", Value::str("latn")),
        ];
        fields.extend(format.resolved.iter().cloned());
        Ok(record(realm, &fields))
    });
    constructor(
        realm,
        intl,
        "NumberFormat",
        &prototype,
        |realm, locales, options| {
            let locale = resolve_locale(realm, locales)?;
            let style = string_option(
                realm,
                options,
                "style",
                &["decimal", "percent", "currency", "unit"],
            )?
            .unwrap_or_else(|| "decimal".into());
            let currency = string_option(realm, options, "currency", &[])?
                .map(|code| code.to_ascii_uppercase());
            if style == "currency" && currency.is_none() {
                return Err(realm.throw(
                    ErrorKind::Type,
                    "Currency code is required with currency style.",
                ));
            }
            let currency_display = string_option(
                realm,
                options,
                "currencyDisplay",
                &["code", "symbol", "narrowSymbol", "name"],
            )?
            .unwrap_or_else(|| "symbol".into());
            let currency_sign =
                string_option(realm, options, "currencySign", &["standard", "accounting"])?
                    .unwrap_or_else(|| "standard".into());
            let unit = string_option(realm, options, "unit", &[])?;
            if style == "unit" && unit.is_none() {
                return Err(realm.throw(ErrorKind::Type, "Unit is required with unit style."));
            }
            let unit_display =
                string_option(realm, options, "unitDisplay", &["short", "narrow", "long"])?
                    .unwrap_or_else(|| "short".into());
            let notation = string_option(
                realm,
                options,
                "notation",
                &["standard", "scientific", "engineering", "compact"],
            )?
            .unwrap_or_else(|| "standard".into());
            let compact_display =
                string_option(realm, options, "compactDisplay", &["short", "long"])?
                    .unwrap_or_else(|| "short".into());
            let sign_display = string_option(
                realm,
                options,
                "signDisplay",
                &["auto", "never", "always", "exceptZero", "negative"],
            )?
            .unwrap_or_else(|| "auto".into());
            let grouping = if matches!(options, Value::Object(_)) {
                let value = ops::get(realm, options, &Key::name("useGrouping"))?;
                match value {
                    Value::Undefined => "auto".to_owned(),
                    Value::Boolean(false) => "false".to_owned(),
                    Value::Boolean(true) => "always".to_owned(),
                    other => ops::to_string(realm, &other)?.to_rust_lossy(),
                }
            } else {
                "auto".to_owned()
            };
            let minimum_integer =
                number_option(realm, options, "minimumIntegerDigits", 1.0, 21.0)?.unwrap_or(1);
            let default_fraction = match (&style[..], &currency) {
                ("currency", Some(code)) => (currency_digits(code), currency_digits(code)),
                ("percent", _) => (0, 0),
                _ => (0, 3),
            };
            let minimum_fraction =
                number_option(realm, options, "minimumFractionDigits", 0.0, 100.0)?;
            let maximum_fraction =
                number_option(realm, options, "maximumFractionDigits", 0.0, 100.0)?;
            let (minimum_fraction, maximum_fraction) = match (minimum_fraction, maximum_fraction) {
                (Some(low), Some(high)) if low > high => {
                    return Err(realm.throw(
                        ErrorKind::Range,
                        "maximumFractionDigits value is out of range.",
                    ));
                }
                (Some(low), Some(high)) => (low, high),
                (Some(low), None) => (low, low.max(default_fraction.1)),
                (None, Some(high)) => (default_fraction.0.min(high), high),
                (None, None) => default_fraction,
            };
            let minimum_significant =
                number_option(realm, options, "minimumSignificantDigits", 1.0, 21.0)?;
            let maximum_significant =
                number_option(realm, options, "maximumSignificantDigits", 1.0, 21.0)?;
            // The ICU number skeleton for the options, as V8 builds it.
            let mut skeleton = Vec::new();
            match style.as_str() {
                "percent" => skeleton.push("percent scale/100".to_owned()),
                "currency" => {
                    skeleton.push(format!("currency/{}", currency.as_deref().unwrap_or("USD")));
                    match currency_display.as_str() {
                        "code" => skeleton.push("unit-width-iso-code".into()),
                        "name" => skeleton.push("unit-width-full-name".into()),
                        "narrowSymbol" => skeleton.push("unit-width-narrow".into()),
                        _ => {}
                    }
                }
                "unit" => {
                    let unit_name = unit.as_deref().unwrap_or("");
                    let (numerator, denominator) = match unit_name.split_once("-per-") {
                        Some((numerator, denominator)) => (numerator, Some(denominator)),
                        None => (unit_name, None),
                    };
                    let measure = |name: &str| unit_type(name).map(|kind| format!("{kind}-{name}"));
                    let (Some(numerator), denominator) =
                        (measure(numerator), denominator.map(measure))
                    else {
                        return Err(realm.throw(
                            ErrorKind::Range,
                            &format!("Invalid unit argument '{unit_name}'"),
                        ));
                    };
                    skeleton.push(format!("measure-unit/{numerator}"));
                    match denominator {
                        Some(Some(denominator)) => {
                            skeleton.push(format!("per-measure-unit/{denominator}"))
                        }
                        Some(None) => {
                            return Err(realm.throw(
                                ErrorKind::Range,
                                &format!("Invalid unit argument '{unit_name}'"),
                            ));
                        }
                        None => {}
                    }
                    match unit_display.as_str() {
                        "long" => skeleton.push("unit-width-full-name".into()),
                        "narrow" => skeleton.push("unit-width-narrow".into()),
                        _ => skeleton.push("unit-width-short".into()),
                    }
                }
                _ => {}
            }
            match notation.as_str() {
                "scientific" => skeleton.push("scientific".into()),
                "engineering" => skeleton.push("engineering".into()),
                "compact" => skeleton.push(
                    if compact_display == "long" {
                        "compact-long"
                    } else {
                        "compact-short"
                    }
                    .into(),
                ),
                _ => {}
            }
            match (minimum_significant, maximum_significant) {
                (None, None) if notation == "compact" => {}
                (None, None) => {
                    let mut precision = String::from(".");
                    precision.push_str(&"0".repeat(minimum_fraction as usize));
                    precision.push_str(&"#".repeat((maximum_fraction - minimum_fraction) as usize));
                    skeleton.push(if precision == "." {
                        "precision-integer".into()
                    } else {
                        precision
                    });
                }
                (low, high) => {
                    let low = low.unwrap_or(1);
                    let high = high.unwrap_or(21).max(low);
                    let mut precision = "@".repeat(low as usize);
                    precision.push_str(&"#".repeat((high - low) as usize));
                    skeleton.push(precision);
                }
            }
            if minimum_integer > 1 {
                skeleton.push(format!(
                    "integer-width/+{}",
                    "0".repeat(minimum_integer as usize)
                ));
            }
            match grouping.as_str() {
                "false" => skeleton.push("group-off".into()),
                "min2" => skeleton.push("group-min2".into()),
                "always" => skeleton.push("group-on-aligned".into()),
                _ => {}
            }
            let accounting = currency_sign == "accounting";
            match (sign_display.as_str(), accounting) {
                ("always", false) => skeleton.push("sign-always".into()),
                ("always", true) => skeleton.push("sign-accounting-always".into()),
                ("never", _) => skeleton.push("sign-never".into()),
                ("exceptZero", false) => skeleton.push("sign-except-zero".into()),
                ("exceptZero", true) => skeleton.push("sign-accounting-except-zero".into()),
                ("negative", _) => skeleton.push("sign-negative".into()),
                (_, true) => skeleton.push("sign-accounting".into()),
                _ => {}
            }
            // ECMA-402 rounds half away from zero; ICU rounds half to even.
            skeleton.push("rounding-mode-half-up".into());
            let skeleton: Vec<u16> = skeleton.join(" ").encode_utf16().collect();
            let handle = icu().and_then(|icu| {
                let id = icu_locale(icu, &locale).unwrap_or_else(|| b"\0".to_vec());
                let mut status = 0;
                // SAFETY: the skeleton and ID are live for the call.
                let handle = unsafe {
                    (icu.unumf_open)(
                        skeleton.as_ptr(),
                        skeleton.len() as i32,
                        id.as_ptr(),
                        &mut status,
                    )
                };
                (!failed(status) && !handle.is_null()).then_some(handle)
            });
            if icu().is_some() && handle.is_none() && style == "unit" {
                return Err(realm.throw(
                    ErrorKind::Range,
                    &format!("Invalid unit argument '{}'", unit.unwrap_or_default()),
                ));
            }
            let mut resolved = vec![("style", Value::str(&style))];
            if let Some(currency) = &currency
                && style == "currency"
            {
                resolved.push(("currency", Value::str(currency)));
                resolved.push(("currencyDisplay", Value::str(&currency_display)));
                resolved.push(("currencySign", Value::str(&currency_sign)));
            }
            if let Some(unit) = &unit
                && style == "unit"
            {
                resolved.push(("unit", Value::str(unit)));
                resolved.push(("unitDisplay", Value::str(&unit_display)));
            }
            resolved.push((
                "minimumIntegerDigits",
                Value::Number(f64::from(minimum_integer)),
            ));
            resolved.push((
                "minimumFractionDigits",
                Value::Number(f64::from(minimum_fraction)),
            ));
            resolved.push((
                "maximumFractionDigits",
                Value::Number(f64::from(maximum_fraction)),
            ));
            if let Some(low) = minimum_significant {
                resolved.push(("minimumSignificantDigits", Value::Number(f64::from(low))));
            }
            if let Some(high) = maximum_significant {
                resolved.push(("maximumSignificantDigits", Value::Number(f64::from(high))));
            }
            resolved.push((
                "useGrouping",
                match grouping.as_str() {
                    "false" => Value::Boolean(false),
                    other => Value::str(other),
                },
            ));
            resolved.push(("notation", Value::str(&notation)));
            if notation == "compact" {
                resolved.push(("compactDisplay", Value::str(&compact_display)));
            }
            resolved.push(("signDisplay", Value::str(&sign_display)));
            Ok(NumberFormat {
                locale,
                handle,
                resolved,
                maximum_fraction,
                percent: style == "percent",
            })
        },
    );
}

// DateTimeFormat ------------------------------------------------------------

struct DateTimeFormat {
    locale: String,
    handle: Option<Handle>,
    time_zone: String,
    resolved: Vec<(&'static str, Value)>,
}

impl Drop for DateTimeFormat {
    fn drop(&mut self) {
        if let (Some(icu), Some(handle)) = (icu(), self.handle) {
            // SAFETY: opened by `udat_open`, closed once.
            unsafe { (icu.udat_close)(handle) };
        }
    }
}

fn format_date(realm: &mut Realm, this: &Value, args: &[Value]) -> crate::Completion {
    let data = state::<DateTimeFormat>(realm, this, "format")?;
    let format = native::<DateTimeFormat>(&data);
    let time = match arg(args, 0) {
        Value::Undefined => crate::date_builtin::now(),
        Value::Object(object) if matches!(object.data().kind, Kind::Date(_)) => {
            match object.data().kind {
                Kind::Date(time) => time,
                _ => f64::NAN,
            }
        }
        value => ops::to_number(realm, &value)?,
    };
    if !time.is_finite() {
        return Err(realm.throw(ErrorKind::Range, "Invalid time value"));
    }
    if let (Some(icu), Some(handle)) = (icu(), format.handle) {
        // SAFETY: the formatter is live; the buffer is writable.
        let text = text_from(|buffer, capacity, status| unsafe {
            (icu.udat_format)(
                handle,
                time,
                buffer,
                capacity,
                core::ptr::null_mut(),
                status,
            )
        });
        if let Some(text) = text {
            return Ok(Value::String(crate::JsStr::from_units(&text)));
        }
    }
    // Without ICU, the date's ISO text.
    let date = Value::Object(Object::new(
        Some(realm.date_prototype.clone()),
        Kind::Date(time),
    ));
    let method = ops::get(realm, &date, &Key::name("toISOString"))?;
    ops::call(realm, &method, date, &[])
}

fn install_date_time_format(realm: &mut Realm, intl: &Object) {
    let prototype = realm.object();
    bound_getter(realm, &prototype, "format", format_date);
    realm.method(&prototype, "resolvedOptions", 0, |realm, this, _| {
        let data = state::<DateTimeFormat>(
            realm,
            &this,
            "Intl.DateTimeFormat.prototype.resolvedOptions",
        )?;
        let format = native::<DateTimeFormat>(&data);
        let mut fields = vec![
            ("locale", Value::str(&format.locale)),
            ("calendar", Value::str("gregory")),
            ("numberingSystem", Value::str("latn")),
            ("timeZone", Value::str(&format.time_zone)),
        ];
        fields.extend(format.resolved.iter().cloned());
        Ok(record(realm, &fields))
    });
    constructor(
        realm,
        intl,
        "DateTimeFormat",
        &prototype,
        |realm, locales, options| date_time_format(realm, locales, options, "date"),
    );
}

/// A date formatter for options; `defaults` names the fields shown when
/// none are requested: `date`, `time` or `all`, as `toLocaleDateString`,
/// `toLocaleTimeString` and `toLocaleString` default.
fn date_time_format(
    realm: &mut Realm,
    locales: &Value,
    options: &Value,
    defaults: &str,
) -> Result<DateTimeFormat, Thrown> {
    let locale = resolve_locale(realm, locales)?;
    let time_zone = string_option(realm, options, "timeZone", &[])?;
    let hour12 = boolean_option(realm, options, "hour12")?;
    let hour_cycle = string_option(realm, options, "hourCycle", &["h11", "h12", "h23", "h24"])?;
    let date_style = string_option(
        realm,
        options,
        "dateStyle",
        &["full", "long", "medium", "short"],
    )?;
    let time_style = string_option(
        realm,
        options,
        "timeStyle",
        &["full", "long", "medium", "short"],
    )?;
    let mut fields: Vec<(&'static str, String)> = Vec::new();
    let mut skeleton = String::new();
    let mut two_digit_hour = false;
    let components: [(&'static str, &[&str]); 11] = [
        ("era", &["narrow", "short", "long"]),
        ("year", &["numeric", "2-digit"]),
        ("month", &["numeric", "2-digit", "narrow", "short", "long"]),
        ("weekday", &["narrow", "short", "long"]),
        ("day", &["numeric", "2-digit"]),
        ("dayPeriod", &["narrow", "short", "long"]),
        ("hour", &["numeric", "2-digit"]),
        ("minute", &["numeric", "2-digit"]),
        ("second", &["numeric", "2-digit"]),
        ("fractionalSecondDigits", &[]),
        (
            "timeZoneName",
            &[
                "short",
                "long",
                "shortOffset",
                "longOffset",
                "shortGeneric",
                "longGeneric",
            ],
        ),
    ];
    for (name, allowed) in components {
        if name == "fractionalSecondDigits" {
            if let Some(digits) = number_option(realm, options, name, 1.0, 3.0)? {
                skeleton.push_str(&"S".repeat(digits as usize));
                fields.push((name, digits.to_string()));
            }
            continue;
        }
        let Some(value) = string_option(realm, options, name, allowed)? else {
            continue;
        };
        let pattern = match (name, value.as_str()) {
            ("era", "narrow") => "GGGGG",
            ("era", "long") => "GGGG",
            ("era", _) => "G",
            ("year", "2-digit") => "yy",
            ("year", _) => "y",
            ("month", "numeric") => "M",
            ("month", "2-digit") => "MM",
            ("month", "narrow") => "MMMMM",
            ("month", "short") => "MMM",
            ("month", _) => "MMMM",
            ("weekday", "narrow") => "EEEEE",
            ("weekday", "short") => "EEE",
            ("weekday", _) => "EEEE",
            ("day", "2-digit") => "dd",
            ("day", _) => "d",
            ("dayPeriod", "narrow") => "BBBBB",
            ("dayPeriod", "long") => "BBBB",
            ("dayPeriod", _) => "B",
            ("hour", digits) => {
                let letter = match (hour_cycle.as_deref(), hour12) {
                    (_, Some(true)) => "h",
                    (_, Some(false)) => "H",
                    (Some("h11"), None) => "K",
                    (Some("h12"), None) => "h",
                    (Some("h23"), None) => "H",
                    (Some("h24"), None) => "k",
                    _ => "j",
                };
                if digits == "2-digit" {
                    skeleton.push_str(letter);
                    two_digit_hour = true;
                }
                letter
            }
            ("minute", "2-digit") => "mm",
            ("minute", _) => "m",
            ("second", "2-digit") => "ss",
            ("second", _) => "s",
            ("timeZoneName", "long") => "zzzz",
            ("timeZoneName", "shortOffset") => "O",
            ("timeZoneName", "longOffset") => "OOOO",
            ("timeZoneName", "shortGeneric") => "v",
            ("timeZoneName", "longGeneric") => "vvvv",
            ("timeZoneName", _) => "z",
            _ => "",
        };
        skeleton.push_str(pattern);
        fields.push((name, value));
    }
    if (date_style.is_some() || time_style.is_some()) && !fields.is_empty() {
        return Err(realm.throw(
            ErrorKind::Type,
            "Can't set option dateStyle or timeStyle with other fields",
        ));
    }
    if fields.is_empty() && date_style.is_none() && time_style.is_none() {
        let hour = match hour12 {
            Some(true) => "h",
            Some(false) => "H",
            None => "j",
        };
        match defaults {
            "time" => {
                skeleton = format!("{hour}ms");
                fields = vec![
                    ("hour", "numeric".into()),
                    ("minute", "2-digit".into()),
                    ("second", "2-digit".into()),
                ];
            }
            "all" => {
                skeleton = format!("yMd{hour}ms");
                fields = vec![
                    ("year", "numeric".into()),
                    ("month", "numeric".into()),
                    ("day", "numeric".into()),
                    ("hour", "numeric".into()),
                    ("minute", "2-digit".into()),
                    ("second", "2-digit".into()),
                ];
            }
            _ => {
                skeleton = "yMd".into();
                fields = vec![
                    ("year", "numeric".into()),
                    ("month", "numeric".into()),
                    ("day", "numeric".into()),
                ];
            }
        }
    }
    let zone: Vec<u16> = time_zone.as_deref().unwrap_or("").encode_utf16().collect();
    let style = |style: &Option<String>| match style.as_deref() {
        Some("full") => 0,
        Some("long") => 1,
        Some("medium") => 2,
        Some("short") => 3,
        _ => -1,
    };
    let newer_hour = (hour12.is_none() && hour_cycle.is_none())
        .then(|| newer_hour_letter(&locale))
        .flatten();
    let handle = icu().and_then(|icu| {
        let id = icu_locale(icu, &locale).unwrap_or_else(|| b"\0".to_vec());
        let zone_pointer = if zone.is_empty() {
            core::ptr::null()
        } else {
            zone.as_ptr()
        };
        let best = |skeleton: &str| {
            let mut status = 0;
            // SAFETY: a null-terminated locale ID.
            let generator = unsafe { (icu.udatpg_open)(id.as_ptr(), &mut status) };
            if failed(status) || generator.is_null() {
                return None;
            }
            let skeleton: Vec<u16> = skeleton.encode_utf16().collect();
            // SAFETY: the generator and skeleton are live; the buffer is writable.
            let pattern = text_from(|buffer, capacity, status| unsafe {
                (icu.udatpg_get_best_pattern)(
                    generator,
                    skeleton.as_ptr(),
                    skeleton.len() as i32,
                    buffer,
                    capacity,
                    status,
                )
            });
            // SAFETY: opened above.
            unsafe { (icu.udatpg_close)(generator) };
            pattern
        };
        let pattern = if date_style.is_none() && time_style.is_none() {
            let mut skeleton = skeleton.clone();
            if let Some(letter) = newer_hour {
                skeleton = skeleton.replace('j', letter);
            }
            let mut pattern = best(&skeleton)?;
            // As V8 does, a two-digit hour pads the hour ICU picks.
            if two_digit_hour {
                pattern = pad_hour(&pattern);
            }
            Some(pattern)
        } else if let (Some(letter), Some(_)) = (newer_hour, &time_style) {
            // The time part of the style's pattern, made again with the
            // locale's newer hour cycle; else the style as ICU has it.
            let restyled = || {
                let time = style_pattern(icu, &id, style(&time_style), -1)?;
                let changed = best(&hour_skeleton(&time, letter))?;
                let mut whole = style_pattern(icu, &id, style(&time_style), style(&date_style))?;
                let at = whole.windows(time.len()).position(|part| part == time)?;
                whole.splice(at..at + time.len(), changed);
                Some(whole)
            };
            restyled()
        } else {
            None
        };
        let mut status = 0;
        let handle = match pattern {
            Some(pattern) => {
                const PATTERN: i32 = -2;
                // SAFETY: the pattern, locale ID and zone are live for the call.
                unsafe {
                    (icu.udat_open)(
                        PATTERN,
                        PATTERN,
                        id.as_ptr(),
                        zone_pointer,
                        zone.len() as i32,
                        pattern.as_ptr(),
                        pattern.len() as i32,
                        &mut status,
                    )
                }
            }
            // SAFETY: the locale ID and zone are live for the call.
            None => unsafe {
                (icu.udat_open)(
                    style(&time_style),
                    style(&date_style),
                    id.as_ptr(),
                    zone_pointer,
                    zone.len() as i32,
                    core::ptr::null(),
                    0,
                    &mut status,
                )
            },
        };
        (!failed(status) && !handle.is_null()).then_some(handle)
    });
    if icu().is_some() && handle.is_none() && time_zone.is_some() {
        return Err(realm.throw(
            ErrorKind::Range,
            &format!(
                "Invalid time zone specified: {}",
                time_zone.unwrap_or_default()
            ),
        ));
    }
    let mut resolved: Vec<(&'static str, Value)> = Vec::new();
    if let Some(cycle) = &hour_cycle {
        resolved.push(("hourCycle", Value::str(cycle)));
    }
    if let Some(hour12) = hour12 {
        resolved.push(("hour12", Value::Boolean(hour12)));
    }
    for (name, value) in fields {
        resolved.push((name, Value::str(&value)));
    }
    if let Some(style) = &date_style {
        resolved.push(("dateStyle", Value::str(style)));
    }
    if let Some(style) = &time_style {
        resolved.push(("timeStyle", Value::str(style)));
    }
    Ok(DateTimeFormat {
        locale,
        handle,
        time_zone: time_zone.unwrap_or_else(default_time_zone),
        resolved,
    })
}

/// The IANA name of the user's time zone, as ICU maps Windows' setting.
fn default_time_zone() -> String {
    icu()
        .and_then(|icu| {
            // SAFETY: the buffer is writable.
            text_from(|buffer, capacity, status| unsafe {
                (icu.ucal_get_default_time_zone)(buffer, capacity, status)
            })
        })
        .map_or_else(
            || "UTC".to_owned(),
            |units| String::from_utf16_lossy(&units),
        )
}

/// The hour letter of a locale's preferred hour cycle where CLDR changed
/// it after Windows' ICU 64 (CLDR 35), as Chromium's ICU has it: `H` for
/// the languages of China, `h` for Spanish and Quechua in Latin America.
/// `None` for other locales, and when the locale names its hour cycle.
fn newer_hour_letter(locale: &str) -> Option<&'static str> {
    const TWELVE_HOUR_SPANISH: [&str; 15] = [
        "419", "AR", "BO", "CL", "CR", "CU", "EC", "GT", "HN", "MX", "NI", "PE", "PY", "SV", "UY",
    ];
    let tag = LocaleTag::parse(locale)?;
    if !matches!(
        tag.language.as_str(),
        "zh" | "bo" | "ii" | "mn" | "ug" | "es" | "qu"
    ) || tag.keyword("hc").is_some()
    {
        return None;
    }
    let region = match &tag.region {
        Some(region) => region.clone(),
        None => {
            tag.transform_base(Some(|icu| icu.uloc_add_likely_subtags))?
                .region?
        }
    };
    match (tag.language.as_str(), region.as_str()) {
        ("es", region) if TWELVE_HOUR_SPANISH.contains(&region) => Some("h"),
        ("qu", "PE" | "BO" | "EC") => Some("h"),
        ("es" | "qu", _) => None,
        (_, "CN") => Some("H"),
        _ => None,
    }
}

/// The pattern ICU has for a time and date style (-1 for none).
fn style_pattern(icu: &Icu, id: &[u8], time: i32, date: i32) -> Option<Vec<u16>> {
    let mut status = 0;
    // SAFETY: a null-terminated locale ID; the default time zone.
    let format = unsafe {
        (icu.udat_open)(
            time,
            date,
            id.as_ptr(),
            core::ptr::null(),
            0,
            core::ptr::null(),
            0,
            &mut status,
        )
    };
    if failed(status) || format.is_null() {
        return None;
    }
    // SAFETY: an open format, closed once below.
    let pattern = text_from(|buffer, capacity, status| unsafe {
        (icu.udat_to_pattern)(format, 0, buffer, capacity, status)
    });
    // SAFETY: as above.
    unsafe { (icu.udat_close)(format) };
    pattern
}

/// The skeleton of a time pattern with its hours in `letter`'s cycle: its
/// field letters outside quoted text, without day periods, which a 12-hour
/// pattern ICU makes from the skeleton brings back.
fn hour_skeleton(pattern: &[u16], letter: &str) -> String {
    let mut skeleton = String::new();
    let mut quoted = false;
    for character in char::decode_utf16(pattern.iter().copied()).flatten() {
        match character {
            '\'' => quoted = !quoted,
            _ if quoted => {}
            'h' | 'H' | 'K' | 'k' => skeleton.push_str(letter),
            'a' | 'b' | 'B' => {}
            _ if character.is_ascii_alphabetic() => skeleton.push(character),
            _ => {}
        }
    }
    skeleton
}

/// A date pattern with single hour letters doubled, outside quoted text.
fn pad_hour(pattern: &[u16]) -> Vec<u16> {
    let mut padded = Vec::with_capacity(pattern.len() + 2);
    let mut quoted = false;
    let mut index = 0;
    while index < pattern.len() {
        let unit = pattern[index];
        if unit == u16::from(b'\'') {
            quoted = !quoted;
        }
        let hour = !quoted && b"hHkK".iter().any(|letter| u16::from(*letter) == unit);
        let single = hour
            && pattern.get(index + 1) != Some(&unit)
            && (index == 0 || pattern[index - 1] != unit);
        padded.push(unit);
        if single {
            padded.push(unit);
        }
        index += 1;
    }
    padded
}

// PluralRules ------------------------------------------------------------

struct PluralRules {
    locale: String,
    kind: String,
    handle: Option<Handle>,
}

impl Drop for PluralRules {
    fn drop(&mut self) {
        if let (Some(icu), Some(handle)) = (icu(), self.handle) {
            // SAFETY: opened by `uplrules_openForType`, closed once.
            unsafe { (icu.uplrules_close)(handle) };
        }
    }
}

fn install_plural_rules(realm: &mut Realm, intl: &Object) {
    let prototype = realm.object();
    realm.method(&prototype, "select", 1, |realm, this, args| {
        let data = state::<PluralRules>(realm, &this, "Intl.PluralRules.prototype.select")?;
        let rules = native::<PluralRules>(&data);
        let number = ops::to_number(realm, &arg(args, 0))?;
        if let (Some(icu), Some(handle)) = (icu(), rules.handle) {
            // SAFETY: the rules are live; the buffer is writable.
            if let Some(keyword) = text_from(|buffer, capacity, status| unsafe {
                (icu.uplrules_select)(handle, number, buffer, capacity, status)
            }) {
                return Ok(Value::String(crate::JsStr::from_units(&keyword)));
            }
        }
        Ok(Value::str(if rules.kind == "cardinal" && number == 1.0 {
            "one"
        } else {
            "other"
        }))
    });
    realm.method(&prototype, "resolvedOptions", 0, |realm, this, _| {
        let data =
            state::<PluralRules>(realm, &this, "Intl.PluralRules.prototype.resolvedOptions")?;
        let rules = native::<PluralRules>(&data);
        Ok(record(
            realm,
            &[
                ("locale", Value::str(&rules.locale)),
                ("type", Value::str(&rules.kind)),
            ],
        ))
    });
    constructor(
        realm,
        intl,
        "PluralRules",
        &prototype,
        |realm, locales, options| {
            let locale = resolve_locale(realm, locales)?;
            let kind = string_option(realm, options, "type", &["cardinal", "ordinal"])?
                .unwrap_or_else(|| "cardinal".into());
            let handle = icu().and_then(|icu| {
                let id = icu_locale(icu, &locale).unwrap_or_else(|| b"\0".to_vec());
                let mut status = 0;
                // SAFETY: a null-terminated locale ID.
                let handle = unsafe {
                    (icu.uplrules_open_for_type)(
                        id.as_ptr(),
                        i32::from(kind == "ordinal"),
                        &mut status,
                    )
                };
                (!failed(status) && !handle.is_null()).then_some(handle)
            });
            Ok(PluralRules {
                locale,
                kind,
                handle,
            })
        },
    );
}

// RelativeTimeFormat --------------------------------------------------------

struct RelativeTimeFormat {
    locale: String,
    style: String,
    numeric: String,
    handle: Option<Handle>,
}

impl Drop for RelativeTimeFormat {
    fn drop(&mut self) {
        if let (Some(icu), Some(handle)) = (icu(), self.handle) {
            // SAFETY: opened by `ureldatefmt_open`, closed once.
            unsafe { (icu.ureldatefmt_close)(handle) };
        }
    }
}

fn install_relative_time_format(realm: &mut Realm, intl: &Object) {
    let prototype = realm.object();
    realm.method(&prototype, "format", 2, |realm, this, args| {
        let data =
            state::<RelativeTimeFormat>(realm, &this, "Intl.RelativeTimeFormat.prototype.format")?;
        let format = native::<RelativeTimeFormat>(&data);
        let value = ops::to_number(realm, &arg(args, 0))?;
        let unit = ops::to_string(realm, &arg(args, 1))?.to_rust_lossy();
        let singular = unit.strip_suffix('s').unwrap_or(&unit);
        let code = match singular {
            "year" => 0,
            "quarter" => 1,
            "month" => 2,
            "week" => 3,
            "day" => 4,
            "hour" => 5,
            "minute" => 6,
            "second" => 7,
            _ => {
                return Err(realm.throw(
                    ErrorKind::Range,
                    &format!("Invalid unit argument for format() '{unit}'"),
                ));
            }
        };
        if !value.is_finite() {
            return Err(realm.throw(ErrorKind::Range, "Invalid value for format()"));
        }
        if let (Some(icu), Some(handle)) = (icu(), format.handle) {
            let numeric = format.numeric == "always";
            // SAFETY: the formatter is live; the buffer is writable.
            if let Some(text) = text_from(|buffer, capacity, status| unsafe {
                if numeric {
                    (icu.ureldatefmt_format_numeric)(handle, value, code, buffer, capacity, status)
                } else {
                    (icu.ureldatefmt_format)(handle, value, code, buffer, capacity, status)
                }
            }) {
                return Ok(Value::String(crate::JsStr::from_units(&text)));
            }
        }
        let count = value.abs();
        let plural = if count == 1.0 {
            singular.to_owned()
        } else {
            format!("{singular}s")
        };
        Ok(Value::str(
            &if value < 0.0 || (value == 0.0 && value.is_sign_negative()) {
                format!("{count} {plural} ago")
            } else {
                format!("in {count} {plural}")
            },
        ))
    });
    realm.method(&prototype, "resolvedOptions", 0, |realm, this, _| {
        let data = state::<RelativeTimeFormat>(
            realm,
            &this,
            "Intl.RelativeTimeFormat.prototype.resolvedOptions",
        )?;
        let format = native::<RelativeTimeFormat>(&data);
        Ok(record(
            realm,
            &[
                ("locale", Value::str(&format.locale)),
                ("style", Value::str(&format.style)),
                ("numeric", Value::str(&format.numeric)),
                ("numberingSystem", Value::str("latn")),
            ],
        ))
    });
    constructor(
        realm,
        intl,
        "RelativeTimeFormat",
        &prototype,
        |realm, locales, options| {
            let locale = resolve_locale(realm, locales)?;
            let style = string_option(realm, options, "style", &["long", "short", "narrow"])?
                .unwrap_or_else(|| "long".into());
            let numeric = string_option(realm, options, "numeric", &["always", "auto"])?
                .unwrap_or_else(|| "always".into());
            let width = match style.as_str() {
                "short" => 1,
                "narrow" => 2,
                _ => 0,
            };
            let handle = icu().and_then(|icu| {
                let id = icu_locale(icu, &locale).unwrap_or_else(|| b"\0".to_vec());
                let mut status = 0;
                const CAPITALIZATION_NONE: i32 = 0x100;
                // SAFETY: a null-terminated locale ID; ICU makes its own number
                // format.
                let handle = unsafe {
                    (icu.ureldatefmt_open)(
                        id.as_ptr(),
                        core::ptr::null_mut(),
                        width,
                        CAPITALIZATION_NONE,
                        &mut status,
                    )
                };
                (!failed(status) && !handle.is_null()).then_some(handle)
            });
            Ok(RelativeTimeFormat {
                locale,
                style,
                numeric,
                handle,
            })
        },
    );
}

// ListFormat ----------------------------------------------------------------

struct ListFormat {
    locale: String,
    kind: String,
    style: String,
    handle: Option<Handle>,
}

impl Drop for ListFormat {
    fn drop(&mut self) {
        if let (Some(icu), Some(handle)) = (icu(), self.handle) {
            // SAFETY: opened by `ulistfmt_openForType`, closed once.
            unsafe { (icu.ulistfmt_close)(handle) };
        }
    }
}

fn install_list_format(realm: &mut Realm, intl: &Object) {
    let prototype = realm.object();
    realm.method(&prototype, "format", 1, |realm, this, args| {
        let data = state::<ListFormat>(realm, &this, "Intl.ListFormat.prototype.format")?;
        let format = native::<ListFormat>(&data);
        let list = arg(args, 0);
        let items = if matches!(list, Value::Undefined) {
            Vec::new()
        } else {
            crate::iter::collect(realm, &list)?
        };
        let mut texts = Vec::new();
        for item in &items {
            let Value::String(text) = item else {
                return Err(realm.throw(ErrorKind::Type, "Iterable yielded a non-string value"));
            };
            texts.push(text.clone());
        }
        if let (Some(icu), Some(handle)) = (icu(), format.handle) {
            let pointers: Vec<*const u16> =
                texts.iter().map(|text| text.units().as_ptr()).collect();
            let lengths: Vec<i32> = texts.iter().map(|text| text.len() as i32).collect();
            // SAFETY: the texts outlive the call; the buffer is writable.
            if let Some(text) = text_from(|buffer, capacity, status| unsafe {
                (icu.ulistfmt_format)(
                    handle,
                    pointers.as_ptr(),
                    lengths.as_ptr(),
                    pointers.len() as i32,
                    buffer,
                    capacity,
                    status,
                )
            }) {
                return Ok(Value::String(crate::JsStr::from_units(&text)));
            }
        }
        let joined: Vec<String> = texts.iter().map(crate::JsStr::to_rust_lossy).collect();
        // English lists, the default of every locale ICU cannot list.
        let last = match format.kind.as_str() {
            "disjunction" => Some(" or "),
            "conjunction" => Some(" and "),
            _ => None,
        };
        Ok(Value::str(&match (joined.as_slice(), last) {
            ([only], _) => only.clone(),
            ([first, second], Some(word)) => format!("{first}{word}{second}"),
            ([rest @ .., final_item], Some(word)) if !rest.is_empty() => {
                format!("{},{word}{final_item}", rest.join(", "))
            }
            (all, _) => all.join(", "),
        }))
    });
    realm.method(&prototype, "resolvedOptions", 0, |realm, this, _| {
        let data = state::<ListFormat>(realm, &this, "Intl.ListFormat.prototype.resolvedOptions")?;
        let format = native::<ListFormat>(&data);
        Ok(record(
            realm,
            &[
                ("locale", Value::str(&format.locale)),
                ("type", Value::str(&format.kind)),
                ("style", Value::str(&format.style)),
            ],
        ))
    });
    constructor(
        realm,
        intl,
        "ListFormat",
        &prototype,
        |realm, locales, options| {
            let locale = resolve_locale(realm, locales)?;
            let kind = string_option(
                realm,
                options,
                "type",
                &["conjunction", "disjunction", "unit"],
            )?
            .unwrap_or_else(|| "conjunction".into());
            let style = string_option(realm, options, "style", &["long", "short", "narrow"])?
                .unwrap_or_else(|| "long".into());
            let handle = icu().and_then(|icu| {
                let id = icu_locale(icu, &locale).unwrap_or_else(|| b"\0".to_vec());
                let kind = match kind.as_str() {
                    "disjunction" => 1,
                    "unit" => 2,
                    _ => 0,
                };
                let width = match style.as_str() {
                    "short" => 1,
                    "narrow" => 2,
                    _ => 0,
                };
                let mut status = 0;
                let handle = match icu.ulistfmt_open_for_type {
                    // SAFETY: a null-terminated locale ID.
                    Some(open) => unsafe { open(id.as_ptr(), kind, width, &mut status) },
                    // Without the typed constructor, only `and` lists are ICU's.
                    // SAFETY: a null-terminated locale ID.
                    None if kind == 0 && width == 0 => unsafe {
                        (icu.ulistfmt_open)(id.as_ptr(), &mut status)
                    },
                    None => return None,
                };
                (!failed(status) && !handle.is_null()).then_some(handle)
            });
            Ok(ListFormat {
                locale,
                kind,
                style,
                handle,
            })
        },
    );
}

// DisplayNames --------------------------------------------------------------

/// The fields `Intl.DisplayNames` names for `dateTimeField`, with their ICU
/// `UDateTimePatternField`s.
const DATE_TIME_FIELDS: &[(&str, i32)] = &[
    ("era", 0),
    ("year", 1),
    ("quarter", 2),
    ("month", 3),
    ("weekOfYear", 4),
    ("weekday", 6),
    ("day", 9),
    ("dayPeriod", 10),
    ("hour", 11),
    ("minute", 12),
    ("second", 13),
    ("timeZoneName", 15),
];

/// CLDR's English names of dialects, which Windows' ICU data lacks: it
/// names `en-US` "English (United States)" where Chromium's ICU says
/// "American English".
const ENGLISH_DIALECTS: &[(&str, &str)] = &[
    ("ar-001", "Modern Standard Arabic"),
    ("de-AT", "Austrian German"),
    ("de-CH", "Swiss High German"),
    ("en-AU", "Australian English"),
    ("en-CA", "Canadian English"),
    ("en-GB", "British English"),
    ("en-US", "American English"),
    ("es-419", "Latin American Spanish"),
    ("es-ES", "European Spanish"),
    ("es-MX", "Mexican Spanish"),
    ("fa-AF", "Dari"),
    ("fr-CA", "Canadian French"),
    ("fr-CH", "Swiss French"),
    ("nds-NL", "Low Saxon"),
    ("nl-BE", "Flemish"),
    ("pt-BR", "Brazilian Portuguese"),
    ("pt-PT", "European Portuguese"),
    ("ro-MD", "Moldavian"),
    ("sw-CD", "Congo Swahili"),
    ("zh-Hans", "Simplified Chinese"),
    ("zh-Hant", "Traditional Chinese"),
];

struct DisplayNames {
    locale: String,
    kind: String,
    style: String,
    fallback: String,
    language_display: String,
    /// ICU's locale display names, which name languages, regions, scripts
    /// and calendars.
    names: Option<Handle>,
    /// ICU's date-time pattern generator, which names date-time fields.
    fields: Option<Handle>,
}

impl Drop for DisplayNames {
    fn drop(&mut self) {
        if let Some(icu) = icu() {
            // SAFETY: each handle was opened once and is closed once.
            unsafe {
                if let Some(names) = self.names {
                    (icu.uldn_close)(names);
                }
                if let Some(fields) = self.fields {
                    (icu.udatpg_close)(fields);
                }
            }
        }
    }
}

impl DisplayNames {
    /// The name of a checked code; the code itself when the locale's data
    /// has none and the fallback is the code, as ICU substitutes it, and
    /// `None` when the fallback is none.
    fn name(&self, code: &str) -> Option<Vec<u16>> {
        let icu = icu()?;
        let ascii: Vec<u8> = code.bytes().chain([0]).collect();
        let name = match self.kind.as_str() {
            "language" => {
                if self.language_display == "dialect"
                    && self.locale.split('-').next() == Some("en")
                    && let Some((_, name)) = ENGLISH_DIALECTS
                        .iter()
                        .find(|(dialect, _)| *dialect == code)
                {
                    return Some(name.encode_utf16().collect());
                }
                let names = self.names?;
                let id = icu_locale(icu, code)?;
                // SAFETY: the display names are live; the ID is
                // null-terminated and the buffer writable.
                text_from(|buffer, capacity, status| unsafe {
                    (icu.uldn_locale_display_name)(names, id.as_ptr(), buffer, capacity, status)
                })
            }
            "region" | "script" => {
                let names = self.names?;
                let display = if self.kind == "region" {
                    icu.uldn_region_display_name
                } else {
                    icu.uldn_script_display_name
                };
                // SAFETY: as above.
                text_from(|buffer, capacity, status| unsafe {
                    display(names, ascii.as_ptr(), buffer, capacity, status)
                })
            }
            "calendar" | "currency" => {
                let names = self.names?;
                // ICU keeps the legacy name of the Gregorian calendar.
                let value: Vec<u8> = match code {
                    "gregory" => b"gregorian\0".to_vec(),
                    _ => ascii,
                };
                let key: &[u8] = if self.kind == "calendar" {
                    b"calendar\0"
                } else {
                    b"currency\0"
                };
                // SAFETY: as above.
                text_from(|buffer, capacity, status| unsafe {
                    (icu.uldn_key_value_display_name)(
                        names,
                        key.as_ptr(),
                        value.as_ptr(),
                        buffer,
                        capacity,
                        status,
                    )
                })
            }
            _ => {
                let fields = self.fields?;
                let field = DATE_TIME_FIELDS.iter().find(|(name, _)| *name == code)?.1;
                let width = match self.style.as_str() {
                    "short" => 1,
                    "narrow" => 2,
                    _ => 0,
                };
                // SAFETY: the generator is live; the buffer writable.
                text_from(|buffer, capacity, status| unsafe {
                    (icu.udatpg_get_field_display_name)(
                        fields, field, width, buffer, capacity, status,
                    )
                })
            }
        };
        // ICU 64 names a currency it does not know by its code even when
        // asked not to substitute.
        let substituted = |name: &[u16]| {
            self.kind == "currency" && String::from_utf16_lossy(name).eq_ignore_ascii_case(code)
        };
        name.filter(|name| !name.is_empty() && (self.fallback == "code" || !substituted(name)))
    }
}

/// A code `Intl.DisplayNames.prototype.of` takes for `kind`: a canonical
/// language tag, other codes as given; a range error when it is not one.
fn display_code(realm: &mut Realm, kind: &str, code: &str) -> Result<String, Thrown> {
    let letters =
        |length: usize| code.len() == length && code.bytes().all(|byte| byte.is_ascii_alphabetic());
    let valid = match kind {
        // A language identifier: no extensions or private use.
        "language" => {
            if code.contains('_') || code.split('-').any(|subtag| subtag.len() == 1) {
                false
            } else {
                return canonical_tag(realm, code);
            }
        }
        "region" => {
            letters(2) || (code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_digit()))
        }
        "script" => letters(4),
        "currency" => letters(3),
        "calendar" => code.split('-').all(|part| {
            (3..=8).contains(&part.len()) && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        }),
        _ => DATE_TIME_FIELDS.iter().any(|(name, _)| *name == code),
    };
    if valid {
        Ok(code.into())
    } else {
        Err(realm.throw(ErrorKind::Range, &format!("invalid_argument: {code}")))
    }
}

fn install_display_names(realm: &mut Realm, intl: &Object) {
    let prototype = realm.object();
    realm.method(&prototype, "of", 1, |realm, this, args| {
        let data = state::<DisplayNames>(realm, &this, "Intl.DisplayNames.prototype.of")?;
        let names = native::<DisplayNames>(&data);
        let code = ops::to_string(realm, &arg(args, 0))?.to_rust_lossy();
        let code = display_code(realm, &names.kind, &code)?;
        Ok(match names.name(&code) {
            Some(name) => Value::String(crate::JsStr::from_units(&name)),
            None if names.fallback == "code" => Value::str(&code),
            None => Value::Undefined,
        })
    });
    realm.method(&prototype, "resolvedOptions", 0, |realm, this, _| {
        let data =
            state::<DisplayNames>(realm, &this, "Intl.DisplayNames.prototype.resolvedOptions")?;
        let names = native::<DisplayNames>(&data);
        let mut fields = alloc::vec![
            ("locale", Value::str(&names.locale)),
            ("style", Value::str(&names.style)),
            ("type", Value::str(&names.kind)),
            ("fallback", Value::str(&names.fallback)),
        ];
        if names.kind == "language" {
            fields.push(("languageDisplay", Value::str(&names.language_display)));
        }
        Ok(record(realm, &fields))
    });
    constructor(
        realm,
        intl,
        "DisplayNames",
        &prototype,
        |realm, locales, options| {
            let locale = resolve_locale(realm, locales)?;
            if matches!(options, Value::Undefined) {
                return Err(realm.throw(ErrorKind::Type, "Required option 'type' is missing"));
            }
            let style = string_option(realm, options, "style", &["long", "short", "narrow"])?
                .unwrap_or_else(|| "long".into());
            let kind = string_option(
                realm,
                options,
                "type",
                &[
                    "language",
                    "region",
                    "script",
                    "currency",
                    "calendar",
                    "dateTimeField",
                ],
            )?
            .ok_or_else(|| realm.throw(ErrorKind::Type, "Required option 'type' is missing"))?;
            let fallback = string_option(realm, options, "fallback", &["code", "none"])?
                .unwrap_or_else(|| "code".into());
            let language_display =
                string_option(realm, options, "languageDisplay", &["dialect", "standard"])?
                    .unwrap_or_else(|| "dialect".into());
            let id = icu().map(|icu| icu_locale(icu, &locale).unwrap_or_else(|| b"\0".to_vec()));
            let names = icu().zip(id.as_ref()).and_then(|(icu, id)| {
                const STANDARD_NAMES: i32 = 0;
                const DIALECT_NAMES: i32 = 1;
                const LENGTH_FULL: i32 = 0x200;
                const LENGTH_SHORT: i32 = 0x201;
                const SUBSTITUTE: i32 = 0x300;
                const NO_SUBSTITUTE: i32 = 0x301;
                let contexts = [
                    if language_display == "standard" {
                        STANDARD_NAMES
                    } else {
                        DIALECT_NAMES
                    },
                    if style == "long" {
                        LENGTH_FULL
                    } else {
                        LENGTH_SHORT
                    },
                    if fallback == "code" {
                        SUBSTITUTE
                    } else {
                        NO_SUBSTITUTE
                    },
                ];
                let mut status = 0;
                // SAFETY: a null-terminated locale ID and the contexts' count.
                let handle = unsafe {
                    (icu.uldn_open_for_context)(
                        id.as_ptr(),
                        contexts.as_ptr(),
                        contexts.len() as i32,
                        &mut status,
                    )
                };
                (!failed(status) && !handle.is_null()).then_some(handle)
            });
            let fields = (kind == "dateTimeField")
                .then(|| icu().zip(id.as_ref()))
                .flatten()
                .and_then(|(icu, id)| {
                    let mut status = 0;
                    // SAFETY: a null-terminated locale ID.
                    let handle = unsafe { (icu.udatpg_open)(id.as_ptr(), &mut status) };
                    (!failed(status) && !handle.is_null()).then_some(handle)
                });
            Ok(DisplayNames {
                locale,
                kind,
                style,
                fallback,
                language_display,
                names,
                fields,
            })
        },
    );
}

// Locale --------------------------------------------------------------------

/// A Unicode BCP 47 locale identifier (UTS 35), in canonical case and
/// order.
#[derive(Clone, Debug, Default, PartialEq)]
struct LocaleTag {
    language: String,
    script: Option<String>,
    region: Option<String>,
    variants: Vec<String>,
    attributes: Vec<String>,
    /// The `-u-` keywords, sorted by key; an empty value stands for `true`.
    keywords: Vec<(String, String)>,
    /// The other extensions, sorted by their singletons.
    extensions: Vec<(u8, String)>,
    private: Option<String>,
}

fn subtag(text: &str, low: usize, high: usize, valid: fn(&u8) -> bool) -> bool {
    (low..=high).contains(&text.len()) && text.as_bytes().iter().all(valid)
}

fn is_language(text: &str) -> bool {
    subtag(text, 2, 3, u8::is_ascii_alphabetic) || subtag(text, 5, 8, u8::is_ascii_alphabetic)
}

fn is_script(text: &str) -> bool {
    subtag(text, 4, 4, u8::is_ascii_alphabetic)
}

fn is_region(text: &str) -> bool {
    subtag(text, 2, 2, u8::is_ascii_alphabetic) || subtag(text, 3, 3, u8::is_ascii_digit)
}

fn is_variant(text: &str) -> bool {
    subtag(text, 5, 8, u8::is_ascii_alphanumeric)
        || (subtag(text, 4, 4, u8::is_ascii_alphanumeric) && text.as_bytes()[0].is_ascii_digit())
}

/// A `type` of the Unicode extension: one or more subtags of 3-8 letters
/// or digits, as calendar, collation and numbering system options are.
fn is_unicode_type(text: &str) -> bool {
    text.split('-')
        .all(|part| subtag(part, 3, 8, u8::is_ascii_alphanumeric))
}

impl LocaleTag {
    /// Parses a tag, `None` when it is not a well-formed Unicode BCP 47
    /// locale identifier.
    fn parse(tag: &str) -> Option<LocaleTag> {
        let lower = tag.to_ascii_lowercase();
        let mut parts = lower.split('-').peekable();
        let language = parts.next()?;
        if !is_language(language) {
            return None;
        }
        let mut parsed = LocaleTag {
            language: language.into(),
            ..LocaleTag::default()
        };
        if let Some(&part) = parts.peek()
            && is_script(part)
        {
            let (first, rest) = part.split_at(1);
            parsed.script = Some(first.to_ascii_uppercase() + rest);
            parts.next();
        }
        if let Some(&part) = parts.peek()
            && is_region(part)
        {
            parsed.region = Some(part.to_ascii_uppercase());
            parts.next();
        }
        while let Some(&part) = parts.peek()
            && is_variant(part)
        {
            if parsed.variants.iter().any(|known| known == part) {
                return None;
            }
            parsed.variants.push(part.into());
            parts.next();
        }
        let mut singletons = Vec::new();
        while let Some(singleton) = parts.next() {
            let &[letter] = singleton.as_bytes() else {
                return None;
            };
            if letter == b'x' {
                let private: Vec<&str> = parts.by_ref().collect();
                if private.is_empty()
                    || !private
                        .iter()
                        .all(|part| subtag(part, 1, 8, u8::is_ascii_alphanumeric))
                {
                    return None;
                }
                parsed.private = Some(private.join("-"));
                break;
            }
            if !letter.is_ascii_alphanumeric() || singletons.contains(&letter) {
                return None;
            }
            singletons.push(letter);
            let mut subtags = Vec::new();
            while let Some(&part) = parts.peek()
                && part.len() > 1
            {
                subtags.push(part);
                parts.next();
            }
            if subtags.is_empty() {
                return None;
            }
            if letter == b'u' {
                parsed.unicode_extension(&subtags)?;
            } else if subtags
                .iter()
                .all(|part| subtag(part, 2, 8, u8::is_ascii_alphanumeric))
            {
                parsed.extensions.push((letter, subtags.join("-")));
            } else {
                return None;
            }
        }
        parsed.variants.sort();
        parsed.extensions.sort();
        Some(parsed)
    }

    /// Reads a `-u-` extension's attributes and keywords; the first of a
    /// repeated key counts, and `true` values are left out, as UTS 35
    /// canonicalizes them.
    fn unicode_extension(&mut self, subtags: &[&str]) -> Option<()> {
        let mut index = 0;
        while let Some(part) = subtags.get(index)
            && subtag(part, 3, 8, u8::is_ascii_alphanumeric)
        {
            if !self.attributes.iter().any(|known| known == part) {
                self.attributes.push((*part).into());
            }
            index += 1;
        }
        while let Some(key) = subtags.get(index) {
            let bytes = key.as_bytes();
            if bytes.len() != 2
                || !bytes[0].is_ascii_alphanumeric()
                || !bytes[1].is_ascii_alphabetic()
            {
                return None;
            }
            index += 1;
            let start = index;
            while let Some(part) = subtags.get(index)
                && part.len() > 2
            {
                if !subtag(part, 3, 8, u8::is_ascii_alphanumeric) {
                    return None;
                }
                index += 1;
            }
            let mut value = subtags[start..index].join("-");
            if value == "true" {
                value.clear();
            }
            if self.keyword(key).is_none() {
                self.keywords.push(((*key).into(), value));
            }
        }
        self.attributes.sort();
        self.keywords.sort();
        Some(())
    }

    fn keyword(&self, key: &str) -> Option<&str> {
        self.keywords
            .iter()
            .find(|(known, _)| known == key)
            .map(|(_, value)| value.as_str())
    }

    fn set_keyword(&mut self, key: &str, value: String) {
        let value = if value == "true" {
            String::new()
        } else {
            value
        };
        match self.keywords.iter_mut().find(|(known, _)| known == key) {
            Some((_, known)) => *known = value,
            None => {
                self.keywords.push((key.into(), value));
                self.keywords.sort();
            }
        }
    }

    /// The language, script, region and variants.
    fn base_name(&self) -> String {
        let mut text = self.language.clone();
        for part in self
            .script
            .iter()
            .chain(self.region.iter())
            .chain(self.variants.iter())
        {
            text.push('-');
            text.push_str(part);
        }
        text
    }

    fn to_tag(&self) -> String {
        let mut text = self.base_name();
        let unicode = !self.attributes.is_empty() || !self.keywords.is_empty();
        let mut extensions: Vec<(u8, String)> = self.extensions.clone();
        if unicode {
            let mut parts: Vec<&str> = self.attributes.iter().map(String::as_str).collect();
            for (key, value) in &self.keywords {
                parts.push(key);
                if !value.is_empty() {
                    parts.push(value);
                }
            }
            extensions.push((b'u', parts.join("-")));
            extensions.sort();
        }
        for (singleton, subtags) in extensions {
            text.push('-');
            text.push(singleton as char);
            text.push('-');
            text.push_str(&subtags);
        }
        if let Some(private) = &self.private {
            text.push_str("-x-");
            text.push_str(private);
        }
        text
    }

    /// Takes the language, script, region and variants of another tag.
    fn with_base(&self, base: &LocaleTag) -> LocaleTag {
        LocaleTag {
            language: base.language.clone(),
            script: base.script.clone(),
            region: base.region.clone(),
            variants: base.variants.clone(),
            ..self.clone()
        }
    }

    /// Replaces deprecated language and region codes as ICU's aliases do
    /// (`iw` is `he`).
    fn canonicalize(self) -> LocaleTag {
        let base = self.transform_base(None);
        match base {
            Some(base) => self.with_base(&base),
            None => self,
        }
    }

    /// The base name through an ICU likely-subtags function (adding or
    /// removing them), or canonicalized alone.
    #[allow(clippy::type_complexity)]
    fn transform_base(
        &self,
        transform: Option<
            fn(&Icu) -> unsafe extern "C" fn(*const u8, *mut u8, i32, *mut Status) -> i32,
        >,
    ) -> Option<LocaleTag> {
        let icu = icu()?;
        // ICU's ID of `und` is empty, which `icu_locale` takes for failure.
        let base = self.base_name();
        let mut id = match base.as_str() {
            "und" => b"\0".to_vec(),
            _ => icu_locale(icu, &base)?,
        };
        if let Some(transform) = transform {
            let mut changed = alloc::vec![0u8; 160];
            let mut status = 0;
            // SAFETY: a null-terminated ID and a writable buffer of its length.
            let length = unsafe {
                transform(icu)(
                    id.as_ptr(),
                    changed.as_mut_ptr(),
                    changed.len() as i32 - 1,
                    &mut status,
                )
            };
            if failed(status) || length < 0 {
                return None;
            }
            changed.truncate(length as usize);
            changed.push(0);
            id = changed;
        }
        LocaleTag::parse(&language_tag(icu, &id)?)
    }

    /// The ICU locale ID of the whole tag, null-terminated.
    fn icu_id(&self, icu: &Icu) -> Vec<u8> {
        icu_locale(icu, &self.to_tag()).unwrap_or_else(|| b"\0".to_vec())
    }

    /// The ICU locale ID of the likely full form of the tag.
    fn likely_id(&self, icu: &Icu) -> Vec<u8> {
        let maximized = self
            .transform_base(Some(|icu| icu.uloc_add_likely_subtags))
            .map(|base| self.with_base(&base))
            .unwrap_or_else(|| self.clone());
        maximized.icu_id(icu)
    }
}

/// The strings of an ICU enumeration, which this closes.
fn enumeration(icu: &Icu, handle: Handle) -> Vec<String> {
    let mut values = Vec::new();
    if handle.is_null() {
        return values;
    }
    loop {
        let mut length = 0;
        let mut status = 0;
        // SAFETY: an open enumeration; the item lives until the next call.
        let item = unsafe { (icu.uenum_next)(handle, &mut length, &mut status) };
        if item.is_null() || failed(status) || length < 0 {
            break;
        }
        // SAFETY: ICU returns `length` bytes at `item`.
        let bytes = unsafe { core::slice::from_raw_parts(item, length as usize) };
        values.push(String::from_utf8_lossy(bytes).into_owned());
    }
    // SAFETY: opened by the caller, closed once.
    unsafe { (icu.uenum_close)(handle) };
    values
}

fn strings(realm: &Realm, values: &[String]) -> Value {
    Value::Object(realm.array(values.iter().map(|value| Value::str(value)).collect()))
}

/// The locale's text direction, as `getTextInfo` gives it.
fn text_info(realm: &Realm, tag: &LocaleTag) -> Value {
    // SAFETY: a null-terminated locale ID.
    let rtl = icu().is_some_and(|icu| unsafe {
        (icu.uloc_is_right_to_left)(tag.likely_id(icu).as_ptr()) != 0
    });
    record(
        realm,
        &[("direction", Value::str(if rtl { "rtl" } else { "ltr" }))],
    )
}

/// The locale's week: its first day, weekend days (1 is Monday, 7 Sunday)
/// and minimal days in the first week, from ICU's calendar data.
fn week_info(realm: &Realm, tag: &LocaleTag) -> Value {
    const FIRST_DAY_OF_WEEK: i32 = 1;
    const MINIMAL_DAYS: i32 = 2;
    const WEEKDAY: i32 = 0;
    // ICU numbers days from Sunday (1) to Saturday (7).
    let icu_day = |day: i32| day % 7 + 1;
    let (mut first, mut weekend, mut minimal) = (1, alloc::vec![6, 7], 1);
    if let Some(icu) = icu() {
        let id = tag.icu_id(icu);
        let mut status = 0;
        // SAFETY: a null-terminated locale ID; the default time zone.
        let calendar =
            unsafe { (icu.ucal_open)(core::ptr::null(), 0, id.as_ptr(), 0, &mut status) };
        if !failed(status) && !calendar.is_null() {
            // SAFETY: an open calendar, closed once below.
            unsafe {
                let day = (icu.ucal_get_attribute)(calendar, FIRST_DAY_OF_WEEK);
                first = if day == 1 { 7 } else { day - 1 };
                minimal = (icu.ucal_get_attribute)(calendar, MINIMAL_DAYS);
                weekend = (1..=7)
                    .filter(|&day| {
                        let mut status = 0;
                        let kind =
                            (icu.ucal_get_day_of_week_type)(calendar, icu_day(day), &mut status);
                        !failed(status) && kind != WEEKDAY
                    })
                    .collect();
                (icu.ucal_close)(calendar);
            }
        }
    }
    // Week data CLDR changed after Windows' ICU 64 (CLDR 35), as Chromium's
    // ICU has it: region, first day, weekend.
    const NEWER_WEEKS: [(&str, i32, &[i32]); 5] = [
        ("AE", 1, &[6, 7]),
        ("AR", 1, &[6, 7]),
        ("AU", 1, &[6, 7]),
        ("CN", 1, &[6, 7]),
        ("UG", 1, &[7]),
    ];
    let region = tag.region.clone().or_else(|| {
        tag.transform_base(Some(|icu| icu.uloc_add_likely_subtags))
            .and_then(|base| base.region)
    });
    if let Some((_, day, days)) = NEWER_WEEKS
        .iter()
        .find(|(known, ..)| Some(*known) == region.as_deref())
    {
        first = *day;
        weekend = days.to_vec();
    }
    const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    if let Some(day) = tag
        .keyword("fw")
        .and_then(|value| DAYS.iter().position(|day| *day == value))
    {
        first = day as i32 + 1;
    }
    let weekend = weekend
        .into_iter()
        .map(|day| Value::Number(day as f64))
        .collect();
    record(
        realm,
        &[
            ("firstDay", Value::Number(first as f64)),
            ("weekend", Value::Object(realm.array(weekend))),
            ("minimalDays", Value::Number(minimal as f64)),
        ],
    )
}

/// The hour cycles of the locale: its `hc` keyword, else the one of its
/// preferred hour pattern.
fn hour_cycles(realm: &Realm, tag: &LocaleTag) -> Value {
    if let Some(cycle) = tag.keyword("hc") {
        return strings(realm, &[cycle.into()]);
    }
    if let Some(letter) = newer_hour_letter(&tag.to_tag()) {
        let cycle = if letter == "H" { "h23" } else { "h12" };
        return strings(realm, &[cycle.into()]);
    }
    let pattern = icu().and_then(|icu| {
        let id = tag.icu_id(icu);
        let mut status = 0;
        // SAFETY: a null-terminated locale ID.
        let generator = unsafe { (icu.udatpg_open)(id.as_ptr(), &mut status) };
        if failed(status) || generator.is_null() {
            return None;
        }
        let skeleton: Vec<u16> = "j".encode_utf16().collect();
        // SAFETY: an open generator, closed once below.
        let pattern = text_from(|buffer, capacity, status| unsafe {
            (icu.udatpg_get_best_pattern)(
                generator,
                skeleton.as_ptr(),
                skeleton.len() as i32,
                buffer,
                capacity,
                status,
            )
        });
        // SAFETY: as above.
        unsafe { (icu.udatpg_close)(generator) };
        pattern
    });
    let pattern = pattern.map(|units| String::from_utf16_lossy(&units));
    let cycle = pattern
        .as_deref()
        .unwrap_or("H")
        .split('\'')
        .step_by(2)
        .flat_map(str::chars)
        .find_map(|letter| match letter {
            'h' => Some("h12"),
            'H' => Some("h23"),
            'K' => Some("h11"),
            'k' => Some("h24"),
            _ => None,
        })
        .unwrap_or("h23");
    strings(realm, &[cycle.into()])
}

/// The calendars the locale commonly uses, preferred first.
fn calendars(realm: &Realm, tag: &LocaleTag) -> Value {
    if let Some(calendar) = tag.keyword("ca") {
        return strings(realm, &[calendar.into()]);
    }
    let mut values = icu()
        .map(|icu| {
            let id = tag.icu_id(icu);
            let mut status = 0;
            // SAFETY: null-terminated key and locale ID; the enumeration
            // is closed by `enumeration`.
            let handle = unsafe {
                (icu.ucal_get_keyword_values_for_locale)(
                    c"calendar".as_ptr().cast(),
                    id.as_ptr(),
                    1,
                    &mut status,
                )
            };
            if failed(status) {
                return Vec::new();
            }
            enumeration(icu, handle)
        })
        .unwrap_or_default();
    for value in &mut values {
        match value.as_str() {
            "gregorian" => *value = "gregory".into(),
            "ethiopic-amete-alem" => *value = "ethioaa".into(),
            _ => {}
        }
    }
    if values.is_empty() {
        values.push("gregory".into());
    }
    strings(realm, &values)
}

/// The collations of the locale, in order, without `standard` and
/// `search`, which are not collation types of BCP 47.
fn collations(realm: &Realm, tag: &LocaleTag) -> Value {
    if let Some(collation) = tag.keyword("co") {
        return strings(realm, &[collation.into()]);
    }
    let mut values: Vec<String> = icu()
        .map(|icu| {
            let id = tag.icu_id(icu);
            let mut status = 0;
            // SAFETY: as for calendars.
            let handle = unsafe {
                (icu.ucol_get_keyword_values_for_locale)(
                    c"collation".as_ptr().cast(),
                    id.as_ptr(),
                    1,
                    &mut status,
                )
            };
            if failed(status) {
                return Vec::new();
            }
            enumeration(icu, handle)
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|value| value != "standard" && value != "search")
        .map(|value| match value.as_str() {
            "phonebook" => "phonebk".into(),
            "traditional" => "trad".into(),
            "dictionary" => "dict".into(),
            "gb2312han" => "gb2312".into(),
            _ => value,
        })
        .collect();
    values.sort();
    values.dedup();
    strings(realm, &values)
}

/// The locale's default numbering system.
fn numbering_systems(realm: &Realm, tag: &LocaleTag) -> Value {
    if let Some(system) = tag.keyword("nu") {
        return strings(realm, &[system.into()]);
    }
    let name = icu()
        .and_then(|icu| {
            let id = tag.icu_id(icu);
            let mut status = 0;
            // SAFETY: a null-terminated locale ID.
            let system = unsafe { (icu.unumsys_open)(id.as_ptr(), &mut status) };
            if failed(status) || system.is_null() {
                return None;
            }
            // SAFETY: an open numbering system, whose name is a
            // null-terminated string it owns; closed once.
            unsafe {
                let name = (icu.unumsys_get_name)(system);
                let name = (!name.is_null()).then(|| {
                    core::ffi::CStr::from_ptr(name.cast())
                        .to_string_lossy()
                        .into_owned()
                });
                (icu.unumsys_close)(system);
                name
            }
        })
        .unwrap_or_else(|| "latn".into());
    strings(realm, &[name])
}

/// The canonical time zones of the locale's region, sorted; undefined
/// without a region.
fn time_zones(realm: &Realm, tag: &LocaleTag) -> Value {
    const CANONICAL_LOCATION: i32 = 2;
    let Some(region) = &tag.region else {
        return Value::Undefined;
    };
    let region: Vec<u8> = region.bytes().chain([0]).collect();
    let mut zones = icu()
        .map(|icu| {
            let mut status = 0;
            // SAFETY: a null-terminated region; the enumeration is closed
            // by `enumeration`.
            let handle = unsafe {
                (icu.ucal_open_time_zone_id_enumeration)(
                    CANONICAL_LOCATION,
                    region.as_ptr(),
                    core::ptr::null(),
                    &mut status,
                )
            };
            if failed(status) {
                return Vec::new();
            }
            enumeration(icu, handle)
        })
        .unwrap_or_default();
    zones.sort();
    strings(realm, &zones)
}

/// A `language`, `script` or `region` option, checked with `valid`.
fn subtag_option(
    realm: &mut Realm,
    options: &Value,
    name: &str,
    valid: fn(&str) -> bool,
) -> Result<Option<String>, Thrown> {
    let Some(value) = string_option(realm, options, name, &[])? else {
        return Ok(None);
    };
    if !valid(&value) {
        return Err(realm.throw(ErrorKind::Range, "Incorrect locale information provided"));
    }
    Ok(Some(value.to_ascii_lowercase()))
}

/// The tag of `new Intl.Locale(tag, options)`.
fn new_locale(realm: &mut Realm, tag: &Value, options: &Value) -> Result<LocaleTag, Thrown> {
    let locale = if let Value::Object(object) = tag
        && let Kind::Native(data) = &object.data().kind
        && data.is::<State<LocaleTag>>()
    {
        Some(native::<LocaleTag>(data).to_tag())
    } else {
        None
    };
    let text = match (locale, tag) {
        (Some(locale), _) => locale,
        (None, Value::Object(_)) => ops::to_string(realm, tag)?.to_rust_lossy(),
        (None, Value::String(text)) => text.to_rust_lossy(),
        _ => {
            return Err(realm.throw(
                ErrorKind::Type,
                "First argument to Intl.Locale constructor can't be empty or missing",
            ));
        }
    };
    let Some(mut locale) = LocaleTag::parse(&text) else {
        return Err(realm.throw(ErrorKind::Range, "Incorrect locale information provided"));
    };
    if matches!(options, Value::Null) {
        return Err(realm.throw(
            ErrorKind::Type,
            "Cannot convert undefined or null to object",
        ));
    }
    if let Some(language) = subtag_option(realm, options, "language", is_language)? {
        locale.language = language;
    }
    if let Some(script) = subtag_option(realm, options, "script", is_script)? {
        let (first, rest) = script.split_at(1);
        locale.script = Some(first.to_ascii_uppercase() + rest);
    }
    if let Some(region) = subtag_option(realm, options, "region", is_region)? {
        locale.region = Some(region.to_ascii_uppercase());
    }
    for (name, key) in [("calendar", "ca"), ("collation", "co")] {
        if let Some(value) = subtag_option(realm, options, name, is_unicode_type)? {
            locale.set_keyword(key, value);
        }
    }
    for (name, key, allowed) in [
        ("hourCycle", "hc", &["h11", "h12", "h23", "h24"][..]),
        ("caseFirst", "kf", &["upper", "lower", "false"][..]),
    ] {
        if let Some(value) = string_option(realm, options, name, &[])? {
            if !allowed.contains(&value.as_str()) {
                return Err(realm.throw(
                    ErrorKind::Range,
                    &format!("Value {value} out of range for locale options property {name}"),
                ));
            }
            locale.set_keyword(key, value);
        }
    }
    if let Some(numeric) = boolean_option(realm, options, "numeric")? {
        locale.set_keyword("kn", if numeric { "true" } else { "false" }.into());
    }
    if let Some(value) = subtag_option(realm, options, "numberingSystem", is_unicode_type)? {
        locale.set_keyword("nu", value);
    }
    Ok(locale.canonicalize())
}

fn install_locale(realm: &mut Realm, intl: &Object) {
    type Info = fn(&Realm, &LocaleTag) -> Value;
    let prototype = realm.object();
    let tag_of = |realm: &Realm, this: &Value, what: &str| -> Result<LocaleTag, Thrown> {
        let data = state::<LocaleTag>(realm, this, what)?;
        Ok(native::<LocaleTag>(&data).clone())
    };
    let getters: [(&str, Info); 10] = [
        ("baseName", |_, tag| Value::str(&tag.base_name())),
        ("language", |_, tag| Value::str(&tag.language)),
        ("script", |_, tag| {
            tag.script.as_deref().map_or(Value::Undefined, Value::str)
        }),
        ("region", |_, tag| {
            tag.region.as_deref().map_or(Value::Undefined, Value::str)
        }),
        ("calendar", |_, tag| {
            tag.keyword("ca").map_or(Value::Undefined, Value::str)
        }),
        ("caseFirst", |_, tag| {
            tag.keyword("kf").map_or(Value::Undefined, Value::str)
        }),
        ("collation", |_, tag| {
            tag.keyword("co").map_or(Value::Undefined, Value::str)
        }),
        ("hourCycle", |_, tag| {
            tag.keyword("hc").map_or(Value::Undefined, Value::str)
        }),
        ("numeric", |_, tag| {
            Value::Boolean(matches!(tag.keyword("kn"), Some("" | "true")))
        }),
        ("numberingSystem", |_, tag| {
            tag.keyword("nu").map_or(Value::Undefined, Value::str)
        }),
    ];
    let infos: [(&str, &str, Info); 7] = [
        ("calendars", "getCalendars", calendars),
        ("collations", "getCollations", collations),
        ("hourCycles", "getHourCycles", hour_cycles),
        ("numberingSystems", "getNumberingSystems", numbering_systems),
        ("textInfo", "getTextInfo", text_info),
        ("timeZones", "getTimeZones", time_zones),
        ("weekInfo", "getWeekInfo", week_info),
    ];
    // Chromium has the info both as methods and, as before, as getters.
    for (_, method, info) in infos {
        let what = format!("Intl.Locale.prototype.{method}");
        realm.method(&prototype, method, 0, move |realm, this, _| {
            Ok(info(realm, &tag_of(realm, &this, &what)?))
        });
    }
    for (name, info) in getters
        .into_iter()
        .chain(infos.into_iter().map(|(name, _, info)| (name, info)))
    {
        let what = format!("Intl.Locale.prototype.{name}");
        let getter = realm.function(&format!("get {name}"), 0, false, move |realm, this, _| {
            Ok(info(realm, &tag_of(realm, &this, &what)?))
        });
        prototype.define(
            Key::name(name),
            Property {
                slot: crate::Slot::Accessor {
                    get: Some(getter),
                    set: None,
                },
                enumerable: false,
                configurable: true,
            },
        );
    }
    let wrap = |prototype: &Object, tag: LocaleTag| {
        let data: Rc<dyn Any> = Rc::new(State(tag));
        Object::new(Some(prototype.clone()), Kind::Native(data))
    };
    realm.method(&prototype, "toString", 0, move |realm, this, _| {
        Ok(Value::str(
            &tag_of(realm, &this, "Intl.Locale.prototype.toString")?.to_tag(),
        ))
    });
    let instances = prototype.clone();
    realm.method(&prototype, "maximize", 0, move |realm, this, _| {
        let tag = tag_of(realm, &this, "Intl.Locale.prototype.maximize")?;
        let base = tag.transform_base(Some(|icu| icu.uloc_add_likely_subtags));
        let tag = base.map(|base| tag.with_base(&base)).unwrap_or(tag);
        Ok(Value::Object(wrap(&instances, tag)))
    });
    let instances = prototype.clone();
    realm.method(&prototype, "minimize", 0, move |realm, this, _| {
        let tag = tag_of(realm, &this, "Intl.Locale.prototype.minimize")?;
        let base = tag.transform_base(Some(|icu| icu.uloc_minimize_subtags));
        let tag = base.map(|base| tag.with_base(&base)).unwrap_or(tag);
        Ok(Value::Object(wrap(&instances, tag)))
    });
    prototype.define(
        Key::Symbol(realm.symbols.to_string_tag.clone()),
        Property::hidden(Value::str("Intl.Locale")),
    );
    let instances = prototype.clone();
    let function = realm.builtin_constructor(
        "Locale",
        1,
        &prototype,
        |realm, _, _| Err(realm.throw(ErrorKind::Type, "Constructor Intl.Locale requires 'new'")),
        move |realm, _, args| {
            let tag = new_locale(realm, &arg(args, 0), &arg(args, 1))?;
            Ok(wrap(&instances, tag))
        },
    );
    intl.define(
        Key::name("Locale"),
        Property::hidden(Value::Object(function)),
    );
}

#[cfg(test)]
mod tests;
