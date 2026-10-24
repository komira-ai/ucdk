// SPDX-License-Identifier: Apache-2.0
//! The `ucdk.resource.v1` resource catalog as Rust types. The types are generated from
//! `proto/ucdk/resource/v1/resource.proto` by `build.rs` into the build directory; this crate has
//! no hand-written code. It is not a pure crate: it runs a build script.

#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/ucdk.resource.v1.rs"));
}
