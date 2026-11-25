use super::*;

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, model: u32) -> Hresult;
}

#[test]
fn the_windows_spell_checker_finds_misspellings_and_suggestions() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    // SAFETY: initializes COM for this test thread, as window loops do.
    unsafe { CoInitializeEx(core::ptr::null_mut(), 2) };
    let available = available_languages();
    if !available.iter().any(|language| language == "en-US") {
        // Windows without the English spell checker has nothing to check.
        return;
    }
    assert!(enabled());
    set_languages(&["en-US".to_owned(), "xx-unknown".to_owned()]);
    assert_eq!(languages(), ["en-US"]);
    let text: Vec<u16> = "this is a tset of the the speling".encode_utf16().collect();
    assert_eq!(misspellings(&text), [(10, 14), (26, 33)]);
    assert!(suggestions("speling").iter().any(|word| word == "spelling"));
    assert!(add_word("tset"));
    assert!(!add_word("tset"));
    assert_eq!(words(), ["tset"]);
    assert_eq!(misspellings(&text), [(26, 33)]);
    assert!(remove_word("tset"));
    assert!(!remove_word("tset"));
    set_enabled(false);
    assert!(misspellings(&text).is_empty());
    set_enabled(true);
    assert!(misspellings(&[]).is_empty());
    set_languages(&[]);
}

#[test]
fn threads_check_spelling_with_their_own_checkers() {
    let _serial = SERIAL.lock().unwrap_or_else(|error| error.into_inner());
    // Each thread's checkers belong to its COM apartment: a thread checking
    // after one that ended does not reach that one's.
    let check = || {
        std::thread::spawn(|| {
            // SAFETY: initializes COM for this thread, as window loops do.
            unsafe { CoInitializeEx(core::ptr::null_mut(), 2) };
            if !available_languages()
                .iter()
                .any(|language| language == "en-US")
            {
                return None;
            }
            set_languages(&["en-US".to_owned()]);
            let text: Vec<u16> = "a tset".encode_utf16().collect();
            let found = misspellings(&text);
            set_languages(&[]);
            Some(found)
        })
        .join()
        .unwrap()
    };
    let first = check();
    assert_eq!(check(), first);
    assert!(first.is_none_or(|found| found == [(2, 6)]));
}
