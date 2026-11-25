//! Native values and abstract operations for compiled application behavior.
//!
//! The compiler lowers each source function to a Rust function that calls
//! these operations directly. This crate holds dynamic data and implements
//! the source language's conversions, operators, property access and calls;
//! it parses no JavaScript source and dispatches no JavaScript instructions.
//! It depends on no
//! JavaScript engine, so a native application linking it ships none.
//!
//! Regular expressions match with the native `nativeboat-regexp` library,
//! which contains no JavaScript engine either.
//!
//! It also links no Rust `std` and no C runtime: generated executables link
//! only `core`, `alloc` and operating-system DLLs (see `platform`), so they
//! build without MSVC, the Windows SDK or another installed toolchain.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

/// The `alloc` items the `std` prelude would otherwise provide.
mod prelude {
    pub use alloc::borrow::ToOwned;
    pub use alloc::format;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

mod abort_builtin;
pub mod app_host;
mod array_builtin;
pub mod bigint;
mod bigint_builtin;
pub mod binding;
mod blob_binding;
mod buffer_binding;
mod builtins;
mod ced;
pub mod child_process_builtin;
pub mod class;
mod clip_filepaths;
mod collection_builtin;
mod com;
mod console_builtin;
pub mod context_bridge;
pub mod coroutine;
mod crypto_builtin;
mod crypto_cipher;
mod crypto_platform;
mod date_builtin;
/// Engine-free document and element bindings for compiled renderer graphs.
pub mod dom_binding;
mod electron_app;
pub mod electron_builtin;
pub mod electron_clipboard;
mod electron_crash_reporter;
pub mod electron_dialog;
pub mod electron_ipc;
mod electron_login_item;
pub mod electron_menu;
mod electron_native_image;
pub mod electron_notify;
mod electron_power;
pub mod electron_protocol;
mod electron_safe_storage;
pub mod electron_screen;
pub mod electron_shell;
mod electron_single_instance;
mod electron_system;
pub mod electron_window;
mod encoding;
pub mod environment_builtin;
mod errors;
mod event_loop;
mod event_target_builtin;
mod float;
pub mod fork;
mod fs_binding;
mod fs_event_binding;
/// Native Windows filesystem operations; Node's source binding follows.
pub mod fs_win;
mod function;
mod global_number;
mod global_uri;
mod http_binding;
mod icu_converter;
mod inspect;
mod intl_builtin;
pub mod iter;
mod json_builtin;
pub mod keyboard;
pub mod keytar;
mod libm;
mod location;
mod math;
mod math_sum;
pub mod modules;
pub mod native_addons;
pub mod node_error;
mod node_error_table;
pub mod number;
mod number_builtin;
pub mod object;
mod object_builtin;
pub mod ops;
mod os_binding;
mod performance_builtin;
mod pipe_binding;
pub mod platform;
pub mod process_builtin;
mod process_events;
mod process_usage;
mod promise_builtin;
mod proxy_builtin;
mod radix;
mod realm;
mod realm_ports;
mod reflect_builtin;
mod regexp_builtin;
pub mod spelling;
pub(crate) mod sqlite;
pub mod string;
mod string_builtin;
mod string_decoder;
mod structured_clone_builtin;
mod symbol;
mod symbol_builtin;
mod system_proxy;
mod table;
mod tcp_binding;
mod text_encoding_builtin;
pub mod trace;
mod typed_array_builtin;
mod unicode;
mod unicode_tables;
mod url_binding;
mod uv_error;
mod value;
pub(crate) mod wasm;
mod wasm_binding;
mod weak_builtin;
#[cfg(feature = "dom")]
pub mod web;
mod web_crypto_builtin;
pub mod woff;
mod work_queue;
#[cfg(feature = "dom")]
pub mod worker;
mod zlib_builtin;

pub use encoding::base64_encode;
pub use event_loop::HostWait;
pub use function::with_source_text;
pub use object::{
    ArrayData, Descriptor, Key, Kind, Object, Property, Slot, WeakObject, Wrapped, same_value,
};
pub use realm::{ErrorKind, Realm};
pub use regexp_builtin::literal as regexp_literal;
pub use string::JsStr;
pub use symbol::{Symbol, WellKnown};
pub use value::{Completion, Thrown, Value};
