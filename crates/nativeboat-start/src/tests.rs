use super::*;
use core::fmt::Write;
use std::os::windows::io::AsRawHandle;

#[test]
fn memory_primitives_copy_move_fill_compare_and_measure() {
    let source = *b"abcdefgh";
    let mut target = [0u8; 8];
    // SAFETY: the buffers are valid and distinct.
    unsafe { memcpy(target.as_mut_ptr(), source.as_ptr(), 8) };
    assert_eq!(&target, b"abcdefgh");
    let mut buffer = *b"0123456789";
    // SAFETY: both ranges are inside `buffer`.
    unsafe { memmove(buffer.as_mut_ptr().add(2), buffer.as_ptr(), 6) };
    assert_eq!(&buffer, b"0101234589");
    let mut buffer = *b"0123456789";
    // SAFETY: as above; the target precedes the source.
    unsafe { memmove(buffer.as_mut_ptr(), buffer.as_ptr().add(3), 5) };
    assert_eq!(&buffer, b"3456756789");
    // SAFETY: zero bytes are moved.
    unsafe { memmove(buffer.as_mut_ptr().add(1), buffer.as_ptr(), 0) };
    assert_eq!(&buffer, b"3456756789");
    // SAFETY: `buffer` holds ten bytes.
    unsafe { memset(buffer.as_mut_ptr().add(1), i32::from(b'z'), 3) };
    assert_eq!(&buffer, b"3zzz756789");
    // SAFETY: the compared buffers hold three bytes each.
    unsafe {
        assert_eq!(memcmp(b"abc".as_ptr(), b"abc".as_ptr(), 3), 0);
        assert!(memcmp(b"abc".as_ptr(), b"abd".as_ptr(), 3) < 0);
        assert!(memcmp(b"b".as_ptr(), b"a".as_ptr(), 1) > 0);
        assert_eq!(bcmp(b"xy".as_ptr(), b"xy".as_ptr(), 2), 0);
        assert_ne!(bcmp(b"xy".as_ptr(), b"xz".as_ptr(), 2), 0);
        assert_eq!(strlen(c"native".as_ptr().cast()), 6);
        assert_eq!(strlen(c"".as_ptr().cast()), 0);
    }
}

fn exercise(layout: Layout) {
    let heap = Heap;
    // SAFETY: each block is used within its layout and released once.
    unsafe {
        let block = heap.alloc(layout);
        assert!(!block.is_null());
        assert_eq!(block as usize % layout.align(), 0);
        block.write_bytes(7, layout.size());
        let grown = heap.realloc(block, layout, layout.size() * 4);
        assert!(!grown.is_null());
        assert_eq!(grown as usize % layout.align(), 0);
        assert_eq!(*grown.add(layout.size() - 1), 7);
        heap.dealloc(
            grown,
            Layout::from_size_align(layout.size() * 4, layout.align()).unwrap(),
        );
        let zeroed = heap.alloc_zeroed(layout);
        assert_eq!(zeroed as usize % layout.align(), 0);
        assert!((0..layout.size()).all(|index| *zeroed.add(index) == 0));
        heap.dealloc(zeroed, layout);
    }
}

#[test]
fn aligned_reallocation_preserves_only_the_existing_bytes() {
    let heap = Heap;
    let original = Layout::from_size_align(1024 * 1024, 4096).unwrap();
    let grown = Layout::from_size_align(8 * 1024 * 1024, original.align()).unwrap();
    let shrunk = Layout::from_size_align(31, original.align()).unwrap();
    // SAFETY: all layouts are valid, every successful reallocation replaces
    // its previous allocation, and reads stay within initialized bytes.
    unsafe {
        let first = heap.alloc(original);
        assert!(!first.is_null());
        first.write_bytes(0x7b, original.size());
        let second = heap.realloc(first, original, grown.size());
        assert!(!second.is_null());
        assert_eq!(second as usize % original.align(), 0);
        assert!((0..original.size()).all(|at| *second.add(at) == 0x7b));
        let last = heap.realloc(second, grown, shrunk.size());
        assert!(!last.is_null());
        assert_eq!(last as usize % shrunk.align(), 0);
        assert!((0..shrunk.size()).all(|at| *last.add(at) == 0x7b));
        heap.dealloc(last, shrunk);
    }
}

#[test]
fn the_heap_allocator_honors_sizes_alignments_and_failures() {
    exercise(Layout::from_size_align(24, 8).unwrap());
    exercise(Layout::from_size_align(100, 64).unwrap());
    exercise(Layout::from_size_align(5000, 4096).unwrap());
    let heap = Heap;
    // SAFETY: failed allocations return null and allocate nothing.
    unsafe {
        let huge = Layout::from_size_align(isize::MAX as usize - 4096, 64).unwrap();
        assert!(heap.alloc(huge).is_null());
        let layout = Layout::from_size_align(64, 64).unwrap();
        let block = heap.alloc(layout);
        assert!(
            heap.realloc(block, layout, isize::MAX as usize - 4096)
                .is_null()
        );
        heap.dealloc(block, layout);
    }
}

#[test]
fn the_writer_writes_whole_text_and_reports_failures() {
    let path = std::env::temp_dir().join(format!("nativeboat-start-{}.txt", std::process::id()));
    let file = std::fs::File::create(&path).unwrap();
    let mut writer = Writer(file.as_raw_handle());
    write!(writer, "panic {}", 7).unwrap();
    drop(file);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "panic 7");
    std::fs::remove_file(&path).unwrap();
    assert!(Writer(core::ptr::null_mut()).write_str("x").is_err());
    assert!(Writer::stderr().write_str("").is_ok());
}

#[test]
fn executables_embed_the_third_party_notices() {
    let text = core::str::from_utf8(&NOTICES).unwrap();
    assert!(text.contains("NativeBoat runtime\nCopyright (c) 2026 XTSoftwareLabs"));
    assert!(text.contains("The Rust Project Contributors"));
    assert!(text.contains("Rich Felker"));
    assert!(text.contains("libuv project contributors"));
}

#[test]
fn small_blocks_come_from_size_classes_and_are_reused() {
    let layout = |size| Layout::from_size_align(size, 8).unwrap();
    assert_eq!(class(layout(0)), Some(0));
    assert_eq!(class(layout(16)), Some(0));
    assert_eq!(class(layout(17)), Some(1));
    assert_eq!(class(layout(129)).map(|class| CLASSES[class]), Some(160));
    assert_eq!(class(layout(1024)).map(|class| CLASSES[class]), Some(1024));
    assert_eq!(class(layout(1025)), None);
    assert_eq!(class(Layout::from_size_align(64, 32).unwrap()), None);
    let heap = Heap;
    // SAFETY: each block is used within its layout and released once.
    unsafe {
        // A freed block of a class is the next one handed out.
        let first = heap.alloc(layout(40));
        first.write_bytes(9, 40);
        heap.dealloc(first, layout(40));
        let again = heap.alloc(layout(48));
        assert_eq!(again, first);
        // Zeroed blocks are cleared even when reused.
        heap.dealloc(again, layout(48));
        let zeroed = heap.alloc_zeroed(layout(33));
        assert_eq!(zeroed, first);
        assert!((0..33).all(|index| *zeroed.add(index) == 0));
        // Growing within a class keeps the block; beyond it moves the bytes.
        zeroed.write_bytes(5, 33);
        assert_eq!(heap.realloc(zeroed, layout(33), 48), zeroed);
        let moved = heap.realloc(zeroed, layout(48), 2000);
        assert!((0..33).all(|index| *moved.add(index) == 5));
        let back = heap.realloc(moved, layout(2000), 20);
        assert!((0..20).all(|index| *back.add(index) == 5));
        heap.dealloc(back, layout(20));
    }
    // Threads share the classes safely.
    let threads: Vec<_> = (0..4)
        .map(|seed| {
            std::thread::spawn(move || {
                let heap = Heap;
                for round in 0..2000 {
                    let size = 1 + (seed * 131 + round * 17) % 1024;
                    // SAFETY: as above.
                    unsafe {
                        let block = heap.alloc(layout(size));
                        block.write_bytes(seed as u8, size);
                        assert!((0..size).all(|index| *block.add(index) == seed as u8));
                        heap.dealloc(block, layout(size));
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}
