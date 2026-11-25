use super::*;

#[test]
fn pipe_names_follow_the_user_data_directory_ignoring_case() {
    let name = |path: &str| String::from_utf16(&pipe_name(path)).unwrap();
    assert_eq!(name(r"C:\Users\A\App"), name(r"c:\users\a\app"));
    assert_ne!(name(r"C:\Users\A\App"), name(r"C:\Users\B\App"));
    assert!(name("x").starts_with(r"\\.\pipe\nativeboat-instance-"));
    assert!(name("x").ends_with('\0'));
}

#[test]
fn a_later_instance_message_reaches_the_owner() {
    let name = pipe_name(&alloc::format!("test-{}", std::process::id()));
    // SAFETY: a null-terminated name and no security attributes.
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            MESSAGE_LIMIT as u32,
            0,
            core::ptr::null_mut(),
        )
    } as usize;
    assert_ne!(pipe as isize, INVALID_HANDLE);
    let owner = std::thread::spawn(move || (receive(pipe), receive(pipe)));
    let long = alloc::vec![b'x'; 10_000];
    assert!(send(&name, br#"{"argv":["a"]}"#));
    assert!(send(&name, &long));
    let (first, second) = owner.join().unwrap();
    assert_eq!(first.unwrap(), br#"{"argv":["a"]}"#);
    assert_eq!(second.unwrap(), long);
    // SAFETY: the pipe was created above.
    unsafe { CloseHandle(pipe as Handle) };
    // Without an owner nothing receives the message.
    assert!(!send(&pipe_name("nobody-owns-this"), b"{}"));
}

#[test]
fn a_message_sent_before_the_owner_waits_is_still_received() {
    let name = pipe_name(&alloc::format!("early-{}", std::process::id()));
    // SAFETY: a null-terminated name and no security attributes.
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            MESSAGE_LIMIT as u32,
            0,
            core::ptr::null_mut(),
        )
    } as usize;
    // The later instance connects, writes and leaves first.
    assert!(send(&name, br#"{"argv":["early"]}"#));
    assert_eq!(receive(pipe).unwrap(), br#"{"argv":["early"]}"#);
    // SAFETY: the pipe was created above.
    unsafe { CloseHandle(pipe as Handle) };
}
