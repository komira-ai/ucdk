// SPDX-License-Identifier: Apache-2.0
//! What the types that hold a credential promise: the identity token, the envelope that carries
//! mopm's token, the GitHub identity source that holds the request token and the `Secret` of a
//! bearer header are wiped by the `zeroize` crate when they are dropped, and none of them can be
//! copied or printed. That the marker is backed by `Zeroizing` storage is held by unit tests
//! inside the crate, next to each field.

// The probe's explicit borrow is what makes the bound-less fallback the second candidate.
#![allow(clippy::needless_borrow)]

mod common;
mod support;

use std::marker::PhantomData;

use ucdk_release::codec::GrantPhase;
use ucdk_release_mopm::{Envelope, GithubIdentity, IdentityToken, Secret};
use ucdk_release_proto::v1::{ArtifactType, Coordinate};
use zeroize::Zeroizing;

/// Answers at run time whether a type satisfies a bound, by method resolution: the by-reference
/// method of the trait with the bound wins when the bound holds, and the fallback is taken when
/// it does not. It needs no unsafe code and no nightly feature.
struct Probe<T>(PhantomData<T>);

macro_rules! probe {
    ($name:ident, $bound:path) => {
        mod $name {
            use super::Probe;

            pub trait Holds {
                fn holds(&self) -> bool {
                    true
                }
            }
            impl<T: $bound> Holds for Probe<T> {}

            pub trait Fails {
                fn holds(&self) -> bool {
                    false
                }
            }
            impl<T> Fails for &Probe<T> {}
        }
    };
}

probe!(wipes, ::zeroize::ZeroizeOnDrop);
probe!(clones, ::std::clone::Clone);
probe!(debugs, ::std::fmt::Debug);
probe!(displays, ::std::fmt::Display);

type Github = GithubIdentity<support::Script>;

#[test]
fn each_holder_of_a_credential_is_wiped_by_the_zeroize_crate_on_drop() {
    use wipes::{Fails, Holds};
    assert!((&Probe::<IdentityToken>(PhantomData)).holds());
    assert!((&Probe::<Envelope>(PhantomData)).holds());
    assert!((&Probe::<Github>(PhantomData)).holds());
    assert!((&Probe::<Secret>(PhantomData)).holds());
    // The probe tells a type that is not wiped from one that is.
    assert!((&Probe::<Zeroizing<Vec<u8>>>(PhantomData)).holds());
    assert!(!(&Probe::<Vec<u8>>(PhantomData)).holds());
}

#[test]
fn no_holder_of_a_credential_can_be_cloned() {
    use clones::{Fails, Holds};
    assert!(!(&Probe::<IdentityToken>(PhantomData)).holds());
    assert!(!(&Probe::<Envelope>(PhantomData)).holds());
    assert!(!(&Probe::<Github>(PhantomData)).holds());
    assert!(!(&Probe::<Secret>(PhantomData)).holds());
    assert!((&Probe::<Vec<u8>>(PhantomData)).holds());
}

#[test]
fn no_holder_of_a_credential_can_be_printed_for_debugging() {
    use debugs::{Fails, Holds};
    assert!(!(&Probe::<IdentityToken>(PhantomData)).holds());
    assert!(!(&Probe::<Envelope>(PhantomData)).holds());
    assert!(!(&Probe::<Github>(PhantomData)).holds());
    assert!(!(&Probe::<Secret>(PhantomData)).holds());
    assert!((&Probe::<Vec<u8>>(PhantomData)).holds());
}

#[test]
fn no_holder_of_a_credential_can_be_displayed() {
    use displays::{Fails, Holds};
    assert!(!(&Probe::<IdentityToken>(PhantomData)).holds());
    assert!(!(&Probe::<Envelope>(PhantomData)).holds());
    assert!(!(&Probe::<Github>(PhantomData)).holds());
    assert!(!(&Probe::<Secret>(PhantomData)).holds());
    assert!((&Probe::<String>(PhantomData)).holds());
}

fn coordinate(name: &str) -> Coordinate {
    Coordinate {
        channel: "mojo".to_owned(),
        r#type: ArtifactType::Conda as i32,
        name: name.to_owned(),
        version: "1.2.3".to_owned(),
        platform: "linux-64".to_owned(),
    }
}

/// `Vec::with_capacity` gives exactly the capacity asked for, so a buffer whose capacity equals
/// its length was sized once and never grew: no reallocation left a copy of the token in freed
/// memory. The coordinate counts cross a change in the number of digits of their framing.
#[test]
fn an_encoded_envelope_is_sized_once_and_wiped_on_drop() {
    for count in [0, 1, 9, 10, 11, 100] {
        for phase in [GrantPhase::Stage, GrantPhase::Commit, GrantPhase::Read] {
            let token = if phase == GrantPhase::Read {
                Vec::new()
            } else {
                b"x".repeat(count + 7)
            };
            let coordinates = (0..count).map(|i| coordinate(&format!("pkg{i}"))).collect();
            let envelope = Envelope::new(
                phase,
                count % 2 == 1,
                "https://registry.example.com/api/v1".to_owned(),
                "ci:gh-repo:9001".to_owned(),
                coordinates,
                token,
            )
            .unwrap();
            let encoded: Zeroizing<Vec<u8>> = envelope.encode();
            assert_eq!(encoded.capacity(), encoded.len(), "{count} {phase:?}");
            assert!(Envelope::decode(&encoded).is_ok(), "{count} {phase:?}");
        }
    }
}
