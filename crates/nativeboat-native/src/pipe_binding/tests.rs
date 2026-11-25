use super::*;

fn done<T>(outcome: Outcome<T>) -> T {
    match outcome {
        Outcome::Done(value) => value,
        Outcome::Failed(error) => panic!("failed with {error}"),
        Outcome::Cancelled => panic!("cancelled"),
    }
}

/// A pipe name of the test's own.
fn name(test: &str) -> String {
    alloc::format!(
        "\\\\.\\pipe\\nativeboat-test-{test}-{:x}",
        crate::platform::random()
    )
}

fn connection(pipe: Handle) -> Arc<Shared> {
    Shared::new(pipe, None).unwrap()
}

#[test]
fn pipe_connections_exchange_bytes_and_see_the_end_of_the_stream() {
    let path = name("exchange");
    let listener = server(&path).unwrap();
    // A second server cannot take the name, as libuv reports.
    let taken = server(&path).err().unwrap();
    assert_eq!(taken.error, WSAEADDRINUSE);
    assert_eq!(
        crate::uv_error::describe(crate::uv_error::translate(taken.error)).0,
        "EADDRINUSE"
    );
    let target = path.clone();
    let client = std::thread::spawn(move || connection(connect_to(&target).ok().unwrap()));
    let accepted = connection(done(accept_next(&listener)) as Handle);
    let client = client.join().unwrap();

    let message = alloc::vec![9u8; 3 * BUFFER_SIZE as usize + 17];
    let writer = client.clone();
    let sent = message.clone();
    let writing = std::thread::spawn(move || done(write(&writer, &sent)));
    let mut received = Vec::new();
    while received.len() < message.len() {
        let chunk = done(read(&accepted));
        assert!(!chunk.is_empty() && chunk.len() <= BUFFER_SIZE as usize);
        received.extend(chunk);
    }
    writing.join().unwrap();
    assert_eq!(received, message);
    done(write(&accepted, b"reply"));
    assert_eq!(done(read(&client)), b"reply");

    // The server waits for the next client on a new instance.
    let target = path.clone();
    let second = std::thread::spawn(move || connection(connect_to(&target).ok().unwrap()));
    let accepted_second = connection(done(accept_next(&listener)) as Handle);
    let second = second.join().unwrap();
    done(write(&second, b"two"));
    assert_eq!(done(read(&accepted_second)), b"two");

    // Closing one end is the end of the other's stream.
    drop(client);
    assert!(done(read(&accepted)).is_empty());
}

#[test]
fn pipes_that_no_server_holds_fail_to_connect_with_enoent() {
    let failure = connect_to(&name("missing")).err().unwrap();
    assert_eq!(failure.syscall, "connect");
    assert_eq!(
        crate::uv_error::describe(crate::uv_error::translate(failure.error)).0,
        "ENOENT"
    );
}

#[test]
fn closing_a_pipe_handle_ends_its_pending_waits() {
    let path = name("cancel");
    let listener = server(&path).unwrap();
    let waiting = listener.clone();
    let accepting = std::thread::spawn(move || matches!(accept_next(&waiting), Outcome::Cancelled));
    std::thread::sleep(std::time::Duration::from_millis(50));
    // SAFETY: the event is open.
    unsafe { SetEvent(listener.cancel.0) };
    assert!(accepting.join().unwrap());

    let path = name("cancel-read");
    let listener = server(&path).unwrap();
    let target = path.clone();
    let client = std::thread::spawn(move || connection(connect_to(&target).ok().unwrap()));
    let accepted = connection(done(accept_next(&listener)) as Handle);
    let _client = client.join().unwrap();
    let reader = accepted.clone();
    let reading = std::thread::spawn(move || matches!(read(&reader), Outcome::Cancelled));
    std::thread::sleep(std::time::Duration::from_millis(50));
    // SAFETY: the event is open.
    unsafe { SetEvent(accepted.cancel.0) };
    assert!(reading.join().unwrap());
}
