// SPDX-License-Identifier: Apache-2.0
//! The grant-source kit run against `FakeGrantSource`: each scenario is a test of the fake.

#[macro_use]
mod rigs;

use rigs::grant_source::FakeGrantRig;
use rigs::secret::{framed, texts};
use ucdk_release::codec::GrantPhase;
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::kit::{
    GrantSetup, GrantSourceRig, coordinate, federated_repository, grant_source_scenarios,
    matching_job, run_grant_source_kit,
};
use ucdk_release_seams::{GrantRequest, GrantSource};

kit_tests!(
    FakeGrantRig, [FakeGrantRig::conforming()], grant_source_scenarios;
    an_anonymous_identity_needs_no_exchange,
    a_grant_covers_exactly_the_coordinates_asked,
    a_differing_claim_is_refused_before_any_exchange,
    an_exceptional_run_presents_the_exceptional_subject,
    a_used_identity_token_is_refused_the_second_time,
    an_exchange_after_expiry_gives_a_new_grant,
    a_missing_scope_is_denied,
    a_read_grant_carries_the_read_scope_and_covers_exactly_the_coordinates_asked,
    an_unreachable_issuer_is_unreachable_and_spends_nothing,
    an_expiry_is_reported_as_the_issuer_stated_it,
    more_coordinates_than_the_limit_are_refused_before_the_exchange,
    a_grant_is_never_printed_cloned_or_logged,
);

#[test]
fn the_whole_kit_passes_on_the_fake() {
    run_grant_source_kit(&FakeGrantRig::conforming()).assert_conforms();
}

/// What the rig reads out of the secret of a grant is read by the lengths the secret carries:
/// a name that holds a newline, a `/`, a `:` or a run of digits and a colon is one name, so a
/// rig that split the secret on a character would read another coordinate.
#[test]
fn the_rig_reads_back_names_that_hold_every_separator() {
    let names = ["a/b", "line\nbreak", "7:x", "a:1:b", "", "\u{e9}/\n:"];
    let coords: Vec<Coordinate> = names
        .iter()
        .map(|name| Coordinate {
            channel: format!("{name}/ch"),
            version: format!("v\n{name}"),
            platform: format!("{name}:{name}"),
            ..coordinate(name)
        })
        .collect();
    let rig = FakeGrantRig::conforming();
    let mut source = rig.source(&GrantSetup {
        job: matching_job(),
        limit: usize::MAX,
        lifetime_ms: 600_000,
        issuer_offset_ms: 0,
    });
    let repository = federated_repository();
    let grant = source
        .grant(&GrantRequest {
            repository: &repository,
            phase: GrantPhase::Stage,
            coordinates: &coords,
            exceptional: false,
        })
        .expect("the issuer admits the job");
    assert_eq!(rig.content(&source, &grant).coordinates, coords);
}

/// The two helpers of the rig that read and write a secret agree with each other and with the
/// framing of the fake: each text is its length, a colon and the text.
#[test]
fn the_secret_helpers_frame_and_read_whole_texts_only() {
    let all = [
        "a".to_owned(),
        "b:c".to_owned(),
        String::new(),
        "d\ne".to_owned(),
    ];
    assert_eq!(framed(&all), b"1:a3:b:c0:3:d\ne");
    assert_eq!(texts(&framed(&all)), all);
    assert_eq!(texts(b""), Vec::<String>::new());
    // Not a run of whole texts: no length, a length that is not a number, a text cut short.
    for malformed in [&b"abc"[..], b"x:y", b"5:ab", b"1:a2:b", b"1:a3"] {
        assert_eq!(texts(malformed), Vec::<String>::new(), "{malformed:?}");
    }
}
