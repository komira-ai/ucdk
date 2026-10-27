// SPDX-License-Identifier: Apache-2.0
//! The fake source: scripted stamps and ancestry, and the faults of a checkout.

use ucdk_release_fake::{FakeSource, SourceFault};
use ucdk_release_proto::v1::{Reason, Tri};
use ucdk_release_seams::{SeamError, Source, SourceStamp};

fn source() -> FakeSource {
    let mut source = FakeSource::new();
    source.stamped("rev1", 7, "abc");
    source.ancestor("a", "b", Tri::Yes);
    source.ancestor("b", "a", Tri::No);
    source
}

#[test]
fn a_scripted_revision_stamps_and_an_unknown_one_is_refused() {
    let source = source();
    assert_eq!(
        source.stamp("rev1"),
        Ok(SourceStamp {
            build_number: 7,
            source_commit: "abc".to_owned()
        })
    );
    assert_eq!(
        source.stamp("other"),
        Err(SeamError::Refused(Reason::Source))
    );
}

#[test]
fn ancestry_is_the_scripted_tri_state_and_an_unscripted_pair_is_unknown() {
    let source = source();
    assert_eq!(source.is_ancestor("a", "b"), Ok(Tri::Yes));
    assert_eq!(source.is_ancestor("b", "a"), Ok(Tri::No));
    assert_eq!(source.is_ancestor("a", "c"), Ok(Tri::Unknown));
}

#[test]
fn a_shallow_dirty_or_mismatched_checkout_refuses_the_stamp() {
    for fault in [
        SourceFault::Shallow,
        SourceFault::Dirty,
        SourceFault::HeadMismatch,
    ] {
        let mut source = source();
        source.fault(fault);
        assert_eq!(
            source.stamp("rev1"),
            Err(SeamError::Refused(Reason::Source))
        );
        assert_eq!(source.is_ancestor("a", "b"), Ok(Tri::Yes));
    }
}

#[test]
fn a_source_that_cannot_tell_answers_unknown_for_both_questions() {
    let mut source = source();
    source.fault(SourceFault::CannotTell);
    assert!(matches!(source.stamp("rev1"), Err(SeamError::Unknown(_))));
    assert_eq!(source.is_ancestor("a", "b"), Ok(Tri::Unknown));
}
