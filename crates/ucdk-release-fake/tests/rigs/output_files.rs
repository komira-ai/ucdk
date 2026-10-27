// SPDX-License-Identifier: Apache-2.0
//! The `OutputFiles` rig: the fake tree, and the stubs that break one promise of the reader.

use std::collections::BTreeMap;

use ucdk_release_fake::FakeOutputFiles;
use ucdk_release_seams::kit::{Node, OutputFilesRig};
use ucdk_release_seams::{DirEntry, FileKind, OutputFiles, SeamError};

pub const ROOT: &str = "/work";

/// What a stub gets wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// A listing in directory-walk order: a directory's children right after it.
    WalkOrder,
    /// A listing that calls every entry a file.
    EveryEntryIsAFile,
    /// A listing that follows a symlink to a directory.
    FollowsLinks,
    /// A `stat` that follows a symlink.
    StatFollows,
    /// A `list` of a path where nothing is, in a directory, that answers an empty listing.
    ListOfMissingIsEmpty,
    /// A `read` that follows a symlink.
    ReadFollowsLink,
    /// A `read` of a file directly below the root that needs a directory listed above it.
    NeedsListedDirAtTheRoot,
    /// A `read` of a file deeper than that which needs a directory listed above it.
    NeedsListedDirBelowTheRoot,
    /// A `read` of a directory that has just been `stat`ed that answers an empty file.
    ReadOfAStattedDirectoryIsEmpty,
    /// A `read` that resolves a path with `..` parts instead of refusing it.
    ReadResolvesClimbs,
    /// A `read` of a file nested below a listed directory that answers `Permanent`: the entry
    /// path of the file drops the part between.
    NestedEntryIsUnreadable,
    /// A `read` that keeps its first answer.
    CachesReads,
    /// A `read` that touches the file it read.
    ReadTouches,
    /// A `read` that turns bytes that are not text into replacement characters.
    LossyRead,
    /// A `read` of an empty file that answers `Permanent`, as a missing file.
    EmptyFileIsMissing,
    /// A `read` that answers no more than the first 4096 bytes.
    ReadTruncatesAt4k,
    /// A listing that leaves out every entry whose name starts with a dot.
    DotfilesHidden,
    /// A listing that leaves out every entry more than two directories down.
    DeepEntriesOmitted,
    /// A listing that calls a symlink to a directory a directory.
    LinkToDirListedAsDirectory,
    /// A listing that leaves out a symlink with nothing at its target.
    DanglingLinkOmitted,
    /// A `list` of a symlink to a directory that lists the directory.
    ListOfSymlinkToDirFollows,
    /// A `stat` of a symlink named with a trailing slash that says what it points at.
    TrailingSlashFollowsStat,
    /// A `read` of a symlink named with a trailing slash that answers the bytes it points at.
    TrailingSlashFollowsRead,
    /// A `read` that refuses a path with `..` anywhere in it, a file name such as `a..b`
    /// included.
    ReadRefusesDotDotNames,
    /// A listing that follows a symlink to a directory that is not directly below the listed
    /// directory.
    FollowsNestedLinks,
    /// A `stat` of a path below a regular file that fails with `Unknown`, as the operating
    /// system's "not a directory" is taken for a failure.
    StatBelowAFileIsAnError,
    /// A `list` of a directory with nothing in it that answers `Permanent`.
    ListOfAnEmptyDirectoryIsPermanent,
    /// A `read` that answers no more than the first 65,536 bytes.
    ReadTruncatesAt64k,
    /// A `read` that answers no more than the first 131,072 bytes.
    ReadTruncatesAt128k,
    /// A `read` that answers no more than the first 1,048,576 bytes.
    ReadTruncatesAt1M,
    /// A listing of more than 256 entries that leaves out the rest.
    ListCappedAt256,
    /// A listing that leaves out every entry more than five parts deep.
    DeepEntriesOmittedBeyond5,
    /// A `read` of a symlink with nothing at its target that answers an empty file.
    ReadOfDanglingLinkIsEmpty,
    /// A `read` of a symlink to a directory that answers an empty file.
    ReadOfLinkToDirIsEmpty,
    /// A `list` of a symlink with nothing at its target that answers an empty listing.
    ListOfDanglingLinkIsEmpty,
    /// A `list` of a path with a trailing slash that answers `Permanent`.
    ListTrailingSlashFails,
    /// A `list` of a path with a trailing slash that gives each `rel` a leading slash.
    ListTrailingSlashRelsLeadingSlash,
    /// A path with a `.` part is `Refused`.
    RefusesDotPart,
    /// A path with an empty part (`//`) is `Refused`.
    RefusesDoubleSlash,
    /// A `read` of a path with a space in it answers `Permanent`.
    ReadFailsOnSpaceInPath,
    /// A listing that leaves out every entry with a space in its name.
    ListOmitsNamesWithSpace,
    /// A listing that leaves out every entry with a name that is not ASCII.
    ListOmitsNonAscii,
    /// A listing that writes every byte of a name that is not a letter, a digit, `.`, `_` or `-`
    /// as `%` and two hexadecimal digits.
    ListNamesPercentEncoded,
    /// A `read` of a path with a newline in it answers `Permanent`.
    ReadOfFileWithNewlineNameFails,
    /// A listing sorted so that runs of digits compare as numbers.
    NaturalSort,
    /// A `read` of an empty file at least two directories down answers `Permanent`.
    ReadOfEmptyFileInSubdirIsMissing,
    /// A `read` of a path below a regular file that answers the bytes of that file.
    ReadBelowAFileAnswersTheParent,
    /// A `list` of a path below a regular file that answers an empty listing.
    ListBelowAFileIsEmpty,
    /// A `read` that turns each carriage return and line feed into a line feed.
    ReadNormalisesCrlf,
    /// A `read` that drops a UTF-8 byte order mark at the start of a file.
    ReadStripsBom,
    /// A `read` that drops a carriage return and line feed at the end of a file.
    ReadStripsTrailingCrlf,
    /// A `read` that drops the zero bytes at the end of a file.
    ReadTrimsTrailingNuls,
    /// A `read` that drops the last byte of a file whose length is a multiple of 4096.
    ReadDropsTheLastByteAtA4kMultiple,
    /// A listing that calls every entry at least two parts deep a file.
    ListNestedKindsAreFiles,
    /// A listing that names every entry at least four parts deep twice.
    ListDeepEntriesTwice,
    /// A listing that adds a file named `phantom` in each directory four parts deep.
    ListAddsAPhantomEntry,
    /// A listing that leaves out every directory at least two parts deep that has nothing in it.
    ListOmitsNestedEmptyDirectories,
    /// A listing that writes a `/` after the `rel` of each directory at least three parts deep.
    ListDeepDirectoriesHaveATrailingSlash,
    /// A `stat` of a symlink named with a trailing slash that answers `None`.
    StatOfASlashedLinkIsNone,
    /// A `stat` of a symlink with nothing at its target, named with a trailing slash, that fails
    /// with `Unknown`.
    StatOfASlashedDanglingLinkFails,
    /// A `stat` that refuses a path with a part that holds two dots, such as `a..b`, without
    /// being a `..` part.
    StatRefusesDotDotNames,
    /// A `list` that refuses a path with a part that holds two dots without being a `..` part.
    ListRefusesDotDotNames,
    /// A `stat`, `list` and `read` that resolve a path whose first part is `..` instead of
    /// refusing it.
    LeadingClimbIsResolved,
    /// A `list` of a symlink to a directory named with a trailing slash that lists the directory.
    ListFollowsASlashedLinkToADirectory,
    /// A `read` of a symlink to a file that answers the file's bytes when the link is not in the
    /// directory of its target.
    ReadFollowsALinkInAnotherDirectory,
    /// A `read` of a symlink to a file that answers the file's bytes when the link is directly
    /// below the root.
    ReadFollowsALinkAtTheRoot,
    /// A `read` of a symlink to a file that answers the file's bytes when the link is three or
    /// more parts below the root.
    ReadFollowsADeepLink,
    /// A `read` of a directory directly below the root that answers an empty file.
    ReadOfADirectoryAtTheRootIsEmpty,
    /// A `read` of a directory three or more parts below the root that answers an empty file.
    ReadOfADeepDirectoryIsEmpty,
    /// A `read` of a path two or more parts below a regular file that answers that file's bytes.
    ReadDeepBelowAFileAnswersTheAncestor,
    /// A `list` of a path two or more parts below a regular file that answers an empty listing.
    ListDeepBelowAFileIsEmpty,
    /// A `stat` of a path two or more parts below a regular file that fails with `Unknown`.
    StatDeepBelowAFileIsAnError,
    /// A `stat` of a path in a directory that is not there that fails with `Unknown`.
    StatBelowAMissingDirectoryIsAnError,
    /// A `list` of a path in a directory that is not there that answers an empty listing.
    ListBelowAMissingDirectoryIsEmpty,
    /// A `read` of a path in a directory that is not there that answers an empty file.
    ReadBelowAMissingDirectoryIsEmpty,
    /// A `read` of a path that starts with `./` that reads it below the root.
    ReadAcceptsADotSlashPath,
    /// A `read` of a path that is the root's path without its first `/` that reads it as absolute.
    ReadAcceptsARootlessPath,
    /// A `read` that answers no more than the first 4,194,304 bytes.
    ReadTruncatesAt4M,
    /// A `read` that drops the last byte of a file whose length is a multiple of 131,072.
    ReadDropsTheLastByteAtA128kMultiple,
    /// A `read` that drops the spaces and tabs at the end of a file.
    ReadTrimsTrailingBlanks,
    /// A `read` that answers the bytes it answered before when the file has the same length.
    CachesReadsByLength,
}

/// How many parts a path below the root has; 0 for any other.
fn parts_below_root(path: &str) -> usize {
    path.strip_prefix(&format!("{ROOT}/"))
        .map_or(0, |rest| rest.split('/').count())
}

/// Whether a part of `path` holds two dots without being a `..` part.
fn has_dot_dot_name(path: &str) -> bool {
    path.split('/')
        .any(|part| part != ".." && part.contains(".."))
}

/// `bytes` with each carriage return before a line feed left out.
fn without_carriage_returns(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for (at, byte) in bytes.iter().enumerate() {
        if !(*byte == b'\r' && bytes.get(at + 1) == Some(&b'\n')) {
            out.push(*byte);
        }
    }
    out
}

/// How many parts a `rel` has.
fn depth(rel: &str) -> usize {
    rel.split('/').count()
}

/// `name` with each run of digits padded, so that a sort by it puts `f2` before `f10`.
fn natural_key(name: &str) -> String {
    let mut key = String::new();
    let mut digits = String::new();
    for c in name.chars().chain(std::iter::once('\0')) {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        if !digits.is_empty() {
            key.push_str(&format!("{digits:0>20}"));
            digits.clear();
        }
        key.push(c);
    }
    key
}

/// `name` with each byte that is not a letter, a digit, `.`, `_`, `-` or `/` as `%` and two
/// hexadecimal digits.
fn percent_encoded(name: &str) -> String {
    let mut out = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"._-/".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub struct FakeFilesRig {
    pub flaw: Option<Flaw>,
}

impl FakeFilesRig {
    pub fn conforming() -> Self {
        Self { flaw: None }
    }

    pub fn flawed(flaw: Flaw) -> Self {
        Self { flaw: Some(flaw) }
    }
}

pub struct Flawed {
    inner: FakeOutputFiles,
    flaw: Option<Flaw>,
    listed: Vec<String>,
    last_stat: Option<String>,
    cache: BTreeMap<String, Vec<u8>>,
    /// Where each symlink of the tree points, both as full paths.
    targets: BTreeMap<String, String>,
}

fn full(rel: &str) -> String {
    format!("{ROOT}/{rel}")
}

impl OutputFilesRig for FakeFilesRig {
    type Files = Flawed;

    fn root(&self) -> String {
        ROOT.to_owned()
    }

    fn reader(&self, tree: &[(&str, Node<'_>)]) -> Flawed {
        let mut inner = FakeOutputFiles::new();
        let mut targets = BTreeMap::new();
        for (rel, node) in tree {
            match node {
                Node::File(bytes) => inner.file(&full(rel), bytes),
                Node::Directory => inner.dir(&full(rel)),
                Node::Link(target) => {
                    inner.symlink(&full(rel));
                    targets.insert(full(rel), full(target));
                }
            }
        }
        Flawed {
            inner,
            flaw: self.flaw,
            listed: Vec::new(),
            last_stat: None,
            cache: BTreeMap::new(),
            targets,
        }
    }

    fn change(&self, files: &mut Flawed, rel: &str, bytes: &[u8]) {
        files.inner.file(&full(rel), bytes);
    }

    fn snapshot(&self, files: &Flawed) -> Vec<(String, FileKind, usize)> {
        files.inner.snapshot()
    }
}

/// `path` with its `..` parts resolved.
fn resolved(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/').skip(1) {
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    format!("/{}", parts.join("/"))
}

impl Flawed {
    fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }

    /// The listed directory above `path`, if one was listed.
    fn listed_above(&self, path: &str) -> Option<&String> {
        self.listed
            .iter()
            .find(|dir| path.starts_with(&format!("{dir}/")))
    }

    /// The path the symlink at `link` points at, when it is one.
    fn target_of(&self, link: &str) -> Option<String> {
        self.targets.get(link).cloned()
    }

    /// What is at the other end of the symlink at `link`; `None` for nothing.
    fn target_kind(&mut self, link: &str) -> Option<FileKind> {
        let target = self.target_of(link)?;
        self.inner.stat(&target).ok().flatten()
    }

    /// What a stub that refuses `.` parts or empty parts answers a path that has one.
    fn refuses(&self, path: &str) -> Option<SeamError> {
        let dot = self.is(Flaw::RefusesDotPart) && path.split('/').any(|part| part == ".");
        let double = self.is(Flaw::RefusesDoubleSlash) && path.contains("//");
        (dot || double).then_some(SeamError::Refused(
            ucdk_release_proto::v1::Reason::BuildOutput,
        ))
    }

    /// The regular file `path` is below, when the part above it is one.
    fn file_above(&mut self, path: &str) -> Option<String> {
        let (parent, _) = path.rsplit_once('/')?;
        (self.inner.stat(parent) == Ok(Some(FileKind::File))).then(|| parent.to_owned())
    }

    /// The regular file that is two or more parts above `path`, when there is one.
    fn file_far_above(&mut self, path: &str) -> Option<String> {
        let mut above = path.rsplit_once('/')?.0;
        while let Some((higher, _)) = above.rsplit_once('/') {
            if self.inner.stat(higher) == Ok(Some(FileKind::File)) {
                return Some(higher.to_owned());
            }
            above = higher;
        }
        None
    }

    /// Whether the directory `path` is in is not there.
    fn directory_above_is_missing(&mut self, path: &str) -> bool {
        path.rsplit_once('/')
            .is_some_and(|(parent, _)| self.inner.stat(parent) == Ok(None))
    }

    /// Whether the directory `path` is in is there.
    fn directory_above_exists(&mut self, path: &str) -> bool {
        path.rsplit_once('/')
            .is_some_and(|(parent, _)| self.inner.stat(parent) == Ok(Some(FileKind::Directory)))
    }

    /// The target of the symlink that `path` names with a slash after it, when it does.
    fn slashed_target(&self, path: &str) -> Option<String> {
        self.target_of(path.strip_suffix('/')?)
    }
}

impl OutputFiles for Flawed {
    fn stat(&mut self, path: &str) -> Result<Option<FileKind>, SeamError> {
        self.last_stat = Some(path.to_owned());
        if self.is(Flaw::StatRefusesDotDotNames) && has_dot_dot_name(path) {
            return Err(SeamError::Refused(
                ucdk_release_proto::v1::Reason::BuildOutput,
            ));
        }
        if self.is(Flaw::LeadingClimbIsResolved) && path.starts_with("/../") {
            return self.inner.stat(&resolved(path));
        }
        if (self.is(Flaw::StatDeepBelowAFileIsAnError) && self.file_far_above(path).is_some())
            || (self.is(Flaw::StatBelowAMissingDirectoryIsAnError)
                && self.directory_above_is_missing(path))
        {
            return Err(SeamError::Unknown(format!("{path} cannot be read")));
        }
        if let Some(link) = path.strip_suffix('/')
            && self.target_of(link).is_some()
        {
            if self.is(Flaw::StatOfASlashedLinkIsNone) {
                return Ok(None);
            }
            if self.is(Flaw::StatOfASlashedDanglingLinkFails) && self.target_kind(link).is_none() {
                return Err(SeamError::Unknown(format!("{path} cannot be read")));
            }
        }
        if self.is(Flaw::TrailingSlashFollowsStat)
            && let Some(target) = self.slashed_target(path)
        {
            return self.inner.stat(&target);
        }
        let kind = self.inner.stat(path)?;
        if self.is(Flaw::StatBelowAFileIsAnError)
            && kind.is_none()
            && let Some((parent, _)) = path.rsplit_once('/')
            && self.inner.stat(parent)? == Some(FileKind::File)
        {
            return Err(SeamError::Unknown(format!("{parent} is not a directory")));
        }
        if self.is(Flaw::StatFollows) && kind == Some(FileKind::Symlink) {
            return Ok(Some(FileKind::File));
        }
        Ok(kind)
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, SeamError> {
        if self.is(Flaw::ListOfMissingIsEmpty)
            && self.inner.stat(path) == Ok(None)
            && self.directory_above_exists(path)
        {
            return Ok(Vec::new());
        }
        if (self.is(Flaw::ListDeepBelowAFileIsEmpty) && self.file_far_above(path).is_some())
            || (self.is(Flaw::ListBelowAMissingDirectoryIsEmpty)
                && self.directory_above_is_missing(path))
        {
            return Ok(Vec::new());
        }
        if self.is(Flaw::ListFollowsASlashedLinkToADirectory)
            && let Some(target) = self.slashed_target(path)
            && self.target_kind(path.trim_end_matches('/')) == Some(FileKind::Directory)
        {
            return self.inner.list(&target);
        }
        if let Some(refused) = self.refuses(path) {
            return Err(refused);
        }
        if self.is(Flaw::ListRefusesDotDotNames) && has_dot_dot_name(path) {
            return Err(SeamError::Refused(
                ucdk_release_proto::v1::Reason::BuildOutput,
            ));
        }
        if self.is(Flaw::LeadingClimbIsResolved) && path.starts_with("/../") {
            return self.list(&resolved(path));
        }
        if self.is(Flaw::ListBelowAFileIsEmpty) && self.file_above(path).is_some() {
            return Ok(Vec::new());
        }
        if self.is(Flaw::ListTrailingSlashFails) && path.ends_with('/') {
            return Err(SeamError::Permanent(format!("{path} is not a directory")));
        }
        if self.is(Flaw::ListOfDanglingLinkIsEmpty)
            && self.inner.stat(path) == Ok(Some(FileKind::Symlink))
            && self.target_kind(path).is_none()
        {
            return Ok(Vec::new());
        }
        if self.is(Flaw::ListOfSymlinkToDirFollows)
            && let Some(target) = self.target_of(path)
        {
            return self.inner.list(&target);
        }
        let mut entries = self.inner.list(path)?;
        if self.is(Flaw::ListOfAnEmptyDirectoryIsPermanent) && entries.is_empty() {
            return Err(SeamError::Permanent(format!("{path} is empty")));
        }
        let base = path.trim_end_matches('/').to_owned();
        self.listed.push(base.clone());
        if self.is(Flaw::DotfilesHidden) {
            entries.retain(|e| !e.rel.split('/').any(|part| part.starts_with('.')));
        }
        if self.is(Flaw::DeepEntriesOmitted) {
            entries.retain(|e| e.rel.split('/').count() <= 2);
        }
        if self.is(Flaw::LinkToDirListedAsDirectory) {
            for entry in &mut entries {
                if entry.kind == FileKind::Symlink
                    && self.target_kind(&format!("{base}/{}", entry.rel))
                        == Some(FileKind::Directory)
                {
                    entry.kind = FileKind::Directory;
                }
            }
        }
        if self.is(Flaw::DanglingLinkOmitted) {
            let mut kept = Vec::new();
            for entry in entries {
                let dangling = entry.kind == FileKind::Symlink
                    && self.target_kind(&format!("{base}/{}", entry.rel)).is_none();
                if !dangling {
                    kept.push(entry);
                }
            }
            entries = kept;
        }
        if self.is(Flaw::ListNestedKindsAreFiles) {
            for entry in &mut entries {
                if depth(&entry.rel) >= 2 {
                    entry.kind = FileKind::File;
                }
            }
        }
        if self.is(Flaw::ListDeepEntriesTwice) {
            entries = entries
                .into_iter()
                .flat_map(|e| {
                    let times = if depth(&e.rel) >= 4 { 2 } else { 1 };
                    std::iter::repeat_n(e, times)
                })
                .collect();
        }
        if self.is(Flaw::ListAddsAPhantomEntry) {
            let phantoms: Vec<DirEntry> = entries
                .iter()
                .filter(|e| depth(&e.rel) == 4 && e.kind == FileKind::Directory)
                .map(|e| DirEntry {
                    rel: format!("{}/phantom", e.rel),
                    kind: FileKind::File,
                })
                .collect();
            entries.extend(phantoms);
            entries.sort_by(|a, b| a.rel.cmp(&b.rel));
        }
        if self.is(Flaw::ListOmitsNestedEmptyDirectories) {
            let rels: Vec<String> = entries.iter().map(|e| e.rel.clone()).collect();
            entries.retain(|e| {
                e.kind != FileKind::Directory
                    || depth(&e.rel) < 2
                    || rels
                        .iter()
                        .any(|other| other.starts_with(&format!("{}/", e.rel)))
            });
        }
        if self.is(Flaw::ListDeepDirectoriesHaveATrailingSlash) {
            for entry in &mut entries {
                if entry.kind == FileKind::Directory && depth(&entry.rel) >= 3 {
                    entry.rel.push('/');
                }
            }
        }
        if self.is(Flaw::ListCappedAt256) {
            entries.truncate(256);
        }
        if self.is(Flaw::DeepEntriesOmittedBeyond5) {
            entries.retain(|e| e.rel.split('/').count() <= 5);
        }
        if self.is(Flaw::ListOmitsNamesWithSpace) {
            entries.retain(|e| !e.rel.contains(' '));
        }
        if self.is(Flaw::ListOmitsNonAscii) {
            entries.retain(|e| e.rel.is_ascii());
        }
        if self.is(Flaw::ListNamesPercentEncoded) {
            for entry in &mut entries {
                entry.rel = percent_encoded(&entry.rel);
            }
        }
        if self.is(Flaw::ListTrailingSlashRelsLeadingSlash) && path.ends_with('/') {
            for entry in &mut entries {
                entry.rel = format!("/{}", entry.rel);
            }
        }
        if self.is(Flaw::NaturalSort) {
            entries.sort_by_key(|e| natural_key(&e.rel));
        }
        if self.is(Flaw::WalkOrder) {
            entries.sort_by_key(|e| e.rel.replace('/', "\u{0}"));
        }
        if self.is(Flaw::EveryEntryIsAFile) {
            for entry in &mut entries {
                entry.kind = FileKind::File;
            }
        }
        if self.is(Flaw::FollowsLinks) {
            let below: Vec<DirEntry> = entries
                .iter()
                .filter_map(|e| {
                    e.rel.strip_prefix("real/").map(|rest| DirEntry {
                        rel: format!("link/{rest}"),
                        kind: e.kind,
                    })
                })
                .collect();
            entries.extend(below);
            entries.sort_by(|a, b| a.rel.cmp(&b.rel));
        }
        if self.is(Flaw::FollowsNestedLinks) {
            let mut below = Vec::new();
            for entry in &entries {
                if entry.kind == FileKind::Symlink
                    && entry.rel.contains('/')
                    && let Some(target) = self.target_of(&format!("{base}/{}", entry.rel))
                {
                    for inside in self.inner.list(&target)? {
                        below.push(DirEntry {
                            rel: format!("{}/{}", entry.rel, inside.rel),
                            kind: inside.kind,
                        });
                    }
                }
            }
            entries.extend(below);
            entries.sort_by(|a, b| a.rel.cmp(&b.rel));
        }
        Ok(entries)
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, SeamError> {
        if let Some(refused) = self.refuses(path) {
            return Err(refused);
        }
        if self.is(Flaw::LeadingClimbIsResolved) && path.starts_with("/../") {
            return self.read(&resolved(path));
        }
        if self.is(Flaw::ReadBelowAFileAnswersTheParent)
            && let Some(parent) = self.file_above(path)
        {
            return self.inner.read(&parent);
        }
        if self.is(Flaw::ReadDeepBelowAFileAnswersTheAncestor)
            && let Some(ancestor) = self.file_far_above(path)
        {
            return self.inner.read(&ancestor);
        }
        if self.is(Flaw::ReadBelowAMissingDirectoryIsEmpty) && self.directory_above_is_missing(path)
        {
            return Ok(Vec::new());
        }
        if self.is(Flaw::ReadAcceptsADotSlashPath)
            && let Some(rest) = path.strip_prefix("./")
        {
            return self.inner.read(&full(rest));
        }
        if self.is(Flaw::ReadAcceptsARootlessPath)
            && let Some(rest) = path.strip_prefix(ROOT.trim_start_matches('/'))
            && rest.starts_with('/')
        {
            return self.inner.read(&format!("{ROOT}{rest}"));
        }
        let parts = parts_below_root(path);
        let at_a_directory_that_is_wrong = (self.is(Flaw::ReadOfADirectoryAtTheRootIsEmpty)
            && parts == 1)
            || (self.is(Flaw::ReadOfADeepDirectoryIsEmpty) && parts >= 3);
        if at_a_directory_that_is_wrong && self.inner.stat(path) == Ok(Some(FileKind::Directory)) {
            return Ok(Vec::new());
        }
        let link_is_wrong = (self.is(Flaw::ReadFollowsALinkAtTheRoot) && parts == 1)
            || (self.is(Flaw::ReadFollowsADeepLink) && parts >= 3)
            || (self.is(Flaw::ReadFollowsALinkInAnotherDirectory)
                && self.target_of(path).is_some_and(|target| {
                    target.rsplit_once('/').map(|t| t.0) != path.rsplit_once('/').map(|p| p.0)
                }));
        if link_is_wrong
            && self.target_kind(path) == Some(FileKind::File)
            && let Some(target) = self.target_of(path)
        {
            return self.inner.read(&target);
        }
        if (self.is(Flaw::ReadFailsOnSpaceInPath) && path.contains(' '))
            || (self.is(Flaw::ReadOfFileWithNewlineNameFails) && path.contains('\n'))
        {
            return Err(SeamError::Permanent(format!("{path} cannot be read")));
        }
        if self.inner.stat(path) == Ok(Some(FileKind::Symlink)) {
            let target = self.target_kind(path);
            if (self.is(Flaw::ReadOfDanglingLinkIsEmpty) && target.is_none())
                || (self.is(Flaw::ReadOfLinkToDirIsEmpty) && target == Some(FileKind::Directory))
            {
                return Ok(Vec::new());
            }
        }
        if self.is(Flaw::ReadRefusesDotDotNames) && path.contains("..") {
            return Err(SeamError::Refused(
                ucdk_release_proto::v1::Reason::BuildOutput,
            ));
        }
        if self.is(Flaw::TrailingSlashFollowsRead)
            && let Some(target) = self.slashed_target(path)
        {
            return self.inner.read(&target);
        }
        if self.is(Flaw::CachesReads)
            && let Some(bytes) = self.cache.get(path)
        {
            return Ok(bytes.clone());
        }
        if self.is(Flaw::CachesReadsByLength)
            && let Some(cached) = self.cache.get(path).cloned()
            && self
                .inner
                .read(path)
                .is_ok_and(|now| now.len() == cached.len())
        {
            return Ok(cached);
        }
        let mut result = self.inner.read(path);
        match self.flaw {
            Some(Flaw::ReadFollowsLink)
                if result.is_err() && self.inner.stat(path) == Ok(Some(FileKind::Symlink)) =>
            {
                result = Ok(b"target".to_vec());
            }
            Some(Flaw::ReadOfAStattedDirectoryIsEmpty)
                if result.is_err()
                    && self.last_stat.as_deref() == Some(path)
                    && self.inner.stat(path) == Ok(Some(FileKind::Directory)) =>
            {
                result = Ok(Vec::new());
            }
            Some(Flaw::ReadResolvesClimbs) if result.is_err() && path.contains("/../") => {
                result = self.inner.read(&resolved(path));
            }
            Some(Flaw::NeedsListedDirAtTheRoot | Flaw::NeedsListedDirBelowTheRoot)
                if result.is_ok() =>
            {
                let depth = path
                    .strip_prefix(&format!("{ROOT}/"))
                    .map_or(0, |rest| rest.split('/').count());
                let wrong_depth = if self.is(Flaw::NeedsListedDirAtTheRoot) {
                    depth == 1
                } else {
                    depth > 1
                };
                if wrong_depth && self.listed_above(path).is_none() {
                    result = Err(SeamError::Permanent(format!("{path} was not listed")));
                }
            }
            Some(Flaw::NestedEntryIsUnreadable) if result.is_ok() => {
                // The parts that name something: a `.` part and an empty part name nothing.
                let nested = self.listed_above(path).is_some_and(|dir| {
                    path[dir.len() + 1..]
                        .split('/')
                        .filter(|part| !part.is_empty() && *part != ".")
                        .count()
                        > 1
                });
                if nested {
                    result = Err(SeamError::Permanent(format!("{path} is not an entry")));
                }
            }
            _ => {}
        }
        if let Ok(bytes) = &mut result {
            match self.flaw {
                Some(Flaw::LossyRead) => {
                    *bytes = String::from_utf8_lossy(bytes).into_owned().into_bytes();
                }
                Some(Flaw::ReadTruncatesAt4k) => bytes.truncate(4096),
                Some(Flaw::ReadTruncatesAt64k) => bytes.truncate(65_536),
                Some(Flaw::ReadTruncatesAt128k) => bytes.truncate(131_072),
                Some(Flaw::ReadTruncatesAt1M) => bytes.truncate(1_048_576),
                Some(Flaw::ReadNormalisesCrlf) => *bytes = without_carriage_returns(bytes),
                Some(Flaw::ReadStripsBom) if bytes.starts_with(&[0xef, 0xbb, 0xbf]) => {
                    bytes.drain(..3);
                }
                Some(Flaw::ReadStripsTrailingCrlf) if bytes.ends_with(b"\r\n") => {
                    bytes.truncate(bytes.len() - 2);
                }
                Some(Flaw::ReadTrimsTrailingNuls) => {
                    while bytes.last() == Some(&0) {
                        bytes.pop();
                    }
                }
                Some(Flaw::ReadDropsTheLastByteAtA4kMultiple)
                    if !bytes.is_empty() && bytes.len() % 4096 == 0 =>
                {
                    bytes.pop();
                }
                Some(Flaw::ReadTruncatesAt4M) => bytes.truncate(4_194_304),
                Some(Flaw::ReadDropsTheLastByteAtA128kMultiple)
                    if !bytes.is_empty() && bytes.len() % 131_072 == 0 =>
                {
                    bytes.pop();
                }
                Some(Flaw::ReadTrimsTrailingBlanks) => {
                    while matches!(bytes.last(), Some(b' ' | b'\t')) {
                        bytes.pop();
                    }
                }
                _ => {}
            }
        }
        if self.is(Flaw::ReadOfEmptyFileInSubdirIsMissing)
            && result.as_ref().is_ok_and(Vec::is_empty)
            && path
                .strip_prefix(&format!("{ROOT}/"))
                .is_some_and(|rest| rest.split('/').count() >= 3)
        {
            result = Err(SeamError::Permanent(format!("{path} is empty")));
        }
        if self.is(Flaw::EmptyFileIsMissing) && result.as_ref().is_ok_and(Vec::is_empty) {
            result = Err(SeamError::Permanent(format!("{path} is empty")));
        }
        if let Ok(bytes) = &result {
            if self.is(Flaw::CachesReads) || self.is(Flaw::CachesReadsByLength) {
                self.cache.insert(path.to_owned(), bytes.clone());
            }
            if self.is(Flaw::ReadTouches) {
                let mut touched = bytes.clone();
                touched.push(b'!');
                self.inner.file(path, &touched);
            }
        }
        result
    }
}
