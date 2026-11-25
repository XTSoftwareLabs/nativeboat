use super::*;
use crate::tests::support::Canvas;
use nativeboat_ir::{Corner, DisplayItem, Rect};

fn fill(rect: Rect, color: Color) -> DisplayItem {
    DisplayItem::Fill {
        rect,
        color,
        radii: [Corner::SQUARE; 4],
    }
}

fn surface(id: u64, rect: Rect, scroll: (f32, f32), items: Vec<DisplayItem>) -> Surface {
    Surface {
        id,
        rect,
        scroll_x: scroll.0,
        scroll_y: scroll.1,
        items,
        ..Surface::default()
    }
}

#[test]
fn trees_paint_nested_scrolled_surfaces_clipped_to_their_bounds() {
    crate::init().unwrap();
    let red = Color::rgb(255, 0, 0);
    let blue = Color::rgb(0, 0, 255);
    let green = Color::rgb(0, 255, 0);
    // A 10×10 scroller at (10, 10), scrolled down 5, holding a blue box
    // at content (0, 5) and a green one running past its bottom.
    let mut scroller = surface(
        1,
        Rect::new(10.0, 10.0, 10.0, 10.0),
        (0.0, 5.0),
        vec![
            fill(Rect::new(0.0, 5.0, 10.0, 5.0), blue),
            fill(Rect::new(0.0, 10.0, 10.0, 30.0), green),
        ],
    );
    // An inner surface scrolled out of view.
    scroller.children.push(surface(
        2,
        Rect::new(0.0, 100.0, 5.0, 5.0),
        (0.0, 0.0),
        vec![fill(Rect::new(0.0, 0.0, 5.0, 5.0), red)],
    ));
    let mut root = surface(
        0,
        Rect::new(0.0, 0.0, 40.0, 40.0),
        (0.0, 0.0),
        vec![fill(Rect::new(30.0, 30.0, 5.0, 5.0), red)],
    );
    root.children.push(scroller);
    let tree = NativeTree { root };
    let canvas = Canvas::new(80, 80);
    // SAFETY: the canvas DC is live.
    unsafe { paint_tree(canvas.dc, &tree, 2.0, (80, 80), Color::rgb(255, 255, 255)) };
    assert_eq!(canvas.pixel(2, 2), (255, 255, 255));
    assert_eq!(canvas.pixel(62, 62), (255, 0, 0));
    // The blue box shows at the scroller's top, the green one under it,
    // and nothing escapes the scroller's bounds.
    assert_eq!(canvas.pixel(22, 22), (0, 0, 255));
    assert_eq!(canvas.pixel(22, 34), (0, 255, 0));
    assert_eq!(canvas.pixel(22, 42), (255, 255, 255));

    // Nothing is painted into an empty area.
    let empty = Canvas::new(4, 4);
    // SAFETY: the canvas DC is live.
    unsafe { paint_tree(empty.dc, &tree, 1.0, (0, 4), Color::rgb(0, 0, 0)) };
    assert_eq!(empty.pixel(1, 1), (255, 255, 255));

    // Nor when GDI has no memory for the off-screen layer.
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Graphics::Gdi::{BITMAPINFO, CreateDIBSection, HBITMAP};
    let mut session = shimforge::Session::new();
    let create = shimforge::mock!(
        session,
        CreateDIBSection,
        unsafe extern "system" fn(
            HDC,
            *const BITMAPINFO,
            u32,
            *mut *mut core::ffi::c_void,
            HANDLE,
            u32,
        ) -> HBITMAP
    );
    create
        .expect()
        .once()
        .returning(|_, _, _, _, _, _| core::ptr::null_mut());
    // SAFETY: the canvas DC is live.
    unsafe { paint_tree(empty.dc, &tree, 1.0, (4, 4), Color::rgb(0, 0, 0)) };
    session.restore();
    assert_eq!(empty.pixel(1, 1), (255, 255, 255));
}
