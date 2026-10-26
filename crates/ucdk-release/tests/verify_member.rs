// SPDX-License-Identifier: Apache-2.0
//! Verifying a member over an in-memory tree: one test per check, each breaking that check
//! alone in a member that otherwise verifies.

mod common;

use common::{PLATFORM, file_artifact, stamp};
use sha2::{Digest as _, Sha256};
use ucdk_release::verify::{Check, Entry, VerifyError, verify_member};
use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{ArtifactDecl, ArtifactType, Manifest, OciMeta, Reason, Stamped};

const BYTES: &[u8] = b"the bytes of the tool";

type Member = (ArtifactDecl, Manifest, Entry);

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn stamped() -> Option<Stamped> {
    Some(Stamped {
        build_number: 41,
        source_commit: "abc123".to_owned(),
    })
}

fn file_member() -> Member {
    let manifest = Manifest {
        schema_version: 1,
        r#type: ArtifactType::File as i32,
        name: "tool".to_owned(),
        version: "1.2.3".to_owned(),
        platform: PLATFORM.to_owned(),
        file: "tool.bin".to_owned(),
        sha256: hex(BYTES),
        size: BYTES.len() as u64,
        stamp: stamped(),
        metadata: None,
    };
    let entry = Entry::from([("tool.bin".to_owned(), BYTES.to_vec())]);
    (file_artifact("tool", "//release:tool"), manifest, entry)
}

fn verdict(member: &Member) -> Result<(), VerifyError> {
    verify_member(&member.0, &member.1, &member.2, PLATFORM, &stamp())
}

/// The check that refused, and the reason it carries.
fn refused(member: &Member) -> (Check, Reason) {
    let err = verdict(member).expect_err("the member should be refused");
    assert!(!err.to_string().is_empty());
    (err.check, err.reason)
}

#[test]
fn a_good_file_member_is_verified() {
    assert_eq!(verdict(&file_member()), Ok(()));
}

#[test]
fn name_must_equal_the_declaration() {
    for name in ["Tool", "tool ", "too", ""] {
        let mut m = file_member();
        m.1.name = name.to_owned();
        assert_eq!(refused(&m), (Check::Name, Reason::SetMember), "{name:?}");
    }
}

#[test]
fn type_must_equal_the_declaration() {
    let mut m = file_member();
    m.1.r#type = ArtifactType::Oci as i32;
    assert_eq!(refused(&m), (Check::Type, Reason::SetMember));
}

#[test]
fn a_type_that_names_no_identity_is_refused() {
    for kind in [
        ArtifactType::Conda,
        ArtifactType::Python,
        ArtifactType::Npm,
        ArtifactType::Unspecified,
    ] {
        let mut m = file_member();
        m.0.r#type = kind as i32;
        m.1.r#type = kind as i32;
        assert_eq!(refused(&m), (Check::Type, Reason::SetMember), "{kind:?}");
    }
    let mut unknown = file_member();
    unknown.0.r#type = 99;
    unknown.1.r#type = 99;
    assert_eq!(refused(&unknown), (Check::Type, Reason::SetMember));
}

#[test]
fn the_file_must_be_a_bare_name() {
    for file in ["", "a/b", "..", ".", "/abs"] {
        let mut m = file_member();
        m.1.file = file.to_owned();
        assert_eq!(
            refused(&m),
            (Check::EntryShape, Reason::SetMember),
            "{file:?}"
        );
    }
}

#[test]
fn the_entry_must_hold_the_file_the_manifest_names() {
    let mut m = file_member();
    m.1.file = "other.bin".to_owned();
    assert_eq!(refused(&m), (Check::EntryShape, Reason::SetMember));
    m.2.clear();
    assert_eq!(refused(&m), (Check::EntryShape, Reason::SetMember));
}

#[test]
fn no_other_path_may_exist() {
    let mut m = file_member();
    m.2.insert("extra.txt".to_owned(), b"x".to_vec());
    assert_eq!(refused(&m), (Check::NoOtherPath, Reason::SetMember));
    let mut nested = file_member();
    nested.2.insert("dir/extra".to_owned(), b"x".to_vec());
    assert_eq!(refused(&nested), (Check::NoOtherPath, Reason::SetMember));
}

#[test]
fn the_file_must_not_be_empty() {
    let mut m = file_member();
    m.2.insert("tool.bin".to_owned(), Vec::new());
    m.1.sha256 = hex(b"");
    m.1.size = 0;
    assert_eq!(refused(&m), (Check::NonEmpty, Reason::SetMember));
}

#[test]
fn the_identity_digest_is_recomputed_from_the_bytes() {
    let mut m = file_member();
    m.2.insert(
        "tool.bin".to_owned(),
        b"other bytes of the same length..".to_vec(),
    );
    m.1.size = 32;
    assert_eq!(refused(&m), (Check::Identity, Reason::SetMember));
    let mut text = file_member();
    text.1.sha256 = hex(BYTES).to_uppercase();
    assert_eq!(refused(&text), (Check::Identity, Reason::SetMember));
}

#[test]
fn the_size_is_recomputed_from_the_bytes() {
    let mut m = file_member();
    m.1.size += 1;
    assert_eq!(refused(&m), (Check::Size, Reason::SetMember));
}

#[test]
fn the_platform_is_the_runs_or_any() {
    let mut other = file_member();
    other.1.platform = "linux-aarch64".to_owned();
    assert_eq!(refused(&other), (Check::Platform, Reason::PlatformMismatch));
    let mut any = file_member();
    any.1.platform = "any".to_owned();
    assert_eq!(verdict(&any), Ok(()));
}

#[test]
fn the_version_equals_the_release_version() {
    let mut m = file_member();
    m.1.version = "9.9.9".to_owned();
    assert_eq!(refused(&m), (Check::Version, Reason::MemberVersion));
    m.1.version = String::new();
    assert_eq!(refused(&m), (Check::Version, Reason::MemberVersion));
}

#[test]
fn the_recorded_stamp_equals_the_builds() {
    let mut number = file_member();
    number.1.stamp = Some(Stamped {
        build_number: 40,
        source_commit: "abc123".to_owned(),
    });
    assert_eq!(refused(&number), (Check::Stamp, Reason::SetMember));
    let mut commit = file_member();
    commit.1.stamp = Some(Stamped {
        build_number: 41,
        source_commit: "other".to_owned(),
    });
    assert_eq!(refused(&commit), (Check::Stamp, Reason::SetMember));
    let mut none = file_member();
    none.1.stamp = None;
    assert_eq!(refused(&none), (Check::Stamp, Reason::SetMember));
}

#[test]
fn the_timestamp_of_the_build_is_not_checked_against_the_member() {
    let mut later = stamp();
    later.timestamp_ms += 1;
    let m = file_member();
    assert_eq!(verify_member(&m.0, &m.1, &m.2, PLATFORM, &later), Ok(()));
}

// ---- OCI ----------------------------------------------------------------------------------

const CONFIG: &[u8] = br#"{"os":"linux","architecture":"x86_64"}"#;
const LAYER: &[u8] = b"layer bytes";

fn blob_path(bytes: &[u8]) -> String {
    format!("image/blobs/sha256/{}", hex(bytes))
}

fn image_manifest_json() -> Vec<u8> {
    format!(
        r#"{{"config":{{"digest":"sha256:{}"}},"layers":[{{"digest":"sha256:{}"}}]}}"#,
        hex(CONFIG),
        hex(LAYER)
    )
    .into_bytes()
}

fn image_member() -> Member {
    let manifest_json = image_manifest_json();
    let index = format!(
        r#"{{"manifests":[{{"digest":"sha256:{}"}}]}}"#,
        hex(&manifest_json)
    );
    let size = (manifest_json.len() + CONFIG.len() + LAYER.len()) as u64;
    let entry = Entry::from([
        (
            "image/oci-layout".to_owned(),
            br#"{"imageLayoutVersion":"1.0.0"}"#.to_vec(),
        ),
        ("image/index.json".to_owned(), index.into_bytes()),
        (blob_path(&manifest_json), manifest_json.clone()),
        (blob_path(CONFIG), CONFIG.to_vec()),
        (blob_path(LAYER), LAYER.to_vec()),
    ]);
    let manifest = Manifest {
        schema_version: 1,
        r#type: ArtifactType::Oci as i32,
        name: "image".to_owned(),
        version: "1.2.3".to_owned(),
        platform: PLATFORM.to_owned(),
        file: "image".to_owned(),
        sha256: hex(&manifest_json),
        size,
        stamp: stamped(),
        metadata: Some(Metadata::Oci(OciMeta {
            os: "linux".to_owned(),
            cpu: "x86_64".to_owned(),
        })),
    };
    let decl = ArtifactDecl {
        r#type: ArtifactType::Oci as i32,
        ..file_artifact("image", "//release:image")
    };
    (decl, manifest, entry)
}

/// The image member with `path` of its entry replaced by `bytes`, or removed when `None`.
fn image_with(path: &str, bytes: Option<&[u8]>) -> Member {
    let mut m = image_member();
    match bytes {
        Some(b) => m.2.insert(path.to_owned(), b.to_vec()),
        None => m.2.remove(path),
    };
    m
}

#[test]
fn a_good_image_member_is_verified() {
    assert_eq!(verdict(&image_member()), Ok(()));
}

#[test]
fn every_path_of_an_image_entry_lies_under_the_layout_directory() {
    let mut m = image_member();
    m.2.insert("elsewhere/oci-layout".to_owned(), b"{}".to_vec());
    assert_eq!(refused(&m), (Check::NoOtherPath, Reason::SetMember));
    let mut top = image_member();
    top.2.insert("stray".to_owned(), b"x".to_vec());
    assert_eq!(refused(&top), (Check::NoOtherPath, Reason::SetMember));
}

#[test]
fn a_path_inside_the_layout_must_be_a_layout_path() {
    for path in [
        "image/notes.txt",
        "image/blobs/sha256/not-hex",
        "image/blobs/md5/abc",
        "image/blobs/sha256/",
    ] {
        let m = image_with(path, Some(b"x"));
        assert_eq!(
            refused(&m),
            (Check::NoOtherPath, Reason::SetMember),
            "{path}"
        );
    }
}

#[test]
fn a_blob_must_hash_to_its_name() {
    let blob = blob_path(LAYER);
    let m = image_with(&blob, Some(b"not the layer"));
    assert_eq!(refused(&m), (Check::BlobDigest, Reason::SetMember));
}

#[test]
fn an_image_needs_its_layout_file_and_its_index() {
    assert_eq!(
        refused(&image_with("image/oci-layout", None)),
        (Check::EntryShape, Reason::SetMember)
    );
    assert_eq!(
        refused(&image_with("image/index.json", None)),
        (Check::EntryShape, Reason::SetMember)
    );
    let mut empty = image_member();
    empty.2.clear();
    assert_eq!(refused(&empty), (Check::EntryShape, Reason::SetMember));
}

#[test]
fn the_layout_file_and_the_index_must_be_readable() {
    for (path, bytes) in [
        ("image/oci-layout", &b"not json"[..]),
        ("image/oci-layout", br#"{"other":1}"#),
        ("image/oci-layout", br#"{"imageLayoutVersion":1}"#),
        ("image/index.json", b"not json"),
    ] {
        assert_eq!(
            refused(&image_with(path, Some(bytes))),
            (Check::EntryShape, Reason::SetMember),
            "{path}"
        );
    }
}

#[test]
fn an_index_names_exactly_one_manifest() {
    for index in [
        &br#"{"manifests":[]}"#[..],
        br#"{}"#,
        br#"{"manifests":"x"}"#,
        br#"{"manifests":[{"digest":"sha256:aa"},{"digest":"sha256:bb"}]}"#,
    ] {
        assert_eq!(
            refused(&image_with("image/index.json", Some(index))),
            (Check::EntryShape, Reason::SetMember)
        );
    }
}

#[test]
fn a_digest_in_the_layout_is_a_sha256_digest_of_a_blob_the_layout_holds() {
    let manifest_json = image_manifest_json();
    let good = hex(&manifest_json);
    for digest in [
        "md5:abc".to_owned(),
        "sha256:xyz".to_owned(),
        format!("sha256:{}", "0".repeat(64)),
        good.clone(),
    ] {
        let index = format!(r#"{{"manifests":[{{"digest":"{digest}"}}]}}"#);
        assert_eq!(
            refused(&image_with("image/index.json", Some(index.as_bytes()))),
            (Check::EntryShape, Reason::SetMember),
            "{digest}"
        );
    }
    let no_digest = br#"{"manifests":[{}]}"#;
    assert_eq!(
        refused(&image_with("image/index.json", Some(no_digest))),
        (Check::EntryShape, Reason::SetMember)
    );
}

/// An image whose manifest blob is `json`, with the index and the member re-made to match.
fn image_with_manifest(json: &[u8]) -> Member {
    let mut m = image_member();
    m.2.insert(blob_path(json), json.to_vec());
    let index = format!(r#"{{"manifests":[{{"digest":"sha256:{}"}}]}}"#, hex(json));
    m.2.insert("image/index.json".to_owned(), index.into_bytes());
    m.1.sha256 = hex(json);
    m
}

#[test]
fn the_image_manifest_must_be_readable_and_name_a_config() {
    for json in [&b"not json"[..], br#"{"layers":[]}"#] {
        assert_eq!(
            refused(&image_with_manifest(json)),
            (Check::EntryShape, Reason::SetMember)
        );
    }
}

#[test]
fn the_blobs_the_manifest_reaches_must_be_in_the_layout() {
    let missing = format!(
        r#"{{"config":{{"digest":"sha256:{}"}},"layers":[]}}"#,
        "9".repeat(64)
    );
    assert_eq!(
        refused(&image_with_manifest(missing.as_bytes())),
        (Check::EntryShape, Reason::SetMember)
    );
    let layer = format!(
        r#"{{"config":{{"digest":"sha256:{}"}},"layers":[{{"digest":"sha256:{}"}}]}}"#,
        hex(CONFIG),
        "8".repeat(64)
    );
    assert_eq!(
        refused(&image_with_manifest(layer.as_bytes())),
        (Check::EntryShape, Reason::SetMember)
    );
}

#[test]
fn an_image_with_no_layers_has_the_size_of_its_manifest_and_config() {
    let json = format!(r#"{{"config":{{"digest":"sha256:{}"}}}}"#, hex(CONFIG));
    let mut m = image_with_manifest(json.as_bytes());
    m.1.size = (json.len() + CONFIG.len()) as u64;
    assert_eq!(verdict(&m), Ok(()));
}

#[test]
fn the_image_config_must_be_readable_and_name_a_platform() {
    for config in [
        &b"not json"[..],
        br#"{"os":"linux"}"#,
        br#"{"architecture":"x86_64"}"#,
    ] {
        let json = format!(r#"{{"config":{{"digest":"sha256:{}"}}}}"#, hex(config));
        let mut m = image_with_manifest(json.as_bytes());
        m.2.insert(blob_path(config), config.to_vec());
        assert_eq!(refused(&m), (Check::EntryShape, Reason::SetMember));
    }
}

#[test]
fn the_image_identity_is_the_digest_of_its_manifest() {
    let mut m = image_member();
    m.1.sha256 = hex(CONFIG);
    assert_eq!(refused(&m), (Check::Identity, Reason::SetMember));
}

#[test]
fn the_image_size_is_the_sum_of_the_blobs_the_manifest_reaches() {
    let mut m = image_member();
    m.1.size += 1;
    assert_eq!(refused(&m), (Check::Size, Reason::SetMember));
    // A blob nothing reaches is a layout path that hashes to its name, and is not counted.
    let mut spare = image_member();
    spare.2.insert(blob_path(b"spare"), b"spare".to_vec());
    assert_eq!(verdict(&spare), Ok(()));
}

#[test]
fn an_image_is_for_the_platform_of_its_layout_and_never_any() {
    let mut any = image_member();
    any.1.platform = "any".to_owned();
    assert_eq!(refused(&any), (Check::Platform, Reason::PlatformMismatch));
    let mut other = image_member();
    other.1.platform = "linux-aarch64".to_owned();
    assert_eq!(refused(&other), (Check::Platform, Reason::PlatformMismatch));
    // The run is for another platform than the layout's.
    let m = image_member();
    let err = verify_member(&m.0, &m.1, &m.2, "linux-aarch64", &stamp()).expect_err("refused");
    assert_eq!(
        (err.check, err.reason),
        (Check::Platform, Reason::PlatformMismatch)
    );
}

#[test]
fn an_image_that_says_it_is_for_the_run_but_whose_layout_is_not_is_refused() {
    // The manifest and the run agree on the platform; the layout inside says another.
    let mut m = image_member();
    m.1.platform = "linux-aarch64".to_owned();
    let err = verify_member(&m.0, &m.1, &m.2, "linux-aarch64", &stamp()).expect_err("refused");
    assert_eq!(
        (err.check, err.reason),
        (Check::Platform, Reason::PlatformMismatch)
    );
}

#[test]
fn the_image_metadata_must_agree_with_the_layout() {
    for meta in [
        None,
        Some(Metadata::Oci(OciMeta {
            os: "darwin".to_owned(),
            cpu: "x86_64".to_owned(),
        })),
        Some(Metadata::Oci(OciMeta {
            os: "linux".to_owned(),
            cpu: "arm64".to_owned(),
        })),
        Some(Metadata::FileMeta(
            ucdk_release_proto::v1::FileMeta::default(),
        )),
    ] {
        let mut m = image_member();
        m.1.metadata = meta;
        assert_eq!(refused(&m), (Check::Platform, Reason::PlatformMismatch));
    }
}

#[test]
fn an_image_member_is_checked_for_version_and_stamp_after_its_layout() {
    let mut version = image_member();
    version.1.version = "9".to_owned();
    assert_eq!(refused(&version), (Check::Version, Reason::MemberVersion));
    let mut stamped = image_member();
    stamped.1.stamp = None;
    assert_eq!(refused(&stamped), (Check::Stamp, Reason::SetMember));
}
