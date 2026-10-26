// SPDX-License-Identifier: Apache-2.0
//! Compiles `proto/ucdk/api/v1/api.proto` into Rust at build time: `protox` parses in-process
//! (no `protoc` binary) and `prost-build` writes into `OUT_DIR` only.

const API_ROOT: &str = "proto";
const FILES: &[&str] = &["ucdk/api/v1/api.proto"];

fn main() {
    // The directive is the build script talking to cargo, not I/O of the library it builds.
    #[allow(clippy::disallowed_macros)]
    {
        println!("cargo:rerun-if-changed={API_ROOT}");
    }
    let descriptors = protox::compile(FILES, [API_ROOT]).expect("api.proto compiles");
    prost_build::Config::new()
        // Maps are ordered, so encoding a message is deterministic.
        .btree_map(["."])
        .compile_fds(descriptors)
        .expect("prost-build generates the API types");
}
