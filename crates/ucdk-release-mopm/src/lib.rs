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
//! - [`Envelope`], [`authorise`]: the grant's secret and the client-side checks of a grant
//!   (registry, phase, coverage, expiry) that send nothing when they refuse;
//! - [`MopmGrantSource`]: the exchange of the job's CI identity token for a short-lived grant,
//!   with [`GithubIdentity`] as the source of the token.
//!
//! The publisher itself, the group verbs and the HTTP adapter are not here yet.

mod config;
mod errors;
mod exchange;
mod grammar;
mod grants;
mod identity;
mod mapping;
mod platform;
mod requests;
mod transport;
mod url;

pub use config::{Call, Config, ConfigError, MAX_PAGE, MOPM_TOKEN_TTL, Proxy, Roots, RootsError};
pub use errors::{
    ConflictAnswer, ConflictReason, DEFAULT_RETRY_AFTER_SECONDS, Mapped, Op, error_code,
    error_reason, map_reply, map_schema_violation, map_transport,
};
pub use exchange::{ActorForm, GITHUB_ISSUER, GRANT_LIMIT, GrantSourceConfig, MopmGrantSource};
pub use grammar::{
    MAX_FILE_NAME_LENGTH, MAX_REASON_UNITS, MAX_VERSION_LENGTH, build_of_file, safe_file_name,
    valid_build, valid_name, valid_reason, valid_sha256, valid_version,
};
pub use grants::{Envelope, Malformed, Use, Verb, authorise};
pub use identity::{
    Claims, Expected, GithubIdentity, IdentitySource, IdentityToken, IdentityUnavailable, Mismatch,
    Verified, check_identity,
};
pub use mapping::{
    ArchiveFacts, Candidate, Key, MAX_FILE_BYTES, MAX_GROUP_BYTES, MAX_MEMBERS, Member, MemberFile,
    check_key, check_observe, check_reason, check_shape, members,
};
pub use platform::{SUBDIRS_V1, Subdir};
pub use requests::read_version;
pub use transport::{
    Header, HeaderValue, Method, Request, Response, Secret, Transport, TransportError,
    TransportKind,
};
pub use url::{Origin, Url, UrlError, encode_segment};
