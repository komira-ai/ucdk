// SPDX-License-Identifier: Apache-2.0
//! The mopm registry channel of a release.
//!
//! This crate is the base of a [`Publisher`](ucdk_release_seams::Publisher) for a mopm registry:
//! everything that is decided before a request is sent and everything that is decided from an
//! answer, each testable without a network. It holds
//!
//! - [`Config`]: the caller's settings (no default registry, one trust bundle named by a path,
//!   no environment variables) and the construction-time check that each call of the staged shape
//!   (stage, commit, abort) fits under both the token's life and the lease;
//! - [`Transport`], [`Request`] and [`Response`], the one I/O seam, with [`Secret`] and [`Url`]
//!   so that a bearer token or a presigned signature cannot be printed;
//! - [`check_key`], [`check_shape`], [`check_observe`] and [`members`]: the mapping of a ucdk
//!   coordinate onto mopm's package, version, file and subdirectory grammars, and the checks of a
//!   set, which refuse before any I/O whatever mopm would answer with a 400;
//! - [`map_reply`], [`map_transport`] and [`map_schema_violation`]: the classification of mopm's
//!   answers into [`SeamError`](ucdk_release_seams::SeamError);
//! - [`read_version`]: the first read, checked and then sent;
//! - [`group_id`], the answers' readers and [`Registry`]: the group read, the version read,
//!   `stage` and `abort` against a [`Transport`].
//!
//! The publisher itself and the HTTP adapter are not here yet.

mod answers;
mod config;
mod errors;
mod grammar;
mod group_id;
mod mapping;
mod platform;
mod registry;
mod requests;
mod transport;
mod url;

pub use answers::{
    Created, FileFact, Group, GroupMember, GroupStatus, MemberAnswer, RecordFile, Target,
    UploadFile, VersionRecord, VersionState, parse_created, parse_group, parse_record,
};

pub use config::{Call, Config, ConfigError, MAX_PAGE, MOPM_TOKEN_TTL, Proxy, Roots, RootsError};
pub use errors::{
    ConflictAnswer, ConflictReason, DEFAULT_RETRY_AFTER_SECONDS, Mapped, Op, error_code,
    error_reason, map_reply, map_schema_violation, map_transport,
};
pub use grammar::{
    MAX_FILE_NAME_LENGTH, MAX_REASON_UNITS, MAX_VERSION_LENGTH, build_of_file, safe_file_name,
    valid_build, valid_name, valid_reason, valid_sha256, valid_version,
};
pub use group_id::{group_id, valid_group_id};
pub use mapping::{
    ArchiveFacts, Candidate, Key, MAX_FILE_BYTES, MAX_GROUP_BYTES, MAX_MEMBERS, Member, MemberFile,
    check_key, check_observe, check_reason, check_shape, members,
};
pub use platform::{SUBDIRS_V1, Subdir};
pub use registry::{Caller, Registry, SetFile};
pub use requests::read_version;
pub use transport::{
    Header, HeaderValue, Method, Request, Response, Secret, Transport, TransportError,
    TransportKind,
};
pub use url::{Origin, Url, UrlError, encode_segment};
