// SPDX-License-Identifier: Apache-2.0
//! The fake's constructor data: which kinds it hosts and how, and the faults to inject.

use std::collections::BTreeSet;

use ucdk_provider::{Kind, ProviderError};

/// An object that is in the cloud before the fake is used and that this cell did not create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignObject {
    pub kind: Kind,
    pub name: String,
    /// Another tool's ownership marker on the object, when it carries one; otherwise the
    /// object carries no stamp at all.
    pub marker: Option<String>,
}

/// The faults to inject. The default injects none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Faults {
    /// The `k`th mutation (create, update or delete, counted from 1) is refused once, and
    /// changes nothing. The calls after it proceed.
    pub fail_at_call: Option<u64>,
    /// Reads are eventually consistent, for `n` reads after the change. A created object is
    /// hidden from the next `n` reads (observe or list); a deleted object is still reported
    /// by the next `n` `observe` reads, which see it as present with its old state.
    /// `list_owned` lags for a create exactly as `observe` does, but after a delete it
    /// tells the truth at once. A read that fails by `fail_reads` still counts.
    pub read_lag: u64,
    /// Objects present from the start.
    pub foreign: Vec<ForeignObject>,
    /// Before the next create lands, another client creates the same object with the same
    /// stamp, so the create loses and is refused as `AlreadyExists`. Fires once.
    pub race_next: bool,
    /// The next mutation is applied and then reports a timeout (`Unknown`). Fires once.
    pub ambiguous_success: bool,
    /// The `k`th mutation (counted from 1, as for `fail_at_call`) is applied if it would
    /// succeed and then reports a timeout (`Unknown`). Fires once, for that `k` only.
    pub ambiguous_at_call: Option<u64>,
    /// The next reads (observe or list_owned), in order, each fail with the next error
    /// here and change nothing; once the list is used up reads succeed again. Each failed
    /// read is logged with its error.
    pub fail_reads: Vec<ProviderError>,
}

/// What the fake hosts. Every kind not listed as unsupported is hosted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FakeConfig {
    /// Kinds that cannot hold tags: their stamp is encoded in the physical name.
    pub tagless: BTreeSet<Kind>,
    /// Kinds whose objects carry data (state class `Stateful`).
    pub stateful: BTreeSet<Kind>,
    /// Kinds whose objects cannot be deleted while they hold fields the catalog does not
    /// model.
    pub deletion_forbidden: BTreeSet<Kind>,
    /// Kinds the cloud lacks: a create of one is refused before anything is created.
    pub unsupported: BTreeSet<Kind>,
    pub faults: Faults,
}

impl FakeConfig {
    /// `service` carries a native tag; `job` cannot, so it is name-encoded.
    pub fn standard() -> Self {
        Self {
            tagless: BTreeSet::from([Kind::job()]),
            ..Self::default()
        }
    }

    /// The standard shapes, on a cloud that has no `job`.
    pub fn limited() -> Self {
        Self {
            unsupported: BTreeSet::from([Kind::job()]),
            ..Self::standard()
        }
    }
}
