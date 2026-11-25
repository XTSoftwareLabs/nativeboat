use super::*;
use crate::crypto_platform::{
    BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash,
    BCryptHashData, BCryptOpenAlgorithmProvider,
};
use std::os::windows::io::AsRawHandle;

#[test]
fn writes_reach_their_file_and_failures_are_reported() {
    let path = std::env::temp_dir().join(format!("nativeboat-platform-{}.txt", std::process::id()));
    let file = std::fs::File::create(&path).unwrap();
    assert!(write(file.as_raw_handle(), b"native output"));
    drop(file);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "native output");
    std::fs::remove_file(&path).unwrap();
    assert!(!write(core::ptr::null_mut(), b"x"));
    assert!(write_stdout(""));
    assert!(write_stderr(""));
}

#[test]
fn changing_the_working_directory_uses_utf16_and_preserves_windows_errors() {
    let path = JsStr::from("C:\\資料");
    let mut expected: Vec<u16> = "C:\\資料".encode_utf16().collect();
    expected.push(0);
    {
        let mut session = shimforge::Session::new();
        let change = shimforge::mock!(
            session,
            SetCurrentDirectoryW,
            unsafe extern "system" fn(*const u16) -> i32
        );
        change.expect().once().returning(move |actual| {
            // SAFETY: the shim receives set_current_directory's live buffer,
            // and this test knows its exact length.
            let actual = unsafe { core::slice::from_raw_parts(actual, expected.len()) };
            assert_eq!(actual, expected);
            1
        });
        assert_eq!(set_current_directory(&path), Ok(()));
    }
    {
        let mut session = shimforge::Session::new();
        let change = shimforge::mock!(
            session,
            SetCurrentDirectoryW,
            unsafe extern "system" fn(*const u16) -> i32
        );
        change.expect().once().returning(|_| 0);
        let last_error =
            shimforge::mock!(session, GetLastError, unsafe extern "system" fn() -> u32);
        last_error.expect().once().returning(|| 3);
        assert_eq!(set_current_directory(&path), Err(3));
    }
}

#[test]
fn random_bits_come_from_the_operating_system() {
    let values: std::collections::HashSet<u64> = (0..8).map(|_| random()).collect();
    assert!(values.len() > 1);
}

#[test]
fn crypto_provider_and_hash_creation_failures_release_owned_handles() {
    {
        let mut session = shimforge::Session::new();
        let open = shimforge::mock!(
            session,
            BCryptOpenAlgorithmProvider,
            unsafe extern "system" fn(*mut *mut c_void, *const u16, *const u16, u32) -> i32
        );
        open.expect().once().returning(|_, _, _, _| -1);
        assert!(matches!(
            crate::crypto_platform::CryptoHash::new("sha1"),
            Err(-1)
        ));
    }

    let mut session = shimforge::Session::new();
    let open = shimforge::mock!(
        session,
        BCryptOpenAlgorithmProvider,
        unsafe extern "system" fn(*mut *mut c_void, *const u16, *const u16, u32) -> i32
    );
    open.expect().once().returning(|output, _, _, _| {
        // SAFETY: the mocked call receives the output slot from CryptoHash.
        unsafe { *output = core::ptr::dangling_mut::<c_void>() };
        0
    });
    let create = shimforge::mock!(
        session,
        BCryptCreateHash,
        unsafe extern "system" fn(
            *mut c_void,
            *mut *mut c_void,
            *mut u8,
            u32,
            *mut u8,
            u32,
            u32,
        ) -> i32
    );
    create.expect().once().returning(|_, _, _, _, _, _, _| -2);
    let close = shimforge::mock!(
        session,
        BCryptCloseAlgorithmProvider,
        unsafe extern "system" fn(*mut c_void, u32) -> i32
    );
    close.expect().once().returning(|algorithm, _| {
        assert_eq!(algorithm, core::ptr::dangling_mut::<c_void>());
        0
    });
    assert!(matches!(
        crate::crypto_platform::CryptoHash::new("sha1"),
        Err(-2)
    ));
}

#[test]
fn crypto_updates_and_finalization_report_cng_failures_and_close_handles() {
    fn open(output: *mut *mut c_void, _: *const u16, _: *const u16, _: u32) -> i32 {
        // SAFETY: the mocked call receives the output slot from CryptoHash.
        unsafe { *output = core::ptr::dangling_mut::<c_void>() };
        0
    }

    fn create(
        _: *mut c_void,
        output: *mut *mut c_void,
        _: *mut u8,
        _: u32,
        _: *mut u8,
        _: u32,
        _: u32,
    ) -> i32 {
        // SAFETY: the mocked call receives the output slot from CryptoHash.
        unsafe { *output = core::ptr::dangling_mut::<c_void>() };
        0
    }

    for fail_update in [true, false] {
        let mut session = shimforge::Session::new();
        let open_mock = shimforge::mock!(
            session,
            BCryptOpenAlgorithmProvider,
            unsafe extern "system" fn(*mut *mut c_void, *const u16, *const u16, u32) -> i32
        );
        open_mock.expect().once().returning(open);
        let create_mock = shimforge::mock!(
            session,
            BCryptCreateHash,
            unsafe extern "system" fn(
                *mut c_void,
                *mut *mut c_void,
                *mut u8,
                u32,
                *mut u8,
                u32,
                u32,
            ) -> i32
        );
        create_mock.expect().once().returning(create);
        let destroy = shimforge::mock!(
            session,
            BCryptDestroyHash,
            unsafe extern "system" fn(*mut c_void) -> i32
        );
        destroy.expect().once().returning(|hash| {
            assert_eq!(hash, core::ptr::dangling_mut::<c_void>());
            0
        });
        let close = shimforge::mock!(
            session,
            BCryptCloseAlgorithmProvider,
            unsafe extern "system" fn(*mut c_void, u32) -> i32
        );
        close.expect().once().returning(|algorithm, _| {
            assert_eq!(algorithm, core::ptr::dangling_mut::<c_void>());
            0
        });

        if fail_update {
            let update = shimforge::mock!(
                session,
                BCryptHashData,
                unsafe extern "system" fn(*mut c_void, *mut u8, u32, u32) -> i32
            );
            update.expect().once().returning(|_, _, _, _| -3);
            let mut hash = crate::crypto_platform::CryptoHash::new("sha1").unwrap();
            assert!(matches!(hash.update(b"input"), Err(-3)));
        } else {
            let update = shimforge::mock!(
                session,
                BCryptHashData,
                unsafe extern "system" fn(*mut c_void, *mut u8, u32, u32) -> i32
            );
            update.expect().once().returning(|_, _, _, _| 0);
            let finish = shimforge::mock!(
                session,
                BCryptFinishHash,
                unsafe extern "system" fn(*mut c_void, *mut u8, u32, u32) -> i32
            );
            finish.expect().once().returning(|_, _, _, _| -4);
            let mut hash = crate::crypto_platform::CryptoHash::new("sha1").unwrap();
            hash.update(b"input").unwrap();
            assert!(matches!(hash.finish(), Err(-4)));
        }
    }
}

#[test]
fn the_clock_is_monotonic() {
    let start = now();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let elapsed = now() - start;
    assert!(elapsed >= 4.0, "{elapsed}");
}

#[test]
fn process_identity_comes_from_the_running_application() {
    assert_eq!(process_id(), std::process::id());
}

#[test]
fn process_paths_come_from_the_running_application() {
    use std::os::windows::ffi::OsStrExt;
    let cwd: Vec<u16> = std::env::current_dir()
        .unwrap()
        .as_os_str()
        .encode_wide()
        .collect();
    let exe: Vec<u16> = std::env::current_exe()
        .unwrap()
        .as_os_str()
        .encode_wide()
        .collect();
    assert_eq!(current_directory().unwrap().units(), cwd);
    assert_eq!(executable_path().unwrap().units(), exe);
}

#[test]
fn path_queries_retry_truncation_preserve_units_and_bound_allocation() {
    fn growing(buffer: &mut [u16]) -> u32 {
        // Model a directory growing again between queries, with Windows'
        // required-size (rather than copied-length) overflow return.
        if buffer.len() < 1024 {
            return (buffer.len() + 40) as u32;
        }
        let units = [67, 58, 92, 0x4e2d, 0xd800];
        buffer[..units.len()].copy_from_slice(&units);
        units.len() as u32
    }
    assert_eq!(
        read_path(growing).unwrap().units(),
        [67, 58, 92, 0x4e2d, 0xd800]
    );
    assert_eq!(read_path(|buffer| buffer.len() as u32), Err(206));
    assert_eq!(read_path(|_| u32::MAX), Err(206));
    assert_eq!(directory_path("C:\\".into()), JsStr::from("C:\\"));
    assert_eq!(
        directory_path("C:\\folder\\".into()),
        JsStr::from("C:\\folder")
    );
    assert_eq!(
        directory_path("\\\\server\\share\\".into()),
        JsStr::from("\\\\server\\share")
    );
    assert_eq!(directory_path("".into()), JsStr::from(""));
    assert_eq!(
        directory_path("C:\\folder".into()),
        JsStr::from("C:\\folder")
    );
    unsafe extern "system" {
        fn SetLastError(code: u32);
    }
    assert_eq!(
        read_path(|_| {
            // SAFETY: sets only this test thread's Windows error value.
            unsafe { SetLastError(3) };
            0
        }),
        Err(3)
    );
}

#[test]
fn windows_environment_values_are_live_case_insensitive_and_distinguish_empty() {
    let name = JsStr::from(format!("NATIVEBOAT_ENV_TEST_{}", process_id()).as_str());
    let previous = environment_value(&name);
    assert!(previous.is_none());
    set_environment(&name, Some(&JsStr::from("")));
    assert_eq!(environment_value(&name), Some(JsStr::from("")));
    let value = JsStr::from(format!("{}🛤", "中".repeat(600)).as_str());
    set_environment(&name, Some(&value));
    assert_eq!(
        environment_value(&JsStr::from(name.to_rust_lossy().to_lowercase().as_str())),
        Some(value)
    );
    assert!(environment_keys().contains(&name));
    set_environment(&name, Some(&JsStr::from_units(&[0xd800, 0, 65])));
    assert_eq!(environment_value(&name), Some(JsStr::from("�")));
    let raw_value = [0xd800, 65, 0xdc00, 0];
    let raw_name = environment_text(&name);
    // SAFETY: owned test name and both valid null-terminated buffers; this
    // changes only the test process's variable. Model a native caller's data.
    let changed = unsafe { SetEnvironmentVariableW(raw_name.as_ptr(), raw_value.as_ptr()) };
    assert_ne!(changed, 0);
    assert_eq!(environment_value(&name), Some(JsStr::from("���A���")));
    assert_eq!(environment_text(&JsStr::from("a\0b")), vec![97, 0]);
    set_environment(
        &JsStr::from("=NATIVEBOAT_READONLY"),
        Some(&JsStr::from("ignored")),
    );
    set_environment(&name, None);
    assert_eq!(environment_value(&name), None);
    set_environment(&name, previous.as_ref());
}

#[test]
fn environment_query_growth_and_snapshot_allocation_failures_are_bounded() {
    {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(session, query_environment, fn(&[u16], &mut [u16]) -> u32);
        query.expect().returning(|_, _| u32::MAX);
        assert_eq!(environment_value(&JsStr::from("owned")), None);
    }
    {
        let mut session = shimforge::Session::new();
        let query = shimforge::mock!(session, environment_block, fn() -> *mut u16);
        query.expect().returning(core::ptr::null_mut);
        assert!(std::panic::catch_unwind(environment_keys).is_err());
    }
}

#[test]
fn native_command_arguments_match_windows_startup_quote_rules() {
    let cases: &[(&str, &[&str])] = &[
        ("", &[""]),
        ("app\t ", &["app"]),
        (" app x", &["", "app", "x"]),
        (
            r#""C:\Program Files\a.exe" "a b c" d e"#,
            &[r"C:\Program Files\a.exe", "a b c", "d", "e"],
        ),
        (r#"app "ab\"c" "\\" d"#, &["app", "ab\"c", "\\", "d"]),
        (r#"app a\\\b d"e f"g h"#, &["app", r"a\\\b", "de fg", "h"]),
        (r#"app a\\\"b c d"#, &["app", r#"a\"b"#, "c", "d"]),
        (r#"app a\\\\"b c" d e"#, &["app", r"a\\b c", "d", "e"]),
        (r#"app a"b"" c d"#, &["app", "ab\" c d"]),
        (
            r#"app "" "a""b" "unterminated"#,
            &["app", "", "a\"b", "unterminated"],
        ),
        ("app end\\", &["app", "end\\"]),
        (
            "app ^x hello\nworld\0ignored",
            &["app", "^x", "hello\nworld"],
        ),
        (r#""unclosed program name"#, &["unclosed program name"]),
    ];
    for (command, expected) in cases {
        let line: Vec<u16> = command.encode_utf16().collect();
        let actual: Vec<String> = parse_arguments(&line)
            .into_iter()
            .map(|value| value.to_rust_lossy())
            .collect();
        assert_eq!(actual, *expected, "{command}");
    }
    assert_eq!(
        parse_arguments(&[97, 32, 0xd800, 0xdc00, 32, 0xd800]),
        vec![JsStr::from("a"), JsStr::from("𐀀"), JsStr::from("�")]
    );
}

#[test]
fn managed_command_line_matches_the_native_test_process_arguments() {
    use std::os::windows::ffi::OsStrExt;
    let expected: Vec<JsStr> = std::env::args_os()
        .map(|argument| {
            let units: Vec<u16> = argument.encode_wide().collect();
            JsStr::from(String::from_utf16_lossy(&units).as_str())
        })
        .collect();
    assert_eq!(command_arguments(), expected);
}

#[test]
fn standard_streams_take_bytes_and_report_consoles() {
    assert!(write_standard(1, b""));
    assert!(write_standard(2, b""));
    // Test runs may or may not have consoles; both queries must answer.
    let _ = (is_console(1), is_console(2));
}

#[test]
fn exiting_passes_the_code_to_windows() {
    let mut session = shimforge::Session::new();
    let exit = shimforge::mock!(session, ExitProcess, unsafe extern "system" fn(u32));
    exit.expect().once().returning(|code| assert_eq!(code, 7));
    exit_process(7);
}

#[test]
fn process_signal_failures_release_open_handles_and_preserve_windows_errors() {
    for (signal, wait, expected) in [(0, 0, 87), (0, u32::MAX, 5), (15, 258, 5)] {
        let mut session = shimforge::Session::new();
        let open = shimforge::mock!(
            session,
            OpenProcess,
            unsafe extern "system" fn(u32, i32, u32) -> *mut c_void
        );
        open.expect()
            .once()
            .returning(|_, _, _| core::ptr::dangling_mut());
        let close = shimforge::mock!(
            session,
            CloseHandle,
            unsafe extern "system" fn(*mut c_void) -> i32
        );
        close.expect().once().returning(|_| 1);
        if signal == 0 {
            let waiting = shimforge::mock!(
                session,
                WaitForSingleObject,
                unsafe extern "system" fn(*mut c_void, u32) -> u32
            );
            waiting.expect().once().returning(move |_, _| wait);
        } else {
            let terminate = shimforge::mock!(
                session,
                TerminateProcess,
                unsafe extern "system" fn(*mut c_void, u32) -> i32
            );
            terminate.expect().once().returning(|_, _| 0);
        }
        if expected == 5 {
            let error = shimforge::mock!(session, GetLastError, unsafe extern "system" fn() -> u32);
            error.expect().once().returning(|| 5);
        }
        assert_eq!(kill(42, signal), Err(expected));
    }
}

#[test]
fn module_evaluation_tracing_reads_its_switch_once() {
    // Unset, the switch keeps tracing off for the rest of the process.
    trace_module_evaluation("first.js");
    trace_module_evaluation("second.js");
}

#[test]
fn the_system_locale_is_the_regional_format_tag() {
    let locale = system_locale();
    // Windows always has a default user locale, such as `en-US`.
    assert!(locale.len() >= 2, "{locale:?}");
    assert!(
        locale
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "{locale:?}"
    );
    assert!(!locale.contains('\0'));
}

#[test]
fn the_application_locale_is_the_first_preferred_language_with_resources() {
    let preferred = ["zh-CN".to_owned(), "fr-FR".to_owned()];
    // All of Electron's locales: the first preferred language.
    assert_eq!(application_locale(&preferred, &[]), "zh-CN");
    assert_eq!(application_locale(&[], &[]), "en-US");
    // Only some kept, as electron-builder's `electronLanguages` does.
    assert_eq!(application_locale(&preferred, &["en-US"]), "en-US");
    assert_eq!(application_locale(&preferred, &["en-US", "fr"]), "fr");
    assert_eq!(application_locale(&preferred, &["ZH_cn"]), "zh-CN");
    assert_eq!(application_locale(&[], &["de"]), "en-US");
    // Keeping the locale in use changes nothing other tests see.
    let current: &'static str = Box::leak(user_locale().into_boxed_str());
    let kept: &'static [&'static str] = Box::leak(Box::new([current]));
    keep_locales(kept);
    assert_eq!(kept_locales(), [current]);
    assert_eq!(user_locale(), current);
    keep_locales(&[]);
    assert!(kept_locales().is_empty());
    assert_eq!(user_locale(), current);
}

#[test]
fn preferred_languages_follow_windows_language_settings() {
    // Windows always lists at least the display language.
    let preferred = preferred_languages();
    assert!(!preferred.is_empty());
    assert!(
        preferred
            .iter()
            .all(|language| !language.is_empty() && !language.contains('\0'))
    );
    // Pages list the application's locale first, and only once.
    let accepted = accept_languages();
    assert_eq!(accepted[0], user_locale());
    assert_eq!(
        accepted
            .iter()
            .filter(|language| **language == accepted[0])
            .count(),
        1
    );
    assert_eq!(strings(&[97, 0, 98, 99, 0, 0]), ["a", "bc"]);
    assert!(!preferred_ui_languages().is_empty());
    // Without a list in the settings, the preferred UI languages.
    assert_eq!(or_ui_languages(Vec::new()), preferred_ui_languages());
    assert_eq!(or_ui_languages(vec!["fr-FR".into()]), ["fr-FR"]);
}
