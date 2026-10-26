// SPDX-License-Identifier: Apache-2.0
//! The records, the document they live in, and the compare-and-swap trait over it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use ucdk_core::{Locator, OwnerLabel, RecordPath, Tag};

use crate::lease::{Fence, Holding, Lease, LeaseError};

/// Who owns a record, as a path: every segment of the record's path above its last. For
/// `app/run` that is `app`; for a record nested one level down, `app/db/run`, it is `app/db`.
/// The first segment is the top-level owner. Recording the whole path, not one flat id, is
/// what lets nested owners be added without changing what a record means.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OwnerPath(Vec<OwnerLabel>);

impl OwnerPath {
    /// The owner path of the record at `path`. A path of one segment is its own owner.
    pub fn of(path: &RecordPath) -> Self {
        let segments: Vec<&str> = path.as_str().split('/').collect();
        let keep = segments.len().saturating_sub(1).max(1);
        Self(
            segments
                .into_iter()
                .take(keep)
                .map(|s| OwnerLabel::new(s).expect("a record path is made of valid segments"))
                .collect(),
        )
    }

    /// Every segment, outermost first. Never empty.
    pub fn segments(&self) -> &[OwnerLabel] {
        &self.0
    }

    /// The top-level owner.
    pub fn top(&self) -> &OwnerLabel {
        &self.0[0]
    }
}

impl fmt::Display for OwnerPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<&str> = self.0.iter().map(OwnerLabel::as_str).collect();
        f.write_str(&parts.join("/"))
    }
}

/// How the thing a record stands for came to be the record's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// This tool made it.
    Created,
    /// This tool took it over when it found it, without making it.
    Adopted,
}

/// Where a record stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Written before an effect: the effect may or may not have landed.
    Intent,
    /// The effect landed and matches what was last recorded.
    Confirmed,
    /// Written before the thing is removed: it may or may not be gone.
    Reaping,
}

/// What the ledger knows about one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub path: RecordPath,
    pub kind: Tag,
    /// The readable name of what the record stands for, so an intent whose effect never
    /// returned can be found again by name.
    pub name: String,
    pub owner: OwnerPath,
    pub origin: Origin,
    pub state: State,
    /// Where the thing is; `None` until an effect has returned or the thing has been found.
    pub physical: Option<Locator>,
    /// The records this one needed: the edges removal follows, in reverse.
    pub deps: BTreeSet<RecordPath>,
    /// The values the record carries, by key. A writer puts nothing secret here: the ledger
    /// is a plain text document.
    pub outputs: BTreeMap<String, String>,
}

impl Record {
    /// An intent for the record at `path`.
    pub fn intent(
        path: RecordPath,
        kind: Tag,
        name: impl Into<String>,
        deps: BTreeSet<RecordPath>,
    ) -> Self {
        Self {
            owner: OwnerPath::of(&path),
            path,
            kind,
            name: name.into(),
            origin: Origin::Created,
            state: State::Intent,
            physical: None,
            deps,
            outputs: BTreeMap::new(),
        }
    }

    /// The record, confirmed with the locator of what it stands for.
    #[must_use]
    pub fn confirmed(self, physical: Locator) -> Self {
        Self {
            state: State::Confirmed,
            physical: Some(physical),
            ..self
        }
    }

    /// The record in another state, everything else kept.
    #[must_use]
    pub fn in_state(self, state: State) -> Self {
        Self { state, ..self }
    }
}

/// The version of the document. A write names the token it read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token(pub(crate) u64);

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// The whole document at one version, with the lease state read in the same step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub token: Token,
    /// The highest fence ever granted; [`Fence::NONE`] when no lease was ever granted.
    pub fence: Fence,
    /// The lease as recorded, live or not; `None` when none is held.
    pub lease: Option<Holding>,
    /// The ledger's own clock reading, in milliseconds, taken with the read: what a
    /// [`Holding`] is judged against.
    pub at: u64,
    pub records: BTreeMap<RecordPath, Record>,
}

/// One change to the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Writes the record for its path, replacing any earlier one.
    Put(Record),
    /// Drops the record of a path; a path with none is left as it is.
    Remove(RecordPath),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    /// The document changed since the token was read: nothing was written.
    Stale,
    /// The ledger could not be reached: nothing was written.
    Unavailable(String),
    /// The fence is not the current lease's: a newer holder has been granted the lease, or the
    /// lease was released. Nothing was written, and the holder must stop.
    Fenced,
    /// The fence is the current lease's, but the lease has lapsed. Nothing was written; the
    /// holder acquires again, and gets a new fence, before it writes.
    LeaseLapsed,
    /// A write without a fence while another holder's lease is live. Nothing was written.
    Leased { holder: String },
    /// The ledger was opened to read and does not write. Nothing was written.
    ReadOnly,
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => f.write_str("the ledger changed since it was read"),
            Self::Unavailable(why) => write!(f, "the ledger is unavailable: {why}"),
            Self::Fenced => f.write_str(
                "the lease was taken over or released: this writer is fenced out of the ledger",
            ),
            Self::LeaseLapsed => f.write_str("the lease lapsed before this write"),
            Self::Leased { holder } => write!(f, "the ledger is leased to {holder}"),
            Self::ReadOnly => f.write_str("the ledger was opened read-only and cannot be written"),
        }
    }
}

impl std::error::Error for LedgerError {}

/// A ledger: one document with compare-and-swap writes and a lease. Methods take `&self` so that
/// two writers can hold one ledger; an implementation does its own locking.
///
/// A writer that takes part in the lease acquires it and writes with [`Ledger::write_fenced`];
/// the lease and its fence are in [`crate::lease`]. [`Ledger::write`] is for a document nobody
/// leases: it is refused while another holder's lease is live.
pub trait Ledger: Send + Sync {
    /// The document as it is now.
    fn read(&self) -> Result<Snapshot, LedgerError>;

    /// Applies `change` if the document is still at `expected` and no lease is live, and
    /// returns the new token.
    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError>;

    /// Applies `change` if the document is still at `expected` and `fence` is the live lease's.
    fn write_fenced(
        &self,
        fence: Fence,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError>;

    /// Grants the lease to `holder` for `ttl`, with a fence higher than every earlier grant's.
    /// It does not change the document's token.
    fn acquire(&self, holder: &str, ttl: Duration) -> Result<Lease, LeaseError>;

    /// Extends the lease of `fence` to `ttl` from now. Refused when the fence is not the current
    /// one or the lease has lapsed.
    fn renew(&self, fence: Fence, ttl: Duration) -> Result<Lease, LedgerError>;

    /// Gives the lease of `fence` back, so another holder may take it at once.
    fn release(&self, fence: Fence) -> Result<(), LedgerError>;
}
