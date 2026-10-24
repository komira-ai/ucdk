// SPDX-License-Identifier: Apache-2.0
//! The plan hash: what an approver pins so that apply can refuse a plan that is not the one
//! approved.

use std::fmt;

use sha2::{Digest as _, Sha256};

use crate::catalog::Lowered;
use crate::plan::{Action, Retained, Step};

const DOMAIN: &[u8] = b"ucdk/plan-hash/1\n";

/// SHA-256 over the desired graph and the ordered steps, in hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanHash(String);

impl PlanHash {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reads a hash from its text: 64 lower-case hex digits, as [`PlanHash::as_str`] writes
    /// it. This is how an approver's pinned hash comes back from a flag or a file.
    pub fn from_hex(text: &str) -> Option<Self> {
        let well_formed = text.len() == 64
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        well_formed.then(|| Self(text.to_owned()))
    }
}

impl fmt::Display for PlanHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn feed(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

fn feed_action(hash: &mut Sha256, action: &Action) {
    match action {
        Action::Create => hash.update([1]),
        Action::Update => hash.update([2]),
        Action::Noop => hash.update([3]),
        Action::Delete { physical } => {
            hash.update([4]);
            feed(hash, physical.as_str().as_bytes());
        }
        Action::Reap => hash.update([5]),
        Action::Retain(Retained::Keep) => hash.update([6, 1]),
        Action::Retain(Retained::Undeletable(reason)) => {
            hash.update([6, 2]);
            feed(hash, reason.as_bytes());
        }
        Action::Retain(Retained::OwnerNotNamed) => hash.update([6, 3]),
        Action::Converging => hash.update([7]),
    }
}

/// The hash covers, in order: a domain prefix; the provider-version slot (a tag, then the
/// version when there is one; reserved so that a provider upgrade can invalidate an approved
/// plan without changing the encoding); every node of the desired graph (path, kind, name,
/// wanted, value references, field digest, dependencies); and the ordered steps (path, kind,
/// action, and for a delete the physical id).
///
/// It does not cover the ledger token, nor the digest of what is live now: a step already
/// says what it will do about the difference.
pub fn compute(desired: &Lowered, steps: &[Step], provider_version: Option<&str>) -> PlanHash {
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    match provider_version {
        None => hash.update([0]),
        Some(version) => {
            hash.update([1]);
            feed(&mut hash, version.as_bytes());
        }
    }
    hash.update((desired.nodes().len() as u64).to_be_bytes());
    for n in desired.nodes() {
        feed(&mut hash, n.node.path().as_str().as_bytes());
        feed(&mut hash, n.node.kind().as_str().as_bytes());
        feed(&mut hash, n.node.name().as_bytes());
        hash.update([u8::from(n.wanted)]);
        hash.update((n.refs.len() as u64).to_be_bytes());
        for r in &n.refs {
            feed(&mut hash, r.field.as_bytes());
            feed(&mut hash, r.resource.as_bytes());
            feed(&mut hash, r.output.as_bytes());
        }
        feed(&mut hash, n.node.digest().as_str().as_bytes());
        hash.update((n.deps.len() as u64).to_be_bytes());
        for dep in &n.deps {
            feed(&mut hash, dep.as_str().as_bytes());
        }
    }
    hash.update((steps.len() as u64).to_be_bytes());
    for step in steps {
        feed(&mut hash, step.path.as_str().as_bytes());
        feed(&mut hash, step.kind.as_str().as_bytes());
        feed_action(&mut hash, &step.action);
    }
    PlanHash(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
