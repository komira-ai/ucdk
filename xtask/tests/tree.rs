// SPDX-License-Identifier: Apache-2.0
mod common;

use common::{scratch, write};
use xtask::tree::read_tree;

#[test]
fn tree_reads_files_sorted_and_skips_build_output_and_git() {
    let dir = scratch("tree");
    write(&dir, "b.txt", "b");
    write(&dir, "a/inner.txt", "inner");
    write(&dir, "target/skipped.txt", "x");
    write(&dir, ".git/skipped.txt", "x");
    let names: Vec<String> = read_tree(&dir)
        .expect("read")
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    assert_eq!(names, ["a/inner.txt", "b.txt"]);
}

#[test]
fn tree_reads_a_file_that_is_not_utf8_with_its_text_around_the_bad_bytes() {
    let dir = scratch("tree-lossy");
    let mut bytes = vec![0xff, 0xfe, b' '];
    bytes.extend_from_slice(b"text after");
    std::fs::write(dir.join("blob.bin"), bytes).expect("write blob");
    let tree = read_tree(&dir).expect("read");
    assert_eq!(tree.len(), 1);
    assert!(tree[0].1.ends_with(" text after"), "{:?}", tree[0].1);
}

#[test]
fn tree_of_a_missing_directory_is_an_error() {
    assert!(read_tree(&scratch("tree-missing").join("nope")).is_err());
}

#[test]
fn only_the_top_level_target_and_git_are_skipped() {
    let dir = scratch("tree-nested");
    write(&dir, "target/skipped.txt", "x");
    write(&dir, ".git/skipped.txt", "x");
    write(&dir, "docs/target/read.md", "x");
    write(&dir, "crates/a/src/target/read.rs", "x");
    write(&dir, "vendor/.git/read.txt", "x");
    let names: Vec<String> = read_tree(&dir)
        .expect("read")
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    assert_eq!(
        names,
        [
            "crates/a/src/target/read.rs",
            "docs/target/read.md",
            "vendor/.git/read.txt"
        ]
    );
}
