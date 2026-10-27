// SPDX-License-Identifier: Apache-2.0
//! Wrong stores of a second kind: each is the fake with one fault in how a read, a discard, a
//! rewrite or the name of a path is handled, a fault that shows only when the entries or the
//! writes around it are chosen to bring it out.

use std::collections::BTreeMap;

use ucdk_release::store::StoreKey;
use ucdk_release_fake::FakeStore;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::kit::StoreRig;
use ucdk_release_seams::{ArtifactStore, EntryWriter, SeamError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrong {
    /// `discard` takes the entry away but `files` of it goes on answering the old names.
    DiscardKeepsTheFileList,
    /// `discard` parks the files of the entry, and the next `begin` of the key gives them back.
    BeginRevivesADiscardedEntry,
    /// `get` answers from the first entry that holds the path and has the same key but for the
    /// stage.
    GetIgnoresTheStage,
    /// The same, but for the platform.
    GetIgnoresThePlatform,
    /// The same, but for the revision.
    GetIgnoresTheRevision,
    /// The same, but for the case of the key.
    GetKeyCaseFold,
    /// The same, but for the composition of the characters of the key.
    GetKeyUnicodeFold,
    /// The same, but for the trailing spaces of the key.
    GetKeyTrailingSpace,
    /// The same, but for the trailing dots of the key.
    GetKeyTrailingDot,
    /// The same, but for the key of a set document and the key of a member called `set`.
    GetSetKeyAliasesMemberSet,
    /// `discard` of an entry also removes the paths of the same names from every other entry.
    DiscardRemovesThePathsOfEveryEntry,
    /// `discard` of an entry also removes the files of every other entry whose bytes equal those
    /// of a file it held, as a store that shares equal bytes without counting who holds them.
    DiscardRemovesSharedBytes,
    /// `get` of a path that climbs out of the entry answers the file of the entry it comes to.
    GetClimbsIntoASibling,
    /// A writer that refuses a path with a NUL, a newline, a tab, a carriage return or a DEL in
    /// it, and accepts, and drops, a path with any other control character.
    OnlyFiveControlsRefused,
    /// Directories of a path are folded to lower case.
    DirectoriesCaseFolded,
    /// Directories of a path are put in one Unicode composition.
    DirectoriesUnicodeFolded,
    /// Trailing spaces of the directories of a path are trimmed.
    DirectoriesTrailingSpaceTrimmed,
    /// Trailing dots of a path are trimmed.
    TrailingDotTrimmed,
    /// A third write of a path, and every one after it, is ignored.
    ThirdWriteIgnored,
    /// A write of bytes that an earlier write gave the path, and another write replaced, is
    /// ignored.
    WriteOfEarlierBytesIgnored,
    /// A write of the bytes the path holds is refused.
    IdenticalRewriteRefused,
    /// A write to a path three or more parts deep that is shorter than what it holds keeps the
    /// tail of what it holds.
    DeepRewriteKeepsTheOldTail,
    /// A write of an empty file to a path three or more parts deep that holds a file is ignored.
    DeepEmptyRewriteIgnored,
}

pub struct HostileRig {
    pub flaw: Wrong,
}

impl StoreRig for HostileRig {
    type Store = Hostile;

    fn store(&self) -> Hostile {
        Hostile {
            inner: FakeStore::new(),
            flaw: self.flaw,
            parked: BTreeMap::new(),
            ghosts: BTreeMap::new(),
        }
    }
}

pub struct Hostile {
    inner: FakeStore,
    flaw: Wrong,
    /// What a discard parked, for the stub that gives it back.
    parked: BTreeMap<StoreKey, Vec<(String, Vec<u8>)>>,
    /// The names a discard left behind, for the stub that goes on listing them.
    ghosts: BTreeMap<StoreKey, Vec<String>>,
}

/// The key as the stubs that read from the wrong entry compare it.
fn alias(flaw: Wrong, key: &StoreKey) -> String {
    let text = key.as_str();
    let without = |index: usize| {
        text.split('/')
            .enumerate()
            .filter(|(at, _)| *at != index)
            .map(|(_, part)| part)
            .collect::<Vec<_>>()
            .join("/")
    };
    match flaw {
        Wrong::GetIgnoresTheStage => without(0),
        Wrong::GetIgnoresThePlatform => without(1),
        Wrong::GetIgnoresTheRevision => without(2),
        Wrong::GetKeyCaseFold => text.to_lowercase(),
        Wrong::GetKeyUnicodeFold => text.replace("e\u{301}", "\u{e9}"),
        Wrong::GetKeyTrailingSpace => text.trim_end_matches(' ').to_owned(),
        Wrong::GetKeyTrailingDot => text.trim_end_matches('.').to_owned(),
        Wrong::GetSetKeyAliasesMemberSet => text.replace("/m/", "/"),
        _ => text.to_owned(),
    }
}

/// `base` and a relative path, resolved as a file system would, as the parts that are left.
fn resolve(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in base.split('/').chain(rel.split('/')) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

impl Hostile {
    /// Every path of the entry, with its bytes.
    fn contents(&mut self, key: &StoreKey) -> Result<Vec<(String, Vec<u8>)>, SeamError> {
        let mut all = Vec::new();
        for rel in self.inner.files(key)? {
            let bytes = self.inner.get(key, &rel)?.unwrap_or_default();
            all.push((rel, bytes));
        }
        Ok(all)
    }

    /// Takes from every entry but `spared` the files that `gone` names.
    fn sweep(
        &mut self,
        spared: &StoreKey,
        gone: impl Fn(&str, &[u8]) -> bool,
    ) -> Result<(), SeamError> {
        for other in self.inner.keys()? {
            if other == *spared {
                continue;
            }
            let kept: Vec<(String, Vec<u8>)> = self
                .contents(&other)?
                .into_iter()
                .filter(|(rel, bytes)| !gone(rel, bytes))
                .collect();
            self.inner.discard(&other)?;
            let mut writer = self.inner.begin(&other)?;
            for (rel, bytes) in kept {
                writer.put_file(&rel, &bytes)?;
            }
        }
        Ok(())
    }

    /// The file of another entry that the path names by climbing out of the entry of `key`.
    fn sibling(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError> {
        let reached = resolve(key.as_str(), rel);
        for other in self.inner.keys()? {
            if other != *key
                && let Some(file) = reached.strip_prefix(&format!("{}/", other.as_str()))
            {
                return self.inner.get(&other, file);
            }
        }
        Ok(None)
    }
}

/// A writer that changes what a write does, by what the path was given before.
struct Writer<'a> {
    inner: Box<dyn EntryWriter + 'a>,
    flaw: Wrong,
    /// What each path was given, write by write, as the entry holds it.
    history: BTreeMap<String, Vec<Vec<u8>>>,
}

fn folded_directories(rel: &str, fold: impl Fn(&str) -> String) -> String {
    match rel.rsplit_once('/') {
        Some((directories, name)) => format!("{}/{name}", fold(directories)),
        None => rel.to_owned(),
    }
}

impl EntryWriter for Writer<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        let five = |c: char| matches!(c, '\0' | '\n' | '\t' | '\r' | '\u{7f}');
        let name = match self.flaw {
            Wrong::OnlyFiveControlsRefused if rel.chars().any(|c| c.is_control() && !five(c)) => {
                return Ok(());
            }
            Wrong::DirectoriesCaseFolded => folded_directories(rel, str::to_lowercase),
            Wrong::DirectoriesUnicodeFolded => {
                folded_directories(rel, |text| text.replace("e\u{301}", "\u{e9}"))
            }
            Wrong::DirectoriesTrailingSpaceTrimmed => folded_directories(rel, |text| {
                text.split('/')
                    .map(|part| part.trim_end_matches(' '))
                    .collect::<Vec<_>>()
                    .join("/")
            }),
            Wrong::TrailingDotTrimmed if !rel.ends_with("/.") && !rel.ends_with("/..") => {
                rel.trim_end_matches('.').to_owned()
            }
            _ => rel.to_owned(),
        };
        let past = self.history.get(&name).cloned().unwrap_or_default();
        let deep = name.split('/').count() >= 3;
        let kept: Vec<u8> = match (self.flaw, past.last()) {
            (Wrong::ThirdWriteIgnored, _) if past.len() >= 2 => return Ok(()),
            (Wrong::WriteOfEarlierBytesIgnored, _)
                if past.iter().rev().skip(1).any(|earlier| earlier == bytes) =>
            {
                return Ok(());
            }
            (Wrong::IdenticalRewriteRefused, Some(held)) if held == bytes => {
                return Err(SeamError::Refused(Reason::SetMember));
            }
            (Wrong::DeepEmptyRewriteIgnored, Some(_)) if deep && bytes.is_empty() => {
                return Ok(());
            }
            (Wrong::DeepRewriteKeepsTheOldTail, Some(held)) if deep => {
                let mut all = bytes.to_vec();
                all.extend_from_slice(held.get(bytes.len()..).unwrap_or_default());
                all
            }
            _ => bytes.to_vec(),
        };
        self.inner.put_file(&name, &kept)?;
        self.history.entry(name).or_default().push(kept);
        Ok(())
    }
}

impl ArtifactStore for Hostile {
    fn begin(&mut self, key: &StoreKey) -> Result<Box<dyn EntryWriter + '_>, SeamError> {
        self.ghosts.remove(key);
        let revived = self.parked.remove(key).unwrap_or_default();
        let mut writer = self.inner.begin(key)?;
        for (rel, bytes) in revived {
            writer.put_file(&rel, &bytes)?;
        }
        Ok(Box::new(Writer {
            inner: writer,
            flaw: self.flaw,
            history: BTreeMap::new(),
        }))
    }

    fn files(&mut self, key: &StoreKey) -> Result<Vec<String>, SeamError> {
        let files = self.inner.files(key)?;
        match self.ghosts.get(key) {
            Some(names) if files.is_empty() => Ok(names.clone()),
            _ => Ok(files),
        }
    }

    fn get(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError> {
        match self.flaw {
            Wrong::GetIgnoresTheStage
            | Wrong::GetIgnoresThePlatform
            | Wrong::GetIgnoresTheRevision
            | Wrong::GetKeyCaseFold
            | Wrong::GetKeyUnicodeFold
            | Wrong::GetKeyTrailingSpace
            | Wrong::GetKeyTrailingDot
            | Wrong::GetSetKeyAliasesMemberSet => {
                let wanted = alias(self.flaw, key);
                for other in self.inner.keys()? {
                    if alias(self.flaw, &other) == wanted
                        && let Some(bytes) = self.inner.get(&other, rel)?
                    {
                        return Ok(Some(bytes));
                    }
                }
                Ok(None)
            }
            Wrong::GetClimbsIntoASibling if rel.contains("..") => self.sibling(key, rel),
            _ => self.inner.get(key, rel),
        }
    }

    fn discard(&mut self, key: &StoreKey) -> Result<(), SeamError> {
        let held = self.contents(key)?;
        match self.flaw {
            Wrong::DiscardKeepsTheFileList => {
                let names = held.iter().map(|(rel, _)| rel.clone()).collect();
                self.ghosts.insert(key.clone(), names);
            }
            Wrong::BeginRevivesADiscardedEntry => {
                self.parked.insert(key.clone(), held);
            }
            Wrong::DiscardRemovesThePathsOfEveryEntry => {
                self.sweep(key, |rel, _| held.iter().any(|(path, _)| path == rel))?;
            }
            Wrong::DiscardRemovesSharedBytes => {
                self.sweep(key, |_, bytes| held.iter().any(|(_, same)| same == bytes))?;
            }
            _ => {}
        }
        self.inner.discard(key)
    }

    fn keys(&mut self) -> Result<Vec<StoreKey>, SeamError> {
        self.inner.keys()
    }
}
