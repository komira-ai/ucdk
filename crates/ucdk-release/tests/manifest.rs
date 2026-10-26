// SPDX-License-Identifier: Apache-2.0
//! Composing the manifest of a member from the declaration, the stamp and the store entry.

mod common;

use common::{PLATFORM, file_artifact, pairs, stamp};
use sha2::{Digest as _, Sha256};
use ucdk_release::manifest::compose_manifest;
use ucdk_release::verify::{Check, Entry, verify_member};
use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{
    ArtifactDecl, ArtifactType, FileMeta, Manifest, OciMeta, Reason, Stamped,
};

const BYTES: &[u8] = b"the bytes of the tool";

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn file_decl() -> ArtifactDecl {
    ArtifactDecl {
        labels: pairs(&[("requires", "lib"), ("kind", "tool")]),
        ..file_artifact("tool", "//release:tool")
    }
}

fn file_entry() -> Entry {
    Entry::from([("tool.bin".to_owned(), BYTES.to_vec())])
}

#[test]
fn a_manifest_is_composed_from_the_declaration_the_stamp_and_the_located_output() {
    let manifest = compose_manifest(&file_decl(), PLATFORM, &stamp(), "tool.bin", &file_entry())
        .expect("composes");
    assert_eq!(
        manifest,
        Manifest {
            schema_version: 1,
            r#type: ArtifactType::File as i32,
            name: "tool".to_owned(),
            version: "1.2.3".to_owned(),
            platform: PLATFORM.to_owned(),
            file: "tool.bin".to_owned(),
            sha256: hex(BYTES),
            size: BYTES.len() as u64,
            stamp: Some(Stamped {
                build_number: 41,
                source_commit: "abc123".to_owned()
            }),
            metadata: Some(Metadata::FileMeta(FileMeta {
                labels: pairs(&[("requires", "lib"), ("kind", "tool")])
            })),
        }
    );
    // The check that follows a build accepts what was composed.
    assert_eq!(
        verify_member(&file_decl(), &manifest, &file_entry(), PLATFORM, &stamp()),
        Ok(())
    );
}

#[test]
fn the_digest_and_the_size_are_those_of_the_entry_and_the_platform_is_the_actions() {
    let entry = Entry::from([("tool.bin".to_owned(), b"other bytes".to_vec())]);
    let manifest =
        compose_manifest(&file_decl(), "any", &stamp(), "tool.bin", &entry).expect("composes");
    assert_eq!(manifest.sha256, hex(b"other bytes"));
    assert_eq!(manifest.size, 11);
    assert_eq!(manifest.platform, "any");
    // A file with no labels still carries its (empty) metadata arm, so the set hash sees one
    // shape for every file.
    let bare = compose_manifest(
        &file_artifact("tool", "//release:tool"),
        PLATFORM,
        &stamp(),
        "tool.bin",
        &file_entry(),
    )
    .expect("composes");
    assert_eq!(bare.metadata, Some(Metadata::FileMeta(FileMeta::default())));
    // The timestamp of the stamp is in no field of the manifest.
    let mut later = stamp();
    later.timestamp_ms += 1;
    assert_eq!(
        compose_manifest(&file_decl(), PLATFORM, &later, "tool.bin", &file_entry()),
        compose_manifest(&file_decl(), PLATFORM, &stamp(), "tool.bin", &file_entry())
    );
}

#[test]
fn a_file_the_entry_does_not_hold_is_refused() {
    let err = compose_manifest(
        &file_decl(),
        PLATFORM,
        &stamp(),
        "missing.bin",
        &file_entry(),
    )
    .expect_err("no such file");
    assert_eq!(
        (err.check, err.reason),
        (Check::EntryShape, Reason::SetMember)
    );
    assert!(err.message.contains("missing.bin"), "{}", err.message);
}

#[test]
fn a_type_that_names_no_identity_has_no_manifest() {
    for kind in [
        ArtifactType::Conda as i32,
        ArtifactType::Python as i32,
        ArtifactType::Npm as i32,
        ArtifactType::Unspecified as i32,
        99,
    ] {
        let decl = ArtifactDecl {
            r#type: kind,
            ..file_decl()
        };
        let err = compose_manifest(&decl, PLATFORM, &stamp(), "tool.bin", &file_entry())
            .expect_err("no identity");
        assert_eq!(
            (err.check, err.reason),
            (Check::Type, Reason::SetMember),
            "{kind}"
        );
    }
}

const CONFIG: &[u8] = br#"{"os":"linux","architecture":"x86_64"}"#;
const LAYER: &[u8] = b"layer bytes";

fn desc(bytes: &[u8]) -> String {
    format!(
        r#"{{"mediaType":"application/octet-stream","digest":"sha256:{}","size":{}}}"#,
        hex(bytes),
        bytes.len()
    )
}

fn image_entry() -> (Entry, Vec<u8>) {
    let manifest_json = format!(
        r#"{{"config":{},"layers":[{}]}}"#,
        desc(CONFIG),
        desc(LAYER)
    )
    .into_bytes();
    let index = format!(
        r#"{{"manifests":[{{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"sha256:{}","size":{}}}]}}"#,
        hex(&manifest_json),
        manifest_json.len()
    )
    .into_bytes();
    let blob = |bytes: &[u8]| format!("image/blobs/sha256/{}", hex(bytes));
    let entry = Entry::from([
        (
            "image/oci-layout".to_owned(),
            br#"{"imageLayoutVersion":"1.0.0"}"#.to_vec(),
        ),
        ("image/index.json".to_owned(), index),
        (blob(&manifest_json), manifest_json.clone()),
        (blob(CONFIG), CONFIG.to_vec()),
        (blob(LAYER), LAYER.to_vec()),
    ]);
    (entry, manifest_json)
}

#[test]
fn an_image_manifest_takes_its_digest_size_and_platform_from_the_layout() {
    let decl = ArtifactDecl {
        r#type: ArtifactType::Oci as i32,
        // Labels are for a file: an image's metadata is read from its layout.
        ..file_decl()
    };
    let (entry, manifest_json) = image_entry();
    let manifest = compose_manifest(&decl, PLATFORM, &stamp(), "image", &entry).expect("composes");
    assert_eq!(manifest.sha256, hex(&manifest_json));
    assert_eq!(
        manifest.size,
        (manifest_json.len() + CONFIG.len() + LAYER.len()) as u64
    );
    assert_eq!(manifest.file, "image");
    assert_eq!(manifest.platform, PLATFORM);
    assert_eq!(
        manifest.metadata,
        Some(Metadata::Oci(OciMeta {
            os: "linux".to_owned(),
            cpu: "x86_64".to_owned()
        }))
    );
    assert_eq!(
        verify_member(&decl, &manifest, &entry, PLATFORM, &stamp()),
        Ok(())
    );
    // A layout that cannot be read is refused by the reader of layouts, with its own check.
    let mut broken = entry.clone();
    broken.remove("image/index.json");
    let err = compose_manifest(&decl, PLATFORM, &stamp(), "image", &broken).expect_err("no index");
    assert_eq!(err.reason, Reason::SetMember);
}
