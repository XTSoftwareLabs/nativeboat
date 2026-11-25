//! Process startup for programs that use Rust `std`, such as nativeboat
//! itself, in place of the Visual C++ runtime: the entry point, the static
//! initializer tables `std` registers in, the thread-local storage
//! directory, the math functions `std`'s floating-point methods call and
//! the exception-handling personalities its unwind tables name.
//!
//! Such programs link with `/NODEFAULTLIB` and abort on panic. `std` reaches
//! Windows directly: it reads the command line, the environment and the
//! standard handles itself, so the entry point only runs the initializers
//! and `main`.

#![no_std]

// The memory primitives and the compiler support symbols.
extern crate nativeboat_start as _;

#[cfg(not(test))]
mod runtime;
#[cfg(not(test))]
pub use runtime::TlsDirectory;
