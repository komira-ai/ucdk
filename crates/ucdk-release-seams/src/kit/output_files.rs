// SPDX-License-Identifier: Apache-2.0
//! The `OutputFiles` kit: thirteen scenarios over the three questions a build system asks of a
//! tree it did not make (what is at a path, what is below a directory, what a file holds).

use super::{Report, Scenario, must, must_err, permanent_naming, run_scenarios, same};
use crate::{FileKind, OutputFiles, SeamError, entry_path};

/// One node of a tree a scenario asks its rig to build.
#[derive(Debug, Clone, Copy)]
pub enum Node<'a> {
    /// A regular file with these bytes.
    File(&'a [u8]),
    Directory,
    /// A symlink to the path below the rig's root that is named, which may hold a file, a
    /// directory or nothing.
    Link(&'a str),
}

/// What an implementation provides so the scenarios can set up a tree and look at it without
/// the reader's help.
pub trait OutputFilesRig {
    type Files: OutputFiles;

    /// The absolute path of the directory every tree is built below, with no trailing `/`.
    fn root(&self) -> String;

    /// A reader over a tree of exactly `tree`, each path relative to [`Self::root`] with its
    /// parent directories made, replacing any tree an earlier call made.
    fn reader(&self, tree: &[(&str, Node<'_>)]) -> Self::Files;

    /// Makes the regular file at `rel` below the root hold `bytes` from now on, behind the
    /// reader's back.
    fn change(&self, files: &mut Self::Files, rel: &str, bytes: &[u8]);

    /// The path, kind and byte length of everything in the tree, taken without the reader.
    fn snapshot(&self, files: &Self::Files) -> Vec<(String, FileKind, usize)>;
}

fn at<R: OutputFilesRig>(rig: &R, rel: &str) -> String {
    format!("{}/{rel}", rig.root())
}

fn is_refused(error: &SeamError) -> bool {
    matches!(error, SeamError::Refused(_))
}

pub fn run_output_files_kit<R: OutputFilesRig>(rig: &R) -> Report {
    run_scenarios(rig, &output_files_scenarios())
}

pub fn output_files_scenarios<R: OutputFilesRig>() -> Vec<Scenario<R>> {
    vec![
        Scenario::new(
            "a_listing_is_sorted_relative_paths",
            a_listing_is_sorted_relative_paths::<R>,
        ),
        Scenario::new(
            "a_listing_names_the_kind_of_every_entry",
            a_listing_names_the_kind_of_every_entry::<R>,
        ),
        Scenario::new(
            "a_listed_symlink_is_never_descended",
            a_listed_symlink_is_never_descended::<R>,
        ),
        Scenario::new(
            "stat_tells_a_file_a_directory_a_symlink_and_nothing_apart",
            stat_tells_a_file_a_directory_a_symlink_and_nothing_apart::<R>,
        ),
        Scenario::new(
            "a_missing_path_is_none_from_stat_and_permanent_from_list_and_read",
            a_missing_path_is_none_from_stat_and_permanent_from_list_and_read::<R>,
        ),
        Scenario::new(
            "listing_a_file_and_reading_a_directory_or_a_symlink_are_refused",
            listing_a_file_and_reading_a_directory_or_a_symlink_are_refused::<R>,
        ),
        Scenario::new(
            "a_single_file_output_is_read_without_a_rel",
            a_single_file_output_is_read_without_a_rel::<R>,
        ),
        Scenario::new(
            "a_report_file_is_read_directly",
            a_report_file_is_read_directly::<R>,
        ),
        Scenario::new(
            "reading_a_directory_is_refused_even_when_it_is_the_located_output",
            reading_a_directory_is_refused_even_when_it_is_the_located_output::<R>,
        ),
        Scenario::new(
            "a_path_that_is_relative_or_climbs_is_refused",
            a_path_that_is_relative_or_climbs_is_refused::<R>,
        ),
        Scenario::new(
            "an_entry_of_a_listed_directory_is_read_through_its_joined_path",
            an_entry_of_a_listed_directory_is_read_through_its_joined_path::<R>,
        ),
        Scenario::new(
            "a_read_returns_the_file_as_it_is_at_the_call",
            a_read_returns_the_file_as_it_is_at_the_call::<R>,
        ),
        Scenario::new("no_call_changes_the_tree", no_call_changes_the_tree::<R>),
    ]
}

/// `a.txt` sorts before `a/y` because `.` is below `/`; a walk that lists a directory's
/// children right after it gets that wrong.
fn a_listing_is_sorted_relative_paths<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[
        ("t/b.txt", Node::File(b"1")),
        ("t/a/z", Node::File(b"2")),
        ("t/a/y", Node::File(b"3")),
        ("t/B", Node::File(b"4")),
        ("t/a.txt", Node::File(b"5")),
    ]);
    let listing = must(files.list(&at(rig, "t")), "list");
    let rels: Vec<&str> = listing.iter().map(|e| e.rel.as_str()).collect();
    same(
        rels,
        vec!["B", "a", "a.txt", "a/y", "a/z", "b.txt"],
        "the listing, sorted bytewise by rel",
    );
}

fn a_listing_names_the_kind_of_every_entry<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[
        ("k/file", Node::File(b"x")),
        ("k/dir", Node::Directory),
        ("k/link", Node::Link("k/file")),
    ]);
    let listing = must(files.list(&at(rig, "k")), "list");
    let kinds: Vec<(&str, FileKind)> = listing.iter().map(|e| (e.rel.as_str(), e.kind)).collect();
    same(
        kinds,
        vec![
            ("dir", FileKind::Directory),
            ("file", FileKind::File),
            ("link", FileKind::Symlink),
        ],
        "the kind of each entry",
    );
}

fn a_listed_symlink_is_never_descended<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[
        ("d/real/inner", Node::File(b"x")),
        ("d/link", Node::Link("d/real")),
    ]);
    let listing = must(files.list(&at(rig, "d")), "list");
    let rels: Vec<&str> = listing.iter().map(|e| e.rel.as_str()).collect();
    same(
        rels.contains(&"real/inner"),
        true,
        "the real directory is listed through its own name",
    );
    same(
        rels.iter().any(|rel| rel.starts_with("link/")),
        false,
        "an entry below the symlink",
    );
}

fn stat_tells_a_file_a_directory_a_symlink_and_nothing_apart<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[
        ("s/file", Node::File(b"x")),
        ("s/dir", Node::Directory),
        ("s/to_file", Node::Link("s/file")),
        ("s/to_dir", Node::Link("s/dir")),
        ("s/dangling", Node::Link("s/nowhere")),
    ]);
    let mut stat = |rel: &str| must(files.stat(&at(rig, rel)), "stat");
    same(stat("s/file"), Some(FileKind::File), "a file");
    same(stat("s/dir"), Some(FileKind::Directory), "a directory");
    same(
        stat("s/to_file"),
        Some(FileKind::Symlink),
        "a link to a file",
    );
    same(
        stat("s/to_dir"),
        Some(FileKind::Symlink),
        "a link to a directory",
    );
    same(
        stat("s/dangling"),
        Some(FileKind::Symlink),
        "a dangling link",
    );
    same(stat("s/nothing"), None, "nothing");
}

fn a_missing_path_is_none_from_stat_and_permanent_from_list_and_read<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[("m/present", Node::File(b"x"))]);
    let gone = at(rig, "m/nothing");
    same(
        must(files.stat(&gone), "stat"),
        None,
        "stat of a missing path",
    );
    let listed = must_err(files.list(&gone), "list of a missing path");
    same(
        permanent_naming(&listed, &gone),
        true,
        "list names the path",
    );
    let read = must_err(files.read(&gone), "read of a missing path");
    same(permanent_naming(&read, &gone), true, "read names the path");
}

fn listing_a_file_and_reading_a_directory_or_a_symlink_are_refused<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[
        ("r/file", Node::File(b"x")),
        ("r/dir", Node::Directory),
        ("r/link", Node::Link("r/file")),
    ]);
    let (file, dir, link) = (at(rig, "r/file"), at(rig, "r/dir"), at(rig, "r/link"));
    let cases = [
        ("list of a file", &file, must_err(files.list(&file), "list")),
        (
            "read of a directory",
            &dir,
            must_err(files.read(&dir), "read"),
        ),
        (
            "read of a symlink",
            &link,
            must_err(files.read(&link), "read"),
        ),
    ];
    for (what, path, error) in &cases {
        same(
            permanent_naming(error, path),
            true,
            &format!("the {what} is Permanent and names the path"),
        );
    }
}

fn a_single_file_output_is_read_without_a_rel<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[("out.bin", Node::File(b"the output"))]);
    let bytes = must(
        files.read(&at(rig, "out.bin")),
        "read of a file never listed",
    );
    same(bytes, b"the output".to_vec(), "the bytes of the file");
}

fn a_report_file_is_read_directly<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[(
        "scratch/inv-1/events.json-lines",
        Node::File(b"{\"event\":1}\n"),
    )]);
    let path = at(rig, "scratch/inv-1/events.json-lines");
    let bytes = must(files.read(&path), "read of a report file never listed");
    same(
        bytes,
        b"{\"event\":1}\n".to_vec(),
        "the bytes of the report",
    );
}

fn reading_a_directory_is_refused_even_when_it_is_the_located_output<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[("out/pkg/layout", Node::File(b"x"))]);
    let located = at(rig, "out/pkg");
    same(
        must(files.stat(&located), "stat"),
        Some(FileKind::Directory),
        "the located output is a directory",
    );
    let error = must_err(files.read(&located), "read of the directory");
    same(
        permanent_naming(&error, &located),
        true,
        "Permanent naming the path",
    );
}

fn a_path_that_is_relative_or_climbs_is_refused<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[("c/real", Node::File(b"secret"))]);
    let paths = [
        "relative/path".to_owned(),
        String::new(),
        "c/real".to_owned(),
        at(rig, "c/sub/../real"),
        at(rig, "../c/real"),
        at(rig, "c/.."),
    ];
    for path in &paths {
        let errors = [
            must_err(files.stat(path), "stat"),
            must_err(files.list(path), "list"),
            must_err(files.read(path), "read"),
        ];
        same(
            errors.iter().all(is_refused),
            true,
            &format!("stat, list and read of {path:?} are Refused"),
        );
    }
}

fn an_entry_of_a_listed_directory_is_read_through_its_joined_path<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[
        ("e/a", Node::File(b"A")),
        ("e/n/b", Node::File(b"B")),
        ("e/n/c", Node::File(b"C")),
    ]);
    let dir = at(rig, "e");
    let listing = must(files.list(&dir), "list");
    let wanted = [("a", b"A"), ("n/b", b"B"), ("n/c", b"C")];
    let mut read = Vec::new();
    for entry in listing
        .iter()
        .filter(|e| wanted.iter().any(|(rel, _)| *rel == e.rel))
    {
        let path = must(entry_path(&dir, &entry.rel), "entry_path");
        read.push((
            entry.rel.as_str(),
            must(files.read(&path), "read of an entry"),
        ));
    }
    same(
        read,
        wanted
            .iter()
            .map(|(rel, bytes)| (*rel, bytes.to_vec()))
            .collect(),
        "every file of the tree read through its joined entry path",
    );
}

fn a_read_returns_the_file_as_it_is_at_the_call<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[("v/f", Node::File(b"first"))]);
    let path = at(rig, "v/f");
    must(files.list(&at(rig, "v")), "list");
    same(
        must(files.read(&path), "first read"),
        b"first".to_vec(),
        "the first read",
    );
    rig.change(&mut files, "v/f", b"second");
    same(
        must(files.read(&path), "second read"),
        b"second".to_vec(),
        "the read after a change",
    );
}

fn no_call_changes_the_tree<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[
        ("n/file", Node::File(b"data")),
        ("n/dir", Node::Directory),
        ("n/link", Node::Link("n/file")),
    ]);
    let before = rig.snapshot(&files);
    let dir = at(rig, "n");
    let _ = files.list(&dir);
    for rel in ["n/file", "n/dir", "n/link", "n/nothing"] {
        let path = at(rig, rel);
        let _ = files.stat(&path);
        let _ = files.list(&path);
        let _ = files.read(&path);
    }
    let _ = files.read(&at(rig, "../escape"));
    same(
        rig.snapshot(&files),
        before,
        "the tree after every method of the reader",
    );
}
