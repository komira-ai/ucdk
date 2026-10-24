// SPDX-License-Identifier: Apache-2.0
//! The identity of a cell: which machine runs it, what it is called, and the random id minted
//! when it was bootstrapped. Every stamp carries it, so objects of two cells never read as
//! one cell's.

use std::fmt;
use std::sync::Arc;

use crate::ids::{ModelError, key_text_is_well_formed, valid_segment};

/// A name in a cell's identity (a machine name or a cell name): `[a-z][a-z0-9-]{0,31}`, the
/// same shape as a path segment, so it fits in any label a provider offers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellName(String);

impl CellName {
    pub fn new(text: &str) -> Result<Self, ModelError> {
        if valid_segment(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(ModelError::BadSegment(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CellName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The random id of a cell, minted once when the cell is bootstrapped: 26 lowercase base32
/// characters (128 bits), the same text form as a node key. A cell that is deleted and created
/// again under the same names has a new id, so its objects are never taken for the old
/// cell's.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellUid(String);

impl CellUid {
    /// Reads an id back from its text: the shape is checked, nothing else.
    pub fn from_text(text: &str) -> Result<Self, ModelError> {
        if key_text_is_well_formed(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(ModelError::BadCellUid(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CellUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which cell an object belongs to: the machine, the cell and the cell's id. Two identities
/// are the same cell only if all three are equal. Cloning is cheap (the parts are shared),
/// as every stamp and scope carries one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellIdentity(Arc<Parts>);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Parts {
    machine: CellName,
    cell: CellName,
    uid: CellUid,
}

impl CellIdentity {
    pub fn new(machine: CellName, cell: CellName, uid: CellUid) -> Self {
        Self(Arc::new(Parts { machine, cell, uid }))
    }

    /// The machine the cell runs on.
    pub fn machine(&self) -> &CellName {
        &self.0.machine
    }

    /// The cell's name.
    pub fn cell(&self) -> &CellName {
        &self.0.cell
    }

    /// The cell's id.
    pub fn uid(&self) -> &CellUid {
        &self.0.uid
    }
}

impl fmt::Display for CellIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.0.machine, self.0.cell, self.0.uid)
    }
}
