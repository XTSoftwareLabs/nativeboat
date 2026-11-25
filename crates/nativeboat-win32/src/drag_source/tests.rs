use super::*;
use windows_sys::Win32::System::Memory::GlobalSize;
use windows_sys::Win32::System::Ole::ReleaseStgMedium;

fn object(items: &[(String, String)]) -> *mut c_void {
    Box::into_raw(Box::new(Object {
        vtable: &DATA_VTABLE,
        references: AtomicU32::new(1),
        iid: IID_DATA_OBJECT,
        contents: formats(items),
    }))
    .cast()
}

fn format(kind: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: kind,
        ptd: null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL as u32,
    }
}

#[test]
fn dragged_text_links_and_html_are_offered_in_global_memory() {
    let items = [
        ("text/plain".to_owned(), "hi".to_owned()),
        (
            "text/uri-list".to_owned(),
            "# comment\nhttps://example.com/\nhttps://b/".to_owned(),
        ),
        ("text/html".to_owned(), "<b>hi</b>".to_owned()),
        ("text/x-other".to_owned(), "ignored".to_owned()),
    ];
    let data = object(&items);
    // SAFETY: the test owns the object and the media it gets.
    unsafe {
        let text = format(CF_UNICODETEXT);
        assert_eq!(query_get_data(data, &text), S_OK);
        let mut medium = STGMEDIUM::default();
        assert_eq!(get_data(data, &text, &mut medium), S_OK);
        let handle = medium.u.hGlobal;
        let pointer = GlobalLock(handle).cast::<u8>();
        let bytes = core::slice::from_raw_parts(pointer, GlobalSize(handle)).to_vec();
        GlobalUnlock(handle);
        assert_eq!(bytes, utf16("hi"));
        ReleaseStgMedium(&mut medium);
        let link = formats(&items[1..2]);
        assert_eq!(link[0].1, utf16("https://example.com/"));
        assert_eq!(
            crate::clipboard::decode_html(&formats(&items[2..3])[0].1),
            "<b>hi</b>"
        );
        // Formats it does not hold, or not in global memory, are refused.
        assert_eq!(query_get_data(data, &format(1)), DV_E_FORMATETC);
        let mut stream = format(CF_UNICODETEXT);
        stream.tymed = 4;
        assert_eq!(query_get_data(data, &stream), DV_E_FORMATETC);
        let mut enumerator = null_mut();
        assert_eq!(enumerate(data, 1, &mut enumerator), S_OK);
        assert!(!enumerator.is_null());
        assert_eq!(enumerate(data, 2, &mut enumerator), E_NOTIMPL);
        assert_eq!(release::<DataObjectVtbl, Formats>(data), 0);
    }
}

#[test]
fn drags_continue_while_the_button_is_held_and_stop_on_escape() {
    assert_eq!(decide(false, MK_LBUTTON), S_OK);
    assert_eq!(decide(false, 0), DRAGDROP_S_DROP);
    assert_eq!(decide(true, MK_LBUTTON), DRAGDROP_S_CANCEL);
    assert_eq!(
        allowed_effects("copyMove"),
        DROPEFFECT_COPY | DROPEFFECT_MOVE
    );
    assert_eq!(
        allowed_effects("uninitialized"),
        DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK
    );
    // Nothing to offer, or nothing allowed, drags nothing.
    assert_eq!(drag_out(&[], "copy"), "none");
    assert_eq!(
        drag_out(&[("text/plain".to_owned(), "x".to_owned())], "none"),
        "none"
    );
}
