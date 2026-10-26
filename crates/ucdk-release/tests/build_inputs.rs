// SPDX-License-Identifier: Apache-2.0
//! The build-inputs hash: what a stored set was built from, and so what may adopt it.

mod common;

use common::*;
use ucdk_core::{Framed, Hash256};
use ucdk_release::hashes::build_inputs_hash;
use ucdk_release::machine::{Checked, builds};
use ucdk_release_proto::v1::{
    ArtifactType, BuildStamp, BuildSystemKind, Machine, ValidationKind, Visibility,
};

type Edit = fn(&mut Machine);

/// The hash for the Build action of stage `build` of `machine`.
fn inputs(machine: Machine, stamp: &BuildStamp) -> Hash256 {
    let checked = Checked::read(machine).expect("the machine is read");
    let stage = checked.stage("build").expect("stage");
    let (_, build) = builds(stage).next().expect("a build action");
    build_inputs_hash(&checked, build, REVISION, stamp)
}

/// The fields of the hash of `good()` written out by hand, up to the declarations.
fn head() -> Vec<String> {
    strings(&[
        PLATFORM,
        REVISION,
        "1.2.3",
        "41",
        "abc123",
        // The build system: kind, binary, config file, mode, no base flags, one variable.
        "1",
        "buck2",
        "release/release.buckconfig",
        "release",
        "0",
        "1",
        "PATH",
        "/opt/tools/bin:/usr/bin:/bin",
    ])
}

/// The hash of `fields`, each framed, under the tag of the build-inputs hash.
fn by_hand(fields: &[String]) -> Hash256 {
    fields
        .iter()
        .fold(Framed::new(b"ucdk/release-build-inputs/1\n"), |h, f| {
            h.field(f.as_bytes())
        })
        .finish()
}

/// The fields of one declaration of the example: a target with the stamped flags.
fn declaration(name: &str, label: &str, needs: &[&str]) -> Vec<String> {
    let mut fields = strings(&[
        name,
        "1",
        label,
        "4",
        "-c",
        "release.version={release_version}",
        "-c",
        "release.commit={source_commit}",
        "0",
    ]);
    fields.push(needs.len().to_string());
    fields.extend(strings(needs));
    fields.push("0".to_owned());
    fields
}

/// The artifacts the Build actions of the example build, and the Publish and Smoke actions use.
fn relist(m: &mut Machine, built: &[&str], published: &[&str]) {
    build_mut(m, "check", "build").artifacts = strings(built);
    build_mut(m, "build", "build").artifacts = strings(built);
    publish_mut(m, "publish", "publish").artifacts = strings(published);
    smoke_mut(m, "publish", "smoke").artifacts = strings(published);
}

/// `good()` with `lib` needed by `ucdk-cli`, and `unused` needed by nothing the action builds.
fn with_lib() -> Machine {
    let mut m = good();
    artifact_mut(&mut m, "ucdk-cli").needs = strings(&["ucdk-lib"]);
    m.artifacts
        .push(file_artifact("unused", "//release:unused"));
    m
}

#[test]
fn the_hash_is_the_definition_written_out() {
    // The platform, revision and stamp without its timestamp, the build system as authored,
    // then a count of declarations and each in dependency order: name, type number, label, the
    // target's flags (count, then each), its environment (count, then each pair), `needs`
    // (count, then each) and the labels (count, then each pair).
    let mut fields = head();
    fields.push("2".to_owned());
    fields.extend(declaration("ucdk-cli", "//release:ucdk_cli_package", &[]));
    fields.extend(declaration("ucdk-lib", "//release:ucdk_lib_package", &[]));
    assert_eq!(inputs(good(), &stamp()), by_hand(&fields));
    assert_eq!(
        by_hand(&fields).as_str(),
        "0936b4c6c7ae819fc42335b32fdfd4434e895f22409b661402548019fc3b2ed8",
        "the hash is a promise: a change here stops a stored set being adopted"
    );
}

#[test]
fn the_declarations_are_in_dependency_order_and_ties_go_by_name() {
    // `a` needs `z`, so `z` comes first although `a` sorts first; `m` and `n` are ready at once
    // with `z` and come after it only because they need it.
    let mut m = good();
    m.artifacts.clear();
    for (name, needs) in [("a", &["z"][..]), ("n", &["z"]), ("m", &["z"]), ("z", &[])] {
        let mut decl = file_artifact(name, &format!("//release:{name}"));
        decl.needs = strings(needs);
        m.artifacts.push(decl);
    }
    relist(&mut m, &["a", "n", "m", "z"], &["a", "z"]);
    let mut fields = head();
    fields.push("4".to_owned());
    for (name, needs) in [("z", &[][..]), ("a", &["z"]), ("m", &["z"]), ("n", &["z"])] {
        fields.extend(declaration(name, &format!("//release:{name}"), needs));
    }
    assert_eq!(inputs(m, &stamp()), by_hand(&fields));
}

#[test]
fn an_unrelated_stage_edit_does_not_change_it() {
    let base = inputs(good(), &stamp());
    let edits: Vec<(&str, Edit)> = vec![
        ("build stage environment", |m| {
            stage_mut(m, "build").environment = "other".to_owned();
        }),
        ("channel", |m| m.channels[0].monotonic = true),
        ("channel trust", |m| {
            federated_mut(m).audience = "other.example.com".to_owned();
        }),
        ("a validation of the action", |m| {
            action_mut(m, "build", "build").validations[0].name = "closed".to_owned();
        }),
        ("another validation", |m| {
            action_mut(m, "build", "build").validations.push(validation(
                "extra",
                ValidationKind::SetClosure,
                0,
            ));
        }),
        ("a stage validation", |m| {
            stage_mut(m, "publish").validations.push(validation(
                "late",
                ValidationKind::ChannelReadback,
                0,
            ));
        }),
        ("stamp ignore", |m| {
            m.stamp_ignore.push("other/**".to_owned());
        }),
        ("machine name", |m| m.name = "renamed".to_owned()),
        ("publish bound", |m| {
            publish_mut(m, "publish", "publish").attempts = 5;
        }),
        ("another stage", |m| {
            stage_mut(m, "check").actions.pop();
        }),
        ("the action's name", |m| {
            action_mut(m, "build", "build").name = "compile".to_owned();
        }),
    ];
    for (what, edit) in &edits {
        let mut m = good();
        edit(&mut m);
        assert_eq!(inputs(m, &stamp()), base, "{what}");
    }
}

#[test]
fn a_test_edit_does_not_change_it() {
    // Test actions are not inputs to a set, so editing a test target never defeats adoption.
    let base = inputs(good(), &stamp());
    let edits: Vec<(&str, Edit)> = vec![
        ("label", |m| {
            test_mut(m, "build", "test").targets[0].label = "//release:other_test".to_owned();
        }),
        ("flag", |m| {
            test_mut(m, "build", "test").targets[0]
                .flags
                .push("--nocache".to_owned());
        }),
        ("environment", |m| {
            test_mut(m, "build", "test").targets[0]
                .env
                .insert("K".to_owned(), "V".to_owned());
        }),
        ("another target", |m| {
            test_mut(m, "build", "test")
                .targets
                .push(target("//release:more"));
        }),
    ];
    for (what, edit) in &edits {
        let mut m = good();
        edit(&mut m);
        assert_eq!(inputs(m, &stamp()), base, "{what}");
    }
}

#[test]
fn a_build_system_target_or_artifact_edit_changes_it() {
    let base = inputs(with_lib(), &stamp());
    let edits: Vec<(&str, Edit)> = vec![
        ("kind", |m| {
            system_mut(m).kind = BuildSystemKind::Bazel as i32;
        }),
        ("binary", |m| {
            system_mut(m).binary = "/usr/bin/buck2".to_owned();
        }),
        ("config file", |m| system_mut(m).config_file.clear()),
        ("mode", |m| system_mut(m).mode.clear()),
        ("target label", |m| {
            artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("t")
                .label = "//release:other".to_owned();
        }),
        ("a needed artifact's target", |m| {
            artifact_mut(m, "ucdk-lib")
                .target
                .as_mut()
                .expect("t")
                .label = "//release:other".to_owned();
        }),
        ("artifact type", |m| {
            artifact_mut(m, "ucdk-lib").r#type = ArtifactType::Oci as i32;
            // The channel must take an image for the machine to be read.
            m.channels[0].repositories.push(repo(
                ArtifactType::Oci,
                "https://packages.example.com/images",
                Visibility::Public,
                federated_push(federated(
                    "repo:example-org/ucdk:environment:publish",
                    "publish",
                )),
            ));
        }),
        ("artifact name", |m| {
            artifact_mut(m, "ucdk-lib").name = "ucdk-lib2".to_owned();
            artifact_mut(m, "ucdk-cli").needs = strings(&["ucdk-lib2"]);
            relist(m, &["ucdk-cli", "ucdk-lib2"], &["ucdk-cli", "ucdk-lib2"]);
        }),
        ("needs", |m| artifact_mut(m, "ucdk-cli").needs.clear()),
        ("labels", |m| {
            artifact_mut(m, "ucdk-cli")
                .labels
                .insert("requires".to_owned(), "ucdk-lib".to_owned());
        }),
        ("label value", |m| {
            artifact_mut(m, "ucdk-cli")
                .labels
                .insert("requires".to_owned(), "other".to_owned());
        }),
        ("an artifact the action no longer builds", |m| {
            artifact_mut(m, "ucdk-cli").needs.clear();
            relist(m, &["ucdk-cli"], &["ucdk-cli"]);
        }),
    ];
    let mut seen = vec![base.to_string()];
    for (what, edit) in &edits {
        let mut m = with_lib();
        // The labels edits need the label to be a FILE label; the example's artifacts are files.
        edit(&mut m);
        let changed = inputs(m, &stamp());
        assert_ne!(changed, base, "{what}");
        seen.push(changed.to_string());
    }
    let count = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), count, "two edits shared a hash");
}

#[test]
fn a_flag_or_environment_edit_changes_it() {
    let base = inputs(good(), &stamp());
    let edits: Vec<(&str, Edit)> = vec![
        ("base flag", |m| {
            system_mut(m).flags.push("--keep-going".to_owned());
        }),
        ("base flag order", |m| {
            system_mut(m).flags = strings(&["-b", "-a"]);
        }),
        ("base environment value", |m| {
            system_mut(m)
                .env
                .insert("PATH".to_owned(), "/bin".to_owned());
        }),
        ("base environment key", |m| {
            system_mut(m).env.insert("HOME".to_owned(), "/h".to_owned());
        }),
        ("target flag", |m| {
            artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("t")
                .flags
                .pop();
        }),
        ("target flag order", |m| {
            artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("t")
                .flags
                .swap(0, 1);
        }),
        ("target environment", |m| {
            artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("t")
                .env
                .insert("K".to_owned(), "V".to_owned());
        }),
    ];
    let mut seen = vec![base.to_string()];
    for (what, edit) in &edits {
        let mut m = good();
        edit(&mut m);
        let changed = inputs(m, &stamp());
        assert_ne!(changed, base, "{what}");
        seen.push(changed.to_string());
    }
    let count = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), count, "two edits shared a hash");
}

#[test]
fn placeholders_are_hashed_as_written() {
    // A flag that uses {timestamp_ms} is hashed as that text, so the stamp's timestamp, which
    // adoption keeps, never decides whether a stored set may be adopted.
    let with = |flag: &str| {
        let mut m = good();
        artifact_mut(&mut m, "ucdk-cli")
            .target
            .as_mut()
            .expect("t")
            .flags[1] = flag.to_owned();
        m
    };
    let base = inputs(with("t={timestamp_ms}"), &stamp());
    let mut later = stamp();
    later.timestamp_ms += 1;
    assert_eq!(inputs(with("t={timestamp_ms}"), &later), base);
    // The text of the placeholder is what is hashed: another placeholder is another hash, and so
    // is the value the placeholder would be replaced by.
    assert_ne!(inputs(with("t={build_number}"), &stamp()), base);
    assert_ne!(inputs(with("t=1700000000000"), &stamp()), base);
    // An environment value is hashed as written too.
    let mut env = good();
    system_mut(&mut env)
        .env
        .insert("RELEASE".to_owned(), "{release_version}".to_owned());
    let mut substituted = good();
    system_mut(&mut substituted)
        .env
        .insert("RELEASE".to_owned(), "1.2.3".to_owned());
    assert_ne!(inputs(env, &stamp()), inputs(substituted, &stamp()));
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
    let edits: [fn(&mut BuildStamp); 3] = [
        |s| s.version = "9".to_owned(),
        |s| s.build_number += 1,
        |s| s.source_commit = "def".to_owned(),
    ];
    for edit in edits {
        let mut s = stamp();
        edit(&mut s);
        assert_ne!(inputs(good(), &s), base);
    }
    let mut m = good();
    // The publish stage follows the build stage to the other platform.
    build_mut(&mut m, "build", "build").platform = "any".to_owned();
    publish_mut(&mut m, "publish", "publish").platform = "any".to_owned();
    smoke_mut(&mut m, "publish", "smoke").platform = "any".to_owned();
    assert_ne!(inputs(m, &stamp()), base);
    let checked = Checked::read(good()).expect("read");
    let (_, build) = builds(checked.stage("build").expect("stage"))
        .next()
        .expect("build");
    assert_ne!(build_inputs_hash(&checked, build, "other", &stamp()), base);
}

#[test]
fn an_artifact_the_action_does_not_build_does_not_change_it() {
    let base = inputs(with_lib(), &stamp());
    let mut m = with_lib();
    artifact_mut(&mut m, "unused")
        .target
        .as_mut()
        .expect("target")
        .label = "//release:other".to_owned();
    assert_eq!(inputs(m, &stamp()), base);
}

#[test]
fn the_order_the_artifacts_are_declared_in_does_not_change_it() {
    let base = inputs(with_lib(), &stamp());
    let mut m = with_lib();
    m.artifacts.reverse();
    assert_eq!(inputs(m, &stamp()), base);
    // Nor does the order the action lists them in.
    let mut listed = with_lib();
    build_mut(&mut listed, "build", "build").artifacts = strings(&["ucdk-lib", "ucdk-cli"]);
    assert_eq!(inputs(listed, &stamp()), base);
}

#[test]
fn an_action_that_names_an_undeclared_artifact_hashes_the_declared_ones() {
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
    assert_eq!(
        build_inputs_hash(&checked, &build, REVISION, &stamp()),
        base
    );
}
