// SPDX-License-Identifier: Apache-2.0
//! The action input digest: what decides, on a resume, whether a confirmed action is the same
//! work, and what it leaves out.

mod common;

use common::*;
use ucdk_core::{Framed, Hash256};
use ucdk_release::action_input::{Given, InputError, action_input};
use ucdk_release::hashes::build_inputs_hash;
use ucdk_release::machine::{Checked, builds};
use ucdk_release_proto::v1::{BuildStamp, Machine, ValidationKind};

type Edit = fn(&mut Machine);

fn set() -> Hash256 {
    Hash256::from_hex(&"3".repeat(64)).expect("a hash")
}

fn given<'a>(stamp: &'a BuildStamp, set: Option<&'a Hash256>) -> Given<'a> {
    Given {
        revision: REVISION,
        stamp,
        set,
    }
}

/// The digest of `action` of `stage`, with the set when the action consumes one.
fn digest(machine: Machine, stage: &str, action: &str, with_set: bool) -> Hash256 {
    digest_stamped(machine, stage, action, with_set, &stamp())
}

fn digest_stamped(
    machine: Machine,
    stage: &str,
    action: &str,
    with_set: bool,
    stamp: &BuildStamp,
) -> Hash256 {
    let checked = Checked::read(machine).expect("the machine is read");
    let sealed = set();
    let set = with_set.then_some(&sealed);
    action_input(&checked, stage, action, &given(stamp, set)).expect("a digest")
}

fn by_hand(fields: &[&str]) -> Hash256 {
    fields
        .iter()
        .fold(Framed::new(b"ucdk/release-action-input/1\n"), |h, f| {
            h.field(f.as_bytes())
        })
        .finish()
}

/// The three actions with their stage and whether they consume the set.
const ACTIONS: [(&str, &str, bool); 4] = [
    ("build", "build", false),
    ("build", "test", false),
    ("publish", "publish", true),
    ("publish", "smoke", true),
];

#[test]
fn golden_text() {
    // Each kind written out by hand from the definition, after the tag: a text naming the
    // kind, and then the fields of that kind.
    let inputs = {
        let checked = Checked::read(good()).expect("read");
        let (_, build) = builds(checked.stage("build").expect("stage"))
            .next()
            .expect("build");
        build_inputs_hash(&checked, build, REVISION, &stamp())
    };
    let build = by_hand(&["build", inputs.as_str()]);
    assert_eq!(digest(good(), "build", "build", false), build);
    let test = by_hand(&[
        "test",
        REVISION,
        PLATFORM,
        "1.2.3",
        "41",
        "abc123",
        // The build system as authored: kind, binary, config file, mode, no flags, one variable.
        "1",
        "buck2",
        "release/release.buckconfig",
        "release",
        "0",
        "1",
        "PATH",
        "/opt/tools/bin:/usr/bin:/bin",
        // One target: its label, no flags and no environment.
        "1",
        "//release:set_test",
        "0",
        "0",
    ]);
    assert_eq!(digest(good(), "build", "test", false), test);
    let set_text = set();
    let consumed = |kind| {
        by_hand(&[
            kind,
            set_text.as_str(),
            REVISION,
            PLATFORM,
            "registry",
            "2",
            "ucdk-cli",
            "ucdk-lib",
        ])
    };
    let publish = consumed("publish");
    let smoke = consumed("smoke");
    assert_eq!(digest(good(), "publish", "publish", true), publish);
    assert_eq!(digest(good(), "publish", "smoke", true), smoke);
    assert_ne!(publish, smoke, "the kind is the first field");
    let pinned = [
        (
            "build",
            &build,
            "7d110fb709c4ca0a04e4e92c4c4202f2aad2ff0614099d4d301ce816c855719e",
        ),
        (
            "test",
            &test,
            "42b024bae3057b1613966c601ec331d6396cde2803f1dcf87c5982eb407cd018",
        ),
        (
            "publish",
            &publish,
            "dba69bb2dedb9dab116e5f117c3c25b95ae62d3c3f2089b098d86def493acae6",
        ),
        (
            "smoke",
            &smoke,
            "23e00981377bdefc25c032fd0c3ab5fc4bcf74f28a25c307c3ca5bfa113e96f6",
        ),
    ];
    for (kind, hash, text) in pinned {
        assert_eq!(
            hash.as_str(),
            text,
            "the {kind} digest is a promise: a change here makes every confirmed action of that kind INPUTS_CHANGED"
        );
    }
}

#[test]
fn a_validation_edit_does_not_change_it() {
    let edits: Vec<(&str, Edit)> = vec![
        ("a build validation renamed", |m| {
            action_mut(m, "build", "build").validations[0].name = "closed".to_owned();
        }),
        ("a build validation added", |m| {
            action_mut(m, "build", "build").validations.push(validation(
                "extra",
                ValidationKind::SetClosure,
                0,
            ));
        }),
        ("a smoke validation changed", |m| {
            action_mut(m, "publish", "smoke").validations[0].name = "seen".to_owned();
        }),
        ("a publish validation added", |m| {
            action_mut(m, "publish", "publish")
                .validations
                .push(validation("pre", ValidationKind::SetClosure, 0));
        }),
        ("a stage validation added", |m| {
            stage_mut(m, "publish").validations.push(validation(
                "late",
                ValidationKind::ChannelReadback,
                2,
            ));
        }),
    ];
    for (stage, action, with_set) in ACTIONS {
        let base = digest(good(), stage, action, with_set);
        for (what, edit) in &edits {
            let mut m = good();
            edit(&mut m);
            assert_eq!(digest(m, stage, action, with_set), base, "{action}: {what}");
        }
    }
}

#[test]
fn a_bound_edit_does_not_change_it() {
    let edits: Vec<(&str, Edit)> = vec![
        ("attempts", |m| {
            publish_mut(m, "publish", "publish").attempts = 7
        }),
        ("polls", |m| publish_mut(m, "publish", "publish").polls = 9),
        ("wait polls", |m| {
            action_mut(m, "publish", "smoke").validations[0].wait_polls = 1;
        }),
    ];
    for (stage, action, with_set) in ACTIONS {
        let base = digest(good(), stage, action, with_set);
        for (what, edit) in &edits {
            let mut m = good();
            edit(&mut m);
            assert_eq!(digest(m, stage, action, with_set), base, "{action}: {what}");
        }
    }
}

#[test]
fn a_flag_or_target_order_edit_changes_it() {
    // A test with two targets, so the order they run in can be swapped.
    let two_targets = || {
        let mut m = good();
        test_mut(&mut m, "build", "test")
            .targets
            .push(target("//release:more_test"));
        m
    };
    let base = digest(two_targets(), "build", "test", false);
    let edits: Vec<(&str, Edit)> = vec![
        ("the order of the targets", |m| {
            test_mut(m, "build", "test").targets.reverse();
        }),
        ("a target label", |m| {
            test_mut(m, "build", "test").targets[1].label = "//release:other_test".to_owned();
        }),
        ("a target flag", |m| {
            test_mut(m, "build", "test").targets[1]
                .flags
                .push("--a".to_owned());
        }),
        ("the order of a target's flags", |m| {
            let t = &mut test_mut(m, "build", "test").targets[1];
            t.flags = strings(&["--a", "--b"]);
        }),
        ("a target environment", |m| {
            test_mut(m, "build", "test").targets[1]
                .env
                .insert("K".to_owned(), "V".to_owned());
        }),
        ("a target removed", |m| {
            test_mut(m, "build", "test").targets.pop();
        }),
        ("a base flag", |m| {
            system_mut(m).flags.push("--keep-going".to_owned())
        }),
        ("the base environment", |m| {
            system_mut(m).env.insert("HOME".to_owned(), "/h".to_owned());
        }),
        ("the binary", |m| {
            system_mut(m).binary = "/usr/bin/buck2".to_owned()
        }),
        ("the mode", |m| system_mut(m).mode.clear()),
        ("the config file", |m| system_mut(m).config_file.clear()),
    ];
    let mut seen = vec![base.to_string()];
    for (what, edit) in &edits {
        let mut m = two_targets();
        edit(&mut m);
        let changed = digest(m, "build", "test", false);
        assert_ne!(changed, base, "{what}");
        seen.push(changed.to_string());
    }
    let count = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), count, "two edits shared a digest");

    // A build's flags and targets reach it through the build-inputs hash.
    let build = digest(good(), "build", "build", false);
    let mut flagged = good();
    artifact_mut(&mut flagged, "ucdk-cli")
        .target
        .as_mut()
        .expect("target")
        .flags
        .swap(0, 1);
    assert_ne!(digest(flagged, "build", "build", false), build);

    // The order a Publish or Smoke action lists its artifacts in is the order it publishes in.
    for action in ["publish", "smoke"] {
        let base = digest(good(), "publish", action, true);
        let mut reversed = good();
        match action {
            "publish" => publish_mut(&mut reversed, "publish", action)
                .artifacts
                .reverse(),
            _ => smoke_mut(&mut reversed, "publish", action)
                .artifacts
                .reverse(),
        }
        assert_ne!(digest(reversed, "publish", action, true), base, "{action}");
    }
}

#[test]
fn what_a_run_gives_an_action_changes_it() {
    let checked = Checked::read(good()).expect("read");
    let other_set = Hash256::from_hex(&"4".repeat(64)).expect("a hash");
    for (stage, action, with_set) in ACTIONS {
        let sealed = set();
        let ask = |revision: &str, stamp: &BuildStamp, set: &Hash256| {
            let set = with_set.then_some(set);
            let given = Given {
                revision,
                stamp,
                set,
            };
            action_input(&checked, stage, action, &given).expect("a digest")
        };
        let base = ask(REVISION, &stamp(), &sealed);
        let mut changes = vec![ask("other", &stamp(), &sealed)];
        if with_set {
            changes.push(ask(REVISION, &stamp(), &other_set));
        }
        let edits: [fn(&mut BuildStamp); 3] = [
            |s| s.version = "9".to_owned(),
            |s| s.build_number += 1,
            |s| s.source_commit = "def".to_owned(),
        ];
        // A Publish or Smoke action reads the set and not the stamp: the set is the build.
        if !with_set {
            for edit in edits {
                let mut s = stamp();
                edit(&mut s);
                changes.push(ask(REVISION, &s, &sealed));
            }
        }
        for changed in &changes {
            assert_ne!(*changed, base, "{action}");
        }
    }
}

#[test]
fn the_platform_and_the_channel_of_a_publish_or_smoke_are_in_it() {
    for action in ["publish", "smoke"] {
        let base = digest(good(), "publish", action, true);
        // The stage follows its build to another platform.
        let mut elsewhere = good();
        build_mut(&mut elsewhere, "build", "build").platform = "any".to_owned();
        publish_mut(&mut elsewhere, "publish", "publish").platform = "any".to_owned();
        smoke_mut(&mut elsewhere, "publish", "smoke").platform = "any".to_owned();
        assert_ne!(digest(elsewhere, "publish", action, true), base, "{action}");
        // The stage publishes to, and reads, another channel, which holds its own location.
        let mut other = good();
        let mut channel = other.channels[0].clone();
        channel.name = "other".to_owned();
        channel.repositories[0].location = "https://other.example.com/files".to_owned();
        other.channels.push(channel);
        publish_mut(&mut other, "publish", "publish").channel = "other".to_owned();
        smoke_mut(&mut other, "publish", "smoke").channel = "other".to_owned();
        assert_ne!(digest(other, "publish", action, true), base, "{action}");
    }
}

#[test]
fn the_timestamp_and_the_run_are_not_in_it() {
    let mut later = stamp();
    later.timestamp_ms += 86_400_000;
    for (stage, action, with_set) in ACTIONS {
        let base = digest(good(), stage, action, with_set);
        assert_eq!(
            digest_stamped(good(), stage, action, with_set, &later),
            base,
            "{action}"
        );
    }
    // Nor is the name of the action, which is in the name of its record, or another action.
    let base = digest(good(), "build", "build", false);
    let mut renamed = good();
    action_mut(&mut renamed, "build", "build").name = "compile".to_owned();
    assert_eq!(digest(renamed, "build", "compile", false), base);
    let mut other = good();
    test_mut(&mut other, "build", "test").targets[0].label = "//release:other_test".to_owned();
    assert_eq!(digest(other, "build", "build", false), base);
    assert_eq!(
        digest(
            {
                let mut m = good();
                test_mut(&mut m, "build", "test").targets[0].label = "//release:x".to_owned();
                m
            },
            "publish",
            "publish",
            true
        ),
        digest(good(), "publish", "publish", true)
    );
}

#[test]
fn an_action_that_cannot_be_asked_is_refused() {
    let checked = Checked::read(good()).expect("read");
    let sealed = set();
    let stamp = stamp();
    let ask = |stage: &str, action: &str, set: Option<&Hash256>| {
        action_input(&checked, stage, action, &given(&stamp, set))
    };
    assert_eq!(
        ask("ghost", "build", None),
        Err(InputError::UnknownStage("ghost".to_owned()))
    );
    assert_eq!(
        ask("build", "ghost", None),
        Err(InputError::UnknownAction {
            stage: "build".to_owned(),
            action: "ghost".to_owned()
        })
    );
    assert_eq!(
        ask("build", "build", Some(&sealed)),
        Err(InputError::SetNotAllowed("build".to_owned()))
    );
    assert_eq!(
        ask("build", "test", Some(&sealed)),
        Err(InputError::SetNotAllowed("test".to_owned()))
    );
    assert_eq!(
        ask("publish", "publish", None),
        Err(InputError::SetRequired("publish".to_owned()))
    );
    assert_eq!(
        ask("publish", "smoke", None),
        Err(InputError::SetRequired("smoke".to_owned()))
    );
    // Each refusal names what it is about.
    for (err, named) in [
        (InputError::UnknownStage("ghost".to_owned()), "ghost"),
        (
            InputError::UnknownAction {
                stage: "build".to_owned(),
                action: "ghost".to_owned(),
            },
            "ghost",
        ),
        (InputError::SetNotAllowed("build".to_owned()), "build"),
        (InputError::SetRequired("publish".to_owned()), "publish"),
    ] {
        assert!(err.to_string().contains(named), "{err}");
    }
}
