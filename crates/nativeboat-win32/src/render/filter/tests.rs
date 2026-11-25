use super::*;
use nativeboat_css::Color;

const IDENTITY: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

fn image(width: usize, height: usize, pixels: Vec<[f32; 4]>) -> Image {
    Image {
        width,
        height,
        pixels,
    }
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.002)
}

fn one(pixel: [f32; 4], filter: Filter) -> [f32; 4] {
    let mut image = image(1, 1, vec![pixel]);
    apply(&mut image, &[filter], IDENTITY);
    image.pixels[0]
}

#[test]
fn layers_are_recovered_from_black_and_white() {
    let black = [0x00FF_0000, 0x0000_0000, 0x0080_0000, 0x00FF_FFFF];
    let white = [0x00FF_0000, 0x00FF_FFFF, 0x00FF_7F7F, 0x0000_0000];
    let layer = Image::isolate(&black, &white, 2, 2);
    assert_eq!((layer.width, layer.height), (2, 2));
    assert_eq!(layer.pixels[0], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(layer.pixels[1], [0.0; 4]);
    let half = 128.0 / 255.0;
    assert!(close(layer.pixels[2], [half, 0.0, 0.0, half]));
    // Inconsistent pixels stay valid premultiplied colors.
    assert_eq!(layer.pixels[3], [1.0; 4]);
}

#[test]
fn pixels_convert_to_and_from_bitmaps() {
    let opaque = Image::opaque(&[0x7711_2233], 1, 1);
    assert_eq!(opaque.pargb(), vec![0xFF11_2233]);
    let translucent = image(
        3,
        1,
        vec![
            [0.5, 0.25, 0.0, 0.5],
            [1.0, 2.0, -1.0, 0.5],
            [0.0, 0.0, 0.0, 0.0],
        ],
    );
    assert_eq!(
        translucent.pargb(),
        vec![0x8080_4000, 0x8080_8000, 0x0000_0000]
    );
}

#[test]
fn edges_mirror_and_images_crop() {
    let a = [1.0, 0.0, 0.0, 1.0];
    let b = [0.0, 1.0, 0.0, 1.0];
    let c = [0.0, 0.0, 1.0, 1.0];
    let d = [0.0; 4];
    let source = image(2, 2, vec![a, b, c, d]);
    let grown = source.mirrored(2, 1);
    assert_eq!((grown.width, grown.height), (6, 4));
    let row = |index: usize| grown.pixels[index * 6..index * 6 + 6].to_vec();
    assert_eq!(row(0), vec![b, a, a, b, b, a]);
    assert_eq!(row(1), vec![b, a, a, b, b, a]);
    assert_eq!(row(2), vec![d, c, c, d, d, c]);
    assert_eq!(row(3), vec![d, c, c, d, d, c]);
    assert_eq!(grown.crop(2, 1, 2, 2), source);
    assert_eq!(mirror(-5, 2), 0);
    assert_eq!(mirror(7, 3), 1);
}

#[test]
fn reaches_add_up_and_scale() {
    assert_eq!(reach(&[], IDENTITY), (0, 0));
    assert_eq!(reach(&[Filter::Brightness(2.0)], IDENTITY), (0, 0));
    // A deviation of 2 doubled by the device scale spreads 12 pixels.
    assert_eq!(reach(&[Filter::Blur(2.0)], [2.0, 0.0, 0.0, 2.0]), (12, 12));
    let shadow = Filter::DropShadow {
        x: 16.0,
        y: -2.0,
        blur: 4.0,
        color: Color::BLACK,
    };
    assert_eq!(reach(std::slice::from_ref(&shadow), IDENTITY), (22, 8));
    assert_eq!(
        reach(&[Filter::Blur(1.0), shadow], [1.25, 0.0, 0.0, 1.25]),
        (4 + 20 + 8, 4 + 3 + 8)
    );
    // A quarter turn moves a horizontal shadow vertically.
    let turned = Filter::DropShadow {
        x: 10.0,
        y: 0.0,
        blur: 0.0,
        color: Color::BLACK,
    };
    assert_eq!(reach(&[turned], [0.0, 1.0, -1.0, 0.0]), (0, 10));
}

#[test]
fn color_functions_follow_the_specification() {
    let color = [0.6, 0.4, 0.2, 1.0];
    assert!(close(
        one(color, Filter::Brightness(0.5)),
        [0.3, 0.2, 0.1, 1.0]
    ));
    assert!(close(
        one(color, Filter::Brightness(3.0)),
        [1.0, 1.0, 0.6, 1.0]
    ));
    assert!(close(
        one(color, Filter::Contrast(2.0)),
        [0.7, 0.3, 0.0, 1.0]
    ));
    let gray = 0.2126 * 0.6 + 0.7152 * 0.4 + 0.0722 * 0.2;
    assert!(close(
        one(color, Filter::Grayscale(1.0)),
        [gray, gray, gray, 1.0]
    ));
    assert!(close(one(color, Filter::Grayscale(0.0)), color));
    let sepia = [
        0.393 * 0.6 + 0.769 * 0.4 + 0.189 * 0.2,
        0.349 * 0.6 + 0.686 * 0.4 + 0.168 * 0.2,
        0.272 * 0.6 + 0.534 * 0.4 + 0.131 * 0.2,
        1.0,
    ];
    assert!(close(one(color, Filter::Sepia(1.0)), sepia));
    assert!(close(one(color, Filter::Saturate(1.0)), color));
    let flat = 0.213 * 0.6 + 0.715 * 0.4 + 0.072 * 0.2;
    assert!(close(
        one(color, Filter::Saturate(0.0)),
        [flat, flat, flat, 1.0]
    ));
    assert!(close(one(color, Filter::HueRotate(0.0)), color));
    assert!(close(one(color, Filter::HueRotate(360.0)), color));
    // Rotating red by a third of a turn mostly turns it green.
    let turned = one([1.0, 0.0, 0.0, 1.0], Filter::HueRotate(120.0));
    assert!(turned[1] > turned[0] && turned[1] > turned[2], "{turned:?}");
    assert!(close(one(color, Filter::Invert(1.0)), [0.4, 0.6, 0.8, 1.0]));
    assert!(close(one(color, Filter::Invert(0.5)), [0.5, 0.5, 0.5, 1.0]));
    assert!(close(
        one(color, Filter::Opacity(0.5)),
        [0.3, 0.2, 0.1, 0.5]
    ));
    // Colors change unpremultiplied: half-transparent pixels keep alpha.
    assert!(close(
        one([0.3, 0.2, 0.1, 0.5], Filter::Invert(1.0)),
        [0.2, 0.3, 0.4, 0.5]
    ));
    // Transparent pixels stay transparent.
    assert_eq!(one([0.0; 4], Filter::Invert(1.0)), [0.0; 4]);
}

#[test]
fn blur_passes_follow_svg() {
    assert_eq!(passes(1.0).len(), 1);
    let Pass::Kernel(weights) = &passes(1.0)[0] else {
        panic!("small deviations use a kernel");
    };
    assert_eq!(weights.len(), 7);
    assert!((weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    assert!(weights[3] > weights[2] && (weights[2] - weights[4]).abs() < 1e-6);
    // d = floor(2.5 * 1.88 + 0.5) = 5 is odd: three centered boxes.
    assert_eq!(passes(2.5), vec![Pass::Box { left: 2, right: 2 }; 3]);
    // d = floor(2 * 1.88 + 0.5) = 4 is even: offset boxes, then a wider one.
    assert_eq!(
        passes(2.0),
        vec![
            Pass::Box { left: 2, right: 1 },
            Pass::Box { left: 1, right: 2 },
            Pass::Box { left: 2, right: 2 },
        ]
    );
}

fn dot(size: usize, at: (usize, usize)) -> Image {
    let mut pixels = vec![[0.0; 4]; size * size];
    pixels[at.1 * size + at.0] = [1.0; 4];
    image(size, size, pixels)
}

fn total(image: &Image) -> f32 {
    image.pixels.iter().map(|pixel| pixel[3]).sum()
}

#[test]
fn blurs_spread_symmetrically_and_keep_energy() {
    for radius in [0.5_f32, 1.0, 2.5, 4.0, 5.0] {
        let mut image = dot(41, (20, 20));
        apply(&mut image, &[Filter::Blur(radius)], IDENTITY);
        assert!((total(&image) - 1.0).abs() < 1e-3, "{radius}");
        let at = |x: usize, y: usize| image.pixels[y * 41 + x][3];
        assert!(
            at(20, 20) > at(21, 20) && at(21, 20) > at(23, 20),
            "{radius}"
        );
        assert!((at(18, 20) - at(22, 20)).abs() < 1e-6, "{radius}");
        assert!((at(20, 18) - at(20, 22)).abs() < 1e-6, "{radius}");
        assert!((at(17, 20) - at(20, 17)).abs() < 1e-6, "{radius}");
        let (x, _) = reach(&[Filter::Blur(radius)], IDENTITY);
        assert_eq!(at(20 + x + 1, 20), 0.0, "{radius}");
    }
    // Pixels beyond the edges are transparent, so energy leaves the image.
    let mut corner = dot(9, (0, 0));
    apply(&mut corner, &[Filter::Blur(1.0)], IDENTITY);
    assert!(total(&corner) < 0.55);
    let mut wide = dot(9, (8, 8));
    apply(&mut wide, &[Filter::Blur(3.0)], IDENTITY);
    assert!(total(&wide) < 0.5);
    // A zero blur changes nothing, nor does an empty image.
    let mut same = dot(3, (1, 1));
    apply(&mut same, &[Filter::Blur(0.0)], IDENTITY);
    assert_eq!(same, dot(3, (1, 1)));
    let mut empty = image(0, 0, Vec::new());
    apply(&mut empty, &[Filter::Blur(2.0)], IDENTITY);
    assert!(empty.pixels.is_empty());
    // Device scale widens the blur.
    let mut scaled = dot(41, (20, 20));
    apply(&mut scaled, &[Filter::Blur(1.0)], [3.0, 0.0, 0.0, 3.0]);
    let mut wider = dot(41, (20, 20));
    apply(&mut wider, &[Filter::Blur(3.0)], IDENTITY);
    assert!(
        scaled
            .pixels
            .iter()
            .zip(&wider.pixels)
            .all(|(a, b)| close(*a, *b))
    );
}

#[test]
fn drop_shadows_recolor_moved_alpha_behind_the_image() {
    // Muya colors icons with a shadow moved into view: the copy takes
    // the shadow color wherever the icon is opaque.
    let blue = Color::rgb(0, 0, 255);
    let mut icon = image(
        4,
        1,
        vec![
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.0, 0.5],
            [0.0; 4],
            [0.0; 4],
        ],
    );
    apply(
        &mut icon,
        &[Filter::DropShadow {
            x: 2.0,
            y: 0.0,
            blur: 0.0,
            color: blue,
        }],
        IDENTITY,
    );
    assert_eq!(icon.pixels[0], [0.0, 0.0, 0.0, 1.0]);
    assert!(close(icon.pixels[1], [0.0, 0.0, 0.0, 0.5]));
    assert_eq!(icon.pixels[2], [0.0, 0.0, 1.0, 1.0]);
    assert!(close(icon.pixels[3], [0.0, 0.0, 0.5, 0.5]));
    // The shadow shows through translucent content and keeps its alpha.
    let mut over = image(2, 1, vec![[0.5, 0.0, 0.0, 0.5], [0.5, 0.0, 0.0, 0.5]]);
    apply(
        &mut over,
        &[Filter::DropShadow {
            x: -1.0,
            y: 0.0,
            blur: 0.0,
            color: Color::rgba(0, 0, 255, 0.5),
        }],
        IDENTITY,
    );
    assert!(close(over.pixels[0], [0.5, 0.0, 0.125, 0.625]));
    assert!(close(over.pixels[1], [0.5, 0.0, 0.0, 0.5]));
    // Vertical offsets scale with the device, and the shadow blurs.
    let mut dot = dot(21, (10, 5));
    apply(
        &mut dot,
        &[Filter::DropShadow {
            x: 0.0,
            y: 4.0,
            blur: 4.0,
            color: Color::BLACK,
        }],
        [1.5, 0.0, 0.0, 1.5],
    );
    let at = |x: usize, y: usize| dot.pixels[y * 21 + x][3];
    assert_eq!(at(10, 5), 1.0);
    assert!(at(10, 11) > at(10, 12) && at(10, 11) > at(10, 10));
    assert!((at(9, 11) - at(11, 11)).abs() < 1e-6);
    assert!((at(10, 9) - at(10, 13)).abs() < 1e-6);
}
