// SPDX-License-Identifier: Apache-2.0
//! Compiles `proto/ucdk/release/v1/release.proto` into Rust at build time: `protox` parses
//! in-process (no `protoc` binary) and `prost-build` writes into `OUT_DIR` only.

const ROOT: &str = "proto";
const FILES: &[&str] = &["ucdk/release/v1/release.proto"];

fn main() {
    // The directive is the build script talking to cargo, not I/O of the library it builds.
    #[allow(clippy::disallowed_macros)]
    {
        println!("cargo:rerun-if-changed={ROOT}");
    }
    let descriptors = protox::compile(FILES, [ROOT]).expect("release.proto compiles");
    prost_build::Config::new()
        // Maps are ordered, so encoding a message is deterministic.
        .btree_map(["."])
        .compile_fds(descriptors)
        .expect("prost-build generates the release types");
}
