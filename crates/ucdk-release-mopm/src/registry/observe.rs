// SPDX-License-Identifier: Apache-2.0
//! The reads of a publisher that decide nothing: `observe`, `newest` and `list`.
//!
//! `observe` reads the authoritative per-version record, and for a declared set also the group
//! the set names. `newest` reads the package's versions and takes the digests from the sparse
//! index, reading the record of any version the index does not show yet. `list` is for reports
//! only and may lag. A read that fails fails the whole call: it is never taken for an absent
//! answer and a partial list is never returned.

use ucdk_release::codec::GrantFault;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{ArtifactType, Coordinate};
use ucdk_release_seams::{ListScope, Listed, SeamError};

use super::{Caller, Registry, refuse, unexpected};
use crate::answers::{Group, GroupStatus, RecordFile, VersionRecord, VersionState};
use crate::errors::{Mapped, Op, error_code, map_reply, map_schema_violation, map_transport};
use crate::grammar::valid_name;
use crate::group_id::group_id;
use crate::listing::{parse_index, parse_package, parse_page, valid_prefix};
use crate::mapping::{Key, Member};
use crate::platform::Subdir;
use crate::requests::get;
use crate::transport::Transport;
use crate::url::encode_segment;

/// The most pages of a search `list` reads: a listing longer than this is an error and is never
/// silently cut off.
pub const MAX_LIST_PAGES: usize = 1000;

/// The page size `list` asks for, the most mopm answers.
const PAGE: &str = "100";

fn held(file: &RecordFile) -> Held {
    Held {
        digest: file.sha256.clone(),
        stamp: None,
        exceptional: false,
    }
}

fn coordinate(channel: &str, name: &str, version: &str, subdir: Subdir) -> Coordinate {
    Coordinate {
        channel: channel.to_owned(),
        r#type: ArtifactType::Conda as i32,
        name: name.to_owned(),
        version: version.to_owned(),
        platform: subdir.platform().to_owned(),
    }
}

/// What a version record says of the coordinate `k`: a burned number is `Burned`, a coordinate the
/// record holds a file for is `Present` or `Yanked` (a takedown keeps its digest and is `Yanked`),
/// and a coordinate the record has no file for is `Absent`: the version is sealed with another
/// file set.
fn presence_of(record: &VersionRecord, k: &Key) -> Presence {
    let file = record.files.iter().find(|f| f.subdir == k.subdir);
    match (record.state, record.files.is_empty(), file) {
        (VersionState::Tombstoned, true, _) => Presence::Burned,
        (_, _, None) => Presence::Absent,
        (VersionState::Published, _, Some(f)) => Presence::Present(held(f)),
        (_, _, Some(f)) => Presence::Yanked(held(f)),
    }
}

/// The member of `set` that holds the file `k` names.
fn member_of<'a>(set: &'a [Member], k: &Key) -> Option<&'a Member> {
    set.iter().find(|m| {
        m.files.iter().any(|f| {
            (m.name.as_str(), m.version.as_str(), f.subdir)
                == (k.name.as_str(), k.version.as_str(), k.subdir)
        })
    })
}

/// What the group read says of `k`: `Staged` only for an open group that lists exactly the declared
/// files of `k`'s member, every one stored; the digest is the declared one. A member the group lists
/// no files for is not reserved yet. A listing of other files than the declared ones is a broken
/// response, because the group id is a function of the set.
fn staged(group: &Group, member: &Member, k: &Key) -> Result<Presence, SeamError> {
    let listed = group
        .members
        .iter()
        .find(|g| {
            (g.name.as_str(), g.version.as_str()) == (member.name.as_str(), member.version.as_str())
        })
        .filter(|g| !g.files.is_empty());
    let Some(listed) = listed else {
        return Ok(Presence::Absent);
    };
    let exact = listed.files.len() == member.files.len()
        && listed.files.iter().all(|f| {
            member.files.iter().any(|d| {
                (d.subdir, d.file_name.as_str(), d.sha256.as_str(), d.size)
                    == (f.subdir, f.filename.as_str(), f.sha256.as_str(), f.size)
            })
        });
    if !exact {
        return Err(map_schema_violation(
            Op::Read,
            "a group lists files other than the declared ones",
        ));
    }
    let stored = listed.files.iter().all(|f| f.uploaded == Some(true));
    let declared = member.files.iter().find(|f| f.subdir == k.subdir);
    Ok(match (group.status, stored, declared) {
        (GroupStatus::Open, true, Some(f)) => Presence::Staged(Held {
            digest: f.sha256.clone(),
            stamp: None,
            exceptional: false,
        }),
        _ => Presence::Absent,
    })
}

impl<T: Transport> Registry<T> {
    /// A point read of `k`. With a non-empty declared `set` it reads the group the set names (with
    /// the caller's token) and then the version record (public), and the record wins; otherwise a
    /// group that is open, lists exactly the declared files and has every one stored is `Staged`.
    /// With an empty `set` only the record is read, and staged state is never reported. A
    /// non-empty `set` that does not hold `k` is `Refused`.
    pub fn observe(
        &mut self,
        caller: Option<&Caller<'_>>,
        k: &Key,
        set: &[Member],
    ) -> Result<Presence, SeamError> {
        let member = if set.is_empty() {
            None
        } else {
            Some(member_of(set, k).ok_or_else(refuse)?)
        };
        let group = match member {
            None => None,
            Some(_) => {
                let caller = caller.ok_or(SeamError::GrantRefused(GrantFault::Denied))?;
                self.read_group(caller, &group_id(caller.actor, set))?
            }
        };
        if let Some(record) = self.read_record(&k.name, &k.version)? {
            return Ok(presence_of(&record, k));
        }
        match (group, member) {
            (Some(group), Some(member)) => staged(&group, member, k),
            _ => Ok(Presence::Absent),
        }
    }

    /// The package file of the sparse index. An index that does not have the package yet is lag
    /// (or a package whose versions are all burned) and is no records; any other failure fails.
    fn read_index(&mut self, name: &str) -> Result<Vec<VersionRecord>, SeamError> {
        let padded: Vec<char> = name.chars().chain(std::iter::repeat('_')).take(4).collect();
        let first: String = padded.iter().take(2).collect();
        let second: String = padded.iter().skip(2).collect();
        let request = get(self
            .config
            .registry
            .rooted(&["index", &first, &second, name]));
        let response = self
            .transport
            .send(&request)
            .map_err(|e| map_transport(Op::Read, &e))?;
        if response.status == 404 && error_code(&response.body).as_deref() == Some("not_in_index") {
            return Ok(Vec::new());
        }
        match map_reply(Op::Read, &response, self.config.retry_after_ceiling) {
            None => parse_index(&response.body, name)
                .ok_or_else(|| map_schema_violation(Op::Read, "the sparse index")),
            Some(other) => Err(unexpected(other)),
        }
    }

    /// Every coordinate of `name` the channel holds, of any version and platform, yanked ones
    /// included, each with its digest. It compares nothing. A package nobody published, and a
    /// claimed package with no version, is an empty list. The package record lists the versions and
    /// the sparse index gives the digests, a cache that can lag the record by a minute, so a
    /// version it does not show yet is read from its own record and never left out. A version a
    /// failed group burned holds no file and is not listed. If any read fails the call fails, and a
    /// list of more versions than the configured cap is an error and never cut off.
    pub fn newest(
        &mut self,
        channel: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        if channel != self.config.channel || !valid_name(name) {
            return Err(refuse());
        }
        let request = get(self.config.registry.join(&["packages", name]));
        let response = self
            .transport
            .send(&request)
            .map_err(|e| map_transport(Op::Read, &e))?;
        let versions = match map_reply(Op::Read, &response, self.config.retry_after_ceiling) {
            None => parse_package(&response.body)
                .ok_or_else(|| map_schema_violation(Op::Read, "a package record"))?,
            Some(Mapped::Missing) => return Ok(Vec::new()),
            Some(other) => return Err(unexpected(other)),
        };
        if versions.len() > self.config.newest_versions {
            return Err(SeamError::Permanent(
                "the package lists more versions than the configured cap".to_owned(),
            ));
        }
        let index = self.read_index(name)?;
        let mut found = Vec::new();
        for (version, _) in &versions {
            let record = match index.iter().find(|r| r.version == *version) {
                Some(record) => record.clone(),
                None => self.read_record(name, version)?.ok_or_else(|| {
                    SeamError::Transient(
                        "a version the package lists has no record to read".to_owned(),
                    )
                })?,
            };
            found.extend(
                record
                    .files
                    .iter()
                    .map(|f| (coordinate(channel, name, version, f.subdir), held(f))),
            );
        }
        Ok(found)
    }

    /// The latest published version of every package of the channel whose name starts with the
    /// prefix, each file with its digest. For reports only: a package whose newest published
    /// version is a yanked or tombstoned one has no latest and is left out, and an older version is
    /// never listed, which is the lag a report allows. A failed read fails the call, and more pages
    /// than [`MAX_LIST_PAGES`] is an error.
    pub fn list(&mut self, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        if scope.channel != self.config.channel || !valid_prefix(&scope.prefix) {
            return Err(refuse());
        }
        let mut latest: Vec<(String, String)> = Vec::new();
        let mut cursor = String::new();
        for _ in 0..MAX_LIST_PAGES {
            let mut query = format!("sort=name&limit={PAGE}&q={}", encode_segment(&scope.prefix));
            if !cursor.is_empty() {
                query.push_str(&format!("&cursor={}", encode_segment(&cursor)));
            }
            let url = self.config.registry.join(&["packages"]).with_query(&query);
            let request = get(url);
            let response = self
                .transport
                .send(&request)
                .map_err(|e| map_transport(Op::Read, &e))?;
            if let Some(mapped) = map_reply(Op::Read, &response, self.config.retry_after_ceiling) {
                return Err(unexpected(mapped));
            }
            let page = parse_page(&response.body)
                .ok_or_else(|| map_schema_violation(Op::Read, "a page of packages"))?;
            latest.extend(
                page.packages
                    .into_iter()
                    .filter(|(name, _)| name.starts_with(&scope.prefix))
                    .filter_map(|(name, version)| version.map(|v| (name, v))),
            );
            match page.next_cursor {
                Some(next) => cursor = next,
                None => return self.listed(&scope.channel, &latest),
            }
        }
        Err(SeamError::Permanent(
            "the search lists more pages than the cap".to_owned(),
        ))
    }

    /// The files of the latest version of each of `latest`, read from its record. A record that
    /// is not there yet is lag and is left out.
    fn listed(
        &mut self,
        channel: &str,
        latest: &[(String, String)],
    ) -> Result<Vec<Listed>, SeamError> {
        let mut out = Vec::new();
        for (name, version) in latest {
            if let Some(record) = self.read_record(name, version)? {
                out.extend(record.files.iter().map(|f| Listed {
                    coordinate: coordinate(channel, name, version, f.subdir),
                    held: held(f),
                }));
            }
        }
        Ok(out)
    }
}
