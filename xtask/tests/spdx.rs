// SPDX-License-Identifier: Apache-2.0
mod common;

use xtask::spdx::check_spdx;
use xtask::tree::read_tree;

fn file(path: &str, text: &str) -> (String, String) {
    (path.to_owned(), text.to_owned())
}

#[test]
fn spdx_flags_file_without_header() {
    let files = [
        file("a.rs", "fn main() {}\n"),
        file("b.proto", "syntax = \"proto3\";\n"),
        file("c.rs", "\n// SPDX-License-Identifier: Apache-2.0\n"),
        file("d.rs", "// SPDX-License-Identifier: MIT\n"),
        file(
            "ok.rs",
            "// SPDX-License-Identifier: Apache-2.0\nfn main() {}\n",
        ),
        file(
            "ok.proto",
            "// SPDX-License-Identifier: Apache-2.0\nsyntax = \"proto3\";\n",
        ),
        file("notes.md", "no header needed\n"),
    ];
    let bad: Vec<String> = check_spdx(&files).into_iter().map(|f| f.subject).collect();
    assert_eq!(bad, ["a.rs", "b.proto", "c.rs", "d.rs"]);
}

#[test]
fn every_source_file_in_the_repository_has_the_header() {
    let tree = read_tree(&xtask::workspace_root()).expect("read the repository");
    assert!(
        tree.iter().any(|(p, _)| p == "xtask/src/spdx.rs"),
        "the walk found sources"
    );
    assert_eq!(check_spdx(&tree), vec![]);
}

#[test]
fn spdx_checks_a_source_file_that_is_not_utf8() {
    let dir = common::scratch("spdx-lossy");
    std::fs::write(dir.join("bad.rs"), [0xff, b'\n']).expect("write");
    let mut ok = b"// SPDX-License-Identifier: Apache-2.0\n".to_vec();
    ok.push(0xff);
    std::fs::write(dir.join("ok.rs"), ok).expect("write");
    let tree = read_tree(&dir).expect("read");
    let bad: Vec<String> = check_spdx(&tree).into_iter().map(|f| f.subject).collect();
    assert_eq!(bad, ["bad.rs"]);
}
