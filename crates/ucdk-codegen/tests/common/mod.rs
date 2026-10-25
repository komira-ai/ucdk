// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]
//! Helpers shared by the generator tests: protos held in memory, and the fixture on disk.

use std::path::PathBuf;

use prost_reflect::DescriptorPool;
use protox::Compiler;
use protox::file::{
    ChainFileResolver, File, FileResolver, GoogleFileResolver, IncludeFileResolver,
};
use ucdk_codegen::{Error, File as Generated};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/proto")
}

/// Source text held in memory, found by name.
struct Memory(Vec<(String, String)>);

impl FileResolver for Memory {
    fn open_file(&self, name: &str) -> Result<File, protox::Error> {
        self.0.iter().find(|(n, _)| n == name).map_or_else(
            || Err(protox::Error::file_not_found(name)),
            |(_, source)| File::from_source(name, source),
        )
    }
}

/// The pool of `open`, with files held in memory (`sources`), the repository's protos and the
/// fixture's; `comments` says whether the source info is kept.
pub fn pool_with(sources: &[(&str, &str)], open: &str, comments: bool) -> DescriptorPool {
    let mut chain = ChainFileResolver::new();
    chain.add(Memory(
        sources
            .iter()
            .map(|(n, s)| ((*n).to_owned(), (*s).to_owned()))
            .collect(),
    ));
    chain.add(IncludeFileResolver::new(fixture_dir()));
    for dir in ucdk_codegen::PROTO_ROOTS {
        chain.add(IncludeFileResolver::new(root().join(dir)));
    }
    chain.add(GoogleFileResolver::new());
    let mut compiler = Compiler::with_file_resolver(chain);
    compiler.include_source_info(comments);
    compiler.include_imports(true);
    compiler.open_file(open).expect("the proto compiles");
    if comments {
        compiler.descriptor_pool()
    } else {
        // The pool of a compile keeps the source info; the descriptor set does not.
        DescriptorPool::from_file_descriptor_set(compiler.file_descriptor_set()).expect("a pool")
    }
}

pub fn pool_of(source: &str) -> DescriptorPool {
    pool_with(&[("test.proto", source)], "test.proto", true)
}

pub const SHAPES: &str = "example/shapes/v1/shapes.proto";

pub fn shapes_pool() -> DescriptorPool {
    pool_with(&[], SHAPES, true)
}

pub fn generate_shapes() -> Vec<Generated> {
    ucdk_codegen::python::generate(&shapes_pool(), &[SHAPES]).expect("the fixture generates")
}

pub fn generate_shapes_ts() -> Vec<Generated> {
    ucdk_codegen::typescript::generate(&shapes_pool(), &[SHAPES]).expect("the fixture generates")
}

/// The refusal for `source` (a proto file in the package `t`).
pub fn refusal(source: &str) -> Error {
    let text = format!("syntax = \"proto3\";\npackage t;\n{source}");
    ucdk_codegen::python::generate(&pool_of(&text), &["test.proto"]).expect_err("it is refused")
}

/// The refusal of the TypeScript target for `source` (a proto file in the package `t`).
pub fn refusal_ts(source: &str) -> Error {
    let text = format!("syntax = \"proto3\";\npackage t;\n{source}");
    ucdk_codegen::typescript::generate(&pool_of(&text), &["test.proto"]).expect_err("it is refused")
}

pub fn text_of<'a>(files: &'a [Generated], name: &str) -> &'a str {
    &files
        .iter()
        .find(|f| f.path.ends_with(name))
        .unwrap_or_else(|| panic!("{name} is not generated"))
        .text
}
