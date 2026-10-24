// SPDX-License-Identifier: Apache-2.0
//! The in-memory ledger.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ucdk_provider::NodePath;

use crate::record::{Change, Ledger, LedgerError, Record, Snapshot, Token};

#[derive(Debug, Default)]
struct Document {
    version: u64,
    records: BTreeMap<NodePath, Record>,
}

/// A ledger in memory. A clone shares the document with the original, so two engines (or an
/// engine and a test) can hold the same ledger.
#[derive(Debug, Clone, Default)]
pub struct MemoryLedger {
    document: Arc<Mutex<Document>>,
}

impl MemoryLedger {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Ledger for MemoryLedger {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        let document = self.document.lock().expect("the lock is never poisoned");
        Ok(Snapshot {
            token: Token(document.version),
            records: document.records.clone(),
        })
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        let mut document = self.document.lock().expect("the lock is never poisoned");
        if expected.0 != document.version {
            return Err(LedgerError::Stale);
        }
        match change {
            Change::Put(record) => {
                document.records.insert(record.path.clone(), record);
            }
            Change::Remove(path) => {
                document.records.remove(&path);
            }
        }
        document.version += 1;
        Ok(Token(document.version))
    }
}
