// SPDX-License-Identifier: Apache-2.0
//! The schema-version policy: which majors of a document a reader accepts and why it refuses
//! the others.

use ucdk_release::schema::{PolicyError, SchemaPolicy, SchemaRefusal};

fn policy(current: i32, oldest_read: i32) -> SchemaPolicy {
    SchemaPolicy::new(current, oldest_read).expect("a valid policy")
}

#[test]
fn the_current_major_is_read() {
    assert_eq!(policy(3, 2).check(3), Ok(()));
}

#[test]
fn every_major_from_the_oldest_read_to_the_current_is_read() {
    let policy = policy(5, 2);
    for found in 2..=5 {
        assert_eq!(policy.check(found), Ok(()), "{found}");
    }
}

#[test]
fn a_newer_major_needs_a_newer_reader() {
    let policy = policy(2, 1);
    assert_eq!(
        policy.check(3),
        Err(SchemaRefusal::NeedsNewer {
            found: 3,
            current: 2
        })
    );
    assert_eq!(
        policy.check(i32::MAX),
        Err(SchemaRefusal::NeedsNewer {
            found: i32::MAX,
            current: 2
        })
    );
}

#[test]
fn a_major_below_the_oldest_read_is_no_longer_read() {
    let policy = policy(3, 2);
    assert_eq!(
        policy.check(1),
        Err(SchemaRefusal::NoLongerRead {
            found: 1,
            oldest_read: 2,
            current: 3
        })
    );
}

#[test]
fn a_document_that_states_no_version_is_refused_as_missing() {
    let policy = policy(1, 1);
    assert_eq!(policy.check(0), Err(SchemaRefusal::Missing { current: 1 }));
}

#[test]
fn a_negative_version_is_refused_as_missing_and_never_read() {
    let policy = policy(1, 1);
    assert_eq!(policy.check(-1), Err(SchemaRefusal::Missing { current: 1 }));
    assert_eq!(
        policy.check(i32::MIN),
        Err(SchemaRefusal::Missing { current: 1 })
    );
}

#[test]
fn a_policy_that_reads_only_its_current_major_refuses_the_one_before() {
    let policy = policy(2, 2);
    assert_eq!(
        policy.check(1),
        Err(SchemaRefusal::NoLongerRead {
            found: 1,
            oldest_read: 2,
            current: 2
        })
    );
    assert_eq!(policy.check(2), Ok(()));
}

#[test]
fn a_writer_states_the_current_major() {
    assert_eq!(policy(4, 2).current(), 4);
}

#[test]
fn the_refusals_say_the_versions_and_what_to_do() {
    assert_eq!(
        SchemaRefusal::Missing { current: 1 }.to_string(),
        "no schema_version; add `schema_version: 1`"
    );
    assert_eq!(
        SchemaRefusal::NeedsNewer {
            found: 3,
            current: 2
        }
        .to_string(),
        "schema_version 3 needs a newer ucdk (this ucdk reads up to major 2)"
    );
    assert_eq!(
        SchemaRefusal::NoLongerRead {
            found: 1,
            oldest_read: 2,
            current: 3
        }
        .to_string(),
        "schema_version 1 is no longer read (this ucdk reads major 2..3)"
    );
    assert_eq!(
        SchemaRefusal::NoLongerRead {
            found: 1,
            oldest_read: 2,
            current: 2
        }
        .to_string(),
        "schema_version 1 is no longer read (this ucdk reads major 2)"
    );
}

#[test]
fn a_policy_is_built_only_from_a_range_that_has_a_current_major() {
    for (current, oldest_read) in [(1, 1), (2, 2), (5, 2), (5, 1), (i32::MAX, 1)] {
        let built = SchemaPolicy::new(current, oldest_read).expect("a valid range");
        assert_eq!(built.current(), current);
    }
}

#[test]
fn a_policy_that_reads_a_major_above_its_current_is_refused() {
    assert_eq!(
        SchemaPolicy::new(1, 2),
        Err(PolicyError {
            current: 1,
            oldest_read: 2
        })
    );
    assert!(SchemaPolicy::new(3, 4).is_err());
}

#[test]
fn a_policy_that_reads_a_major_below_one_is_refused() {
    assert_eq!(
        SchemaPolicy::new(2, 0),
        Err(PolicyError {
            current: 2,
            oldest_read: 0
        })
    );
    assert!(SchemaPolicy::new(2, -1).is_err());
    assert!(SchemaPolicy::new(0, 0).is_err());
    assert!(SchemaPolicy::new(-1, -1).is_err());
}

#[test]
fn the_refusal_of_a_policy_names_both_majors() {
    assert_eq!(
        PolicyError {
            current: 1,
            oldest_read: 2
        }
        .to_string(),
        "a policy needs 1 <= oldest read <= current, and this one has oldest read 2 and current 1"
    );
}
