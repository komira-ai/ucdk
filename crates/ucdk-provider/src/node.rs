// SPDX-License-Identifier: Apache-2.0
//! The node: one desired object, as lowering produces it. It carries no proto types.

use sha2::{Digest as _, Sha256};

use crate::ids::{Digest, ModelError, NodePath};

/// The kind of object a node is. A provider decides which kinds it hosts; the first catalog
/// has `service` and `job`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Kind(String);

impl Kind {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn service() -> Self {
        Self::new("service")
    }

    pub fn job() -> Self {
        Self::new("job")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The value of a modelled field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    Text(String),
    Int(i64),
    Bool(bool),
}

/// One modelled field of a node: the part of the object the engine converges and compares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredField {
    pub name: String,
    pub value: FieldValue,
}

impl DesiredField {
    pub fn new(name: impl Into<String>, value: FieldValue) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }
}

/// One desired object. `name` is the physical name the object carries in the cloud, chosen
/// by the engine; `fields` are kept sorted by name, each name once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    path: NodePath,
    kind: Kind,
    name: String,
    fields: Vec<DesiredField>,
}

impl Node {
    pub fn new(
        path: NodePath,
        kind: Kind,
        name: impl Into<String>,
        mut fields: Vec<DesiredField>,
    ) -> Result<Self, ModelError> {
        fields.sort_by(|a, b| a.name.cmp(&b.name));
        if let Some(pair) = fields.windows(2).find(|w| w[0].name == w[1].name) {
            return Err(ModelError::DuplicateField(pair[0].name.clone()));
        }
        Ok(Self {
            path,
            kind,
            name: name.into(),
            fields,
        })
    }

    pub fn path(&self) -> &NodePath {
        &self.path
    }

    pub fn kind(&self) -> &Kind {
        &self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn fields(&self) -> &[DesiredField] {
        &self.fields
    }

    /// The digest of this node's kind and modelled fields.
    pub fn digest(&self) -> Digest {
        digest_of(&self.kind, &self.fields)
    }
}

fn feed(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
}

/// The digest of a kind and its fields (already sorted by name). It covers the kind, every
/// field name and every field value, each length-prefixed so no two inputs share an encoding.
/// It does not cover the physical name or anything the catalog does not model.
pub fn digest_of(kind: &Kind, fields: &[DesiredField]) -> Digest {
    let mut hash = Sha256::new();
    feed(&mut hash, kind.as_str().as_bytes());
    for field in fields {
        feed(&mut hash, field.name.as_bytes());
        match &field.value {
            FieldValue::Text(t) => {
                hash.update([1]);
                feed(&mut hash, t.as_bytes());
            }
            FieldValue::Int(i) => {
                hash.update([2]);
                hash.update(i.to_be_bytes());
            }
            FieldValue::Bool(b) => hash.update([3, u8::from(*b)]),
        }
    }
    Digest::from_bytes(&hash.finalize())
}
