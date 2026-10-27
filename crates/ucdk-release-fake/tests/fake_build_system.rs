// SPDX-License-Identifier: Apache-2.0
//! The fake build system: bytes that follow the layered command, and the faults of a build.

use std::collections::BTreeMap;
use std::time::Duration;

use ucdk_core::{Clock, ManualClock};
use ucdk_release::store::{SetScope, StoreKey};
use ucdk_release_fake::{FakeBuildSystem, FakeHeartbeat, FakeStore};
use ucdk_release_proto::v1::{BuildStamp, BuildSystem, BuildSystemKind, Reason, Target};
use ucdk_release_seams::{
    ArtifactStore, BuildSystem as _, Invocation, Located, SeamError, Shape, Verb,
};

const LABEL: &str = "//release:tool";

fn system() -> BuildSystem {
    BuildSystem {
        kind: BuildSystemKind::Buck2 as i32,
        binary: "buck2".to_owned(),
        config_file: String::new(),
        mode: String::new(),
        flags: vec![
            "-c".to_owned(),
            "release.version={release_version}".to_owned(),
        ],
        env: BTreeMap::from([("HOME".to_owned(), "/nowhere".to_owned())]),
    }
}

fn target(label: &str, flags: &[&str]) -> Target {
    Target {
        label: label.to_owned(),
        flags: flags.iter().map(|f| (*f).to_owned()).collect(),
        env: BTreeMap::new(),
    }
}

fn stamp(version: &str) -> BuildStamp {
    BuildStamp {
        version: version.to_owned(),
        build_number: 41,
        source_commit: "abc123".to_owned(),
        timestamp_ms: 1,
    }
}

fn invocation<'a>(
    system: &'a BuildSystem,
    target: &'a Target,
    stamp: &'a BuildStamp,
    verb: Verb,
) -> Invocation<'a> {
    Invocation {
        system,
        target,
        verb,
        report: (verb == Verb::Test).then_some("/scratch/t1/events.json-lines"),
        stamp,
        revision: "0123",
        platform: "linux-x86_64",
    }
}

fn fake() -> (ManualClock, FakeBuildSystem, FakeHeartbeat) {
    let clock = ManualClock::at(1_000);
    (
        clock.clone(),
        FakeBuildSystem::new(BuildSystemKind::Buck2, &clock),
        FakeHeartbeat::new(&clock),
    )
}

/// What a build of `label` with `flags` and `version` writes into a store entry.
fn built_bytes(flags: &[&str], version: &str) -> Vec<u8> {
    let (_, mut fake, hb) = fake();
    let (system, target, stamp) = (system(), target(LABEL, flags), stamp(version));
    let inv = invocation(&system, &target, &stamp, Verb::Build);
    let located = fake.build(&inv, &hb).unwrap();
    copied(&mut fake, &located[0]).remove(0).1
}

/// The files `copy_out` writes for `out`.
fn copied(fake: &mut FakeBuildSystem, out: &Located) -> Vec<(String, Vec<u8>)> {
    let scope = SetScope {
        stage: "build".to_owned(),
        platform: "p".to_owned(),
        revision: "r".to_owned(),
        generation: "g".to_owned(),
    };
    let key = StoreKey::member(&scope, "tool").unwrap();
    let mut store = FakeStore::new();
    fake.copy_out(out, store.begin(&key).unwrap().as_mut())
        .unwrap();
    store
        .files(&key)
        .unwrap()
        .into_iter()
        .map(|f| {
            let bytes = store.get(&key, &f).unwrap().unwrap();
            (f, bytes)
        })
        .collect()
}

#[test]
fn a_changed_flag_changes_the_bytes() {
    let base = built_bytes(&[], "1.2.3");
    assert_eq!(base, built_bytes(&[], "1.2.3"));
    assert_ne!(base, built_bytes(&["--extra"], "1.2.3"));
    // The stamp reaches the bytes through the placeholder of a flag.
    assert_ne!(base, built_bytes(&[], "1.2.4"));
}

#[test]
fn records_every_command_it_ran() {
    let (_, mut fake, hb) = fake();
    let (system, target, stamp) = (system(), target(LABEL, &["--x"]), stamp("1.2.3"));
    fake.build(&invocation(&system, &target, &stamp, Verb::Build), &hb)
        .unwrap();
    fake.test(&invocation(&system, &target, &stamp, Verb::Test), &hb)
        .unwrap();
    assert_eq!(fake.kind(), BuildSystemKind::Buck2);
    let commands = fake.commands();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].argv[..2], ["buck2", "build"]);
    assert!(
        commands[0]
            .argv
            .contains(&"release.version=1.2.3".to_owned())
    );
    assert_eq!(commands[1].argv[1], "test");
    assert_eq!(
        commands[0].env,
        [("HOME".to_owned(), "/nowhere".to_owned())]
    );
    // The command is the pure layering, which a test can ask for without a run.
    let asked = fake
        .command(&invocation(&system, &target, &stamp, Verb::Build))
        .unwrap();
    assert_eq!(asked, commands[0]);
    assert_eq!(fake.commands().len(), 2);
}

#[test]
fn a_command_that_cannot_be_layered_is_permanent() {
    let (_, mut fake, hb) = fake();
    let (system, target, stamp) = (system(), target(LABEL, &[]), stamp("1"));
    let inv = invocation(&system, &target, &stamp, Verb::Query);
    assert!(matches!(
        fake.build(&inv, &hb),
        Err(SeamError::Permanent(_))
    ));
    assert!(fake.commands().is_empty());
}

#[test]
fn beats_advance_the_manual_clock_before_each_beat() {
    let (clock, mut fake, hb) = fake();
    fake.beats(3, Duration::from_secs(5));
    let (system, target, stamp) = (system(), target(LABEL, &[]), stamp("1"));
    fake.build(&invocation(&system, &target, &stamp, Verb::Build), &hb)
        .unwrap();
    assert_eq!(hb.readings(), [6_000, 11_000, 16_000]);
    assert_eq!(clock.now_millis(), 16_000);
    // A stop at a beat ends the invocation with the lease lost.
    let stopping = FakeHeartbeat::new(&clock).stop_at(1);
    assert_eq!(
        fake.test(&invocation(&system, &target, &stamp, Verb::Test), &stopping),
        Err(SeamError::Refused(Reason::LeaseLost))
    );
}

#[test]
fn silent_for_advances_the_clock_and_never_beats() {
    let (clock, mut fake, hb) = fake();
    fake.silent_for(Duration::from_secs(2_000));
    let (system, target, stamp) = (system(), target(LABEL, &[]), stamp("1"));
    fake.build(&invocation(&system, &target, &stamp, Verb::Build), &hb)
        .unwrap();
    assert_eq!(hb.beats(), 0);
    assert_eq!(clock.now_millis(), 1_000 + 2_000_000);
}

#[test]
fn an_unreadable_report_is_zero_cases() {
    let (system, target, stamp) = (system(), target(LABEL, &[]), stamp("1"));
    for fault in [
        FakeBuildSystem::report_missing,
        FakeBuildSystem::report_unparseable,
    ] {
        let (_, mut fake, hb) = fake();
        fault(&mut fake, LABEL);
        let outcome = fake
            .test(&invocation(&system, &target, &stamp, Verb::Test), &hb)
            .unwrap();
        assert_eq!((outcome.ran, outcome.passed), (0, false));
        assert!(outcome.tail.contains(LABEL));
    }
}

#[test]
fn a_test_passes_only_when_it_ran_something_and_did_not_fail() {
    let (system, stamp) = (system(), stamp("1"));
    let (_, mut fake, hb) = fake();
    fake.test_runs_nothing("//a:nothing");
    fake.test_fails("//a:fails");
    let mut outcome = |label: &str| {
        let target = target(label, &[]);
        fake.test(&invocation(&system, &target, &stamp, Verb::Test), &hb)
            .unwrap()
    };
    let (ok, nothing, fails) = (
        outcome("//a:ok"),
        outcome("//a:nothing"),
        outcome("//a:fails"),
    );
    assert_eq!(
        (ok.passed, ok.ran, ok.tail.as_str()),
        (true, 1, "//a:ok passed")
    );
    assert_eq!((nothing.passed, nothing.ran), (false, 0));
    assert_eq!((fails.passed, fails.ran), (false, 1));
}

#[test]
fn a_failed_invocation_carries_the_tail_it_printed() {
    let (system, target, stamp) = (system(), target(LABEL, &[]), stamp("1"));
    let (_, mut fake, hb) = fake();
    fake.fail_at_call(1);
    fake.failure_tail("error: boom");
    let failure = fake.build(&invocation(&system, &target, &stamp, Verb::Build), &hb);
    assert!(
        matches!(failure, Err(SeamError::Permanent(why)) if why.contains(LABEL) && why.ends_with("error: boom"))
    );
}

#[test]
fn a_build_that_fails_dies_or_cannot_start() {
    let (system, target, stamp) = (system(), target(LABEL, &[]), stamp("1"));
    let (_, mut fake, hb) = fake();
    fake.fail_at_call(2);
    fake.die_at_call(3);
    let build = |fake: &mut FakeBuildSystem| {
        fake.build(&invocation(&system, &target, &stamp, Verb::Build), &hb)
    };
    assert!(build(&mut fake).is_ok());
    assert!(matches!(build(&mut fake), Err(SeamError::Permanent(why)) if why.contains(LABEL)));
    assert!(matches!(build(&mut fake), Err(SeamError::Unknown(_))));
    fake.unstartable();
    assert!(matches!(build(&mut fake), Err(SeamError::Unknown(_))));
    assert_eq!(fake.commands().len(), 4);
}

/// Renamed from `the_listing_shows_none_two_or_a_wrong_shape_of_output`: the seam refuses a
/// listing of none or of two outputs, so the test now expects the refusal. A shape the artifact
/// does not want is not the seam's to refuse (it does not know the type), so a shape is still
/// reported.
#[test]
fn the_build_system_refuses_none_or_two_outputs_and_reports_the_shape_of_one() {
    let (system, stamp) = (system(), stamp("1"));
    let (_, mut fake, hb) = fake();
    fake.no_output("//a:none");
    fake.two_outputs("//a:two");
    fake.no_output("//a:both");
    fake.two_outputs("//a:both");
    fake.wrong_shape("//a:flipped");
    fake.directory_output("//a:image");
    fake.directory_output("//a:twice");
    fake.wrong_shape("//a:twice");
    let mut listing = |label: &str| {
        let target = target(label, &[]);
        fake.build(&invocation(&system, &target, &stamp, Verb::Build), &hb)
    };
    for label in ["//a:none", "//a:two", "//a:both"] {
        assert_eq!(
            listing(label),
            Err(SeamError::Refused(Reason::BuildOutput)),
            "{label}"
        );
    }
    let one = |listed: Vec<Located>| {
        assert_eq!(listed.len(), 1);
        listed.into_iter().next().unwrap()
    };
    assert_eq!(one(listing("//a:flipped").unwrap()).shape, Shape::Directory);
    assert_eq!(one(listing("//a:image").unwrap()).shape, Shape::Directory);
    assert_eq!(one(listing("//a:plain").unwrap()).shape, Shape::File);
    // A directory wrong-shaped twice is the shape it should be.
    assert_eq!(one(listing("//a:twice").unwrap()).shape, Shape::File);
    let plain = one(listing("//a:plain").unwrap());
    assert_eq!(plain.label, "//a:plain");
    assert_eq!(plain.path, "/out/_s_sa_cplain/plain");
}

#[test]
fn an_environment_value_changes_the_bytes() {
    let built = |value: &str| {
        let (_, mut fake, hb) = fake();
        let (system, stamp) = (system(), stamp("1.2.3"));
        let mut target = target(LABEL, &[]);
        target.env = BTreeMap::from([("MODE".to_owned(), value.to_owned())]);
        let inv = invocation(&system, &target, &stamp, Verb::Build);
        let located = fake.build(&inv, &hb).unwrap();
        copied(&mut fake, &located[0]).remove(0).1
    };
    // Only one environment value differs between these builds.
    assert_eq!(built("release"), built("release"));
    assert_ne!(built("release"), built("debug"));
}

#[test]
fn a_symlink_in_an_output_is_build_output() {
    let (system, target, stamp) = (system(), target(LABEL, &[]), stamp("1"));
    let (_, mut fake, hb) = fake();
    fake.symlink_in_output(LABEL);
    assert_eq!(
        fake.build(&invocation(&system, &target, &stamp, Verb::Build), &hb),
        Err(SeamError::Refused(Reason::BuildOutput))
    );
}

#[test]
fn copy_out_writes_the_file_or_the_directory_and_only_what_was_built() {
    let (_, mut fake, hb) = fake();
    fake.directory_output("//a:image");
    fake.output_changes_after_listing("//a:changing");
    let file = build_one(&mut fake, &hb, "//a:file");
    let image = build_one(&mut fake, &hb, "//a:image");
    let changing = build_one(&mut fake, &hb, "//a:changing");
    let files = copied(&mut fake, &file);
    assert_eq!((files.len(), files[0].0.as_str()), (1, "file"));
    assert_eq!(copied(&mut fake, &image)[0].0, "image/layout");
    let changed = copied(&mut fake, &changing).remove(0).1;
    assert!(changed.ends_with(b"changed"));
    let unknown = Located {
        label: "x".to_owned(),
        path: "/out/never".to_owned(),
        shape: Shape::File,
    };
    let mut store = FakeStore::new();
    let scope = SetScope {
        stage: "s".to_owned(),
        platform: "p".to_owned(),
        revision: "r".to_owned(),
        generation: "g".to_owned(),
    };
    let key = StoreKey::member(&scope, "x").unwrap();
    assert!(matches!(
        fake.copy_out(&unknown, store.begin(&key).unwrap().as_mut()),
        Err(SeamError::Permanent(_))
    ));
}

/// The first output of a build of `label`.
fn build_one(fake: &mut FakeBuildSystem, hb: &FakeHeartbeat, label: &str) -> Located {
    let (system, target, stamp) = (system(), target(label, &[]), stamp("1"));
    fake.build(&invocation(&system, &target, &stamp, Verb::Build), hb)
        .unwrap()
        .remove(0)
}

#[test]
fn a_later_build_changes_an_earlier_members_tree() {
    let (_, mut fake, hb) = fake();
    fake.mutate_earlier_output();
    let first = build_one(&mut fake, &hb, "//a:first");
    let before = copied(&mut fake, &first).remove(0).1;
    build_one(&mut fake, &hb, "//a:second");
    let after = copied(&mut fake, &first).remove(0).1;
    assert_ne!(before, after);
    assert!(after.ends_with(b"mutated"));
}

#[test]
fn a_later_build_changes_the_earliest_built_output_not_the_smallest_path() {
    let (_, mut fake, hb) = fake();
    fake.mutate_earlier_output();
    // Built in the order z, a, b: the smallest path is a, the earliest built is z.
    let z = build_one(&mut fake, &hb, "//a:z");
    let a = build_one(&mut fake, &hb, "//a:a");
    let a_before = copied(&mut fake, &a).remove(0).1;
    let b = build_one(&mut fake, &hb, "//a:b");
    // The build of a changed z once, and the build of b changed z again; neither touched a.
    assert!(
        copied(&mut fake, &z)
            .remove(0)
            .1
            .ends_with(b"mutatedmutated")
    );
    assert_eq!(copied(&mut fake, &a).remove(0).1, a_before);
    assert!(!copied(&mut fake, &b).remove(0).1.ends_with(b"mutated"));
}

#[test]
fn labels_that_differ_only_in_the_separators_get_their_own_outputs() {
    let (_, mut fake, hb) = fake();
    let labels = ["//p:x", "//p/x", "//p_x"];
    let outs: Vec<Located> = labels
        .iter()
        .map(|label| build_one(&mut fake, &hb, label))
        .collect();
    for (i, a) in outs.iter().enumerate() {
        for b in &outs[i + 1..] {
            assert_ne!(a.path, b.path);
        }
    }
    // Every output still holds the bytes of its own build.
    for (label, out) in labels.iter().zip(&outs) {
        let bytes = copied(&mut fake, out).remove(0).1;
        assert!(String::from_utf8(bytes).unwrap().contains(label));
    }
}

#[test]
fn arguments_that_differ_only_in_where_the_line_breaks_fall_change_the_bytes() {
    assert_ne!(built_bytes(&["a\nb"], "1"), built_bytes(&["a", "b"], "1"));
}

#[test]
fn environment_pairs_that_differ_only_in_where_the_equals_sign_falls_change_the_bytes() {
    let built = |key: &str, value: &str| {
        let (_, mut fake, hb) = fake();
        let (system, stamp) = (system(), stamp("1.2.3"));
        let mut target = target(LABEL, &[]);
        target.env = BTreeMap::from([(key.to_owned(), value.to_owned())]);
        let inv = invocation(&system, &target, &stamp, Verb::Build);
        let located = fake.build(&inv, &hb).unwrap();
        copied(&mut fake, &located[0]).remove(0).1
    };
    assert_ne!(built("A", "b=c"), built("A=b", "c"));
}
