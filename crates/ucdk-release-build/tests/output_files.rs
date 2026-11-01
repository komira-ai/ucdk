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
use ucdk_release_proto::v1::Reason;
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

    /// A reader confined to the root of the rig.
    fn files(&self) -> LocalFiles {
        LocalFiles::within([&self.root]).expect("the root resolves")
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
        self.files()
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
    let mut files = rig.files();
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
    let result = rig.files().list(&rig.at("d"));
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
    let mut files = rig.files();
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
    let mut files = rig.files();
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
    let mut files = rig.files();
    // The operating system says "not a directory", which is an answer about the path.
    assert_eq!(files.stat(&rig.at("file/below")), Ok(None));
    assert_eq!(files.stat(&rig.at("loop")), Ok(Some(FileKind::Symlink)));
    // Below a link, whether it leads anywhere or only back to itself, nothing is looked up.
    assert_eq!(
        files.stat(&rig.at("loop/below")),
        Err(SeamError::Refused(Reason::BuildOutput))
    );
}

const OUTSIDE: Result<(), SeamError> = Err(SeamError::Refused(Reason::BuildOutput));

/// What `stat`, `list` and `read` of `path` answer, each reduced to whether it is the refusal
/// of a path outside the output tree.
fn refused_by_all(files: &mut LocalFiles, path: &str) -> [bool; 3] {
    [
        files.stat(path).map(|_| ()) == OUTSIDE,
        files.list(path).map(|_| ()) == OUTSIDE,
        files.read(path).map(|_| ()) == OUTSIDE,
    ]
}

/// A tree with the root `tree/out` and, beside it, `tree/outside/x` and `tree/out-other/x`.
fn with_neighbours(rig: &Rig) -> (String, String) {
    let outside = format!("{}/outside", rig.root);
    let near = format!("{}/out-other", rig.root);
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(&near).unwrap();
    fs::write(format!("{outside}/x"), b"secret").unwrap();
    fs::write(format!("{near}/x"), b"near").unwrap();
    fs::create_dir_all(rig.at("out")).unwrap();
    (outside, near)
}

#[test]
fn an_absolute_path_outside_the_root_is_refused_by_every_method() {
    let rig = Rig::new();
    let (outside, near) = with_neighbours(&rig);
    let mut files = LocalFiles::within([rig.at("out")]).unwrap();
    for path in [
        format!("{outside}/x"),
        outside,
        // The name of the root with more after it is another directory, not a part of the root.
        format!("{near}/x"),
        "/etc/passwd".to_owned(),
        "/".to_owned(),
    ] {
        assert_eq!(refused_by_all(&mut files, &path), [true; 3], "{path}");
    }
}

#[test]
fn the_root_itself_and_a_nested_output_below_it_are_read() {
    let rig = Rig::new();
    fs::create_dir_all(rig.at("out/a/b")).unwrap();
    fs::write(rig.at("out/a/b/x"), b"built").unwrap();
    let mut files = LocalFiles::within([rig.at("out")]).unwrap();
    assert_eq!(files.stat(&rig.at("out")), Ok(Some(FileKind::Directory)));
    assert_eq!(files.stat(&rig.at("out/a/b/x")), Ok(Some(FileKind::File)));
    assert_eq!(files.read(&rig.at("out/a/b/x")), Ok(b"built".to_vec()));
    let listed = files.list(&rig.at("out/a")).unwrap();
    assert_eq!(
        listed.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(),
        ["b", "b/x"]
    );
    // A path with a slash too many names the same place.
    assert_eq!(files.read(&rig.at("out//a/./b/x")), Ok(b"built".to_vec()));
}

#[test]
fn a_symlink_between_the_root_and_the_path_that_leads_out_of_the_root_is_refused() {
    let rig = Rig::new();
    let (outside, _) = with_neighbours(&rig);
    symlink(&outside, rig.at("out/link")).unwrap();
    let mut files = LocalFiles::within([rig.at("out")]).unwrap();
    assert_eq!(refused_by_all(&mut files, &rig.at("out/link/x")), [true; 3]);
    // Deeper than one step, and with the link in the middle of the path.
    fs::create_dir_all(rig.at("out/a")).unwrap();
    symlink(&outside, rig.at("out/a/link")).unwrap();
    fs::create_dir_all(format!("{outside}/d")).unwrap();
    fs::write(format!("{outside}/d/y"), b"secret").unwrap();
    assert_eq!(
        refused_by_all(&mut files, &rig.at("out/a/link/d/y")),
        [true; 3]
    );
    // The link itself is still a link that `stat` reports, as the contract says.
    assert_eq!(files.stat(&rig.at("out/link")), Ok(Some(FileKind::Symlink)));
}

#[test]
fn a_symlink_between_the_root_and_the_path_that_stays_inside_the_root_is_refused_too() {
    // The design says a symlink anywhere in an output is a defect of the output, so a path
    // is never resolved through one, whatever it leads to.
    let rig = Rig::new();
    fs::create_dir_all(rig.at("out/real")).unwrap();
    fs::write(rig.at("out/real/x"), b"built").unwrap();
    symlink(rig.at("out/real"), rig.at("out/alias")).unwrap();
    let mut files = LocalFiles::within([rig.at("out")]).unwrap();
    assert_eq!(
        refused_by_all(&mut files, &rig.at("out/alias/x")),
        [true; 3]
    );
    assert_eq!(files.read(&rig.at("out/real/x")), Ok(b"built".to_vec()));
}

#[test]
fn nothing_below_a_directory_that_is_not_there_is_nothing_and_not_a_refusal() {
    let rig = Rig::new();
    fs::create_dir_all(rig.at("out")).unwrap();
    let mut files = LocalFiles::within([rig.at("out")]).unwrap();
    assert_eq!(files.stat(&rig.at("out/none/deeper/x")), Ok(None));
    assert!(matches!(
        files.read(&rig.at("out/none/deeper/x")),
        Err(SeamError::Permanent(_))
    ));
}

#[test]
fn a_directory_on_the_way_that_cannot_be_examined_is_unknown() {
    not_the_superuser();
    let rig = Rig::new();
    fs::create_dir_all(rig.at("out/closed/inner")).unwrap();
    let closed = rig.at("out/closed");
    let _restore = Restore(&closed);
    fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
    let mut files = LocalFiles::within([rig.at("out")]).unwrap();
    let result = files.stat(&rig.at("out/closed/inner/x"));
    assert!(matches!(result, Err(SeamError::Unknown(_))), "{result:?}");
}

#[test]
fn more_than_one_root_may_be_named_and_each_confines_its_own_paths() {
    let rig = Rig::new();
    fs::create_dir_all(rig.at("one")).unwrap();
    fs::create_dir_all(rig.at("two")).unwrap();
    fs::write(rig.at("one/x"), b"1").unwrap();
    fs::write(rig.at("two/x"), b"2").unwrap();
    fs::write(rig.at("three"), b"3").unwrap();
    let mut files = LocalFiles::within([rig.at("one"), rig.at("two")]).unwrap();
    assert_eq!(files.read(&rig.at("one/x")), Ok(b"1".to_vec()));
    assert_eq!(files.read(&rig.at("two/x")), Ok(b"2".to_vec()));
    assert_eq!(refused_by_all(&mut files, &rig.at("three")), [true; 3]);
}

#[test]
fn a_root_reached_through_a_link_confines_paths_named_by_either_spelling() {
    // A build system may report the root as the project is reached, through a link, or as it
    // is on the disk; the root is the directory either way, and a link below it is not followed.
    let rig = Rig::new();
    fs::create_dir_all(rig.at("real/sub")).unwrap();
    fs::write(rig.at("real/sub/x"), b"built").unwrap();
    symlink(rig.at("real"), rig.at("front")).unwrap();
    symlink(rig.at("real/sub"), rig.at("real/via")).unwrap();
    let mut files = LocalFiles::within([rig.at("front")]).unwrap();
    assert_eq!(files.read(&rig.at("front/sub/x")), Ok(b"built".to_vec()));
    assert_eq!(files.read(&rig.at("real/sub/x")), Ok(b"built".to_vec()));
    assert_eq!(
        refused_by_all(&mut files, &rig.at("front/via/x")),
        [true; 3]
    );
    assert_eq!(refused_by_all(&mut files, &rig.at("real/via/x")), [true; 3]);
}

#[test]
fn a_root_given_with_a_trailing_slash_is_the_same_root() {
    let rig = Rig::new();
    let (outside, near) = with_neighbours(&rig);
    fs::write(rig.at("out/x"), b"built").unwrap();
    let mut files = LocalFiles::within([format!("{}/", rig.at("out"))]).unwrap();
    assert_eq!(files.read(&rig.at("out/x")), Ok(b"built".to_vec()));
    assert_eq!(files.stat(&rig.at("out")), Ok(Some(FileKind::Directory)));
    for path in [format!("{outside}/x"), format!("{near}/x")] {
        assert_eq!(refused_by_all(&mut files, &path), [true; 3], "{path}");
    }
}

#[test]
fn a_relative_root_is_resolved_against_the_current_directory_and_confines_absolute_paths() {
    let rig = Rig::new();
    let (outside, _) = with_neighbours(&rig);
    fs::write(rig.at("out/x"), b"built").unwrap();
    // The root spelled relative to the current directory by climbing out of it, so that the
    // test does not change the directory of the process.
    let here = std::env::current_dir().unwrap();
    let up = here.components().count() - 1;
    let target = rig.at("out");
    let relative = format!("{}{}", "../".repeat(up), target.trim_start_matches('/'));
    assert!(Path::new(&relative).is_relative());
    let mut files = LocalFiles::within([relative]).unwrap();
    assert_eq!(files.read(&rig.at("out/x")), Ok(b"built".to_vec()));
    assert_eq!(files.stat(&rig.at("out")), Ok(Some(FileKind::Directory)));
    assert_eq!(
        refused_by_all(&mut files, &format!("{outside}/x")),
        [true; 3]
    );
}

#[test]
fn a_root_that_cannot_be_resolved_is_permanent() {
    let rig = Rig::new();
    let result = LocalFiles::within([rig.at("not-there")]);
    assert!(
        matches!(&result, Err(SeamError::Permanent(why)) if why.contains("not-there")),
        "{result:?}"
    );
}
