//! Node's error codes whose messages are templates, as Node 22's
//! `internal/errors` defines them with `E()`. The codes the runtime has no
//! dedicated constructor for take their classes and messages from here, so
//! library code never meets a code that is missing.

use crate::prelude::*;
use crate::{ErrorKind, Key, Object, Property, Realm, Thrown, Value, ops};

/// Code, classes (the first is the constructor's, the others are its
/// properties) and message template.
const TEMPLATES: &[(&str, &[ErrorKind], &str)] = &[
    (
        "ERR_AMBIGUOUS_ARGUMENT",
        &[ErrorKind::Type],
        "The \"%s\" argument is ambiguous. %s",
    ),
    (
        "ERR_ARG_NOT_ITERABLE",
        &[ErrorKind::Type],
        "%s must be iterable",
    ),
    ("ERR_ASSERTION", &[ErrorKind::Error], "%s"),
    (
        "ERR_ASYNC_CALLBACK",
        &[ErrorKind::Type],
        "%s must be a function",
    ),
    (
        "ERR_ASYNC_TYPE",
        &[ErrorKind::Type],
        "Invalid name for async \"type\": %s",
    ),
    (
        "ERR_BROTLI_INVALID_PARAM",
        &[ErrorKind::Range],
        "%s is not a valid Brotli parameter",
    ),
    (
        "ERR_BUFFER_TOO_LARGE",
        &[ErrorKind::Range],
        "Cannot create a Buffer larger than %s bytes",
    ),
    (
        "ERR_CANNOT_WATCH_SIGINT",
        &[ErrorKind::Error],
        "Cannot watch for SIGINT signals",
    ),
    (
        "ERR_CHILD_CLOSED_BEFORE_REPLY",
        &[ErrorKind::Error],
        "Child closed before reply received",
    ),
    (
        "ERR_CHILD_PROCESS_STDIO_MAXBUFFER",
        &[ErrorKind::Range],
        "%s maxBuffer length exceeded",
    ),
    (
        "ERR_CONSOLE_WRITABLE_STREAM",
        &[ErrorKind::Type],
        "Console expects a writable stream instance for %s",
    ),
    (
        "ERR_CONTEXT_NOT_INITIALIZED",
        &[ErrorKind::Error],
        "context used is not initialized",
    ),
    (
        "ERR_CRYPTO_CUSTOM_ENGINE_NOT_SUPPORTED",
        &[ErrorKind::Error],
        "Custom engines not supported by this OpenSSL",
    ),
    (
        "ERR_CRYPTO_ECDH_INVALID_FORMAT",
        &[ErrorKind::Type],
        "Invalid ECDH format: %s",
    ),
    (
        "ERR_CRYPTO_ECDH_INVALID_PUBLIC_KEY",
        &[ErrorKind::Error],
        "Public key is not valid for specified curve",
    ),
    (
        "ERR_CRYPTO_ENGINE_UNKNOWN",
        &[ErrorKind::Error],
        "Engine \"%s\" was not found",
    ),
    (
        "ERR_CRYPTO_FIPS_FORCED",
        &[ErrorKind::Error],
        "Cannot set FIPS mode, it was forced with --force-fips at startup.",
    ),
    (
        "ERR_CRYPTO_FIPS_UNAVAILABLE",
        &[ErrorKind::Error],
        "Cannot set FIPS mode in a non-FIPS build.",
    ),
    (
        "ERR_CRYPTO_HASH_FINALIZED",
        &[ErrorKind::Error],
        "Digest already called",
    ),
    (
        "ERR_CRYPTO_HASH_UPDATE_FAILED",
        &[ErrorKind::Error],
        "Hash update failed",
    ),
    (
        "ERR_CRYPTO_INCOMPATIBLE_KEY",
        &[ErrorKind::Error],
        "Incompatible %s: %s",
    ),
    (
        "ERR_CRYPTO_INCOMPATIBLE_KEY_OPTIONS",
        &[ErrorKind::Error],
        "The selected key encoding %s %s.",
    ),
    (
        "ERR_CRYPTO_INVALID_DIGEST",
        &[ErrorKind::Type],
        "Invalid digest: %s",
    ),
    (
        "ERR_CRYPTO_INVALID_JWK",
        &[ErrorKind::Type],
        "Invalid JWK data",
    ),
    (
        "ERR_CRYPTO_INVALID_KEY_OBJECT_TYPE",
        &[ErrorKind::Type],
        "Invalid key object type %s, expected %s.",
    ),
    (
        "ERR_CRYPTO_INVALID_STATE",
        &[ErrorKind::Error],
        "Invalid state for operation %s",
    ),
    (
        "ERR_CRYPTO_PBKDF2_ERROR",
        &[ErrorKind::Error],
        "PBKDF2 error",
    ),
    (
        "ERR_CRYPTO_SCRYPT_INVALID_PARAMETER",
        &[ErrorKind::Error],
        "Invalid scrypt parameter",
    ),
    (
        "ERR_CRYPTO_SCRYPT_NOT_SUPPORTED",
        &[ErrorKind::Error],
        "Scrypt algorithm not supported",
    ),
    (
        "ERR_CRYPTO_SIGN_KEY_REQUIRED",
        &[ErrorKind::Error],
        "No key provided to sign",
    ),
    ("ERR_DEBUGGER_ERROR", &[ErrorKind::Error], "%s"),
    ("ERR_DEBUGGER_STARTUP_ERROR", &[ErrorKind::Error], "%s"),
    (
        "ERR_DIR_CLOSED",
        &[ErrorKind::Error],
        "Directory handle was closed",
    ),
    (
        "ERR_DIR_CONCURRENT_OPERATION",
        &[ErrorKind::Error],
        "Cannot do synchronous work on directory handle with concurrent asynchronous operations",
    ),
    (
        "ERR_DNS_SET_SERVERS_FAILED",
        &[ErrorKind::Error],
        "c-ares failed to set servers: \"%s\" [%s]",
    ),
    (
        "ERR_DOMAIN_CALLBACK_NOT_AVAILABLE",
        &[ErrorKind::Error],
        "A callback was registered through process.setUncaughtExceptionCaptureCallback(), which is mutually exclusive with using the `domain` module",
    ),
    (
        "ERR_DOMAIN_CANNOT_SET_UNCAUGHT_EXCEPTION_CAPTURE",
        &[ErrorKind::Error],
        "The `domain` module is in use, which is mutually exclusive with calling process.setUncaughtExceptionCaptureCallback()",
    ),
    (
        "ERR_DUPLICATE_STARTUP_SNAPSHOT_MAIN_FUNCTION",
        &[ErrorKind::Error],
        "Deserialize main function is already configured.",
    ),
    (
        "ERR_ENCODING_NOT_SUPPORTED",
        &[ErrorKind::Range],
        "The \"%s\" encoding is not supported",
    ),
    (
        "ERR_EVAL_ESM_CANNOT_PRINT",
        &[ErrorKind::Error],
        "--print cannot be used with ESM input",
    ),
    (
        "ERR_EVENT_RECURSION",
        &[ErrorKind::Error],
        "The event \"%s\" is already being dispatched",
    ),
    (
        "ERR_FEATURE_UNAVAILABLE_ON_PLATFORM",
        &[ErrorKind::Type],
        "The feature %s is unavailable on the current platform, which is being used to run Node.js",
    ),
    (
        "ERR_FS_FILE_TOO_LARGE",
        &[ErrorKind::Range],
        "File size (%s) is greater than 2 GiB",
    ),
    (
        "ERR_FS_INVALID_SYMLINK_TYPE",
        &[ErrorKind::Error],
        "Symlink type must be one of \"dir\", \"file\", or \"junction\". Received \"%s\"",
    ),
    (
        "ERR_HTTP2_ALTSVC_INVALID_ORIGIN",
        &[ErrorKind::Type],
        "HTTP/2 ALTSVC frames require a valid origin",
    ),
    (
        "ERR_HTTP2_ALTSVC_LENGTH",
        &[ErrorKind::Type],
        "HTTP/2 ALTSVC frames are limited to 16382 bytes",
    ),
    (
        "ERR_HTTP2_CONNECT_AUTHORITY",
        &[ErrorKind::Error],
        ":authority header is required for CONNECT requests",
    ),
    (
        "ERR_HTTP2_CONNECT_PATH",
        &[ErrorKind::Error],
        "The :path header is forbidden for CONNECT requests",
    ),
    (
        "ERR_HTTP2_CONNECT_SCHEME",
        &[ErrorKind::Error],
        "The :scheme header is forbidden for CONNECT requests",
    ),
    (
        "ERR_HTTP2_GOAWAY_SESSION",
        &[ErrorKind::Error],
        "New streams cannot be created after receiving a GOAWAY",
    ),
    (
        "ERR_HTTP2_HEADERS_AFTER_RESPOND",
        &[ErrorKind::Error],
        "Cannot specify additional headers after response initiated",
    ),
    (
        "ERR_HTTP2_HEADERS_SENT",
        &[ErrorKind::Error],
        "Response has already been initiated.",
    ),
    (
        "ERR_HTTP2_HEADER_SINGLE_VALUE",
        &[ErrorKind::Type],
        "Header field \"%s\" must only have a single value",
    ),
    (
        "ERR_HTTP2_INFO_STATUS_NOT_ALLOWED",
        &[ErrorKind::Range],
        "Informational status codes cannot be used",
    ),
    (
        "ERR_HTTP2_INVALID_CONNECTION_HEADERS",
        &[ErrorKind::Type],
        "HTTP/1 Connection specific headers are forbidden: \"%s\"",
    ),
    (
        "ERR_HTTP2_INVALID_HEADER_VALUE",
        &[ErrorKind::Type],
        "Invalid value \"%s\" for header \"%s\"",
    ),
    (
        "ERR_HTTP2_INVALID_INFO_STATUS",
        &[ErrorKind::Range],
        "Invalid informational status code: %s",
    ),
    (
        "ERR_HTTP2_INVALID_ORIGIN",
        &[ErrorKind::Type],
        "HTTP/2 ORIGIN frames require a valid origin",
    ),
    (
        "ERR_HTTP2_INVALID_PACKED_SETTINGS_LENGTH",
        &[ErrorKind::Range],
        "Packed settings length must be a multiple of six",
    ),
    (
        "ERR_HTTP2_INVALID_PSEUDOHEADER",
        &[ErrorKind::Type],
        "\"%s\" is an invalid pseudoheader or is used incorrectly",
    ),
    (
        "ERR_HTTP2_INVALID_SESSION",
        &[ErrorKind::Error],
        "The session has been destroyed",
    ),
    (
        "ERR_HTTP2_INVALID_STREAM",
        &[ErrorKind::Error],
        "The stream has been destroyed",
    ),
    (
        "ERR_HTTP2_MAX_PENDING_SETTINGS_ACK",
        &[ErrorKind::Error],
        "Maximum number of pending settings acknowledgements",
    ),
    (
        "ERR_HTTP2_NESTED_PUSH",
        &[ErrorKind::Error],
        "A push stream cannot initiate another push stream.",
    ),
    ("ERR_HTTP2_NO_MEM", &[ErrorKind::Error], "Out of memory"),
    (
        "ERR_HTTP2_NO_SOCKET_MANIPULATION",
        &[ErrorKind::Error],
        "HTTP/2 sockets should not be directly manipulated (e.g. read and written)",
    ),
    (
        "ERR_HTTP2_ORIGIN_LENGTH",
        &[ErrorKind::Type],
        "HTTP/2 ORIGIN frames are limited to 16382 bytes",
    ),
    (
        "ERR_HTTP2_OUT_OF_STREAMS",
        &[ErrorKind::Error],
        "No stream ID is available because maximum stream ID has been reached",
    ),
    (
        "ERR_HTTP2_PAYLOAD_FORBIDDEN",
        &[ErrorKind::Error],
        "Responses with %s status must not have a payload",
    ),
    (
        "ERR_HTTP2_PING_CANCEL",
        &[ErrorKind::Error],
        "HTTP2 ping cancelled",
    ),
    (
        "ERR_HTTP2_PING_LENGTH",
        &[ErrorKind::Range],
        "HTTP2 ping payload must be 8 bytes",
    ),
    (
        "ERR_HTTP2_PSEUDOHEADER_NOT_ALLOWED",
        &[ErrorKind::Type],
        "Cannot set HTTP/2 pseudo-headers",
    ),
    (
        "ERR_HTTP2_PUSH_DISABLED",
        &[ErrorKind::Error],
        "HTTP/2 client has disabled push streams",
    ),
    (
        "ERR_HTTP2_SEND_FILE",
        &[ErrorKind::Error],
        "Directories cannot be sent",
    ),
    (
        "ERR_HTTP2_SEND_FILE_NOSEEK",
        &[ErrorKind::Error],
        "Offset or length can only be specified for regular files",
    ),
    (
        "ERR_HTTP2_SESSION_ERROR",
        &[ErrorKind::Error],
        "Session closed with error code %s",
    ),
    (
        "ERR_HTTP2_SETTINGS_CANCEL",
        &[ErrorKind::Error],
        "HTTP2 session settings canceled",
    ),
    (
        "ERR_HTTP2_SOCKET_BOUND",
        &[ErrorKind::Error],
        "The socket is already bound to an Http2Session",
    ),
    (
        "ERR_HTTP2_SOCKET_UNBOUND",
        &[ErrorKind::Error],
        "The socket has been disconnected from the Http2Session",
    ),
    (
        "ERR_HTTP2_STATUS_101",
        &[ErrorKind::Error],
        "HTTP status code 101 (Switching Protocols) is forbidden in HTTP/2",
    ),
    (
        "ERR_HTTP2_STATUS_INVALID",
        &[ErrorKind::Range],
        "Invalid status code: %s",
    ),
    (
        "ERR_HTTP2_STREAM_ERROR",
        &[ErrorKind::Error],
        "Stream closed with error code %s",
    ),
    (
        "ERR_HTTP2_STREAM_SELF_DEPENDENCY",
        &[ErrorKind::Error],
        "A stream cannot depend on itself",
    ),
    (
        "ERR_HTTP2_TOO_MANY_CUSTOM_SETTINGS",
        &[ErrorKind::Error],
        "Number of custom settings exceeds MAX_ADDITIONAL_SETTINGS",
    ),
    (
        "ERR_HTTP2_TOO_MANY_INVALID_FRAMES",
        &[ErrorKind::Error],
        "Too many invalid HTTP/2 frames",
    ),
    (
        "ERR_HTTP2_TRAILERS_ALREADY_SENT",
        &[ErrorKind::Error],
        "Trailing headers have already been sent",
    ),
    (
        "ERR_HTTP2_TRAILERS_NOT_READY",
        &[ErrorKind::Error],
        "Trailing headers cannot be sent until after the wantTrailers event is emitted",
    ),
    (
        "ERR_HTTP2_UNSUPPORTED_PROTOCOL",
        &[ErrorKind::Error],
        "protocol \"%s\" is unsupported.",
    ),
    (
        "ERR_HTTP_BODY_NOT_ALLOWED",
        &[ErrorKind::Error],
        "Adding content for this request method or response status is not allowed.",
    ),
    (
        "ERR_HTTP_CONTENT_LENGTH_MISMATCH",
        &[ErrorKind::Error],
        "Response body's content-length of %s byte(s) does not match the content-length of %s byte(s) set in header",
    ),
    (
        "ERR_HTTP_HEADERS_SENT",
        &[ErrorKind::Error],
        "Cannot %s headers after they are sent to the client",
    ),
    (
        "ERR_HTTP_INVALID_HEADER_VALUE",
        &[ErrorKind::Type],
        "Invalid value \"%s\" for header \"%s\"",
    ),
    (
        "ERR_HTTP_INVALID_STATUS_CODE",
        &[ErrorKind::Range],
        "Invalid status code: %s",
    ),
    (
        "ERR_HTTP_REQUEST_TIMEOUT",
        &[ErrorKind::Error],
        "Request timeout",
    ),
    (
        "ERR_HTTP_SOCKET_ASSIGNED",
        &[ErrorKind::Error],
        "ServerResponse has an already assigned socket",
    ),
    (
        "ERR_HTTP_SOCKET_ENCODING",
        &[ErrorKind::Error],
        "Changing the socket encoding is not allowed per RFC7230 Section 3.",
    ),
    (
        "ERR_HTTP_TRAILER_INVALID",
        &[ErrorKind::Error],
        "Trailers are invalid with this transfer encoding",
    ),
    (
        "ERR_ILLEGAL_CONSTRUCTOR",
        &[ErrorKind::Type],
        "Illegal constructor",
    ),
    (
        "ERR_IMPORT_ATTRIBUTE_MISSING",
        &[ErrorKind::Type],
        "Module \"%s\" needs an import attribute of \"%s: %s\"",
    ),
    (
        "ERR_IMPORT_ATTRIBUTE_TYPE_INCOMPATIBLE",
        &[ErrorKind::Type],
        "Module \"%s\" is not of type \"%s\"",
    ),
    (
        "ERR_IMPORT_ATTRIBUTE_UNSUPPORTED",
        &[ErrorKind::Type],
        "Import attribute \"%s\" with value \"%s\" is not supported",
    ),
    (
        "ERR_INCOMPATIBLE_OPTION_PAIR",
        &[ErrorKind::Type],
        "Option \"%s\" cannot be used in combination with option \"%s\"",
    ),
    (
        "ERR_INPUT_TYPE_NOT_ALLOWED",
        &[ErrorKind::Error],
        "--input-type can only be used with string input via --eval, --print, or STDIN",
    ),
    (
        "ERR_INSPECTOR_ALREADY_ACTIVATED",
        &[ErrorKind::Error],
        "Inspector is already activated. Close it with inspector.close() before activating it again.",
    ),
    (
        "ERR_INSPECTOR_ALREADY_CONNECTED",
        &[ErrorKind::Error],
        "%s is already connected",
    ),
    (
        "ERR_INSPECTOR_CLOSED",
        &[ErrorKind::Error],
        "Session was closed",
    ),
    (
        "ERR_INSPECTOR_COMMAND",
        &[ErrorKind::Error],
        "Inspector error %d: %s",
    ),
    (
        "ERR_INSPECTOR_NOT_ACTIVE",
        &[ErrorKind::Error],
        "Inspector is not active",
    ),
    (
        "ERR_INSPECTOR_NOT_AVAILABLE",
        &[ErrorKind::Error],
        "Inspector is not available",
    ),
    (
        "ERR_INSPECTOR_NOT_CONNECTED",
        &[ErrorKind::Error],
        "Session is not connected",
    ),
    (
        "ERR_INSPECTOR_NOT_WORKER",
        &[ErrorKind::Error],
        "Current thread is not a worker",
    ),
    (
        "ERR_INVALID_ASYNC_ID",
        &[ErrorKind::Range],
        "Invalid %s value: %s",
    ),
    (
        "ERR_INVALID_BUFFER_SIZE",
        &[ErrorKind::Range],
        "Buffer size must be a multiple of %s",
    ),
    (
        "ERR_INVALID_CURSOR_POS",
        &[ErrorKind::Type],
        "Cannot set cursor row without setting its column",
    ),
    (
        "ERR_INVALID_FD",
        &[ErrorKind::Range],
        "\"fd\" must be a positive integer: %s",
    ),
    (
        "ERR_INVALID_FD_TYPE",
        &[ErrorKind::Type],
        "Unsupported fd type: %s",
    ),
    (
        "ERR_INVALID_FILE_URL_HOST",
        &[ErrorKind::Type],
        "File URL host must be \"localhost\" or empty on %s",
    ),
    (
        "ERR_INVALID_FILE_URL_PATH",
        &[ErrorKind::Type],
        "File URL path %s",
    ),
    (
        "ERR_INVALID_HANDLE_TYPE",
        &[ErrorKind::Type],
        "This handle type cannot be sent",
    ),
    (
        "ERR_INVALID_HTTP_TOKEN",
        &[ErrorKind::Type],
        "%s must be a valid HTTP token [\"%s\"]",
    ),
    (
        "ERR_INVALID_IP_ADDRESS",
        &[ErrorKind::Type],
        "Invalid IP address: %s",
    ),
    (
        "ERR_INVALID_PROTOCOL",
        &[ErrorKind::Type],
        "Protocol \"%s\" not supported. Expected \"%s\"",
    ),
    (
        "ERR_INVALID_REPL_EVAL_CONFIG",
        &[ErrorKind::Type],
        "Cannot specify both \"breakEvalOnSigint\" and \"eval\" for REPL",
    ),
    ("ERR_INVALID_REPL_INPUT", &[ErrorKind::Type], "%s"),
    (
        "ERR_INVALID_STATE",
        &[ErrorKind::Error, ErrorKind::Type, ErrorKind::Range],
        "Invalid state: %s",
    ),
    (
        "ERR_INVALID_SYNC_FORK_INPUT",
        &[ErrorKind::Type],
        "Asynchronous forks do not support Buffer, TypedArray, DataView or string input: %s",
    ),
    (
        "ERR_INVALID_THIS",
        &[ErrorKind::Type],
        "Value of \"this\" must be of type %s",
    ),
    (
        "ERR_INVALID_TUPLE",
        &[ErrorKind::Type],
        "%s must be an iterable %s tuple",
    ),
    ("ERR_INVALID_TYPESCRIPT_SYNTAX", &[ErrorKind::Syntax], "%s"),
    ("ERR_INVALID_URI", &[ErrorKind::Uri], "URI malformed"),
    (
        "ERR_IPC_CHANNEL_CLOSED",
        &[ErrorKind::Error],
        "Channel closed",
    ),
    (
        "ERR_IPC_DISCONNECTED",
        &[ErrorKind::Error],
        "IPC channel is already disconnected",
    ),
    (
        "ERR_IPC_ONE_PIPE",
        &[ErrorKind::Error],
        "Child process can have only one IPC pipe",
    ),
    (
        "ERR_IPC_SYNC_FORK",
        &[ErrorKind::Error],
        "IPC cannot be used with synchronous forks",
    ),
    (
        "ERR_METHOD_NOT_IMPLEMENTED",
        &[ErrorKind::Error],
        "The %s method is not implemented",
    ),
    ("ERR_MISSING_OPTION", &[ErrorKind::Type], "%s is required"),
    (
        "ERR_MULTIPLE_CALLBACK",
        &[ErrorKind::Error],
        "Callback called multiple times",
    ),
    (
        "ERR_NAPI_CONS_FUNCTION",
        &[ErrorKind::Type],
        "Constructor must be a function",
    ),
    (
        "ERR_NAPI_INVALID_DATAVIEW_ARGS",
        &[ErrorKind::Range],
        "byte_offset + byte_length should be less than or equal to the size in bytes of the array passed in",
    ),
    (
        "ERR_NAPI_INVALID_TYPEDARRAY_ALIGNMENT",
        &[ErrorKind::Range],
        "start offset of %s should be a multiple of %s",
    ),
    (
        "ERR_NAPI_INVALID_TYPEDARRAY_LENGTH",
        &[ErrorKind::Range],
        "Invalid typed array length",
    ),
    (
        "ERR_NOT_BUILDING_SNAPSHOT",
        &[ErrorKind::Error],
        "Operation cannot be invoked when not building startup snapshot",
    ),
    (
        "ERR_NOT_IN_SINGLE_EXECUTABLE_APPLICATION",
        &[ErrorKind::Error],
        "Operation cannot be invoked when not in a single-executable application",
    ),
    (
        "ERR_NOT_SUPPORTED_IN_SNAPSHOT",
        &[ErrorKind::Error],
        "%s is not supported in startup snapshot",
    ),
    (
        "ERR_NO_CRYPTO",
        &[ErrorKind::Error],
        "Node.js is not compiled with OpenSSL crypto support",
    ),
    (
        "ERR_NO_ICU",
        &[ErrorKind::Type],
        "%s is not supported on Node.js compiled without ICU",
    ),
    (
        "ERR_NO_TYPESCRIPT",
        &[ErrorKind::Error],
        "Node.js is not compiled with TypeScript support",
    ),
    (
        "ERR_OPERATION_FAILED",
        &[ErrorKind::Error, ErrorKind::Type],
        "Operation failed: %s",
    ),
    (
        "ERR_PARSE_ARGS_INVALID_OPTION_VALUE",
        &[ErrorKind::Type],
        "%s",
    ),
    (
        "ERR_PERFORMANCE_INVALID_TIMESTAMP",
        &[ErrorKind::Type],
        "%d is not a valid timestamp",
    ),
    (
        "ERR_PERFORMANCE_MEASURE_INVALID_OPTIONS",
        &[ErrorKind::Type],
        "%s",
    ),
    (
        "ERR_QUIC_APPLICATION_ERROR",
        &[ErrorKind::Error],
        "A QUIC application error occurred. %d [%s]",
    ),
    (
        "ERR_QUIC_CONNECTION_FAILED",
        &[ErrorKind::Error],
        "QUIC connection failed",
    ),
    (
        "ERR_QUIC_ENDPOINT_CLOSED",
        &[ErrorKind::Error],
        "QUIC endpoint closed: %s (%d)",
    ),
    (
        "ERR_QUIC_OPEN_STREAM_FAILED",
        &[ErrorKind::Error],
        "Failed to open QUIC stream",
    ),
    (
        "ERR_QUIC_TRANSPORT_ERROR",
        &[ErrorKind::Error],
        "A QUIC transport error occurred. %d [%s]",
    ),
    (
        "ERR_QUIC_VERSION_NEGOTIATION_ERROR",
        &[ErrorKind::Error],
        "The QUIC session requires version negotiation",
    ),
    ("ERR_REQUIRE_CYCLE_MODULE", &[ErrorKind::Error], "%s"),
    (
        "ERR_SCRIPT_EXECUTION_INTERRUPTED",
        &[ErrorKind::Error],
        "Script execution was interrupted by `SIGINT`",
    ),
    (
        "ERR_SERVER_ALREADY_LISTEN",
        &[ErrorKind::Error],
        "Listen method has been called more than once without closing.",
    ),
    (
        "ERR_SERVER_NOT_RUNNING",
        &[ErrorKind::Error],
        "Server is not running.",
    ),
    (
        "ERR_SINGLE_EXECUTABLE_APPLICATION_ASSET_NOT_FOUND",
        &[ErrorKind::Error],
        "Cannot find asset %s for the single executable application",
    ),
    (
        "ERR_SOCKET_ALREADY_BOUND",
        &[ErrorKind::Error],
        "Socket is already bound",
    ),
    (
        "ERR_SOCKET_BAD_BUFFER_SIZE",
        &[ErrorKind::Type],
        "Buffer size must be a positive integer",
    ),
    (
        "ERR_SOCKET_BAD_TYPE",
        &[ErrorKind::Type],
        "Bad socket type specified. Valid types are: udp4, udp6",
    ),
    ("ERR_SOCKET_CLOSED", &[ErrorKind::Error], "Socket is closed"),
    (
        "ERR_SOCKET_CLOSED_BEFORE_CONNECTION",
        &[ErrorKind::Error],
        "Socket closed before the connection was established",
    ),
    (
        "ERR_SOCKET_CONNECTION_TIMEOUT",
        &[ErrorKind::Error],
        "Socket connection timeout",
    ),
    (
        "ERR_SOCKET_DGRAM_IS_CONNECTED",
        &[ErrorKind::Error],
        "Already connected",
    ),
    (
        "ERR_SOCKET_DGRAM_NOT_CONNECTED",
        &[ErrorKind::Error],
        "Not connected",
    ),
    (
        "ERR_SOCKET_DGRAM_NOT_RUNNING",
        &[ErrorKind::Error],
        "Not running",
    ),
    (
        "ERR_SRI_PARSE",
        &[ErrorKind::Syntax],
        "Subresource Integrity string %j had an unexpected %j at position %d",
    ),
    (
        "ERR_STREAM_ALREADY_FINISHED",
        &[ErrorKind::Error],
        "Cannot call %s after a stream was finished",
    ),
    (
        "ERR_STREAM_CANNOT_PIPE",
        &[ErrorKind::Error],
        "Cannot pipe, not readable",
    ),
    (
        "ERR_STREAM_DESTROYED",
        &[ErrorKind::Error],
        "Cannot call %s after a stream was destroyed",
    ),
    (
        "ERR_STREAM_NULL_VALUES",
        &[ErrorKind::Type],
        "May not write null values to stream",
    ),
    (
        "ERR_STREAM_PREMATURE_CLOSE",
        &[ErrorKind::Error],
        "Premature close",
    ),
    (
        "ERR_STREAM_PUSH_AFTER_EOF",
        &[ErrorKind::Error],
        "stream.push() after EOF",
    ),
    (
        "ERR_STREAM_UNSHIFT_AFTER_END_EVENT",
        &[ErrorKind::Error],
        "stream.unshift() after end event",
    ),
    (
        "ERR_STREAM_WRAP",
        &[ErrorKind::Error],
        "Stream has StringDecoder set or is in objectMode",
    ),
    (
        "ERR_STREAM_WRITE_AFTER_END",
        &[ErrorKind::Error],
        "write after end",
    ),
    ("ERR_SYNTHETIC", &[ErrorKind::Error], "JavaScript Callstack"),
    (
        "ERR_TLS_ALPN_CALLBACK_WITH_PROTOCOLS",
        &[ErrorKind::Type],
        "The ALPNCallback and ALPNProtocols TLS options are mutually exclusive",
    ),
    (
        "ERR_TLS_CERT_ALTNAME_FORMAT",
        &[ErrorKind::Syntax],
        "Invalid subject alternative name string",
    ),
    (
        "ERR_TLS_DH_PARAM_SIZE",
        &[ErrorKind::Error],
        "DH parameter size %s is less than 2048",
    ),
    (
        "ERR_TLS_HANDSHAKE_TIMEOUT",
        &[ErrorKind::Error],
        "TLS handshake timeout",
    ),
    (
        "ERR_TLS_INVALID_CONTEXT",
        &[ErrorKind::Type],
        "%s must be a SecureContext",
    ),
    (
        "ERR_TLS_INVALID_PROTOCOL_VERSION",
        &[ErrorKind::Type],
        "%j is not a valid %s TLS protocol version",
    ),
    (
        "ERR_TLS_INVALID_STATE",
        &[ErrorKind::Error],
        "TLS socket connection must be securely established",
    ),
    (
        "ERR_TLS_PROTOCOL_VERSION_CONFLICT",
        &[ErrorKind::Type],
        "TLS protocol version %j conflicts with secureProtocol %j",
    ),
    (
        "ERR_TLS_RENEGOTIATION_DISABLED",
        &[ErrorKind::Error],
        "TLS session renegotiation disabled for this socket",
    ),
    (
        "ERR_TLS_REQUIRED_SERVER_NAME",
        &[ErrorKind::Error],
        "\"servername\" is required parameter for Server.addContext",
    ),
    (
        "ERR_TLS_SESSION_ATTACK",
        &[ErrorKind::Error],
        "TLS session renegotiation attack detected",
    ),
    (
        "ERR_TLS_SNI_FROM_SERVER",
        &[ErrorKind::Error],
        "Cannot issue SNI from a TLS server-side socket",
    ),
    (
        "ERR_TRACE_EVENTS_CATEGORY_REQUIRED",
        &[ErrorKind::Type],
        "At least one category is required",
    ),
    (
        "ERR_TRACE_EVENTS_UNAVAILABLE",
        &[ErrorKind::Error],
        "Trace events are unavailable",
    ),
    (
        "ERR_UNAVAILABLE_DURING_EXIT",
        &[ErrorKind::Error],
        "Cannot call function in process exit handler",
    ),
    (
        "ERR_UNCAUGHT_EXCEPTION_CAPTURE_ALREADY_SET",
        &[ErrorKind::Error],
        "`process.setupUncaughtExceptionCapture()` was called while a capture callback was already active",
    ),
    (
        "ERR_UNESCAPED_CHARACTERS",
        &[ErrorKind::Type],
        "%s contains unescaped characters",
    ),
    (
        "ERR_UNKNOWN_BUILTIN_MODULE",
        &[ErrorKind::Error],
        "No such built-in module: %s",
    ),
    (
        "ERR_UNKNOWN_CREDENTIAL",
        &[ErrorKind::Error],
        "%s identifier does not exist: %s",
    ),
    (
        "ERR_UNKNOWN_ENCODING",
        &[ErrorKind::Type],
        "Unknown encoding: %s",
    ),
    (
        "ERR_UNKNOWN_FILE_EXTENSION",
        &[ErrorKind::Type],
        "Unknown file extension \"%s\" for %s",
    ),
    (
        "ERR_UNKNOWN_MODULE_FORMAT",
        &[ErrorKind::Range],
        "Unknown module format: %s for URL %s",
    ),
    (
        "ERR_UNKNOWN_SIGNAL",
        &[ErrorKind::Type],
        "Unknown signal: %s",
    ),
    (
        "ERR_UNSUPPORTED_NODE_MODULES_TYPE_STRIPPING",
        &[ErrorKind::Error],
        "Stripping types is currently unsupported for files under node_modules, for \"%s\"",
    ),
    (
        "ERR_UNSUPPORTED_RESOLVE_REQUEST",
        &[ErrorKind::Type],
        "Failed to resolve module specifier \"%s\" from \"%s\": Invalid relative URL or base scheme is not hierarchical.",
    ),
    (
        "ERR_UNSUPPORTED_TYPESCRIPT_SYNTAX",
        &[ErrorKind::Syntax],
        "%s",
    ),
    ("ERR_USE_AFTER_CLOSE", &[ErrorKind::Error], "%s was closed"),
    (
        "ERR_VALID_PERFORMANCE_ENTRY_TYPE",
        &[ErrorKind::Error],
        "At least one valid performance entry type is required",
    ),
    (
        "ERR_VM_DYNAMIC_IMPORT_CALLBACK_MISSING",
        &[ErrorKind::Type],
        "A dynamic import callback was not specified.",
    ),
    (
        "ERR_VM_DYNAMIC_IMPORT_CALLBACK_MISSING_FLAG",
        &[ErrorKind::Type],
        "A dynamic import callback was invoked without --experimental-vm-modules",
    ),
    (
        "ERR_VM_MODULE_ALREADY_LINKED",
        &[ErrorKind::Error],
        "Module has already been linked",
    ),
    (
        "ERR_VM_MODULE_CANNOT_CREATE_CACHED_DATA",
        &[ErrorKind::Error],
        "Cached data cannot be created for a module which has been evaluated",
    ),
    (
        "ERR_VM_MODULE_DIFFERENT_CONTEXT",
        &[ErrorKind::Error],
        "Linked modules must use the same context",
    ),
    (
        "ERR_VM_MODULE_NOT_MODULE",
        &[ErrorKind::Error],
        "Provided module is not an instance of Module",
    ),
    (
        "ERR_VM_MODULE_STATUS",
        &[ErrorKind::Error],
        "Module status %s",
    ),
    (
        "ERR_WASI_ALREADY_STARTED",
        &[ErrorKind::Error],
        "WASI instance has already started",
    ),
    (
        "ERR_WEBASSEMBLY_RESPONSE",
        &[ErrorKind::Type],
        "WebAssembly response %s",
    ),
    (
        "ERR_WORKER_INIT_FAILED",
        &[ErrorKind::Error],
        "Worker initialization failure: %s",
    ),
    (
        "ERR_WORKER_MESSAGING_ERRORED",
        &[ErrorKind::Error],
        "The destination thread threw an error while processing the message",
    ),
    (
        "ERR_WORKER_MESSAGING_FAILED",
        &[ErrorKind::Error],
        "Cannot find the destination thread or listener",
    ),
    (
        "ERR_WORKER_MESSAGING_SAME_THREAD",
        &[ErrorKind::Error],
        "Cannot sent a message to the same thread",
    ),
    (
        "ERR_WORKER_MESSAGING_TIMEOUT",
        &[ErrorKind::Error],
        "Sending a message to another thread timed out",
    ),
    (
        "ERR_WORKER_NOT_RUNNING",
        &[ErrorKind::Error],
        "Worker instance not running",
    ),
    (
        "ERR_WORKER_OUT_OF_MEMORY",
        &[ErrorKind::Error],
        "Worker terminated due to reaching memory limit: %s",
    ),
    (
        "ERR_WORKER_UNSERIALIZABLE_ERROR",
        &[ErrorKind::Error],
        "Serializing an uncaught exception failed",
    ),
    (
        "ERR_WORKER_UNSUPPORTED_OPERATION",
        &[ErrorKind::Type],
        "%s is not supported in workers",
    ),
    (
        "ERR_ZSTD_INVALID_PARAM",
        &[ErrorKind::Range],
        "%s is not a valid zstd parameter",
    ),
];

/// Defines each code `codes` lacks: a constructor for its first class, with
/// its other classes and `HideStackFramesError` as properties.
pub(crate) fn install(realm: &Realm, codes: &Object) {
    for &(code, kinds, template) in TEMPLATES {
        if codes.own_property(&Key::name(code)).is_some() {
            continue;
        }
        let class = constructor(realm, code, kinds[0], template);
        for &kind in &kinds[1..] {
            class.define(
                Key::name(kind_name(kind)),
                Property::data(Value::Object(constructor(realm, code, kind, template))),
            );
        }
        class.define(
            Key::name("HideStackFramesError"),
            Property::data(Value::Object(class.clone())),
        );
        codes.define(Key::name(code), Property::data(Value::Object(class)));
    }
}

fn constructor(
    realm: &Realm,
    code: &'static str,
    kind: ErrorKind,
    template: &'static str,
) -> Object {
    realm.function("NodeError", 0, true, move |realm, _, args| {
        let message = format(realm, template, args)?;
        Ok(Value::Object(crate::node_error::object(
            realm, kind, code, &message,
        )))
    })
}

fn kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Type => "TypeError",
        ErrorKind::Range => "RangeError",
        ErrorKind::Syntax => "SyntaxError",
        ErrorKind::Uri => "URIError",
        _ => "Error",
    }
}

/// Node's `format` for message templates: `%s` as `String()` gives it, or
/// inspected for objects and symbols; `%d` and `%i` as numbers; `%j` as
/// JSON; `%%` as `%`. A directive without an argument stays as written.
pub(crate) fn format(realm: &mut Realm, template: &str, args: &[Value]) -> Result<String, Thrown> {
    let mut out = String::new();
    let mut next = 0;
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            Some('%') => {
                chars.next();
                out.push('%');
            }
            Some(directive @ ('s' | 'd' | 'i' | 'j')) if next < args.len() => {
                chars.next();
                let value = &args[next];
                next += 1;
                match directive {
                    's' => out.push_str(&crate::modules::formatted(realm, value)?),
                    'j' => {
                        let json = crate::json_builtin::stringify(
                            realm,
                            Value::Undefined,
                            core::slice::from_ref(value),
                        )?;
                        out.push_str(&ops::to_string(realm, &json)?.to_rust_lossy());
                    }
                    _ => {
                        let mut number = ops::to_number(realm, value)?;
                        if directive == 'i' {
                            number = crate::libm::trunc(number);
                        }
                        let text = ops::to_string(realm, &Value::Number(number))?;
                        out.push_str(&text.to_rust_lossy());
                    }
                }
            }
            _ => out.push('%'),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
