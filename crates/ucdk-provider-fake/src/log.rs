// SPDX-License-Identifier: Apache-2.0
//! The ordered call log: what the store did, call by call.

use std::collections::BTreeMap;

use ucdk_provider::{Kind, PhysicalId, ProviderError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Create,
    Update,
    Delete,
    Observe,
    ListOwned,
}

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Observe => "observe",
            Self::ListOwned => "list_owned",
        }
    }

    /// Whether the call changes the store.
    pub fn mutates(self) -> bool {
        matches!(self, Self::Create | Self::Update | Self::Delete)
    }
}

/// One call. `name` and `labels` are as written by a create; `outcome` is what the store
/// did (the caller may have been told otherwise, as with an ambiguous success).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub op: Op,
    pub kind: Option<Kind>,
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub physical: Option<PhysicalId>,
    pub outcome: Result<(), ProviderError>,
}
