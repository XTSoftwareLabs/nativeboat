//! The runtime symbols; left out of test builds, which run on the platform
//! runtime.

use core::ffi::c_void;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ExitProcess(code: u32) -> !;
}

unsafe extern "C" {
    /// The C `main` rustc generates for a `std` program; it ignores its
    /// arguments on Windows.
    fn main(argc: i32, argv: *const *const u8) -> i32;
}

/// The entry point the linker selects for console programs.
#[unsafe(no_mangle)]
pub extern "C" fn mainCRTStartup() -> ! {
    // SAFETY: runs once, before any other code of the program.
    unsafe {
        if !initialize() {
            ExitProcess(255);
        }
        ExitProcess(main(0, core::ptr::null()) as u32)
    }
}

/// A C initializer, which reports failure with a nonzero result.
type CInitializer = Option<unsafe extern "C" fn() -> i32>;
/// A C++-style initializer.
type Initializer = Option<unsafe extern "C" fn()>;

// The linker sorts sections of the same name by the text after `$`, so the
// entries other objects place in `.CRT$XI*` and `.CRT$XC*` lie between
// these markers, as the Visual C++ runtime arranges them.
#[used]
#[unsafe(link_section = ".CRT$XIA")]
static C_FIRST: CInitializer = None;
#[used]
#[unsafe(link_section = ".CRT$XIZ")]
static C_LAST: CInitializer = None;
#[used]
#[unsafe(link_section = ".CRT$XCA")]
static FIRST: Initializer = None;
#[used]
#[unsafe(link_section = ".CRT$XCZ")]
static LAST: Initializer = None;

/// Runs the static initializers in order; returns whether all succeeded.
unsafe fn initialize() -> bool {
    let mut entry = (&raw const C_FIRST).wrapping_add(1);
    while entry < &raw const C_LAST {
        // SAFETY: the entries between the markers are initializers, or
        // zero padding.
        if let Some(initializer) = unsafe { entry.read_volatile() }
            // SAFETY: initializers run once, in order, as the compiler that
            // registered them expects.
            && unsafe { initializer() } != 0
        {
            return false;
        }
        entry = entry.wrapping_add(1);
    }
    let mut entry = (&raw const FIRST).wrapping_add(1);
    while entry < &raw const LAST {
        // SAFETY: as above.
        if let Some(initializer) = unsafe { entry.read_volatile() } {
            // SAFETY: as above.
            unsafe { initializer() };
        }
        entry = entry.wrapping_add(1);
    }
    true
}

/// A thread-local storage callback, which Windows calls as threads and the
/// process start and end.
type TlsCallback = Option<unsafe extern "system" fn(*mut c_void, u32, *mut c_void)>;

/// The image's thread-local storage directory (`IMAGE_TLS_DIRECTORY64`).
#[repr(C)]
#[derive(Debug)]
pub struct TlsDirectory {
    start: *const u8,
    end: *const u8,
    index: *const u32,
    callbacks: *const TlsCallback,
    zero_fill: u32,
    characteristics: u32,
}

// SAFETY: the directory is read only, by the loader.
unsafe impl Sync for TlsDirectory {}

/// The slot the loader stores the image's thread-local storage index in.
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static mut _tls_index: u32 = 0;

// Thread-local variables are in `.tls$`, between these markers.
// Writable, as thread-local data is.
#[used]
#[unsafe(link_section = ".tls")]
static mut TLS_START: u8 = 0;
#[used]
#[unsafe(link_section = ".tls$ZZZ")]
static mut TLS_END: u8 = 0;
// `std` registers its thread-exit callback in `.CRT$XLB`.
#[used]
#[unsafe(link_section = ".CRT$XLA")]
static CALLBACKS_FIRST: TlsCallback = None;
#[used]
#[unsafe(link_section = ".CRT$XLZ")]
static CALLBACKS_LAST: TlsCallback = None;

/// The thread-local storage directory; the linker points the image's TLS
/// data directory at the symbol of this name.
#[unsafe(no_mangle)]
#[used]
#[allow(non_upper_case_globals)]
pub static _tls_used: TlsDirectory = TlsDirectory {
    start: &raw const TLS_START,
    end: &raw const TLS_END,
    index: &raw const _tls_index,
    callbacks: (&raw const CALLBACKS_FIRST).wrapping_add(1),
    zero_fill: 0,
    characteristics: 0,
};

/// The virtual table of C++ `type_info`, which the type descriptors of
/// Rust panics on this target name. Programs abort on panic, so no
/// descriptor is ever examined.
#[unsafe(export_name = "??_7type_info@@6B@")]
pub static TYPE_INFO_VTABLE: [usize; 1] = [0];

/// `ExceptionContinueSearch`: the exception is not this frame's.
const CONTINUE_SEARCH: i32 = 1;

/// The personality of Rust frames with cleanups. Programs abort on panic,
/// so no Rust exception is ever thrown; other exceptions pass through.
#[unsafe(no_mangle)]
pub extern "C" fn __CxxFrameHandler3(
    _record: *mut c_void,
    _frame: *mut c_void,
    _context: *mut c_void,
    _dispatcher: *mut c_void,
) -> i32 {
    CONTINUE_SEARCH
}

/// The personality of frames with structured exception handlers, which
/// Rust code does not have; exceptions pass through.
#[unsafe(no_mangle)]
pub extern "C" fn __C_specific_handler(
    _record: *mut c_void,
    _frame: *mut c_void,
    _context: *mut c_void,
    _dispatcher: *mut c_void,
) -> i32 {
    CONTINUE_SEARCH
}

macro_rules! math {
    ($($name:ident($($argument:ident),*);)*) => {
        $(
            #[doc = concat!("C `", stringify!($name), "`.")]
            #[unsafe(no_mangle)]
            pub extern "C" fn $name($($argument: f64),*) -> f64 {
                libm::$name($($argument),*)
            }
        )*
    };
}

math! {
    acos(x); asin(x); atan(x); atan2(y, x); cbrt(x); ceil(x); cos(x); cosh(x);
    exp(x); exp2(x); expm1(x); floor(x); fmod(x, y); hypot(x, y); log(x);
    log10(x); log1p(x); log2(x); pow(x, y); round(x); sin(x); sinh(x);
    sqrt(x); tan(x); tanh(x); tgamma(x); lgamma(x); trunc(x); fdim(x, y);
    erf(x); erfc(x);
}

macro_rules! math_f32 {
    ($($name:ident = $libm:ident($($argument:ident),*);)*) => {
        $(
            #[doc = concat!("C `", stringify!($name), "`.")]
            #[unsafe(no_mangle)]
            pub extern "C" fn $name($($argument: f32),*) -> f32 {
                libm::$libm($($argument),*)
            }
        )*
    };
}

math_f32! {
    acosf = acosf(x); asinf = asinf(x); atanf = atanf(x); atan2f = atan2f(y, x);
    ceilf = ceilf(x); cosf = cosf(x); coshf = coshf(x); expf = expf(x);
    floorf = floorf(x); fmodf = fmodf(x, y); logf = logf(x); log10f = log10f(x);
    log2f = log2f(x); powf = powf(x, y); sinf = sinf(x); sinhf = sinhf(x);
    tanf = tanf(x); tanhf = tanhf(x); truncf = truncf(x); roundf = roundf(x);
}
