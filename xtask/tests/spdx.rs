// SPDX-License-Identifier: Apache-2.0
use xtask::spdx::{check_spdx, collect};

fn file(path: &str, text: &str) -> (String, String) {
    (path.to_owned(), text.to_owned())
}

#[test]
fn missing_header_flagged() {
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
fn every_file_in_the_repository_has_the_header() {
    let files = collect(&xtask::workspace_root()).expect("walk");
    assert!(
        files.iter().any(|(p, _)| p.ends_with("xtask/src/spdx.rs")),
        "the walk found sources"
    );
    assert_eq!(check_spdx(&files), vec![]);
}
