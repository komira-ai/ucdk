// SPDX-License-Identifier: Apache-2.0
//! The length-framed hash: what is fed, in which bytes, and what that keeps apart.

use sha2::{Digest as _, Sha256};
use ucdk_core::{Framed, Hash256};

const DOMAIN: &[u8] = b"ucdk/example/1\n";

#[test]
fn a_field_is_its_length_in_eight_big_endian_bytes_then_its_bytes() {
    // The expected hash is made here by hand, byte by byte, from the rule in the doc comment of
    // `Framed`, so the framing is held independently of the code that writes it.
    let mut by_hand = Sha256::new();
    by_hand.update(DOMAIN);
    by_hand.update([0, 0, 0, 0, 0, 0, 0, 5]);
    by_hand.update(b"hello");
    by_hand.update([0, 0, 0, 0, 0, 0, 0, 0]);
    by_hand.update([0, 0, 0, 0, 0, 0, 0, 2]);
    by_hand.update(b"ok");
    let wanted = Hash256::from_sha256(&by_hand.finalize().into());
    let got = Framed::new(DOMAIN)
        .field(b"hello")
        .field(b"")
        .field(b"ok")
        .finish();
    assert_eq!(got, wanted);
    // And the text of the first example is pinned, so a change of the rule is a visible one.
    let pinned = Framed::new(DOMAIN).field(b"a").finish();
    assert_eq!(
        pinned.as_str(),
        "648d4dba9bf0294991fcfecde31a342546976478edc5e49ea06fb27df16d1c70"
    );
}

#[test]
fn two_fields_do_not_run_together() {
    let split = |a: &[u8], b: &[u8]| Framed::new(DOMAIN).field(a).field(b).finish();
    assert_ne!(split(b"ab", b"c"), split(b"a", b"bc"));
    assert_ne!(split(b"", b"abc"), split(b"abc", b""));
    assert_eq!(split(b"ab", b"c"), split(b"ab", b"c"));
}

#[test]
fn a_different_domain_gives_a_different_hash() {
    let under = |domain: &[u8]| Framed::new(domain).field(b"same").finish();
    assert_ne!(under(b"ucdk/one/1\n"), under(b"ucdk/two/1\n"));
    assert_ne!(under(b"ucdk/one/1\n"), under(b"ucdk/one/2\n"));
}
