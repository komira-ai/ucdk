// SPDX-License-Identifier: Apache-2.0
//! What a grant promises about its secret: it is wiped by the `zeroize` crate when the grant is
//! dropped, and the type offers no way to copy or print it.

// The probe's explicit borrow is what makes the bound-less fallback the second candidate.
#![allow(clippy::needless_borrow)]

use std::marker::PhantomData;

use zeroize::Zeroizing;

use ucdk_release_seams::Grant;

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

#[test]
fn a_grant_is_wiped_by_the_zeroize_crate_on_drop() {
    use wipes::{Fails, Holds};
    assert!((&Probe::<Grant>(PhantomData)).holds());
    // The probe tells a type that is not wiped from one that is.
    assert!((&Probe::<Zeroizing<Vec<u8>>>(PhantomData)).holds());
    assert!(!(&Probe::<Vec<u8>>(PhantomData)).holds());
}

#[test]
fn a_grant_cannot_be_cloned_or_printed() {
    {
        use clones::{Fails, Holds};
        assert!(!(&Probe::<Grant>(PhantomData)).holds());
        assert!((&Probe::<Vec<u8>>(PhantomData)).holds());
    }
    {
        use debugs::{Fails, Holds};
        assert!(!(&Probe::<Grant>(PhantomData)).holds());
        assert!((&Probe::<Vec<u8>>(PhantomData)).holds());
    }
    {
        use displays::{Fails, Holds};
        assert!(!(&Probe::<Grant>(PhantomData)).holds());
        assert!((&Probe::<String>(PhantomData)).holds());
    }
}

#[test]
fn a_grant_still_reads_back_what_it_was_issued_with() {
    let issued = Grant::issued(b"s3cret".to_vec(), 42);
    assert_eq!(issued.secret(), b"s3cret");
    assert_eq!(issued.expires_ms(), Some(42));
    assert!(!issued.is_none());
    let none = Grant::none();
    assert!(none.secret().is_empty());
    assert_eq!(none.expires_ms(), None);
    assert!(none.is_none());
}
