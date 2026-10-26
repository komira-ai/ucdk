// SPDX-License-Identifier: Apache-2.0
//! The leak sweep: objects that carry this cell's stamp and that no ledger record accounts for
//! are reported, and nothing is ever deleted on that evidence. Each scenario runs on the
//! in-memory ledger and on the file ledger.

mod common;

use common::{
    AnyLedger, Backend, FlakyLedger, Hooked, apply, apply_with, cell, chain, destroy, done,
    lowered, mutations, objects, path, physical_of, records, service_of,
};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ucdk_engine::{Engine, EngineError, Leak, Outcome, Report, Request, State};
use ucdk_ledger::{
    Change, Fence, Lease, LeaseError, Ledger as _, LedgerError, Record, Snapshot, Token,
};
use ucdk_provider::{
    CellIdentity, CellName, CellUid, Kind, Node, PhysicalId, Provider as _, ProviderError, Stamp,
    kit,
};
use ucdk_provider_fake::{Fake, Op};

fn sweep(fake: &mut Fake, ledger: &dyn ucdk_ledger::Ledger) -> Outcome {
    Engine::new(fake, ledger, cell()).handle(Request::Leaks)
}

fn leaks_of(outcome: Outcome) -> Vec<Leak> {
    match outcome {
        Outcome::Leaks(leaks) => leaks,
        other => panic!("not a sweep: {other:?}"),
    }
}

/// Creates an object of the service `id` in `fake` that carries the stamp of `in_cell`, as a
/// run of that cell would have, and returns its physical id.
fn stamped(fake: &mut Fake, id: &str, in_cell: &CellIdentity) -> PhysicalId {
    let list = lowered(vec![service_of(id, 80, false, &[])]);
    let node = list
        .nodes()
        .iter()
        .find(|n| n.node.path().as_str().ends_with("/run"))
        .expect("a run node")
        .node
        .clone();
    let stamp = Stamp::for_node(in_cell, node.path());
    fake.create(&node, &stamp).expect("create").physical
}

fn applied(backend: Backend) -> (Fake, AnyLedger) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    done(apply(&mut fake, &ledger, &chain()));
    (fake, ledger)
}

/// Proves: an object that carries this cell's stamp and has no ledger record is reported, with
/// its kind, physical id, name, owner and node; the recorded objects beside it are not. Catches
/// a sweep that reports nothing, or everything.
fn an_object_we_stamped_with_no_record_is_a_leak(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    let ghost = stamped(&mut fake, "ghost", &cell());
    let leaks = leaks_of(sweep(&mut fake, &ledger));
    assert_eq!(leaks.len(), 1, "{leaks:?}");
    let leak = &leaks[0];
    assert_eq!(leak.physical, ghost);
    assert_eq!(leak.kind, Kind::service());
    assert_eq!(leak.name, "ghost-run");
    assert_eq!(leak.owner, path("ghost/run").owner());
    assert_eq!(leak.node, path("ghost/run").key());
}

type Writer = Box<dyn FnOnce(&AnyLedger) + Send>;

/// A ledger that runs `then` right after its `nth` read has been answered: a writer that gets in
/// between the sweep's reads.
struct Interleaved {
    inner: AnyLedger,
    reads: AtomicUsize,
    nth: usize,
    then: Mutex<Option<Writer>>,
}

impl Interleaved {
    fn new(inner: &AnyLedger, nth: usize, then: impl FnOnce(&AnyLedger) + Send + 'static) -> Self {
        Self {
            inner: inner.clone(),
            reads: AtomicUsize::new(0),
            nth,
            then: Mutex::new(Some(Box::new(then))),
        }
    }
}

impl ucdk_ledger::Ledger for Interleaved {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        let answer = self.inner.read();
        if self.reads.fetch_add(1, Ordering::SeqCst) + 1 == self.nth
            && let Some(then) = self.then.lock().expect("lock").take()
        {
            then(&self.inner);
        }
        answer
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        self.inner.write(expected, change)
    }

    fn write_fenced(
        &self,
        fence: Fence,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        self.inner.write_fenced(fence, expected, change)
    }

    fn acquire(&self, holder: &str, ttl: Duration) -> Result<Lease, LeaseError> {
        self.inner.acquire(holder, ttl)
    }

    fn renew(&self, fence: Fence, ttl: Duration) -> Result<Lease, LedgerError> {
        self.inner.renew(fence, ttl)
    }

    fn release(&self, fence: Fence) -> Result<(), LedgerError> {
        self.inner.release(fence)
    }
}

/// Proves: an object a concurrent run created after the sweep read the ledger (its intent is
/// written before the object exists, so the sweep's second read has it) is not reported, and
/// neither is one that run deleted and unrecorded after the sweep read the ledger but before it
/// listed. Catches a sweep that judges the listing by a ledger read made before it.
fn a_concurrent_writers_objects_are_not_leaks(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    // A writer that, right after the sweep's first read, records an intent for a new object that
    // already exists, and drops the record of a recorded one that is still listed.
    let new = stamped(&mut fake, "fresh", &cell());
    let old = physical_of(&ledger, "db/run");
    let racing = Interleaved::new(&ledger, 1, |ledger| {
        let intent = Record::intent(
            path("fresh/run"),
            Kind::service(),
            "fresh-run",
            BTreeSet::new(),
        );
        let token = ledger.read().expect("read").token;
        ledger
            .write(&token, Change::Put(intent))
            .expect("the writer records its intent");
        let token = ledger.read().expect("read").token;
        ledger
            .write(&token, Change::Remove(path("db/run")))
            .expect("the writer drops a record");
    });
    assert_eq!(leaks_of(sweep(&mut fake, &racing)), []);
    // Settled, nothing hides them: the new object has its intent, and the old one lost its record.
    let rest = leaks_of(sweep(&mut fake, &ledger));
    assert_eq!(rest.len(), 1, "{rest:?}");
    assert_eq!(rest[0].physical, old);
    assert_ne!(rest[0].physical, new);
}

/// Proves: an intent whose create never happened (no object, no physical id) reports nothing,
/// and a stamped object of another node is still reported beside it. Catches an intent that is
/// counted as standing for an object it has none of.
fn an_intent_with_no_object_accounts_for_nothing(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    fake.faults_mut().fail_at_call.replace(1);
    assert!(matches!(
        apply(&mut fake, &ledger, &chain()),
        Outcome::Failed(_)
    ));
    assert_eq!(records(&ledger)[&path("db/run")].physical, None);
    assert_eq!(fake.object_count(), 0);
    assert_eq!(leaks_of(sweep(&mut fake, &ledger)), []);
    let ghost = stamped(&mut fake, "ghost", &cell());
    let leaks = leaks_of(sweep(&mut fake, &ledger));
    assert_eq!(leaks.len(), 1, "{leaks:?}");
    assert_eq!(leaks[0].physical, ghost);
}

/// Proves: a stamped object beside an intent that names no physical id is reported when the
/// intent already stands for another object of its node, and the intent's own object is not.
/// Catches an intent that hides every stamped object of its node.
fn a_duplicate_beside_an_intent_with_no_physical_id_is_a_leak(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    fake.faults_mut().ambiguous_at_call.replace(1);
    assert!(matches!(
        apply(&mut fake, &ledger, &chain()),
        Outcome::Failed(_)
    ));
    let intent = records(&ledger)[&path("db/run")].clone();
    assert_eq!((intent.state, intent.physical), (State::Intent, None));
    // A second object for the same node, under another name, as a retried create would leave.
    let node = Node::new(path("db/run"), Kind::service(), "db-run-again", Vec::new())
        .expect("a node with no fields");
    let twin = fake
        .create(&node, &Stamp::for_node(&cell(), node.path()))
        .expect("create")
        .physical;
    let leaks = leaks_of(sweep(&mut fake, &ledger));
    assert_eq!(leaks.len(), 1, "{leaks:?}");
    assert_eq!(leaks[0].physical, twin);
    assert_eq!(leaks[0].node, path("db/run").key());
}

/// Proves: an object stamped by another cell (another id, another cell name, another machine) is
/// not a leak of this one. Catches a sweep that lists any stamped object, or tells cells apart
/// by one part of the identity.
fn a_stamp_of_another_cell_is_not_a_leak(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    for (i, other) in kit::neighbour_cells().into_iter().skip(1).enumerate() {
        stamped(&mut fake, &format!("stranger{i}"), &other);
    }
    let name = |t: &str| CellName::new(t).expect("a name");
    let uid = CellUid::from_text("cccccccccccccccccccccccccc").expect("an id");
    stamped(
        &mut fake,
        "alien",
        &CellIdentity::new(name("local"), name("default"), uid),
    );
    assert_eq!(fake.object_count(), 4);
    assert_eq!(leaks_of(sweep(&mut fake, &ledger)), []);
}

/// Proves: an object a record stands for is not a leak: a confirmed record that names its
/// physical id, and an intent whose create was lost (no physical id yet) for the node the
/// stamp names. Catches a sweep that reports what a later run would take up.
fn a_recorded_object_is_not_a_leak(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    assert_eq!(fake.object_count(), 4);
    assert_eq!(leaks_of(sweep(&mut fake, &ledger)), []);

    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    fake.faults_mut().ambiguous_at_call.replace(1);
    assert!(matches!(
        apply(&mut fake, &ledger, &chain()),
        Outcome::Failed(_)
    ));
    let intent = records(&ledger)[&path("db/run")].clone();
    assert_eq!((intent.state, intent.physical), (State::Intent, None));
    assert_eq!(fake.object_count(), 1, "the create landed");
    assert_eq!(leaks_of(sweep(&mut fake, &ledger)), []);
}

/// Proves: a record that names another object of the same node does not account for a stamped
/// object (a duplicate left beside a recorded one is a leak). Catches a sweep that matches by
/// node alone.
fn a_record_of_another_object_does_not_hide_a_duplicate(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    let old = physical_of(&ledger, "db/run");
    fake.delete(&Kind::service(), &old).expect("delete");
    let twin = stamped(&mut fake, "db", &cell());
    assert_ne!(twin, old);
    let leaks = leaks_of(sweep(&mut fake, &ledger));
    assert_eq!(leaks.len(), 1, "{leaks:?}");
    assert_eq!(leaks[0].physical, twin);
}

/// Proves: an object whose stamp was stripped is not reported, whether it has a record (it is
/// then a conflict for a run, not a leak) or none (it is unclaimed, and not ours to name), and
/// neither is another tool's object. Catches a sweep that claims objects by name or by record
/// instead of by stamp.
fn a_stripped_stamp_is_not_reported(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    let recorded = physical_of(&ledger, "web/public");
    assert!(fake.set_label(&recorded, "ucdk-owner", None));
    let ghost = stamped(&mut fake, "ghost", &cell());
    assert!(fake.set_label(&ghost, "ucdk-owner", None));
    fake.add_foreign(&Kind::service(), "bystander", None);
    fake.add_foreign(&Kind::service(), "other-tool", Some("other:1".to_owned()));
    assert_eq!(leaks_of(sweep(&mut fake, &ledger)), []);
}

/// Proves: the sweep makes one provider list call and no other provider call, writes no ledger
/// record, leaves the ledger document and the objects as they were, and leaves the leaked object
/// in place. Catches a sweep that deletes, adopts or records what it finds.
fn the_sweep_writes_nothing_and_mutates_nothing(backend: Backend) {
    let (mut fake, inner) = (Fake::standard(), FlakyLedger::on(backend));
    done(apply(&mut fake, &inner, &chain()));
    let ghost = stamped(&mut fake, "ghost", &cell());
    let writes = inner.writes();
    let document = inner.read().expect("read");
    let world = objects(&fake);
    let mutated = mutations(&fake);
    assert!(mutated > 0);
    fake.clear_calls();

    assert_eq!(leaks_of(sweep(&mut fake, &inner)).len(), 1);

    let ops: Vec<Op> = fake.calls().iter().map(|c| c.op).collect();
    assert_eq!(ops, [Op::ListStamped], "one list and nothing else");
    assert_eq!(mutations(&fake), 0);
    assert_eq!(inner.writes(), writes, "no ledger write");
    assert_eq!(inner.read().expect("read"), document);
    assert_eq!(objects(&fake), world, "the objects are as they were");
    assert!(
        fake.list_stamped(&ucdk_provider::CellScope::new(cell()))
            .expect("list")
            .iter()
            .any(|o| o.physical == ghost),
        "the leaked object is still there"
    );
}

/// Proves: an empty world has no leaks, and so does one whose every stamped object is recorded.
/// Catches a sweep that reports something where there is nothing.
fn an_empty_world_has_no_leaks(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    assert_eq!(leaks_of(sweep(&mut fake, &ledger)), []);
}

/// Proves: an apply that finds a leak still ends done, with the leak in its report beside what
/// it created, and the leaked object is not touched by that apply or by a destroy that follows
/// (a stamp alone never authorises a delete); the sweep run afterwards still reports it.
/// Catches an apply that fails for leaks, drops them, or cleans them up.
fn an_apply_reports_a_leak_and_is_still_done(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    let ghost = stamped(&mut fake, "ghost", &cell());
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(report.created.len(), 4);
    assert_eq!(report.leak_sweep_failed, None);
    assert_eq!(report.leaks.len(), 1, "{:?}", report.leaks);
    assert_eq!(report.leaks[0].physical, ghost);
    assert_eq!(fake.object_count(), 5);

    done(destroy(&mut fake, &ledger, &chain()));
    assert_eq!(fake.object_count(), 1, "only the leaked object remains");
    let leaks = leaks_of(sweep(&mut fake, &ledger));
    assert_eq!(leaks.len(), 1);
    assert_eq!(leaks[0].physical, ghost);
}

/// Proves: an apply with nothing leaked reports no leaks, and a second apply that changes
/// nothing still sweeps (its report carries a leak that appeared between the two). Catches a
/// sweep tied to runs that create.
fn an_apply_with_no_changes_still_sweeps(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    let ghost = stamped(&mut fake, "ghost", &cell());
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert!(report.created.is_empty());
    assert_eq!(report.leaks.len(), 1);
    assert_eq!(report.leaks[0].physical, ghost);

    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(report.leaks, []);
    assert_eq!(report.leak_sweep_failed, None);
}

/// Proves: a sweep that cannot list does not fail an apply that finished: the apply is done, its
/// changes stand, and the report says the sweep failed. Catches an apply turned into a failure
/// (or into a silent success) by a read after the work was done.
fn a_failed_sweep_does_not_fail_the_apply(backend: Backend) {
    // The probe counts the provider calls of a clean apply: the sweep's list is the last.
    let mut probe_fake = Fake::standard();
    let probe_ledger = backend.ledger();
    let mut probe = Hooked::new(&mut probe_fake, usize::MAX, |_| {});
    done(apply_with(&mut probe, &probe_ledger, &chain(), false));
    let last = probe.calls();

    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    let mut hooked = Hooked::new(&mut fake, last, |fake| {
        fake.faults_mut()
            .fail_reads
            .push(ProviderError::Transient("down".to_owned()));
    });
    let report = done(apply_with(&mut hooked, &ledger, &chain(), false));
    assert_eq!(report.created.len(), 4);
    assert_eq!(report.leaks, []);
    assert_eq!(
        report.leak_sweep_failed,
        Some(EngineError::Provider(ProviderError::Transient(
            "down".to_owned()
        )))
    );
    assert!(
        records(&ledger)
            .values()
            .all(|r| r.state == State::Confirmed)
    );
}

/// Proves: an apply that fails does not sweep (no list call). Catches a sweep that runs after a run that did not finish.
fn a_run_that_did_not_finish_does_not_sweep(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    fake.faults_mut().ambiguous_at_call.replace(1);
    assert!(matches!(
        apply(&mut fake, &ledger, &chain()),
        Outcome::Failed(_)
    ));
    assert!(fake.calls().iter().all(|c| c.op != Op::ListStamped));
}

/// Proves: a ledger that cannot be read, and a provider list that fails, each end the sweep as a
/// failure that started nothing (no node named, an empty report). Catches a sweep that
/// swallows a read error into an empty answer, which would read as "no leaks".
fn a_sweep_fails_when_a_read_fails(backend: Backend) {
    let (mut fake, flaky) = (Fake::standard(), FlakyLedger::on(backend));
    done(apply(&mut fake, &flaky, &chain()));
    flaky.fail_reads(true);
    let Outcome::Failed(f) = sweep(&mut fake, &flaky) else {
        panic!("expected a failure");
    };
    assert!(matches!(f.error, EngineError::Ledger(_)));
    assert_eq!((f.at, f.report), (None, Report::default()));

    flaky.fail_reads(false);
    fake.faults_mut()
        .fail_reads
        .push(ProviderError::Transient("down".to_owned()));
    let Outcome::Failed(f) = sweep(&mut fake, &flaky) else {
        panic!("expected a failure");
    };
    assert_eq!(
        f.error,
        EngineError::Provider(ProviderError::Transient("down".to_owned()))
    );
    assert_eq!(f.at, None);
}

macro_rules! on_every_ledger {
    ($($name:ident),* $(,)?) => {
        $(
            mod $name {
                #[test]
                fn memory() {
                    super::$name(super::Backend::Memory);
                }

                #[test]
                fn file() {
                    super::$name(super::Backend::File);
                }
            }
        )*
    };
}

on_every_ledger!(
    an_object_we_stamped_with_no_record_is_a_leak,
    a_stamp_of_another_cell_is_not_a_leak,
    a_concurrent_writers_objects_are_not_leaks,
    a_duplicate_beside_an_intent_with_no_physical_id_is_a_leak,
    an_intent_with_no_object_accounts_for_nothing,
    a_recorded_object_is_not_a_leak,
    a_record_of_another_object_does_not_hide_a_duplicate,
    a_stripped_stamp_is_not_reported,
    the_sweep_writes_nothing_and_mutates_nothing,
    an_empty_world_has_no_leaks,
    an_apply_reports_a_leak_and_is_still_done,
    an_apply_with_no_changes_still_sweeps,
    a_failed_sweep_does_not_fail_the_apply,
    a_run_that_did_not_finish_does_not_sweep,
    a_sweep_fails_when_a_read_fails,
);
