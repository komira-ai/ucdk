// SPDX-License-Identifier: Apache-2.0
//! Reading an OCI image layout held in memory: the identity digest of the image (the digest of
//! its manifest), the size of what the manifest reaches, and the platform of its config.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use sha2::{Digest as _, Sha256};
use ucdk_core::Hash256;

use crate::verify::{Check, VerifyError, fail};

/// The only `imageLayoutVersion` read: the version of the OCI image-layout specification.
const LAYOUT_VERSION: &str = "1.0.0";

/// The media type the index's descriptor must carry: the index points at an image manifest.
const IMAGE_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";

/// What a layout says about its image.
pub(crate) struct Layout {
    /// The image manifest digest, 64 lower-case hex digits.
    pub digest: String,
    /// The sum of the lengths of the manifest blob and of the blobs it reaches (its config and
    /// its layers), each distinct blob counted once however many descriptors name it.
    pub size: u64,
    pub os: String,
    pub cpu: String,
}

/// The SHA-256 of `bytes`, as 64 lower-case hex digits.
pub(crate) fn hex_of(bytes: &[u8]) -> String {
    Hash256::from_sha256(&Sha256::digest(bytes).into()).to_string()
}

fn is_hex_digest(text: &str) -> bool {
    Hash256::from_hex(text).is_some()
}

fn json(entry: &BTreeMap<String, Vec<u8>>, path: &str) -> Result<Value, VerifyError> {
    let bytes = entry
        .get(path)
        .ok_or_else(|| fail(Check::EntryShape, format!("the layout has no {path}")))?;
    serde_json::from_slice(bytes)
        .map_err(|e| fail(Check::EntryShape, format!("{path} is not JSON: {e}")))
}

fn text<'a>(value: &'a Value, key: &str, path: &str) -> Result<&'a str, VerifyError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| fail(Check::EntryShape, format!("{path} has no text field {key}")))
}

/// The blob a descriptor names: its hex digest and its bytes. The descriptor's `size` is
/// required and must be the length of the blob; its `mediaType` must be `media_type` when one
/// is given, and is not read otherwise.
fn blob<'a>(
    root: &str,
    entry: &'a BTreeMap<String, Vec<u8>>,
    descriptor: &Value,
    media_type: Option<&str>,
    path: &str,
) -> Result<(String, &'a Vec<u8>), VerifyError> {
    if let Some(want) = media_type {
        let found = text(descriptor, "mediaType", path)?;
        if found != want {
            return Err(fail(
                Check::EntryShape,
                format!("a descriptor in {path} has mediaType {found:?}, and {want:?} is expected"),
            ));
        }
    }
    let digest = text(descriptor, "digest", path)?;
    let hex = digest
        .strip_prefix("sha256:")
        .filter(|h| is_hex_digest(h))
        .ok_or_else(|| {
            fail(
                Check::EntryShape,
                format!("{digest:?} in {path} is not a sha256 digest"),
            )
        })?;
    let bytes = entry
        .get(&format!("{root}/blobs/sha256/{hex}"))
        .ok_or_else(|| fail(Check::EntryShape, format!("the layout has no blob {hex}")))?;
    let size = descriptor.get("size").and_then(Value::as_u64);
    if size != Some(bytes.len() as u64) {
        return Err(fail(
            Check::EntryShape,
            format!(
                "the descriptor of blob {hex} in {path} says size {size:?}, and the blob holds {} bytes",
                bytes.len()
            ),
        ));
    }
    Ok((hex.to_owned(), bytes))
}

/// Reads the layout under the directory `root` of `entry`.
///
/// Every path must be `oci-layout`, `index.json` or `blobs/sha256/<hex>`, and every blob must
/// hash to its name. The index names exactly one image manifest; the size is the length of that
/// manifest and of the config and layers it names. `oci-layout` must say `imageLayoutVersion`
/// `1.0.0`; the index's descriptor must have the image manifest media type; every descriptor
/// carries a `size` equal to the length of its blob. A blob that hashes to its name and that
/// nothing reaches is allowed and is not counted.
///
/// The platform is the config's `os` and `architecture`; its `variant` is not read, because a
/// platform name here is an operating system and a CPU and nothing finer. The two are returned
/// apart: the caller compares each with the manifest's metadata, and compares their join with
/// `-` with the platform of the run.
pub(crate) fn read(root: &str, entry: &BTreeMap<String, Vec<u8>>) -> Result<Layout, VerifyError> {
    let prefix = format!("{root}/");
    for (path, bytes) in entry {
        let inside = path.strip_prefix(&prefix).unwrap_or("");
        match inside.strip_prefix("blobs/sha256/") {
            Some(hex) if is_hex_digest(hex) => {
                if hex_of(bytes) != hex {
                    return Err(fail(
                        Check::BlobDigest,
                        format!("the blob {hex} does not hash to its name"),
                    ));
                }
            }
            _ if inside == "oci-layout" || inside == "index.json" => {}
            _ => {
                return Err(fail(
                    Check::NoOtherPath,
                    format!("{path} is not part of a layout under {root}"),
                ));
            }
        }
    }
    let layout = json(entry, &format!("{root}/oci-layout"))?;
    let version = text(&layout, "imageLayoutVersion", "oci-layout")?;
    if version != LAYOUT_VERSION {
        return Err(fail(
            Check::EntryShape,
            format!(
                "oci-layout has imageLayoutVersion {version:?}, and only {LAYOUT_VERSION:?} is read"
            ),
        ));
    }
    let index = json(entry, &format!("{root}/index.json"))?;
    let manifests = index
        .get("manifests")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    let [descriptor] = manifests else {
        return Err(fail(
            Check::EntryShape,
            format!(
                "index.json names {} manifests, and a layout of one image names one",
                manifests.len()
            ),
        ));
    };
    let (digest, manifest_bytes) =
        blob(root, entry, descriptor, Some(IMAGE_MANIFEST), "index.json")?;
    let manifest: Value = serde_json::from_slice(manifest_bytes).map_err(|e| {
        fail(
            Check::EntryShape,
            format!("the image manifest is not JSON: {e}"),
        )
    })?;
    let config = manifest.get("config").ok_or_else(|| {
        fail(
            Check::EntryShape,
            "the image manifest has no config".to_owned(),
        )
    })?;
    let (config_hex, config_bytes) = blob(root, entry, config, None, "the image manifest")?;
    let mut reached = BTreeSet::from([digest.clone()]);
    let mut size = manifest_bytes.len() as u64;
    if reached.insert(config_hex) {
        size += config_bytes.len() as u64;
    }
    let layers = manifest
        .get("layers")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for layer in layers {
        let (hex, bytes) = blob(root, entry, layer, None, "the image manifest")?;
        if reached.insert(hex) {
            size += bytes.len() as u64;
        }
    }
    let config: Value = serde_json::from_slice(config_bytes).map_err(|e| {
        fail(
            Check::EntryShape,
            format!("the image config is not JSON: {e}"),
        )
    })?;
    Ok(Layout {
        digest,
        size,
        os: text(&config, "os", "the image config")?.to_owned(),
        cpu: text(&config, "architecture", "the image config")?.to_owned(),
    })
}
