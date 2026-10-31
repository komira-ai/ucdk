// SPDX-License-Identifier: Apache-2.0
//! The crash sweep: a Test action is run on a ledger that dies at its `n`th call, for every `n`
//! a clean run makes, and then run again as the next attempt of the same run. Whatever call the
//! process died at, the rerun ends with the action confirmed, the records decode, nothing the
//! engine made is left behind, and the test ran at most twice.

mod common;

use common::Rig;
use ucdk_ledger::State;
use ucdk_release_proto::v1::Verdict;

const ONLY: [&str; 1] = ["action:test"];

#[test]
fn test_action_every_n() {
    // The calls of a clean run.
    let mut clean = Rig::new("sweep_clean");
    assert_eq!(clean.handle("build", &ONLY).verdict, Verdict::Succeeded);
    let calls = clean.ledger.calls();
    assert!(
        calls > 8,
        "a run reads, writes, renews and releases: {calls}"
    );

    for n in 1..=calls {
        let mut rig = Rig::new(&format!("sweep_{n}"));
        rig.ledger.crash_at(n);
        let crashed = rig.handle("build", &ONLY);
        // Only the last call, the release of the lease, comes after everything the run did.
        assert!(
            n == calls || crashed.verdict != Verdict::Succeeded,
            "a run whose ledger died at call {n} of {calls} did not succeed"
        );
        let tested = rig.build.commands().len();

        // The process is back. The next attempt is the same holder, so it takes the lease at
        // once even when the dead one never gave it back.
        rig.ledger.revive();
        rig.attempt = 2;
        let again = rig.handle("build", &ONLY);
        assert!(
            matches!(again.verdict, Verdict::Succeeded | Verdict::Noop),
            "after a death at call {n}: {:?} {}",
            again.verdict,
            again.message
        );
        let (_, state) = rig
            .ledger
            .action("build", "test")
            .expect("the action record");
        assert_eq!(state, State::Confirmed, "death at call {n}");
        let (_, state) = rig.ledger.run_record().expect("the run record");
        assert_eq!(state, State::Confirmed, "death at call {n}");
        let total = rig.build.commands().len();
        assert!(
            (1..=2).contains(&total) && total >= tested,
            "death at call {n}: the test ran {tested} time(s), then {total} in all"
        );
        assert_eq!(
            rig.scratch.entries(),
            Vec::<String>::new(),
            "death at call {n}"
        );
        assert_eq!(rig.ledger.lease(), None, "death at call {n}");
    }
}
