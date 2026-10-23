// SPDX-License-Identifier: Apache-2.0
mod common;

use common::{scratch, write};
use xtask::tree::read_tree;

#[test]
fn tree_reads_text_files_sorted_and_skips_build_output_git_and_binary_files() {
    let dir = scratch("tree");
    write(&dir, "b.txt", "b");
    write(&dir, "a/inner.txt", "inner");
    write(&dir, "target/skipped.txt", "x");
    write(&dir, ".git/skipped.txt", "x");
    std::fs::write(dir.join("blob.bin"), [0xff, 0xfe, 0x00]).expect("write blob");
    let names: Vec<String> = read_tree(&dir)
        .expect("read")
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    assert_eq!(names, ["a/inner.txt", "b.txt"]);
}

#[test]
fn tree_of_a_missing_directory_is_an_error() {
    assert!(read_tree(&scratch("tree-missing").join("nope")).is_err());
}
