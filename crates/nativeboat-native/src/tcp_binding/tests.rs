use super::*;

fn connection(socket: Socket) -> Arc<Shared> {
    Shared::new(socket).unwrap()
}

fn port_of(shared: &Shared) -> u16 {
    socket_address(shared.socket, false).unwrap().port
}

fn done<T>(outcome: Outcome<T>) -> T {
    match outcome {
        Outcome::Done(value) => value,
        Outcome::Failed(error) => panic!("failed with {error}"),
        Outcome::Cancelled => panic!("cancelled"),
    }
}

#[test]
fn connections_exchange_bytes_and_see_the_end_of_the_stream() {
    let listener = server("127.0.0.1", 0, 16, false).unwrap();
    let port = port_of(&listener);
    let client = std::thread::spawn(move || connection(connect_to("127.0.0.1", port).unwrap()));
    let accepted = connection(done(accept_next(&listener)));
    let client = client.join().unwrap();
    assert_eq!(
        socket_address(accepted.socket, false).unwrap(),
        Address {
            address: "127.0.0.1".into(),
            port,
            family: "IPv4",
        }
    );
    assert_eq!(socket_address(client.socket, true).unwrap().port, port);

    let message = alloc::vec![7u8; 3 * READ_SIZE];
    let writer = client.clone();
    let sent = message.clone();
    let writing = std::thread::spawn(move || done(write(&writer, &sent)));
    let mut received = Vec::new();
    while received.len() < message.len() {
        let chunk = done(read(&accepted));
        assert!(!chunk.is_empty() && chunk.len() <= READ_SIZE);
        received.extend(chunk);
    }
    writing.join().unwrap();
    assert_eq!(received, message);

    done(write(&accepted, b"reply"));
    assert_eq!(done(read(&client)), b"reply");
    // SAFETY: the socket is open.
    unsafe { shutdown(client.socket, SD_SEND) };
    assert!(done(read(&accepted)).is_empty());
}

#[test]
fn closing_a_handle_ends_its_pending_waits() {
    let listener = server("127.0.0.1", 0, 16, false).unwrap();
    let port = port_of(&listener);
    let waiting = listener.clone();
    let accepting = std::thread::spawn(move || matches!(accept_next(&waiting), Outcome::Cancelled));
    std::thread::sleep(std::time::Duration::from_millis(50));
    // SAFETY: the event is open.
    unsafe { SetEvent(listener.cancel.0) };
    assert!(accepting.join().unwrap());

    let listener = server("127.0.0.1", 0, 16, false).unwrap();
    let port2 = port_of(&listener);
    assert_ne!(port, 0);
    let client = std::thread::spawn(move || connection(connect_to("localhost", port2).unwrap()));
    let accepted = connection(done(accept_next(&listener)));
    let client = client.join().unwrap();
    let reader = accepted.clone();
    let reading = std::thread::spawn(move || matches!(read(&reader), Outcome::Cancelled));
    std::thread::sleep(std::time::Duration::from_millis(50));
    // SAFETY: the event is open.
    unsafe { SetEvent(accepted.cancel.0) };
    assert!(reading.join().unwrap());
    drop(accepted);
    // The peer sees the connection end once the last user lets it go.
    let end = read(&client);
    assert!(
        matches!(&end, Outcome::Done(bytes) if bytes.is_empty())
            || matches!(end, Outcome::Failed(_))
    );
}

// Listening on every interface makes the Windows firewall ask the user about
// the test executable, so this one runs only when asked for.
#[test]
#[ignore = "listens on every interface"]
fn servers_without_a_host_take_ipv6_and_ipv4_connections() {
    let listener = server("", 0, 16, false).unwrap();
    let address = socket_address(listener.socket, false).unwrap();
    assert_eq!((address.address.as_str(), address.family), ("::", "IPv6"));
    for host in ["127.0.0.1", "::1"] {
        let port = address.port;
        let target = String::from(host);
        let client = std::thread::spawn(move || connect_to(&target, port).map(connection));
        let accepted = connection(done(accept_next(&listener)));
        assert!(client.join().unwrap().is_ok());
        let peer = socket_address(accepted.socket, true).unwrap();
        assert_eq!(peer.family, "IPv6");
    }
}

#[test]
fn failures_name_the_call_and_the_winsock_error() {
    let listener = server("127.0.0.1", 0, 16, false).unwrap();
    let port = port_of(&listener);
    // A second server on the same address.
    let Err(failure) = server("127.0.0.1", port, 16, false) else {
        panic!("the address is in use");
    };
    assert_eq!(
        failure,
        Failure {
            error: 10048,
            syscall: "listen",
        }
    );
    drop(listener);
    // Nothing listens there now.
    assert_eq!(
        connect_to("127.0.0.1", port).unwrap_err(),
        Failure {
            error: 10061,
            syscall: "connect",
        }
    );
    assert_eq!(
        connect_to("name.invalid", 80).unwrap_err().syscall,
        "getaddrinfo"
    );
}

#[test]
fn errors_read_as_node_reports_them() {
    let mut realm = Realm::new();
    let refused = Failure {
        error: 10061,
        syscall: "connect",
    };
    let error = socket_error(&mut realm, &refused, Some(("127.0.0.1", 9)), false);
    let text = |realm: &mut Realm, name: &'static str| {
        let value = ops::get_named(realm, &error, name).unwrap();
        ops::to_string(realm, &value).unwrap().to_rust_lossy()
    };
    assert_eq!(
        text(&mut realm, "message"),
        "connect ECONNREFUSED 127.0.0.1:9"
    );
    assert_eq!(text(&mut realm, "code"), "ECONNREFUSED");
    assert_eq!(text(&mut realm, "errno"), "-4078");
    assert_eq!(text(&mut realm, "address"), "127.0.0.1");
    assert_eq!(text(&mut realm, "port"), "9");
    let in_use = Failure {
        error: 10048,
        syscall: "listen",
    };
    let error = socket_error(&mut realm, &in_use, Some(("::", 80)), true);
    let message = ops::get_named(&mut realm, &error, "message").unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &message)
            .unwrap()
            .to_rust_lossy(),
        "listen EADDRINUSE: address already in use :::80"
    );
    let missing = Failure {
        error: 11001,
        syscall: "getaddrinfo",
    };
    let error = socket_error(&mut realm, &missing, Some(("name.invalid", 80)), false);
    let message = ops::get_named(&mut realm, &error, "message").unwrap();
    assert_eq!(
        ops::to_string(&mut realm, &message)
            .unwrap()
            .to_rust_lossy(),
        "getaddrinfo ENOTFOUND name.invalid"
    );
}
