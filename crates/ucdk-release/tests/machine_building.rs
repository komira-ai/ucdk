// SPDX-License-Identifier: Apache-2.0
//! Reading a machine, the rules of the build system, the targets that name what it builds and
//! tests, the flags and environment a machine authors, and the Build and Test actions that run
//! it. The text rules themselves are tested over `ucdk_release::flags` in `flags.rs`; here each
//! is reached through a whole machine, in each of the three places a flag or an environment is
//! authored: the build system, an artifact's target and a test's target.

mod common;

use std::collections::BTreeMap;

use common::*;
use ucdk_release::machine::Rule;
use ucdk_release_proto::v1::{
    ArtifactType, BuildSystemKind, Machine, Trigger, ValidationKind, Visibility,
};

/// Where a flag or an environment is authored.
#[derive(Clone, Copy, Debug)]
enum Layer {
    Base,
    Artifact,
    Test,
}

const LAYERS: [Layer; 3] = [Layer::Base, Layer::Artifact, Layer::Test];

fn flags_of(m: &mut Machine, layer: Layer) -> &mut Vec<String> {
    match layer {
        Layer::Base => &mut system_mut(m).flags,
        Layer::Artifact => {
            &mut artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("target")
                .flags
        }
        Layer::Test => &mut test_mut(m, "build", "test").targets[0].flags,
    }
}

fn env_of(m: &mut Machine, layer: Layer) -> &mut BTreeMap<String, String> {
    match layer {
        Layer::Base => &mut system_mut(m).env,
        Layer::Artifact => {
            &mut artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("target")
                .env
        }
        Layer::Test => &mut test_mut(m, "build", "test").targets[0].env,
    }
}

fn kind(m: &mut Machine, kind: BuildSystemKind) {
    system_mut(m).kind = kind as i32;
}

#[test]
fn build_system_required_is_refused() {
    refuses_only(Rule::BuildSystemRequired, |m| m.build_system = None);
    // A Test action alone asks for it as well.
    refuses_only(Rule::BuildSystemRequired, |m| {
        m.build_system = None;
        m.stages.truncate(0);
        m.stages.push(stage(
            "verify",
            Trigger::Release,
            &[],
            vec![test_action("test", &["//release:set_test"])],
        ));
        m.artifacts.clear();
    });
    // A machine with no stage that builds or tests needs none.
    accepts(|m| {
        m.build_system = None;
        m.stages.clear();
        m.artifacts.clear();
        m.channels.clear();
    });
}

#[test]
fn build_system_shape_is_refused() {
    refuses_only(Rule::BuildSystemShape, |m| system_mut(m).kind = 0);
    refuses_only(Rule::BuildSystemShape, |m| {
        system_mut(m).binary = String::new();
    });
    for binary in ["bin/buck2", "./buck2", "tools/"] {
        refuses_only(Rule::BuildSystemShape, |m| {
            system_mut(m).binary = binary.to_owned();
        });
    }
    for config in ["/etc/buckconfig", "..", "a/../b", "../x"] {
        refuses_only(Rule::BuildSystemShape, |m| {
            system_mut(m).config_file = config.to_owned();
        });
    }
    for mode in ["rel ease", "mode;rm", "m\u{e9}", "a\0b"] {
        refuses_only(Rule::BuildSystemShape, |m| {
            system_mut(m).mode = mode.to_owned();
        });
    }
    for flag in ["", "-c\0x"] {
        refuses_only(Rule::BuildSystemShape, |m| {
            system_mut(m).flags.push(flag.to_owned());
        });
    }
    // The characters a mode may hold, an absolute binary, and dots inside a name.
    accepts(|m| system_mut(m).mode = "@mode/dev=1.x-y_z".to_owned());
    accepts(|m| system_mut(m).binary = "/opt/tools/bin/buck2".to_owned());
    accepts(|m| system_mut(m).config_file = "release/a..b.buckconfig".to_owned());
}

#[test]
fn a_bare_binary_without_a_path_is_refused() {
    refuses_only(Rule::BuildSystemShape, |m| {
        system_mut(m).env.remove("PATH");
    });
    // A lower-case key is another key.
    refuses_only(Rule::BuildSystemShape, |m| {
        let value = system_mut(m).env.remove("PATH").expect("path");
        system_mut(m).env.insert("path".to_owned(), value);
    });
    // An absolute binary needs no PATH.
    accepts(|m| {
        system_mut(m).env.remove("PATH");
        system_mut(m).binary = "/opt/tools/bin/buck2".to_owned();
    });
}

#[test]
fn an_empty_mode_or_config_file_means_none() {
    accepts(|m| {
        system_mut(m).mode = String::new();
        system_mut(m).config_file = String::new();
    });
}

#[test]
fn a_reserved_flag_is_refused() {
    let buck2_refused = [
        "--event-log=events",
        "--event-log",
        "--event_log=events",
        "--no-event-log",
        "--noevent-log",
        "--event",
        "--show-output",
        "--show_full_output",
        "--show-json-output",
        "--no_show_output",
        "--",
        "@argfile",
    ];
    let buck2_accepted = [
        "--output=files",
        "--nooutput",
        "--build_event_json_file=x",
        "--output_filter=x",
        "--event-log-extra",
    ];
    let bazel_refused = [
        "--output=files",
        "--output",
        "--nooutput",
        "--no_output",
        "--build_event_json_file=x",
        "--build-event-text-file",
        "--build_event_binary_file=x",
        "--symlink_prefix=/x",
        "--",
    ];
    let bazel_accepted = [
        "--event-log=events",
        "--event",
        "--show-output",
        "--build",
        "--nobuild",
        "--o",
        "--build_event",
        "--output_filter=x",
        "--build_event_json_file_path_conversion",
        "@argfile",
        "files",
        "-c",
    ];
    for layer in LAYERS {
        for flag in buck2_refused {
            refuses_only(Rule::ReservedFlag, |m| {
                flags_of(m, layer).push(flag.to_owned());
            });
        }
        for flag in buck2_accepted {
            accepts(|m| flags_of(m, layer).push(flag.to_owned()));
        }
        for flag in bazel_refused {
            refuses_only(Rule::ReservedFlag, |m| {
                kind(m, BuildSystemKind::Bazel);
                flags_of(m, layer).push(flag.to_owned());
            });
        }
        for flag in bazel_accepted {
            accepts(|m| {
                kind(m, BuildSystemKind::Bazel);
                flags_of(m, layer).push(flag.to_owned());
            });
        }
    }
}

#[test]
fn an_env_key_outside_the_grammar_is_refused() {
    for layer in LAYERS {
        for key in ["", "1X", "A-B", "A B", "A=B", "\u{e9}", "A.B"] {
            refuses_only(Rule::EnvKey, |m| {
                env_of(m, layer).insert(key.to_owned(), "v".to_owned());
            });
        }
    }
}

#[test]
fn a_lowercase_env_key_is_accepted_and_a_ucdk_prefix_in_any_case_is_refused() {
    for layer in LAYERS {
        for key in [
            "http_proxy",
            "no_proxy",
            "_x",
            "A1_b",
            "UCDK",
            "ucdk",
            "UCD_K",
        ] {
            accepts(|m| {
                env_of(m, layer).insert(key.to_owned(), "v".to_owned());
            });
        }
        for key in ["UCDK_X", "ucdk_x", "Ucdk_Y", "uCdK_"] {
            refuses_only(Rule::EnvKey, |m| {
                env_of(m, layer).insert(key.to_owned(), "v".to_owned());
            });
        }
    }
}

#[test]
fn an_unknown_placeholder_is_refused() {
    for layer in LAYERS {
        for text in ["{nope}", "{", "}", "{release_version", "{}", "a}b", "{{x}"] {
            refuses_only(Rule::Placeholder, |m| {
                flags_of(m, layer).push(format!("--p={text}"));
            });
            refuses_only(Rule::Placeholder, |m| {
                env_of(m, layer).insert("P".to_owned(), text.to_owned());
            });
        }
        // The six names, doubled braces, and text with no brace.
        accepts(|m| {
            flags_of(m, layer).push(
                "{release_version}{build_number}{source_commit}{revision}{timestamp_ms}{platform}"
                    .to_owned(),
            );
            flags_of(m, layer).push("{{literal}}".to_owned());
            env_of(m, layer).insert("P".to_owned(), "{revision}-{{x}}".to_owned());
        });
    }
    // The binary, config file, mode and labels are not substituted, so a brace in them is
    // not a placeholder.
    accepts(|m| system_mut(m).config_file = "release/{x}.buckconfig".to_owned());
}

#[test]
fn platform_in_a_test_of_a_stage_without_one_is_refused() {
    let verify = || {
        stage(
            "verify",
            Trigger::Release,
            &[],
            vec![test_action("test", &["//release:set_test"])],
        )
    };
    // A test target that uses it, in a stage that names no platform.
    refuses_only(Rule::Placeholder, |m| {
        let mut stage = verify();
        if let Some(ucdk_release_proto::v1::action::Kind::Test(t)) = &mut stage.actions[0].kind {
            t.targets[0].flags.push("--p={platform}".to_owned());
        }
        m.stages.push(stage);
    });
    // The build system's own flags and environment reach every test too.
    refuses_only(Rule::Placeholder, |m| {
        m.stages.push(verify());
        system_mut(m).flags.push("--p={platform}".to_owned());
    });
    refuses_only(Rule::Placeholder, |m| {
        m.stages.push(verify());
        system_mut(m)
            .env
            .insert("P".to_owned(), "{platform}".to_owned());
    });
    // Without such a stage, or in a stage that names a platform, it is a placeholder like the
    // others.
    accepts(|m| system_mut(m).flags.push("--p={platform}".to_owned()));
    accepts(|m| {
        m.stages.push(verify());
        flags_of(m, Layer::Test).push("--p={platform}".to_owned());
    });
}

#[test]
fn a_target_that_is_a_pattern_is_refused() {
    let buck2_refused = [
        "//release:all",
        "//release/...",
        "//...",
        "//release:*",
        "//release",
        "release:x",
        "//a b:c",
        "//a:b:c",
        "//a:",
        "//a//b:c",
        "//a/../b:c",
        "//a:b\n",
        "@repo//a:b",
        "--show-output//pkg:t",
        "-c//a:b",
        "-",
        "",
    ];
    for label in buck2_refused {
        refuses_only(Rule::TargetShape, |m| {
            artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("target")
                .label = label.to_owned();
        });
        refuses_only(Rule::TargetShape, |m| {
            test_mut(m, "build", "test").targets[0].label = label.to_owned();
        });
    }
    for label in [
        "//:x",
        "root//pkg/sub:name-1",
        "//a:b.c",
        "my-cell//pkg:t",
        "//release:ucdk_cli_package",
    ] {
        accepts(|m| {
            artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("target")
                .label = label.to_owned();
        });
    }
    for label in [
        "@repo//pkg:t",
        "@//pkg:t",
        "@@canon~1.0//pkg:t",
        "//pkg:t",
        "//:t",
    ] {
        accepts(|m| {
            kind(m, BuildSystemKind::Bazel);
            artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("target")
                .label = label.to_owned();
        });
    }
    for label in [
        "repo//pkg:t",
        "-x//pkg:t",
        "@re po//pkg:t",
        "//pkg:all",
        "@repo//pkg/...",
        "//pkg",
    ] {
        refuses_only(Rule::TargetShape, |m| {
            kind(m, BuildSystemKind::Bazel);
            artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("target")
                .label = label.to_owned();
        });
    }
}

#[test]
fn an_empty_flag_or_a_nul_in_a_flag_or_value_is_refused_in_every_layer() {
    for layer in LAYERS {
        let rule = match layer {
            Layer::Base => Rule::BuildSystemShape,
            Layer::Artifact | Layer::Test => Rule::TargetShape,
        };
        for flag in ["", "--a\0b", "\0"] {
            refuses_only(rule, |m| flags_of(m, layer).push(flag.to_owned()));
        }
        refuses_only(rule, |m| {
            env_of(m, layer).insert("K".to_owned(), "a\0b".to_owned());
        });
        // An empty environment value is a value, and a flag with text is a flag.
        accepts(|m| {
            env_of(m, layer).insert("K".to_owned(), String::new());
            flags_of(m, layer).push("-x".to_owned());
        });
    }
}

#[test]
fn an_artifact_without_a_target_is_refused() {
    refuses_only(Rule::ArtifactTargetMissing, |m| {
        artifact_mut(m, "ucdk-cli").target = None;
    });
}

#[test]
fn a_build_naming_an_undeclared_artifact_is_refused() {
    refuses_only(Rule::BuildUndeclared, |m| {
        build_mut(m, "check", "build")
            .artifacts
            .push("ghost".to_owned());
    });
}

#[test]
fn a_build_not_closed_under_needs_is_refused() {
    refuses_only(Rule::BuildNotClosed, |m| {
        artifact_mut(m, "ucdk-cli")
            .needs
            .push("ucdk-lib".to_owned());
        build_mut(m, "check", "build").artifacts = vec!["ucdk-cli".to_owned()];
    });
    // Listing both closes the list, and an action that lists only the needed one is closed.
    accepts(|m| {
        artifact_mut(m, "ucdk-cli")
            .needs
            .push("ucdk-lib".to_owned());
    });
    accepts(|m| {
        artifact_mut(m, "ucdk-cli")
            .needs
            .push("ucdk-lib".to_owned());
        build_mut(m, "check", "build").artifacts = vec!["ucdk-lib".to_owned()];
    });
}

#[test]
fn labels_on_a_non_file_artifact_are_refused() {
    let as_image = |m: &mut Machine| {
        artifact_mut(m, "ucdk-lib").r#type = ArtifactType::Oci as i32;
        m.channels[0].repositories.push(repo(
            ArtifactType::Oci,
            "https://packages.example.com/images",
            Visibility::Public,
            anonymous_push(),
        ));
    };
    accepts(as_image);
    refuses_only(Rule::LabelsOnNonFile, |m| {
        as_image(m);
        artifact_mut(m, "ucdk-lib")
            .labels
            .insert("requires".to_owned(), "x".to_owned());
    });
    accepts(|m| {
        artifact_mut(m, "ucdk-lib")
            .labels
            .insert("requires".to_owned(), "x".to_owned());
    });
}

#[test]
fn a_test_action_without_targets_is_refused() {
    refuses_only(Rule::TestShape, |m| {
        test_mut(m, "build", "test").targets.clear();
    });
    let too_many = |count: usize| {
        move |m: &mut Machine| {
            test_mut(m, "build", "test").targets = (0..count)
                .map(|n| target(&format!("//release:t{n}")))
                .collect();
        }
    };
    refuses_only(Rule::TestShape, too_many(33));
    accepts(too_many(32));
    refuses_only(Rule::TestShape, |m| {
        test_mut(m, "build", "test")
            .targets
            .push(target("//release:set_test"));
    });
}

#[test]
fn a_test_action_with_validations_is_refused() {
    // A test holds no validation, and the place is refused as well.
    assert_eq!(
        refused_after(|m| {
            action_mut(m, "build", "test").validations.push(validation(
                "extra",
                ValidationKind::SetClosure,
                0,
            ));
        }),
        [Rule::TestShape, Rule::ValidationPlacement]
    );
}

#[test]
fn a_closure_validation_on_a_stage_is_refused() {
    refuses_only(Rule::ValidationPlacement, |m| {
        stage_mut(m, "build").validations.push(validation(
            "stage-closure",
            ValidationKind::SetClosure,
            0,
        ));
    });
}
