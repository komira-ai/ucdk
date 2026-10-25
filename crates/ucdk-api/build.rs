// SPDX-License-Identifier: Apache-2.0
//! Compiles `proto/ucdk/api/v1/api.proto` into Rust at build time, the way `ucdk-proto` does for
//! the catalog: `protox` parses in-process (no `protoc` binary) and `prost-build` writes into
//! `OUT_DIR` only. The catalog messages the API refers to are the ones `ucdk-proto` already
//! generated, so they are mapped to it instead of being generated twice.

const API_ROOT: &str = "proto";
const CATALOG_ROOT: &str = "../ucdk-proto/proto";
const FILES: &[&str] = &["ucdk/api/v1/api.proto"];

fn main() {
    // The directives are the build script talking to cargo, not I/O of the library it builds.
    #[allow(clippy::disallowed_macros)]
    {
        println!("cargo:rerun-if-changed={API_ROOT}");
        println!("cargo:rerun-if-changed={CATALOG_ROOT}");
    }
    let descriptors = protox::compile(FILES, [API_ROOT, CATALOG_ROOT]).expect("api.proto compiles");
    prost_build::Config::new()
        // Maps are ordered, so encoding a message is deterministic.
        .btree_map(["."])
        .extern_path(".ucdk.resource.v1", "::ucdk_proto::v1")
        .compile_fds(descriptors)
        .expect("prost-build generates the API types");
}
