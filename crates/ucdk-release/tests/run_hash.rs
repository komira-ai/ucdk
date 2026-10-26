// SPDX-License-Identifier: Apache-2.0
//! The run hash: what an approver agreed to run, and nothing that was observed.

mod common;

use common::*;
use ucdk_core::{Framed, Hash256};
use ucdk_release::hashes::{RunHashError, machine_hash, run_hash};
use ucdk_release::machine::Checked;
use ucdk_release::selection::Selection;
use ucdk_release_proto::v1::{BuildStamp, Machine, Trigger};

fn set() -> Hash256 {
    Hash256::from_hex(&"3".repeat(64)).expect("a hash")
}

fn selection(texts: &[&str]) -> Selection {
    Selection::parse(&strings(texts)).expect("selectors")
}

/// The worked example with a stage `later`, which needs the publish stage and holds only a
/// Test action, so it names no platform and consumes no member.
fn with_test_only_stage() -> Machine {
    let mut m = good();
    m.stages.push(stage(
        "later",
        Trigger::Release,
        &["publish"],
        vec![test_action("install", &["//release:install_test"])],
    ));
    m
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

/// The hash of `fields`, each framed, under the tag of the run hash.
fn by_hand(fields: &[&str]) -> Hash256 {
    fields
        .iter()
        .fold(Framed::new(b"ucdk/release-run/1\n"), |h, f| {
            h.field(f.as_bytes())
        })
        .finish()
}

#[test]
fn the_hash_is_the_definition_written_out() {
    let machine = machine_hash(&good());
    // The machine hash, the stage, the selectors (count, then each, sorted), the revision, the
    // stage's platform and the stamp without its timestamp; for a stage that holds a Publish or
    // Smoke action, the sealed set last.
    let build = [
        machine.as_str(),
        "build",
        "1",
        "action:build",
        REVISION,
        PLATFORM,
        "1.2.3",
        "41",
        "abc123",
    ];
    assert_eq!(build_hash(&stamp(), &["action:build"]), by_hand(&build));
    let set_text = set();
    let publish = [
        machine.as_str(),
        "publish",
        "0",
        REVISION,
        PLATFORM,
        "1.2.3",
        "41",
        "abc123",
        set_text.as_str(),
    ];
    assert_eq!(publish_hash(&stamp(), &set()), by_hand(&publish));
    assert_eq!(
        by_hand(&publish).as_str(),
        "ee6e2598cb03bd9281c3ddd7bc34399d7fa73c7aff928a13d1e95e6a80588661",
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
fn includes_the_set_only_for_a_stage_with_a_publish_or_smoke_action() {
    let checked = Checked::read(with_test_only_stage()).expect("read");
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
    // A stage that holds a Build action, or only Test actions, is given no set.
    assert_eq!(ask("build", Some(&set())), Err(RunHashError::SetNotAllowed));
    assert_eq!(ask("check", Some(&set())), Err(RunHashError::SetNotAllowed));
    assert_eq!(ask("later", Some(&set())), Err(RunHashError::SetNotAllowed));
    assert!(ask("build", None).is_ok());
    assert!(ask("later", None).is_ok());
    // A stage that holds a Publish or Smoke action must be.
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
fn a_stage_that_names_no_platform_hashes_an_empty_one() {
    let m = with_test_only_stage();
    let machine = machine_hash(&m);
    let checked = Checked::read(m).expect("read");
    let hash = run_hash(
        &checked,
        "later",
        &Selection::default(),
        REVISION,
        &stamp(),
        None,
    )
    .expect("a hash");
    // The platform field is present and empty: the same fields as any stage's, in the same
    // order, with an empty text where the platform goes and no set at the end.
    let mut fields = [
        machine.as_str(),
        "later",
        "0",
        REVISION,
        "",
        "1.2.3",
        "41",
        "abc123",
    ];
    assert_eq!(hash, by_hand(&fields));
    fields[4] = PLATFORM;
    assert_ne!(hash, by_hand(&fields));
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
        ask("build", &selection(&["action:build"]), REVISION, &stamp()),
        ask(
            "build",
            &selection(&["validation:closure"]),
            REVISION,
            &stamp(),
        ),
        ask("build", &none, "other", &stamp()),
    ];
    let edits: [fn(&mut BuildStamp); 3] = [
        |s| s.version = "9".to_owned(),
        |s| s.build_number += 1,
        |s| s.source_commit = "def".to_owned(),
    ];
    for edit in edits {
        let mut s = stamp();
        edit(&mut s);
        changes.push(ask("build", &none, REVISION, &s));
    }
    let mut renamed = good();
    renamed.name = "other".to_owned();
    let other_machine = Checked::read(renamed).expect("read");
    changes
        .push(run_hash(&other_machine, "build", &none, REVISION, &stamp(), None).expect("a hash"));
    // The stage's platform is covered: the same stage with another platform.
    let mut elsewhere = good();
    build_mut(&mut elsewhere, "build", "build").platform = "any".to_owned();
    publish_mut(&mut elsewhere, "publish", "publish").platform = "any".to_owned();
    smoke_mut(&mut elsewhere, "publish", "smoke").platform = "any".to_owned();
    let elsewhere = Checked::read(elsewhere).expect("read");
    changes.push(run_hash(&elsewhere, "build", &none, REVISION, &stamp(), None).expect("a hash"));
    for hash in &changes {
        assert_ne!(*hash, base);
    }
    let mut seen: Vec<&str> = changes.iter().map(Hash256::as_str).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), changes.len());
}

/// The stamp a driver composes from the stamp flags and the source: the same four values,
/// built afresh each time, as `plan` and `run` each build theirs.
fn composed(version: &str, number: u64, commit: &str, timestamp_ms: u64) -> BuildStamp {
    BuildStamp {
        version: version.to_owned(),
        build_number: number,
        source_commit: commit.to_owned(),
        timestamp_ms,
    }
}

#[test]
fn plan_and_run_agree_for_a_build_stage() {
    let checked = Checked::read(good()).expect("read");
    // `plan` and `run` each read the stamp inputs, each parse the selectors, and call the one
    // function; the clock is read at different moments and the selectors come in another order.
    let planned = run_hash(
        &checked,
        "build",
        &selection(&["validation:closure", "action:build"]),
        REVISION,
        &composed("1.2.3", 41, "abc123", 1_700_000_000_000),
        None,
    )
    .expect("a hash");
    let run = run_hash(
        &checked,
        "build",
        &selection(&["action:build", "validation:closure"]),
        REVISION,
        &composed("1.2.3", 41, "abc123", 1_700_000_999_999),
        None,
    )
    .expect("a hash");
    assert_eq!(planned, run);
    // The agreement is not an accident of ignoring the stamp: another build number differs.
    let other = run_hash(
        &checked,
        "build",
        &selection(&["action:build", "validation:closure"]),
        REVISION,
        &composed("1.2.3", 42, "abc123", 1_700_000_000_000),
        None,
    )
    .expect("a hash");
    assert_ne!(planned, other);
}

#[test]
fn plan_and_run_agree_for_a_stage_without_build() {
    let checked = Checked::read(good()).expect("read");
    // A stage that holds a Publish action takes the stamp the run record holds, and the set it
    // consumes; `plan` reads both from the record and `run` reads them again.
    let ask = |timestamp_ms: u64| {
        run_hash(
            &checked,
            "publish",
            &Selection::default(),
            REVISION,
            &composed("1.2.3", 41, "abc123", timestamp_ms),
            Some(&set()),
        )
        .expect("a hash")
    };
    assert_eq!(ask(1), ask(2));
    assert_eq!(ask(1), publish_hash(&stamp(), &set()));
}

#[test]
fn a_change_check_stage_takes_its_version_from_the_flag() {
    // The stage holds a Test action, so its stamp comes from the flags of the run: the version
    // the driver gives is the version approved, and another one is another approval.
    let checked = Checked::read(good()).expect("read");
    let ask = |version: &str| {
        run_hash(
            &checked,
            "check",
            &Selection::default(),
            REVISION,
            &composed(version, 41, "abc123", 7),
            None,
        )
        .expect("a hash")
    };
    assert_eq!(ask("1.2.3"), ask("1.2.3"));
    assert_ne!(ask("1.2.3"), ask("1.2.4"));
    let machine = machine_hash(&good());
    let fields = [
        machine.as_str(),
        "check",
        "0",
        REVISION,
        "",
        "1.2.3",
        "41",
        "abc123",
    ];
    assert_eq!(ask("1.2.3"), by_hand(&fields));
}

#[test]
fn a_test_only_stage_after_a_publish_stage_takes_the_stamp_flags_and_the_admission_inputs() {
    let checked = Checked::read(with_test_only_stage()).expect("read");
    let ask = |stamp: &BuildStamp, sealed: Option<&Hash256>| {
        run_hash(
            &checked,
            "later",
            &Selection::default(),
            REVISION,
            stamp,
            sealed,
        )
    };
    // It takes the stamp flags: the version, the build number and the commit are covered.
    let base = ask(&stamp(), None).expect("a hash");
    assert_ne!(
        base,
        ask(&composed("2", 41, "abc123", 0), None).expect("a hash")
    );
    assert_ne!(
        base,
        ask(&composed("1.2.3", 42, "abc123", 0), None).expect("a hash")
    );
    assert_ne!(
        base,
        ask(&composed("1.2.3", 41, "def", 0), None).expect("a hash")
    );
    // The set it reads from the stage it needs is for admission: the run hash neither covers it
    // nor takes it, so a stage that consumes no member cannot be approved with one.
    assert_eq!(
        ask(&stamp(), Some(&set())),
        Err(RunHashError::SetNotAllowed)
    );
    // It is not a stage that publishes, so it hashes an empty platform.
    let machine = machine_hash(&with_test_only_stage());
    let fields = [
        machine.as_str(),
        "later",
        "0",
        REVISION,
        "",
        "1.2.3",
        "41",
        "abc123",
    ];
    assert_eq!(base, by_hand(&fields));
}

#[test]
fn the_selectors_are_covered_in_sorted_order_and_each_once() {
    let a = build_hash(&stamp(), &["validation:closure", "action:build"]);
    let b = build_hash(
        &stamp(),
        &["action:build", "validation:closure", "action:build"],
    );
    assert_eq!(a, b);
}
