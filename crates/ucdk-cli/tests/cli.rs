// SPDX-License-Identifier: Apache-2.0
//! The command line: usage errors, the version, `ledger show` over the golden ledger files, and
//! the real binary.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use ucdk_cli::{USAGE, World};
use ucdk_core::{Locator, ManualClock, RecordPath, Tag};
use ucdk_ledger::{Change, FileLedger, Ledger as _, Origin, Record, State, escape_token};

/// What a command did: its exit code and what it printed.
#[derive(Debug)]
struct Ran {
    code: u8,
    out: String,
    err: String,
}

fn cmd(world: &mut World, args: &[&str]) -> Ran {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let code = ucdk_cli::run(&args, world);
    Ran {
        code,
        out: std::mem::take(&mut world.out),
        err: std::mem::take(&mut world.err),
    }
}

fn golden(name: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sdk/ledger-golden")
        .join(name)
        .to_string_lossy()
        .into_owned()
}

/// A directory under the build's scratch area, empty on creation and removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make the scratch directory");
        Self(dir)
    }

    fn file(&self) -> String {
        self.0.join("ledger.ucdk").to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_prints_version() {
    let ran = cmd(&mut World::new(), &["--version"]);
    assert_eq!((ran.code, ran.err.as_str()), (0, ""));
    assert_eq!(ran.out, format!("ucdk {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_default_world_is_a_new_one() {
    let world = World::default();
    assert_eq!((world.out.as_str(), world.err.as_str()), ("", ""));
    // Its clock is the wall clock: it reads a time well after the year 2020, in milliseconds.
    assert!(world.clock.now_millis() > 1_577_836_800_000);
}

#[test]
fn version_with_an_extra_argument_is_usage() {
    for extra in [vec!["--version", "extra"], vec!["--version", "--ledger=x"]] {
        let ran = cmd(&mut World::new(), &extra);
        assert_eq!((ran.code, ran.out.as_str()), (2, ""), "{extra:?}");
        assert!(
            ran.err.contains("`--version` takes no argument"),
            "{}",
            ran.err
        );
        assert!(ran.err.contains(USAGE));
    }
}

#[test]
fn usage_errors_name_the_problem_and_exit_2() {
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec![], "a command is needed"),
        (vec!["list"], "`list` is not a command"),
        (vec!["--ledger=x"], "`--ledger=x` is not a command"),
        (vec!["ledger"], "`ledger` needs a command"),
        (vec!["ledger", "list"], "`ledger list` is not a command"),
        (vec!["ledger", "show"], "`--ledger` is needed"),
        (
            vec!["ledger", "show", "--ledger"],
            "`--ledger` needs a value",
        ),
        (
            vec!["ledger", "show", "--ledger=a", "--ledger=b"],
            "`--ledger` is given twice",
        ),
        (
            vec!["ledger", "show", "--ledger=a", "--bogus=1"],
            "`--bogus` is not a flag of this command",
        ),
    ];
    for (args, message) in cases {
        let ran = cmd(&mut World::new(), &args);
        assert_eq!((ran.code, ran.out.as_str()), (2, ""), "{args:?}");
        assert!(ran.err.contains(message), "{args:?}: {}", ran.err);
        assert!(ran.err.contains(USAGE), "{args:?}");
    }
}

#[test]
fn an_unknown_verb_is_usage() {
    let ran = cmd(&mut World::new(), &["plan", "--ledger=x"]);
    assert_eq!((ran.code, ran.out.as_str()), (2, ""));
    assert!(
        ran.err
            .starts_with("ucdk: `plan` is not a command\nusage: ucdk --version\n")
    );
}

#[test]
fn a_positional_argument_is_usage() {
    let ran = cmd(&mut World::new(), &["ledger", "show", "ledger.ucdk"]);
    assert_eq!((ran.code, ran.out.as_str()), (2, ""));
    assert!(
        ran.err.contains("`ledger.ucdk` is not a flag"),
        "{}",
        ran.err
    );
}

#[test]
fn ledger_show_without_a_ledger_flag_is_usage() {
    let ran = cmd(&mut World::new(), &["ledger", "show", "--holder=ci-7"]);
    assert_eq!((ran.code, ran.out.as_str()), (2, ""));
    assert!(
        ran.err.starts_with("ucdk: `--ledger` is needed\n"),
        "{}",
        ran.err
    );
}

#[test]
fn a_flag_may_take_its_value_as_the_next_argument() {
    let file = golden("leased.ucdk");
    let ran = cmd(&mut World::new(), &["ledger", "show", "--ledger", &file]);
    assert_eq!(ran.code, 0, "{ran:?}");
    assert!(ran.out.starts_with("token v0\nfence 1\nlease ci-7 "));
}

/// What `ledger show` prints for the records golden: one line each, in path order.
const RECORDS_TEXT: &str = "\
token v5
fence 0
lease none
record app/db service app-db created confirmed physical=db-1 deps= HOST=db-host PORT=5432
record app/old job app-old created reaping physical=old-1 deps=
record app/pending job app-pending created intent physical=% deps=
record app/run service app-run created confirmed physical=run-1 deps=app/db URL=run-url
record shared/found volume shared-found adopted confirmed physical=found-1 deps= EMPTY=% NOTE=two%20words
";

#[test]
fn ledger_show_prints_the_golden_document() {
    let file = golden("records.ucdk");
    let ran = cmd(
        &mut World::new(),
        &["ledger", "show", &format!("--ledger={file}")],
    );
    assert_eq!((ran.code, ran.err.as_str()), (0, ""));
    assert_eq!(ran.out, RECORDS_TEXT);

    // The leased golden, read while its hour lasts: `ci-7` took the lease at 1,000,000 ms.
    let leased = golden("leased.ucdk");
    let mut world = World::with_clock(Arc::new(ManualClock::at(1_000_001)));
    let ran = cmd(
        &mut world,
        &["ledger", "show", &format!("--ledger={leased}")],
    );
    assert_eq!((ran.code, ran.err.as_str()), (0, ""));
    assert_eq!(
        ran.out,
        "token v0\nfence 1\nlease ci-7 expires_ms 4600000 live other\n"
    );
}

#[test]
fn ledger_show_with_holder_marks_the_lease_as_yours() {
    let leased = golden("leased.ucdk");
    let at = |ms: u64, holder: Option<&str>| {
        let mut world = World::with_clock(Arc::new(ManualClock::at(ms)));
        let mut args = vec!["ledger", "show"];
        let ledger = format!("--ledger={leased}");
        args.push(&ledger);
        let flag = holder.map(|h| format!("--holder={h}"));
        if let Some(flag) = &flag {
            args.push(flag);
        }
        cmd(&mut world, &args).out
    };
    let lease_line = |out: String| out.lines().nth(2).expect("a lease line").to_owned();
    assert_eq!(
        lease_line(at(1_000_001, Some("ci-7"))),
        "lease ci-7 expires_ms 4600000 live yours"
    );
    assert_eq!(
        lease_line(at(1_000_001, Some("ci-8"))),
        "lease ci-7 expires_ms 4600000 live other"
    );
    // With no holder named, the default holder is used, which no lease names.
    assert_eq!(
        lease_line(at(1_000_001, None)),
        "lease ci-7 expires_ms 4600000 live other"
    );
    // By the clock of the world: the hour at 4,600,000 ms, the skew bound after it.
    assert!(lease_line(at(4_600_000, Some("ci-7"))).ends_with(" lapsing yours"));
    assert!(lease_line(at(4_605_000, Some("ci-7"))).ends_with(" lapsed yours"));
}

#[test]
fn ledger_show_of_an_unreadable_file_is_class_4() {
    let scratch = Scratch::new("unreadable");
    let cases = [
        // Missing.
        None,
        // Not a ledger.
        Some("not a ledger\n"),
    ];
    for content in cases {
        if let Some(content) = content {
            std::fs::write(scratch.file(), content).expect("write");
        }
        let ran = cmd(
            &mut World::new(),
            &["ledger", "show", &format!("--ledger={}", scratch.file())],
        );
        assert_eq!((ran.code, ran.out.as_str()), (4, ""), "{ran:?}");
        assert!(ran.err.starts_with("ucdk: "), "{}", ran.err);
        assert!(ran.err.ends_with('\n'));
    }
}

#[test]
fn ledger_show_escapes_every_value_as_the_ledger_file_does() {
    let scratch = Scratch::new("escapes");
    let physical = "id\nlease ci-9 expires_ms 1 live yours";
    let output = "v\nrecord x/y";
    let ledger = FileLedger::open_with_clock(scratch.file(), Arc::new(ManualClock::at(1_000)))
        .expect("open");
    ledger
        .acquire("ci-7\nlease evil 1 live yours", Duration::from_secs(60))
        .expect("acquire");
    let record = Record {
        origin: Origin::Created,
        state: State::Confirmed,
        physical: Some(Locator::new(physical)),
        deps: [RecordPath::new("db/run").expect("a path")].into(),
        outputs: BTreeMap::from([("k ey=".to_owned(), output.to_owned())]),
        ..Record::intent(
            RecordPath::new("app/run").expect("a path"),
            Tag::new("a kind"),
            "a name\nrecord forged/path",
            Default::default(),
        )
    };
    let token = ledger.read().expect("read").token;
    let fence = ledger.read().expect("read").fence;
    ledger
        .write_fenced(fence, &token, Change::Put(record))
        .expect("write");
    drop(ledger);

    let mut world = World::with_clock(Arc::new(ManualClock::at(2_000)));
    let ran = cmd(
        &mut world,
        &["ledger", "show", &format!("--ledger={}", scratch.file())],
    );
    assert_eq!((ran.code, ran.err.as_str()), (0, ""));
    let lines: Vec<&str> = ran.out.lines().collect();
    // Exactly one lease line (the real one) and one record line, whatever the values hold.
    assert_eq!(lines.len(), 4, "{}", ran.out);
    assert!(lines[0].starts_with("token ") && lines[1].starts_with("fence "));
    assert!(lines[2].starts_with("lease ") && lines[3].starts_with("record "));
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("lease ")).count(),
        1,
        "no value forged a lease line"
    );
    // The escaped tokens are the ones the file writer makes.
    assert!(lines[2].starts_with(&format!(
        "lease {} ",
        escape_token("ci-7\nlease evil 1 live yours")
    )));
    let record_line = lines[3];
    for value in [
        "app/run",
        "a kind",
        "a name\nrecord forged/path",
        physical,
        "db/run",
        "k ey=",
        output,
    ] {
        assert!(
            record_line.contains(&escape_token(value)),
            "{value:?} is not in {record_line}"
        );
    }
    assert_eq!(
        record_line.split(' ').count(),
        9,
        "no value shifted a field: {record_line}"
    );
}

#[test]
fn binary_unknown_flag_exits_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_ucdk"))
        .args(["ledger", "show", "--ledger=x", "--bogus=1"])
        .output()
        .expect("run ucdk");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("`--bogus` is not a flag"));
}

#[test]
fn binary_version_exits_0() {
    let out = Command::new(env!("CARGO_BIN_EXE_ucdk"))
        .arg("--version")
        .output()
        .expect("run ucdk");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("ucdk {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(out.stderr.is_empty());
}

#[test]
fn binary_shows_a_ledger_and_leaves_it_alone() {
    let file = golden("records.ucdk");
    let before = std::fs::read(&file).expect("read");
    let out = Command::new(env!("CARGO_BIN_EXE_ucdk"))
        .args(["ledger", "show", "--ledger", &file])
        .output()
        .expect("run ucdk");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&out.stdout), RECORDS_TEXT);
    assert_eq!(std::fs::read(&file).expect("read"), before);
}
