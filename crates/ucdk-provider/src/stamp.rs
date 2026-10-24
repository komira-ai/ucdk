// SPDX-License-Identifier: Apache-2.0
//! The stamp: the ownership mark a provider writes in the create call.

use std::collections::BTreeMap;

use crate::ids::{NodeKey, NodePath, OwnerLabel};

/// The stamp scheme: scheme 1 identifies a node by its [`NodeKey`]. A change of scheme is a
/// migration, never a second form in use at once.
pub const SCHEME: u8 = 1;

const NAME_MARK: &str = "~ucdk";

/// Who owns an object and which node it is. `extra` is an open label map for values added
/// later (a validation run id, a composite depth) without changing what the stamp means; it
/// travels only where the kind can carry tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub owner: OwnerLabel,
    pub node: NodeKey,
    pub scheme: u8,
    pub extra: BTreeMap<String, String>,
}

impl Stamp {
    /// The stamp of the node at `path`, in the current scheme, with no extra labels.
    pub fn for_node(path: &NodePath) -> Self {
        Self {
            owner: path.owner(),
            node: path.key(),
            scheme: SCHEME,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// Whether two stamps name the same owner and node in the same scheme (extra labels are
    /// annotations and do not count).
    pub fn same_identity(&self, other: &Stamp) -> bool {
        self.owner == other.owner && self.node == other.node && self.scheme == other.scheme
    }

    /// The physical name that carries this stamp for a kind that cannot hold tags: the base
    /// name, then the mark, the scheme, the owner and the node key.
    pub fn encode_name(&self, base: &str) -> String {
        format!(
            "{base}{NAME_MARK}{}~{}~{}",
            self.scheme, self.owner, self.node
        )
    }

    /// Splits a physical name into its base name and the stamp it carries. A name that does
    /// not carry a well-formed stamp comes back whole, with no stamp.
    pub fn decode_name(name: &str) -> (String, Option<Stamp>) {
        let mut parts = name.rsplitn(3, '~');
        let decoded = (|| {
            let key = NodeKey::from_text(parts.next()?)?;
            let owner = OwnerLabel::new(parts.next()?).ok()?;
            let rest = parts.next()?;
            let (base, scheme) = rest.rsplit_once(NAME_MARK)?;
            let scheme = scheme.parse().ok()?;
            let stamp = Stamp {
                owner,
                node: key,
                scheme,
                extra: BTreeMap::new(),
            };
            Some((base.to_owned(), stamp))
        })();
        match decoded {
            Some((base, stamp)) => (base, Some(stamp)),
            None => (name.to_owned(), None),
        }
    }
}

/// Where a kind carries its stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampMode {
    /// A native tag, written in the create call.
    Tag,
    /// The kind cannot hold tags: the stamp is encoded in the physical name, written in the
    /// create call.
    NameEncoded,
}

/// What an observed object says about its owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampEvidence {
    /// The object carries a stamp (it may be ours or another cell's: the engine compares).
    Present(Stamp),
    /// The object carries none.
    Absent,
    /// Another tool's ownership marker, recognised by the provider; the engine holds no
    /// marker prefixes of its own.
    OtherTool { marker: String },
}
