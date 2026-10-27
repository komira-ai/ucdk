// SPDX-License-Identifier: Apache-2.0
//! The fake registry: what it holds, what each fault does, and how it checks a grant.

mod common;

use std::time::Duration;

use common::*;
use sha2::{Digest as _, Sha256};
use ucdk_core::{Hash256, ManualClock};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::{Held, Presence};
use ucdk_release_fake::{FakeRegistry, Read, Scopes};
use ucdk_release_proto::v1::{ArtifactType, Coordinate, Reason};
use ucdk_release_seams::{
    Blob, CommitAnswer, CommitUnit, Grant, ListScope, Publisher, SeamError, StageAnswer, StageItem,
};

fn sha(bytes: &[u8]) -> String {
    Hash256::from_sha256(&Sha256::digest(bytes).into()).to_string()
}

/// A world with the set `names` staged under a stage grant.
fn staged(names: &[&str]) -> (World, Set) {
    let mut w = World::new();
    let set = Set::of(names);
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    let items = set.items(&blobs);
    assert_eq!(w.registry.stage(&g, &items), Ok(StageAnswer::Staged));
    (w, set)
}

fn commit(w: &mut World, set: &Set) -> Result<CommitAnswer, SeamError> {
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Commit, &set.coords));
    let items = set.items(&blobs);
    w.registry.commit(&g, &items)
}

fn stranger(w: &mut World, set: &Set, at: usize) -> Presence {
    w.registry
        .observe(&Grant::none(), &set.coords[at], &[])
        .expect("an observe")
}

#[test]
fn a_set_is_staged_observed_then_committed_and_read_by_a_stranger() {
    let (mut w, set) = staged(&["a", "b"]);
    assert_eq!(stranger(&mut w, &set, 0), Presence::Absent);
    let g = w.grant(GrantPhase::Stage, &set.coords);
    let Presence::Staged(held) = w.registry.observe(&g, &set.coords[0], &[]).unwrap() else {
        panic!("a grant of the action sees what it staged");
    };
    assert_eq!(held.digest, sha(b"a"));
    assert_eq!(held.stamp, set.held.stamp);
    assert_eq!(commit(&mut w, &set), Ok(CommitAnswer::Committed));
    let Presence::Present(held) = stranger(&mut w, &set, 1) else {
        panic!("a committed coordinate is readable");
    };
    assert_eq!(held.digest, sha(b"b"));
    assert_eq!(w.registry.bytes(&set.coords[1]), Some(&b"b"[..]));
    assert!(!w.registry.is_staged(&set.coords[0]));
    assert_eq!(w.registry.stage_log().len(), 1);
    assert_eq!(
        w.registry.commit_log(),
        [vec![coordinate_key_of(&set, 0), coordinate_key_of(&set, 1)]]
    );
    assert_eq!(w.registry.call_log().len(), 5);
}

fn coordinate_key_of(set: &Set, at: usize) -> String {
    ucdk_release_fake::coordinate_key(&set.coords[at])
}

#[test]
fn lost_answer_after_commit_leaves_the_bytes() {
    let (mut w, set) = staged(&["a"]);
    w.registry.lose_answer_after_commit(1);
    assert!(matches!(commit(&mut w, &set), Err(SeamError::Unknown(_))));
    assert_eq!(w.registry.bytes(&set.coords[0]), Some(&b"a"[..]));
    assert!(matches!(stranger(&mut w, &set, 0), Presence::Present(_)));
}

#[test]
fn lost_answer_before_commit_commits_nothing() {
    let (mut w, set) = staged(&["a"]);
    w.registry.lose_answer_before_commit(1);
    assert!(matches!(commit(&mut w, &set), Err(SeamError::Unknown(_))));
    assert_eq!(w.registry.bytes(&set.coords[0]), None);
    assert!(w.registry.is_staged(&set.coords[0]));
    // The next call is the second, and lands.
    assert_eq!(commit(&mut w, &set), Ok(CommitAnswer::Committed));
}

#[test]
fn fail_after_commit_commits_and_answers_transient() {
    let (mut w, set) = staged(&["a"]);
    w.registry.fail_after_commit(1);
    assert!(matches!(commit(&mut w, &set), Err(SeamError::Transient(_))));
    assert!(matches!(stranger(&mut w, &set, 0), Presence::Present(_)));
}

#[test]
fn tag_replacing_registry_declares_itself_not_conditional() {
    let clock = ManualClock::at(0);
    assert!(FakeRegistry::new(&clock, LOCATION).conditional_put());
    assert!(
        !FakeRegistry::new(&clock, LOCATION)
            .tag_replacing()
            .conditional_put()
    );
}

#[test]
fn staged_coordinates_vanish_with_lose_staged_state() {
    let mut w = World::new();
    w.registry.lose_staged_state();
    let set = Set::of(&["a"]);
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    let items = set.items(&blobs);
    assert_eq!(w.registry.stage(&g, &items), Ok(StageAnswer::Staged));
    assert_eq!(
        w.registry.observe(&g, &set.coords[0], &[]),
        Ok(Presence::Absent)
    );
    assert!(matches!(commit(&mut w, &set), Err(SeamError::Permanent(_))));
}

#[test]
fn an_expired_grant_is_refused_by_the_shared_manual_clock() {
    let mut w = World::new();
    let set = Set::of(&["a"]);
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    let items = set.items(&blobs);
    w.clock.advance(Duration::from_secs(601));
    assert_eq!(
        w.registry.stage(&g, &items),
        Err(SeamError::GrantRefused(GrantFault::Expired))
    );
    assert!(w.registry.stage_log().is_empty());
    let fresh = w.grant(GrantPhase::Stage, &set.coords);
    assert_eq!(w.registry.stage(&fresh, &items), Ok(StageAnswer::Staged));
}

#[test]
fn a_grant_of_the_wrong_phase_or_scope_is_denied() {
    let mut w = World::new();
    let set = Set::of(&["a", "b"]);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let denied = Some(SeamError::GrantRefused(GrantFault::Denied));
    let commit_grant = w.grant(GrantPhase::Commit, &set.coords);
    assert_eq!(w.registry.stage(&commit_grant, &items).err(), denied);
    let stage_grant = w.grant(GrantPhase::Stage, &set.coords);
    assert_eq!(w.registry.commit(&stage_grant, &items).err(), denied);
    let one = w.grant(GrantPhase::Stage, &set.coords[..1]);
    assert_eq!(w.registry.stage(&one, &items).err(), denied);
    w.repository.location = "another/repository".to_owned();
    let elsewhere = w.grant(GrantPhase::Stage, &set.coords);
    assert_eq!(w.registry.stage(&elsewhere, &items).err(), denied);
    let junk = Grant::issued(b"junk".to_vec(), u64::MAX);
    assert_eq!(w.registry.stage(&junk, &items).err(), denied);
    assert_eq!(w.registry.stage(&Grant::none(), &items).err(), denied);
    assert!(w.registry.stage_log().is_empty());
    assert!(w.registry.commit_log().is_empty());
}

#[test]
fn equal_scopes_let_a_stage_grant_commit() {
    let clock = ManualClock::at(1_000_000);
    let same = Scopes {
        stage: "publish".to_owned(),
        commit: "publish".to_owned(),
        read: "read".to_owned(),
    };
    let mut w = World::with(
        &clock,
        FakeRegistry::new(&clock, LOCATION).with_scopes(same),
    );
    let Some(ucdk_release_proto::v1::push_identity::Kind::Federated(f)) =
        w.repository.push.as_mut().and_then(|p| p.kind.as_mut())
    else {
        panic!("the fixture is federated");
    };
    f.stage_scope = "publish".to_owned();
    f.commit_scope = "publish".to_owned();
    let set = Set::of(&["a"]);
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    let items = set.items(&blobs);
    assert_eq!(w.registry.stage(&g, &items), Ok(StageAnswer::Staged));
    assert_eq!(w.registry.commit(&g, &items), Ok(CommitAnswer::Committed));
}

#[test]
fn a_read_grant_observes_and_can_neither_stage_nor_commit() {
    let (mut w, set) = staged(&["a"]);
    let read = w.grant(GrantPhase::Read, &set.coords);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let denied = Some(SeamError::GrantRefused(GrantFault::Denied));
    assert!(matches!(
        w.registry.observe(&read, &set.coords[0], &[]),
        Ok(Presence::Staged(_))
    ));
    assert_eq!(w.registry.fetch(&read, &set.coords[0]), Ok(None));
    assert_eq!(w.registry.stage(&read, &items).err(), denied);
    assert_eq!(w.registry.commit(&read, &items).err(), denied);
    assert_eq!(w.registry.abort(&read, &set.coords).err(), denied);
    assert_eq!(w.registry.yank(&read, &set.coords[0], "no").err(), denied);
    assert_eq!(w.registry.stage_log().len(), 1);
    assert!(w.registry.commit_log().is_empty());
}

#[test]
fn a_grant_that_is_already_spent_is_refused() {
    let (mut w, set) = staged(&["a"]);
    w.registry.grants_already_used();
    let g = w.grant(GrantPhase::Commit, &set.coords);
    let blobs = set.blobs();
    assert_eq!(
        w.registry.commit(&g, &set.items(&blobs)),
        Err(SeamError::GrantRefused(GrantFault::Used))
    );
    // A read that carries no grant has nothing to spend.
    assert_eq!(
        w.registry.observe(&Grant::none(), &set.coords[0], &[]),
        Ok(Presence::Absent)
    );
}

#[test]
fn a_stage_of_a_held_or_staged_coordinate_is_a_duplicate_and_changes_nothing() {
    let (mut w, set) = staged(&["a", "b"]);
    let other = Set::of(&["b", "c"]);
    let (blobs, g) = (other.blobs(), w.grant(GrantPhase::Stage, &other.coords));
    // `b` is staged already: the whole stage is a duplicate and `c` is not staged.
    assert_eq!(
        w.registry.stage(&g, &other.items(&blobs)),
        Ok(StageAnswer::Duplicate)
    );
    assert!(!w.registry.is_staged(&other.coords[1]));
    assert_eq!(commit(&mut w, &set), Ok(CommitAnswer::Committed));
    // `b` is held now.
    let g = w.grant(GrantPhase::Stage, &other.coords);
    assert_eq!(
        w.registry.stage(&g, &other.items(&blobs)),
        Ok(StageAnswer::Duplicate)
    );
}

#[test]
fn a_commit_over_other_bytes_or_a_yanked_coordinate_is_a_duplicate_and_leaves_the_old_bytes() {
    let (mut w, set) = staged(&["a", "b"]);
    w.registry.present_different(&set.coords[0], b"theirs");
    assert_eq!(commit(&mut w, &set), Ok(CommitAnswer::Duplicate));
    assert_eq!(w.registry.bytes(&set.coords[0]), Some(&b"theirs"[..]));
    assert_eq!(w.registry.bytes(&set.coords[1]), None);
}

#[test]
fn a_racing_writer_wins_the_next_commit() {
    let (mut w, set) = staged(&["a"]);
    w.registry.race_next_commit(b"racer");
    assert_eq!(commit(&mut w, &set), Ok(CommitAnswer::Duplicate));
    assert_eq!(w.registry.bytes(&set.coords[0]), Some(&b"racer"[..]));
}

#[test]
fn a_torn_commit_makes_only_the_first_coordinate_readable() {
    let (mut w, set) = staged(&["a", "b"]);
    w.registry.torn_commit();
    assert!(matches!(commit(&mut w, &set), Err(SeamError::Unknown(_))));
    assert!(w.registry.bytes(&set.coords[0]).is_some());
    assert!(w.registry.bytes(&set.coords[1]).is_none());
}

#[test]
fn a_commit_of_a_set_that_fails_whole_makes_nothing_readable() {
    let (mut w, set) = staged(&["a", "b"]);
    w.registry.commit_set_fails_whole();
    assert!(matches!(commit(&mut w, &set), Err(SeamError::Permanent(_))));
    assert!(w.registry.bytes(&set.coords[0]).is_none());
    assert!(w.registry.is_staged(&set.coords[0]));
}

#[test]
fn a_rate_limited_call_is_refused_before_it_runs_and_the_next_runs() {
    let mut w = World::new();
    let set = Set::of(&["a"]);
    w.registry.rate_limit(1);
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    let items = set.items(&blobs);
    assert!(matches!(
        w.registry.stage(&g, &items),
        Err(SeamError::RateLimited(b)) if b.ticks == 1
    ));
    assert!(w.registry.stage_log().is_empty());
    assert_eq!(w.registry.stage(&g, &items), Ok(StageAnswer::Staged));
}

#[test]
fn no_digest_exposed_reports_a_held_coordinate_with_no_digest() {
    let (mut w, set) = staged(&["a"]);
    commit(&mut w, &set).unwrap();
    w.registry.no_digest_exposed();
    let Presence::Present(held) = stranger(&mut w, &set, 0) else {
        panic!("held");
    };
    assert_eq!(held.digest, "");
    let g = w.grant(GrantPhase::Read, &[]);
    let newest = w.registry.newest(&g, "stable", "a").unwrap();
    assert_eq!(newest[0].1.digest, "");
}

#[test]
fn observes_after_the_nth_are_unreadable() {
    let (mut w, set) = staged(&["a"]);
    w.registry.unreadable_after(1);
    assert!(stranger_result(&mut w, &set).is_ok());
    assert!(matches!(
        stranger_result(&mut w, &set),
        Err(SeamError::Unknown(_))
    ));
}

fn stranger_result(w: &mut World, set: &Set) -> Result<Presence, SeamError> {
    w.registry.observe(&Grant::none(), &set.coords[0], &[])
}

#[test]
fn read_lag_hides_a_committed_coordinate_for_the_stated_reads() {
    let (mut w, set) = staged(&["a"]);
    commit(&mut w, &set).unwrap();
    w.registry.read_lag(2);
    assert_eq!(stranger(&mut w, &set, 0), Presence::Absent);
    assert_eq!(stranger(&mut w, &set, 0), Presence::Absent);
    assert!(matches!(stranger(&mut w, &set, 0), Presence::Present(_)));
}

#[test]
fn a_yanked_coordinate_keeps_its_bytes_and_cannot_be_committed_again() {
    let (mut w, set) = staged(&["a"]);
    commit(&mut w, &set).unwrap();
    let g = w.grant(GrantPhase::Commit, &set.coords);
    w.registry.yank(&g, &set.coords[0], "withdrawn").unwrap();
    assert!(w.registry.is_yanked(&set.coords[0]));
    assert!(matches!(stranger(&mut w, &set, 0), Presence::Yanked(_)));
    assert_eq!(
        w.registry.fetch(&g, &set.coords[0]),
        Ok(Some(b"a".to_vec()))
    );
    assert_eq!(commit(&mut w, &set), Ok(CommitAnswer::Duplicate));
    let missing = coordinate("missing");
    let g = w.grant(GrantPhase::Commit, std::slice::from_ref(&missing));
    assert!(matches!(
        w.registry.yank(&g, &missing, "x"),
        Err(SeamError::Permanent(_))
    ));
}

#[test]
fn newest_and_list_show_what_the_channel_holds() {
    let (mut w, set) = staged(&["tool", "tool-extra", "other"]);
    commit(&mut w, &set).unwrap();
    let g = w.grant(GrantPhase::Read, &[]);
    let newest = w.registry.newest(&g, "stable", "tool").unwrap();
    assert_eq!(newest.len(), 1);
    assert_eq!(newest[0].0, set.coords[0]);
    let scope = ListScope {
        channel: "stable".to_owned(),
        prefix: "tool".to_owned(),
    };
    let listed = w.registry.list(&g, &scope).unwrap();
    assert_eq!(listed.len(), 2);
    // The same name in another channel is another coordinate, which neither answer shows.
    let beta = ucdk_release_proto::v1::Coordinate {
        channel: "beta".to_owned(),
        ..coordinate("tool")
    };
    w.registry.present_different(&beta, b"beta");
    assert_eq!(w.registry.newest(&g, "stable", "tool").unwrap().len(), 1);
    assert_eq!(w.registry.list(&g, &scope).unwrap().len(), 2);
    assert_eq!(w.registry.newest(&g, "beta", "tool").unwrap()[0].0, beta);
    w.registry.list_lag();
    assert_eq!(w.registry.list(&g, &scope), Ok(Vec::new()));
}

#[test]
fn abort_gives_staged_state_back_and_never_touches_a_committed_coordinate() {
    let (mut w, set) = staged(&["a", "b"]);
    w.registry.present_different(&set.coords[1], b"held");
    let g = w.grant(GrantPhase::Stage, &set.coords);
    w.registry.abort(&g, &set.coords).unwrap();
    assert!(!w.registry.is_staged(&set.coords[0]));
    assert_eq!(w.registry.bytes(&set.coords[1]), Some(&b"held"[..]));
}

#[test]
fn staged_state_of_another_writer_blocks_a_stage_and_is_not_seen() {
    let mut w = World::new();
    let set = Set::of(&["a"]);
    w.registry.stage_other_bytes(&set.coords[0], b"theirs");
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    assert_eq!(
        w.registry.stage(&g, &set.items(&blobs)),
        Ok(StageAnswer::Duplicate)
    );
    assert_eq!(
        w.registry.observe(&g, &set.coords[0], &[]),
        Ok(Presence::Absent)
    );
}

#[test]
fn a_channel_that_does_not_stage_commits_with_the_bytes() {
    let clock = ManualClock::at(1_000_000);
    let registry = FakeRegistry::new(&clock, LOCATION)
        .without_staging()
        .commit_per_coordinate();
    let mut w = World::with(&clock, registry);
    assert!(!w.registry.stages());
    assert_eq!(w.registry.commit_unit(), CommitUnit::Coordinate);
    let set = Set::of(&["a"]);
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    assert!(matches!(
        w.registry.stage(&g, &set.items(&blobs)),
        Err(SeamError::Permanent(_))
    ));
    assert_eq!(commit(&mut w, &set), Ok(CommitAnswer::Committed));
    assert_eq!(w.registry.bytes(&set.coords[0]), Some(&b"a"[..]));
}

#[test]
fn an_open_channel_admits_a_write_with_no_grant_and_a_closed_one_does_not() {
    let clock = ManualClock::at(0);
    let set = Set::of(&["a"]);
    let blobs = set.blobs();
    let mut open = FakeRegistry::new(&clock, LOCATION).without_staging().open();
    assert_eq!(
        open.commit(&Grant::none(), &set.items(&blobs)),
        Ok(CommitAnswer::Committed)
    );
}

#[test]
fn the_registry_describes_itself() {
    let clock = ManualClock::at(0);
    let registry = FakeRegistry::new(&clock, LOCATION).with_grant_limit(3);
    assert_eq!(registry.grant_limit(), 3);
    assert_eq!(registry.types(), [ArtifactType::File]);
    assert!(registry.stages());
    assert_eq!(registry.commit_unit(), CommitUnit::Set);
    assert_eq!(
        FakeRegistry::new(&clock, LOCATION).grant_limit(),
        usize::MAX
    );
}

#[test]
fn an_observe_with_a_set_that_lacks_the_coordinate_is_refused() {
    let mut w = World::new();
    let set = Set::of(&["a"]);
    let other = Set::of(&["b"]);
    let blobs = other.blobs();
    let g = w.grant(GrantPhase::Read, &set.coords);
    assert_eq!(
        w.registry.observe(&g, &set.coords[0], &other.items(&blobs)),
        Err(SeamError::Refused(Reason::Channel))
    );
}

fn blob<'a>(c: &'a Coordinate, bytes: &'a [u8]) -> Blob<'a> {
    Blob {
        name: c.name.as_str(),
        bytes,
    }
}

fn item<'a>(c: &'a Coordinate, file: &'a Blob<'a>, held: &'a Held) -> StageItem<'a> {
    StageItem {
        coordinate: c,
        file,
        held,
    }
}

fn is_permanent<T: std::fmt::Debug>(answer: &Result<T, SeamError>) -> bool {
    matches!(answer, Err(SeamError::Permanent(_)))
}

#[test]
fn a_stage_that_names_a_coordinate_twice_is_refused_and_stages_nothing() {
    let mut w = World::new();
    let (a, h) = (coordinate("a"), held());
    let (x, evil) = (blob(&a, b"X"), blob(&a, b"EVIL"));
    let g = w.grant(GrantPhase::Stage, std::slice::from_ref(&a));
    let answer = w
        .registry
        .stage(&g, &[item(&a, &x, &h), item(&a, &evil, &h)]);
    assert!(is_permanent(&answer), "{answer:?}");
    assert!(!w.registry.is_staged(&a));
    assert!(w.registry.stage_log().is_empty());
}

#[test]
fn a_commit_that_names_a_staged_coordinate_twice_is_refused_and_commits_nothing() {
    let mut w = World::new();
    let (a, h) = (coordinate("a"), held());
    let (x, evil) = (blob(&a, b"X"), blob(&a, b"EVIL"));
    let g = w.grant(GrantPhase::Stage, std::slice::from_ref(&a));
    assert_eq!(
        w.registry.stage(&g, &[item(&a, &x, &h)]),
        Ok(StageAnswer::Staged)
    );
    let g = w.grant(GrantPhase::Commit, std::slice::from_ref(&a));
    let answer = w
        .registry
        .commit(&g, &[item(&a, &x, &h), item(&a, &evil, &h)]);
    assert!(is_permanent(&answer), "{answer:?}");
    assert_eq!(w.registry.bytes(&a), None);
    assert!(w.registry.is_staged(&a));
    assert!(w.registry.commit_log().is_empty());
    // The honest commit still lands, with the staged bytes.
    assert_eq!(
        w.registry.commit(&g, &[item(&a, &x, &h)]),
        Ok(CommitAnswer::Committed)
    );
    assert_eq!(w.registry.bytes(&a), Some(&b"X"[..]));
}

#[test]
fn a_commit_that_names_a_coordinate_twice_on_a_channel_that_does_not_stage_commits_nothing() {
    let clock = ManualClock::at(1_000_000);
    let mut w = World::with(
        &clock,
        FakeRegistry::new(&clock, LOCATION).without_staging(),
    );
    let (a, h) = (coordinate("a"), held());
    let (x, evil) = (blob(&a, b"X"), blob(&a, b"EVIL"));
    let g = w.grant(GrantPhase::Commit, std::slice::from_ref(&a));
    let answer = w
        .registry
        .commit(&g, &[item(&a, &x, &h), item(&a, &evil, &h)]);
    assert!(is_permanent(&answer), "{answer:?}");
    assert_eq!(w.registry.bytes(&a), None);
}

#[test]
fn an_observe_with_a_set_that_names_a_coordinate_twice_is_refused() {
    let mut w = World::new();
    let (a, h) = (coordinate("a"), held());
    let (x, evil) = (blob(&a, b"X"), blob(&a, b"EVIL"));
    let g = w.grant(GrantPhase::Read, std::slice::from_ref(&a));
    let answer = w
        .registry
        .observe(&g, &a, &[item(&a, &x, &h), item(&a, &evil, &h)]);
    assert!(is_permanent(&answer), "{answer:?}");
}

#[test]
fn a_commit_that_differs_from_what_was_staged_is_refused_and_the_staged_state_stays() {
    let mut w = World::new();
    let (a, h) = (coordinate("a"), held());
    let (x, y) = (blob(&a, b"X"), blob(&a, b"Y"));
    let g = w.grant(GrantPhase::Stage, std::slice::from_ref(&a));
    w.registry.stage(&g, &[item(&a, &x, &h)]).unwrap();
    let g = w.grant(GrantPhase::Commit, std::slice::from_ref(&a));
    // Other bytes.
    let answer = w.registry.commit(&g, &[item(&a, &y, &h)]);
    assert!(is_permanent(&answer), "{answer:?}");
    // The same bytes with another stamp.
    let mut stamped = held();
    stamped.exceptional = true;
    let answer = w.registry.commit(&g, &[item(&a, &x, &stamped)]);
    assert!(is_permanent(&answer), "{answer:?}");
    assert_eq!(w.registry.bytes(&a), None);
    assert!(w.registry.is_staged(&a));
    assert_eq!(
        w.registry.commit(&g, &[item(&a, &x, &h)]),
        Ok(CommitAnswer::Committed)
    );
    assert_eq!(w.registry.bytes(&a), Some(&b"X"[..]));
}

#[test]
fn a_private_repository_refuses_every_read_without_a_grant() {
    let clock = ManualClock::at(1_000_000);
    let mut w = World::with(&clock, FakeRegistry::new(&clock, LOCATION).private());
    let set = Set::of(&["a"]);
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    w.registry.stage(&g, &set.items(&blobs)).unwrap();
    assert_eq!(commit(&mut w, &set), Ok(CommitAnswer::Committed));
    let denied = Some(SeamError::GrantRefused(GrantFault::Denied));
    let none = Grant::none();
    assert_eq!(w.registry.observe(&none, &set.coords[0], &[]).err(), denied);
    assert_eq!(w.registry.fetch(&none, &set.coords[0]).err(), denied);
    assert_eq!(w.registry.newest(&none, "stable", "a").err(), denied);
    let scope = ListScope {
        channel: "stable".to_owned(),
        prefix: String::new(),
    };
    assert_eq!(w.registry.list(&none, &scope).err(), denied);
    // The same reads with a read grant are answered.
    let g = w.grant(GrantPhase::Read, &set.coords);
    assert!(matches!(
        w.registry.observe(&g, &set.coords[0], &[]),
        Ok(Presence::Present(_))
    ));
    assert_eq!(
        w.registry.fetch(&g, &set.coords[0]),
        Ok(Some(b"a".to_vec()))
    );
    assert_eq!(w.registry.newest(&g, "stable", "a").unwrap().len(), 1);
    assert_eq!(w.registry.list(&g, &scope).unwrap().len(), 1);
}

#[test]
fn a_channel_that_commits_per_coordinate_refuses_a_set_and_takes_one_at_a_time() {
    let clock = ManualClock::at(1_000_000);
    let mut w = World::with(
        &clock,
        FakeRegistry::new(&clock, LOCATION).commit_per_coordinate(),
    );
    let set = Set::of(&["a", "b"]);
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    let items = set.items(&blobs);
    assert_eq!(w.registry.stage(&g, &items), Ok(StageAnswer::Staged));
    let g = w.grant(GrantPhase::Commit, &set.coords);
    let answer = w.registry.commit(&g, &items);
    assert!(is_permanent(&answer), "{answer:?}");
    assert_eq!(w.registry.bytes(&set.coords[0]), None);
    assert_eq!(w.registry.bytes(&set.coords[1]), None);
    assert!(w.registry.commit_log().is_empty());
    for (at, item) in items.iter().enumerate() {
        assert_eq!(
            w.registry.commit(&g, std::slice::from_ref(item)),
            Ok(CommitAnswer::Committed)
        );
        assert_eq!(w.registry.commit_log().len(), at + 1);
    }
    assert_eq!(w.registry.bytes(&set.coords[1]), Some(&b"b"[..]));
}

#[test]
fn staged_state_of_another_writer_is_seen_with_its_digest_only_when_made_visible() {
    let mut w = World::new();
    let set = Set::of(&["a"]);
    w.registry.stage_other_bytes(&set.coords[0], b"theirs");
    w.registry.others_staged_visible();
    let g = w.grant(GrantPhase::Read, &set.coords);
    let Ok(Presence::Staged(held)) = w.registry.observe(&g, &set.coords[0], &[]) else {
        panic!("a grant sees the staged state of another writer");
    };
    assert_eq!(held.digest, sha(b"theirs"));
    // A stranger still sees nothing.
    assert_eq!(stranger(&mut w, &set, 0), Presence::Absent);
}

#[test]
fn staged_state_of_another_writer_on_a_grouped_registry_is_seen_only_with_a_set() {
    let clock = ManualClock::at(1_000_000);
    let mut w = World::with(
        &clock,
        FakeRegistry::new(&clock, LOCATION).group_addressed(),
    );
    let set = Set::of(&["a"]);
    w.registry.stage_other_bytes(&set.coords[0], b"theirs");
    w.registry.others_staged_visible();
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Read, &set.coords));
    assert_eq!(
        w.registry.observe(&g, &set.coords[0], &[]),
        Ok(Presence::Absent)
    );
    assert!(matches!(
        w.registry.observe(&g, &set.coords[0], &set.items(&blobs)),
        Ok(Presence::Staged(_))
    ));
}

mod grouped {
    use super::*;

    fn world() -> World {
        let clock = ManualClock::at(1_000_000);
        World::with(
            &clock,
            FakeRegistry::new(&clock, LOCATION).group_addressed(),
        )
    }

    #[test]
    fn staged_state_is_found_only_through_the_set_it_was_staged_under() {
        let mut w = world();
        let set = Set::of(&["a", "b"]);
        let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
        let items = set.items(&blobs);
        w.registry.stage(&g, &items).unwrap();
        assert!(matches!(
            w.registry.observe(&g, &set.coords[0], &items),
            Ok(Presence::Staged(_))
        ));
        assert_eq!(
            w.registry.observe(&g, &set.coords[0], &[]),
            Ok(Presence::Absent)
        );
        // A set that differs in one member is another group.
        let other = Set::of(&["a", "c"]);
        let other_blobs = other.blobs();
        assert_eq!(
            w.registry
                .observe(&g, &set.coords[0], &other.items(&other_blobs)),
            Ok(Presence::Absent)
        );
        // A stranger sees no staged state even with the right set.
        assert_eq!(
            w.registry.observe(&Grant::none(), &set.coords[0], &items),
            Ok(Presence::Absent)
        );
    }

    #[test]
    fn a_private_observe_reads_the_group_then_the_index() {
        let mut w = world();
        let set = Set::of(&["a"]);
        let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Read, &set.coords));
        w.registry
            .observe(&g, &set.coords[0], &set.items(&blobs))
            .unwrap();
        w.registry.observe(&g, &set.coords[0], &[]).unwrap();
        assert_eq!(
            w.registry.read_log(),
            [Read::Group, Read::Index, Read::Index]
        );
    }

    #[test]
    fn a_coordinate_committed_in_another_group_reads_as_present() {
        let mut w = world();
        let set = Set::of(&["a"]);
        w.registry.present_different(&set.coords[0], b"theirs");
        let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Read, &set.coords));
        let Ok(Presence::Present(held)) =
            w.registry.observe(&g, &set.coords[0], &set.items(&blobs))
        else {
            panic!("the index answer wins");
        };
        assert_eq!(held.digest, sha(b"theirs"));
    }

    #[test]
    fn a_failed_group_read_or_index_read_fails_the_observe() {
        let mut w = world();
        let set = Set::of(&["a"]);
        let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Read, &set.coords));
        let items = set.items(&blobs);
        w.registry.fail_group_read();
        assert!(matches!(
            w.registry.observe(&g, &set.coords[0], &items),
            Err(SeamError::Unknown(_))
        ));
        assert_eq!(w.registry.read_log(), [Read::Group]);
        let mut w = world();
        let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Read, &set.coords));
        w.registry.fail_index_read();
        assert!(matches!(
            w.registry.observe(&g, &set.coords[0], &set.items(&blobs)),
            Err(SeamError::Unknown(_))
        ));
        assert_eq!(w.registry.read_log(), [Read::Group, Read::Index]);
    }
}
