// SPDX-License-Identifier: Apache-2.0
//! The run hash: what an approver agreed to run, and nothing that was observed.

mod common;

use common::*;
use ucdk_core::{Framed, Hash256};
use ucdk_release::hashes::{RunHashError, machine_hash, run_hash};
use ucdk_release::machine::Checked;
use ucdk_release::selection::Selection;
use ucdk_release_proto::v1::BuildStamp;

const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";

fn set() -> Hash256 {
    Hash256::from_hex(&"3".repeat(64)).expect("a hash")
}

fn selection(texts: &[&str]) -> Selection {
    Selection::parse(&texts.iter().map(|t| (*t).to_owned()).collect::<Vec<_>>()).expect("selectors")
}

fn build_hash(stamp: &BuildStamp, only: &[&str]) -> Hash256 {
    let checked = Checked::read(good()).expect("read");
    run_hash(&checked, "build", &selection(only), REVISION, stamp, None).expect("a hash")
}

fn publish_hash(stamp: &BuildStamp, sealed: &Hash256) -> Hash256 {
    let checked = Checked::read(good()).expect("read");
    run_hash(
        &checked,
        "publish",
        &Selection::default(),
        REVISION,
        stamp,
        Some(sealed),
    )
    .expect("a hash")
}

#[test]
fn the_hash_is_the_definition_written_out() {
    let machine = machine_hash(&good());
    // The machine hash, the stage, the selectors (count, then each, sorted), the revision, the
    // stage's platform and the stamp without its timestamp; for a stage with no BUILD step, the
    // sealed set last.
    let build = [
        machine.as_str(),
        "build",
        "1",
        "step:compile",
        REVISION,
        "linux-x86_64",
        "1.2.3",
        "41",
        "abc123",
    ];
    let by_hand = |fields: &[&str]| {
        fields
            .iter()
            .fold(Framed::new(b"ucdk/release-run/1\n"), |h, f| {
                h.field(f.as_bytes())
            })
            .finish()
    };
    assert_eq!(build_hash(&stamp(), &["step:compile"]), by_hand(&build));
    let set_text = set();
    let publish = [
        machine.as_str(),
        "publish",
        "0",
        REVISION,
        "linux-x86_64",
        "1.2.3",
        "41",
        "abc123",
        set_text.as_str(),
    ];
    assert_eq!(publish_hash(&stamp(), &set()), by_hand(&publish));
    assert_eq!(
        by_hand(&publish).as_str(),
        "03e637b311260ff430baba3f1d54b3afbd37bb121ce145295ec4c436601cc766",
        "the hash is a promise: a change here voids every approval"
    );
}

#[test]
fn ignores_observed_state() {
    // The function has no parameter for a registry, a ledger, a run id or an attempt, and the
    // timestamp the run record pins is not hashed either.
    let base = build_hash(&stamp(), &[]);
    let mut later = stamp();
    later.timestamp_ms += 86_400_000;
    assert_eq!(build_hash(&later, &[]), base);
    assert_eq!(publish_hash(&later, &set()), publish_hash(&stamp(), &set()));
}

#[test]
fn includes_the_set_only_for_a_stage_without_build() {
    let checked = Checked::read(good()).expect("read");
    let ask = |stage: &str, sealed: Option<&Hash256>| {
        run_hash(
            &checked,
            stage,
            &Selection::default(),
            REVISION,
            &stamp(),
            sealed,
        )
    };
    assert_eq!(ask("build", Some(&set())), Err(RunHashError::SetNotAllowed));
    assert_eq!(ask("publish", None), Err(RunHashError::SetRequired));
    assert_eq!(
        ask("ghost", None),
        Err(RunHashError::UnknownStage("ghost".to_owned()))
    );
    let other = Hash256::from_hex(&"4".repeat(64)).expect("a hash");
    assert_ne!(
        publish_hash(&stamp(), &set()),
        publish_hash(&stamp(), &other)
    );
    for err in [
        RunHashError::SetNotAllowed,
        RunHashError::SetRequired,
        RunHashError::UnknownStage("ghost".to_owned()),
    ] {
        assert!(!err.to_string().is_empty());
    }
}

#[test]
fn everything_it_covers_changes_it() {
    let base = build_hash(&stamp(), &[]);
    let checked = Checked::read(good()).expect("read");
    let ask = |stage: &str, only: &Selection, revision: &str, stamp: &BuildStamp| {
        run_hash(&checked, stage, only, revision, stamp, None).expect("a hash")
    };
    let none = Selection::default();
    let mut changes = vec![
        ask("check", &none, REVISION, &stamp()),
        ask("build", &selection(&["step:compile"]), REVISION, &stamp()),
        ask(
            "build",
            &selection(&["validation:closure"]),
            REVISION,
            &stamp(),
        ),
        ask("build", &none, "other", &stamp()),
    ];
    for edit in [
        (|s: &mut BuildStamp| s.version = "9".to_owned()) as fn(&mut BuildStamp),
        |s| s.build_number += 1,
        |s| s.source_commit = "def".to_owned(),
    ] {
        let mut s = stamp();
        edit(&mut s);
        changes.push(ask("build", &none, REVISION, &s));
    }
    let mut edited = good();
    edited.name = "other".to_owned();
    let other_machine = Checked::read(edited).expect("read");
    changes
        .push(run_hash(&other_machine, "build", &none, REVISION, &stamp(), None).expect("a hash"));
    for hash in &changes {
        assert_ne!(*hash, base);
    }
    let mut seen: Vec<&str> = changes.iter().map(Hash256::as_str).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), changes.len());
}

#[test]
fn the_selectors_are_covered_in_sorted_order_and_each_once() {
    let a = build_hash(&stamp(), &["validation:closure", "step:compile"]);
    let b = build_hash(
        &stamp(),
        &["step:compile", "validation:closure", "step:compile"],
    );
    assert_eq!(a, b);
}

#[test]
fn a_stage_with_no_steps_hashes_with_an_empty_platform() {
    let mut m = good();
    m.stages.push(stage(
        "idle",
        ucdk_release_proto::v1::Trigger::Release,
        &[],
        vec![],
    ));
    let checked = Checked::read(m).expect("read");
    let hash = run_hash(
        &checked,
        "idle",
        &Selection::default(),
        REVISION,
        &stamp(),
        Some(&set()),
    );
    assert!(hash.is_ok());
}
