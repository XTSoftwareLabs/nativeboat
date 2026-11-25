//! Immutable UTF-16 strings. Code units are stored as they are, so unpaired
//! surrogates survive every operation, and lengths and indexes are UTF-16
//! offsets as the source language observes them.
//!
//! A string is a view of a shared buffer: substrings share their parent's
//! code units instead of copying them, as engines' sliced strings do, so
//! scanning text by repeatedly taking its rest costs no copies. String
//! literals compiled into a program live in static memory
//! ([`StaticStr`]), so using one allocates nothing.

#[allow(unused_imports)]
use crate::prelude::*;

use alloc::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use core::cell::Cell;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// The start of a shared buffer: its reference count, how many code units
/// it holds and has room for, followed by the code units. Appending to the
/// string that ends where the units do writes after them while there is
/// room; strings viewing fewer units are unaffected.
#[repr(C)]
struct Header {
    count: Cell<usize>,
    len: Cell<usize>,
    capacity: usize,
    /// The buffer's [`fingerprint`] once a table has hashed it; zero
    /// before. Property lookups hash the same keys over and over.
    hash: AtomicU64,
}

/// A seeded hash of code units, never zero. Equal units hash alike, so a
/// whole buffer's hash, cached, stands for every view of the same units.
fn fingerprint(units: &[u16]) -> u64 {
    let mut hash = crate::table::process_seed();
    for unit in units {
        hash = (hash ^ u64::from(*unit)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    (hash ^ (hash >> 32)) | 1
}

/// The header of a string in static memory. Its count is zero, which no
/// shared buffer's count is while a handle exists, so handles to it neither
/// count references nor free it.
#[repr(transparent)]
pub struct StaticHeader(Header);

/// Where reads of a property last found it, the latest two places: a slot
/// of the object read, or of the object on its prototype chain whose
/// address `holder` keeps. Lookups check the slot still holds the key, so a
/// stale entry only misses. Compiled reads of a name each have one, as
/// objects read at one place in a program are mostly alike, or of two
/// kinds, as subclasses with a method each; a static name has one for the
/// reads of runtime code.
#[derive(Debug)]
#[repr(C)]
pub struct LookupCache {
    holder: [AtomicUsize; 2],
    slot: [AtomicU32; 2],
}

impl LookupCache {
    /// An empty cache: all zeros, so a program's caches take no space in
    /// its file.
    #[allow(clippy::new_without_default)]
    pub const fn new() -> LookupCache {
        LookupCache {
            holder: [AtomicUsize::new(0), AtomicUsize::new(0)],
            slot: [AtomicU32::new(0), AtomicU32::new(0)],
        }
    }

    /// The places, latest first: the holder's address, zero for the object
    /// read itself, and the slot.
    pub(crate) fn get(&self) -> [(usize, usize); 2] {
        let entry = |index: usize| {
            (
                self.holder[index].load(Ordering::Relaxed),
                self.slot[index].load(Ordering::Relaxed) as usize,
            )
        };
        [entry(0), entry(1)]
    }

    /// Remembers a place, keeping the latest other one.
    pub(crate) fn set(&self, holder: usize, slot: usize) {
        let latest = self.get()[0];
        if latest == (holder, slot) {
            return;
        }
        self.holder[1].store(latest.0, Ordering::Relaxed);
        self.slot[1].store(latest.1 as u32, Ordering::Relaxed);
        self.holder[0].store(holder, Ordering::Relaxed);
        self.slot[0].store(slot as u32, Ordering::Relaxed);
    }
}

/// A string literal compiled into the program: a header and its units laid
/// out as a shared buffer is, after the cache of reads of the property it
/// names.
#[repr(C)]
pub struct StaticStr<const N: usize> {
    lookup: LookupCache,
    /// The header that handles to the string point to.
    pub header: StaticHeader,
    units: [u16; N],
}

// The header follows the cache directly, so the header finds it.
const _: () =
    assert!(core::mem::offset_of!(StaticStr<1>, header) == core::mem::size_of::<LookupCache>());

// SAFETY: nothing writes a static string but its atomic cache: handles skip
// counting when the count is zero.
unsafe impl<const N: usize> Sync for StaticStr<N> {}

impl<const N: usize> StaticStr<N> {
    /// The UTF-16 encoding of `text`, which has `N` code units.
    pub const fn new(text: &str) -> StaticStr<N> {
        let bytes = text.as_bytes();
        let mut units = [0; N];
        let (mut read, mut written) = (0, 0);
        while read < bytes.len() {
            let lead = bytes[read] as u32;
            let (scalar, width) = if lead < 0x80 {
                (lead, 1)
            } else if lead < 0xe0 {
                (((lead & 0x1f) << 6) | continuation(bytes, read + 1), 2)
            } else if lead < 0xf0 {
                (
                    ((lead & 0x0f) << 12)
                        | (continuation(bytes, read + 1) << 6)
                        | continuation(bytes, read + 2),
                    3,
                )
            } else {
                (
                    ((lead & 0x07) << 18)
                        | (continuation(bytes, read + 1) << 12)
                        | (continuation(bytes, read + 2) << 6)
                        | continuation(bytes, read + 3),
                    4,
                )
            };
            if scalar >= 0x1_0000 {
                let offset = scalar - 0x1_0000;
                units[written] = (0xd800 | (offset >> 10)) as u16;
                units[written + 1] = (0xdc00 | (offset & 0x3ff)) as u16;
                written += 2;
            } else {
                units[written] = scalar as u16;
                written += 1;
            }
            read += width;
        }
        assert!(written == N, "the length is the text's UTF-16 length");
        StaticStr::from_units(units)
    }

    /// A string of the given code units, which may be unpaired surrogates.
    pub const fn from_units(units: [u16; N]) -> StaticStr<N> {
        StaticStr {
            lookup: LookupCache::new(),
            header: StaticHeader(Header {
                count: Cell::new(0),
                len: Cell::new(N),
                capacity: N,
                hash: AtomicU64::new(0),
            }),
            units,
        }
    }
}

impl fmt::Debug for StaticHeader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("StaticHeader")
            .field(&self.0.len.get())
            .finish()
    }
}

impl<const N: usize> fmt::Debug for StaticStr<N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&String::from_utf16_lossy(&self.units), formatter)
    }
}

/// The payload bits of the UTF-8 continuation byte at `at`.
const fn continuation(bytes: &[u8], at: usize) -> u32 {
    (bytes[at] as u32) & 0x3f
}

impl StaticHeader {
    /// A handle to the string.
    #[inline]
    pub fn str(&'static self) -> JsStr {
        JsStr {
            buffer: Some(NonNull::from(&self.0)),
            start: 0,
            len: self.0.len.get() as u32,
        }
    }

    /// The string as a value.
    #[inline]
    pub fn value(&'static self) -> crate::Value {
        crate::Value::String(self.str())
    }

    /// The string as a property key; it must not be an array index, whose
    /// key is [`Key::Index`](crate::Key::Index).
    #[inline]
    pub fn key(&'static self) -> crate::Key {
        crate::Key::String(self.str())
    }

    /// The cache of reads of the property this string names.
    pub(crate) fn lookup(&'static self) -> &'static LookupCache {
        // SAFETY: every static header is the `header` of a `StaticStr`,
        // which lays its cache out directly before it.
        unsafe {
            &*(self as *const StaticHeader)
                .cast::<u8>()
                .sub(core::mem::size_of::<LookupCache>())
                .cast::<LookupCache>()
        }
    }
}

/// The property key of an ASCII name in static memory, made without
/// allocating, for keys runtime code uses on every call.
macro_rules! static_key {
    ($text:literal) => {{
        static TEXT: $crate::string::StaticStr<{ $text.len() }> =
            $crate::string::StaticStr::new($text);
        TEXT.header.key()
    }};
}
pub(crate) use static_key;

/// Substrings shorter than this are copied: sharing would keep a large
/// buffer alive for a few units.
const SHARED_SLICE: usize = 16;

/// A shared, immutable sequence of UTF-16 code units.
#[derive(Default)]
pub struct JsStr {
    /// The shared buffer; `None` for the empty string.
    buffer: Option<NonNull<Header>>,
    start: u32,
    len: u32,
}

/// The layout of a buffer of `len` units and the offset of its units.
/// The longest addition after which a concatenation leaves room to append.
const APPENDED_UNITS: usize = 64;

/// The least room a concatenation leaves for appending.
const MIN_ROOM: usize = 16;

fn layout(len: usize) -> (Layout, usize) {
    let (layout, offset) = Layout::new::<Header>()
        .extend(Layout::array::<u16>(len).expect("string length fits in memory"))
        .expect("string length fits in memory");
    (layout.pad_to_align(), offset)
}

impl JsStr {
    /// A string of `len` units written by `fill`, which gets them all.
    fn build(len: usize, fill: impl FnOnce(&mut [u16])) -> JsStr {
        JsStr::build_with_room(len, len, fill)
    }

    /// [`JsStr::build`] in a buffer with room for `capacity` units.
    fn build_with_room(len: usize, capacity: usize, fill: impl FnOnce(&mut [u16])) -> JsStr {
        if len == 0 {
            return JsStr::default();
        }
        let length = u32::try_from(len).expect("strings hold fewer than 2³² code units");
        let capacity = capacity.max(len);
        let (layout, offset) = layout(capacity);
        // SAFETY: the layout has a non-zero size: it holds the header.
        let memory = unsafe { alloc(layout) };
        let Some(header) = NonNull::new(memory.cast::<Header>()) else {
            handle_alloc_error(layout)
        };
        // SAFETY: the allocation holds a header followed by `len` units,
        // which `fill` initializes before anything reads them.
        unsafe {
            header.as_ptr().write(Header {
                count: Cell::new(1),
                len: Cell::new(len),
                capacity,
                hash: AtomicU64::new(0),
            });
            let units = memory.add(offset).cast::<u16>();
            units.write_bytes(0, len);
            fill(core::slice::from_raw_parts_mut(units, len));
        }
        JsStr {
            buffer: Some(header),
            start: 0,
            len: length,
        }
    }

    /// A string of the given code units.
    pub fn from_units(units: &[u16]) -> JsStr {
        JsStr::build(units.len(), |slots| slots.copy_from_slice(units))
    }

    /// The code units.
    pub fn units(&self) -> &[u16] {
        let Some(header) = self.buffer else {
            return &[];
        };
        let (_, offset) = layout(0);
        // SAFETY: the buffer is live while this handle is, and holds the
        // `start..start + len` units it was made with.
        unsafe {
            let units = header.as_ptr().cast::<u8>().add(offset).cast::<u16>();
            core::slice::from_raw_parts(units.add(self.start as usize), self.len as usize)
        }
    }

    /// The length in code units.
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether the string has no code units.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The units `start..end`: shared with this string unless they are few.
    pub fn slice(&self, start: usize, end: usize) -> JsStr {
        let units = &self.units()[start..end];
        if units.len() < SHARED_SLICE || units.len() == self.len() {
            if units.len() == self.len() {
                return self.clone();
            }
            return JsStr::from_units(units);
        }
        // The view takes over the reference the clone counted.
        let shared = core::mem::ManuallyDrop::new(self.clone());
        JsStr {
            buffer: shared.buffer,
            start: shared.start + start as u32,
            len: units.len() as u32,
        }
    }

    /// This string followed by another.
    pub fn concat(&self, other: &JsStr) -> JsStr {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        let (left, right) = (self.units(), other.units());
        let len = left.len() + right.len();
        if let Some(buffer) = self.buffer {
            // SAFETY: a handle keeps its buffer, and the header, alive.
            let header = unsafe { buffer.as_ref() };
            let end = self.start as usize + self.len as usize;
            let used = header.len.get();
            // A string that ends where its buffer's units do, with room after
            // them, grows in place, as `text += more` in a loop does.
            if header.count.get() != 0 && end == used && used + right.len() <= header.capacity {
                let (_, offset) = layout(0);
                // SAFETY: the units from `used` to the capacity are allocated
                // and no handle views them.
                unsafe {
                    let units = buffer.as_ptr().cast::<u8>().add(offset).cast::<u16>();
                    core::ptr::copy_nonoverlapping(right.as_ptr(), units.add(used), right.len());
                }
                header.len.set(used + right.len());
                // The cached hash was of the units before.
                header.hash.store(0, Ordering::Relaxed);
                header.count.set(header.count.get() + 1);
                return JsStr {
                    buffer: Some(buffer),
                    start: self.start,
                    len: u32::try_from(len).expect("strings hold fewer than 2³² code units"),
                };
            }
        }
        // Short additions, as appended pieces are, leave room for more.
        let room = if right.len() <= APPENDED_UNITS {
            len + len.max(MIN_ROOM)
        } else {
            len
        };
        JsStr::build_with_room(len, room, |slots| {
            slots[..left.len()].copy_from_slice(left);
            slots[left.len()..].copy_from_slice(right);
        })
    }

    /// The text, with each unpaired surrogate replaced by U+FFFD.
    pub fn to_rust_lossy(&self) -> String {
        String::from_utf16_lossy(self.units())
    }

    /// Whether both handles view the same units of the same storage, which
    /// implies equal contents.
    pub fn ptr_eq(&self, other: &JsStr) -> bool {
        self.buffer == other.buffer && self.start == other.start && self.len == other.len
    }
}

impl Clone for JsStr {
    fn clone(&self) -> JsStr {
        if let Some(header) = self.buffer {
            // SAFETY: the buffer is live while this handle is.
            let count = unsafe { &header.as_ref().count };
            if count.get() != 0 {
                count.set(count.get() + 1);
            }
        }
        JsStr {
            buffer: self.buffer,
            start: self.start,
            len: self.len,
        }
    }
}

impl Drop for JsStr {
    fn drop(&mut self) {
        let Some(header) = self.buffer else {
            return;
        };
        // SAFETY: the buffer is live while this handle is; the last handle
        // frees it with the layout it was made with. Static strings, whose
        // count is zero, are never freed.
        unsafe {
            let count = &header.as_ref().count;
            if count.get() == 0 {
                return;
            }
            let remaining = count.get() - 1;
            count.set(remaining);
            if remaining == 0 {
                let (layout, _) = layout(header.as_ref().capacity);
                dealloc(header.as_ptr().cast(), layout);
            }
        }
    }
}

impl PartialEq for JsStr {
    fn eq(&self, other: &JsStr) -> bool {
        self.ptr_eq(other) || self.units() == other.units()
    }
}

impl Eq for JsStr {}

impl JsStr {
    /// The units' [`fingerprint`], cached in the buffer when this string
    /// is all of it.
    pub(crate) fn fingerprint(&self) -> u64 {
        let Some(buffer) = self.buffer else {
            return fingerprint(&[]);
        };
        // SAFETY: a handle keeps its buffer, and the header, alive.
        let header = unsafe { buffer.as_ref() };
        if self.start != 0 || self.len as usize != header.len.get() {
            return fingerprint(self.units());
        }
        match header.hash.load(Ordering::Relaxed) {
            0 => {
                let hash = fingerprint(self.units());
                header.hash.store(hash, Ordering::Relaxed);
                hash
            }
            hash => hash,
        }
    }
}

impl Hash for JsStr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.fingerprint());
    }
}

impl PartialOrd for JsStr {
    fn partial_cmp(&self, other: &JsStr) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for JsStr {
    fn cmp(&self, other: &JsStr) -> core::cmp::Ordering {
        self.units().cmp(other.units())
    }
}

impl From<&str> for JsStr {
    /// Property keys come from Rust text on every access by name, so the
    /// units are written straight into one allocation of the exact length.
    fn from(text: &str) -> JsStr {
        if text.is_ascii() {
            JsStr::build(text.len(), |slots| {
                for (slot, byte) in slots.iter_mut().zip(text.bytes()) {
                    *slot = u16::from(byte);
                }
            })
        } else {
            JsStr::build(text.encode_utf16().count(), |slots| {
                for (slot, unit) in slots.iter_mut().zip(text.encode_utf16()) {
                    *slot = unit;
                }
            })
        }
    }
}

impl fmt::Debug for JsStr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.to_rust_lossy(), formatter)
    }
}

impl fmt::Display for JsStr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_rust_lossy())
    }
}

/// Whether a code unit is white space or a line terminator in the source
/// language's numeric string grammar and `trim`.
pub fn is_space(unit: u16) -> bool {
    matches!(
        unit,
        0x09..=0x0D
            | 0x20
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000
            | 0xFEFF
    )
}

#[cfg(test)]
mod tests;
