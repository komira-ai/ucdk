// SPDX-License-Identifier: Apache-2.0
//! The path rules the `OutputFiles` readers share, written once.

use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{SeamError, check_path, entry_path, valid_rel};

#[test]
fn entry_path_joins_a_checked_rel() {
    assert_eq!(
        entry_path("/out/pkg", "a/b.txt"),
        Ok("/out/pkg/a/b.txt".to_owned())
    );
    assert_eq!(entry_path("/out/pkg/", "a"), Ok("/out/pkg/a".to_owned()));
}

#[test]
fn entry_path_refuses_an_empty_dot_dotdot_or_absolute_rel_and_an_empty_part() {
    for rel in ["", ".", "..", "a/../b", "/a", "a//b", "a/", "./a"] {
        assert_eq!(
            entry_path("/out", rel),
            Err(SeamError::Refused(Reason::BuildOutput)),
            "{rel:?}"
        );
        assert!(!valid_rel(rel), "{rel:?}");
    }
    assert!(valid_rel("a/b..c/.d"));
}

#[test]
fn a_path_must_be_absolute_and_must_not_climb() {
    assert_eq!(check_path("/out/pkg"), Ok(()));
    assert_eq!(check_path("/"), Ok(()));
    for path in ["", "out", "/out/../etc", "/..", "./out"] {
        assert_eq!(
            check_path(path),
            Err(SeamError::Refused(Reason::BuildOutput)),
            "{path:?}"
        );
    }
}
