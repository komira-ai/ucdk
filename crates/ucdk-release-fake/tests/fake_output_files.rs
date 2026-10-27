// SPDX-License-Identifier: Apache-2.0
//! The fake output files: a tree built by a test, read through the seam, changed by nothing.

use ucdk_release_fake::FakeOutputFiles;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{DirEntry, FileKind, OutputFiles, SeamError};

fn entry(rel: &str, kind: FileKind) -> DirEntry {
    DirEntry {
        rel: rel.to_owned(),
        kind,
    }
}

/// A directory with nested files, a symlink to a directory with something below it in the flat
/// tree, and a sibling whose name starts like the symlink's.
fn tree() -> FakeOutputFiles {
    let mut files = FakeOutputFiles::new();
    files.file("/out/pkg/b.txt", b"b");
    files.file("/out/pkg/a/z.txt", b"z");
    files.file("/out/pkg/a.json", b"{}");
    files.symlink("/out/pkg/link");
    files.file("/out/pkg/link/hidden", b"never listed");
    files.file("/out/pkg/linked", b"a sibling, not below the link");
    files
}

#[test]
fn a_listing_is_sorted_and_reports_a_symlink_without_following_it() {
    let mut files = tree();
    assert_eq!(
        files.list("/out/pkg").unwrap(),
        [
            entry("a", FileKind::Directory),
            entry("a.json", FileKind::File),
            entry("a/z.txt", FileKind::File),
            entry("b.txt", FileKind::File),
            entry("link", FileKind::Symlink),
            entry("linked", FileKind::File),
        ]
    );
}

#[test]
fn stat_tells_a_file_a_directory_a_symlink_and_nothing_apart() {
    let mut files = tree();
    files.symlink_at("/out/pkg/b.txt");
    assert_eq!(files.stat("/out/pkg/a.json"), Ok(Some(FileKind::File)));
    assert_eq!(files.stat("/out/pkg"), Ok(Some(FileKind::Directory)));
    assert_eq!(files.stat("/out/pkg/b.txt"), Ok(Some(FileKind::Symlink)));
    assert_eq!(files.stat("/out/pkg/nothing"), Ok(None));
}

#[test]
fn a_file_changed_after_listing_is_read_as_it_is_now() {
    let mut files = tree();
    files.changes_after_listing("/out/pkg/a.json", b"changed");
    assert_eq!(files.read("/out/pkg/a.json"), Ok(b"{}".to_vec()));
    // A listing of a directory elsewhere does not change it.
    files.dir("/elsewhere");
    files.list("/elsewhere").unwrap();
    assert_eq!(files.read("/out/pkg/a.json"), Ok(b"{}".to_vec()));
    files.list("/out/pkg").unwrap();
    assert_eq!(files.read("/out/pkg/a.json"), Ok(b"changed".to_vec()));
    // It changes once: a second listing leaves the new bytes.
    files.list("/out/pkg").unwrap();
    assert_eq!(files.read("/out/pkg/a.json"), Ok(b"changed".to_vec()));
}

#[test]
fn a_missing_path_and_an_unreadable_file_are_named_errors() {
    let mut files = tree();
    files.missing("/out/pkg/a.json");
    files.missing("/out/pkg");
    files.unreadable("/out/pkg/b.txt");
    assert_eq!(files.stat("/out/pkg/a.json"), Ok(None));
    for result in [files.read("/out/pkg/a.json"), files.read("/never/there")] {
        assert!(matches!(&result, Err(SeamError::Permanent(why)) if why.contains('/')));
    }
    assert!(matches!(
        files.list("/out/pkg"),
        Err(SeamError::Permanent(_))
    ));
    assert!(matches!(
        files.stat("/out/pkg/b.txt"),
        Err(SeamError::Unknown(_))
    ));
    assert!(matches!(
        files.read("/out/pkg/b.txt"),
        Err(SeamError::Unknown(_))
    ));
    assert!(matches!(
        files.list("/out/pkg/b.txt"),
        Err(SeamError::Unknown(_))
    ));
}

#[test]
fn a_missing_path_is_not_listed_by_its_parent_nor_anything_below_it() {
    let mut files = tree();
    files.missing("/out/pkg/a.json");
    files.missing("/out/pkg/a");
    let listed = files.list("/out/pkg").unwrap();
    // What stat says is absent is not listed, and neither is anything under an absent directory.
    assert_eq!(
        listed,
        [
            entry("b.txt", FileKind::File),
            entry("link", FileKind::Symlink),
            entry("linked", FileKind::File),
        ]
    );
    for entry in &listed {
        let path = format!("/out/pkg/{}", entry.rel);
        assert_eq!(files.stat(&path), Ok(Some(entry.kind)), "{path}");
    }
    for gone in ["/out/pkg/a.json", "/out/pkg/a", "/out/pkg/a/z.txt"] {
        assert_eq!(files.stat(gone), Ok(None), "{gone}");
    }
}

#[test]
fn an_unreadable_path_is_still_listed_by_its_parent() {
    // The design makes `stat`, `list` and `read` of the path itself fail with `Unknown`. The
    // listing of its parent does not read the entry, so it still names it, as a directory does.
    let mut files = tree();
    files.unreadable("/out/pkg/b.txt");
    files.unreadable("/out/pkg/a");
    let listed = files.list("/out/pkg").unwrap();
    assert!(listed.contains(&entry("b.txt", FileKind::File)));
    assert!(listed.contains(&entry("a", FileKind::Directory)));
    assert!(matches!(
        files.stat("/out/pkg/b.txt"),
        Err(SeamError::Unknown(_))
    ));
    assert!(matches!(
        files.list("/out/pkg/a"),
        Err(SeamError::Unknown(_))
    ));
}

#[test]
fn the_tree_it_was_given_is_equal_after_every_call() {
    let mut files = tree();
    let before = files.snapshot();
    files.stat("/out/pkg").unwrap();
    files.list("/out/pkg").unwrap();
    files.read("/out/pkg/b.txt").unwrap();
    assert!(files.list("/out/pkg/b.txt").is_err());
    assert!(files.read("/out/pkg/a").is_err());
    assert_eq!(files.snapshot(), before);
    assert_eq!(files.calls().len(), 5);
    assert_eq!(files.calls()[0], "stat /out/pkg");
}

#[test]
fn a_directory_a_symlink_and_a_file_are_only_read_as_a_file() {
    let mut files = tree();
    for path in ["/out/pkg", "/out/pkg/link", "/out/pkg/nothing"] {
        assert!(
            matches!(files.read(path), Err(SeamError::Permanent(_))),
            "{path}"
        );
    }
    for path in ["/out/pkg/a.json", "/out/pkg/link", "/out/pkg/nothing"] {
        assert!(
            matches!(files.list(path), Err(SeamError::Permanent(_))),
            "{path}"
        );
    }
}

#[test]
fn a_path_that_is_relative_or_climbs_is_refused() {
    let mut files = tree();
    let refused = Err(SeamError::Refused(Reason::BuildOutput));
    for path in ["out/pkg", "/out/../etc", "", "./x"] {
        assert_eq!(files.stat(path), refused.clone().map(|()| None));
        assert_eq!(files.list(path), refused.clone().map(|()| Vec::new()));
        assert_eq!(files.read(path), refused.clone().map(|()| Vec::new()));
    }
}
