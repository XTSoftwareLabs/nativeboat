//! libuv's errors on Windows, from the libuv in Node 22.15.0: each `UV_*`
//! code with the name and message `uv_err_name` and `uv_strerror` give,
//! and `uv_translate_sys_error` from Windows and Winsock errors. Generated
//! from libuv's `uv.h`, `uv/errno.h` and `src/win/error.c`.

#[allow(unused_imports)]
use crate::prelude::*;
use crate::{Key, Realm, Thrown, Value, ops};

/// `UV_UNKNOWN`.
pub(crate) const UNKNOWN: i32 = -4094;

/// libuv's codes, names and messages.
pub(crate) const ERRORS: &[(i32, &str, &str)] = &[
    (-4093, "E2BIG", "argument list too long"),
    (-4092, "EACCES", "permission denied"),
    (-4091, "EADDRINUSE", "address already in use"),
    (-4090, "EADDRNOTAVAIL", "address not available"),
    (-4089, "EAFNOSUPPORT", "address family not supported"),
    (-4088, "EAGAIN", "resource temporarily unavailable"),
    (-3000, "EAI_ADDRFAMILY", "address family not supported"),
    (-3001, "EAI_AGAIN", "temporary failure"),
    (-3002, "EAI_BADFLAGS", "bad ai_flags value"),
    (-3013, "EAI_BADHINTS", "invalid value for hints"),
    (-3003, "EAI_CANCELED", "request canceled"),
    (-3004, "EAI_FAIL", "permanent failure"),
    (-3005, "EAI_FAMILY", "ai_family not supported"),
    (-3006, "EAI_MEMORY", "out of memory"),
    (-3007, "EAI_NODATA", "no address"),
    (-3008, "EAI_NONAME", "unknown node or service"),
    (-3009, "EAI_OVERFLOW", "argument buffer overflow"),
    (-3014, "EAI_PROTOCOL", "resolved protocol is unknown"),
    (
        -3010,
        "EAI_SERVICE",
        "service not available for socket type",
    ),
    (-3011, "EAI_SOCKTYPE", "socket type not supported"),
    (-4084, "EALREADY", "connection already in progress"),
    (-4083, "EBADF", "bad file descriptor"),
    (-4082, "EBUSY", "resource busy or locked"),
    (-4081, "ECANCELED", "operation canceled"),
    (-4080, "ECHARSET", "invalid Unicode character"),
    (-4079, "ECONNABORTED", "software caused connection abort"),
    (-4078, "ECONNREFUSED", "connection refused"),
    (-4077, "ECONNRESET", "connection reset by peer"),
    (-4076, "EDESTADDRREQ", "destination address required"),
    (-4075, "EEXIST", "file already exists"),
    (-4074, "EFAULT", "bad address in system call argument"),
    (-4036, "EFBIG", "file too large"),
    (-4073, "EHOSTUNREACH", "host is unreachable"),
    (-4072, "EINTR", "interrupted system call"),
    (-4071, "EINVAL", "invalid argument"),
    (-4070, "EIO", "i/o error"),
    (-4069, "EISCONN", "socket is already connected"),
    (-4068, "EISDIR", "illegal operation on a directory"),
    (-4067, "ELOOP", "too many symbolic links encountered"),
    (-4066, "EMFILE", "too many open files"),
    (-4065, "EMSGSIZE", "message too long"),
    (-4064, "ENAMETOOLONG", "name too long"),
    (-4063, "ENETDOWN", "network is down"),
    (-4062, "ENETUNREACH", "network is unreachable"),
    (-4061, "ENFILE", "file table overflow"),
    (-4060, "ENOBUFS", "no buffer space available"),
    (-4059, "ENODEV", "no such device"),
    (-4058, "ENOENT", "no such file or directory"),
    (-4057, "ENOMEM", "not enough memory"),
    (-4056, "ENONET", "machine is not on the network"),
    (-4035, "ENOPROTOOPT", "protocol not available"),
    (-4055, "ENOSPC", "no space left on device"),
    (-4054, "ENOSYS", "function not implemented"),
    (-4053, "ENOTCONN", "socket is not connected"),
    (-4052, "ENOTDIR", "not a directory"),
    (-4051, "ENOTEMPTY", "directory not empty"),
    (-4050, "ENOTSOCK", "socket operation on non-socket"),
    (-4049, "ENOTSUP", "operation not supported on socket"),
    (-4026, "EOVERFLOW", "value too large for defined data type"),
    (-4048, "EPERM", "operation not permitted"),
    (-4047, "EPIPE", "broken pipe"),
    (-4046, "EPROTO", "protocol error"),
    (-4045, "EPROTONOSUPPORT", "protocol not supported"),
    (-4044, "EPROTOTYPE", "protocol wrong type for socket"),
    (-4034, "ERANGE", "result too large"),
    (-4043, "EROFS", "read-only file system"),
    (
        -4042,
        "ESHUTDOWN",
        "cannot send after transport endpoint shutdown",
    ),
    (-4041, "ESPIPE", "invalid seek"),
    (-4040, "ESRCH", "no such process"),
    (-4039, "ETIMEDOUT", "connection timed out"),
    (-4038, "ETXTBSY", "text file is busy"),
    (-4037, "EXDEV", "cross-device link not permitted"),
    (-4094, "UNKNOWN", "unknown error"),
    (-4095, "EOF", "end of file"),
    (-4033, "ENXIO", "no such device or address"),
    (-4032, "EMLINK", "too many links"),
    (-4031, "EHOSTDOWN", "host is down"),
    (-4030, "EREMOTEIO", "remote I/O error"),
    (-4029, "ENOTTY", "inappropriate ioctl for device"),
    (-4028, "EFTYPE", "inappropriate file type or format"),
    (-4027, "EILSEQ", "illegal byte sequence"),
    (-4025, "ESOCKTNOSUPPORT", "socket type not supported"),
    (-4024, "ENODATA", "no data available"),
    (-4023, "EUNATCH", "protocol driver not attached"),
];

/// Windows and Winsock errors with the libuv code each becomes.
const TRANSLATIONS: &[(u32, i32)] = &[
    (10013, -4092), // WSAEACCES
    (740, -4092),   // ERROR_ELEVATION_REQUIRED
    (1920, -4092),  // ERROR_CANT_ACCESS_FILE
    (1227, -4091),  // ERROR_ADDRESS_ALREADY_ASSOCIATED
    (10048, -4091), // WSAEADDRINUSE
    (10049, -4090), // WSAEADDRNOTAVAIL
    (10047, -4089), // WSAEAFNOSUPPORT
    (10035, -4088), // WSAEWOULDBLOCK
    (232, -4088),   // ERROR_NO_DATA
    (10037, -4084), // WSAEALREADY
    (1004, -4083),  // ERROR_INVALID_FLAGS
    (6, -4083),     // ERROR_INVALID_HANDLE
    (33, -4082),    // ERROR_LOCK_VIOLATION
    (231, -4082),   // ERROR_PIPE_BUSY
    (32, -4082),    // ERROR_SHARING_VIOLATION
    (995, -4081),   // ERROR_OPERATION_ABORTED
    (10004, -4081), // WSAEINTR
    (1113, -4080),  // ERROR_NO_UNICODE_TRANSLATION
    (1236, -4079),  // ERROR_CONNECTION_ABORTED
    (10053, -4079), // WSAECONNABORTED
    (1225, -4078),  // ERROR_CONNECTION_REFUSED
    (10061, -4078), // WSAECONNREFUSED
    (64, -4077),    // ERROR_NETNAME_DELETED
    (10054, -4077), // WSAECONNRESET
    (183, -4075),   // ERROR_ALREADY_EXISTS
    (80, -4075),    // ERROR_FILE_EXISTS
    (998, -4074),   // ERROR_NOACCESS
    (10014, -4074), // WSAEFAULT
    (1232, -4073),  // ERROR_HOST_UNREACHABLE
    (10065, -4073), // WSAEHOSTUNREACH
    (122, -4071),   // ERROR_INSUFFICIENT_BUFFER
    (13, -4071),    // ERROR_INVALID_DATA
    (87, -4071),    // ERROR_INVALID_PARAMETER
    (1464, -4071),  // ERROR_SYMLINK_NOT_SUPPORTED
    (10022, -4071), // WSAEINVAL
    (10046, -4071), // WSAEPFNOSUPPORT
    (1102, -4070),  // ERROR_BEGINNING_OF_MEDIA
    (1111, -4070),  // ERROR_BUS_RESET
    (23, -4070),    // ERROR_CRC
    (1166, -4070),  // ERROR_DEVICE_DOOR_OPEN
    (1165, -4070),  // ERROR_DEVICE_REQUIRES_CLEANING
    (1393, -4070),  // ERROR_DISK_CORRUPT
    (1129, -4070),  // ERROR_EOM_OVERFLOW
    (1101, -4070),  // ERROR_FILEMARK_DETECTED
    (31, -4070),    // ERROR_GEN_FAILURE
    (1106, -4070),  // ERROR_INVALID_BLOCK_LENGTH
    (1117, -4070),  // ERROR_IO_DEVICE
    (1104, -4070),  // ERROR_NO_DATA_DETECTED
    (205, -4070),   // ERROR_NO_SIGNAL_SENT
    (110, -4070),   // ERROR_OPEN_FAILED
    (1103, -4070),  // ERROR_SETMARK_DETECTED
    (156, -4070),   // ERROR_SIGNAL_REFUSED
    (10056, -4069), // WSAEISCONN
    (1921, -4067),  // ERROR_CANT_RESOLVE_FILENAME
    (4, -4066),     // ERROR_TOO_MANY_OPEN_FILES
    (10024, -4066), // WSAEMFILE
    (10040, -4065), // WSAEMSGSIZE
    (111, -4064),   // ERROR_BUFFER_OVERFLOW
    (206, -4064),   // ERROR_FILENAME_EXCED_RANGE
    (1231, -4062),  // ERROR_NETWORK_UNREACHABLE
    (10051, -4062), // WSAENETUNREACH
    (10055, -4060), // WSAENOBUFS
    (161, -4058),   // ERROR_BAD_PATHNAME
    (267, -4058),   // ERROR_DIRECTORY
    (203, -4058),   // ERROR_ENVVAR_NOT_FOUND
    (2, -4058),     // ERROR_FILE_NOT_FOUND
    (123, -4058),   // ERROR_INVALID_NAME
    (15, -4058),    // ERROR_INVALID_DRIVE
    (4392, -4058),  // ERROR_INVALID_REPARSE_DATA
    (126, -4058),   // ERROR_MOD_NOT_FOUND
    (3, -4058),     // ERROR_PATH_NOT_FOUND
    (11001, -4058), // WSAHOST_NOT_FOUND
    (11004, -4058), // WSANO_DATA
    (8, -4057),     // ERROR_NOT_ENOUGH_MEMORY
    (14, -4057),    // ERROR_OUTOFMEMORY
    (82, -4055),    // ERROR_CANNOT_MAKE
    (112, -4055),   // ERROR_DISK_FULL
    (277, -4055),   // ERROR_EA_TABLE_FULL
    (1100, -4055),  // ERROR_END_OF_MEDIA
    (39, -4055),    // ERROR_HANDLE_DISK_FULL
    (2250, -4053),  // ERROR_NOT_CONNECTED
    (10057, -4053), // WSAENOTCONN
    (145, -4051),   // ERROR_DIR_NOT_EMPTY
    (10038, -4050), // WSAENOTSOCK
    (50, -4049),    // ERROR_NOT_SUPPORTED
    (109, -4095),   // ERROR_BROKEN_PIPE
    (5, -4048),     // ERROR_ACCESS_DENIED
    (1314, -4048),  // ERROR_PRIVILEGE_NOT_HELD
    (230, -4047),   // ERROR_BAD_PIPE
    (233, -4047),   // ERROR_PIPE_NOT_CONNECTED
    (10058, -4047), // WSAESHUTDOWN
    (10043, -4045), // WSAEPROTONOSUPPORT
    (19, -4043),    // ERROR_WRITE_PROTECT
    (121, -4039),   // ERROR_SEM_TIMEOUT
    (10060, -4039), // WSAETIMEDOUT
    (17, -4037),    // ERROR_NOT_SAME_DEVICE
    (1, -4068),     // ERROR_INVALID_FUNCTION
    (208, -4093),   // ERROR_META_EXPANSION_TOO_LONG
    (10044, -4025), // WSAESOCKTNOSUPPORT
    (193, -4028),   // ERROR_BAD_EXE_FORMAT
];

/// `uv_translate_sys_error`: the libuv code for a Windows error.
pub(crate) fn translate(error: u32) -> i32 {
    TRANSLATIONS
        .iter()
        .find(|(known, _)| *known == error)
        .map_or(UNKNOWN, |(_, code)| *code)
}

/// A known libuv error-map row; lookup never coerces the key.
/// Unknown keys are absent, unlike `describe`'s diagnostic fallback.
pub(crate) fn mapped(value: f64) -> Option<(&'static str, &'static str)> {
    ERRORS
        .iter()
        .find(|(code, _, _)| f64::from(*code) == value)
        .map(|(_, name, message)| (*name, *message))
}

/// `uv_err_name` and `uv_strerror` for a libuv code.
pub(crate) fn describe(code: i32) -> (String, String) {
    match ERRORS.iter().find(|(known, _, _)| *known == code) {
        Some((_, name, message)) => ((*name).to_owned(), (*message).to_owned()),
        None => {
            let unknown = format!("Unknown system error {code}");
            (unknown.clone(), unknown)
        }
    }
}

/// Node's `CollectUVExceptionInfo`: records a failed libuv call on the
/// context object a binding was given, for the caller to throw.
pub(crate) fn collect(
    realm: &mut Realm,
    context: &Value,
    code: i32,
    syscall: &str,
) -> Result<(), Thrown> {
    let (name, message) = describe(code);
    for (key, value) in [
        ("errno", Value::Number(f64::from(code))),
        ("code", Value::str(&name)),
        ("message", Value::str(&message)),
        ("syscall", Value::str(syscall)),
    ] {
        ops::set(realm, context, &Key::name(key), value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
