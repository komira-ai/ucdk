// SPDX-License-Identifier: Apache-2.0
//! The set document: written sealed, read back with every check a reader makes.

use prost::Message;
use ucdk_release::hashes::set_hash;
use ucdk_release::set_document::{SET_SCHEMA, SetError, encode, read, seal};
use ucdk_release_proto::v1::{
    ArtifactType, Manifest, Member, ProducedBy, Producer, Reason, ReleaseSet,
};

fn member(name: &str) -> Member {
    Member {
        manifest: Some(Manifest {
            schema_version: 1,
            r#type: ArtifactType::File as i32,
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            platform: "any".to_owned(),
            file: format!("{name}.bin"),
            sha256: "a".repeat(64),
            size: 3,
            ..Manifest::default()
        }),
    }
}

fn unsealed(names: &[&str]) -> ReleaseSet {
    ReleaseSet {
        schema_version: 0,
        revision: "0123456789abcdef".to_owned(),
        platform: "any".to_owned(),
        members: names.iter().map(|n| member(n)).collect(),
        set_hash: String::new(),
        producer: Some(Producer {
            stage: "build".to_owned(),
            environment: "build".to_owned(),
        }),
        produced_by: Some(ProducedBy {
            run_id: "run-1".to_owned(),
            attempt: 1,
        }),
    }
}

#[test]
fn round_trips_and_recomputes_its_hash() {
    let sealed = seal(unsealed(&["tool", "image"]));
    assert_eq!(sealed.schema_version, SET_SCHEMA);
    let names: Vec<String> = sealed
        .members
        .iter()
        .map(|m| m.manifest.clone().expect("manifest").name)
        .collect();
    assert_eq!(
        names,
        ["image", "tool"],
        "sealing sorts the members by name"
    );
    assert_eq!(sealed.set_hash, set_hash(&sealed).to_string());
    let bytes = encode(&sealed);
    assert_eq!(read(&bytes), Ok(sealed.clone()));
    // The canonical bytes are those of the message itself.
    assert_eq!(bytes, sealed.encode_to_vec());
    // The run and attempt are kept in the document and are not what its hash is of.
    let again = seal(ReleaseSet {
        produced_by: Some(ProducedBy {
            run_id: "run-2".to_owned(),
            attempt: 4,
        }),
        ..unsealed(&["tool", "image"])
    });
    assert_eq!(again.set_hash, sealed.set_hash);
    assert_eq!(
        read(&encode(&again)).expect("read").produced_by,
        again.produced_by
    );
}

#[test]
fn a_differing_set_hash_is_refused() {
    let mut sealed = seal(unsealed(&["tool"]));
    let computed = sealed.set_hash.clone();
    sealed.set_hash = "0".repeat(64);
    let err = read(&sealed.encode_to_vec()).expect_err("a differing hash");
    assert_eq!(
        err,
        SetError::Hash {
            stored: "0".repeat(64),
            computed
        }
    );
    assert_eq!(err.reason(), Reason::SetHash);
    assert!(err.to_string().contains(&"0".repeat(64)), "{err}");
    // A member changed after sealing no longer matches the hash.
    let mut changed = seal(unsealed(&["tool"]));
    changed.members[0].manifest.as_mut().expect("manifest").size = 4;
    assert!(matches!(
        read(&changed.encode_to_vec()),
        Err(SetError::Hash { .. })
    ));
    // An empty stored hash is a differing one.
    let mut blank = seal(unsealed(&["tool"]));
    blank.set_hash.clear();
    assert!(matches!(
        read(&blank.encode_to_vec()),
        Err(SetError::Hash { .. })
    ));
}

#[test]
fn bytes_that_are_not_a_set_document_are_refused() {
    let err = read(&[0xFF, 0xFF, 0xFF]).expect_err("not a message");
    assert!(matches!(err, SetError::Decode(_)), "{err:?}");
    assert_eq!(err.reason(), Reason::SetHash);
    assert!(err.to_string().contains("does not decode"), "{err}");
}

#[test]
fn a_set_document_of_another_schema_is_refused() {
    for version in [0, 2] {
        let mut sealed = seal(unsealed(&["tool"]));
        sealed.schema_version = version;
        let err = read(&sealed.encode_to_vec()).expect_err("another schema");
        assert_eq!(err, SetError::Version(version));
        assert_eq!(err.reason(), Reason::SetHash);
        assert!(err.to_string().contains(&version.to_string()), "{err}");
    }
}

#[test]
fn a_member_with_no_manifest_is_refused() {
    let mut sealed = seal(unsealed(&["tool"]));
    sealed.members.push(Member { manifest: None });
    let err = read(&sealed.encode_to_vec()).expect_err("a member with no manifest");
    assert_eq!(err, SetError::MemberWithoutManifest(1));
    assert_eq!(err.reason(), Reason::SetMember);
    assert!(err.to_string().contains("member 1"), "{err}");
}

#[test]
fn members_must_be_sorted_by_name_each_name_once() {
    for names in [["tool", "image"], ["tool", "tool"]] {
        let mut set = unsealed(&names);
        set.schema_version = SET_SCHEMA;
        set.set_hash = set_hash(&set).to_string();
        let err = read(&set.encode_to_vec()).expect_err("not sorted");
        assert_eq!(err, SetError::MembersNotSorted(names[1].to_owned()));
        assert_eq!(err.reason(), Reason::SetMember);
        assert!(err.to_string().contains(names[1]), "{err}");
    }
}

#[test]
fn a_set_of_no_members_is_a_document_too() {
    let sealed = seal(unsealed(&[]));
    assert_eq!(read(&encode(&sealed)), Ok(sealed));
}

#[test]
fn sealing_a_member_with_no_manifest_sorts_it_first() {
    // A member with no manifest has an empty name, so it sorts before every other and is
    // refused on read; sealing does not hide it.
    let mut set = unsealed(&["tool"]);
    set.members.push(Member { manifest: None });
    let sealed = seal(set);
    assert!(sealed.members[0].manifest.is_none());
    assert_eq!(
        read(&encode(&sealed)),
        Err(SetError::MemberWithoutManifest(0))
    );
}
