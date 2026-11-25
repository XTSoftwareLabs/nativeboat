//! Process startup for executables built by nativeboat, in place of a C
//! runtime and Rust `std`: the entry point, a global allocator over the
//! Windows process heap, panic handling, the memory primitives compiled code
//! calls (`memcpy`, `memmove`, `memset`, `memcmp`, `bcmp`, `strlen`) and the
//! `_fltused` and unwind-personality symbols the target expects.
//!
//! Executables link this crate, `core`, `alloc` and operating-system DLLs
//! only; Windows functions are `raw-dylib` imports, so no MSVC, Visual C++
//! runtime or Windows SDK library is involved. The crate is compiled without
//! compiler builtins so the memory primitives are not lowered into calls to
//! themselves. Items that only make sense in an executable (the entry point,
//! the global allocator registration, the panic handler and the C runtime
//! symbols) are left out of test builds, which run on the platform runtime.
//!
//! With the `std-host` feature, the crate serves programs that use Rust
//! `std` instead, nativeboat itself among them: `std` brings its own
//! allocator and panic handling, and `nativeboat-std-start` supplies the rest
//! of what the Visual C++ runtime otherwise would.

#![cfg_attr(not(test), no_std)]
#![no_builtins]

use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut c_void;
    fn HeapAlloc(heap: *mut c_void, flags: u32, bytes: usize) -> *mut u8;
    fn HeapReAlloc(heap: *mut c_void, flags: u32, memory: *mut u8, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut c_void, flags: u32, memory: *mut u8) -> i32;
    fn GetStdHandle(which: u32) -> *mut c_void;
    fn WriteFile(
        file: *mut c_void,
        data: *const u8,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    #[cfg(not(any(test, feature = "std-host")))]
    fn ExitProcess(code: u32) -> !;
}

const HEAP_ZERO_MEMORY: u32 = 0x8;
/// The alignment `HeapAlloc` guarantees on 64-bit Windows.
const HEAP_ALIGNMENT: usize = 16;

/// Blocks up to this size come from size classes rather than the process
/// heap: compiled programs make many small, short-lived allocations, which
/// the process heap serves slowly.
const SMALL: usize = 1024;
/// The block sizes of the size classes, multiples of the heap alignment.
const CLASSES: [usize; 20] = [
    16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896, 1024,
];
/// The size class of each size, by its count of 16-byte units.
const CLASS_OF_UNITS: [u8; SMALL / 16 + 1] = {
    let mut table = [0u8; SMALL / 16 + 1];
    let mut units = 0;
    while units < table.len() {
        let mut class = 0;
        while CLASSES[class] < units * 16 {
            class += 1;
        }
        table[units] = class as u8;
        units += 1;
    }
    table
};
/// Size classes are carved from slabs of this size.
const SLAB: usize = 64 * 1024;

/// Free blocks of each size class and the rest of the current slab.
struct Pools {
    free: [*mut u8; CLASSES.len()],
    next: *mut u8,
    end: *mut u8,
}

/// The pools, behind a lock: a compiled program allocates almost only on its
/// UI thread, so the lock is almost never contended.
struct Locked {
    busy: core::sync::atomic::AtomicBool,
    pools: core::cell::UnsafeCell<Pools>,
}

// SAFETY: the pools are only used while `busy` is held.
unsafe impl Sync for Locked {}

static POOLS: Locked = Locked {
    busy: core::sync::atomic::AtomicBool::new(false),
    pools: core::cell::UnsafeCell::new(Pools {
        free: [core::ptr::null_mut(); CLASSES.len()],
        next: core::ptr::null_mut(),
        end: core::ptr::null_mut(),
    }),
};

impl Locked {
    fn with<R>(&self, work: impl FnOnce(&mut Pools) -> R) -> R {
        use core::sync::atomic::Ordering;
        while self
            .busy
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        // SAFETY: `busy` is held, so no other thread uses the pools.
        let result = work(unsafe { &mut *self.pools.get() });
        self.busy.store(false, Ordering::Release);
        result
    }
}

/// The size class of a layout the classes serve.
fn class(layout: Layout) -> Option<usize> {
    (layout.align() <= HEAP_ALIGNMENT && layout.size() <= SMALL)
        .then(|| usize::from(CLASS_OF_UNITS[layout.size().div_ceil(16)]))
}

/// A block of a size class: a freed one, or a new one from the slab.
fn small_block(class: usize) -> *mut u8 {
    POOLS.with(|pools| {
        let free = pools.free[class];
        if !free.is_null() {
            // SAFETY: a free block holds the next free block of its class.
            pools.free[class] = unsafe { (free as *mut *mut u8).read() };
            return free;
        }
        let size = CLASSES[class];
        if (pools.end as usize) - (pools.next as usize) < size {
            // The rest of the old slab is left unused.
            // SAFETY: the process heap is valid for the life of the process.
            let slab = unsafe { HeapAlloc(GetProcessHeap(), 0, SLAB) };
            if slab.is_null() {
                return slab;
            }
            pools.next = slab;
            // SAFETY: the slab is `SLAB` bytes long.
            pools.end = unsafe { slab.add(SLAB) };
        }
        let block = pools.next;
        // SAFETY: the slab has room for the block.
        pools.next = unsafe { block.add(size) };
        block
    })
}

/// Returns a block to its size class.
///
/// # Safety
/// `block` must be a block of `class` that is no longer used.
unsafe fn free_small(block: *mut u8, class: usize) {
    POOLS.with(|pools| {
        // SAFETY: the block is unused and at least a pointer wide.
        unsafe { (block as *mut *mut u8).write(pools.free[class]) };
        pools.free[class] = block;
    });
}

/// The global allocator. Small blocks come from size classes carved from
/// process-heap slabs; larger ones from the Windows process heap, where
/// alignments above the heap's are served by over-allocating and recording
/// the heap block just below the aligned address.
#[derive(Debug)]
pub struct Heap;

impl Heap {
    unsafe fn aligned(&self, layout: Layout, flags: u32) -> *mut u8 {
        // A layout's size is at most `isize::MAX` rounded down to its
        // alignment, so adding the alignment cannot overflow.
        let size = layout.size() + layout.align();
        // SAFETY: the process heap is valid for the life of the process.
        let block = unsafe { HeapAlloc(GetProcessHeap(), flags, size) };
        if block.is_null() {
            return block;
        }
        // The block is 16-byte aligned and the alignment is at least 32, so
        // the aligned address leaves at least 16 bytes for the header.
        let aligned = ((block as usize + layout.align()) & !(layout.align() - 1)) as *mut u8;
        // SAFETY: the header slot lies inside the block, below `aligned`.
        unsafe { (aligned as *mut *mut u8).sub(1).write_unaligned(block) };
        aligned
    }
}

// SAFETY: small blocks are handed out once until freed, under the pools'
// lock; other blocks come from the process heap, which is thread-safe, and
// are returned to it with the pointer the heap issued.
unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if let Some(class) = class(layout) {
            small_block(class)
        } else if layout.align() <= HEAP_ALIGNMENT {
            // SAFETY: as above.
            unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size()) }
        } else {
            // SAFETY: as above.
            unsafe { self.aligned(layout, 0) }
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if let Some(class) = class(layout) {
            let block = small_block(class);
            if !block.is_null() {
                // SAFETY: the block holds at least `layout.size()` bytes.
                unsafe { memset(block, 0, layout.size()) };
            }
            block
        } else if layout.align() <= HEAP_ALIGNMENT {
            // SAFETY: as above.
            unsafe { HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, layout.size()) }
        } else {
            // SAFETY: as above.
            unsafe { self.aligned(layout, HEAP_ZERO_MEMORY) }
        }
    }

    unsafe fn dealloc(&self, memory: *mut u8, layout: Layout) {
        if let Some(class) = class(layout) {
            // SAFETY: the caller frees the block it allocated with `layout`.
            unsafe { free_small(memory, class) };
            return;
        }
        let block = if layout.align() <= HEAP_ALIGNMENT {
            memory
        } else {
            // SAFETY: `aligned` stored the heap block just below `memory`.
            unsafe { (memory as *mut *mut u8).sub(1).read_unaligned() }
        };
        // SAFETY: `block` was issued by the process heap.
        unsafe { HeapFree(GetProcessHeap(), 0, block) };
    }

    unsafe fn realloc(&self, memory: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `realloc`'s contract.
        let grown = unsafe { Layout::from_size_align_unchecked(size, layout.align()) };
        let (old, new) = (class(layout), class(grown));
        if old.is_some() && old == new {
            // The block already has room.
            return memory;
        }
        if old.is_none() && new.is_none() && layout.align() <= HEAP_ALIGNMENT {
            // SAFETY: `memory` was issued by the process heap.
            return unsafe { HeapReAlloc(GetProcessHeap(), 0, memory, size) };
        }
        // SAFETY: as above.
        let fresh = unsafe { self.alloc(grown) };
        if !fresh.is_null() {
            // SAFETY: both blocks are valid for the smaller size and
            // distinct; the old block is released afterwards.
            unsafe {
                memcpy(fresh, memory, size.min(layout.size()));
                self.dealloc(memory, layout);
            }
        }
        fresh
    }
}

#[cfg(not(any(test, feature = "std-host")))]
#[global_allocator]
static HEAP: Heap = Heap;

/// The license notices of the third-party code every executable contains
/// (Rust's `core`/`alloc`/`compiler_builtins` and the vendored math
/// functions), kept in the executable itself so they travel with every copy.
#[used]
pub static NOTICES: [u8; include_bytes!("NOTICES.txt").len()] = *include_bytes!("NOTICES.txt");

/// A file handle written directly; used for panic reports on standard error.
#[derive(Debug)]
pub struct Writer(*mut c_void);

impl Writer {
    /// Standard error.
    pub fn stderr() -> Writer {
        // SAFETY: GetStdHandle has no preconditions.
        Writer(unsafe { GetStdHandle(-12i32 as u32) })
    }
}

impl core::fmt::Write for Writer {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let mut bytes = text.as_bytes();
        while !bytes.is_empty() {
            let length = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            let mut written = 0;
            // SAFETY: `bytes` is valid for `length` bytes.
            let ok = unsafe {
                WriteFile(
                    self.0,
                    bytes.as_ptr(),
                    length,
                    &mut written,
                    core::ptr::null_mut(),
                )
            };
            if ok == 0 || written == 0 {
                return Err(core::fmt::Error);
            }
            // `get` cannot panic, so programs that link this crate outside
            // link-time optimization need no `core` panic paths.
            bytes = bytes.get(written as usize..).unwrap_or_default();
        }
        Ok(())
    }
}

#[cfg(not(any(test, feature = "std-host")))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo<'_>) -> ! {
    use core::fmt::Write;
    let _ = writeln!(Writer::stderr(), "nativeboat: internal error: {info}");
    // SAFETY: ExitProcess has no preconditions.
    unsafe { ExitProcess(101) }
}

#[cfg(not(any(test, feature = "std-host")))]
unsafe extern "Rust" {
    /// The generated application, returning the process exit code.
    fn nativeboat_main() -> i32;
}

/// The executable's entry point (`/ENTRY:nativeboat_start`).
#[cfg(not(any(test, feature = "std-host")))]
#[unsafe(no_mangle)]
pub extern "C" fn nativeboat_start() -> ! {
    // SAFETY: every executable nativeboat builds defines `nativeboat_main`.
    let code = unsafe { nativeboat_main() };
    // SAFETY: ExitProcess has no preconditions.
    unsafe { ExitProcess(code as u32) }
}

/// The C runtime's floating-point marker, which floating-point code references.
#[cfg(not(test))]
#[unsafe(no_mangle)]
#[allow(non_upper_case_globals)]
pub static _fltused: i32 = 0;

/// The exception-handling personality the target's unwind tables name.
/// Executables abort on panic, so it never runs; it ends the process if it
/// ever does.
#[cfg(not(any(test, feature = "std-host")))]
#[unsafe(no_mangle)]
pub extern "C" fn __CxxFrameHandler3() -> ! {
    // SAFETY: ExitProcess has no preconditions.
    unsafe { ExitProcess(134) }
}

/// The stack probe functions with frames larger than a page call before
/// moving the stack pointer: it touches each page of the `rax` bytes below
/// the caller's stack pointer in order, so the guard page grows the stack.
/// It preserves every register (the same routine as LLVM compiler-rt's).
#[cfg(not(test))]
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub extern "C" fn __chkstk() {
    core::arch::naked_asm!(
        "push rcx",
        "push rax",
        "cmp rax, 0x1000",
        "lea rcx, [rsp + 24]",
        "jb 3f",
        "2:",
        "sub rcx, 0x1000",
        "test [rcx], rcx",
        "sub rax, 0x1000",
        "cmp rax, 0x1000",
        "ja 2b",
        "3:",
        "sub rcx, rax",
        "test [rcx], rcx",
        "pop rax",
        "pop rcx",
        "ret",
    );
}

/// Copies `count` bytes between non-overlapping buffers.
///
/// # Safety
/// Both pointers must be valid for `count` bytes and must not overlap.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn memcpy(target: *mut u8, source: *const u8, count: usize) -> *mut u8 {
    // SAFETY: `rep movsb` copies `rcx` bytes forward from `rsi` to `rdi`; the
    // direction flag is clear under the Windows calling convention.
    unsafe {
        core::arch::asm!(
            "rep movsb",
            inout("rcx") count => _,
            inout("rdi") target => _,
            inout("rsi") source => _,
            options(nostack, preserves_flags)
        );
    }
    target
}

/// Copies `count` bytes between buffers that may overlap.
///
/// # Safety
/// Both pointers must be valid for `count` bytes.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn memmove(target: *mut u8, source: *const u8, count: usize) -> *mut u8 {
    if (target as usize).wrapping_sub(source as usize) >= count {
        // The target starts before the source or past its end, so a forward
        // copy never overwrites unread bytes.
        // SAFETY: as for `memcpy`.
        return unsafe { memcpy(target, source, count) };
    }
    // SAFETY: the target overlaps the end of the source, so the bytes are
    // copied backwards from the last one, with the direction flag restored.
    unsafe {
        core::arch::asm!(
            "std",
            "rep movsb",
            "cld",
            inout("rcx") count => _,
            inout("rdi") target.add(count - 1) => _,
            inout("rsi") source.add(count - 1) => _,
            options(nostack)
        );
    }
    target
}

/// Fills `count` bytes with a byte value.
///
/// # Safety
/// `target` must be valid for `count` bytes.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn memset(target: *mut u8, value: i32, count: usize) -> *mut u8 {
    // SAFETY: `rep stosb` stores `al` into `rcx` bytes from `rdi`.
    unsafe {
        core::arch::asm!(
            "rep stosb",
            inout("rcx") count => _,
            inout("rdi") target => _,
            in("al") value as u8,
            options(nostack, preserves_flags)
        );
    }
    target
}

/// Compares `count` bytes: negative, zero or positive like C `memcmp`.
///
/// # Safety
/// Both pointers must be valid for `count` bytes.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn memcmp(left: *const u8, right: *const u8, count: usize) -> i32 {
    for index in 0..count {
        // SAFETY: both buffers are valid for `count` bytes.
        let (a, b) = unsafe { (*left.add(index), *right.add(index)) };
        if a != b {
            return i32::from(a) - i32::from(b);
        }
    }
    0
}

/// Whether `count` bytes differ: zero when equal.
///
/// # Safety
/// Both pointers must be valid for `count` bytes.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn bcmp(left: *const u8, right: *const u8, count: usize) -> i32 {
    // SAFETY: as for `memcmp`.
    unsafe { memcmp(left, right, count) }
}

/// The length of a NUL-terminated byte string.
///
/// # Safety
/// `text` must point to a NUL-terminated byte string.
#[cfg_attr(not(test), unsafe(no_mangle))]
pub unsafe extern "C" fn strlen(text: *const u8) -> usize {
    let mut length = 0;
    // SAFETY: the string is NUL-terminated, so every byte read up to the
    // terminator is inside it.
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    length
}

#[cfg(test)]
mod tests;
