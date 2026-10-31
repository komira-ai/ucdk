// SPDX-License-Identifier: Apache-2.0
//! The real reader of local files, held to the `OutputFiles` kit over a temporary directory
//! (each scenario as a test of its name), and to the promises of its own that the kit does not
//! make: what a node that is neither a file, a directory nor a symlink is, and what each
//! failure of the disk is.

mod support;

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::net::UnixListener;
use std::path::Path;

use support::run_scenario;
use tempfile::TempDir;
use ucdk_release_build::LocalFiles;
use ucdk_release_seams::kit::{Node, OutputFilesRig, output_files_scenarios, run_output_files_kit};
use ucdk_release_seams::{FileKind, OutputFiles, SeamError};

struct Rig {
    _dir: TempDir,
    root: String,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("a directory");
        let root = fs::canonicalize(dir.path())
            .expect("its path")
            .join("tree")
            .to_string_lossy()
            .into_owned();
        fs::create_dir_all(&root).expect("the root");
        Self { _dir: dir, root }
    }

    fn at(&self, rel: &str) -> String {
        format!("{}/{rel}", self.root)
    }
}

/// Every node below `dir`, as the rig sees the tree without the reader.
fn walk(dir: &Path, into: &mut Vec<(String, FileKind, Vec<u8>)>) {
    let mut names: Vec<_> = fs::read_dir(dir)
        .expect("a directory")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    names.sort();
    for path in names {
        let meta = fs::symlink_metadata(&path).expect("metadata");
        let name = path.to_string_lossy().into_owned();
        if meta.file_type().is_symlink() {
            into.push((name, FileKind::Symlink, Vec::new()));
        } else if meta.is_dir() {
            into.push((name, FileKind::Directory, Vec::new()));
            walk(&path, into);
        } else {
            into.push((name, FileKind::File, fs::read(&path).expect("bytes")));
        }
    }
}

impl OutputFilesRig for Rig {
    type Files = LocalFiles;

    fn root(&self) -> String {
        self.root.clone()
    }

    fn reader(&self, tree: &[(&str, Node<'_>)]) -> LocalFiles {
        fs::remove_dir_all(&self.root).expect("the old tree goes");
        fs::create_dir_all(&self.root).expect("the root");
        for (rel, node) in tree {
            let path = self.at(rel);
            fs::create_dir_all(Path::new(&path).parent().expect("a parent")).expect("parents");
            match node {
                Node::File(bytes) => fs::write(&path, bytes).expect("a file"),
                Node::Directory => fs::create_dir_all(&path).expect("a directory"),
                Node::Link(target) => symlink(self.at(target), &path).expect("a link"),
            }
        }
        LocalFiles
    }

    fn change(&self, _files: &mut LocalFiles, rel: &str, bytes: &[u8]) {
        fs::write(self.at(rel), bytes).expect("new bytes");
    }

    fn snapshot(&self, _files: &LocalFiles) -> Vec<(String, FileKind, Vec<u8>)> {
        let mut nodes = Vec::new();
        walk(Path::new(&self.root), &mut nodes);
        nodes
    }
}

macro_rules! kit_tests {
    ($($name:ident),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                run_scenario(&output_files_scenarios(), stringify!($name), &Rig::new());
            }
        )+

        #[test]
        fn every_scenario_of_the_kit_has_its_test() {
            let named: Vec<&str> = vec![$(stringify!($name)),+];
            let kit: Vec<&str> = output_files_scenarios::<Rig>().iter().map(|s| s.name).collect();
            assert_eq!(named, kit);
        }
    };
}

kit_tests!(
    a_listing_is_sorted_relative_paths,
    a_listing_names_the_kind_of_every_entry,
    a_listed_symlink_is_never_descended,
    stat_tells_a_file_a_directory_a_symlink_and_nothing_apart,
    a_missing_path_is_none_from_stat_and_permanent_from_list_and_read,
    listing_a_file_and_reading_a_directory_or_a_symlink_are_refused,
    a_single_file_output_is_read_without_a_rel,
    a_report_file_is_read_directly,
    reading_a_directory_is_refused_even_when_it_is_the_located_output,
    a_path_that_is_relative_or_climbs_is_refused,
    an_entry_of_a_listed_directory_is_read_through_its_joined_path,
    a_read_returns_the_file_as_it_is_at_the_call,
    no_call_changes_the_tree,
    a_dot_part_a_repeated_slash_and_a_trailing_slash_name_the_path_they_stand_in,
);

#[test]
fn the_whole_kit_passes_on_the_disk() {
    run_output_files_kit(&Rig::new()).assert_conforms();
}

/// Restores the permissions of a path when it goes out of scope, so a failed assertion does not
/// leave a directory that the cleanup of the temporary tree cannot enter.
struct Restore<'a>(&'a str);

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
    }
}

/// A test of a failure of the disk needs permission checks that bind the test, which they do
/// not for the superuser.
fn not_the_superuser() {
    assert!(
        !rustix::process::geteuid().is_root(),
        "these tests need permission checks that bind the user running them"
    );
}

#[test]
fn a_node_that_is_neither_a_file_a_directory_nor_a_symlink_is_permanent() {
    let rig = Rig::new();
    let socket = rig.at("d/socket");
    fs::create_dir_all(rig.at("d")).unwrap();
    let _listener = UnixListener::bind(&socket).expect("a socket");
    let mut files = LocalFiles;
    for result in [
        files.stat(&socket).map(|_| ()),
        files.read(&socket).map(|_| ()),
        files.list(&rig.at("d")).map(|_| ()),
    ] {
        assert!(
            matches!(&result, Err(SeamError::Permanent(why)) if why.contains("not a file")),
            "{result:?}"
        );
    }
}

#[test]
fn a_name_that_is_not_text_makes_the_listing_permanent_and_not_shorter() {
    let rig = Rig::new();
    fs::create_dir_all(rig.at("d")).unwrap();
    fs::write(rig.at("d/ok"), b"x").unwrap();
    let name = std::ffi::OsStr::from_bytes(b"bad-\xff-name");
    fs::write(Path::new(&rig.at("d")).join(name), b"y").unwrap();
    let result = LocalFiles.list(&rig.at("d"));
    assert!(
        matches!(&result, Err(SeamError::Permanent(why)) if why.contains("not text")),
        "{result:?}"
    );
}

#[test]
fn a_directory_that_cannot_be_entered_is_unknown_and_never_missing() {
    not_the_superuser();
    let rig = Rig::new();
    fs::create_dir_all(rig.at("closed")).unwrap();
    fs::write(rig.at("closed/x"), b"x").unwrap();
    let closed = rig.at("closed");
    let _restore = Restore(&closed);
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
    let mut files = LocalFiles;
    // What is in it cannot be asked about, and that is not the same as nothing being there.
    for result in [
        files.stat(&rig.at("closed/x")).map(|_| ()),
        files.read(&rig.at("closed/x")).map(|_| ()),
        files.list(&closed).map(|_| ()),
    ] {
        assert!(matches!(result, Err(SeamError::Unknown(_))), "{result:?}");
    }
    // The directory itself can be examined.
    assert_eq!(files.stat(&closed), Ok(Some(FileKind::Directory)));
}

#[test]
fn a_file_that_cannot_be_read_is_unknown_and_a_subdirectory_that_cannot_be_listed_fails_the_listing()
 {
    not_the_superuser();
    let rig = Rig::new();
    fs::create_dir_all(rig.at("d/sub")).unwrap();
    fs::write(rig.at("d/locked"), b"x").unwrap();
    let (locked, sub) = (rig.at("d/locked"), rig.at("d/sub"));
    let _restore = (Restore(&locked), Restore(&sub));
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o000)).unwrap();
    let mut files = LocalFiles;
    assert_eq!(files.stat(&locked), Ok(Some(FileKind::File)));
    assert!(matches!(files.read(&locked), Err(SeamError::Unknown(_))));
    // A listing that could not read a directory below the one listed is no listing.
    assert!(matches!(
        files.list(&rig.at("d")),
        Err(SeamError::Unknown(_))
    ));
}

#[test]
fn a_path_below_a_file_is_nothing_there_and_a_link_loop_is_not() {
    let rig = Rig::new();
    fs::write(rig.at("file"), b"x").unwrap();
    symlink(rig.at("loop"), rig.at("loop")).unwrap();
    let mut files = LocalFiles;
    // The operating system says "not a directory", which is an answer about the path.
    assert_eq!(files.stat(&rig.at("file/below")), Ok(None));
    assert_eq!(files.stat(&rig.at("loop")), Ok(Some(FileKind::Symlink)));
    // Below a link that leads nowhere but back to itself the disk cannot be asked.
    assert!(matches!(
        files.stat(&rig.at("loop/below")),
        Err(SeamError::Unknown(_))
    ));
}
