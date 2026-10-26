// SPDX-License-Identifier: Apache-2.0
//! Verifying what a store holds, over an in-memory tree.
//!
//! [`verify_member`] checks one member: that its entry is exactly what its manifest says and
//! the manifest agrees with the declaration and the build. [`verify_keys`] is about the
//! generation: the keys it holds are exactly the member keys of the set, and the set document
//! once it is written. The engine runs the first after each build, again before sealing and
//! again before it publishes; the second before and after the set document is written.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{
    ArtifactDecl, ArtifactType, BuildStamp, Manifest, Reason, ReleaseSet,
};

use crate::machine::{Identity, identity};
use crate::oci;
use crate::store::{SetScope, StoreKey, is_plain};

/// The schema version of a manifest that [`verify_member`] reads. The version is not part of
/// the set hash (the design's field list for it does not name it), so this check is the only
/// thing that holds a stored manifest to it.
pub(crate) const MANIFEST_SCHEMA: i32 = 1;

/// One check of a member or of a generation; each has a test that breaks it alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// The manifest is of the schema version this release reads.
    Schema,
    /// The manifest name equals the declaration's, byte for byte.
    Name,
    /// The manifest type equals the declaration's, and names an identity.
    Type,
    /// The entry is what the manifest says: the file it names, or the layout it names.
    EntryShape,
    /// No path exists in the entry beyond the file or the layout.
    NoOtherPath,
    /// An image blob hashes to its name.
    BlobDigest,
    /// The file is not empty.
    NonEmpty,
    /// The identity digest recomputed from the entry equals the manifest's.
    Identity,
    /// The size recomputed from the entry equals the manifest's.
    Size,
    /// The platform is the run's, or `any` for a file, and for an image the layout's.
    Platform,
    /// The version equals the build's.
    Version,
    /// The recorded stamp equals the build's.
    Stamp,
    /// The keys of a generation are the member keys of the set, and the set key when required.
    Keys,
}

impl Check {
    /// The reason a refusal by this check carries.
    pub fn reason(self) -> Reason {
        match self {
            Check::Platform => Reason::PlatformMismatch,
            Check::Version => Reason::MemberVersion,
            _ => Reason::SetMember,
        }
    }
}

/// A member or generation that failed a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError {
    pub check: Check,
    pub reason: Reason,
    pub message: String,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

pub(crate) fn fail(check: Check, message: String) -> VerifyError {
    VerifyError {
        check,
        reason: check.reason(),
        message,
    }
}

/// The files of one store entry: each relative path with its bytes.
pub type Entry = BTreeMap<String, Vec<u8>>;

/// Checks that `manifest` and `entry` are a member of the build described by `decl`, `platform`
/// and `stamp`. The first check that fails is returned.
///
/// The identity digest and the size are recomputed from the bytes of the entry, never read from
/// the manifest: a file entry holds one non-empty file named by the manifest, and an image
/// entry is a layout under the directory the manifest names, every blob hashing to its name.
pub fn verify_member(
    decl: &ArtifactDecl,
    manifest: &Manifest,
    entry: &Entry,
    platform: &str,
    stamp: &BuildStamp,
) -> Result<(), VerifyError> {
    if manifest.name != decl.name {
        return Err(fail(
            Check::Name,
            format!(
                "the manifest names {:?} and the declaration {:?}",
                manifest.name, decl.name
            ),
        ));
    }
    if manifest.r#type != decl.r#type {
        return Err(fail(
            Check::Type,
            format!(
                "the manifest of {} has type {}, and the declaration {}",
                decl.name, manifest.r#type, decl.r#type
            ),
        ));
    }
    if manifest.schema_version != MANIFEST_SCHEMA {
        return Err(fail(
            Check::Schema,
            format!(
                "the manifest of {} is of schema version {}, and only {MANIFEST_SCHEMA} is read",
                decl.name, manifest.schema_version
            ),
        ));
    }
    // The set document is not trusted, and the store reader joins this name to the entry path:
    // it is held to the rule every part of a store key is held to.
    let file = &manifest.file;
    if !is_plain(file) {
        return Err(fail(
            Check::EntryShape,
            format!(
                "the manifest of {} names {file:?}, which is not a bare name",
                decl.name
            ),
        ));
    }
    match ArtifactType::try_from(manifest.r#type)
        .ok()
        .and_then(identity)
    {
        Some(Identity::OciManifestDigest) => {
            let layout = oci::read(file, entry)?;
            check_same(manifest, &layout.digest, layout.size)?;
            check_oci(manifest, &layout, platform)?;
        }
        Some(Identity::FileBytes) => {
            check_file(manifest, entry)?;
            if manifest.platform != platform && manifest.platform != "any" {
                return Err(fail(
                    Check::Platform,
                    format!(
                        "the platform of {} is {:?}, and the run's is {platform:?} or any",
                        decl.name, manifest.platform
                    ),
                ));
            }
        }
        None => {
            return Err(fail(
                Check::Type,
                format!(
                    "the type {} of {} names no identity",
                    manifest.r#type, decl.name
                ),
            ));
        }
    }
    if manifest.version != stamp.version {
        return Err(fail(
            Check::Version,
            format!(
                "the version of {} is {:?}, and the release's is {:?}",
                decl.name, manifest.version, stamp.version
            ),
        ));
    }
    let stamped = manifest.stamp.clone().unwrap_or_default();
    if stamped.build_number != stamp.build_number || stamped.source_commit != stamp.source_commit {
        return Err(fail(
            Check::Stamp,
            format!(
                "the stamp recorded in {} is not the stamp of this build",
                decl.name
            ),
        ));
    }
    Ok(())
}

fn check_file(manifest: &Manifest, entry: &Entry) -> Result<(), VerifyError> {
    let Some(bytes) = entry.get(&manifest.file) else {
        return Err(fail(
            Check::EntryShape,
            format!(
                "the entry of {} holds no file {:?}",
                manifest.name, manifest.file
            ),
        ));
    };
    if entry.len() > 1 {
        return Err(fail(
            Check::NoOtherPath,
            format!(
                "the entry of {} holds {} paths, and a file entry holds one",
                manifest.name,
                entry.len()
            ),
        ));
    }
    if bytes.is_empty() {
        return Err(fail(
            Check::NonEmpty,
            format!("the file of {} is empty", manifest.name),
        ));
    }
    check_same(manifest, &oci::hex_of(bytes), bytes.len() as u64)
}

/// The identity digest and the size recomputed from the entry against the manifest.
fn check_same(manifest: &Manifest, digest: &str, size: u64) -> Result<(), VerifyError> {
    if manifest.sha256 != digest {
        return Err(fail(
            Check::Identity,
            format!(
                "{} has digest {:?} in its manifest and {digest} in its entry",
                manifest.name, manifest.sha256
            ),
        ));
    }
    if manifest.size != size {
        return Err(fail(
            Check::Size,
            format!(
                "{} has size {} in its manifest and {size} in its entry",
                manifest.name, manifest.size
            ),
        ));
    }
    Ok(())
}

/// An image carries the platform of its config: the run's, never `any`. The config's `os` and
/// `architecture` each equal the manifest's metadata, and their join with `-` is the run's
/// platform. The config's `variant` is ignored: a platform here is an OS and a CPU, and an
/// image of another variant of the same CPU is the same platform.
fn check_oci(manifest: &Manifest, layout: &oci::Layout, platform: &str) -> Result<(), VerifyError> {
    let meta_agrees = matches!(&manifest.metadata, Some(Metadata::Oci(o)) if o.os == layout.os && o.cpu == layout.cpu);
    let own = format!("{}-{}", layout.os, layout.cpu);
    if manifest.platform != platform || own != platform || !meta_agrees {
        return Err(fail(
            Check::Platform,
            format!(
                "{} is for {:?}, its layout and metadata say {own:?}, and the run's platform is {platform:?}",
                manifest.name, manifest.platform
            ),
        ));
    }
    Ok(())
}

/// Whether the set document's key is expected among the keys of a generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// Before the set document is written: it is neither required nor refused.
    MembersOnly,
    /// After it is written: it must exist.
    MembersAndSet,
}

/// Checks the keys of the generation at `scope` against `set`: they are exactly the member
/// keys, and the set key when `expect` requires it, and no other key of the generation exists.
/// Keys of other generations are not this generation's and are ignored.
pub fn verify_keys(
    scope: &SetScope,
    set: &ReleaseSet,
    expect: Expect,
    keys: &[StoreKey],
) -> Result<(), VerifyError> {
    let bad = |e: &dyn fmt::Display| fail(Check::Keys, e.to_string());
    let set_key = StoreKey::set(scope).map_err(|e| bad(&e))?;
    // The set key is the prefix of the generation and `set`, so this is the prefix of every key
    // of the generation, up to and including the last `/`.
    let prefix = set_key.as_str().trim_end_matches("set");
    let mut wanted = BTreeSet::new();
    for member in &set.members {
        let name = member.manifest.clone().unwrap_or_default().name;
        wanted.insert(StoreKey::member(scope, &name).map_err(|e| bad(&e))?);
    }
    let present: BTreeSet<&StoreKey> = keys
        .iter()
        .filter(|k| k.as_str().starts_with(prefix))
        .collect();
    if let Some(key) = wanted.iter().find(|k| !present.contains(k)) {
        return Err(fail(
            Check::Keys,
            format!("the generation lacks the member key {key}"),
        ));
    }
    if expect == Expect::MembersAndSet && !present.contains(&set_key) {
        return Err(fail(
            Check::Keys,
            format!("the generation lacks the set document at {set_key}"),
        ));
    }
    match present
        .iter()
        .find(|k| !wanted.contains(**k) && **k != &set_key)
    {
        Some(extra) => Err(fail(
            Check::Keys,
            format!("the generation holds the key {extra}, which is no member of the set"),
        )),
        None => Ok(()),
    }
}
