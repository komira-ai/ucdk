// SPDX-License-Identifier: Apache-2.0
//! `Bazel` over a process runner scripted with the output of the tool and the fake files: the
//! commands it runs to build and to find the output, the output it checks, the report it reads,
//! and how it ends.

mod support;

use std::time::Duration;

use support::{Case, Entry, Handle, LABEL, ROOT, argv};
use ucdk_core::ManualClock;
use ucdk_release::invocation::Verb;
use ucdk_release_build::Bazel;
use ucdk_release_fake::{FakeOutputFiles, FakeProcessRunner, RunCall};
use ucdk_release_proto::v1::{BuildSystemKind, Reason};
use ucdk_release_seams::kit::Beats;
use ucdk_release_seams::{BuildSystem, Exit, Located, SeamError, Shape, TestOutcome};

const BIN: &str = "/opt/bazel";
const EXEC_ROOT: &str = "/cache/ws/execroot/_main";
const REL: &str = "bazel-out/k8-fastbuild/bin/pkg/out";

/// A Bazel over a scripted tool, and the ways into the fakes it holds.
struct Rig {
    case: Case,
    runner: Handle<FakeProcessRunner>,
    files: Handle<FakeOutputFiles>,
}

impl Rig {
    fn new() -> Self {
        let runner = Handle::new(FakeProcessRunner::new(&ManualClock::at(0)));
        runner.with(|r| r.program(BIN));
        Self {
            case: Case::bazel(),
            runner,
            files: Handle::new(FakeOutputFiles::new()),
        }
    }

    /// The tool answers `verb` with `code`, `stdout` and `stderr`.
    fn script(&self, verb: Verb, code: i32, stdout: &str, stderr: &str) {
        let command = self.case.command(verb);
        self.runner.with(|r| {
            r.script(
                command,
                Exit::Code(code),
                stdout.as_bytes(),
                stderr.as_bytes(),
            );
        });
    }

    /// The three calls of a build: it builds, and the listing and the root are what is given.
    fn script_build(&self, query: &str, info: &str) {
        self.script(Verb::Build, 0, "", "");
        self.script(Verb::Query, 0, query, "");
        self.script(Verb::Info, 0, info, "");
    }

    fn system(&self) -> Bazel<Handle<FakeProcessRunner>, Handle<FakeOutputFiles>> {
        Bazel::new(self.runner.clone(), self.files.clone(), ROOT)
    }

    fn build(&self) -> Result<Vec<Located>, SeamError> {
        self.system()
            .build(&self.case.inv(Verb::Build, None), &Beats::counting())
    }

    fn test(&self) -> Result<TestOutcome, SeamError> {
        let report = self.case.report();
        self.system().test(
            &self.case.inv(Verb::Test, Some(&report)),
            &Beats::counting(),
        )
    }

    fn calls(&self) -> Vec<RunCall> {
        self.runner.with(|r| r.calls().to_vec())
    }

    fn file_calls(&self) -> Vec<String> {
        self.files.with(|f| f.calls().to_vec())
    }
}

/// The path `rel` below the execution root.
fn below(rel: &str) -> String {
    format!("{EXEC_ROOT}/{rel}")
}

#[test]
fn the_command_line_is_the_golden_for_each_verb() {
    let rig = Rig::new();
    let system = rig.system();
    let report = rig.case.report();
    let head = [BIN, "--bazelrc=/repo/.bazelrc"];
    let flags = ["--config=ci", "--define=version=1.2.3", "--keep_going"];
    let of = |verb: &str, owned: &[&str], tail: &[&str]| {
        let mut words: Vec<&str> = head.to_vec();
        words.push(verb);
        words.extend(flags);
        words.extend(owned);
        words.extend(tail);
        argv(&words)
    };
    let cases = [
        (Verb::Build, None, of("build", &[], &["--", LABEL])),
        (
            Verb::Query,
            None,
            of("cquery", &["--output=files"], &["--", LABEL]),
        ),
        (Verb::Info, None, of("info", &[], &["execution_root"])),
        (
            Verb::Test,
            Some(report.as_str()),
            of(
                "test",
                &["--build_event_json_file=/scratch/t1/events.json"],
                &["--", LABEL],
            ),
        ),
    ];
    for (verb, report, want) in cases {
        let command = system.command(&rig.case.inv(verb, report)).unwrap();
        assert_eq!(command.argv, want, "{verb}");
        assert_eq!(command.env, [("LOG".to_owned(), "warn".to_owned())]);
    }
    assert_eq!(system.kind(), BuildSystemKind::Bazel);
}

#[test]
fn outputs_come_from_cquery_and_resolve_against_the_execution_root() {
    for root in [format!("{EXEC_ROOT}\n"), format!("{EXEC_ROOT}/\n")] {
        let rig = Rig::new();
        rig.files.with(|f| f.file(&below(REL), b"bytes"));
        rig.script_build(&format!("{REL}\n"), &root);
        assert_eq!(
            rig.build().unwrap(),
            [Located {
                label: LABEL.to_owned(),
                path: below(REL),
                shape: Shape::File,
            }],
            "{root:?}"
        );
    }
}

#[test]
fn locate_runs_cquery_then_info_with_the_builds_flags() {
    let rig = Rig::new();
    rig.files.with(|f| f.file(&below(REL), b"bytes"));
    rig.script_build(&format!("{REL}\n"), &format!("{EXEC_ROOT}\n"));
    rig.build().unwrap();
    let want: Vec<RunCall> = [Verb::Build, Verb::Query, Verb::Info]
        .into_iter()
        .map(|verb| RunCall {
            command: rig.case.command(verb),
            cwd: ROOT.to_owned(),
        })
        .collect();
    assert_eq!(rig.calls(), want);
    // The two locating calls take the author's flags as the build does, so a tool that rejects
    // a flag rejects it there too.
    for call in &rig.calls()[1..] {
        for flag in ["--config=ci", "--define=version=1.2.3", "--keep_going"] {
            assert!(call.command.argv.contains(&flag.to_owned()), "{flag}");
        }
    }
    // Locating asks what is at the path, and reads nothing.
    assert_eq!(rig.file_calls(), [format!("stat {}", below(REL))]);
}

#[test]
fn a_located_path_that_does_not_exist_is_build_output() {
    let rig = Rig::new();
    rig.script_build(&format!("{REL}\n"), &format!("{EXEC_ROOT}\n"));
    assert_eq!(rig.build(), Err(SeamError::Refused(Reason::BuildOutput)));
}

#[test]
fn located_shape_is_what_stat_reports() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.file(&below("a/file"), b"x");
        f.file(&below("a/dir/one"), b"1");
        f.file(&below("a/dir/sub/two"), b"2");
    });
    let shape = |rel: &str| {
        let rig = Rig {
            case: Case::bazel(),
            runner: Handle::new(FakeProcessRunner::new(&ManualClock::at(0))),
            files: rig.files.clone(),
        };
        rig.runner.with(|r| r.program(BIN));
        rig.script_build(&format!("{rel}\n"), &format!("{EXEC_ROOT}\n"));
        let built = rig.build().unwrap();
        (built[0].shape, rig.file_calls())
    };
    assert_eq!(shape("a/file").0, Shape::File);
    let (dir, calls) = shape("a/dir");
    assert_eq!(dir, Shape::Directory);
    // A directory is listed once.
    assert_eq!(
        calls[calls.len() - 2..],
        [
            format!("stat {}", below("a/dir")),
            format!("list {}", below("a/dir"))
        ]
    );
}

#[test]
fn a_located_symlink_or_a_symlink_inside_a_located_directory_is_build_output() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.symlink(&below("link"));
        f.file(&below("dir/a"), b"a");
        f.symlink(&below("dir/sub/link"));
    });
    for rel in ["link", "dir"] {
        let built = {
            let again = Rig {
                case: Case::bazel(),
                runner: Handle::new(FakeProcessRunner::new(&ManualClock::at(0))),
                files: rig.files.clone(),
            };
            again.runner.with(|r| r.program(BIN));
            again.script_build(&format!("{rel}\n"), &format!("{EXEC_ROOT}\n"));
            again.build()
        };
        assert_eq!(built, Err(SeamError::Refused(Reason::BuildOutput)), "{rel}");
    }
}

#[test]
fn a_failing_stat_is_unknown_and_not_build_output() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.file(&below(REL), b"x");
        f.unreadable(&below(REL));
    });
    rig.script_build(&format!("{REL}\n"), &format!("{EXEC_ROOT}\n"));
    assert!(matches!(rig.build(), Err(SeamError::Unknown(_))));
}

#[test]
fn a_listing_of_nothing_two_outputs_or_a_root_that_is_not_a_path_is_build_output() {
    let refused = Err(SeamError::Refused(Reason::BuildOutput));
    let root = format!("{EXEC_ROOT}\n");
    for (query, info) in [
        (String::new(), root.clone()),
        (format!("{REL}\n{REL}2\n"), root.clone()),
        (format!("{REL}\n"), "relative/root\n".to_owned()),
        (format!("/{REL}\n"), root),
    ] {
        let rig = Rig::new();
        rig.files.with(|f| f.file(&below(REL), b"x"));
        rig.script_build(&query, &info);
        assert_eq!(rig.build(), refused, "{query:?} {info:?}");
    }
}

#[test]
fn a_locating_call_that_rejects_a_flag_fails_the_build_and_names_the_verb() {
    for (failing, named) in [(Verb::Query, "the query of"), (Verb::Info, "the info of")] {
        let rig = Rig::new();
        rig.script(Verb::Build, 0, "", "");
        for verb in [Verb::Query, Verb::Info] {
            if verb == failing {
                rig.script(verb, 2, "", "ERROR: Unrecognized option: --keep_going");
            } else {
                rig.script(verb, 0, "x\n", "");
            }
        }
        let SeamError::Permanent(why) = rig.build().unwrap_err() else {
            panic!("a locating call that fails fails the build for good");
        };
        assert!(why.contains(named), "{why}");
        assert!(why.contains("exited 2"), "{why}");
        assert!(why.contains("Unrecognized option"), "{why}");
    }
}

#[test]
fn the_test_report_is_the_build_event_file() {
    let rig = Rig::new();
    let report = rig.case.report();
    let lines = [
        r#"{"started":{}}"#,
        r#"{"id":{"testSummary":{"label":"//pkg:out"}},"testSummary":{"totalRunCount":2,"overallStatus":"PASSED"}}"#,
        r#"{"testSummary":{"totalRunCount":5,"overallStatus":"NO_STATUS"}}"#,
        r#"{"testSummary":{"totalRunCount":1,"overallStatus":"FLAKY"}}"#,
    ];
    rig.files
        .with(|f| f.file(&report, lines.join("\n").as_bytes()));
    rig.script(Verb::Test, 0, "", "");
    let outcome = rig.test().unwrap();
    assert_eq!((outcome.passed, outcome.ran), (true, 3));
    assert_eq!(
        rig.file_calls(),
        [format!("stat {report}"), format!("read {report}")]
    );
    assert_eq!(rig.calls()[0].command, rig.case.command(Verb::Test));
}

#[test]
fn a_test_that_ran_nothing_is_not_a_pass() {
    let summaries = [
        r#"{"testSummary":{"totalRunCount":4,"overallStatus":"FAILED_TO_BUILD"}}"#,
        r#"{"testSummary":{"totalRunCount":4,"overallStatus":"TOOL_HALTED_BEFORE_TESTING"}}"#,
        r#"{"testSummary":{"totalRunCount":0,"overallStatus":"PASSED"}}"#,
        "this is not a report",
    ];
    // Each of them, a report that is missing and one that is a directory.
    for setup in 0..summaries.len() + 2 {
        let rig = Rig::new();
        let report = rig.case.report();
        rig.script(Verb::Test, 0, "", "");
        rig.files.with(|f| match summaries.get(setup) {
            Some(text) => f.file(&report, text.as_bytes()),
            None if setup == summaries.len() => {}
            None => f.dir(&report),
        });
        let outcome = rig.test().unwrap();
        assert_eq!((outcome.passed, outcome.ran), (false, 0), "{setup}");
    }
}

#[test]
fn a_nonzero_exit_is_permanent_with_the_tail() {
    let rig = Rig::new();
    rig.script(Verb::Build, 1, "", "ERROR: no such target");
    let SeamError::Permanent(why) = rig.build().unwrap_err() else {
        panic!("a failed build is Permanent");
    };
    assert!(why.contains("exited 1"), "{why}");
    assert!(why.contains("ERROR: no such target"), "{why}");
    // Nothing else was run after the build that failed.
    assert_eq!(rig.calls().len(), 1);
    // A test that ran and failed is an outcome that did not pass, with the tail.
    let rig = Rig::new();
    let report = rig.case.report();
    rig.files.with(|f| {
        f.file(
            &report,
            br#"{"testSummary":{"totalRunCount":1,"overallStatus":"FAILED"}}"#,
        );
    });
    rig.script(Verb::Test, 3, "", "FAILED //pkg:out");
    let outcome = rig.test().unwrap();
    assert_eq!((outcome.passed, outcome.ran), (false, 1));
    assert!(
        outcome.tail.contains("FAILED //pkg:out"),
        "{}",
        outcome.tail
    );
    assert!(outcome.tail.contains("exited 3"), "{}", outcome.tail);
}

#[test]
fn a_tool_that_cannot_be_started_or_a_flag_that_cannot_be_layered_runs_nothing_good() {
    let rig = Rig::new();
    rig.script(Verb::Build, 0, "", "");
    rig.script(Verb::Test, 0, "", "");
    rig.runner.with(FakeProcessRunner::unstartable);
    assert!(matches!(rig.build(), Err(SeamError::Unknown(_))));
    assert!(matches!(rig.test(), Err(SeamError::Unknown(_))));
    let mut rig = Rig::new();
    rig.case.system.flags.push("{unknown}".to_owned());
    assert!(matches!(rig.build(), Err(SeamError::Permanent(_))));
    assert!(matches!(rig.test(), Err(SeamError::Permanent(_))));
    assert!(rig.calls().is_empty());
}

#[test]
fn a_stop_ends_the_invocation_and_answers_lease_lost() {
    let rig = Rig::new();
    rig.script_build("", "");
    rig.script(Verb::Test, 0, "", "");
    rig.runner.with(|r| r.beats(3, Duration::from_millis(1)));
    let build = rig
        .system()
        .build(&rig.case.inv(Verb::Build, None), &Beats::stopping_at(1));
    assert_eq!(build, Err(SeamError::Refused(Reason::LeaseLost)));
    let report = rig.case.report();
    let test = rig.system().test(
        &rig.case.inv(Verb::Test, Some(&report)),
        &Beats::stopping_at(1),
    );
    assert_eq!(test.unwrap_err(), SeamError::Refused(Reason::LeaseLost));
}

#[test]
fn copy_out_writes_a_file_under_its_name_and_a_directory_under_its_files() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.file(&below("pkg/out"), b"the file");
        f.file(&below("pkg/layout/index.json"), b"i");
    });
    let at = |rel: &str, shape| Located {
        label: LABEL.to_owned(),
        path: below(rel),
        shape,
    };
    let mut entry = Entry::default();
    rig.system()
        .copy_out(&at("pkg/out", Shape::File), &mut entry)
        .unwrap();
    rig.system()
        .copy_out(&at("pkg/layout", Shape::Directory), &mut entry)
        .unwrap();
    assert_eq!(
        entry.written,
        [
            ("out".to_owned(), b"the file".to_vec()),
            ("layout/index.json".to_owned(), b"i".to_vec()),
        ]
    );
}
