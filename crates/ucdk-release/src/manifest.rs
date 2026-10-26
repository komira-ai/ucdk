// SPDX-License-Identifier: Apache-2.0
//! Composing the manifest of a member. The engine, not the build system, composes it: type and
//! name from the declaration, version from the stamp, platform from the action, `file` from the
//! name of what was located, `size` and `sha256` recomputed from the store entry that was just
//! written (never from the build system's tree), the stamp, and the metadata arm (the
//! declaration's labels for a file; `os` and `cpu` read from the layout for an image).
//!
//! The composed manifest is then held to the entry by [`crate::verify::verify_member`], which
//! is the check that the entry is what the manifest says.

use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{
    ArtifactDecl, ArtifactType, BuildStamp, FileMeta, Manifest, OciMeta, Stamped,
};

use crate::machine::{Identity, identity};
use crate::oci;
use crate::verify::{Check, Entry, MANIFEST_SCHEMA, VerifyError, fail};

/// The manifest of the member `decl` that `entry` holds under the bare name `file`, built for
/// `platform` by the build `stamp`.
///
/// A file is the entry's one file of that name; an image is the layout under that directory.
/// A type that names no identity, a file the entry does not hold and a layout that cannot be
/// read are refused.
pub fn compose_manifest(
    decl: &ArtifactDecl,
    platform: &str,
    stamp: &BuildStamp,
    file: &str,
    entry: &Entry,
) -> Result<Manifest, VerifyError> {
    let (sha256, size, metadata) = match ArtifactType::try_from(decl.r#type).ok().and_then(identity)
    {
        Some(Identity::FileBytes) => {
            let bytes = entry.get(file).ok_or_else(|| {
                fail(
                    Check::EntryShape,
                    format!("the entry of {} holds no file {file:?}", decl.name),
                )
            })?;
            (
                oci::hex_of(bytes),
                bytes.len() as u64,
                Metadata::FileMeta(FileMeta {
                    labels: decl.labels.clone(),
                }),
            )
        }
        Some(Identity::OciManifestDigest) => {
            let layout = oci::read(file, entry)?;
            (
                layout.digest,
                layout.size,
                Metadata::Oci(OciMeta {
                    os: layout.os,
                    cpu: layout.cpu,
                }),
            )
        }
        None => {
            return Err(fail(
                Check::Type,
                format!(
                    "the type {} of {} names no identity, so it has no manifest",
                    decl.r#type, decl.name
                ),
            ));
        }
    };
    Ok(Manifest {
        schema_version: MANIFEST_SCHEMA,
        r#type: decl.r#type,
        name: decl.name.clone(),
        version: stamp.version.clone(),
        platform: platform.to_owned(),
        file: file.to_owned(),
        sha256,
        size,
        stamp: Some(Stamped {
            build_number: stamp.build_number,
            source_commit: stamp.source_commit.clone(),
        }),
        metadata: Some(metadata),
    })
}
