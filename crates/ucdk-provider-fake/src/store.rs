// SPDX-License-Identifier: Apache-2.0
//! The stored object and how a stamp is written on it and read back.

use std::collections::BTreeMap;

use ucdk_provider::{
    CellIdentity, CellName, CellUid, DesiredField, Kind, NodeKey, OwnerLabel, Stamp, StampEvidence,
    Status,
};

const MACHINE: &str = "ucdk-machine";
const CELL: &str = "ucdk-cell";
const CELL_UID: &str = "ucdk-cell-uid";
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
    /// The name and labels as they were when the object was inserted: what it was born
    /// with. Later changes (an out-of-band label or rename) never touch these.
    pub born_name: String,
    pub born_labels: BTreeMap<String, String>,
    pub fields: Vec<DesiredField>,
    pub unmodelled: BTreeMap<String, String>,
    pub marker: Option<String>,
    /// The read count from which this object is visible (read lag).
    pub visible_at: u64,
    /// The status the object is in. `Converging` holds until the read count reaches
    /// `ready_at`, from which the object reads as `Ready`.
    pub status: Status,
    pub ready_at: u64,
}

/// The labels of a native tag, as written.
pub fn labels_of(stamp: &Stamp) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::from([
        (MACHINE.to_owned(), stamp.cell.machine().to_string()),
        (CELL.to_owned(), stamp.cell.cell().to_string()),
        (CELL_UID.to_owned(), stamp.cell.uid().to_string()),
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
    let cell = CellIdentity::new(
        CellName::new(labels.get(MACHINE)?).ok()?,
        CellName::new(labels.get(CELL)?).ok()?,
        CellUid::from_text(labels.get(CELL_UID)?).ok()?,
    );
    let owner = OwnerLabel::new(labels.get(OWNER)?).ok()?;
    let node = NodeKey::from_text(labels.get(NODE)?)?;
    let scheme = labels.get(SCHEME)?.parse().ok()?;
    let extra = labels
        .iter()
        .filter_map(|(k, v)| Some((k.strip_prefix(EXTRA)?.to_owned(), v.clone())))
        .collect();
    Some(Stamp {
        cell,
        owner,
        node,
        scheme,
        extra,
    })
}

impl Object {
    /// The status the object reports at the read numbered `read`.
    pub fn status_at(&self, read: u64) -> Status {
        match self.status {
            Status::Converging if read >= self.ready_at => Status::Ready,
            other => other,
        }
    }

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
