// SPDX-License-Identifier: Apache-2.0
//! The stored object and how a stamp is written on it and read back.

use std::collections::BTreeMap;

use ucdk_provider::{DesiredField, Kind, NodeKey, OwnerLabel, Stamp, StampEvidence};

const OWNER: &str = "ucdk-owner";
const NODE: &str = "ucdk-node";
const SCHEME: &str = "ucdk-scheme";
const EXTRA: &str = "ucdk-x-";

/// A live object. `name` is the physical name as the cloud has it (stamp-encoded for a
/// tag-less kind); `base` is the human name without the encoding.
#[derive(Debug, Clone)]
pub struct Object {
    pub kind: Kind,
    pub name: String,
    pub base: String,
    pub labels: BTreeMap<String, String>,
    pub fields: Vec<DesiredField>,
    pub unmodelled: BTreeMap<String, String>,
    pub marker: Option<String>,
    /// The read count from which this object is visible (read lag).
    pub visible_at: u64,
}

/// The labels of a native tag, as written.
pub fn labels_of(stamp: &Stamp) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::from([
        (OWNER.to_owned(), stamp.owner.to_string()),
        (NODE.to_owned(), stamp.node.to_string()),
        (SCHEME.to_owned(), stamp.scheme.to_string()),
    ]);
    for (k, v) in &stamp.extra {
        labels.insert(format!("{EXTRA}{k}"), v.clone());
    }
    labels
}

/// The stamp a set of labels carries, when it carries a whole one.
pub fn stamp_of(labels: &BTreeMap<String, String>) -> Option<Stamp> {
    let owner = OwnerLabel::new(labels.get(OWNER)?).ok()?;
    let node = NodeKey::from_text(labels.get(NODE)?)?;
    let scheme = labels.get(SCHEME)?.parse().ok()?;
    let extra = labels
        .iter()
        .filter_map(|(k, v)| Some((k.strip_prefix(EXTRA)?.to_owned(), v.clone())))
        .collect();
    Some(Stamp {
        owner,
        node,
        scheme,
        extra,
    })
}

impl Object {
    /// What the object says about its owner. Another tool's marker wins; then the tag, or the
    /// name for a tag-less kind.
    pub fn evidence(&self, tagless: bool) -> StampEvidence {
        if let Some(marker) = &self.marker {
            return StampEvidence::OtherTool {
                marker: marker.clone(),
            };
        }
        let stamp = if tagless {
            Stamp::decode_name(&self.name).1
        } else {
            stamp_of(&self.labels)
        };
        stamp.map_or(StampEvidence::Absent, StampEvidence::Present)
    }
}
