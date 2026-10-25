// SPDX-License-Identifier: Apache-2.0
//! Reads the descriptors of the ucdk protos and writes the SDK sources generated from them.
//!
//! The input is a [`DescriptorPool`] compiled in-process by `protox` with the protos' comments
//! (no `protoc`, no per-language protobuf tooling). The output is a list of [`File`]s, each a
//! path relative to an SDK's package directory and its text. Nothing here touches the file
//! system but [`compile`], which reads the protos; the caller writes (or compares) the files.
//!
//! One target exists: [`python`]. Per proto package it writes a module of message classes that
//! encode and decode themselves (so the SDK has no protobuf runtime dependency), and, for the
//! resource catalog, typed templates and typed output references read from the message options.

mod docs;
mod kinds;
mod model;
mod names;
pub mod python;

use std::fmt;
use std::path::{Path, PathBuf};

use prost_reflect::DescriptorPool;

/// The directories the repository's protos are found in, relative to the workspace root.
pub const PROTO_ROOTS: [&str; 2] = ["crates/ucdk-proto/proto", "crates/ucdk-api/proto"];
/// The proto files SDKs are generated from, as named inside [`PROTO_ROOTS`].
pub const PROTO_FILES: [&str; 2] = ["ucdk/resource/v1/resource.proto", "ucdk/api/v1/api.proto"];

/// One generated file: its path inside the SDK package and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: String,
    pub text: String,
}

/// Why nothing was generated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The protos do not compile.
    Compile(String),
    /// A message the generator reads by name is not in the descriptors.
    Missing(String),
    /// A proto uses something the generator does not emit code for, or two names collide in the
    /// output. The generator refuses rather than skipping: a field left out would be one the SDK
    /// silently drops.
    Refused { subject: String, why: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(why) => write!(f, "the protos do not compile: {why}"),
            Self::Missing(name) => write!(f, "`{name}` is not in the descriptors"),
            Self::Refused { subject, why } => write!(f, "{subject}: {why}"),
        }
    }
}

impl std::error::Error for Error {}

impl Error {
    pub(crate) fn refused(subject: impl Into<String>, why: impl Into<String>) -> Self {
        Self::Refused {
            subject: subject.into(),
            why: why.into(),
        }
    }
}

#[allow(clippy::needless_pass_by_value)] // used as a `map_err` function
fn compile_error(e: impl fmt::Display) -> Error {
    Error::Compile(e.to_string())
}

/// Compiles `files` found under `roots`, with imports and comments.
pub fn compile_files(roots: &[PathBuf], files: &[&str]) -> Result<DescriptorPool, Error> {
    let mut compiler = protox::Compiler::new(roots).map_err(compile_error)?;
    compiler.include_imports(true).include_source_info(true);
    compiler.open_files(files).map_err(compile_error)?;
    Ok(compiler.descriptor_pool())
}

/// Compiles the repository's protos ([`PROTO_FILES`]) under the workspace root `root`.
pub fn compile(root: &Path) -> Result<DescriptorPool, Error> {
    let roots: Vec<PathBuf> = PROTO_ROOTS.iter().map(|r| root.join(r)).collect();
    compile_files(&roots, &PROTO_FILES)
}

/// The Python SDK's generated files for the repository's protos under `root`.
pub fn python_for(root: &Path) -> Result<Vec<File>, Error> {
    python::generate(&compile(root)?, &PROTO_FILES)
}
