// SPDX-License-Identifier: Apache-2.0
//! The stamp: the ownership mark a provider writes in the create call.

use std::collections::BTreeMap;

use crate::cell::{CellIdentity, CellName, CellUid};
use crate::ids::{NodeKey, NodePath, OwnerLabel};

/// The stamp scheme: scheme 1 identifies a node by its [`NodeKey`] and its cell by the
/// machine, the cell name and the cell id. The byte reaches cloud tags, names and the ledger,
/// so it is a format promise. A change of scheme is a migration, never a second form in use
/// at once.
pub const SCHEME: u8 = 1;

const NAME_MARK: &str = "~ucdk";

/// Which cell owns an object, who owns it within the cell and which node it is. `extra` is an open label map for values added
/// later (a validation run id, a composite depth) without changing what the stamp means; it
/// travels only where the kind can carry tags ([`StampMode::carries_extra`]); a create that asks a
/// kind without tags to carry any is refused, never silently stripped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub cell: CellIdentity,
    pub owner: OwnerLabel,
    pub node: NodeKey,
    pub scheme: u8,
    pub extra: BTreeMap<String, String>,
}

impl Stamp {
    /// The stamp of the node at `path` in `cell`, in the current scheme, with no extra labels.
    pub fn for_node(cell: &CellIdentity, path: &NodePath) -> Self {
        Self {
            cell: cell.clone(),
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

    /// Whether two stamps name the same cell, owner and node in the same scheme (extra labels
    /// are annotations and do not count).
    pub fn same_identity(&self, other: &Stamp) -> bool {
        self.cell == other.cell
            && self.owner == other.owner
            && self.node == other.node
            && self.scheme == other.scheme
    }

    /// The physical name that carries this stamp for a kind that cannot hold tags: the base
    /// name, then the mark, the scheme, the machine, the cell name, the cell id, the owner and
    /// the node key.
    pub fn encode_name(&self, base: &str) -> String {
        format!(
            "{base}{NAME_MARK}{}~{}~{}~{}~{}~{}",
            self.scheme,
            self.cell.machine(),
            self.cell.cell(),
            self.cell.uid(),
            self.owner,
            self.node
        )
    }

    /// Splits a physical name into its base name and the stamp it carries. A name that does
    /// not carry a well-formed stamp comes back whole, with no stamp.
    pub fn decode_name(name: &str) -> (String, Option<Stamp>) {
        let mut parts = name.rsplitn(6, '~');
        let decoded = (|| {
            let key = NodeKey::from_text(parts.next()?)?;
            let owner = OwnerLabel::new(parts.next()?).ok()?;
            let uid = CellUid::from_text(parts.next()?).ok()?;
            let cell = CellName::new(parts.next()?).ok()?;
            let machine = CellName::new(parts.next()?).ok()?;
            let rest = parts.next()?;
            let (base, scheme) = rest.rsplit_once(NAME_MARK)?;
            let scheme = scheme.parse().ok()?;
            let stamp = Stamp {
                cell: CellIdentity::new(machine, cell, uid),
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

/// Where a kind carries its stamp, and so whether it can carry the stamp's extra labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StampMode {
    /// A native tag, written in the create call. The extra labels are carried too: `observe`
    /// and `list_owned` return them exactly as written.
    Tag,
    /// The kind cannot hold tags: the stamp is encoded in the physical name, written in the
    /// create call. The name has room for the owner and node key only, so the extra labels
    /// are not carried, and a create with a non-empty `Stamp::extra` is
    /// `Refused(Unsupported)` and changes nothing. A provider never drops extra labels
    /// silently.
    NameEncoded,
    /// The object carries no stamp at all: its owner is the ledger record, not a mark on the
    /// object. It carries no extra labels. Declared now and not yet applied: a run whose file
    /// holds a node of such a kind is refused ([`StampMode::applicable`]), and a provider
    /// refuses to create one.
    Embedded,
}

impl StampMode {
    /// Whether a kind in this mode carries `Stamp::extra`.
    pub fn carries_extra(self) -> bool {
        match self {
            Self::Tag => true,
            Self::NameEncoded | Self::Embedded => false,
        }
    }

    /// Whether the engine can apply kinds in this mode today.
    pub fn applicable(self) -> bool {
        match self {
            Self::Tag | Self::NameEncoded => true,
            Self::Embedded => false,
        }
    }
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
