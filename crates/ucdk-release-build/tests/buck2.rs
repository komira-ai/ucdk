// SPDX-License-Identifier: Apache-2.0
//! `Buck2` over a process runner scripted with the output of the tool and the fake files: the
//! command it runs, the output it finds and checks, the report it reads, and how it ends.

mod support;

use support::{Brittle, Case, Entry, Faults, Handle, LABEL, ROOT, argv};
use ucdk_core::ManualClock;
use ucdk_release::invocation::Verb;
use ucdk_release_build::Buck2;
use ucdk_release_fake::{FakeOutputFiles, FakeProcessRunner, RunCall};
use ucdk_release_proto::v1::{BuildSystemKind, Reason};
use ucdk_release_seams::kit::Beats;
use ucdk_release_seams::{BuildSystem, DirEntry, Exit, FileKind, Located, SeamError, Shape};

const BIN: &str = "/opt/buck2";

/// A Buck2 over a scripted tool, and the ways into the fakes it holds.
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
            case: Case::buck2(),
            runner,
            files: Handle::new(FakeOutputFiles::new()),
        }
    }

    /// A tool that has been asked nothing and answers nothing yet.
    fn fresh(&self) {
        self.runner.with(|r| {
            *r = FakeProcessRunner::new(&ManualClock::at(0));
            r.program(BIN);
        });
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

    fn system(&self) -> Buck2<Handle<FakeProcessRunner>, Handle<FakeOutputFiles>> {
        Buck2::new(self.runner.clone(), self.files.clone(), ROOT)
    }

    fn build(&self) -> Result<Vec<Located>, SeamError> {
        self.system()
            .build(&self.case.inv(Verb::Build, None), &Beats::counting())
    }

    fn test(&self) -> Result<ucdk_release_seams::TestOutcome, SeamError> {
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

/// A build whose listing names `path`, on a tool that has done nothing else.
fn result(rig: &Rig, path: &str) -> Result<Vec<Located>, SeamError> {
    rig.fresh();
    rig.script(Verb::Build, 0, &format!("root//pkg:out {path}\n"), "");
    rig.build()
}

#[test]
fn the_command_line_is_the_golden_for_each_verb() {
    let rig = Rig::new();
    let system = rig.system();
    let report = rig.case.report();
    let build = system.command(&rig.case.inv(Verb::Build, None)).unwrap();
    assert_eq!(
        build.argv,
        argv(&[
            BIN,
            "build",
            "--config-file",
            "/repo/.buckconfig.ci",
            "@ci",
            "release.version=1.2.3",
            "--local-only",
            "--show-full-output",
            LABEL,
        ])
    );
    assert_eq!(build.env, [("LOG".to_owned(), "warn".to_owned())]);
    let test = system
        .command(&rig.case.inv(Verb::Test, Some(&report)))
        .unwrap();
    assert_eq!(
        test.argv,
        argv(&[
            BIN,
            "test",
            "--config-file",
            "/repo/.buckconfig.ci",
            "@ci",
            "release.version=1.2.3",
            "--local-only",
            "--event-log=/scratch/t1/events.json-lines",
            LABEL,
        ])
    );
    assert_eq!(test.env, build.env);
    // Buck2 has no call that locates an output apart from the build.
    for verb in [Verb::Query, Verb::Info] {
        let refusal = system.command(&rig.case.inv(verb, None)).unwrap_err();
        assert!(matches!(&refusal, SeamError::Permanent(why) if why.contains("no `")));
    }
    assert_eq!(system.kind(), BuildSystemKind::Buck2);
}

#[test]
fn a_listing_with_one_output_is_located() {
    let rig = Rig::new();
    rig.files.with(|f| f.file("/out/pkg/out", b"bytes"));
    let located = result(&rig, "/out/pkg/out").unwrap();
    assert_eq!(
        located,
        [Located {
            label: LABEL.to_owned(),
            path: "/out/pkg/out".to_owned(),
            shape: Shape::File,
        }]
    );
    // One call, the build, in the directory the system was given.
    assert_eq!(
        rig.calls(),
        [RunCall {
            command: rig.case.command(Verb::Build),
            cwd: ROOT.to_owned(),
        }]
    );
    // Locating asks what is at the path, and reads nothing.
    assert_eq!(rig.file_calls(), ["stat /out/pkg/out"]);
}

#[test]
fn a_path_with_a_space_is_one_path() {
    let rig = Rig::new();
    rig.files.with(|f| f.file("/out dir/with space/out", b"x"));
    let located = result(&rig, "/out dir/with space/out").unwrap();
    assert_eq!(located[0].path, "/out dir/with space/out");
}

#[test]
fn a_located_path_that_does_not_exist_is_build_output() {
    let rig = Rig::new();
    assert_eq!(
        result(&rig, "/out/not/built"),
        Err(SeamError::Refused(Reason::BuildOutput))
    );
}

#[test]
fn located_shape_is_what_stat_reports() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.file("/out/file", b"x");
        f.file("/out/dir/a", b"a");
        f.file("/out/dir/sub/b", b"b");
    });
    assert_eq!(result(&rig, "/out/file").unwrap()[0].shape, Shape::File);
    let before = rig.file_calls().len();
    assert_eq!(result(&rig, "/out/dir").unwrap()[0].shape, Shape::Directory);
    // A directory is listed once.
    assert_eq!(
        rig.file_calls()[before..],
        ["stat /out/dir", "list /out/dir"]
    );
}

#[test]
fn a_located_symlink_or_a_symlink_inside_a_located_directory_is_build_output() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.symlink("/out/link");
        f.file("/out/dir/a", b"a");
        f.symlink("/out/dir/sub/link");
    });
    for path in ["/out/link", "/out/dir"] {
        assert_eq!(
            result(&rig, path),
            Err(SeamError::Refused(Reason::BuildOutput)),
            "{path}"
        );
    }
}

#[test]
fn a_failing_stat_is_unknown_and_not_build_output() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.file("/out/file", b"x");
        f.unreadable("/out/file");
    });
    assert!(matches!(
        result(&rig, "/out/file"),
        Err(SeamError::Unknown(_))
    ));
}

#[test]
fn a_test_that_ran_nothing_is_not_a_pass() {
    // A report with no result, one that is missing and one that is a directory are the same.
    for setup in 0..3 {
        let rig = Rig::new();
        let report = rig.case.report();
        rig.script(Verb::Test, 0, "", "");
        rig.files.with(|f| match setup {
            0 => f.file(&report, b"{\"Event\":{}}\n"),
            1 => {}
            _ => f.dir(&report),
        });
        let outcome = rig.test().unwrap();
        assert_eq!((outcome.passed, outcome.ran), (false, 0), "{setup}");
    }
}

#[test]
fn the_test_report_is_the_event_log_and_skips_are_not_counted() {
    let rig = Rig::new();
    let report = rig.case.report();
    let lines = [
        r#"{"Event":{"data":{"TestResult":{"status":"SKIP"}}}}"#,
        r#"{"Event":{"data":{"TestResult":{"status":"PASS"}}}}"#,
        r#"{"Event":{"data":{"TestResult":{"status":2}}}}"#,
        r#"{"Event":{"data":{"TestResult":{"status":"LISTING_SUCCESS"}}}}"#,
        "not json",
    ];
    rig.files
        .with(|f| f.file(&report, lines.join("\n").as_bytes()));
    rig.script(Verb::Test, 0, "", "");
    let outcome = rig.test().unwrap();
    assert_eq!((outcome.passed, outcome.ran), (true, 2));
    // The file read is the one the invocation was told to write.
    assert_eq!(
        rig.file_calls(),
        [format!("stat {report}"), format!("read {report}")]
    );
    assert_eq!(rig.calls()[0].command, rig.case.command(Verb::Test));
}

#[test]
fn a_nonzero_exit_is_permanent_with_the_tail() {
    let rig = Rig::new();
    rig.script(Verb::Build, 3, "", "error: the target does not build");
    let SeamError::Permanent(why) = rig.build().unwrap_err() else {
        panic!("a failed build is Permanent");
    };
    assert!(why.contains("exited 3"), "{why}");
    assert!(why.contains("error: the target does not build"), "{why}");
    assert!(why.contains(LABEL), "{why}");
    // A signal is not an exit code, and is told so.
    let rig = Rig::new();
    rig.script(Verb::Build, 0, "", "");
    rig.runner.with(|r| r.signal(1));
    let SeamError::Permanent(why) = rig.build().unwrap_err() else {
        panic!("a killed build is Permanent");
    };
    assert!(why.contains("killed by signal 9"), "{why}");
}

#[test]
fn a_tool_that_cannot_be_started_is_unknown() {
    let rig = Rig::new();
    rig.script(Verb::Build, 0, "", "");
    rig.script(Verb::Test, 0, "", "");
    rig.runner.with(FakeProcessRunner::unstartable);
    assert!(matches!(rig.build(), Err(SeamError::Unknown(_))));
    assert!(matches!(rig.test(), Err(SeamError::Unknown(_))));
}

#[test]
fn a_listing_of_another_target_two_outputs_or_nothing_is_build_output() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.file("/out/a", b"a");
        f.file("/out/b", b"b");
    });
    for listing in [
        "root//other:thing /out/a\n",
        "root//pkg:out /out/a\nroot//pkg:out /out/b\n",
        "",
        "root//pkg:out relative/out\n",
    ] {
        rig.fresh();
        rig.script(Verb::Build, 0, listing, "");
        assert_eq!(
            rig.build(),
            Err(SeamError::Refused(Reason::BuildOutput)),
            "{listing:?}"
        );
    }
}

#[test]
fn a_flag_that_cannot_be_layered_is_permanent_and_nothing_is_run() {
    let mut rig = Rig::new();
    rig.case.target.flags.push("{unknown}".to_owned());
    assert!(matches!(rig.build(), Err(SeamError::Permanent(_))));
    assert!(matches!(rig.test(), Err(SeamError::Permanent(_))));
    assert!(rig.calls().is_empty());
}

#[test]
fn a_test_that_fails_is_not_a_pass_and_names_what_it_printed_last() {
    let rig = Rig::new();
    let report = rig.case.report();
    rig.files.with(|f| {
        f.file(&report, b"{\"TestResult\":{\"status\":\"FAIL\"}}\n");
    });
    rig.script(Verb::Test, 32, "", "FAILED //pkg:out");
    let outcome = rig.test().unwrap();
    assert_eq!((outcome.passed, outcome.ran), (false, 1));
    assert!(
        outcome.tail.contains("FAILED //pkg:out"),
        "{}",
        outcome.tail
    );
    assert!(outcome.tail.contains("exited 32"), "{}", outcome.tail);
}

#[test]
fn a_report_that_cannot_be_examined_or_read_is_unknown() {
    let rig = Rig::new();
    let report = rig.case.report();
    rig.script(Verb::Test, 0, "", "");
    rig.files.with(|f| {
        f.file(&report, b"{}");
        f.unreadable(&report);
    });
    assert!(matches!(rig.test(), Err(SeamError::Unknown(_))));
    // `stat` answers a file and then the read fails.
    let brittle = Brittle {
        files: Handle::new(FakeOutputFiles::new()),
        faults: Faults {
            read_fails: true,
            ..Faults::default()
        },
    };
    brittle.files.with(|f| f.file(&report, b"{}"));
    let mut system = Buck2::new(rig.runner.clone(), brittle, ROOT);
    let refusal = system.test(&rig.case.inv(Verb::Test, Some(&report)), &Beats::counting());
    assert!(matches!(refusal, Err(SeamError::Unknown(_))));
}

#[test]
fn a_stop_ends_the_invocation_and_answers_lease_lost() {
    let rig = Rig::new();
    rig.script(Verb::Build, 0, "", "");
    rig.script(Verb::Test, 0, "", "");
    rig.runner
        .with(|r| r.beats(3, std::time::Duration::from_millis(1)));
    let stopping = Beats::stopping_at(1);
    let build = rig
        .system()
        .build(&rig.case.inv(Verb::Build, None), &stopping);
    assert_eq!(build, Err(SeamError::Refused(Reason::LeaseLost)));
    let report = rig.case.report();
    let test = rig.system().test(
        &rig.case.inv(Verb::Test, Some(&report)),
        &Beats::stopping_at(1),
    );
    assert_eq!(test.unwrap_err(), SeamError::Refused(Reason::LeaseLost));
}

fn written(rig: &Rig, located: &Located) -> Result<Vec<(String, Vec<u8>)>, SeamError> {
    let mut entry = Entry::default();
    rig.system().copy_out(located, &mut entry)?;
    Ok(entry.written)
}

fn at(path: &str, shape: Shape) -> Located {
    Located {
        label: LABEL.to_owned(),
        path: path.to_owned(),
        shape,
    }
}

#[test]
fn copy_out_writes_a_file_under_its_name_and_a_directory_under_the_files_below_it() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.file("/out/pkg/out", b"the file");
        f.file("/out/layout/index.json", b"i");
        f.file("/out/layout/blobs/sha256/aa", b"b");
        f.dir("/out/layout/empty");
    });
    assert_eq!(
        written(&rig, &at("/out/pkg/out", Shape::File)).unwrap(),
        [("out".to_owned(), b"the file".to_vec())]
    );
    // A trailing slash on the located path is not part of the name.
    assert_eq!(
        written(&rig, &at("/out/layout/", Shape::Directory)).unwrap(),
        [
            ("layout/blobs/sha256/aa".to_owned(), b"b".to_vec()),
            ("layout/index.json".to_owned(), b"i".to_vec()),
        ]
    );
}

#[test]
fn copy_out_refuses_what_is_not_what_was_located() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.file("/out/file", b"x");
        f.file("/out/dir/a", b"a");
        f.symlink("/out/dir/link");
        f.symlink("/out/link");
    });
    let refused = Err(SeamError::Refused(Reason::BuildOutput));
    // Nothing there now, a symlink, and a file where a directory was or the reverse.
    for located in [
        at("/out/gone", Shape::File),
        at("/out/link", Shape::File),
        at("/out/dir", Shape::File),
        at("/out/file", Shape::Directory),
        at("/out/dir", Shape::Directory),
    ] {
        assert_eq!(written(&rig, &located), refused, "{located:?}");
    }
}

#[test]
fn copy_out_fails_where_the_tree_or_the_entry_fails() {
    let rig = Rig::new();
    rig.files.with(|f| {
        f.file("/out/file", b"x");
        f.file("/out/dir/a", b"a");
        f.unreadable("/out/unreadable");
    });
    let mut entry = Entry {
        refuse: true,
        ..Entry::default()
    };
    // An entry that refuses a file, for a file and for a directory.
    for located in [
        at("/out/file", Shape::File),
        at("/out/dir", Shape::Directory),
    ] {
        let refusal = rig.system().copy_out(&located, &mut entry);
        assert!(
            matches!(refusal, Err(SeamError::Permanent(_))),
            "{located:?}"
        );
    }
    // A path that cannot be examined.
    assert!(matches!(
        written(&rig, &at("/out/unreadable", Shape::File)),
        Err(SeamError::Unknown(_))
    ));
    // A directory whose listing fails, a file that cannot be read, and a listing that names a
    // path no entry can have.
    let faults = [
        Faults {
            list_fails: true,
            ..Faults::default()
        },
        Faults {
            read_fails: true,
            ..Faults::default()
        },
        Faults {
            listed: Some(vec![DirEntry {
                rel: "../x".to_owned(),
                kind: FileKind::File,
            }]),
            ..Faults::default()
        },
    ];
    for faults in faults {
        let mut system = Buck2::new(
            rig.runner.clone(),
            Brittle {
                files: rig.files.clone(),
                faults,
            },
            ROOT,
        );
        let refusal = system.copy_out(&at("/out/dir", Shape::Directory), &mut Entry::default());
        assert!(refusal.is_err());
    }
    let mut system = Buck2::new(
        rig.runner.clone(),
        Brittle {
            files: rig.files.clone(),
            faults: Faults {
                read_fails: true,
                ..Faults::default()
            },
        },
        ROOT,
    );
    assert!(
        system
            .copy_out(&at("/out/file", Shape::File), &mut Entry::default())
            .is_err()
    );
}

#[test]
fn the_fake_files_are_not_changed_by_a_build_or_a_copy() {
    let rig = Rig::new();
    rig.files.with(|f| f.file("/out/pkg/out", b"bytes"));
    let before = rig.files.with(|f| f.snapshot());
    result(&rig, "/out/pkg/out").unwrap();
    written(&rig, &at("/out/pkg/out", Shape::File)).unwrap();
    assert_eq!(rig.files.with(|f| f.snapshot()), before);
}

#[test]
fn a_directory_that_cannot_be_listed_is_unknown_and_not_build_output() {
    let rig = Rig::new();
    rig.files.with(|f| f.file("/out/dir/a", b"a"));
    rig.script(Verb::Build, 0, "root//pkg:out /out/dir\n", "");
    let brittle = Brittle {
        files: rig.files.clone(),
        faults: Faults {
            list_fails: true,
            ..Faults::default()
        },
    };
    let mut system = Buck2::new(rig.runner.clone(), brittle, ROOT);
    let built = system.build(&rig.case.inv(Verb::Build, None), &Beats::counting());
    assert!(matches!(built, Err(SeamError::Unknown(_))), "{built:?}");
}
