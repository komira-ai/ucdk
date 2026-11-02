// SPDX-License-Identifier: Apache-2.0
//! The engine run over a ledger the test brings, and the records of any ledger decoded.

use super::*;

impl Rig {
    /// Like `handle`, over `ledger` instead of the rig's own: a ledger that lives in a file, say.
    pub fn handle_over(&mut self, ledger: &dyn Ledger, stage: &str, only: &[&str]) -> Outcome {
        let Self {
            machine,
            build,
            store,
            registry,
            pause,
            scratch_dir,
            stamp,
            attempt,
            run_id,
            revision,
            ttl,
            with_publisher,
            ..
        } = self;
        run_on(
            machine,
            ledger,
            build,
            store,
            registry,
            pause,
            scratch_dir,
            stamp,
            *attempt,
            run_id,
            revision,
            *ttl,
            *with_publisher,
            stage,
            only,
        )
    }
}

/// The release records of `ledger` with their states; a record that does not decode fails the
/// test.
pub fn decoded(ledger: &dyn Ledger) -> Vec<(codec::Record, State)> {
    ledger
        .read()
        .expect("read")
        .records
        .values()
        .map(|row| {
            let encoded = codec::Encoded {
                kind: row.kind.clone(),
                name: row.name.clone(),
                physical: row.physical.clone(),
                outputs: row.outputs.clone(),
            };
            (codec::Record::decode(&encoded).expect("decodes"), row.state)
        })
        .collect()
}
