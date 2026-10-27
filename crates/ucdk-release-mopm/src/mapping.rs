// SPDX-License-Identifier: Apache-2.0
//! The mapping of ucdk coordinates onto mopm members, and the checks of a set that run before
//! any request is built.
//!
//! A ucdk [`Coordinate`] is `{channel, type, name, version, platform}`; mopm's identity is
//! `(name, version)` with one file per subdirectory. The mapping is injective by construction:
//! name, version and platform are each checked against an exact grammar (which cannot hold a
//! `/` or any separator), nothing is normalised, and no field is joined to another with a
//! separator to compare or key anything. Sets are keyed by owned fields in a `BTreeMap`.
//!
//! A coordinate mopm cannot hold is `Refused(Channel)`: nothing happened and the same call
//! fails again. That holds for every verb, `observe` included, so a name mopm would answer with a
//! 400 is never read as `Transient` and retried.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_release_proto::v1::{ArtifactType, Coordinate, Reason};
use ucdk_release_seams::SeamError;

use crate::grammar::{build_of_file, valid_name, valid_reason, valid_sha256, valid_version};
use crate::platform::Subdir;

/// Members in one group.
pub const MAX_MEMBERS: usize = 500;
/// Bytes in one file (decimal, as mopm counts).
pub const MAX_FILE_BYTES: u64 = 100_000_000;
/// Bytes in one group (decimal, as mopm counts).
pub const MAX_GROUP_BYTES: u64 = 20_000_000_000;

fn refuse() -> SeamError {
    SeamError::Refused(Reason::Channel)
}

fn permanent(message: String) -> SeamError {
    SeamError::Permanent(message)
}

/// The mopm identity of a coordinate: its package, version and subdirectory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub name: String,
    pub version: String,
    pub subdir: Subdir,
}

/// The mopm key of `c`, or `Refused(Channel)` when the coordinate is of another channel or type,
/// or its name, version or platform is outside what mopm holds.
pub fn check_key(channel: &str, c: &Coordinate) -> Result<Key, SeamError> {
    let holds = c.channel == channel
        && c.r#type == ArtifactType::Conda as i32
        && valid_name(&c.name)
        && valid_version(&c.version);
    match Subdir::of_platform(&c.platform) {
        Some(subdir) if holds => Ok(Key {
            name: c.name.clone(),
            version: c.version.clone(),
            subdir,
        }),
        _ => Err(refuse()),
    }
}

/// The set-wide checks, with no I/O: every coordinate is one mopm can hold, none is named twice
/// (`Permanent`), no package name has two versions (`Refused`: neither a group nor a token can
/// cover it) and there are no more than [`MAX_MEMBERS`] packages. Returns the keys in order.
///
/// Two coordinates are the same exactly when their keys are: after the checks the channel and
/// type are fixed, and the platform maps to its subdirectory one to one.
pub fn check_shape(channel: &str, set: &[&Coordinate]) -> Result<Vec<Key>, SeamError> {
    let keys = set
        .iter()
        .map(|c| check_key(channel, c))
        .collect::<Result<Vec<_>, _>>()?;
    let mut seen = BTreeSet::new();
    if !keys.iter().all(|k| seen.insert(k)) {
        return Err(permanent("a coordinate is named twice".to_owned()));
    }
    let mut versions: BTreeMap<&str, &str> = BTreeMap::new();
    for key in &keys {
        let earlier = versions.insert(&key.name, &key.version);
        if earlier.is_some_and(|v| v != key.version) {
            return Err(refuse());
        }
    }
    if versions.len() > MAX_MEMBERS {
        return Err(refuse());
    }
    Ok(keys)
}

/// What `observe` checks before it reads: `k` is a coordinate mopm can hold, the declared `set`
/// passes [`check_shape`], and a non-empty `set` holds `k`.
pub fn check_observe(channel: &str, k: &Coordinate, set: &[&Coordinate]) -> Result<Key, SeamError> {
    let key = check_key(channel, k)?;
    check_shape(channel, set)?;
    if !set.is_empty() && !set.contains(&k) {
        return Err(refuse());
    }
    Ok(key)
}

/// A yank reason mopm accepts, else `Refused(Channel)`.
pub fn check_reason(reason: &str) -> Result<(), SeamError> {
    valid_reason(reason).then_some(()).ok_or_else(refuse)
}

/// What a package archive says about itself, read from `info/index.json`, the manifest
/// `lib/mojo/<name>.mopm.json` and the native library it names. The extraction of these facts
/// from the archive is not in this crate yet; the checks of them are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveFacts {
    /// `name` of `info/index.json`: `mojo-<name>`.
    pub index_name: String,
    pub index_version: String,
    pub index_subdir: String,
    pub index_build: String,
    /// `name` of the manifest.
    pub manifest_name: String,
    /// `version` of the manifest, when it has one.
    pub manifest_version: Option<String>,
    /// The SHA-256 of the raw manifest bytes, lower-case hex.
    pub manifest_sha256: String,
    /// The native library (id and soname) the manifest declares, if any.
    pub native_library: Option<String>,
}

/// One file of a set, as the publisher reads it before sending anything.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    pub coordinate: &'a Coordinate,
    /// The blob's name: `mojo-<name>-<version>-<build>.conda`.
    pub file_name: &'a str,
    /// The length of the file in bytes.
    pub size: u64,
    /// The SHA-256 of the file, lower-case hex.
    pub sha256: &'a str,
    pub archive: &'a ArchiveFacts,
}

/// A candidate placed in its member: its subdirectory and the build its file name carries.
struct Placed<'c, 'a> {
    subdir: Subdir,
    candidate: &'c Candidate<'a>,
    build: &'a str,
}

/// One file of a member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberFile {
    pub subdir: Subdir,
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
}

/// One package of a group: every platform file of one `(name, version)`, which mopm checks
/// against one manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub version: String,
    pub build: String,
    /// The manifest hash every archive of the member agrees on.
    pub manifest_sha256: String,
    /// Sorted by subdirectory.
    pub files: Vec<MemberFile>,
}

/// The members of a set, sorted by name, or the refusal. Everything mopm would reject is found
/// here, so that no request is built and no daily allowance is spent on a group that cannot
/// succeed. In order: the set-wide checks ([`check_shape`]); then per file in the given order
/// (the file name form, a size of 1 to [`MAX_FILE_BYTES`], the digest, the archive against the
/// coordinate); then the group total against [`MAX_GROUP_BYTES`]; then that the archives of each
/// member agree on one manifest, one build and one native library.
///
/// `Refused(Channel)`: a file name that is not `mojo-<name>-<version>-<build>.conda`, a file or a
/// group over its limit. `Permanent`: a coordinate named twice, an empty file, a malformed
/// digest, an archive that disagrees with its coordinate or with its siblings.
pub fn members(channel: &str, set: &[Candidate<'_>]) -> Result<Vec<Member>, SeamError> {
    let coordinates: Vec<&Coordinate> = set.iter().map(|c| c.coordinate).collect();
    let keys = check_shape(channel, &coordinates)?;
    let mut grouped: BTreeMap<(String, String), Vec<Placed<'_, '_>>> = BTreeMap::new();
    let mut total: u64 = 0;
    for (key, c) in keys.into_iter().zip(set) {
        let build = build_of_file(c.file_name, &key.name, &key.version).ok_or_else(refuse)?;
        if c.size == 0 {
            return Err(permanent(format!("{}: the file is empty", c.file_name)));
        }
        if c.size > MAX_FILE_BYTES {
            return Err(refuse());
        }
        if !valid_sha256(c.sha256) {
            return Err(permanent(format!("{}: malformed sha256", c.file_name)));
        }
        let a = c.archive;
        let agrees = [
            (
                "info/index.json name",
                a.index_name == format!("mojo-{}", key.name),
            ),
            ("info/index.json version", a.index_version == key.version),
            (
                "info/index.json subdir",
                a.index_subdir == key.subdir.as_str(),
            ),
            ("info/index.json build", a.index_build == build),
            ("manifest name", a.manifest_name == key.name),
            (
                "manifest version",
                a.manifest_version
                    .as_deref()
                    .is_none_or(|v| v == key.version),
            ),
        ];
        if let Some((what, _)) = agrees.iter().find(|(_, ok)| !ok) {
            return Err(permanent(format!(
                "{}: the archive's {what} disagrees with the coordinate",
                c.file_name
            )));
        }
        total = total.saturating_add(c.size);
        grouped
            .entry((key.name, key.version))
            .or_default()
            .push(Placed {
                subdir: key.subdir,
                candidate: c,
                build,
            });
    }
    if total > MAX_GROUP_BYTES {
        return Err(refuse());
    }
    grouped
        .into_iter()
        .map(|((name, version), mut files)| {
            files.sort_by_key(|f| f.subdir);
            let (first, build) = (files[0].candidate, files[0].build);
            let one_manifest = files.iter().all(|f| {
                f.candidate.archive.manifest_sha256 == first.archive.manifest_sha256
                    && f.candidate.archive.native_library == first.archive.native_library
                    && f.build == build
            });
            if !one_manifest {
                return Err(permanent(format!(
                    "mojo-{name}-{version}: the archives do not agree on one manifest, build and native library"
                )));
            }
            Ok(Member {
                build: build.to_owned(),
                manifest_sha256: first.archive.manifest_sha256.clone(),
                files: files
                    .iter()
                    .map(|f| MemberFile {
                        subdir: f.subdir,
                        file_name: f.candidate.file_name.to_owned(),
                        size: f.candidate.size,
                        sha256: f.candidate.sha256.to_owned(),
                    })
                    .collect(),
                name,
                version,
            })
        })
        .collect()
}
