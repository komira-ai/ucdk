// SPDX-License-Identifier: Apache-2.0
//! What validation reports.

use std::fmt;

/// One problem in a resource list: which resource, which field, and what is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The resource's id (for a cycle, the first resource on it).
    pub resource: String,
    /// The field, such as `service.env.DB_URL` or `uses[0]`; empty when the whole resource is
    /// meant.
    pub field: String,
    pub kind: FindingKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindingKind {
    /// The id is not a single path segment (`[a-z][a-z0-9-]{0,31}`).
    BadId,
    /// Another resource of the list has this id.
    DuplicateId,
    /// The resource has neither a service nor a job.
    MissingBody,
    /// No image, or an image with no digest.
    MissingImage,
    /// A release parameter or a build step's output: both are resolved before the list is
    /// planned, so one still here has no value.
    Unresolved { what: &'static str },
    /// The image names a platform other than `linux/amd64`.
    UnsupportedPlatform { platform: String },
    /// A `Value` with none of its arms set.
    EmptyValue,
    /// A key is in both `env` and `secret_env`.
    EnvSecretOverlap,
    /// A reference to a resource that is not in the list.
    UnboundRef { target: String },
    /// A `Uses` with no target.
    NoTarget,
    /// A `Uses` access other than `CALL`.
    UnsupportedAccess,
    /// A `Uses` target that names one of the resource's values; access is to a resource.
    AccessToOutput,
    /// A value reference that names no output, or `OUTPUT_UNSET`, or a number the catalog does
    /// not know.
    MissingOutput,
    /// A value reference to a named output, which no v1 type produces.
    NamedOutput,
    /// A value reference to an output the target's type does not expose.
    UnexposedOutput {
        target: String,
        output: &'static str,
    },
    /// Resources that cannot be ordered because they reference each other. The members are
    /// the resources on a cycle and those that depend on one.
    Cycle { members: Vec<String> },
}

impl Finding {
    pub(crate) fn new(resource: &str, field: &str, kind: FindingKind) -> Self {
        Self {
            resource: resource.to_owned(),
            field: field.to_owned(),
            kind,
        }
    }
}

impl fmt::Display for FindingKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadId => write!(f, "the id is not a valid name"),
            Self::DuplicateId => write!(f, "another resource has the same id"),
            Self::MissingBody => write!(f, "the resource is neither a service nor a job"),
            Self::MissingImage => write!(f, "there is no image digest"),
            Self::Unresolved { what } => {
                write!(f, "a {what} was not resolved before planning")
            }
            Self::UnsupportedPlatform { platform } => {
                write!(
                    f,
                    "platform `{platform}` is not supported (only linux/amd64)"
                )
            }
            Self::EmptyValue => write!(f, "the value has no literal, parameter or reference"),
            Self::EnvSecretOverlap => write!(f, "the key is in both env and secret_env"),
            Self::UnboundRef { target } => write!(f, "`{target}` is not a resource of the list"),
            Self::NoTarget => write!(f, "the access names no target"),
            Self::UnsupportedAccess => write!(f, "the only access is CALL"),
            Self::AccessToOutput => write!(f, "access is to a resource, not to one of its values"),
            Self::MissingOutput => write!(f, "the reference names no known output"),
            Self::NamedOutput => write!(f, "no type produces a named output"),
            Self::UnexposedOutput { target, output } => {
                write!(f, "`{target}` does not expose {output}")
            }
            Self::Cycle { members } => {
                write!(
                    f,
                    "these resources cannot be ordered: {}",
                    members.join(", ")
                )
            }
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.field.is_empty() {
            write!(f, "resource `{}`: {}", self.resource, self.kind)
        } else {
            write!(
                f,
                "resource `{}`, {}: {}",
                self.resource, self.field, self.kind
            )
        }
    }
}
