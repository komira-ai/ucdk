// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]
//! Helpers shared by the ledger's tests: a scratch directory that cleans up after itself and
//! the records the tests write.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ucdk_ledger::{Change, Ledger, Record};
use ucdk_provider::{Kind, NodePath, PhysicalId};

/// A directory under the build's scratch area, empty on creation and removed on drop.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("ledger-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make the scratch directory");
        Self(dir)
    }

    pub fn dir(&self) -> &Path {
        &self.0
    }

    /// The ledger file of the directory.
    pub fn file(&self) -> PathBuf {
        self.0.join("ledger.ucdk")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn path(text: &str) -> NodePath {
    NodePath::new(text).expect("a valid path")
}

pub fn record(text: &str) -> Record {
    Record::intent(path(text), Kind::service(), "name", BTreeSet::new())
}

pub fn confirmed(text: &str, id: &str) -> Record {
    record(text).confirmed(PhysicalId::new(id))
}

/// Writes `change` against the current token.
pub fn put(ledger: &dyn Ledger, change: Change) {
    let token = ledger.read().expect("read").token;
    ledger.write(&token, change).expect("write");
}

/// FNV-1a, the checksum of the file format, so a test can seal a document it wrote by hand.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A document made of `lines` and a correct `end` line claiming `records` records.
pub fn sealed(lines: &[&str], records: usize) -> Vec<u8> {
    let mut body = lines.join("\n");
    body.push('\n');
    let end = format!("end {records} {:016x}\n", fnv(body.as_bytes()));
    body.push_str(&end);
    body.into_bytes()
}
