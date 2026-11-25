//! Spell checking with the Windows spell checker, as Electron's session
//! spell checker on Windows: whether it is enabled, its languages, the
//! misspelled words of a text, suggestions for a word, and the words added
//! to the custom dictionary. Without the spell checker (before Windows 8),
//! no word is misspelled.

use crate::com::{CoTaskMemFree, Com, Guid, Hresult, S_OK};
use crate::prelude::*;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, Ordering};

const CLSID_SPELL_CHECKER_FACTORY: Guid = Guid {
    data1: 0x7ab3_6653,
    data2: 0x1796,
    data3: 0x484b,
    data4: [0xbd, 0xfa, 0xe7, 0x4f, 0x1d, 0xb7, 0xc1, 0xdc],
};
const IID_SPELL_CHECKER_FACTORY: Guid = Guid {
    data1: 0x8e01_8a9d,
    data2: 0x2415,
    data3: 0x4677,
    data4: [0xbf, 0x08, 0x79, 0x4e, 0xa6, 0x1f, 0x94, 0xbb],
};
// Vtable slots after IUnknown's QueryInterface, AddRef and Release.
const FACTORY_SUPPORTED_LANGUAGES: usize = 3;
const FACTORY_IS_SUPPORTED: usize = 4;
const FACTORY_CREATE: usize = 5;
const CHECKER_CHECK: usize = 4;
const CHECKER_SUGGEST: usize = 5;
const ENUM_NEXT: usize = 3;
const ERROR_START: usize = 3;
const ERROR_LENGTH: usize = 4;
const ERROR_ACTION: usize = 5;
/// `CORRECTIVE_ACTION_GET_SUGGESTIONS` and `CORRECTIVE_ACTION_REPLACE`;
/// repeated words (`DELETE`) are not marked, as in Chromium.
const MISSPELLED: [i32; 2] = [1, 2];

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

/// A null-terminated wide copy of a string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// The strings an `IEnumString` lists, freeing each.
fn strings(list: Com) -> Vec<String> {
    let mut found = Vec::new();
    loop {
        let mut text: *mut u16 = core::ptr::null_mut();
        let mut fetched = 0u32;
        // SAFETY: Next writes one string the caller frees, and its count.
        let result = unsafe {
            list.method::<unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16, *mut u32) -> Hresult>(
                ENUM_NEXT,
            )(list.0, 1, &mut text, &mut fetched)
        };
        if result != S_OK || fetched != 1 || text.is_null() {
            return found;
        }
        // SAFETY: the string is null-terminated and freed after copying.
        unsafe {
            let length = (0..).take_while(|index| *text.add(*index) != 0).count();
            found.push(String::from_utf16_lossy(core::slice::from_raw_parts(
                text, length,
            )));
            CoTaskMemFree(text.cast());
        }
    }
}

/// What spell checking keeps between calls.
struct State {
    enabled: bool,
    /// The languages set, in order; empty for the user's language.
    languages: Vec<String>,
    /// `None` until first needed; then the factory, if Windows has one.
    factory: Option<Option<Com>>,
    /// Checkers by language.
    checkers: Vec<(String, Com)>,
    /// Words the user added, correct in every language.
    words: Vec<String>,
    /// The thread whose COM apartment the factory and checkers belong to.
    thread: u32,
}

impl State {
    fn factory(&mut self) -> Option<&Com> {
        self.factory
            .get_or_insert_with(|| {
                Com::create(&CLSID_SPELL_CHECKER_FACTORY, &IID_SPELL_CHECKER_FACTORY)
            })
            .as_ref()
    }

    fn supported(&mut self) -> Vec<String> {
        let Some(factory) = self.factory() else {
            return Vec::new();
        };
        let mut list = core::ptr::null_mut();
        // SAFETY: the factory is live and the out pointer writable.
        let result = unsafe {
            factory.method::<unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult>(
                FACTORY_SUPPORTED_LANGUAGES,
            )(factory.0, &mut list)
        };
        if result != S_OK || list.is_null() {
            return Vec::new();
        }
        strings(Com(list))
    }

    /// The checkers of the languages in use: those set, else the user's.
    fn active(&mut self) -> Vec<usize> {
        let languages = if self.languages.is_empty() {
            vec![crate::platform::user_locale()]
        } else {
            self.languages.clone()
        };
        let mut active = Vec::new();
        for language in languages {
            if let Some(index) = self.checker(&language) {
                active.push(index);
            }
        }
        active
    }

    fn checker(&mut self, language: &str) -> Option<usize> {
        if let Some(index) = self
            .checkers
            .iter()
            .position(|(known, _)| known == language)
        {
            return Some(index);
        }
        let factory = self.factory()?;
        let tag = wide(language);
        let mut supported = 0i32;
        let mut checker = core::ptr::null_mut();
        // SAFETY: the factory is live, the tag null-terminated and the out
        // pointers writable.
        unsafe {
            let result = factory.method::<unsafe extern "system" fn(
                *mut c_void,
                *const u16,
                *mut i32,
            ) -> Hresult>(FACTORY_IS_SUPPORTED)(
                factory.0, tag.as_ptr(), &mut supported
            );
            if result != S_OK || supported == 0 {
                return None;
            }
            let result =
                factory.method::<unsafe extern "system" fn(
                    *mut c_void,
                    *const u16,
                    *mut *mut c_void,
                ) -> Hresult>(FACTORY_CREATE)(factory.0, tag.as_ptr(), &mut checker);
            if result != S_OK || checker.is_null() {
                return None;
            }
        }
        self.checkers.push((language.to_owned(), Com(checker)));
        Some(self.checkers.len() - 1)
    }

    /// The UTF-16 ranges a checker finds misspelled in a text.
    fn errors(&self, checker: &Com, text: &[u16]) -> Vec<(u32, u32)> {
        let terminated: Vec<u16> = text.iter().copied().chain([0]).collect();
        let mut list = core::ptr::null_mut();
        // SAFETY: the checker is live, the text null-terminated and the out
        // pointer writable.
        let result = unsafe {
            checker.method::<unsafe extern "system" fn(*mut c_void, *const u16, *mut *mut c_void) -> Hresult>(
                CHECKER_CHECK,
            )(checker.0, terminated.as_ptr(), &mut list)
        };
        if result != S_OK || list.is_null() {
            return Vec::new();
        }
        let list = Com(list);
        let mut found = Vec::new();
        loop {
            let mut error = core::ptr::null_mut();
            // SAFETY: Next writes an error the caller releases.
            let result = unsafe {
                list.method::<unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult>(
                    ENUM_NEXT,
                )(list.0, &mut error)
            };
            if result != S_OK || error.is_null() {
                return found;
            }
            let error = Com(error);
            let (mut start, mut length, mut action) = (0u32, 0u32, 0i32);
            // SAFETY: the error is live and the outputs writable.
            unsafe {
                error.method::<unsafe extern "system" fn(*mut c_void, *mut u32) -> Hresult>(
                    ERROR_START,
                )(error.0, &mut start);
                error.method::<unsafe extern "system" fn(*mut c_void, *mut u32) -> Hresult>(
                    ERROR_LENGTH,
                )(error.0, &mut length);
                error.method::<unsafe extern "system" fn(*mut c_void, *mut i32) -> Hresult>(
                    ERROR_ACTION,
                )(error.0, &mut action);
            }
            let end = start.saturating_add(length);
            if MISSPELLED.contains(&action) && end as usize <= text.len() && length > 0 {
                let word = String::from_utf16_lossy(&text[start as usize..end as usize]);
                if !self.words.contains(&word) {
                    found.push((start, end));
                }
            }
        }
    }
}

/// The spell checker's state, used from one UI thread at a time.
struct Shared {
    busy: AtomicBool,
    state: UnsafeCell<Option<State>>,
}

// SAFETY: the state is only reached through `with`, which holds `busy`; COM
// pointers in it are used on the thread holding the lock.
unsafe impl Sync for Shared {}

static SHARED: Shared = Shared {
    busy: AtomicBool::new(false),
    state: UnsafeCell::new(None),
};

fn with<R>(run: impl FnOnce(&mut State) -> R) -> R {
    while SHARED
        .busy
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            SHARED.busy.store(false, Ordering::Release);
        }
    }
    let _release = Release;
    // SAFETY: `busy` gives this call the only access.
    let state = unsafe { &mut *SHARED.state.get() }.get_or_insert_with(|| State {
        enabled: true,
        languages: Vec::new(),
        factory: None,
        checkers: Vec::new(),
        words: Vec::new(),
        thread: 0,
    });
    // COM objects belong to the apartment of the thread that made them,
    // which may have ended: another thread leaves them, unused and
    // unreleased, and makes its own.
    // SAFETY: reads the calling thread's id.
    let thread = unsafe { GetCurrentThreadId() };
    if state.thread != thread {
        state.thread = thread;
        if let Some(Some(factory)) = state.factory.take() {
            core::mem::forget(factory);
        }
        for (_, checker) in state.checkers.drain(..) {
            core::mem::forget(checker);
        }
    }
    run(state)
}

/// Whether spell checking is enabled; Electron enables it by default.
pub fn enabled() -> bool {
    with(|state| state.enabled)
}

/// Enables or disables spell checking.
pub fn set_enabled(enabled: bool) {
    with(|state| state.enabled = enabled);
}

/// The languages set for spell checking.
pub fn languages() -> Vec<String> {
    with(|state| state.languages.clone())
}

/// Sets the languages to check, keeping those Windows can check.
pub fn set_languages(languages: &[String]) {
    with(|state| {
        let supported = state.supported();
        state.languages = languages
            .iter()
            .filter(|language| {
                supported
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(language))
            })
            .cloned()
            .collect();
    });
}

/// The languages Windows can check.
pub fn available_languages() -> Vec<String> {
    with(State::supported)
}

/// The UTF-16 ranges of the words misspelled in every language checked,
/// ascending; none while spell checking is disabled.
pub fn misspellings(text: &[u16]) -> Vec<(u32, u32)> {
    with(|state| {
        if !state.enabled || text.is_empty() {
            return Vec::new();
        }
        let active = state.active();
        let mut found: Option<Vec<(u32, u32)>> = None;
        for index in active {
            let errors = state.errors(&state.checkers[index].1, text);
            // A word is misspelled when no language knows it.
            found = Some(match found {
                None => errors,
                Some(previous) => previous
                    .into_iter()
                    .filter(|range| errors.contains(range))
                    .collect(),
            });
        }
        found.unwrap_or_default()
    })
}

/// Suggestions for a misspelled word, best first.
pub fn suggestions(word: &str) -> Vec<String> {
    with(|state| {
        let mut found: Vec<String> = Vec::new();
        for index in state.active() {
            let checker = &state.checkers[index].1;
            let wide = wide(word);
            let mut list = core::ptr::null_mut();
            // SAFETY: the checker is live, the word null-terminated and the
            // out pointer writable.
            let result = unsafe {
                checker.method::<unsafe extern "system" fn(
                    *mut c_void,
                    *const u16,
                    *mut *mut c_void,
                ) -> Hresult>(CHECKER_SUGGEST)(checker.0, wide.as_ptr(), &mut list)
            };
            if result == S_OK && !list.is_null() {
                for suggestion in strings(Com(list)) {
                    if !found.contains(&suggestion) {
                        found.push(suggestion);
                    }
                }
            }
        }
        found
    })
}

/// Adds a word to the custom dictionary; `false` if it was there.
pub fn add_word(word: &str) -> bool {
    with(|state| {
        let added = !state.words.iter().any(|known| known == word);
        if added {
            state.words.push(word.to_owned());
        }
        added
    })
}

/// Removes a word from the custom dictionary; `false` if it was not there.
pub fn remove_word(word: &str) -> bool {
    with(|state| {
        let before = state.words.len();
        state.words.retain(|known| known != word);
        state.words.len() != before
    })
}

/// The words in the custom dictionary.
pub fn words() -> Vec<String> {
    with(|state| state.words.clone())
}

/// Held by tests that use the process's spell checker state, which tests
/// on other threads would otherwise change under them.
#[cfg(test)]
pub(crate) static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests;
