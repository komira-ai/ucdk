// SPDX-License-Identifier: Apache-2.0
//! The records of a run in its ledger, read and written through the codec of `ucdk-release`.
//! The ledger holds a plain record per path; the codec says which paths and payload keys a
//! release record has, and refuses what it does not know.

use std::collections::BTreeSet;

use ucdk_ledger::{Origin, OwnerPath, Record as Row, Snapshot, State};
use ucdk_release::codec::{self, CodecError, Encoded};
use ucdk_release::transitions::Kind;

/// Every release record of a ledger, decoded, with the state the ledger holds it in.
pub(crate) struct Records(Vec<(codec::Record, State)>);

impl Records {
    /// Decodes the release records of `snapshot`. A record of a tag that is not a release kind
    /// belongs to something else and is left alone; a release record the codec refuses is an
    /// error.
    pub(crate) fn decode(snapshot: &Snapshot) -> Result<Self, CodecError> {
        snapshot
            .records
            .values()
            .filter(|row| Kind::from_tag(row.kind.as_str()).is_some())
            .map(|row| {
                let encoded = Encoded {
                    kind: row.kind.clone(),
                    name: row.name.clone(),
                    physical: row.physical.clone(),
                    outputs: row.outputs.clone(),
                };
                codec::Record::decode(&encoded).map(|record| (record, row.state))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }

    /// The run record, and its state.
    pub(crate) fn run(&self) -> Option<(&codec::Run, State)> {
        self.0.iter().find_map(|(record, state)| match record {
            codec::Record::Run(run) => Some((run, *state)),
            _ => None,
        })
    }

    /// The record of the stage `stage`.
    pub(crate) fn stage(&self, stage: &str) -> Option<&codec::Stage> {
        self.0.iter().find_map(|(record, _)| match record {
            codec::Record::Stage(found) if found.stage == stage => Some(found),
            _ => None,
        })
    }

    /// The record of the action `action` of `stage`, and its state.
    pub(crate) fn action(&self, stage: &str, action: &str) -> Option<(&codec::Action, State)> {
        self.0.iter().find_map(|(record, state)| match record {
            codec::Record::Action(found)
                if (found.stage.as_str(), found.action.as_str()) == (stage, action) =>
            {
                Some((found, *state))
            }
            _ => None,
        })
    }

    /// The record of the validation `name` of `stage`, and its state.
    pub(crate) fn validation(
        &self,
        stage: &str,
        name: &str,
    ) -> Option<(&codec::Validation, State)> {
        self.0.iter().find_map(|(record, state)| match record {
            codec::Record::Validation(found)
                if (found.stage.as_str(), found.validation.as_str()) == (stage, name) =>
            {
                Some((found, *state))
            }
            _ => None,
        })
    }
}

/// The ledger row of `record` in `state`.
pub(crate) fn row(record: &codec::Record, state: State) -> Result<Row, CodecError> {
    let encoded = record.encode()?;
    let path = encoded
        .path()
        .expect("a release tag and a key segment make a record path");
    Ok(Row {
        owner: OwnerPath::of(&path),
        path,
        kind: encoded.kind,
        name: encoded.name,
        origin: Origin::Created,
        state,
        physical: None,
        deps: BTreeSet::new(),
        outputs: encoded.outputs,
    })
}
