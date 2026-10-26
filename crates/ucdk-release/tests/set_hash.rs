// SPDX-License-Identifier: Apache-2.0
//! The set hash: what it covers, what it leaves out, and its text pinned.

use ucdk_core::{Framed, Hash256};
use ucdk_release::hashes::set_hash;
use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{
    ArtifactType, FileMeta, Manifest, Member, OciMeta, ProducedBy, Producer, ReleaseSet, Stamped,
};

const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";
const SHA_TOOL: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SHA_IMAGE: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn tool() -> Manifest {
    Manifest {
        schema_version: 1,
        r#type: ArtifactType::File as i32,
        name: "tool".to_owned(),
        version: "1.2.3".to_owned(),
        platform: "linux-x86_64".to_owned(),
        file: "tool.bin".to_owned(),
        sha256: SHA_TOOL.to_owned(),
        size: 100,
        stamp: Some(Stamped {
            build_number: 41,
            source_commit: "abc123".to_owned(),
        }),
        metadata: Some(Metadata::FileMeta(FileMeta {
            labels: [("requires".to_owned(), "lib".to_owned())].into(),
        })),
    }
}

fn image() -> Manifest {
    Manifest {
        schema_version: 1,
        r#type: ArtifactType::Oci as i32,
        name: "image".to_owned(),
        version: "1.2.3".to_owned(),
        platform: "linux-x86_64".to_owned(),
        file: "image".to_owned(),
        sha256: SHA_IMAGE.to_owned(),
        size: 2000,
        stamp: Some(Stamped {
            build_number: 41,
            source_commit: "abc123".to_owned(),
        }),
        metadata: Some(Metadata::Oci(OciMeta {
            os: "linux".to_owned(),
            cpu: "x86_64".to_owned(),
        })),
    }
}

fn set_of(members: Vec<Manifest>) -> ReleaseSet {
    ReleaseSet {
        schema_version: 1,
        revision: REVISION.to_owned(),
        platform: "linux-x86_64".to_owned(),
        members: members
            .into_iter()
            .map(|m| Member { manifest: Some(m) })
            .collect(),
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

fn sample() -> ReleaseSet {
    set_of(vec![image(), tool()])
}

/// Edits the manifest named `name` of the sample set.
fn edited(name: &str, edit: impl FnOnce(&mut Manifest)) -> Hash256 {
    let mut set = sample();
    let manifest = set
        .members
        .iter_mut()
        .filter_map(|m| m.manifest.as_mut())
        .find(|m| m.name == name)
        .expect("the member");
    edit(manifest);
    set_hash(&set)
}

#[test]
fn golden_text() {
    // The hash written out field by field, from the definition and not from the code: the tag,
    // then each field, each framed by its length. The members are in name order, and a number
    // is its decimal text.
    let fields = [
        REVISION,
        "linux-x86_64",
        "build",
        "build",
        "2",
        // image
        "image",
        "1.2.3",
        "2",
        "linux-x86_64",
        "image",
        "2000",
        SHA_IMAGE,
        "41",
        "abc123",
        "11",
        "linux",
        "x86_64",
        // tool
        "tool",
        "1.2.3",
        "1",
        "linux-x86_64",
        "tool.bin",
        "100",
        SHA_TOOL,
        "41",
        "abc123",
        "10",
        "1",
        "requires",
        "lib",
    ];
    let by_hand = fields
        .iter()
        .fold(Framed::new(b"ucdk/release-set/1\n"), |h, f| {
            h.field(f.as_bytes())
        })
        .finish();
    assert_eq!(set_hash(&sample()), by_hand);
    assert_eq!(
        set_hash(&sample()).as_str(),
        "29886c571ec1fae8c9d3bf5437683532c8eab5de889a41691110c9f327287198",
        "the hash of the sample set is a promise: a change here orphans every stored set"
    );
}

#[test]
fn ignores_the_run_and_attempt_that_built_it() {
    let mut other = sample();
    other.produced_by = Some(ProducedBy {
        run_id: "run-2".to_owned(),
        attempt: 9,
    });
    assert_eq!(set_hash(&other), set_hash(&sample()));
    other.produced_by = None;
    assert_eq!(set_hash(&other), set_hash(&sample()));
    // Nor does it cover the hash the document carries, or the schema version of the document.
    other.set_hash = "anything".to_owned();
    other.schema_version = 7;
    assert_eq!(set_hash(&other), set_hash(&sample()));
}

#[test]
fn changes_with_the_producing_stage_and_environment() {
    let base = set_hash(&sample());
    let mut stage = sample();
    stage.producer = Some(Producer {
        stage: "check".to_owned(),
        environment: "build".to_owned(),
    });
    let mut environment = sample();
    environment.producer = Some(Producer {
        stage: "build".to_owned(),
        environment: "review".to_owned(),
    });
    let mut none = sample();
    none.producer = None;
    let all = [set_hash(&stage), set_hash(&environment), set_hash(&none)];
    assert!(all.iter().all(|h| *h != base));
    assert_ne!(all[0], all[1]);
    assert_ne!(all[1], all[2]);
}

#[test]
fn file_and_size_change_the_hash() {
    let base = set_hash(&sample());
    assert_ne!(edited("tool", |m| m.file = "other.bin".to_owned()), base);
    assert_ne!(edited("tool", |m| m.size += 1), base);
}

#[test]
fn changes_with_revision_and_platform() {
    let base = set_hash(&sample());
    let mut revision = sample();
    revision.revision = "f".repeat(40);
    let mut platform = sample();
    platform.platform = "linux-aarch64".to_owned();
    assert_ne!(set_hash(&revision), base);
    assert_ne!(set_hash(&platform), base);
    assert_ne!(set_hash(&revision), set_hash(&platform));
}

#[test]
fn member_order_does_not_matter() {
    assert_eq!(
        set_hash(&set_of(vec![tool(), image()])),
        set_hash(&set_of(vec![image(), tool()]))
    );
}

#[test]
fn every_manifest_field_changes_the_hash() {
    let base = set_hash(&sample());
    let changes: Vec<(&str, Hash256)> = vec![
        ("name", edited("tool", |m| m.name = "tool2".to_owned())),
        ("version", edited("tool", |m| m.version = "2".to_owned())),
        (
            "type",
            edited("tool", |m| m.r#type = ArtifactType::Npm as i32),
        ),
        (
            "platform",
            edited("tool", |m| m.platform = "any".to_owned()),
        ),
        ("file", edited("tool", |m| m.file = "x".to_owned())),
        ("size", edited("tool", |m| m.size = 1)),
        (
            "sha256",
            edited("tool", |m| m.sha256 = SHA_IMAGE.to_owned()),
        ),
        (
            "build_number",
            edited("tool", |m| {
                m.stamp.as_mut().expect("stamp").build_number = 42;
            }),
        ),
        (
            "source_commit",
            edited("tool", |m| {
                m.stamp.as_mut().expect("stamp").source_commit = "d".to_owned();
            }),
        ),
        ("stamp", edited("tool", |m| m.stamp = None)),
    ];
    for (field, hash) in &changes {
        assert_ne!(*hash, base, "{field}");
    }
    let mut seen: Vec<&str> = changes.iter().map(|(_, h)| h.as_str()).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        changes.len(),
        "two different edits shared a hash"
    );
}

#[test]
fn every_metadata_field_changes_the_hash() {
    let base = set_hash(&sample());
    let changes: Vec<(&str, Hash256)> = vec![
        (
            "label value",
            edited("tool", |m| {
                m.metadata = Some(Metadata::FileMeta(FileMeta {
                    labels: [("requires".to_owned(), "other".to_owned())].into(),
                }));
            }),
        ),
        (
            "label key",
            edited("tool", |m| {
                m.metadata = Some(Metadata::FileMeta(FileMeta {
                    labels: [("needs".to_owned(), "lib".to_owned())].into(),
                }));
            }),
        ),
        (
            "extra label",
            edited("tool", |m| {
                m.metadata = Some(Metadata::FileMeta(FileMeta {
                    labels: [
                        ("requires".to_owned(), "lib".to_owned()),
                        ("zone".to_owned(), "a".to_owned()),
                    ]
                    .into(),
                }));
            }),
        ),
        (
            "no labels",
            edited("tool", |m| {
                m.metadata = Some(Metadata::FileMeta(FileMeta::default()));
            }),
        ),
        ("no metadata", edited("tool", |m| m.metadata = None)),
        (
            "arm",
            edited("tool", |m| {
                m.metadata = Some(Metadata::Oci(OciMeta {
                    os: "requires".to_owned(),
                    cpu: "lib".to_owned(),
                }));
            }),
        ),
        (
            "os",
            edited("image", |m| {
                m.metadata = Some(Metadata::Oci(OciMeta {
                    os: "darwin".to_owned(),
                    cpu: "x86_64".to_owned(),
                }));
            }),
        ),
        (
            "cpu",
            edited("image", |m| {
                m.metadata = Some(Metadata::Oci(OciMeta {
                    os: "linux".to_owned(),
                    cpu: "arm64".to_owned(),
                }));
            }),
        ),
        ("oci to none", edited("image", |m| m.metadata = None)),
    ];
    for (field, hash) in &changes {
        assert_ne!(*hash, base, "{field}");
    }
    let mut seen: Vec<&str> = changes.iter().map(|(_, h)| h.as_str()).collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        changes.len(),
        "two different edits shared a hash"
    );
}

#[test]
fn labels_are_hashed_in_key_order_whatever_order_they_were_given() {
    let a = edited("tool", |m| {
        m.metadata = Some(Metadata::FileMeta(FileMeta {
            labels: [
                ("b".to_owned(), "2".to_owned()),
                ("a".to_owned(), "1".to_owned()),
            ]
            .into(),
        }));
    });
    let b = edited("tool", |m| {
        m.metadata = Some(Metadata::FileMeta(FileMeta {
            labels: [
                ("a".to_owned(), "1".to_owned()),
                ("b".to_owned(), "2".to_owned()),
            ]
            .into(),
        }));
    });
    assert_eq!(a, b);
}

#[test]
fn a_member_with_no_manifest_hashes_as_an_empty_one() {
    let mut set = sample();
    set.members.push(Member { manifest: None });
    let mut empty = sample();
    empty.members.push(Member {
        manifest: Some(Manifest::default()),
    });
    assert_eq!(set_hash(&set), set_hash(&empty));
    assert_ne!(set_hash(&set), set_hash(&sample()));
}

#[test]
fn two_member_lists_that_concatenate_alike_hash_differently() {
    // The labels of one member and the fields of the next never run together.
    let one = set_of(vec![
        edited_manifest("a", &[("k", "v")]),
        edited_manifest("b", &[]),
    ]);
    let two = set_of(vec![
        edited_manifest("a", &[]),
        edited_manifest("b", &[("k", "v")]),
    ]);
    assert_ne!(set_hash(&one), set_hash(&two));
}

fn edited_manifest(name: &str, labels: &[(&str, &str)]) -> Manifest {
    let mut m = tool();
    m.name = name.to_owned();
    m.metadata = Some(Metadata::FileMeta(FileMeta {
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }));
    m
}
