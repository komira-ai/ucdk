// SPDX-License-Identifier: Apache-2.0
//! The build-inputs hash: what a stored set was built from, and so what may adopt it.

mod common;

use common::*;
use ucdk_core::{Framed, Hash256};
use ucdk_release::hashes::build_inputs_hash;
use ucdk_release::machine::{Checked, builds};
use ucdk_release_proto::v1::step::Action;
use ucdk_release_proto::v1::{ArtifactType, BuildStamp, Machine, Subject};

const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";

/// The hash for the BUILD step of stage `build` of `machine`.
fn inputs(machine: Machine, stamp: &BuildStamp) -> Hash256 {
    let checked = Checked::read(machine).expect("the machine is read");
    let stage = checked.stage("build").expect("stage");
    let (_, build) = builds(stage).next().expect("a build step");
    build_inputs_hash(&checked, build, REVISION, stamp)
}

/// `good()` with `lib` (needed by `tool`) and `unused` (needed by nothing the step builds).
fn with_lib() -> Machine {
    let mut m = good();
    m.artifacts[0].needs = vec!["lib".to_owned()];
    m.artifacts.push(decl("lib", ArtifactType::File, &[]));
    m.artifacts.push(decl("unused", ArtifactType::File, &[]));
    m
}

#[test]
fn the_hash_is_the_definition_written_out() {
    // The platform, revision and stamp without its timestamp, then the declarations in
    // dependency order: each is name, type number, executable, arguments (count, then each),
    // needs (count, then each) and the independence flag.
    let fields = [
        "linux-x86_64",
        REVISION,
        "1.2.3",
        "41",
        "abc123",
        "1",
        "tool",
        "1",
        "make",
        "2",
        "tool",
        "{out_dir}",
        "0",
        "0",
    ];
    let by_hand = fields
        .iter()
        .fold(Framed::new(b"ucdk/release-build-inputs/1\n"), |h, f| {
            h.field(f.as_bytes())
        })
        .finish();
    assert_eq!(inputs(good(), &stamp()), by_hand);
    assert_eq!(
        by_hand.as_str(),
        "9b351db4fad12b4e72034dff41394b238dcc8d1d76b265850cdb3e630479343c",
        "the hash is a promise: a change here stops a stored set being adopted"
    );
}

#[test]
fn an_unrelated_stage_edit_does_not_change_it() {
    let base = inputs(good(), &stamp());
    let edits: Vec<Edit> = vec![
        Box::new(|m| m.stages[2].environment = "publish".to_owned()),
        Box::new(|m| m.channels[0].monotonic = true),
        Box::new(|m| {
            m.stages[1].steps[0].validations[0].rows[0].asserts = "another sentence".to_owned();
        }),
        Box::new(|m| {
            m.stages[1].steps[0]
                .validations
                .push(validation("extra", "fake", Subject::LocalSet));
        }),
        Box::new(|m| m.stamp_ignore.push("other/**".to_owned())),
        Box::new(|m| m.name = "renamed".to_owned()),
        Box::new(|m| {
            if let Some(Action::Publish(p)) = &mut m.stages[2].steps[0].action {
                p.attempts = 5;
            }
        }),
    ];
    for edit in &edits {
        let mut m = good();
        edit(&mut m);
        assert_eq!(inputs(m, &stamp()), base);
    }
}

#[test]
fn a_builder_or_artifact_edit_changes_it() {
    let base = inputs(with_lib(), &stamp());
    let edits: Vec<(&str, Edit)> = vec![
        (
            "executable",
            Box::new(|m| m.artifacts[0].builder.as_mut().expect("b").executable = "cc".to_owned()),
        ),
        (
            "args",
            Box::new(|m| {
                m.artifacts[0]
                    .builder
                    .as_mut()
                    .expect("b")
                    .args
                    .push("-O2".to_owned());
            }),
        ),
        (
            "needed artifact's builder",
            Box::new(|m| m.artifacts[1].builder.as_mut().expect("b").args.clear()),
        ),
        (
            "type",
            Box::new(|m| m.artifacts[1].r#type = ArtifactType::Npm as i32),
        ),
        (
            "independent version",
            Box::new(|m| m.artifacts[0].independent_version = true),
        ),
        ("needs", Box::new(|m| m.artifacts[0].needs = vec![])),
        ("no builder", Box::new(|m| m.artifacts[0].builder = None)),
    ];
    let mut seen = vec![base.to_string()];
    for (what, edit) in &edits {
        let mut m = with_lib();
        edit(&mut m);
        let changed = inputs(m, &stamp());
        assert_ne!(changed, base, "{what}");
        seen.push(changed.to_string());
    }
    let count = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), count);
}

#[test]
fn an_artifact_needed_by_two_others_is_hashed_once() {
    // `base` is needed by both `left` and `right`, which `tool` needs: the diamond.
    let mut m = good();
    m.artifacts[0].needs = vec!["left".to_owned(), "right".to_owned()];
    m.artifacts
        .push(decl("left", ArtifactType::File, &["base"]));
    m.artifacts
        .push(decl("right", ArtifactType::File, &["base"]));
    m.artifacts.push(decl("base", ArtifactType::File, &[]));
    let base = inputs(m.clone(), &stamp());
    // Naming an artifact twice, or an artifact the others already need, changes nothing.
    let Some(Action::Build(b)) = &mut m.stages[1].steps[0].action else {
        panic!("a build step");
    };
    b.artifacts = vec!["tool".to_owned(), "base".to_owned(), "tool".to_owned()];
    assert_eq!(inputs(m, &stamp()), base);
}

#[test]
fn an_artifact_the_step_does_not_build_or_need_does_not_change_it() {
    let base = inputs(with_lib(), &stamp());
    let mut m = with_lib();
    m.artifacts[2].builder.as_mut().expect("builder").executable = "other".to_owned();
    assert_eq!(inputs(m, &stamp()), base);
}

#[test]
fn the_order_the_artifacts_are_declared_in_does_not_change_it() {
    let base = inputs(with_lib(), &stamp());
    let mut m = with_lib();
    m.artifacts.reverse();
    assert_eq!(inputs(m, &stamp()), base);
}

#[test]
fn the_timestamp_is_not_in_it() {
    let base = inputs(good(), &stamp());
    let mut later = stamp();
    later.timestamp_ms += 1;
    assert_eq!(inputs(good(), &later), base);
}

#[test]
fn the_platform_the_revision_and_the_rest_of_the_stamp_are_in_it() {
    let base = inputs(good(), &stamp());
    for edit in [
        (|s: &mut BuildStamp| s.version = "9".to_owned()) as fn(&mut BuildStamp),
        |s| s.build_number += 1,
        |s| s.source_commit = "def".to_owned(),
    ] {
        let mut s = stamp();
        edit(&mut s);
        assert_ne!(inputs(good(), &s), base);
    }
    let mut m = good();
    // The publish stage follows the build stage to the other platform.
    for at in [1, 2] {
        match &mut m.stages[at].steps[0].action {
            Some(Action::Build(b)) => b.platform = "any".to_owned(),
            Some(Action::Publish(p)) => p.platform = "any".to_owned(),
            None => panic!("an action"),
        }
    }
    assert_ne!(inputs(m, &stamp()), base);
    let checked = Checked::read(good()).expect("read");
    let (_, build) = builds(checked.stage("build").expect("stage"))
        .next()
        .expect("build");
    assert_ne!(build_inputs_hash(&checked, build, "other", &stamp()), base);
}

#[test]
fn a_step_that_names_an_undeclared_artifact_hashes_the_declared_ones() {
    // The reader refuses such a machine, so the hash is only ever asked of a read one; the
    // function itself leaves an unknown name out rather than guess a declaration for it.
    let checked = Checked::read(good()).expect("read");
    let mut build = builds(checked.stage("build").expect("stage"))
        .next()
        .expect("build")
        .1
        .clone();
    let base = build_inputs_hash(&checked, &build, REVISION, &stamp());
    build.artifacts.push("ghost".to_owned());
    let with_ghost = build_inputs_hash(&checked, &build, REVISION, &stamp());
    assert_eq!(with_ghost, base);
}
