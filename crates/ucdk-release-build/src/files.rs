// SPDX-License-Identifier: Apache-2.0
//! The local disk, read through the `OutputFiles` seam.

use ucdk_release_seams::{DirEntry, FileKind, OutputFiles, SeamError};

/// An [`OutputFiles`] over the local disk.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalFiles;

fn not_written() -> SeamError {
    SeamError::Permanent("not written yet".to_owned())
}

impl OutputFiles for LocalFiles {
    fn stat(&mut self, _path: &str) -> Result<Option<FileKind>, SeamError> {
        Err(not_written())
    }

    fn list(&mut self, _path: &str) -> Result<Vec<DirEntry>, SeamError> {
        Err(not_written())
    }

    fn read(&mut self, _path: &str) -> Result<Vec<u8>, SeamError> {
        Err(not_written())
    }
}
