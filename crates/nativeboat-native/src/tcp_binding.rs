//! TCP connections and servers for `net` on Winsock.
//!
//! Blocking steps run on the I/O pool and report on the realm thread. Reads
//! and writes are overlapped requests; a server waits for connections on a
//! network event. Every socket has a cancel event, so closing one ends the
//! waits of its pending requests at once and idle sockets cost no wakeups.
//! The socket itself is closed when the last of the handle and its pending
//! requests lets it go, so no request ever uses a reused socket value.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::ffi::c_void;

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

type Handle = *mut c_void;
type Socket = usize;

const INVALID_SOCKET: Socket = usize::MAX;
const AF_UNSPEC: i32 = 0;
const AF_INET: i32 = 2;
const AF_INET6: i32 = 23;
const SOCK_STREAM: i32 = 1;
const IPPROTO_TCP: i32 = 6;
const IPPROTO_IPV6: i32 = 41;
const SOL_SOCKET: i32 = 0xffff;
const SO_KEEPALIVE: i32 = 8;
const TCP_NODELAY: i32 = 1;
const IPV6_V6ONLY: i32 = 27;
const AI_PASSIVE: i32 = 1;
const WSA_FLAG_OVERLAPPED: u32 = 1;
const WSA_FLAG_NO_HANDLE_INHERIT: u32 = 0x80;
const WSA_IO_PENDING: i32 = 997;
const WSAEWOULDBLOCK: i32 = 10035;
const WSAECONNRESET: i32 = 10054;
const FD_ACCEPT: i32 = 8;
const FIONBIO: i32 = 0x8004_667e_u32 as i32;
const SIO_KEEPALIVE_VALS: u32 = 0x9800_0004;
const SD_SEND: i32 = 1;
const WAIT_OBJECT_0: u32 = 0;
const INFINITE: u32 = u32::MAX;
/// The most one read takes, as libuv's 64 KiB read buffer.
const READ_SIZE: usize = 64 * 1024;

#[repr(C)]
struct AddrInfo {
    flags: i32,
    family: i32,
    socktype: i32,
    protocol: i32,
    addrlen: usize,
    canonname: *mut u16,
    addr: *mut u8,
    next: *mut AddrInfo,
}

#[repr(C)]
struct WsaBuf {
    len: u32,
    buf: *mut u8,
}

#[repr(C)]
#[derive(Default)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: Handle,
}

#[repr(C)]
struct KeepAlive {
    on: u32,
    time: u32,
    interval: u32,
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut u8) -> i32;
    fn WSASocketW(
        family: i32,
        kind: i32,
        protocol: i32,
        info: *const c_void,
        group: u32,
        flags: u32,
    ) -> Socket;
    fn GetAddrInfoW(
        node: *const u16,
        service: *const u16,
        hints: *const AddrInfo,
        result: *mut *mut AddrInfo,
    ) -> i32;
    fn FreeAddrInfoW(info: *mut AddrInfo);
    fn connect(socket: Socket, address: *const u8, length: i32) -> i32;
    fn bind(socket: Socket, address: *const u8, length: i32) -> i32;
    fn listen(socket: Socket, backlog: i32) -> i32;
    fn accept(socket: Socket, address: *mut u8, length: *mut i32) -> Socket;
    fn closesocket(socket: Socket) -> i32;
    fn shutdown(socket: Socket, how: i32) -> i32;
    fn setsockopt(socket: Socket, level: i32, name: i32, value: *const u8, length: i32) -> i32;
    fn getsockname(socket: Socket, address: *mut u8, length: *mut i32) -> i32;
    fn getpeername(socket: Socket, address: *mut u8, length: *mut i32) -> i32;
    fn ioctlsocket(socket: Socket, command: i32, argument: *mut u32) -> i32;
    fn WSAIoctl(
        socket: Socket,
        code: u32,
        input: *const c_void,
        input_length: u32,
        output: *mut c_void,
        output_length: u32,
        returned: *mut u32,
        overlapped: *mut Overlapped,
        routine: *const c_void,
    ) -> i32;
    fn WSARecv(
        socket: Socket,
        buffers: *mut WsaBuf,
        count: u32,
        received: *mut u32,
        flags: *mut u32,
        overlapped: *mut Overlapped,
        routine: *const c_void,
    ) -> i32;
    fn WSASend(
        socket: Socket,
        buffers: *const WsaBuf,
        count: u32,
        sent: *mut u32,
        flags: u32,
        overlapped: *mut Overlapped,
        routine: *const c_void,
    ) -> i32;
    fn WSAGetOverlappedResult(
        socket: Socket,
        overlapped: *mut Overlapped,
        transferred: *mut u32,
        wait: i32,
        flags: *mut u32,
    ) -> i32;
    fn WSAEventSelect(socket: Socket, event: Handle, events: i32) -> i32;
    fn WSAGetLastError() -> i32;
    fn InetNtopW(family: i32, address: *const u8, buffer: *mut u16, size: usize) -> *const u16;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateEventW(security: *mut c_void, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn ResetEvent(event: Handle) -> i32;
    fn WaitForMultipleObjects(count: u32, handles: *const Handle, all: i32, timeout: u32) -> u32;
    fn WaitForSingleObject(handle: Handle, timeout: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
    fn CancelIoEx(handle: Handle, overlapped: *mut Overlapped) -> i32;
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

fn last_error() -> i32 {
    // SAFETY: reads this thread's latest Winsock error.
    unsafe { WSAGetLastError() }
}

fn startup() -> Result<(), i32> {
    static STARTED: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
    if STARTED.load(core::sync::atomic::Ordering::Acquire) {
        return Ok(());
    }
    let mut data = [0u8; 512];
    // SAFETY: the buffer is larger than WSADATA; startup is reference
    // counted, so a race that starts twice is harmless.
    let status = unsafe { WSAStartup(0x0202, data.as_mut_ptr()) };
    if status != 0 {
        return Err(status);
    }
    STARTED.store(true, core::sync::atomic::Ordering::Release);
    Ok(())
}

/// A manual-reset event, closed when dropped.
struct Event(Handle);

// SAFETY: event handles may be used from any thread.
unsafe impl Send for Event {}
// SAFETY: event operations are thread-safe.
unsafe impl Sync for Event {}

impl Event {
    fn new() -> Result<Self, i32> {
        // SAFETY: an unnamed manual-reset event without security attributes.
        let event = unsafe { CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null()) };
        if event.is_null() {
            return Err(last_error());
        }
        Ok(Self(event))
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the event is owned here.
        unsafe { CloseHandle(self.0) };
    }
}

/// A socket and its cancel event, shared by the handle and its pending
/// requests; the socket closes with the last of them.
struct Shared {
    socket: Socket,
    cancel: Event,
    /// The network event a server waits on for connections.
    accept: Option<Event>,
}

impl Shared {
    fn new(socket: Socket) -> Result<Arc<Self>, i32> {
        Ok(Arc::new(Self {
            socket,
            cancel: Event::new()?,
            accept: None,
        }))
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        // SAFETY: no request uses the socket any more.
        unsafe { closesocket(self.socket) };
    }
}

/// How a pending request ended.
enum Outcome<T> {
    Done(T),
    Failed(i32),
    /// The handle was closed while the request waited.
    Cancelled,
}

/// Runs an overlapped request on `shared`'s socket and waits for it or for
/// cancellation.
fn overlapped(shared: &Shared, start: impl FnOnce(*mut Overlapped) -> i32) -> Outcome<u32> {
    let event = match Event::new() {
        Ok(event) => event,
        Err(error) => return Outcome::Failed(error),
    };
    let mut request = Overlapped {
        event: event.0,
        ..Overlapped::default()
    };
    if start(&mut request) != 0 {
        let error = last_error();
        if error != WSA_IO_PENDING {
            return Outcome::Failed(error);
        }
    }
    let handles = [event.0, shared.cancel.0];
    // SAFETY: both events stay open for the wait.
    let woke = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
    if woke != WAIT_OBJECT_0 {
        // SAFETY: the request belongs to this socket; it must finish before
        // its OVERLAPPED and buffers are released.
        unsafe {
            CancelIoEx(shared.socket as Handle, &mut request);
            WaitForSingleObject(event.0, INFINITE);
        }
    }
    let mut transferred = 0;
    let mut flags = 0;
    // SAFETY: the request completed on this socket.
    let done = unsafe {
        WSAGetOverlappedResult(shared.socket, &mut request, &mut transferred, 0, &mut flags)
    };
    if woke != WAIT_OBJECT_0 {
        return Outcome::Cancelled;
    }
    if done == 0 {
        return Outcome::Failed(last_error());
    }
    Outcome::Done(transferred)
}

/// Reads what arrives next; empty at the end of the stream.
fn read(shared: &Shared) -> Outcome<Vec<u8>> {
    let mut buffer = alloc::vec![0u8; READ_SIZE];
    let mut slice = WsaBuf {
        len: buffer.len() as u32,
        buf: buffer.as_mut_ptr(),
    };
    let mut flags = 0;
    let mut received = 0;
    // SAFETY: the buffer outlives the request, which `overlapped` awaits.
    let outcome = overlapped(shared, |request| unsafe {
        WSARecv(
            shared.socket,
            &mut slice,
            1,
            &mut received,
            &mut flags,
            request,
            core::ptr::null(),
        )
    });
    match outcome {
        Outcome::Done(count) => {
            buffer.truncate(count as usize);
            Outcome::Done(buffer)
        }
        Outcome::Failed(error) => Outcome::Failed(error),
        Outcome::Cancelled => Outcome::Cancelled,
    }
}

fn write(shared: &Shared, bytes: &[u8]) -> Outcome<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        let slice = WsaBuf {
            len: (bytes.len() - offset).min(u32::MAX as usize) as u32,
            buf: bytes[offset..].as_ptr().cast_mut(),
        };
        let mut sent = 0;
        // SAFETY: the bytes outlive the request, which `overlapped` awaits.
        let outcome = overlapped(shared, |request| unsafe {
            WSASend(
                shared.socket,
                &slice,
                1,
                &mut sent,
                0,
                request,
                core::ptr::null(),
            )
        });
        match outcome {
            Outcome::Done(count) => offset += count as usize,
            Outcome::Failed(error) => return Outcome::Failed(error),
            Outcome::Cancelled => return Outcome::Cancelled,
        }
    }
    Outcome::Done(())
}

/// A socket address as Node reports it.
#[derive(Clone, Debug, PartialEq)]
struct Address {
    address: String,
    port: u16,
    family: &'static str,
}

fn parse_address(storage: &[u8]) -> Option<Address> {
    let family = i32::from(u16::from_le_bytes([storage[0], storage[1]]));
    let port = u16::from_be_bytes([storage[2], storage[3]]);
    let (raw, name): (&[u8], _) = match family {
        AF_INET => (&storage[4..8], "IPv4"),
        AF_INET6 => (&storage[8..24], "IPv6"),
        _ => return None,
    };
    let mut text = [0u16; 64];
    // SAFETY: the address bytes match the family and the buffer is writable.
    let written = unsafe { InetNtopW(family, raw.as_ptr(), text.as_mut_ptr(), text.len()) };
    if written.is_null() {
        return None;
    }
    let length = text.iter().position(|unit| *unit == 0).unwrap_or(0);
    Some(Address {
        address: String::from_utf16_lossy(&text[..length]),
        port,
        family: name,
    })
}

fn socket_address(socket: Socket, peer: bool) -> Option<Address> {
    let mut storage = [0u8; 128];
    let mut length = storage.len() as i32;
    // SAFETY: the storage is a writable SOCKADDR_STORAGE.
    let status = unsafe {
        if peer {
            getpeername(socket, storage.as_mut_ptr(), &mut length)
        } else {
            getsockname(socket, storage.as_mut_ptr(), &mut length)
        }
    };
    (status == 0).then(|| parse_address(&storage)).flatten()
}

/// The addresses a host name and port resolve to, in the resolver's order.
fn resolve(host: &str, port: u16, passive: bool) -> Result<Vec<Vec<u8>>, i32> {
    startup()?;
    let hints = AddrInfo {
        flags: if passive { AI_PASSIVE } else { 0 },
        family: AF_UNSPEC,
        socktype: SOCK_STREAM,
        protocol: IPPROTO_TCP,
        addrlen: 0,
        canonname: core::ptr::null_mut(),
        addr: core::ptr::null_mut(),
        next: core::ptr::null_mut(),
    };
    let node = wide(host);
    let service = wide(&alloc::format!("{port}"));
    let mut list = core::ptr::null_mut();
    // SAFETY: the strings are NUL-terminated; the list is freed below.
    let status = unsafe { GetAddrInfoW(node.as_ptr(), service.as_ptr(), &hints, &mut list) };
    if status != 0 {
        return Err(status);
    }
    let mut addresses = Vec::new();
    let mut entry = list;
    while !entry.is_null() {
        // SAFETY: the resolver returned a linked list of valid entries.
        let info = unsafe { &*entry };
        if !info.addr.is_null() {
            // SAFETY: `addr` holds `addrlen` bytes.
            addresses
                .push(unsafe { core::slice::from_raw_parts(info.addr, info.addrlen) }.to_vec());
        }
        entry = info.next;
    }
    // SAFETY: the list came from GetAddrInfoW.
    unsafe { FreeAddrInfoW(list) };
    Ok(addresses)
}

fn new_socket(family: i32) -> Result<Socket, i32> {
    // SAFETY: a plain overlapped TCP socket that child processes do not
    // inherit.
    let socket = unsafe {
        WSASocketW(
            family,
            SOCK_STREAM,
            IPPROTO_TCP,
            core::ptr::null(),
            0,
            WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT,
        )
    };
    if socket == INVALID_SOCKET {
        return Err(last_error());
    }
    Ok(socket)
}

fn family_of(address: &[u8]) -> i32 {
    i32::from(u16::from_le_bytes([address[0], address[1]]))
}

/// A failed step: the Winsock error and the call Node names.
#[derive(Debug, PartialEq)]
struct Failure {
    error: i32,
    syscall: &'static str,
}

/// Connects to the first address of `host` that accepts, as Node tries a
/// host's addresses in turn.
fn connect_to(host: &str, port: u16) -> Result<Socket, Failure> {
    let addresses = resolve(host, port, false).map_err(|error| Failure {
        error,
        syscall: "getaddrinfo",
    })?;
    let mut failure = Failure {
        error: WSAECONNRESET,
        syscall: "connect",
    };
    for address in addresses {
        let socket = new_socket(family_of(&address)).map_err(|error| Failure {
            error,
            syscall: "socket",
        })?;
        // SAFETY: the address bytes are a SOCKADDR of their stated length.
        if unsafe { connect(socket, address.as_ptr(), address.len() as i32) } == 0 {
            return Ok(socket);
        }
        failure.error = last_error();
        // SAFETY: the socket is owned here.
        unsafe { closesocket(socket) };
    }
    Err(failure)
}

/// Binds and listens. Without a host, Node listens on every IPv6 and IPv4
/// address.
fn listen_on(host: &str, port: u16, backlog: i32, ipv6_only: bool) -> Result<Socket, Failure> {
    let fail = |syscall| move |error| Failure { error, syscall };
    let addresses = if host.is_empty() {
        let mut any6 = alloc::vec![0u8; 28];
        any6[..2].copy_from_slice(&(AF_INET6 as u16).to_le_bytes());
        any6[2..4].copy_from_slice(&port.to_be_bytes());
        let mut any4 = alloc::vec![0u8; 16];
        any4[..2].copy_from_slice(&(AF_INET as u16).to_le_bytes());
        any4[2..4].copy_from_slice(&port.to_be_bytes());
        startup().map_err(fail("listen"))?;
        alloc::vec![any6, any4]
    } else {
        resolve(host, port, true).map_err(fail("getaddrinfo"))?
    };
    let mut last = Failure {
        error: WSAECONNRESET,
        syscall: "listen",
    };
    // Without a host, the IPv4 address is only a fallback.
    let count = if host.is_empty() { 2 } else { 1 };
    for address in addresses.into_iter().take(count) {
        let family = family_of(&address);
        let socket = match new_socket(family) {
            Ok(socket) => socket,
            Err(error) => {
                last = Failure {
                    error,
                    syscall: "listen",
                };
                continue;
            }
        };
        if family == AF_INET6 {
            let only = u32::from(ipv6_only);
            // SAFETY: the option value is a DWORD.
            unsafe {
                setsockopt(
                    socket,
                    IPPROTO_IPV6,
                    IPV6_V6ONLY,
                    (&raw const only).cast(),
                    4,
                );
            }
        }
        // SAFETY: the address bytes are a SOCKADDR of their stated length.
        let bound = unsafe { bind(socket, address.as_ptr(), address.len() as i32) } == 0
            // SAFETY: the socket is bound.
            && unsafe { listen(socket, backlog) } == 0;
        if bound {
            return Ok(socket);
        }
        last = Failure {
            error: last_error(),
            syscall: "listen",
        };
        // SAFETY: the socket is owned here.
        unsafe { closesocket(socket) };
    }
    Err(last)
}

/// A listening socket whose connections are announced on its accept event.
fn server(host: &str, port: u16, backlog: i32, ipv6_only: bool) -> Result<Arc<Shared>, Failure> {
    let socket = listen_on(host, port, backlog, ipv6_only)?;
    let fail = |error| {
        // SAFETY: the socket is owned here.
        unsafe { closesocket(socket) };
        Failure {
            error,
            syscall: "listen",
        }
    };
    let ready = Event::new().map_err(fail)?;
    // SAFETY: both the socket and the event are open.
    if unsafe { WSAEventSelect(socket, ready.0, FD_ACCEPT) } != 0 {
        return Err(fail(last_error()));
    }
    let cancel = Event::new().map_err(fail)?;
    Ok(Arc::new(Shared {
        socket,
        cancel,
        accept: Some(ready),
    }))
}

/// Waits for the next connection to a server.
fn accept_next(shared: &Shared) -> Outcome<Socket> {
    let Some(ready) = &shared.accept else {
        return Outcome::Failed(WSAEWOULDBLOCK);
    };
    loop {
        // SAFETY: the event is open; a connection arriving after the reset
        // signals it again, since `accept` re-enables the notification.
        unsafe { ResetEvent(ready.0) };
        // SAFETY: the listening socket is open while `shared` lives.
        let socket = unsafe { accept(shared.socket, core::ptr::null_mut(), core::ptr::null_mut()) };
        if socket != INVALID_SOCKET {
            // A connection inherits the server's event selection and
            // non-blocking mode; its own requests are overlapped.
            let mut blocking = 0;
            // SAFETY: the new socket is owned here.
            unsafe {
                WSAEventSelect(socket, core::ptr::null_mut(), 0);
                ioctlsocket(socket, FIONBIO, &mut blocking);
            }
            return Outcome::Done(socket);
        }
        let error = last_error();
        if error != WSAEWOULDBLOCK {
            return Outcome::Failed(error);
        }
        let handles = [ready.0, shared.cancel.0];
        // SAFETY: both events stay open for the wait.
        if unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) } != WAIT_OBJECT_0 {
            return Outcome::Cancelled;
        }
    }
}

/// The realm-side state of a socket or server handle.
struct HandleState {
    shared: Option<Arc<Shared>>,
    referenced: Rc<Cell<bool>>,
    reading: bool,
}

type State = Rc<RefCell<HandleState>>;

/// An Error as Node's `exceptionWithHostPort` and `errnoException` make
/// them: `syscall CODE` and the address, with `errno`, `code` and `syscall`
/// properties; `uv_message` adds libuv's description as `listen` errors do.
fn socket_error(
    realm: &mut Realm,
    failure: &Failure,
    details: Option<(&str, u16)>,
    uv_message: bool,
) -> Value {
    let errno = if failure.syscall == "getaddrinfo" {
        // Node reports names that do not resolve as ENOTFOUND.
        None
    } else {
        Some(crate::uv_error::translate(failure.error as u32))
    };
    let (code, description) = match errno {
        Some(errno) => crate::uv_error::describe(errno),
        None => ("ENOTFOUND".into(), String::new()),
    };
    let mut message = alloc::format!("{} {code}", failure.syscall);
    if uv_message && !description.is_empty() {
        message.push_str(": ");
        message.push_str(&description);
    }
    match (failure.syscall, details) {
        ("getaddrinfo", Some((host, _))) => {
            message.push(' ');
            message.push_str(host);
        }
        (_, Some((address, port))) => {
            message.push_str(&alloc::format!(" {address}:{port}"));
        }
        _ => {}
    }
    let error = realm.error(ErrorKind::Error, &message);
    let errno = errno.map_or(Value::str("ENOTFOUND"), |errno| {
        Value::Number(f64::from(errno))
    });
    error.define(Key::name("errno"), Property::data(errno));
    error.define(Key::name("code"), Property::data(Value::str(&code)));
    error.define(
        Key::name("syscall"),
        Property::data(Value::str(failure.syscall)),
    );
    match (failure.syscall, details) {
        ("getaddrinfo", Some((host, _))) => {
            error.define(Key::name("hostname"), Property::data(Value::str(host)));
        }
        (_, Some((address, port))) => {
            error.define(Key::name("address"), Property::data(Value::str(address)));
            error.define(
                Key::name("port"),
                Property::data(Value::Number(f64::from(port))),
            );
        }
        _ => {}
    }
    Value::Object(error)
}

fn address_object(realm: &Realm, address: Option<Address>) -> Value {
    let Some(address) = address else {
        return Value::Undefined;
    };
    let object = realm.object();
    object.define(
        Key::name("address"),
        Property::data(Value::str(&address.address)),
    );
    object.define(
        Key::name("family"),
        Property::data(Value::str(address.family)),
    );
    object.define(
        Key::name("port"),
        Property::data(Value::Number(f64::from(address.port))),
    );
    Value::Object(object)
}

fn text_arg(realm: &mut Realm, args: &[Value], index: usize) -> Result<String, Thrown> {
    match args.get(index) {
        None | Some(Value::Undefined | Value::Null) => Ok(String::new()),
        Some(value) => Ok(ops::to_string(realm, value)?.to_rust_lossy()),
    }
}

fn port_arg(args: &[Value], index: usize) -> u16 {
    match args.get(index) {
        Some(Value::Number(port)) if (0.0..=65535.0).contains(port) => *port as u16,
        _ => 0,
    }
}

fn callback_arg(realm: &Realm, args: &[Value], index: usize) -> Result<Value, Thrown> {
    let callback = args.get(index).cloned().unwrap_or(Value::Undefined);
    if !crate::array_builtin::is_callable(&callback) {
        return Err(realm.throw(ErrorKind::Type, "callback must be a function"));
    }
    Ok(callback)
}

fn define_function(
    realm: &Realm,
    object: &Object,
    name: &str,
    length: u32,
    body: impl Fn(&mut Realm, &[Value]) -> Result<Value, Thrown> + 'static,
) {
    let function = realm.function(name, length, false, move |realm, _, args| body(realm, args));
    object.define(Key::name(name), Property::data(Value::Object(function)));
}

/// The methods shared by connections and servers.
fn handle_object(realm: &Realm, state: &State) -> Object {
    let handle = realm.object();
    let closing = state.clone();
    define_function(realm, &handle, "close", 0, move |_, _| {
        if let Some(shared) = closing.borrow_mut().shared.take() {
            // SAFETY: the event is open while `shared` lives.
            unsafe { SetEvent(shared.cancel.0) };
        }
        Ok(Value::Undefined)
    });
    for (name, value) in [("ref", true), ("unref", false)] {
        let state = state.clone();
        define_function(realm, &handle, name, 0, move |_, _| {
            state.borrow().referenced.set(value);
            Ok(Value::Undefined)
        });
    }
    let state = state.clone();
    define_function(realm, &handle, "hasRef", 0, move |_, _| {
        Ok(Value::Boolean(state.borrow().referenced.get()))
    });
    handle
}

fn new_state(shared: Arc<Shared>) -> State {
    Rc::new(RefCell::new(HandleState {
        shared: Some(shared),
        referenced: Rc::new(Cell::new(true)),
        reading: false,
    }))
}

fn queue_error(realm: &Realm) -> Thrown {
    realm.throw(ErrorKind::Error, "could not queue a socket request")
}

/// A connection's handle: `readStart(onread)` and `readStop()`, where
/// `onread(error, chunk)` receives `null` at the end of the stream;
/// `write(bytes, callback)`, `shutdown()`, `setNoDelay(on)`,
/// `setKeepAlive(on, delay)`, `localAddress()` and `remoteAddress()`.
fn connection_object(realm: &Realm, shared: Arc<Shared>) -> Object {
    let state = new_state(shared);
    let handle = handle_object(realm, &state);
    let reader = state.clone();
    define_function(realm, &handle, "readStart", 1, move |realm, args| {
        let callback = callback_arg(realm, args, 0)?;
        if reader.borrow().reading {
            return Ok(Value::Undefined);
        }
        reader.borrow_mut().reading = true;
        queue_read(realm, reader.clone(), callback)?;
        Ok(Value::Undefined)
    });
    let stopper = state.clone();
    define_function(realm, &handle, "readStop", 0, move |_, _| {
        stopper.borrow_mut().reading = false;
        Ok(Value::Undefined)
    });
    let writer = state.clone();
    define_function(realm, &handle, "write", 2, move |realm, args| {
        let callback = callback_arg(realm, args, 1)?;
        let Some(bytes) = args
            .first()
            .and_then(crate::typed_array_builtin::view_bytes)
        else {
            return Err(realm.throw(ErrorKind::Type, "data must be a Buffer"));
        };
        let Some(shared) = writer.borrow().shared.clone() else {
            let error = socket_error(
                realm,
                &Failure {
                    error: 10038,
                    syscall: "write",
                },
                None,
                false,
            );
            ops::call(realm, &callback, Value::Undefined, &[error])?;
            return Ok(Value::Undefined);
        };
        let referenced = writer.borrow().referenced.clone();
        realm
            .submit_io_with_ref(
                move || write(&shared, &bytes),
                referenced,
                move |realm, outcome| {
                    let error = match outcome {
                        Outcome::Done(()) | Outcome::Cancelled => Value::Null,
                        Outcome::Failed(error) => socket_error(
                            realm,
                            &Failure {
                                error,
                                syscall: "write",
                            },
                            None,
                            false,
                        ),
                    };
                    ops::call(realm, &callback, Value::Undefined, &[error])?;
                    Ok(())
                },
            )
            .map_err(|_| queue_error(realm))?;
        Ok(Value::Undefined)
    });
    let shutter = state.clone();
    define_function(realm, &handle, "shutdown", 0, move |_, _| {
        if let Some(shared) = &shutter.borrow().shared {
            // SAFETY: the socket is open while `shared` lives.
            unsafe { shutdown(shared.socket, SD_SEND) };
        }
        Ok(Value::Undefined)
    });
    let options = state.clone();
    define_function(realm, &handle, "setNoDelay", 1, move |_, args| {
        if let Some(shared) = &options.borrow().shared {
            let on = u32::from(args.first().is_none_or(ops::to_boolean));
            // SAFETY: the option value is a DWORD.
            unsafe {
                setsockopt(
                    shared.socket,
                    IPPROTO_TCP,
                    TCP_NODELAY,
                    (&raw const on).cast(),
                    4,
                )
            };
        }
        Ok(Value::Undefined)
    });
    let options = state.clone();
    define_function(realm, &handle, "setKeepAlive", 2, move |_, args| {
        if let Some(shared) = &options.borrow().shared {
            let on = args.first().is_some_and(ops::to_boolean);
            let delay = match args.get(1) {
                Some(Value::Number(delay)) if *delay > 0.0 => *delay as u32,
                _ => 0,
            };
            let flag = u32::from(on);
            // SAFETY: the option value is a DWORD.
            unsafe {
                setsockopt(
                    shared.socket,
                    SOL_SOCKET,
                    SO_KEEPALIVE,
                    (&raw const flag).cast(),
                    4,
                )
            };
            if on && delay > 0 {
                // libuv probes once a second after the delay.
                let values = KeepAlive {
                    on: 1,
                    time: delay,
                    interval: 1000,
                };
                let mut returned = 0;
                // SAFETY: the input is a `tcp_keepalive` structure.
                unsafe {
                    WSAIoctl(
                        shared.socket,
                        SIO_KEEPALIVE_VALS,
                        (&raw const values).cast(),
                        core::mem::size_of::<KeepAlive>() as u32,
                        core::ptr::null_mut(),
                        0,
                        &mut returned,
                        core::ptr::null_mut(),
                        core::ptr::null(),
                    )
                };
            }
        }
        Ok(Value::Undefined)
    });
    for (name, peer) in [("localAddress", false), ("remoteAddress", true)] {
        let state = state.clone();
        define_function(realm, &handle, name, 0, move |realm, _| {
            let address = state
                .borrow()
                .shared
                .as_ref()
                .and_then(|shared| socket_address(shared.socket, peer));
            Ok(address_object(realm, address))
        });
    }
    handle
}

fn queue_read(realm: &mut Realm, state: State, callback: Value) -> Result<(), Thrown> {
    let Some(shared) = state.borrow().shared.clone() else {
        state.borrow_mut().reading = false;
        return Ok(());
    };
    let referenced = state.borrow().referenced.clone();
    realm
        .submit_io_with_ref(
            move || read(&shared),
            referenced,
            move |realm, outcome| {
                let (error, chunk) = match outcome {
                    Outcome::Cancelled => return Ok(()),
                    Outcome::Done(bytes) if bytes.is_empty() => (Value::Null, Value::Null),
                    Outcome::Done(bytes) => (
                        Value::Null,
                        Value::Object(crate::typed_array_builtin::new_bytes(realm, bytes, None)),
                    ),
                    Outcome::Failed(error) => (
                        socket_error(
                            realm,
                            &Failure {
                                error,
                                syscall: "read",
                            },
                            None,
                            false,
                        ),
                        Value::Null,
                    ),
                };
                let more = matches!(chunk, Value::Object(_));
                if !more {
                    state.borrow_mut().reading = false;
                }
                ops::call(realm, &callback, Value::Undefined, &[error, chunk])?;
                // The callback may have stopped reading or closed the handle.
                if more && state.borrow().reading {
                    queue_read(realm, state, callback)?;
                }
                Ok(())
            },
        )
        .map_err(|_| queue_error(realm))
}

/// A server's handle: `accept(callback)` waits for one connection and calls
/// `callback(error, connection)`; `address()` gives the bound address.
fn server_object(realm: &Realm, shared: Arc<Shared>) -> Object {
    let state = new_state(shared);
    let handle = handle_object(realm, &state);
    let acceptor = state.clone();
    define_function(realm, &handle, "accept", 1, move |realm, args| {
        let callback = callback_arg(realm, args, 0)?;
        let Some(shared) = acceptor.borrow().shared.clone() else {
            return Ok(Value::Undefined);
        };
        let referenced = acceptor.borrow().referenced.clone();
        realm
            .submit_io_with_ref(
                move || accept_next(&shared),
                referenced,
                move |realm, outcome| {
                    let (error, connection) = match outcome {
                        Outcome::Cancelled => return Ok(()),
                        Outcome::Done(socket) => match Shared::new(socket) {
                            Ok(shared) => {
                                (Value::Null, Value::Object(connection_object(realm, shared)))
                            }
                            Err(error) => (
                                socket_error(
                                    realm,
                                    &Failure {
                                        error,
                                        syscall: "accept",
                                    },
                                    None,
                                    false,
                                ),
                                Value::Null,
                            ),
                        },
                        Outcome::Failed(error) => (
                            socket_error(
                                realm,
                                &Failure {
                                    error,
                                    syscall: "accept",
                                },
                                None,
                                false,
                            ),
                            Value::Null,
                        ),
                    };
                    ops::call(realm, &callback, Value::Undefined, &[error, connection])?;
                    Ok(())
                },
            )
            .map_err(|_| queue_error(realm))?;
        Ok(Value::Undefined)
    });
    define_function(realm, &handle, "address", 0, move |realm, _| {
        let address = state
            .borrow()
            .shared
            .as_ref()
            .and_then(|shared| socket_address(shared.socket, false));
        Ok(address_object(realm, address))
    });
    handle
}

pub(crate) fn binding(realm: &mut Realm) -> Object {
    let exports = realm.object();
    // connect(host, port, callback(error, connection)).
    define_function(realm, &exports, "connect", 3, |realm, args| {
        let host = text_arg(realm, args, 0)?;
        let host = if host.is_empty() {
            String::from("localhost")
        } else {
            host
        };
        let port = port_arg(args, 1);
        let callback = callback_arg(realm, args, 2)?;
        let target = host.clone();
        realm
            .submit_io(
                move || connect_to(&target, port),
                move |realm, result| {
                    let (error, connection) = match result.and_then(|socket| {
                        Shared::new(socket).map_err(|error| Failure {
                            error,
                            syscall: "connect",
                        })
                    }) {
                        Ok(shared) => {
                            (Value::Null, Value::Object(connection_object(realm, shared)))
                        }
                        Err(failure) => (
                            socket_error(realm, &failure, Some((&host, port)), false),
                            Value::Null,
                        ),
                    };
                    ops::call(realm, &callback, Value::Undefined, &[error, connection])?;
                    Ok(())
                },
            )
            .map_err(|_| queue_error(realm))?;
        Ok(Value::Undefined)
    });
    // lookup(host, callback(error, [[address, family], ...])): the addresses
    // a host name resolves to, in the resolver's order, as `dns.lookup`
    // reads them; names that do not resolve fail with ENOTFOUND.
    define_function(realm, &exports, "lookup", 2, |realm, args| {
        let host = text_arg(realm, args, 0)?;
        let callback = callback_arg(realm, args, 1)?;
        let target = host.clone();
        realm
            .submit_io(
                move || {
                    resolve(&target, 0, false).map_err(|error| Failure {
                        error,
                        syscall: "getaddrinfo",
                    })
                },
                move |realm, result| {
                    let (error, list) = match result {
                        Ok(addresses) => {
                            let entries: Vec<Value> = addresses
                                .iter()
                                .filter_map(|storage| parse_address(storage))
                                .map(|address| {
                                    let family = if address.family == "IPv6" { 6.0 } else { 4.0 };
                                    Value::Object(realm.array(alloc::vec![
                                        Value::str(&address.address),
                                        Value::Number(family),
                                    ]))
                                })
                                .collect();
                            (Value::Null, Value::Object(realm.array(entries)))
                        }
                        Err(failure) => (
                            socket_error(realm, &failure, Some((&host, 0)), false),
                            Value::Null,
                        ),
                    };
                    ops::call(realm, &callback, Value::Undefined, &[error, list])?;
                    Ok(())
                },
            )
            .map_err(|_| queue_error(realm))?;
        Ok(Value::Undefined)
    });
    // listen(host, port, backlog, ipv6Only): a server, or a thrown error.
    define_function(realm, &exports, "listen", 4, |realm, args| {
        let host = text_arg(realm, args, 0)?;
        let port = port_arg(args, 1);
        let backlog = match args.get(2) {
            Some(Value::Number(backlog)) if *backlog >= 0.0 => *backlog as i32,
            _ => 511,
        };
        let ipv6_only = args.get(3).is_some_and(ops::to_boolean);
        match server(&host, port, backlog, ipv6_only) {
            Ok(shared) => Ok(Value::Object(server_object(realm, shared))),
            Err(failure) => {
                let address = if host.is_empty() { "::" } else { host.as_str() };
                let error = socket_error(realm, &failure, Some((address, port)), true);
                Err(Thrown(error))
            }
        }
    });
    exports
}

#[cfg(test)]
mod tests;
