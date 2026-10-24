// SPDX-License-Identifier: Apache-2.0
//! Compiles `proto/ucdk/resource/v1/resource.proto` into Rust at build time. `protox` parses the
//! file in-process, so no `protoc` binary is involved; `prost-build` writes the types into
//! `OUT_DIR` only. Nothing generated is committed.

const PROTO_ROOT: &str = "proto";
const FILES: &[&str] = &["proto/ucdk/resource/v1/resource.proto"];

fn main() {
    println!("cargo:rerun-if-changed={PROTO_ROOT}");
    let descriptors = protox::compile(FILES, [PROTO_ROOT]).expect("resource.proto compiles");
    prost_build::Config::new()
        // Maps are ordered, so encoding a message is deterministic.
        .btree_map(["."])
        .compile_fds(descriptors)
        .expect("prost-build generates the catalog types");
}
