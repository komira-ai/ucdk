// SPDX-License-Identifier: Apache-2.0
//! The checks every publisher applies to a declaration of names, and how their errors read.

use ucdk_release_seams::{DeclareError, check_declaration};

fn list(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

#[test]
fn plain_distinct_names_and_the_empty_list_are_accepted() {
    assert_eq!(check_declaration(&list(&[])), Ok(()));
    assert_eq!(
        check_declaration(&list(&["tool", "Tool2", "my_pkg"])),
        Ok(())
    );
}

#[test]
fn an_empty_name_is_refused_with_its_position() {
    assert_eq!(
        check_declaration(&list(&["a", "b", ""])),
        Err(DeclareError::Empty { index: 2 })
    );
}

#[test]
fn whitespace_and_a_slash_are_refused_naming_the_entry_and_whitespace_is_asked_first() {
    for bad in ["a b", " a", "a\t", "a\nb", "a\u{a0}b"] {
        assert_eq!(
            check_declaration(&list(&["ok", bad])),
            Err(DeclareError::Whitespace {
                name: bad.to_owned()
            }),
            "{bad:?}"
        );
    }
    assert_eq!(
        check_declaration(&list(&["a/b"])),
        Err(DeclareError::Slash {
            name: "a/b".to_owned()
        })
    );
    assert_eq!(
        check_declaration(&list(&["a /b"])),
        Err(DeclareError::Whitespace {
            name: "a /b".to_owned()
        })
    );
}

#[test]
fn an_earlier_name_again_is_refused_ignoring_ascii_case_and_only_ascii_case() {
    assert_eq!(
        check_declaration(&list(&["tool", "x", "TOOL"])),
        Err(DeclareError::Duplicate {
            name: "TOOL".to_owned()
        })
    );
    assert_eq!(
        check_declaration(&list(&["a", "a"])),
        Err(DeclareError::Duplicate {
            name: "a".to_owned()
        })
    );
    // Non-ASCII letters are not case-folded, and a name is not normalised: these stay distinct.
    assert_eq!(check_declaration(&list(&["\u{c9}", "\u{e9}"])), Ok(()));
    assert_eq!(
        check_declaration(&list(&["my_pkg", "my-pkg", "my.pkg"])),
        Ok(())
    );
}

#[test]
fn declaration_errors_read_as_sentences() {
    let texts: Vec<String> = [
        DeclareError::Empty { index: 1 },
        DeclareError::Whitespace {
            name: "a b".to_owned(),
        },
        DeclareError::Slash {
            name: "a/b".to_owned(),
        },
        DeclareError::Duplicate {
            name: "A".to_owned(),
        },
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        texts,
        [
            "declared name 1 is empty",
            "declared name \"a b\" holds whitespace",
            "declared name \"a/b\" holds a '/'",
            "declared name \"A\" repeats an earlier one, ignoring ASCII case",
        ]
    );
}
