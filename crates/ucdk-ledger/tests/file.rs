// SPDX-License-Identifier: Apache-2.0
//! The file ledger: what survives a reopen, what a second open does, and what is refused when
//! the file is not a whole ledger of a version this build reads.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use common::{Scratch, confirmed, path, put, record, sealed};
use ucdk_core::{Locator, Tag};
use ucdk_ledger::{
    Change, FileLedger, LeaseError, LeaseState, Ledger, LedgerError, ManualClock, OpenError,
    Origin, Record, State,
};

fn open(scratch: &Scratch) -> FileLedger {
    FileLedger::open(scratch.file()).expect("open")
}

fn refused(scratch: &Scratch) -> OpenError {
    FileLedger::open(scratch.file()).expect_err("the open should be refused")
}

/// A record that uses every field, with values that need escaping.
fn full_record() -> Record {
    Record {
        origin: Origin::Adopted,
        state: State::Reaping,
        physical: Some(Locator::new("thing:one/with space%and\nnewline")),
        deps: BTreeSet::from([path("db/run"), path("cache/run")]),
        outputs: BTreeMap::from([
            ("URL".to_owned(), "a b?c=d%".to_owned()),
            ("EMPTY".to_owned(), String::new()),
            ("UNICODE".to_owned(), "caf\u{e9} \u{1f600}".to_owned()),
            ("odd name".to_owned(), "x".to_owned()),
        ]),
        ..Record::intent(
            path("app/db/run"),
            Tag::new("a kind"),
            "name with spaces",
            BTreeSet::new(),
        )
    }
}

// ---- what survives -----------------------------------------------------------------------

#[test]
fn nothing_is_written_until_the_first_write() {
    let scratch = Scratch::new("lazy");
    let ledger = open(&scratch);
    assert!(ledger.read().expect("read").records.is_empty());
    assert!(!scratch.file().exists(), "an open made no ledger file");
    put(&ledger, Change::Put(record("app/run")));
    assert!(scratch.file().exists());
}

#[test]
fn a_reopened_ledger_holds_every_field_of_every_record_and_the_same_token() {
    let scratch = Scratch::new("reopen");
    let (before, expected) = {
        let ledger = open(&scratch);
        put(&ledger, Change::Put(record("app/run")));
        put(&ledger, Change::Put(confirmed("db/run", "id-1")));
        put(&ledger, Change::Put(full_record()));
        put(&ledger, Change::Put(record("gone/run")));
        put(&ledger, Change::Remove(path("gone/run")));
        let snapshot = ledger.read().expect("read");
        (snapshot.token, snapshot.records)
    };
    assert_eq!(expected.len(), 3);
    let after = open(&scratch).read().expect("read");
    assert_eq!(after.token, before);
    assert_eq!(after.records, expected);
    assert_eq!(after.records[&path("app/db/run")], full_record());
}

#[test]
fn a_write_after_a_reopen_continues_the_versions() {
    let scratch = Scratch::new("continue");
    put(&open(&scratch), Change::Put(record("app/run")));
    let ledger = open(&scratch);
    let token = ledger.read().expect("read").token;
    assert_eq!(token.to_string(), "v1");
    let next = ledger
        .write(&token, Change::Put(record("db/run")))
        .expect("write");
    assert_eq!(next.to_string(), "v2");
    assert_eq!(open_after_drop(ledger, &scratch).records.len(), 2);
}

fn open_after_drop(ledger: FileLedger, scratch: &Scratch) -> ucdk_ledger::Snapshot {
    drop(ledger);
    open(scratch).read().expect("read")
}

#[cfg(unix)]
#[test]
fn the_ledger_and_its_lock_are_readable_by_their_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;
    let scratch = Scratch::new("mode");
    let ledger = open(&scratch);
    put(&ledger, Change::Put(record("app/run")));
    put(&ledger, Change::Put(record("db/run")));
    for name in ["ledger.ucdk", "ledger.ucdk.lock"] {
        let mode = fs::metadata(scratch.dir().join(name))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "{name}");
    }
}

#[test]
fn a_write_leaves_no_temporary_file_behind() {
    let scratch = Scratch::new("no_tmp");
    let ledger = open(&scratch);
    put(&ledger, Change::Put(record("app/run")));
    let mut names: Vec<String> = fs::read_dir(scratch.dir())
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["ledger.ucdk", "ledger.ucdk.lock"]);
}

#[test]
fn a_relative_path_with_no_directory_part_works() {
    // The directory flush needs a directory: the current one, when the path names none.
    let name = format!("ledger-relative-{}.ucdk", std::process::id());
    let ledger = FileLedger::open(&name).expect("open");
    put(&ledger, Change::Put(record("app/run")));
    drop(ledger);
    for suffix in ["", ".lock"] {
        fs::remove_file(format!("{name}{suffix}")).expect("remove");
    }
}

// ---- more than one handle ----------------------------------------------------------------

#[test]
fn a_second_handle_opens_beside_the_first_and_each_sees_what_the_other_wrote() {
    let scratch = Scratch::new("two_handles");
    let first = open(&scratch);
    let second = open(&scratch);
    put(&first, Change::Put(record("app/one")));
    assert_eq!(second.read().expect("read").records.len(), 1);
    put(&second, Change::Put(record("app/two")));
    assert_eq!(first.read().expect("read").records.len(), 2);
    // A write made against a token the other handle moved is stale, not lost.
    let old = first.read().expect("read").token;
    put(&second, Change::Put(record("app/three")));
    assert_eq!(
        first.write(&old, Change::Put(record("app/four"))),
        Err(LedgerError::Stale)
    );
}

#[test]
fn handles_writing_at_once_lose_no_write() {
    let scratch = Scratch::new("racing_handles");
    let file = scratch.file();
    let threads: Vec<_> = (0..4)
        .map(|n| {
            let file = file.clone();
            std::thread::spawn(move || {
                let ledger = FileLedger::open(&file).expect("open");
                for i in 0..10 {
                    // Each write is retried against the token it finds, as a lease guard does.
                    loop {
                        let token = ledger.read().expect("read").token;
                        let change = Change::Put(record(&format!("app{n}/r{i}")));
                        match ledger.write(&token, change) {
                            Ok(_) => break,
                            Err(LedgerError::Stale) => {}
                            Err(other) => panic!("{other:?}"),
                        }
                    }
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("a writer finished");
    }
    let snapshot = open(&scratch).read().expect("read");
    assert_eq!(snapshot.records.len(), 40);
    assert_eq!(snapshot.token.to_string(), "v40");
}

#[test]
fn a_handle_gives_the_file_lock_back_after_each_call() {
    // A second descriptor on the lock file takes the lock between two calls of the handle:
    // nothing holds it for the handle's life.
    let scratch = Scratch::new("lock_between_calls");
    let ledger = open(&scratch);
    put(&ledger, Change::Put(record("app/run")));
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(scratch.dir().join("ledger.ucdk.lock"))
        .expect("open the lock file");
    lock.try_lock().expect("the lock is free between calls");
    drop(lock);
    put(&ledger, Change::Put(record("app/again")));
}

#[test]
fn two_different_files_in_one_directory_do_not_lock_each_other() {
    let scratch = Scratch::new("two_files");
    let _a = FileLedger::open(scratch.dir().join("a.ucdk")).expect("open a");
    let _b = FileLedger::open(scratch.dir().join("b.ucdk")).expect("open b");
}

// ---- crashes -----------------------------------------------------------------------------

#[test]
fn a_temporary_file_left_by_a_crashed_write_is_ignored_and_removed() {
    let scratch = Scratch::new("stale_tmp");
    put(&open(&scratch), Change::Put(record("app/run")));
    // The process died after writing part of the next document and before the rename.
    let tmp = scratch.dir().join("ledger.ucdk.tmp");
    fs::write(&tmp, "ucdk-ledger 1.0\nversion 9\nrecord half").expect("write");
    let ledger = open(&scratch);
    assert!(!tmp.exists());
    let snapshot = ledger.read().expect("read");
    assert_eq!(snapshot.token.to_string(), "v1");
    assert_eq!(snapshot.records.len(), 1);
}

#[test]
fn a_failed_write_changes_neither_the_file_nor_what_the_handle_reads() {
    let scratch = Scratch::new("failed_write");
    let ledger = open(&scratch);
    put(&ledger, Change::Put(record("app/run")));
    let bytes = fs::read(scratch.file()).expect("read");
    let token = ledger.read().expect("read").token;
    // A directory where the temporary file belongs: the write cannot start.
    let blocker = scratch.dir().join("ledger.ucdk.tmp");
    fs::create_dir(&blocker).expect("block");
    let failed = ledger.write(&token, Change::Put(record("db/run")));
    assert!(
        matches!(failed, Err(LedgerError::Unavailable(_))),
        "{failed:?}"
    );
    assert_eq!(fs::read(scratch.file()).expect("read"), bytes);
    let snapshot = ledger.read().expect("read");
    assert_eq!(snapshot.token, token);
    assert_eq!(snapshot.records.len(), 1);
    // With the way clear, the same token still writes.
    fs::remove_dir(&blocker).expect("unblock");
    ledger
        .write(&token, Change::Put(record("db/run")))
        .expect("write");
}

#[test]
fn a_write_to_a_directory_that_has_gone_is_unavailable_and_reads_as_an_empty_ledger() {
    let scratch = Scratch::new("gone_dir");
    let dir = scratch.dir().join("sub");
    fs::create_dir(&dir).expect("mkdir");
    let ledger = FileLedger::open(dir.join("ledger.ucdk")).expect("open");
    put(&ledger, Change::Put(record("app/run")));
    let token = ledger.read().expect("read").token;
    fs::remove_dir_all(&dir).expect("remove the directory");
    // Every call goes by the file, and there is none: the ledger is empty, as a new one is.
    let snapshot = ledger.read().expect("read");
    assert!(snapshot.records.is_empty());
    assert_eq!(
        ledger.write(&token, Change::Put(record("db/run"))),
        Err(LedgerError::Stale)
    );
    let failed = ledger.write(&snapshot.token, Change::Put(record("db/run")));
    let Err(LedgerError::Unavailable(why)) = failed else {
        panic!("the write should be unavailable: {failed:?}");
    };
    assert!(why.contains("may or may not have landed"), "{why}");
}

#[test]
fn every_call_goes_by_the_file_as_it_stands_even_after_a_failed_write() {
    // Another handle's document is put in the file by hand; this handle's next write against
    // its old token is stale, and the one that then fails to write leaves the file as it was.
    let scratch = Scratch::new("reload");
    let ledger = open(&scratch);
    put(&ledger, Change::Put(record("app/run")));
    let token = ledger.read().expect("read").token;
    let other = scratch.dir().join("other");
    fs::create_dir(&other).expect("mkdir");
    let theirs = FileLedger::open(other.join("ledger.ucdk")).expect("open");
    put(&theirs, Change::Put(record("app/run")));
    put(&theirs, Change::Put(record("db/run")));
    drop(theirs);
    fs::copy(other.join("ledger.ucdk"), scratch.file()).expect("copy");
    let snapshot = ledger.read().expect("read");
    assert_eq!(snapshot.token.to_string(), "v2");
    assert_eq!(snapshot.records.len(), 2);
    assert_eq!(
        ledger.write(&token, Change::Put(record("web/run"))),
        Err(LedgerError::Stale)
    );
    let bytes = fs::read(scratch.file()).expect("read");
    fs::create_dir(scratch.dir().join("ledger.ucdk.tmp")).expect("block");
    let failed = ledger.write(&snapshot.token, Change::Put(record("web/run")));
    assert!(matches!(failed, Err(LedgerError::Unavailable(_))));
    assert_eq!(fs::read(scratch.file()).expect("read"), bytes);
    assert_eq!(ledger.read().expect("read").records.len(), 2);
}

// ---- refusals ----------------------------------------------------------------------------

/// A ledger file with three records, as the ledger itself writes it.
fn written(scratch: &Scratch) -> Vec<u8> {
    let ledger = open(scratch);
    put(&ledger, Change::Put(record("app/run")));
    put(&ledger, Change::Put(full_record()));
    put(&ledger, Change::Put(confirmed("db/run", "id-1")));
    drop(ledger);
    fs::read(scratch.file()).expect("read")
}

#[test]
fn a_document_cut_short_at_any_length_is_refused_and_left_alone() {
    let scratch = Scratch::new("truncated");
    let whole = written(&scratch);
    for cut in 0..whole.len() {
        fs::write(scratch.file(), &whole[..cut]).expect("write");
        let err = refused(&scratch);
        assert!(
            matches!(err, OpenError::Corrupt { .. }),
            "cut at {cut}: {err:?}"
        );
        assert_eq!(fs::read(scratch.file()).expect("read"), &whole[..cut]);
    }
    fs::write(scratch.file(), &whole).expect("write");
    assert_eq!(open(&scratch).read().expect("read").records.len(), 3);
}

#[test]
fn a_document_with_any_one_byte_changed_is_refused() {
    let scratch = Scratch::new("flipped");
    let whole = written(&scratch);
    for at in 0..whole.len() {
        let mut changed = whole.clone();
        changed[at] ^= 0x01;
        fs::write(scratch.file(), &changed).expect("write");
        let err = refused(&scratch);
        assert!(
            matches!(err, OpenError::Corrupt { .. }),
            "byte {at}: {err:?}"
        );
    }
}

#[test]
fn a_file_that_is_not_a_ledger_is_refused() {
    let scratch = Scratch::new("not_a_ledger");
    for content in [
        &b""[..],
        b"hello\n",
        b"ucdk-ledger\n",
        b"ucdk-ledger x.y\n",
        b"ucdk-ledger 0.1\n",
        b"\xff\xfe",
    ] {
        fs::write(scratch.file(), content).expect("write");
        let err = refused(&scratch);
        assert!(
            matches!(err, OpenError::Corrupt { .. }),
            "{content:?}: {err:?}"
        );
    }
}

#[test]
fn a_newer_major_version_is_refused_by_name_and_left_alone() {
    let scratch = Scratch::new("newer_major");
    // Whatever else a newer major writes, the first line says so.
    let content = b"ucdk-ledger 2.0\nsomething else entirely\n";
    fs::write(scratch.file(), content).expect("write");
    let err = refused(&scratch);
    assert_eq!(
        err,
        OpenError::NewerMajor {
            path: scratch.file(),
            found: 2
        }
    );
    assert!(err.to_string().contains("newer than this build reads"));
    assert_eq!(fs::read(scratch.file()).expect("read"), content);
}

#[test]
fn a_newer_minor_is_read_ignoring_unknown_tags_and_is_never_rewritten() {
    let scratch = Scratch::new("newer_minor");
    let content = sealed(
        &[
            "ucdk-ledger 1.7",
            "version 4",
            "added-at-top-level whatever",
            "record app/run",
            "kind service",
            "name n",
            "origin created",
            "state confirmed",
            "physical id-1",
            "added-in-a-record 1 2 3",
        ],
        1,
    );
    fs::write(scratch.file(), &content).expect("write");
    let ledger = open(&scratch);
    let snapshot = ledger.read().expect("read");
    assert_eq!(snapshot.token.to_string(), "v4");
    assert_eq!(snapshot.records[&path("app/run")].state, State::Confirmed);
    let failed = ledger.write(&snapshot.token, Change::Put(record("db/run")));
    let Err(LedgerError::Unavailable(why)) = failed else {
        panic!("the write should be refused: {failed:?}");
    };
    assert!(why.contains("1.7") && why.contains("drop"), "{why}");
    assert_eq!(fs::read(scratch.file()).expect("read"), content);
}

#[test]
fn a_document_that_is_whole_but_wrong_is_refused_with_the_line_and_the_reason() {
    let h = "ucdk-ledger 1.0";
    let record_lines = [
        "record a/run",
        "kind service",
        "name n",
        "origin created",
        "state intent",
    ];
    let with = |extra: &[&'static str]| -> (Vec<&'static str>, usize) {
        let mut lines = vec![h, "version 1"];
        lines.extend(record_lines);
        lines.extend(extra);
        (lines, 1)
    };
    let without = |tag: &str| -> (Vec<&'static str>, usize) {
        let mut lines = vec![h, "version 1"];
        lines.extend(record_lines.iter().copied().filter(|l| !l.starts_with(tag)));
        (lines, 1)
    };
    let cases: Vec<((Vec<&str>, usize), &str)> = vec![
        ((vec![h, "version 1"], 1), "the record count"),
        (
            with(&[
                "record a/run",
                "kind service",
                "name n",
                "origin created",
                "state intent",
            ]),
            "given twice",
        ),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v.remove(1);
                    v
                },
                1,
            ),
            "no version",
        ),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v[1] = "version x";
                    v
                },
                1,
            ),
            "not a number",
        ),
        (without("kind"), "no `kind`"),
        (without("name"), "no `name`"),
        (without("origin"), "no `origin`"),
        (without("state"), "no `state`"),
        (with(&["state intent"]), "`state` is given twice"),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v[5] = "origin lent";
                    v
                },
                1,
            ),
            "not an origin",
        ),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v[6] = "state asleep";
                    v
                },
                1,
            ),
            "not a state",
        ),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v[2] = "record Upper";
                    v
                },
                1,
            ),
            "record path",
        ),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v[2] = "record %zz";
                    v
                },
                1,
            ),
            "validly escaped",
        ),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v[3] = "kind %FF";
                    v
                },
                1,
            ),
            "validly escaped",
        ),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v[3] = "kind %4";
                    v
                },
                1,
            ),
            "validly escaped",
        ),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v[3] = "kind";
                    v
                },
                1,
            ),
            "validly escaped",
        ),
        (
            (
                {
                    let mut v = with(&[]).0;
                    v[3] = "kind a b";
                    v
                },
                1,
            ),
            "needs 1 value",
        ),
        (with(&["dep not/ok/../path here"]), "needs 1 value"),
        (with(&["dep Upper"]), "record path"),
        (with(&["output only-a-name"]), "needs 2 value"),
    ];
    let scratch = Scratch::new("wrong");
    for ((lines, count), expected) in cases {
        fs::write(scratch.file(), sealed(&lines, count)).expect("write");
        let OpenError::Corrupt { why, line, .. } = refused(&scratch) else {
            panic!("{expected}: not refused as corrupt");
        };
        assert!(
            why.contains(expected),
            "wanted `{expected}`, got `{why}` (line {line}): {lines:?}"
        );
        assert!(line >= 1);
    }
}

fn flipped_checksum(good: &str) -> String {
    let mut text = good.trim_end().to_owned();
    let last = text.pop().expect("a last digit");
    text.push(if last == '0' { '1' } else { '0' });
    text.push('\n');
    text
}

#[test]
fn a_document_with_a_bad_end_line_is_refused() {
    let scratch = Scratch::new("end_line");
    let good = String::from_utf8(sealed(&["ucdk-ledger 1.0", "version 1"], 0)).expect("text");
    let cases = [
        ("ucdk-ledger 1.0\nversion 1\n".to_owned(), "cut short"),
        (
            "ucdk-ledger 1.0\nversion 1\nfinish 0 0000000000000000\n".to_owned(),
            "cut short",
        ),
        (
            "ucdk-ledger 1.0\nversion 1\nend zero 0000000000000000\n".to_owned(),
            "malformed",
        ),
        (
            "ucdk-ledger 1.0\nversion 1\nend 0 nothex\n".to_owned(),
            "malformed",
        ),
        (
            "ucdk-ledger 1.0\nversion 1\nend 0\n".to_owned(),
            "malformed",
        ),
        (good.replace("end 0", "end 1"), "record count"),
        (flipped_checksum(&good), "checksum"),
    ];
    for (content, expected) in cases {
        fs::write(scratch.file(), &content).expect("write");
        let OpenError::Corrupt { why, .. } = refused(&scratch) else {
            panic!("{expected}: not refused as corrupt");
        };
        assert!(why.contains(expected), "wanted `{expected}`, got `{why}`");
    }
    fs::write(scratch.file(), &good).expect("write");
    assert!(open(&scratch).read().expect("read").records.is_empty());
}

#[test]
fn an_open_that_the_file_system_refuses_says_which_path_and_why() {
    let scratch = Scratch::new("io");
    // The directory does not exist: the lock file cannot be made.
    let missing = scratch.dir().join("nope").join("ledger.ucdk");
    let err = FileLedger::open(&missing).expect_err("refused");
    let OpenError::Io { path, .. } = &err else {
        panic!("not an io refusal: {err:?}");
    };
    assert_eq!(path, &scratch.dir().join("nope").join("ledger.ucdk.lock"));
    assert!(err.to_string().starts_with(&path.display().to_string()));

    // The ledger path is a directory: it cannot be read as a file.
    fs::create_dir(scratch.file()).expect("mkdir");
    assert!(matches!(refused(&scratch), OpenError::Io { .. }));
    fs::remove_dir(scratch.file()).expect("rmdir");

    // The temporary path is a directory: a stale file cannot be removed.
    fs::create_dir(scratch.dir().join("ledger.ucdk.tmp")).expect("mkdir");
    let err = refused(&scratch);
    assert!(matches!(&err, OpenError::Io { path, .. } if path.ends_with("ledger.ucdk.tmp")));
}

#[test]
fn every_refusal_reads_as_a_sentence_naming_the_file() {
    let file = std::path::PathBuf::from("/x/ledger.ucdk");
    let sentences = [
        OpenError::Corrupt {
            path: file.clone(),
            line: 3,
            why: "bad".to_owned(),
        }
        .to_string(),
        OpenError::NewerMajor {
            path: file,
            found: 2,
        }
        .to_string(),
    ];
    assert_eq!(
        sentences,
        [
            "/x/ledger.ucdk: the ledger is corrupt at line 3: bad; it was not changed",
            "/x/ledger.ucdk: the ledger is format version 2, newer than this build reads (version 1); use a newer build",
        ]
    );
}

// ---- the lease in the file ---------------------------------------------------------------------

const TTL: std::time::Duration = std::time::Duration::from_secs(60);

#[test]
fn a_file_written_before_the_lease_existed_is_read_and_its_first_lease_is_fence_one() {
    let scratch = Scratch::new("lease_old_file");
    let content = sealed(
        &[
            "ucdk-ledger 1.0",
            "version 4",
            "record app/run",
            "kind service",
            "name n",
            "origin created",
            "state confirmed",
            "physical id-1",
        ],
        1,
    );
    fs::write(scratch.file(), &content).expect("write");
    let ledger = open(&scratch);
    assert_eq!(ledger.read().expect("read").token.to_string(), "v4");
    let lease = ledger.acquire("alice", TTL).expect("acquire");
    assert_eq!(lease.fence.value(), 1);
    drop(ledger);
    let text = fs::read_to_string(scratch.file()).expect("read");
    assert!(
        text.starts_with("ucdk-ledger 1.1\nversion 4\nfence 1\nlease alice "),
        "{text}"
    );
    assert!(text.contains("record app/run"), "the records are kept");
}

#[test]
fn a_lease_line_that_is_not_a_lease_is_refused_with_its_line() {
    let scratch = Scratch::new("lease_bad_lines");
    let cases = [
        (
            &["ucdk-ledger 1.1", "version 1", "fence many"][..],
            "the fence is not a number",
        ),
        (
            &["ucdk-ledger 1.1", "version 1", "fence"][..],
            "not validly escaped",
        ),
        (
            &[
                "ucdk-ledger 1.1",
                "version 1",
                "fence 1",
                "lease alice soon",
            ][..],
            "the lease expiry is not a number",
        ),
        (
            &["ucdk-ledger 1.1", "version 1", "fence 1", "lease alice"][..],
            "needs 2 value",
        ),
        (
            &["ucdk-ledger 1.1", "version 1", "fence 0", "lease alice 5"][..],
            "the fence is missing or 0",
        ),
        (
            &["ucdk-ledger 1.1", "version 1", "lease alice 5"][..],
            "the fence is missing or 0",
        ),
        (
            &["ucdk-ledger 1.1", "version 1", "fence 1", "fence 2"][..],
            "`fence` is given twice",
        ),
        (
            &["ucdk-ledger 1.1", "version 1", "fence 1", "version 2"][..],
            "`version` is given twice",
        ),
        (
            &[
                "ucdk-ledger 1.1",
                "version 1",
                "fence 1",
                "lease alice 5",
                "lease bob 9",
            ][..],
            "`lease` is given twice",
        ),
    ];
    for (lines, expected) in cases {
        fs::write(scratch.file(), sealed(lines, 0)).expect("write");
        let OpenError::Corrupt { why, line, .. } = refused(&scratch) else {
            panic!("{expected}: not refused as corrupt");
        };
        assert!(why.contains(expected), "wanted `{expected}`, got `{why}`");
        assert_eq!(line, lines.len(), "the line of the bad tag");
    }
}

#[test]
fn a_lease_change_that_cannot_be_written_is_unavailable_and_changes_nothing() {
    let scratch = Scratch::new("lease_gone_dir");
    let dir = scratch.dir().join("sub");
    fs::create_dir(&dir).expect("mkdir");
    let ledger = FileLedger::open(dir.join("ledger.ucdk")).expect("open");
    let lease = ledger.acquire("alice", TTL).expect("acquire");
    fs::remove_dir_all(&dir).expect("remove the directory");

    // The file is gone with its directory, so there is no lease to be refused by, and no place
    // to write one.
    let acquired = ledger.acquire("bob", TTL);
    assert!(
        matches!(acquired, Err(LeaseError::Unavailable(_))),
        "{acquired:?}"
    );
    let renewed = ledger.renew(lease.fence, TTL);
    assert_eq!(renewed, Err(LedgerError::Fenced), "no lease is on file");
    let release = ledger.release(lease.fence);
    assert_eq!(release, Err(LedgerError::Fenced));
    let token = ledger.read().expect("read").token;
    assert!(
        ledger
            .write_fenced(lease.fence, &token, Change::Put(record("a/run")))
            .is_err(),
        "the write itself cannot land without the directory"
    );
    let fresh = FileLedger::open(scratch.dir().join("other.ucdk")).expect("open");
    assert!(matches!(
        fresh.acquire("carol", TTL).map(|l| l.fence.value()),
        Ok(1)
    ));
}

#[test]
fn a_lease_change_to_a_file_with_a_newer_minor_is_refused_like_a_write() {
    let scratch = Scratch::new("lease_newer_minor");
    fs::write(scratch.file(), sealed(&["ucdk-ledger 1.7", "version 4"], 0)).expect("write");
    let ledger = open(&scratch);
    let acquired = ledger.acquire("alice", TTL);
    let Err(LeaseError::Unavailable(why)) = acquired else {
        panic!("not unavailable: {acquired:?}");
    };
    assert!(why.contains("newer build"), "{why}");
    assert_eq!(
        fs::read(scratch.file()).expect("read"),
        sealed(&["ucdk-ledger 1.7", "version 4"], 0),
        "the file is as it was"
    );
}

#[test]
fn a_physical_line_with_no_value_is_refused() {
    let scratch = Scratch::new("physical_empty");
    let lines = [
        "ucdk-ledger 1.0",
        "version 1",
        "record a/run",
        "kind service",
        "name n",
        "origin created",
        "state confirmed",
        "physical",
    ];
    fs::write(scratch.file(), sealed(&lines, 1)).expect("write");
    let OpenError::Corrupt { why, line, .. } = refused(&scratch) else {
        panic!("not refused as corrupt");
    };
    assert_eq!(line, 8, "the line of the `physical` tag");
    assert!(why.contains("not validly escaped"), "{why}");
}

#[test]
fn every_change_over_a_file_that_became_corrupt_after_the_open_is_unavailable() {
    let scratch = Scratch::new("corrupt_after_open");
    let ledger = open(&scratch);
    put(&ledger, Change::Put(record("app/run")));
    let lease = ledger.acquire("alice", TTL).expect("acquire");
    let token = ledger.read().expect("read").token;
    // Another process, or a disk, changed the file under the open handle.
    fs::write(scratch.file(), "this is not a ledger\n").expect("write");
    let unavailable = |why: &str| why.contains("corrupt");
    let write = ledger.write(&token, Change::Put(record("db/run")));
    assert!(
        matches!(write, Err(LedgerError::Unavailable(ref why)) if unavailable(why)),
        "{write:?}"
    );
    let fenced = ledger.write_fenced(lease.fence, &token, Change::Put(record("db/run")));
    assert!(
        matches!(fenced, Err(LedgerError::Unavailable(ref why)) if unavailable(why)),
        "{fenced:?}"
    );
    let renewed = ledger.renew(lease.fence, TTL);
    assert!(
        matches!(renewed, Err(LedgerError::Unavailable(ref why)) if unavailable(why)),
        "{renewed:?}"
    );
    let released = ledger.release(lease.fence);
    assert!(
        matches!(released, Err(LedgerError::Unavailable(ref why)) if unavailable(why)),
        "{released:?}"
    );
    let acquired = ledger.acquire("bob", TTL);
    assert!(
        matches!(acquired, Err(LeaseError::Unavailable(ref why)) if unavailable(why)),
        "{acquired:?}"
    );
    let read = ledger.read();
    assert!(
        matches!(read, Err(LedgerError::Unavailable(ref why)) if unavailable(why)),
        "{read:?}"
    );
    assert_eq!(
        fs::read_to_string(scratch.file()).expect("read"),
        "this is not a ledger\n",
        "nothing was repaired"
    );
}

// ---- opened to read ----------------------------------------------------------------------

/// Every file of the directory with its bytes, by name.
fn listing(scratch: &Scratch) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(scratch.dir())
        .expect("list")
        .map(|entry| {
            let entry = entry.expect("entry");
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).expect("read"),
            )
        })
        .collect()
}

/// A scratch directory holding a ledger of two records and a lease, and nothing else: the lock
/// file the writer made is removed, so any file that appears later is the reader's.
fn written_ledger(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    {
        let ledger = open(&scratch);
        put(&ledger, Change::Put(record("app/run")));
        put(&ledger, Change::Put(confirmed("db/run", "id-1")));
        ledger.acquire("alice", TTL).expect("acquire");
    }
    fs::remove_file(scratch.dir().join("ledger.ucdk.lock")).expect("remove the lock file");
    scratch
}

#[test]
fn a_read_only_open_changes_no_file_and_creates_none() {
    let scratch = written_ledger("ro_unchanged");
    // A write that never finished: a writer's open would remove it.
    fs::write(scratch.dir().join("ledger.ucdk.tmp"), b"half a write").expect("write");
    let before = listing(&scratch);
    assert_eq!(before.len(), 2, "the ledger and the stale temporary");
    {
        let ledger = FileLedger::open_read_only(scratch.file()).expect("open read-only");
        let snapshot = ledger.read().expect("read");
        assert_eq!(snapshot.records.len(), 2);
        assert_eq!(snapshot.lease.expect("a lease").holder, "alice");
    }
    assert_eq!(
        listing(&scratch),
        before,
        "names and bytes are as they were"
    );
}

#[test]
fn a_read_only_open_reads_a_document_another_handle_holds_locked() {
    let scratch = Scratch::new("ro_locked");
    let writer = open(&scratch);
    put(&writer, Change::Put(record("app/run")));
    // The file lock of a writer's read-modify-write, taken here and held while the reader reads.
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(scratch.dir().join("ledger.ucdk.lock"))
        .expect("the lock file");
    lock.lock().expect("take the lock");
    let reader = FileLedger::open_read_only(scratch.file()).expect("open read-only");
    assert_eq!(reader.read().expect("read").records.len(), 1);
    lock.unlock().expect("give the lock back");
}

#[test]
fn a_read_only_ledger_refuses_every_write() {
    let scratch = written_ledger("ro_refuses");
    let reader = FileLedger::open_read_only(scratch.file()).expect("open read-only");
    let snapshot = reader.read().expect("read");
    let fence = snapshot.fence;
    let before = listing(&scratch);
    assert_eq!(
        reader.write(&snapshot.token, Change::Put(record("x/run"))),
        Err(LedgerError::ReadOnly)
    );
    assert_eq!(
        reader.write_fenced(fence, &snapshot.token, Change::Put(record("x/run"))),
        Err(LedgerError::ReadOnly)
    );
    assert_eq!(reader.renew(fence, TTL), Err(LedgerError::ReadOnly));
    assert_eq!(reader.release(fence), Err(LedgerError::ReadOnly));
    assert_eq!(reader.acquire("bob", TTL), Err(LeaseError::ReadOnly));
    assert_eq!(listing(&scratch), before, "no refusal changed a file");
}

#[test]
fn a_read_only_ledger_reports_the_clock_it_was_given() {
    let scratch = Scratch::new("ro_clock");
    {
        let clock = ManualClock::at(5_000);
        let ledger = FileLedger::open_with_clock(scratch.file(), Arc::new(clock)).expect("open");
        ledger
            .acquire("alice", Duration::from_secs(60))
            .expect("acquire");
    }
    let clock = ManualClock::at(7_000);
    let reader = FileLedger::open_read_only_with_clock(scratch.file(), Arc::new(clock.clone()))
        .expect("open read-only");
    let first = reader.read().expect("read");
    assert_eq!(first.at, 7_000);
    let holding = first.lease.expect("a lease");
    assert_eq!(holding.expires_at, 65_000);
    assert_eq!(holding.state(first.at), LeaseState::Live);
    clock.advance(Duration::from_secs(60));
    let later = reader.read().expect("read");
    assert_eq!(
        later.at, 67_000,
        "the reading follows the clock it was given"
    );
    assert_eq!(holding.state(later.at), LeaseState::Lapsing);
}

#[test]
fn a_read_only_open_of_a_missing_file_is_an_error_not_an_empty_ledger() {
    let scratch = Scratch::new("ro_missing");
    let err = FileLedger::open_read_only(scratch.file()).expect_err("refused");
    let OpenError::Io { path, why } = &err else {
        panic!("not an I/O error: {err:?}");
    };
    assert_eq!(path, &scratch.file());
    assert!(why.contains("does not exist"), "{why}");
    assert!(listing(&scratch).is_empty(), "nothing was created");

    // A file that is not a whole ledger, or is of a newer major, is refused as `open` refuses
    // it, and is left as it is.
    for (content, corrupt) in [
        (b"not a ledger".to_vec(), true),
        (sealed(&["ucdk-ledger 2.0", "version 1"], 0), false),
    ] {
        fs::write(scratch.file(), &content).expect("write");
        let err = FileLedger::open_read_only(scratch.file()).expect_err("refused");
        match err {
            OpenError::Corrupt { .. } => assert!(corrupt),
            OpenError::NewerMajor { found, .. } => assert!(!corrupt && found == 2),
            other => panic!("{other:?}"),
        }
        assert_eq!(fs::read(scratch.file()).expect("read"), content);
    }
    assert_eq!(listing(&scratch).len(), 1, "still only the ledger file");
}

// ---- names that collide with another ledger's siblings ------------------------------------

#[test]
fn a_ledger_named_like_another_ledgers_temp_or_lock_file_is_refused() {
    for suffix in [".tmp", ".lock"] {
        let scratch = Scratch::new(&format!("reserved{}", suffix.replace('.', "_")));
        let name = scratch.dir().join(format!("x{suffix}"));
        let err = FileLedger::open(&name).expect_err("a reserved name is refused");
        assert_eq!(err, OpenError::ReservedName { path: name.clone() });
        let text = err.to_string();
        assert!(text.starts_with(&name.display().to_string()), "{text}");
        assert!(text.contains(".tmp") && text.contains(".lock"), "{text}");
        assert!(
            listing(&scratch).is_empty(),
            "nothing was created for {suffix}"
        );
        // A name that merely contains the suffix is an ordinary ledger.
        let fine = scratch.dir().join(format!("x{suffix}.ucdk"));
        put(
            &FileLedger::open(&fine).expect("open"),
            Change::Put(record("app/run")),
        );
    }
}

#[test]
fn opening_a_ledger_never_touches_another_ledgers_file() {
    let scratch = Scratch::new("siblings");
    let x = scratch.dir().join("x");
    let at_tmp = scratch.dir().join("x.tmp");
    put(
        &FileLedger::open(&x).expect("open x"),
        Change::Put(record("app/run")),
    );
    // A ledger that the file system lets exist at the temporary name of "x" is deleted by
    // the next open of "x". With the reserved names refused there is none to lose.
    if let Ok(other) = FileLedger::open(&at_tmp) {
        put(&other, Change::Put(record("other/run")));
        drop(FileLedger::open(&x).expect("reopen x"));
        assert!(at_tmp.exists(), "opening x deleted the ledger at x.tmp");
    }
    let reopened = FileLedger::open(&x).expect("reopen x");
    assert_eq!(reopened.read().expect("read").records.len(), 1);
}
