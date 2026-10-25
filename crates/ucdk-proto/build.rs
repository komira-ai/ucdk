// SPDX-License-Identifier: Apache-2.0
//! Compiles `proto/ucdk/resource/v1/resource.proto` into Rust at build time. `protox` parses the
//! file in-process, so no `protoc` binary is involved; `prost-build` writes the types into
//! `OUT_DIR` only. Nothing generated is committed.

const PROTO_ROOT: &str = "proto";
const FILES: &[&str] = &["proto/ucdk/resource/v1/resource.proto"];

fn main() {
    // The directive is the build script talking to cargo, not I/O of the library it builds.
    #[allow(clippy::disallowed_macros)]
    {
        println!("cargo:rerun-if-changed={PROTO_ROOT}");
    }
    let mut compiler = protox::Compiler::new([PROTO_ROOT]).expect("the proto root opens");
    compiler.include_imports(true);
    compiler.open_files(FILES).expect("resource.proto compiles");
    let descriptors = compiler.file_descriptor_set();
    // The same descriptors, for the planner. The encoded form is the compiler's own, not prost's:
    // only it keeps the custom options lowering is driven by. `descriptor_set.rs` is the
    // constant that holds them, included by `lib.rs` like the generated types.
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let set = out.join("resource.fds");
    let constant = format!(
        "/// The encoded `FileDescriptorSet` of `resource.proto` and the files it imports, with the\n\
         /// custom options (`portability`, `exposes`, `render`, ...) the planner lowers by.\n\
         pub const DESCRIPTOR_SET: &[u8] = include_bytes!({set:?});\n"
    );
    // Writing into OUT_DIR is what a build script is for, not I/O of the library it builds.
    #[allow(clippy::disallowed_methods)]
    {
        std::fs::write(&set, compiler.encode_file_descriptor_set()).expect("OUT_DIR is writable");
        std::fs::write(out.join("descriptor_set.rs"), constant).expect("OUT_DIR is writable");
    }
    prost_build::Config::new()
        // Maps are ordered, so encoding a message is deterministic.
        .btree_map(["."])
        .compile_fds(descriptors)
        .expect("prost-build generates the catalog types");
}
