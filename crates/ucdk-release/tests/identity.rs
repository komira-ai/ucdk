// SPDX-License-Identifier: Apache-2.0
//! The comparison of the digests two sides expose for a file: field by field, and never equal by
//! having nothing to compare.

use ucdk_release::identity::{ContentIdentity, IdentityMatch, identity_matches};

use IdentityMatch::{Match, Mismatch, NoCommonField};

const SHA256: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER256: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SRI: &str = "sha512-AAAA";
const OTHER_SRI: &str = "sha512-BBBB";
const SHA1: &str = "cccccccccccccccccccccccccccccccccccccccc";
const OTHER1: &str = "dddddddddddddddddddddddddddddddddddddddd";

fn all(sha256: &str, sri: &str, sha1: &str) -> ContentIdentity {
    ContentIdentity::none()
        .with_sha256(sha256)
        .with_sha512(sri)
        .with_sha1(sha1)
}

#[test]
fn the_same_field_equal_is_a_match_and_another_value_is_a_mismatch() {
    let ours = ContentIdentity::of_sha256(SHA256);
    assert_eq!(
        identity_matches(&ours, &ContentIdentity::of_sha256(SHA256)),
        Match
    );
    assert_eq!(
        identity_matches(&ours, &ContentIdentity::of_sha256(OTHER256)),
        Mismatch
    );
    let ours = ContentIdentity::none().with_sha512(SRI);
    assert_eq!(
        identity_matches(&ours, &ContentIdentity::none().with_sha512(SRI)),
        Match
    );
    assert_eq!(
        identity_matches(&ours, &ContentIdentity::none().with_sha512(OTHER_SRI)),
        Mismatch
    );
    let ours = ContentIdentity::none().with_sha1(SHA1);
    assert_eq!(
        identity_matches(&ours, &ContentIdentity::none().with_sha1(SHA1)),
        Match
    );
    assert_eq!(
        identity_matches(&ours, &ContentIdentity::none().with_sha1(OTHER1)),
        Mismatch
    );
}

#[test]
fn identities_with_disjoint_fields_are_no_common_field_whichever_side_holds_which() {
    // One side only a digest, the other only the other two: nothing is exposed on both sides.
    let digest_only = ContentIdentity::of_sha256(SHA256);
    let others_only = ContentIdentity::none().with_sha512(SRI).with_sha1(SHA1);
    assert_eq!(identity_matches(&digest_only, &others_only), NoCommonField);
    assert_eq!(identity_matches(&others_only, &digest_only), NoCommonField);
    // A sha512 is never compared with a sha256 or a sha1, whatever the text.
    let a = ContentIdentity::none().with_sha512(SHA256);
    let b = ContentIdentity::of_sha256(SHA256);
    assert_eq!(identity_matches(&a, &b), NoCommonField);
}

#[test]
fn an_identity_that_exposes_nothing_is_no_common_field_with_anything_and_with_itself() {
    let nothing = ContentIdentity::none();
    assert!(nothing.exposes_nothing());
    assert_eq!(identity_matches(&nothing, &nothing), NoCommonField);
    assert_eq!(
        identity_matches(&nothing, &all(SHA256, SRI, SHA1)),
        NoCommonField
    );
    assert_eq!(
        identity_matches(&all(SHA256, SRI, SHA1), &nothing),
        NoCommonField
    );
}

#[test]
fn an_empty_value_is_not_exposed_and_never_equal_to_another_empty_one() {
    let empty = ContentIdentity::of_sha256("");
    assert_eq!(empty, ContentIdentity::none());
    assert!(empty.exposes_nothing());
    assert_eq!(identity_matches(&empty, &empty), NoCommonField);
    assert_eq!(
        identity_matches(&empty, &ContentIdentity::of_sha256(SHA256)),
        NoCommonField
    );
    // Setting a field to empty takes the field back.
    let cleared = all(SHA256, SRI, SHA1)
        .with_sha256("")
        .with_sha512("")
        .with_sha1("");
    assert!(cleared.exposes_nothing());
    assert!(!ContentIdentity::of_sha256(SHA256).exposes_nothing());
    assert!(!ContentIdentity::none().with_sha512(SRI).exposes_nothing());
    assert!(!ContentIdentity::none().with_sha1(SHA1).exposes_nothing());
}

#[test]
fn a_field_one_side_alone_exposes_is_a_witness_and_not_a_difference() {
    // We hold all three; a registry that exposes the sha512 and the sha1 (and a sha256 that is
    // not exposed) agrees on the two it exposes.
    let ours = all(SHA256, SRI, SHA1);
    let theirs = ContentIdentity::none().with_sha512(SRI).with_sha1(SHA1);
    assert_eq!(identity_matches(&ours, &theirs), Match);
    assert_eq!(identity_matches(&theirs, &ours), Match);
}

#[test]
fn one_differing_common_field_is_a_mismatch_even_when_the_others_agree() {
    let ours = all(SHA256, SRI, SHA1);
    for theirs in [
        all(OTHER256, SRI, SHA1),
        all(SHA256, OTHER_SRI, SHA1),
        all(SHA256, SRI, OTHER1),
    ] {
        assert_eq!(identity_matches(&ours, &theirs), Mismatch);
        assert_eq!(identity_matches(&theirs, &ours), Mismatch);
    }
}

#[test]
fn fields_are_compared_byte_for_byte_with_no_case_folding() {
    let ours = ContentIdentity::of_sha256(SHA256);
    let upper = ContentIdentity::of_sha256(&SHA256.to_uppercase());
    assert_eq!(identity_matches(&ours, &upper), Mismatch);
}
