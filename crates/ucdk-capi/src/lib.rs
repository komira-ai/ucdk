// SPDX-License-Identifier: Apache-2.0
//! The C ABI of ucdk: the `cdylib` (`libucdk_capi.so`, `libucdk_capi.dylib`) and the header
//! `include/ucdk.h` that declares it.
//!
//! This crate adds no behaviour. A session wraps the engine's one seam, `Engine::handle`, over
//! the `ucdk.api.v1` messages of `ucdk-api`; whatever a caller can do through the library the
//! command line's tests can do through the engine. Eight functions cross the boundary, all
//! carrying protobuf bytes, none taking a callback; see the header for the contract and
//! `docs/architecture.md` (sections 3 to 5) for why.
//!
//! # Where the `unsafe` is
//!
//! The workspace forbids `unsafe`. This is the one crate that needs it, because a C caller hands
//! over raw pointers, and it is scoped in three steps: the manifest lowers the lint to `deny`
//! (and `cargo xtask check` refuses that line in any other manifest), only the [`ffi`] module
//! lifts the denial, and every `unsafe` block there says why it is sound. [`Session`] and
//! everything it does is safe Rust.

mod session;
mod status;

#[allow(unsafe_code)]
mod ffi;

pub use ffi::{
    Buf, SessionHandle, into_handle, ucdk_abi_version, ucdk_apply, ucdk_buf_free, ucdk_destroy,
    ucdk_load_catalog, ucdk_plan, ucdk_session_close, ucdk_session_open, ucdk_status,
};
pub use session::{Session, Verb};
pub use status::{ABI, BUSY, CallError, DECODE, INTERNAL, INVALID_ARGUMENT, OK, PANIC, TOO_LARGE};

/// The major version of the ABI. Zero until the first release, when nothing is promised.
pub const ABI_MAJOR: u32 = 0;
/// The minor version of the ABI: raised when a function or a verb is added.
pub const ABI_MINOR: u32 = 1;
/// The longest request, in bytes, that a call accepts.
pub const MAX_REQUEST_LEN: usize = 64 * 1024 * 1024;

/// The exported functions, in the order the header declares them.
pub const SYMBOLS: [&str; 9] = [
    "ucdk_abi_version",
    "ucdk_session_open",
    "ucdk_session_close",
    "ucdk_load_catalog",
    "ucdk_plan",
    "ucdk_apply",
    "ucdk_destroy",
    "ucdk_status",
    "ucdk_buf_free",
];
