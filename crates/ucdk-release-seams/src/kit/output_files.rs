// SPDX-License-Identifier: Apache-2.0
//! The `OutputFiles` kit: fourteen scenarios over the three questions a build system asks of a
//! tree it did not make (what is at a path, what is below a directory, what a file holds).

use super::{Report, Scenario, must, must_err, must_some, permanent_naming, run_scenarios, same};
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

    /// The path, kind and bytes of everything in the tree (no bytes for a directory or a
    /// symlink), taken without the reader.
    fn snapshot(&self, files: &Self::Files) -> Vec<(String, FileKind, Vec<u8>)>;
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
        Scenario::new(
            "a_dot_part_a_repeated_slash_and_a_trailing_slash_name_the_path_they_stand_in",
            a_dot_part_a_repeated_slash_and_a_trailing_slash_name_the_path_they_stand_in::<R>,
        ),
    ]
}

/// A reader drops each `.` part and each empty part of a path itself, a trailing slash
/// included, before it looks the path up, so `/d/./x` and `/d//x` are `/d/x`, `/d/x/` is `/d/x`
/// (a file) and `list` of `/d/` is `list` of `/d`; the contract refuses only a `..` part. A
/// reader that refuses or fails on any of them, answers a slashed file as nothing, or lists
/// with a leading slash, is not one.
fn a_dot_part_a_repeated_slash_and_a_trailing_slash_name_the_path_they_stand_in<
    R: OutputFilesRig,
>(
    rig: &R,
) {
    let mut files = rig.reader(&[
        ("d/x", Node::File(b"X")),
        ("d/z", Node::File(b"Z")),
        ("d/sub/y", Node::File(b"Y")),
    ]);
    let listed = must(files.list(&at(rig, "d")), "list of the plain path");
    for suffix in ["/d/", "/d//", "//d", "/./d", "/d/.", "/./d/./"] {
        let spelled = format!("{}{suffix}", rig.root());
        same(
            must(files.list(&spelled), &format!("list of {spelled:?}")),
            listed.clone(),
            &format!("the listing of {spelled:?}, which is the listing of the plain path"),
        );
        same(
            must(files.stat(&spelled), &format!("stat of {spelled:?}")),
            Some(FileKind::Directory),
            &format!("what is at {spelled:?}"),
        );
    }
    for suffix in [
        "/d/./x", "/d//x", "/./d/x", "/d/./z", "/d//z", "/d/x/", "/d/z/",
    ] {
        let spelled = format!("{}{suffix}", rig.root());
        let bare = suffix.trim_end_matches('/');
        let want: &[u8] = if bare.ends_with('x') { b"X" } else { b"Z" };
        same(
            must(files.read(&spelled), &format!("read of {spelled:?}")),
            want.to_vec(),
            &format!("the bytes of {spelled:?}"),
        );
        same(
            must(files.stat(&spelled), &format!("stat of {spelled:?}")),
            Some(FileKind::File),
            &format!("what is at {spelled:?}"),
        );
    }
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
    // More entries than a reader might list at most, whose names sort differently as numbers
    // than as bytes (`f10` is before `f2`).
    let paths: Vec<String> = (0..600).map(|n| format!("w/f{n}")).collect();
    let tree: Vec<(&str, Node<'_>)> = paths
        .iter()
        .map(|path| (path.as_str(), Node::File(b"x")))
        .collect();
    let mut files = rig.reader(&tree);
    let listing = must(files.list(&at(rig, "w")), "list of a wide directory");
    let mut want: Vec<String> = (0..600).map(|n| format!("f{n}")).collect();
    want.sort();
    same(
        listing.into_iter().map(|e| e.rel).collect::<Vec<_>>(),
        want,
        "the listing of 600 entries, sorted bytewise by rel",
    );
}

fn a_listing_names_the_kind_of_every_entry<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[
        ("k/file", Node::File(b"x")),
        ("k/dir", Node::Directory),
        ("k/link", Node::Link("k/file")),
        ("k/link_to_dir", Node::Link("k/dir")),
        ("k/dangling", Node::Link("k/nowhere")),
        // Below the top: a link, a file and an empty directory three directories down, an empty
        // directory one down, and directories and files named with two dots.
        ("k/d/e/f", Node::File(b"f")),
        ("k/d/e/lnk", Node::Link("k/a..b")),
        ("k/d/emptyin", Node::Directory),
        ("k/g/h/i/j/leaf", Node::File(b"l")),
        ("k/g/h/i/j/lnk", Node::Link("k/dir")),
        ("k/g/h/i/j/empty", Node::Directory),
        ("k/p/empty", Node::Directory),
        ("k/a..b/x", Node::File(b"x")),
        ("k/..dots", Node::Directory),
        ("k/dots../y", Node::File(b"y")),
    ]);
    let listing = must(files.list(&at(rig, "k")), "list");
    let kinds: Vec<(&str, FileKind)> = listing.iter().map(|e| (e.rel.as_str(), e.kind)).collect();
    let (file, dir, link) = (FileKind::File, FileKind::Directory, FileKind::Symlink);
    same(
        kinds,
        vec![
            ("..dots", dir),
            ("a..b", dir),
            ("a..b/x", file),
            ("d", dir),
            ("d/e", dir),
            ("d/e/f", file),
            ("d/e/lnk", link),
            ("d/emptyin", dir),
            ("dangling", link),
            ("dir", dir),
            ("dots..", dir),
            ("dots../y", file),
            ("file", file),
            ("g", dir),
            ("g/h", dir),
            ("g/h/i", dir),
            ("g/h/i/j", dir),
            ("g/h/i/j/empty", dir),
            ("g/h/i/j/leaf", file),
            ("g/h/i/j/lnk", link),
            ("link", link),
            ("link_to_dir", link),
            ("p", dir),
            ("p/empty", dir),
        ],
        "the kind of each entry, once each, at every depth",
    );
    // A directory whose name holds two dots lists as any other.
    let dotted = must(files.list(&at(rig, "k/a..b")), "list of a..b");
    same(
        dotted.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(),
        vec!["x"],
        "the listing of a directory named a..b",
    );
    let dotted = must(files.list(&at(rig, "k/dots..")), "list of dots..");
    same(
        dotted.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(),
        vec!["y"],
        "the listing of a directory named dots..",
    );
    // A directory with nothing in it lists as empty; it is not a fault.
    same(
        must(files.list(&at(rig, "k/dir")), "list of an empty directory").is_empty(),
        true,
        "the listing of an empty directory is empty",
    );
    // Siblings whose names start with the name of the listed directory are not below it: `d`
    // next to `d2`, `d.bak` and `d-x`, at the top and a directory down, and an empty `e` next
    // to `e2`.
    let mut files = rig.reader(&[
        ("q/d/x", Node::File(b"x")),
        ("q/d2/x", Node::File(b"2")),
        ("q/d.bak/x", Node::File(b"b")),
        ("q/d-x/x", Node::File(b"-")),
        ("q/d.bak/deeper/y", Node::File(b"y")),
        ("q/e", Node::Directory),
        ("q/e2", Node::File(b"2")),
        ("q/e-x", Node::Link("q/e")),
        ("r/p/d/x", Node::File(b"x")),
        ("r/p/d2/y", Node::File(b"y")),
        ("r/p/d.bak", Node::File(b"z")),
        ("r/p/d-x", Node::Directory),
    ]);
    for (rel, want) in [
        ("q/d", vec!["x"]),
        ("q/d2", vec!["x"]),
        ("q/d.bak", vec!["deeper", "deeper/y", "x"]),
        ("q/d-x", vec!["x"]),
        ("q/e", vec![]),
        ("r/p/d", vec!["x"]),
        ("r/p/d-x", vec![]),
    ] {
        let listing = must(files.list(&at(rig, rel)), &format!("list of {rel:?}"));
        same(
            listing.iter().map(|e| e.rel.as_str()).collect::<Vec<_>>(),
            want,
            &format!("the listing of {rel:?}, which holds nothing of its siblings"),
        );
    }
}

fn a_listed_symlink_is_never_descended<R: OutputFilesRig>(rig: &R) {
    // One link at the top of the listed directory and one below it.
    let mut files = rig.reader(&[
        ("d/real/inner", Node::File(b"x")),
        ("d/link", Node::Link("d/real")),
        ("d/a/link", Node::Link("d/real")),
    ]);
    let listing = must(files.list(&at(rig, "d")), "list");
    let rels: Vec<&str> = listing.iter().map(|e| e.rel.as_str()).collect();
    same(
        rels,
        vec!["a", "a/link", "link", "real", "real/inner"],
        "the listing: the real directory through its own name, and nothing below a symlink",
    );
}

fn stat_tells_a_file_a_directory_a_symlink_and_nothing_apart<R: OutputFilesRig>(rig: &R) {
    let mut files = rig.reader(&[
        ("s/file", Node::File(b"x")),
        ("s/dir", Node::Directory),
        ("s/to_file", Node::Link("s/file")),
        ("s/to_dir", Node::Link("s/dir")),
        ("s/dangling", Node::Link("s/nowhere")),
        ("s/a..b", Node::File(b"x")),
        ("s/..dots", Node::Directory),
        ("s/dots../x", Node::File(b"x")),
        // A link to itself and two links to each other: a reader that follows a link to tell
        // what it is gets a loop, and the operating system's answer to a loop is an error.
        ("s/self", Node::Link("s/self")),
        ("s/loop_a", Node::Link("s/loop_b")),
        ("s/loop_b", Node::Link("s/loop_a")),
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
    // Below a regular file nothing is, and the operating system's answer is no fault.
    same(stat("s/file/child"), None, "a path below a regular file");
    same(
        stat("s/file/a/b"),
        None,
        "a path two parts below a regular file",
    );
    same(
        stat("s/nodir/x"),
        None,
        "a path in a directory that is not there",
    );
    // Two dots inside a part make no `..` part: a file, a directory and a file in a directory
    // so named are found as any other.
    same(stat("s/a..b"), Some(FileKind::File), "a file named a..b");
    same(
        stat("s/..dots"),
        Some(FileKind::Directory),
        "a directory named ..dots",
    );
    same(
        stat("s/dots.."),
        Some(FileKind::Directory),
        "a directory named dots..",
    );
    same(
        stat("s/dots../x"),
        Some(FileKind::File),
        "a file in a directory named dots..",
    );
    // A trailing slash makes an operating system follow a symlink; the reader must not, and the
    // slash names nothing of its own, so the answer is the symlink, whatever is at its target.
    for rel in ["s/to_file/", "s/to_dir/", "s/dangling/"] {
        same(
            stat(rel),
            Some(FileKind::Symlink),
            &format!("what is at the symlink {rel:?} named with a trailing slash"),
        );
    }
    // A link is a `Symlink` whatever it points at, and a link that points at itself, or at a
    // link that points back, is one too: `stat` does not follow, so there is no loop to meet.
    for rel in [
        "s/self",
        "s/loop_a",
        "s/loop_b",
        "s/self/",
        "s/loop_a/",
        "s/loop_b/",
    ] {
        same(
            stat(rel),
            Some(FileKind::Symlink),
            &format!("what is at the looping symlink {rel:?}"),
        );
    }
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
        ("r/dir/inside", Node::File(b"y")),
        ("r/link", Node::Link("r/file")),
        ("r/link_to_dir", Node::Link("r/dir")),
        ("r/dangling", Node::Link("r/nowhere")),
        // Links to a file that are in another directory than it, directly below the root and
        // three parts down, and directories directly below the root and three parts down.
        ("q/lnk", Node::Link("r/file")),
        ("rootlink", Node::Link("r/file")),
        ("a/b/deeplink", Node::Link("r/file")),
        ("rootdir/inside", Node::File(b"z")),
        ("a/b/deepdir/inside", Node::File(b"z")),
    ]);
    let (file, dir, link) = (at(rig, "r/file"), at(rig, "r/dir"), at(rig, "r/link"));
    let (link_to_dir, slashed) = (at(rig, "r/link_to_dir"), at(rig, "r/link/"));
    let slashed_dir_link = at(rig, "r/link_to_dir/");
    let dangling = at(rig, "r/dangling");
    let below = at(rig, "r/file/child");
    // Two or more parts below a regular file, and in a directory that is not there.
    let far_below = at(rig, "r/file/a/b");
    let in_missing = at(rig, "r/nodir/x");
    let extra = [
        (
            "read of a symlink in another directory than its target",
            at(rig, "q/lnk"),
        ),
        (
            "read of a symlink directly below the root",
            at(rig, "rootlink"),
        ),
        (
            "read of a symlink three parts below the root",
            at(rig, "a/b/deeplink"),
        ),
        (
            "read of a directory directly below the root",
            at(rig, "rootdir"),
        ),
        (
            "read of a directory three parts below the root",
            at(rig, "a/b/deepdir"),
        ),
    ];
    let cases = [
        (
            "read of a path two parts below a regular file",
            &far_below,
            must_err(files.read(&far_below), "read"),
        ),
        (
            "list of a path two parts below a regular file",
            &far_below,
            must_err(files.list(&far_below), "list"),
        ),
        (
            "read of a path in a directory that is not there",
            &in_missing,
            must_err(files.read(&in_missing), "read"),
        ),
        (
            "list of a path in a directory that is not there",
            &in_missing,
            must_err(files.list(&in_missing), "list"),
        ),
        (
            "list of a symlink to a directory named with a trailing slash",
            &slashed_dir_link,
            must_err(files.list(&slashed_dir_link), "list"),
        ),
        (
            "read of a path below a regular file",
            &below,
            must_err(files.read(&below), "read"),
        ),
        (
            "list of a path below a regular file",
            &below,
            must_err(files.list(&below), "list"),
        ),
        (
            "read of a symlink to a directory",
            &link_to_dir,
            must_err(files.read(&link_to_dir), "read"),
        ),
        (
            "read of a dangling symlink",
            &dangling,
            must_err(files.read(&dangling), "read"),
        ),
        (
            "list of a dangling symlink",
            &dangling,
            must_err(files.list(&dangling), "list"),
        ),
        ("list of a file", &file, must_err(files.list(&file), "list")),
        (
            "list of a symlink to a directory",
            &link_to_dir,
            must_err(files.list(&link_to_dir), "list"),
        ),
        (
            "read of a symlink named with a trailing slash",
            &slashed,
            must_err(files.read(&slashed), "read"),
        ),
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
    for (what, path) in &extra {
        let error = must_err(files.read(path), "read");
        same(
            permanent_naming(&error, path),
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
        // A first part that climbs out of the root.
        format!("/..{}/c/real", rig.root()),
        "/..".to_owned(),
        // Relative, though it starts with `./` or with the first part of the root.
        "./c/real".to_owned(),
        format!("{}/c/real", rig.root().trim_start_matches('/')),
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
    // Bytes that are not text, a file past any buffer a reader might size at 4 KiB, 64 KiB,
    // 128 KiB or a megabyte, an empty file (below the top too), a hidden one, ones three and
    // seven directories down, names that hold two dots without being a `..` part, and names
    // with a space, a newline or a character that is not ASCII, in a file or a directory.
    let binary: &[u8] = &[0xff, 0xfe, 0x00, 0x80, 0xc3, 0x28];
    let big: Vec<u8> = (0..=255u8).cycle().take(2_100_000).collect();
    // Bytes a reader that treats a file as text, or as whole buffers, would alter: a carriage
    // return before a line feed, a byte order mark, one carriage return and line feed at the end,
    // zero bytes at the end, and lengths that are a multiple of 4096 or one past a power of two.
    let crlf: &[u8] = b"a\r\nb";
    let bom: &[u8] = &[0xef, 0xbb, 0xbf, b'x'];
    let ends_crlf: &[u8] = b"x\r\n";
    let nuls: &[u8] = &[1, 0, 0];
    let sized = |len: usize| -> Vec<u8> { (1..=251u8).cycle().take(len).collect() };
    let (k4096, k4097, k8192) = (sized(4096), sized(4097), sized(8192));
    let (k65536, k65537) = (sized(65_536), sized(65_537));
    // A multiple of 131,072, and ten bytes past 4 MiB, and bytes that end in blanks.
    let (m1, m4) = (sized(1_048_576), sized(4_194_314));
    let blanks: &[u8] = b"x  \t";
    let mut files = rig.reader(&[
        ("e/crlf", Node::File(crlf)),
        ("e/bom", Node::File(bom)),
        ("e/ends_crlf", Node::File(ends_crlf)),
        ("e/nuls", Node::File(nuls)),
        ("e/k4096", Node::File(&k4096)),
        ("e/k4097", Node::File(&k4097)),
        ("e/k8192", Node::File(&k8192)),
        ("e/k65536", Node::File(&k65536)),
        ("e/k65537", Node::File(&k65537)),
        ("e/m1", Node::File(&m1)),
        ("e/m4", Node::File(&m4)),
        ("e/blanks", Node::File(blanks)),
        ("e/sp ace", Node::File(b"S")),
        ("e/\u{e9}.txt", Node::File(b"E")),
        ("e/new\nline", Node::File(b"N")),
        ("e/n/empty", Node::File(b"")),
        ("e/dir with space/x", Node::File(b"W")),
        ("e/g/h/i/j/k/l/m", Node::File(b"M")),
        ("e/a", Node::File(b"A")),
        ("e/n/b", Node::File(b"B")),
        ("e/n/c", Node::File(b"C")),
        ("e/binary", Node::File(binary)),
        ("e/big", Node::File(&big)),
        ("e/empty", Node::File(b"")),
        ("e/.hidden", Node::File(b"H")),
        ("e/n/.hidden", Node::File(b"I")),
        ("e/p/q/r/deep", Node::File(b"D")),
        ("e/a..b", Node::File(b"J")),
        ("e/..dots", Node::File(b"K")),
        ("e/n/dots..", Node::File(b"L")),
    ]);
    let dir = at(rig, "e");
    let listing = must(files.list(&dir), "list");
    let wanted: [(&str, &[u8]); 30] = [
        ("m1", &m1),
        ("m4", &m4),
        ("blanks", blanks),
        ("crlf", crlf),
        ("bom", bom),
        ("ends_crlf", ends_crlf),
        ("nuls", nuls),
        ("k4096", &k4096),
        ("k4097", &k4097),
        ("k8192", &k8192),
        ("k65536", &k65536),
        ("k65537", &k65537),
        ("sp ace", b"S"),
        ("\u{e9}.txt", b"E"),
        ("new\nline", b"N"),
        ("n/empty", b""),
        ("dir with space/x", b"W"),
        ("g/h/i/j/k/l/m", b"M"),
        (".hidden", b"H"),
        ("..dots", b"K"),
        ("a", b"A"),
        ("a..b", b"J"),
        ("n/dots..", b"L"),
        ("big", &big),
        ("binary", binary),
        ("empty", b""),
        ("n/.hidden", b"I"),
        ("n/b", b"B"),
        ("n/c", b"C"),
        ("p/q/r/deep", b"D"),
    ];
    let mut read = Vec::new();
    for (rel, _) in &wanted {
        let entry = must_some(listing.iter().find(|e| e.rel == *rel), rel);
        let path = must(entry_path(&dir, &entry.rel), "entry_path");
        read.push((*rel, must(files.read(&path), "read of an entry")));
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
    // A change that keeps the length: a reader that trusts a length to tell a file is the same
    // answers the old bytes.
    rig.change(&mut files, "v/f", b"FIRST");
    same(
        must(files.read(&path), "read after a change of the same length"),
        b"FIRST".to_vec(),
        "the read after a change that keeps the length",
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
        // Capitals and a carriage return, so that a call that rewrites the bytes to others of
        // the same length (lower case, a normalised line end) shows in the snapshot.
        ("n/file", Node::File(b"Data\r\nEnd")),
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
