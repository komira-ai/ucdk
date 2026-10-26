// SPDX-License-Identifier: Apache-2.0
//! The `ucdk.release.v1` messages, generated from `proto/ucdk/release/v1/release.proto` by
//! `build.rs` into the build directory, in [`v1`].
//!
//! The package is not in the list the SDK generator reads, so the generated SDKs do not carry
//! it yet. This crate is pure: it holds the messages and nothing that acts on them.

/// The generated messages of `ucdk.release.v1`.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/ucdk.release.v1.rs"));
}
