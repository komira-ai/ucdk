// SPDX-License-Identifier: Apache-2.0
//! The pure `command` every build system gets by default: the layering, with a refusal that
//! reads as a permanent error.

use std::collections::BTreeMap;

use ucdk_release_proto::v1::{BuildStamp, BuildSystem as Declared, BuildSystemKind, Target};
use ucdk_release_seams::{
    BuildSystem, EntryWriter, Heartbeat, Invocation, Located, SeamError, TestOutcome, Verb,
};

/// A build system that does nothing but keep the default `command`.
struct Layering;

impl BuildSystem for Layering {
    fn kind(&self) -> BuildSystemKind {
        BuildSystemKind::Buck2
    }

    fn build(&mut self, _: &Invocation<'_>, _: &dyn Heartbeat) -> Result<Vec<Located>, SeamError> {
        Ok(Vec::new())
    }

    fn test(&mut self, _: &Invocation<'_>, _: &dyn Heartbeat) -> Result<TestOutcome, SeamError> {
        Err(SeamError::Unknown("not run".to_owned()))
    }

    fn copy_out(&mut self, _: &Located, _: &mut dyn EntryWriter) -> Result<(), SeamError> {
        Ok(())
    }
}

fn invocation<'a>(
    system: &'a Declared,
    target: &'a Target,
    stamp: &'a BuildStamp,
    verb: Verb,
) -> Invocation<'a> {
    Invocation {
        system,
        target,
        verb,
        report: None,
        stamp,
        revision: "r",
        platform: "",
    }
}

#[test]
fn the_default_command_is_the_layering_and_a_failed_layering_is_permanent() {
    let system = Declared {
        kind: BuildSystemKind::Buck2 as i32,
        binary: "buck2".to_owned(),
        config_file: String::new(),
        mode: String::new(),
        flags: Vec::new(),
        env: BTreeMap::new(),
    };
    let target = Target {
        label: "//a:b".to_owned(),
        flags: Vec::new(),
        env: BTreeMap::new(),
    };
    let stamp = BuildStamp::default();
    let built = Layering
        .command(&invocation(&system, &target, &stamp, Verb::Build))
        .unwrap();
    assert_eq!(built.argv[..2], ["buck2", "build"]);
    let refused = Layering.command(&invocation(&system, &target, &stamp, Verb::Info));
    assert!(matches!(refused, Err(SeamError::Permanent(why)) if why.contains("info")));
}
