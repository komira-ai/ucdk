// SPDX-License-Identifier: Apache-2.0
//! A ledger snapshot as the `ucdk.api.v1` messages that show it. Lists, never maps: records are
//! in path order and outputs in key order, so every host prints the same bytes.

use ucdk_ledger::{Holding, LeaseState, Origin, Record, Snapshot, State};

use crate::v1;

fn lease_state(state: LeaseState) -> v1::LeaseState {
    match state {
        LeaseState::Live => v1::LeaseState::Live,
        LeaseState::Lapsing => v1::LeaseState::Lapsing,
        LeaseState::Lapsed => v1::LeaseState::Lapsed,
    }
}

fn record_origin(origin: Origin) -> v1::RecordOrigin {
    match origin {
        Origin::Created => v1::RecordOrigin::Created,
        Origin::Adopted => v1::RecordOrigin::Adopted,
    }
}

fn record_state(state: State) -> v1::RecordState {
    match state {
        State::Intent => v1::RecordState::Intent,
        State::Confirmed => v1::RecordState::Confirmed,
        State::Reaping => v1::RecordState::Reaping,
    }
}

/// The lease as `holder` sees it at `at`, a reading of the ledger's clock.
fn lease_view(holding: &Holding, at: u64, holder: &str) -> v1::LeaseView {
    v1::LeaseView {
        holder: holding.holder.clone(),
        expires_ms: holding.expires_at,
        state: lease_state(holding.state(at)) as i32,
        yours: holding.holder == holder,
    }
}

fn record(record: &Record) -> v1::LedgerRecord {
    v1::LedgerRecord {
        path: record.path.to_string(),
        kind: record.kind.to_string(),
        name: record.name.clone(),
        origin: record_origin(record.origin) as i32,
        state: record_state(record.state) as i32,
        physical: record
            .physical
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        deps: record.deps.iter().map(ToString::to_string).collect(),
        outputs: record
            .outputs
            .iter()
            .map(|(key, value)| v1::Pair {
                key: key.clone(),
                value: value.clone(),
            })
            .collect(),
    }
}

/// The whole document, with the lease judged at the snapshot's own reading of the clock.
pub(crate) fn response(snapshot: &Snapshot, holder: &str) -> v1::LedgerShowResponse {
    v1::LedgerShowResponse {
        token: snapshot.token.to_string(),
        fence: snapshot.fence.value(),
        lease: snapshot
            .lease
            .as_ref()
            .map(|holding| lease_view(holding, snapshot.at, holder)),
        records: snapshot.records.values().map(record).collect(),
    }
}
