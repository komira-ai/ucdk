// SPDX-License-Identifier: Apache-2.0
//! The bodies mopm answers with on success, read into plain types: the group a create or a read
//! describes, and the record of a version. A body that does not have the shape of mopm's response
//! schema is `None`, and the caller makes a schema violation of it. Nothing here is lenient: a
//! field of the wrong type, an unknown status or subdirectory and a malformed digest all fail the
//! whole body, so that nothing is decided on a half-read answer.

use serde_json::Value;

use crate::grammar::valid_sha256;
use crate::group_id::valid_group_id;
use crate::mapping::{Member, MemberFile};
use crate::platform::Subdir;
use crate::url::Url;

/// Where a group is. A value mopm does not list is not one this crate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupStatus {
    Open,
    Validating,
    Committed,
    Visible,
    Rejected,
    Failed,
    Expired,
    Aborted,
    Superseded,
}

impl GroupStatus {
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "open" => Self::Open,
            "validating" => Self::Validating,
            "committed" => Self::Committed,
            "visible" => Self::Visible,
            "rejected" => Self::Rejected,
            "failed" => Self::Failed,
            "expired" => Self::Expired,
            "aborted" => Self::Aborted,
            "superseded" => Self::Superseded,
            _ => return None,
        })
    }
}

/// One declared file of a member, as `GET /groups/{id}` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFact {
    pub subdir: Subdir,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
    /// Present only while the group is open: the registry found the object at the declared size.
    pub uploaded: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMember {
    pub name: String,
    pub version: String,
    pub files: Vec<FileFact>,
}

/// A group as `GET /groups/{id}` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub id: String,
    pub status: GroupStatus,
    pub members: Vec<GroupMember>,
}

/// Where and how to `PUT` a file the registry does not hold yet. The headers go with the `PUT`
/// exactly as given, and the bearer token never does.
#[derive(Clone)]
pub struct Target {
    pub url: Url,
    pub headers: Vec<(String, String)>,
}

/// One file of a create answer. `target` is present only for a file that is not stored yet.
#[derive(Clone)]
pub struct UploadFile {
    pub subdir: Subdir,
    pub filename: String,
    pub target: Option<Target>,
}

#[derive(Clone)]
pub struct MemberAnswer {
    pub name: String,
    pub version: String,
    pub files: Vec<UploadFile>,
}

/// The answer to `POST /groups`.
#[derive(Clone)]
pub struct Created {
    pub id: String,
    pub status: GroupStatus,
    /// The call ran out of time before it reached every member: the same request is sent again.
    pub reserving: bool,
    /// The groups of the caller this create ended.
    pub superseded: Vec<String>,
    pub members: Vec<MemberAnswer>,
}

/// The state of a version record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionState {
    Published,
    Yanked,
    Tombstoned,
}

impl VersionState {
    /// The state a version record or version list names; `None` for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "published" => Self::Published,
            "yanked" => Self::Yanked,
            "tombstoned" => Self::Tombstoned,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordFile {
    pub subdir: Subdir,
    pub filename: String,
    pub sha256: String,
    pub size: u64,
    /// Absent for a tombstoned version.
    pub url: Option<String>,
}

/// A version as `GET /packages/{name}/{version}` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRecord {
    pub name: String,
    pub version: String,
    pub state: VersionState,
    pub files: Vec<RecordFile>,
    /// The group that published the version, when one did.
    pub group_id: Option<String>,
}

impl Group {
    /// Whether the group is open and lists every declared file of every member, each with the
    /// declared digest and size and found in storage.
    pub fn holds_every_file(&self, members: &[Member]) -> bool {
        let open = self.status == GroupStatus::Open;
        open && members.iter().all(|m| {
            self.members
                .iter()
                .find(|g| {
                    (g.name.as_str(), g.version.as_str()) == (m.name.as_str(), m.version.as_str())
                })
                .is_some_and(|g| m.files.iter().all(|f| g.files.contains(&stored(f))))
        })
    }
}

/// The fact a group lists for `file` once its bytes are in storage.
fn stored(file: &MemberFile) -> FileFact {
    FileFact {
        subdir: file.subdir,
        filename: file.file_name.clone(),
        sha256: file.sha256.clone(),
        size: file.size,
        uploaded: Some(true),
    }
}

impl Created {
    /// The target of the `PUT` of `file` of `member`, if the registry lists the file as not stored.
    pub fn pending(&self, member: &Member, file: &MemberFile) -> Option<&Target> {
        self.members
            .iter()
            .find(|a| {
                (a.name.as_str(), a.version.as_str())
                    == (member.name.as_str(), member.version.as_str())
            })
            .and_then(|a| {
                a.files.iter().find(|f| {
                    (f.subdir, f.filename.as_str()) == (file.subdir, file.file_name.as_str())
                })
            })
            .and_then(|f| f.target.as_ref())
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn digest<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    text(value, key).filter(|d| valid_sha256(d))
}

fn array<T>(value: &Value, key: &str, one: fn(&Value) -> Option<T>) -> Option<Vec<T>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .and_then(|items| items.iter().map(one).collect())
}

fn file_fact(value: &Value) -> Option<FileFact> {
    let subdir = text(value, "subdir").and_then(Subdir::parse);
    let size = value.get("size").and_then(Value::as_u64);
    match (
        subdir,
        text(value, "filename"),
        digest(value, "sha256"),
        size,
    ) {
        (Some(subdir), Some(filename), Some(sha256), Some(size)) => Some(FileFact {
            subdir,
            filename: filename.to_owned(),
            sha256: sha256.to_owned(),
            size,
            uploaded: value.get("uploaded").and_then(Value::as_bool),
        }),
        _ => None,
    }
}

fn group_member(value: &Value) -> Option<GroupMember> {
    match (
        text(value, "name"),
        text(value, "version"),
        array(value, "files", file_fact),
    ) {
        (Some(name), Some(version), Some(files)) => Some(GroupMember {
            name: name.to_owned(),
            version: version.to_owned(),
            files,
        }),
        _ => None,
    }
}

fn group_id_of(value: &Value) -> Option<&str> {
    text(value, "group_id").filter(|id| valid_group_id(id))
}

/// The body of `GET /groups/{id}`.
pub fn parse_group(body: &[u8]) -> Option<Group> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let status = text(&value, "status").and_then(GroupStatus::parse);
    match (
        group_id_of(&value),
        status,
        array(&value, "members", group_member),
    ) {
        (Some(id), Some(status), Some(members)) => Some(Group {
            id: id.to_owned(),
            status,
            members,
        }),
        _ => None,
    }
}

fn target(value: &Value) -> Option<Target> {
    let url = text(value, "put_url").and_then(|u| Url::parse(u).ok());
    let headers: Option<Vec<(String, String)>> = match value.get("put_headers") {
        None => Some(Vec::new()),
        Some(map) => map.as_object().and_then(|map| {
            map.iter()
                .map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                .collect()
        }),
    };
    match (url, headers) {
        (Some(url), Some(headers)) => Some(Target { url, headers }),
        _ => None,
    }
}

fn upload_file(value: &Value) -> Option<UploadFile> {
    let subdir = text(value, "subdir").and_then(Subdir::parse);
    let stored = value.get("uploaded").and_then(Value::as_bool);
    match (subdir, text(value, "filename"), stored) {
        (Some(subdir), Some(filename), Some(true)) => Some(UploadFile {
            subdir,
            filename: filename.to_owned(),
            target: None,
        }),
        (Some(subdir), Some(filename), Some(false)) => target(value).map(|t| UploadFile {
            subdir,
            filename: filename.to_owned(),
            target: Some(t),
        }),
        _ => None,
    }
}

fn member_answer(value: &Value) -> Option<MemberAnswer> {
    match (
        text(value, "name"),
        text(value, "version"),
        array(value, "files", upload_file),
    ) {
        (Some(name), Some(version), Some(files)) => Some(MemberAnswer {
            name: name.to_owned(),
            version: version.to_owned(),
            files,
        }),
        _ => None,
    }
}

fn group_ids(value: &Value, key: &str) -> Option<Vec<String>> {
    match value.get(key) {
        None => Some(Vec::new()),
        Some(list) => list.as_array().and_then(|ids| {
            ids.iter()
                .map(|id| id.as_str().map(str::to_owned))
                .collect()
        }),
    }
}

/// The body of the answer to `POST /groups`.
pub fn parse_created(body: &[u8]) -> Option<Created> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let status = text(&value, "status").and_then(GroupStatus::parse);
    let members = array(&value, "members", member_answer);
    match (
        group_id_of(&value),
        status,
        members,
        group_ids(&value, "superseded"),
    ) {
        (Some(id), Some(status), Some(members), Some(superseded)) => Some(Created {
            id: id.to_owned(),
            status,
            reserving: value.get("reserving").is_some(),
            superseded,
            members,
        }),
        _ => None,
    }
}

fn record_file(value: &Value) -> Option<RecordFile> {
    let subdir = text(value, "subdir").and_then(Subdir::parse);
    let size = value.get("size").and_then(Value::as_u64);
    match (
        subdir,
        text(value, "filename"),
        digest(value, "sha256"),
        size,
    ) {
        (Some(subdir), Some(filename), Some(sha256), Some(size)) => Some(RecordFile {
            subdir,
            filename: filename.to_owned(),
            sha256: sha256.to_owned(),
            size,
            url: text(value, "url").map(str::to_owned),
        }),
        _ => None,
    }
}

/// The body of `GET /packages/{name}/{version}`. A record that shows two files for one
/// subdirectory is a broken response, since one version holds at most one file per subdirectory.
pub fn parse_record(body: &[u8]) -> Option<VersionRecord> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let state = text(&value, "state").and_then(VersionState::parse);
    let group_id = match value.get("group_id") {
        None => Some(None),
        Some(_) => group_id_of(&value).map(|id| Some(id.to_owned())),
    };
    match (
        text(&value, "name"),
        text(&value, "version"),
        state,
        array(&value, "files", record_file),
        group_id,
    ) {
        (Some(name), Some(version), Some(state), Some(files), Some(group_id))
            if files
                .iter()
                .enumerate()
                .all(|(i, f)| files[..i].iter().all(|g| g.subdir != f.subdir)) =>
        {
            Some(VersionRecord {
                name: name.to_owned(),
                version: version.to_owned(),
                state,
                files,
                group_id,
            })
        }
        _ => None,
    }
}
