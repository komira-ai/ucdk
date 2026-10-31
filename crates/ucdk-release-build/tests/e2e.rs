// SPDX-License-Identifier: Apache-2.0
//! `Buck2` and `Bazel` over the real process runner and the real files, with a stand-in for the
//! tool (the program of `tests/child` told what to print and what to write), so that the
//! whole path is run on this machine: the tool is found on the `PATH` the build system's own
//! environment gives, it runs in the directory it was given, what it wrote is checked on the
//! disk and copied out, and the report it wrote is read. The real Buck2 and Bazel are not run
//! here; the on-demand job of the design runs them.

mod support;

use std::fs;
use std::path::Path;

use support::{Case, Entry, LABEL, Never, hex};
use tempfile::TempDir;
use ucdk_release::invocation::Verb;
use ucdk_release_build::{Bazel, Buck2, LocalFiles, OsProcessRunner};
use ucdk_release_seams::{BuildSystem, Located, SeamError, Shape};

const CHILD: &str = env!("CARGO_BIN_EXE_ucdk-test-child");

fn scratch() -> (TempDir, String) {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("a directory");
    let path = fs::canonicalize(dir.path())
        .expect("its path")
        .to_string_lossy()
        .into_owned();
    (dir, path)
}

/// A stand-in for the tool called `name` in `bin` below the scratch directory, with these
/// lines to tell it what to do.
fn install(scratch: &str, name: &str, lines: &[String]) {
    let path = format!("{scratch}/bin/{name}");
    fs::create_dir_all(Path::new(&path).parent().expect("a directory")).expect("bin");
    fs::hard_link(CHILD, &path)
        .or_else(|_| fs::copy(CHILD, &path).map(|_| ()))
        .expect("the program is put in place");
    fs::write(
        format!("{path}.behavior"),
        format!("tool\n{}\n", lines.join("\n")),
    )
    .expect("its behavior");
}

fn file_line(path: &str, bytes: &[u8]) -> String {
    format!("file {} {}", hex(path.as_bytes()), hex(bytes))
}

fn rule(verb: &str, code: i32, stdout: &str, stderr: &str, report: &str) -> String {
    format!(
        "rule {verb} {code} {} {} {}",
        hex(stdout.as_bytes()),
        hex(stderr.as_bytes()),
        hex(report.as_bytes())
    )
}

/// The case with its binary named without a path, found on the `PATH` of its own environment.
fn found_on_the_path(mut case: Case, name: &str, scratch: &str) -> Case {
    case.system.binary = name.to_owned();
    case.system
        .env
        .insert("PATH".to_owned(), format!("{scratch}/bin"));
    case
}

#[test]
fn buck2_builds_copies_and_tests_through_the_real_runner_and_the_real_files() {
    let (_dir, s) = scratch();
    let out = format!("{s}/out dir/out");
    install(
        &s,
        "buck2",
        &[
            file_line(&out, b"the built bytes"),
            rule("build", 0, &format!("root//pkg:out {out}\n"), "", ""),
            rule(
                "test",
                0,
                "",
                "",
                "{\"TestResult\":{\"status\":\"PASS\"}}\n",
            ),
        ],
    );
    let case = found_on_the_path(Case::buck2(), "buck2", &s);
    let mut system = Buck2::new(OsProcessRunner, LocalFiles, s.clone());
    let located = system.build(&case.inv(Verb::Build, None), &Never).unwrap();
    assert_eq!(
        located,
        [Located {
            label: LABEL.to_owned(),
            path: out,
            shape: Shape::File,
        }]
    );
    let mut entry = Entry::default();
    system.copy_out(&located[0], &mut entry).unwrap();
    assert_eq!(
        entry.written,
        [("out".to_owned(), b"the built bytes".to_vec())]
    );
    // The report is written where the invocation said, and read from there.
    fs::create_dir_all(format!("{s}/t1")).unwrap();
    let report = format!("{s}/t1/events.json-lines");
    let outcome = system
        .test(&case.inv(Verb::Test, Some(&report)), &Never)
        .unwrap();
    assert_eq!((outcome.passed, outcome.ran), (true, 1));
    assert!(fs::read_to_string(&report).unwrap().contains("PASS"));
}

#[test]
fn bazel_builds_locates_and_tests_through_the_real_runner_and_the_real_files() {
    let (_dir, s) = scratch();
    let root = format!("{s}/exec root");
    let rel = "bazel-out/bin/pkg/out";
    install(
        &s,
        "bazel",
        &[
            file_line(&format!("{root}/{rel}"), b"the built bytes"),
            rule("build", 0, "", "", ""),
            rule("cquery", 0, &format!("{rel}\n"), "", ""),
            rule("info", 0, &format!("{root}\n"), "", ""),
            rule(
                "test",
                0,
                "",
                "",
                "{\"testSummary\":{\"totalRunCount\":1,\"overallStatus\":\"PASSED\"}}\n",
            ),
        ],
    );
    let case = found_on_the_path(Case::bazel(), "bazel", &s);
    let mut system = Bazel::new(OsProcessRunner, LocalFiles, s.clone());
    let located = system.build(&case.inv(Verb::Build, None), &Never).unwrap();
    assert_eq!(located[0].path, format!("{root}/{rel}"));
    assert_eq!(located[0].shape, Shape::File);
    let mut entry = Entry::default();
    system.copy_out(&located[0], &mut entry).unwrap();
    assert_eq!(
        entry.written,
        [("out".to_owned(), b"the built bytes".to_vec())]
    );
    fs::create_dir_all(format!("{s}/t1")).unwrap();
    let report = format!("{s}/t1/events.json");
    let outcome = system
        .test(&case.inv(Verb::Test, Some(&report)), &Never)
        .unwrap();
    assert_eq!((outcome.passed, outcome.ran), (true, 1));
}

#[test]
fn a_failing_tool_is_permanent_with_its_tail_and_a_missing_one_is_unknown() {
    let (_dir, s) = scratch();
    install(
        &s,
        "buck2",
        &[rule(
            "build",
            3,
            "",
            "error: the target does not build\n",
            "",
        )],
    );
    let case = found_on_the_path(Case::buck2(), "buck2", &s);
    let mut system = Buck2::new(OsProcessRunner, LocalFiles, s.clone());
    let SeamError::Permanent(why) = system
        .build(&case.inv(Verb::Build, None), &Never)
        .unwrap_err()
    else {
        panic!("a tool that fails fails the build for good");
    };
    assert!(why.contains("exited 3"), "{why}");
    assert!(why.contains("error: the target does not build"), "{why}");
    // The same tool is not on a PATH that does not hold it: nothing ran.
    let mut elsewhere = Case::buck2();
    elsewhere.system.binary = "buck2".to_owned();
    elsewhere
        .system
        .env
        .insert("PATH".to_owned(), format!("{s}/nowhere"));
    let missing = system.build(&elsewhere.inv(Verb::Build, None), &Never);
    assert!(matches!(missing, Err(SeamError::Unknown(_))), "{missing:?}");
}
