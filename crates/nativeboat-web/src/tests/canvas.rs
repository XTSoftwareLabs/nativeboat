use super::*;
use nativeboat_ir::{CanvasCommand, CanvasPaint, CanvasSource, PathSegment, TextAnchor};

const PAGE: &str = "<canvas id=c></canvas><canvas id=other width=10 height=10></canvas><img id=logo src='logo.png'>";

/// The node id of the element matching a selector.
fn node(page: &mut Fixture, selector: &str) -> String {
    page.eval(&format!(
        "(() => {{ const element = document.querySelector('{selector}'); return element[Object.getOwnPropertySymbols(element).find((symbol) => String(symbol) === 'Symbol(nativeboat.node)')]; }})()"
    ))
}

/// A batch of drawing commands the page sent.
#[derive(Debug)]
struct Batch {
    node: u32,
    size: String,
    commands: Vec<CanvasCommand>,
}

/// The batches sent since the last call, with every command parsed as the
/// platform parses it.
fn batches(page: &Fixture) -> Vec<Batch> {
    page.page
        .canvas
        .borrow_mut()
        .drain(..)
        .filter(|record| !record.contains(" write "))
        .map(|record| {
            let mut parts = record.splitn(3, ' ');
            let node = parts.next().unwrap().parse().unwrap();
            let size = parts.next().unwrap().to_owned();
            let json = parts.next().unwrap();
            let commands =
                serde_json::from_str(json).unwrap_or_else(|error| panic!("{error}: {json}"));
            Batch {
                node,
                size,
                commands,
            }
        })
        .collect()
}

fn commands(page: &Fixture) -> Vec<CanvasCommand> {
    batches(page)
        .into_iter()
        .flat_map(|batch| batch.commands)
        .collect()
}

fn close(actual: &[PathSegment], expected: &[PathSegment]) {
    let numbers = |segments: &[PathSegment]| -> Vec<f32> {
        segments
            .iter()
            .flat_map(|segment| match *segment {
                PathSegment::Move(x, y) => vec![0.0, x, y],
                PathSegment::Line(x, y) => vec![1.0, x, y],
                PathSegment::Cubic(a, b, c, d, e, f) => vec![2.0, a, b, c, d, e, f],
                PathSegment::Close => vec![3.0],
            })
            .collect()
    };
    let (a, e) = (numbers(actual), numbers(expected));
    assert_eq!(a.len(), e.len(), "{actual:?} != {expected:?}");
    for (x, y) in a.iter().zip(&e) {
        assert!((x - y).abs() < 1e-3, "{actual:?} != {expected:?}");
    }
}

fn path_of(command: &CanvasCommand) -> &[PathSegment] {
    match command {
        CanvasCommand::Fill { path, .. }
        | CanvasCommand::Stroke { path, .. }
        | CanvasCommand::Clear { path }
        | CanvasCommand::Clip { path, .. } => path,
        other => panic!("{other:?}"),
    }
}

use PathSegment::{Close, Cubic, Line, Move};

#[test]
fn contexts_batch_commands_per_task() {
    let mut page = fixture(PAGE);
    let canvas = node(&mut page, "#c");
    page.check(
        r#"
        const canvas = ids('#c'); eq([canvas.width, canvas.height], [300, 150], 'default size');
        const ctx = canvas.getContext('2d'); eq(canvas.getContext('2d'), ctx, 'one context');
        eq(canvas.getContext('webgl'), null, 'no webgl'); eq(ctx.canvas, canvas, 'canvas');
        assert(ctx instanceof CanvasRenderingContext2D && !ctx.isContextLost(), 'context');
        eq(ctx.getContextAttributes().alpha, true, 'attributes');
        eq(ctx.fillStyle, '#000000', 'initial fill'); ctx.fillStyle = 'red'; eq(ctx.fillStyle, '#ff0000', 'fill');
        ctx.fillStyle = 'bogus'; eq(ctx.fillStyle, '#ff0000', 'invalid fill ignored');
        ctx.strokeStyle = 'rgba(0, 0, 255, 0.5)'; eq(ctx.strokeStyle, 'rgba(0, 0, 255, 0.5)', 'translucent');
        ctx.fillRect(1, 2, 3, 4); ctx.fillRect(NaN, 0, 1, 1); ctx.clearRect(0, 0, 2, 2); ctx.strokeRect(0, 0, 1, 1);
        ctx.strokeRect(Infinity, 0, 1, 1); ctx.clearRect(0, 0, NaN, 1);
        "#,
    );
    let batches = batches(&page);
    assert_eq!(batches.len(), 1, "one batch per task");
    assert_eq!(batches[0].node.to_string(), canvas);
    assert_eq!(batches[0].size, "300x150");
    let commands = &batches[0].commands;
    assert_eq!(commands.len(), 3);
    let CanvasCommand::Fill {
        path, paint, alpha, ..
    } = &commands[0]
    else {
        panic!("{commands:?}");
    };
    close(
        path,
        &[
            Move(1.0, 2.0),
            Line(4.0, 2.0),
            Line(4.0, 6.0),
            Line(1.0, 6.0),
            Close,
        ],
    );
    assert_eq!(
        *paint,
        CanvasPaint::Color(nativeboat_css::Color::rgb(255, 0, 0))
    );
    assert_eq!(*alpha, 1.0);
    assert!(matches!(commands[1], CanvasCommand::Clear { .. }));
    let CanvasCommand::Stroke { paint, width, .. } = &commands[2] else {
        panic!("{commands:?}");
    };
    assert_eq!(
        *paint,
        CanvasPaint::Color(nativeboat_css::Color::rgba(0, 0, 255, 0.5))
    );
    assert_eq!(*width, 1.0);
}

#[test]
fn paths_follow_the_transform() {
    let mut page = fixture(PAGE);
    page.check(
        r#"
        const ctx = ids('#c').getContext('2d');
        ctx.translate(10, 20); ctx.scale(2, 2);
        ctx.beginPath(); ctx.moveTo(1, 1); ctx.lineTo(2, 1); ctx.lineTo(NaN, 1); ctx.closePath(); ctx.fill('evenodd');
        ctx.resetTransform(); ctx.beginPath(); ctx.lineTo(5, 5); ctx.bezierCurveTo(1, 2, 3, 4, 5, 6); ctx.quadraticCurveTo(8, 8, 11, 6); ctx.stroke();
        ctx.beginPath(); ctx.arc(0, 0, 10, 0, Math.PI * 2); ctx.fill();
        ctx.beginPath(); ctx.arc(0, 0, 10, 0, Math.PI / 2, false); ctx.arc(0, 0, 10, 0, Math.PI / 2, true); ctx.ellipse(0, 0, 4, 2, 0, 0, 0); ctx.fill();
        throws(() => ctx.arc(0, 0, -1, 0, 1), 'IndexSizeError', 'negative radius');
        ctx.beginPath(); ctx.bezierCurveTo(1, 1, 2, 2, 3, 3); ctx.quadraticCurveTo(1, 1, 2, 2); ctx.arc(NaN, 0, 1, 0, 1); ctx.bezierCurveTo(NaN, 0, 0, 0, 0, 0); ctx.quadraticCurveTo(NaN, 0, 0, 0); ctx.fill();
        ctx.beginPath(); ctx.moveTo(0, 0); ctx.arcTo(10, 0, 10, 10, 5); ctx.arcTo(10, 10, 20, 10, 0); ctx.arcTo(NaN, 0, 0, 0, 1); ctx.stroke();
        throws(() => ctx.arcTo(0, 0, 1, 1, -1), 'IndexSizeError', 'negative arcTo');
        ctx.beginPath(); ctx.arcTo(5, 5, 6, 6, 2); ctx.closePath(); ctx.stroke();
        ctx.beginPath(); ctx.closePath(); ctx.rect(0, 0, 10, 10); ctx.rect(NaN, 0, 1, 1); ctx.clip();
        ctx.beginPath(); ctx.roundRect(0, 0, 20, 10, [2, { x: 4 }]); ctx.roundRect(0, 0, 10, 10, 2); ctx.roundRect(0, 0, 10, 10, [1, 2, 3]); ctx.roundRect(0, 0, 10, 10, [1, 2, 3, 4]); ctx.roundRect(0, 0, 10, 10); ctx.roundRect(NaN, 0, 1, 1); ctx.fill();
        throws(() => ctx.roundRect(0, 0, 1, 1, []), 'RangeError', 'no radii');
        throws(() => ctx.roundRect(0, 0, 1, 1, -1), 'RangeError', 'negative radii');
        ctx.setTransform(1, 0, 0, 1, 5, 5); eq(ctx.getTransform().e, 5, 'set transform'); ctx.setTransform({ a: 2, d: 2 }); eq(ctx.getTransform().a, 2, 'matrix init');
        ctx.setTransform(); eq(ctx.getTransform().isIdentity, true, 'identity'); ctx.setTransform(NaN, 0, 0, 1, 0, 0); eq(ctx.getTransform().isIdentity, true, 'non-finite ignored');
        ctx.rotate(Math.PI / 2); ctx.transform(NaN, 0, 0, 0, 0, 0); ctx.beginPath(); ctx.rect(1, 0, 1, 1); ctx.fill();
        "#,
    );
    let commands = commands(&page);
    assert!(matches!(
        commands[0],
        CanvasCommand::Fill {
            rule: nativeboat_css::types::FillRule::EvenOdd,
            ..
        }
    ));
    close(
        path_of(&commands[0]),
        &[Move(12.0, 22.0), Line(14.0, 22.0), Close],
    );
    close(
        path_of(&commands[1]),
        &[
            Move(5.0, 5.0),
            Cubic(1.0, 2.0, 3.0, 4.0, 5.0, 6.0),
            Cubic(7.0, 22.0 / 3.0, 9.0, 22.0 / 3.0, 11.0, 6.0),
        ],
    );
    let circle = path_of(&commands[2]);
    assert_eq!(circle.len(), 5, "a full circle is four curves");
    assert_eq!(circle[0], Move(10.0, 0.0));
    // A quarter clockwise, the long way back anticlockwise, and a whole ellipse.
    assert_eq!(path_of(&commands[3]).len(), 2 + 1 + 3 + 1);
    assert!(
        path_of(&commands[4]).len() >= 3,
        "curves start new subpaths"
    );
    let corner = path_of(&commands[5]);
    assert_eq!(corner[0], Move(0.0, 0.0));
    assert_eq!(
        corner[1],
        Line(5.0, 0.0),
        "the corner arc starts on the first line"
    );
    assert!(
        matches!(corner[3], Cubic(_, _, _, _, x, y) if (x, y) == (10.0, 5.0)),
        "{corner:?}"
    );
    assert_eq!(corner[4], Line(10.0, 10.0), "a zero radius draws a line");
    assert_eq!(path_of(&commands[6])[0], Move(5.0, 5.0));
    assert!(matches!(commands[7], CanvasCommand::Clip { .. }));
    assert_eq!(path_of(&commands[7]).len(), 5);
    assert!(path_of(&commands[8]).len() > 40, "five rounded rectangles");
    close(
        path_of(&commands[9]),
        &[
            Move(0.0, 1.0),
            Line(0.0, 2.0),
            Line(-1.0, 2.0),
            Line(-1.0, 1.0),
            Close,
        ],
    );
}

#[test]
fn path2d_objects_draw_under_the_transform() {
    let mut page = fixture(PAGE);
    page.check(
        r#"
        const ctx = ids('#c').getContext('2d');
        const path = new Path2D(); path.rect(0, 0, 2, 2); path.moveTo(5, 5); path.lineTo(6, 6);
        const copy = new Path2D(path); const parsed = new Path2D('M0 0 L10 0 Z'); const empty = new Path2D('');
        copy.addPath(parsed, { e: 100 }); copy.addPath(empty);
        ctx.translate(1, 1); ctx.fill(path, 'evenodd'); ctx.stroke(copy); ctx.clip(parsed, 'nonzero');
        "#,
    );
    let commands = commands(&page);
    close(
        path_of(&commands[0]),
        &[
            Move(1.0, 1.0),
            Line(3.0, 1.0),
            Line(3.0, 3.0),
            Line(1.0, 3.0),
            Close,
            Move(6.0, 6.0),
            Line(7.0, 7.0),
        ],
    );
    let copy = path_of(&commands[1]);
    assert_eq!(copy.len(), 10);
    assert_eq!(copy[7], Move(101.0, 1.0));
    close(
        path_of(&commands[2]),
        &[Move(1.0, 1.0), Line(11.0, 1.0), Close],
    );
}

#[test]
fn styles_gradients_and_state() {
    let mut page = fixture(PAGE);
    page.check(
        r#"
        const ctx = ids('#c').getContext('2d');
        const linear = ctx.createLinearGradient(0, 0, 10, 0); linear.addColorStop(0, 'red'); linear.addColorStop(1, 'blue');
        throws(() => linear.addColorStop(2, 'red'), 'IndexSizeError', 'offset range');
        throws(() => linear.addColorStop(0.5, 'bogus'), 'SyntaxError', 'bad color');
        throws(() => ctx.createLinearGradient(NaN, 0, 0, 0), 'TypeError', 'non-finite');
        throws(() => ctx.createRadialGradient(0, 0, -1, 0, 0, 1), 'IndexSizeError', 'negative radius');
        throws(() => ctx.createRadialGradient(0, 0, NaN, 0, 0, 1), 'TypeError', 'non-finite radial');
        throws(() => new CanvasGradient(), 'TypeError', 'illegal gradient');
        throws(() => new CanvasPattern(), 'TypeError', 'illegal pattern');
        throws(() => new CanvasRenderingContext2D(), 'TypeError', 'illegal context');
        const radial = ctx.createRadialGradient(5, 5, 0, 5, 5, 10); radial.addColorStop(0.5, 'red');
        const conic = ctx.createConicGradient(0, 1, 1); const pattern = ctx.createPattern(ids('#other'), 'repeat'); pattern.setTransform();
        ctx.fillStyle = linear; eq(ctx.fillStyle, linear, 'gradient style'); ctx.scale(2, 2); ctx.fillRect(0, 0, 1, 1);
        ctx.fillStyle = radial; ctx.fillRect(0, 0, 1, 1); ctx.fillStyle = pattern; ctx.fillRect(0, 0, 1, 1); ctx.fillStyle = conic; ctx.fillRect(0, 0, 1, 1);
        ctx.strokeStyle = linear; ctx.lineWidth = 3; ctx.lineWidth = -1; ctx.lineWidth = 'x'; eq(ctx.lineWidth, 3, 'line width');
        ctx.lineCap = 'round'; ctx.lineCap = 'bogus'; ctx.lineJoin = 'bevel'; ctx.miterLimit = 4; ctx.miterLimit = 0;
        ctx.setLineDash([1, 2, 3]); eq(ctx.getLineDash(), [1, 2, 3, 1, 2, 3], 'odd dashes repeat'); ctx.setLineDash([1, -1]); ctx.setLineDash([NaN]);
        ctx.lineDashOffset = 2; ctx.lineDashOffset = 'x'; eq(ctx.lineDashOffset, 2, 'dash offset');
        ctx.globalAlpha = 0.5; ctx.globalAlpha = 2; eq(ctx.globalAlpha, 0.5, 'alpha'); ctx.globalCompositeOperation = 'copy'; ctx.globalCompositeOperation = 'bogus';
        ctx.beginPath(); ctx.moveTo(0, 0); ctx.lineTo(1, 0); ctx.stroke();
        ctx.save(); ctx.fillStyle = 'blue'; ctx.globalCompositeOperation = 'source-over'; ctx.restore(); eq(ctx.fillStyle, conic, 'restored'); ctx.restore();
        eq([ctx.lineCap, ctx.lineJoin, ctx.miterLimit, ctx.globalCompositeOperation], ['round', 'bevel', 4, 'copy'], 'state');
        for (const [name, value] of [['shadowBlur', 2], ['shadowColor', 'red'], ['shadowOffsetX', 1], ['shadowOffsetY', 1], ['filter', 'none'], ['imageSmoothingEnabled', false], ['imageSmoothingQuality', 'high'], ['letterSpacing', '1px'], ['wordSpacing', '1px'], ['fontKerning', 'none'], ['fontStretch', 'normal'], ['fontVariantCaps', 'normal'], ['textRendering', 'auto'], ['direction', 'rtl']]) {
          ctx[name] = value; eq(ctx[name], value, name);
        }
        assert(!ctx.isPointInPath(0, 0) && !ctx.isPointInStroke(0, 0), 'hit tests');
        "#,
    );
    let commands = commands(&page);
    let paints: Vec<&CanvasPaint> = commands
        .iter()
        .filter_map(|command| match command {
            CanvasCommand::Fill { paint, .. } | CanvasCommand::Stroke { paint, .. } => Some(paint),
            _ => None,
        })
        .collect();
    assert!(
        matches!(paints[0], CanvasPaint::Linear { start: (0.0, 0.0), end: (20.0, 0.0), stops } if stops.len() == 2),
        "{paints:?}"
    );
    assert!(
        matches!(
            paints[1],
            CanvasPaint::Radial {
                end: (10.0, 10.0, 20.0),
                ..
            }
        ),
        "{paints:?}"
    );
    assert!(matches!(paints[2], CanvasPaint::Color(color) if color.a == 0.0));
    assert!(matches!(paints[3], CanvasPaint::Linear { .. }));
    let stroke = commands
        .iter()
        .find(|command| matches!(command, CanvasCommand::Stroke { .. }))
        .unwrap();
    let CanvasCommand::Stroke {
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
        ..
    } = stroke
    else {
        unreachable!()
    };
    assert!(
        matches!(
            paint,
            CanvasPaint::Linear {
                end: (10.0, 0.0),
                ..
            }
        ),
        "strokes keep user space"
    );
    assert_eq!(
        (*width, *miter, *offset, *alpha, *copy),
        (3.0, 4.0, 2.0, 0.5, true)
    );
    assert_eq!(
        (*cap, *join),
        (
            nativeboat_css::types::LineCap::Round,
            nativeboat_css::types::LineJoin::Bevel
        )
    );
    assert_eq!(dash, &[1.0, 2.0, 3.0, 1.0, 2.0, 3.0]);
    assert_eq!(*transform, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    assert!(commands.contains(&CanvasCommand::Save) && commands.contains(&CanvasCommand::Restore));
    page.check(
        "const ctx = ids('#c').getContext('2d'); ctx.fillRect(0, 0, 1, 1); ctx.reset();         eq([ctx.fillStyle, ctx.lineWidth, ctx.getTransform().isIdentity], ['#000000', 1, true], 'reset');",
    );
    let sizes: Vec<String> = batches(&page).into_iter().map(|batch| batch.size).collect();
    assert_eq!(sizes, ["0x0", "300x150"], "reset drops pending drawing");
}

#[test]
fn text_is_measured_and_aligned() {
    let mut page = fixture(PAGE);
    page.check(
        r#"
        const ctx = ids('#c').getContext('2d');
        eq(ctx.font, '10px sans-serif', 'default font'); ctx.font = 'bogus'; eq(ctx.font, '10px sans-serif', 'invalid font');
        eq(ctx.measureText('abc').width, 15, 'default metrics');
        ctx.font = ' bold 20px serif '; eq(ctx.font, 'bold 20px serif', 'font');
        const metrics = ctx.measureText('ab'); eq([metrics.width, metrics.fontBoundingBoxAscent, metrics.actualBoundingBoxDescent], [20, 16, 4], 'metrics');
        ctx.textAlign = 'center'; eq(ctx.measureText('ab').actualBoundingBoxLeft, 10, 'centered box'); ctx.textAlign = 'bogus';
        ctx.textBaseline = 'top'; ctx.fillText('ab', 10, 20);
        ctx.textAlign = 'end'; ctx.textBaseline = 'middle'; ctx.fillText('a\nb', 10, 20, 5);
        ctx.textAlign = 'start'; ctx.direction = 'rtl'; ctx.textBaseline = 'bottom'; ctx.strokeText('ab', 0, 0, 100);
        ctx.direction = 'ltr'; ctx.textAlign = 'right'; ctx.textBaseline = 'hanging'; ctx.fillText('ab', 0, 0);
        ctx.textAlign = 'left'; ctx.textBaseline = 'ideographic'; ctx.fillText('ab', 0, 0); ctx.fillText('ab', NaN, 0);
        "#,
    );
    let texts: Vec<CanvasCommand> = commands(&page);
    assert_eq!(texts.len(), 5);
    let CanvasCommand::Text {
        text,
        y,
        anchor,
        font,
        stroke,
        ..
    } = &texts[0]
    else {
        panic!("{texts:?}");
    };
    assert_eq!(
        (text.as_str(), *y, *anchor, font.weight, *stroke),
        ("ab", 36.0, TextAnchor::Center, 700, None)
    );
    let CanvasCommand::Text {
        text,
        y,
        anchor,
        transform,
        ..
    } = &texts[1]
    else {
        panic!("{texts:?}");
    };
    assert_eq!(
        (text.as_str(), *y, *anchor),
        ("a b", 26.0, TextAnchor::Right)
    );
    assert!(
        (transform[0] - 1.0 / 6.0).abs() < 1e-6,
        "squeezed into the maximum width"
    );
    assert!(
        matches!(&texts[2], CanvasCommand::Text { anchor: TextAnchor::Right, stroke: Some(1.0), y, .. } if *y == -4.0)
    );
    assert!(
        matches!(&texts[3], CanvasCommand::Text { anchor: TextAnchor::Right, y, .. } if (*y - 12.8).abs() < 1e-4)
    );
    assert!(
        matches!(&texts[4], CanvasCommand::Text { anchor: TextAnchor::Left, y, .. } if *y == -4.0)
    );
}

#[test]
fn images_pixels_and_urls() {
    let mut page = fixture(PAGE);
    let other = node(&mut page, "#other");
    page.check(
        r#"
        const ctx = ids('#c').getContext('2d'); const other = ids('#other'); const logo = ids('#logo');
        other.getContext('2d').fillRect(0, 0, 1, 1);
        ctx.drawImage(logo, 1, 2); ctx.drawImage(logo, 1, 2, 3, 4); ctx.drawImage(other, 0, 0, 5, 5, 1, 1, 2, 2);
        ctx.drawImage(new Image(), 0, 0); ctx.drawImage(logo, NaN, 0); ctx.drawImage({}, 0, 0);
        throws(() => ctx.drawImage(null, 0, 0), 'TypeError', 'no image');
        throws(() => ctx.drawImage(new ImageData(1, 1), 0, 0), 'TypeError', 'image data');
        "#,
    );
    let batches = batches(&page);
    assert_eq!(batches.len(), 2, "the other canvas was drawn first");
    assert_eq!(batches[0].node.to_string(), other);
    let images: Vec<&CanvasCommand> = batches[1].commands.iter().collect();
    assert_eq!(images.len(), 3);
    let CanvasCommand::Image {
        source, from, to, ..
    } = images[0]
    else {
        panic!("{images:?}");
    };
    assert!(matches!(source, CanvasSource::Url(url) if url.ends_with("logo.png")));
    assert_eq!((from.width, to.x, to.width), (64.0, 1.0, 64.0));
    assert!(matches!(images[1], CanvasCommand::Image { to, .. } if to.height == 4.0));
    assert!(
        matches!(images[2], CanvasCommand::Image { source: CanvasSource::Canvas(node), from, .. } if node.to_string() == other && from.width == 5.0)
    );
    page.check(
        r#"
        const canvas = ids('#c'); const ctx = canvas.getContext('2d');
        const data = ctx.getImageData(2, 2, -2, 1); eq([data.width, data.height, data.data.length, data.data[4]], [2, 1, 8, 4], 'read');
        assert(data instanceof ImageData && data.data instanceof Uint8ClampedArray, 'image data');
        throws(() => ctx.getImageData(0, 0, 0, 1), 'IndexSizeError', 'empty read');
        const image = ctx.createImageData(2, 2); image.data.set([1, 2, 3, 4], 4);
        ctx.putImageData(image, 10, 20, 1, 0, 1, 1); ctx.putImageData(image, 0, 0, 5, 5, 1, 1);
        eq(ctx.createImageData(image).width, 2, 'copy size'); eq(ctx.createImageData(-3, 1).width, 3, 'absolute size');
        eq(new ImageData(new Uint8ClampedArray(8), 2).height, 1, 'height from data');
        throws(() => new ImageData(new Uint8ClampedArray(7), 2), 'IndexSizeError', 'data length');
        throws(() => new ImageData(0, 1), 'IndexSizeError', 'zero size');
        eq(canvas.toDataURL(), 'data:image/png;base64,iVBO', 'png'); eq(canvas.toDataURL('image/jpeg'), 'data:image/jpeg;base64,/9j/', 'jpeg');
        canvas.toBlob((blob) => { globalThis.blob = blob; });
        const empty = document.createElement('canvas'); empty.width = 0; eq(empty.toDataURL(), 'data:,', 'empty url');
        empty.toBlob((blob) => { globalThis.none = blob; });
        "#,
    );
    page.settle();
    page.check("eq([blob.type, blob.size, none], ['image/png', 3, null], 'blobs');");
    let writes: Vec<String> = page
        .page
        .canvas
        .borrow()
        .iter()
        .filter(|record| record.contains(" write "))
        .cloned()
        .collect();
    assert_eq!(writes.len(), 1);
    assert!(
        writes[0].ends_with("write 11,20 1x1 [1, 2, 3, 4]"),
        "{writes:?}"
    );
}

#[test]
fn resizing_starts_over() {
    let mut page = fixture(PAGE);
    page.check(
        r#"
        const canvas = ids('#c'); canvas.width = 50;
        const ctx = canvas.getContext('2d'); ctx.fillStyle = 'red'; ctx.fillRect(0, 0, 1, 1);
        canvas.height = 40; eq(ctx.fillStyle, '#000000', 'state reset');
        "#,
    );
    let sizes: Vec<String> = batches(&page).into_iter().map(|batch| batch.size).collect();
    assert_eq!(sizes, ["0x0", "50x40"], "pending drawing is dropped");
    page.check(
        r#"
        const m = new DOMMatrix([1, 0, 0, 1, 5, 6]); eq([m.m11, m.m12, m.m21, m.m22, m.m41, m.m42, m.is2D], [1, 0, 0, 1, 5, 6, true], 'components');
        eq(m.translate(1).scale(2).toArray(), [2, 0, 0, 2, 6, 6], 'chained'); eq(new DOMMatrix().isIdentity, true, 'identity');
        eq(m.inverse().e, -5, 'inverse'); assert(Number.isNaN(new DOMMatrix([0, 0, 0, 0, 0, 0]).inverse().a), 'singular');
        eq(m.transformPoint({ x: 1 }), { x: 6, y: 6, z: 0, w: 1 }, 'point'); eq(m.transformPoint().x, 5, 'origin');
        eq(m.multiply(null).e, 5, 'identity init'); eq(new DOMMatrix('x').a, 1, 'bad init');
        "#,
    );
}
