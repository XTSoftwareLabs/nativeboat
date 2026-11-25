use super::*;

#[test]
fn full_width_bands_preserve_pixels_before_and_after_the_contiguous_region() {
    let mut pixels = [0x25ff_ffff_u32; 25];
    // SAFETY: the pixel array outlives groups and drawing is not concurrent.
    let mut groups = unsafe {
        Groups::new(BITMAP {
            bmBitsPixel: 32,
            bmWidthBytes: 20,
            bmHeight: 5,
            bmBits: pixels.as_mut_ptr().cast(),
            ..BITMAP::default()
        })
    };
    assert!(groups.push_in(
        0.5,
        Some(RECT {
            left: 0,
            top: 1,
            right: 5,
            bottom: 4
        })
    ));
    assert_eq!(groups.allocated, 60);
    pixels.fill(0x2500_0000);
    groups.pop();
    assert_eq!(&pixels[..5], &[0x2500_0000; 5]);
    assert_eq!(&pixels[5..20], &[0x257f_7f7f; 15]);
    assert_eq!(&pixels[20..], &[0x2500_0000; 5]);
}

#[test]
#[ignore = "run with CARGO_PROFILE_TEST_OPT_LEVEL=2 for native opacity kernel timing"]
fn opacity_region_copy_and_compositing_performance() {
    let sample = |region| {
        let mut pixels = vec![0x25ff_ffff_u32; 1500 * 1000];
        // SAFETY: the stable pixel allocation outlives groups and has no
        // concurrent drawing; only pixels inside the declared region change.
        let mut groups = unsafe {
            Groups::new(BITMAP {
                bmBitsPixel: 32,
                bmWidthBytes: 6000,
                bmHeight: 1000,
                bmBits: pixels.as_mut_ptr().cast(),
                ..BITMAP::default()
            })
        };
        assert!(groups.push_in(0.5, region));
        groups.pop();
        let started = std::time::Instant::now();
        for _ in 0..100 {
            assert!(std::hint::black_box(&mut groups).push_in(0.5, region));
            pixels[40 * 1500 + 30] = 0x2500_0000;
            groups.pop();
        }
        let milliseconds = started.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(pixels[0], 0x25ff_ffff);
        (milliseconds, groups.allocated, pixels)
    };
    let region = RECT {
        left: 30,
        top: 40,
        right: 80,
        bottom: 90,
    };
    for trial in 1..=3 {
        // Reverse trial order to expose warmup/order effects.
        let (full, bounded) = if trial == 2 {
            let bounded = sample(Some(region));
            (sample(None), bounded)
        } else {
            (sample(None), sample(Some(region)))
        };
        assert_eq!(full.1, 6_000_000);
        assert_eq!(bounded.1, 10_000);
        assert_eq!(full.2, bounded.2, "both paths produce identical pixels");
        println!(
            "OPACITY_REGION_PERFORMANCE trial={trial} iterations=100 full_ms={} bounded_ms={} full_bytes={} bounded_bytes={}",
            full.0, bounded.0, full.1, bounded.1
        );
    }
}

#[test]
fn small_groups_copy_only_their_region_and_reuse_or_evict_completed_buffers() {
    let mut pixels = vec![0x25ff_ffff_u32; 100 * 100];
    // SAFETY: the pixel allocation remains live and stable for every group.
    let mut groups = unsafe {
        Groups::new(BITMAP {
            bmBitsPixel: 32,
            bmWidthBytes: 400,
            bmHeight: 100,
            bmBits: pixels.as_mut_ptr().cast(),
            ..BITMAP::default()
        })
    };
    groups.budget = 32;
    let region = RECT {
        left: 30,
        top: 40,
        right: 32,
        bottom: 42,
    };
    assert!(groups.push_in(0.5, Some(region)));
    assert_eq!(
        groups.allocated, 16,
        "four pixels rather than the entire bitmap"
    );
    pixels[4030] = 0x2500_0000;
    pixels[4131] = 0x2500_0000;
    pixels[0] = 0x2512_3456;
    groups.pop();
    assert_eq!(pixels[4030], 0x257f_7f7f);
    assert_eq!(pixels[4131], 0x257f_7f7f);
    assert_eq!(pixels[4031], 0x25ff_ffff, "unchanged pixels stay unchanged");
    assert_eq!(pixels[0], 0x2512_3456, "outside pixels are never blended");
    let reused = groups.pool[0].as_ptr();
    assert!(groups.push_in(0.5, Some(region)));
    assert_eq!(groups.open[0].saved.as_ref().unwrap().as_ptr(), reused);
    groups.pop();
    let clipped = RECT {
        left: -2,
        top: -1,
        right: 2,
        bottom: 3,
    };
    assert!(groups.push_in(0.5, Some(clipped)));
    assert_eq!(
        groups.allocated, 24,
        "evicts the completed buffer to fit six pixels"
    );
    assert_eq!(groups.open[0].saved.as_ref().unwrap().len(), 6);
    assert!(
        !groups.push_in(0.5, Some(region)),
        "open buffers cannot be evicted"
    );
    groups.pop();
    groups.pop();
    for region in [
        RECT {
            left: 100,
            top: 0,
            right: 200,
            bottom: 10,
        },
        RECT {
            left: 4,
            top: 4,
            right: 2,
            bottom: 2,
        },
    ] {
        assert!(!groups.push_in(0.5, Some(region)));
        groups.pop();
    }
}

#[test]
fn backdrop_buffers_are_reused_and_bounded_across_nested_groups() {
    let mut pixel = [0x2532_64c8_u32];
    // SAFETY: the pixel array outlives groups and no drawing is concurrent.
    let mut groups = unsafe {
        Groups::new(BITMAP {
            bmBitsPixel: 32,
            bmWidthBytes: 4,
            bmHeight: 1,
            bmBits: pixel.as_mut_ptr().cast(),
            ..BITMAP::default()
        })
    };
    groups.budget = 4;
    assert!(groups.is_empty());
    assert!(groups.push(0.5));
    assert_eq!(groups.allocated, 4);
    assert!(
        !groups.push(0.5),
        "a nested buffer must fit the same budget"
    );
    pixel = [0x2500_0000];
    groups.pop();
    assert_eq!(pixel, [0x2500_0000], "fallback is composited by the caller");
    groups.pop();
    assert_eq!(pixel, [0x2519_3264]);
    assert!(groups.is_empty());
    let pointer = groups.pool[0].as_ptr();
    assert!(groups.push(0.25));
    assert_eq!(groups.open[0].saved.as_ref().unwrap().as_ptr(), pointer);
    pixel = [0x2500_0000];
    groups.pop();
    assert_eq!(pixel, [0x2513_254b]);
    assert_eq!(groups.allocated, 4);
    assert!(!groups.push(0.0));
    groups.pop();
    assert!(!groups.push(1.0));
    groups.pop();
    groups.pop();
    assert!(groups.is_empty());
}

#[test]
fn unsupported_bitmap_storage_uses_the_callers_alpha_fallback() {
    for bitmap in [
        BITMAP::default(),
        BITMAP {
            bmBitsPixel: 24,
            ..BITMAP::default()
        },
        BITMAP {
            bmBitsPixel: 32,
            bmWidthBytes: -4,
            bmHeight: 1,
            ..BITMAP::default()
        },
    ] {
        // SAFETY: unsupported descriptors do not dereference their bits.
        let mut groups = unsafe { Groups::new(bitmap) };
        assert!(!groups.push(0.5));
        groups.pop();
        assert_eq!(groups.allocated, 0);
    }
}
