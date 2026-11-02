// SPDX-License-Identifier: Apache-2.0
//! The bodies mopm answers with on success, read into plain types: the group a create or a read
//! describes, and the record of a version. A body that does not have the shape of mopm's response
//! schema is `None`, and the caller makes a schema violation of it. Nothing here is lenient: a
//! field of the wrong type (an optional one that is present included), an unknown status or
//! subdirectory, a malformed digest or group id, a file of an open group without `uploaded`, a
//! file of a version that is not tombstoned without its `url`, and an upload header that is not a
//! plain header or that is a credential all fail the whole body, so that nothing is decided on a
//! half-read answer. Fields this crate does not read are ignored.

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
/// exactly as given, and the bearer token never does: a header named `authorization` or
/// `proxy-authorization` in any case, a name that is not an HTTP token, a value that is not
/// visible ASCII, spaces and tabs, and a name given twice are refused when the answer is read.
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
    /// The call ran out of time before it reached every member (`reserving: {done, total}`): the
    /// same request is sent again.
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
    /// Whether a finished answer (not `reserving`) answers the declared set `members` and nothing
    /// else. An open group lists each declared member once, and each of its declared files once,
    /// stored or with a target; a group past open (`validating`, `committed`, `visible`) lists each
    /// declared member once, with no files. Any other status, a member or file missing, given
    /// twice or not declared does not answer the set, and nothing is uploaded on it. A member of an
    /// open group that lists no files is not read as stored: mopm answers `skip_identical` with no
    /// files only for a group past open, and a set whose files are all stored is `Duplicate` by the
    /// group read before any create is sent.
    pub fn answers(&self, members: &[Member]) -> bool {
        let declared = |a: &MemberAnswer| {
            members.iter().find(|m| {
                (m.name.as_str(), m.version.as_str()) == (a.name.as_str(), a.version.as_str())
            })
        };
        let once = self.members.iter().enumerate().all(|(i, a)| {
            self.members[..i].iter().all(|b| {
                (b.name.as_str(), b.version.as_str()) != (a.name.as_str(), a.version.as_str())
            })
        });
        match self.status {
            GroupStatus::Open => {
                once && self.members.len() == members.len()
                    && self.members.iter().all(|a| {
                        declared(a).is_some_and(|m| {
                            a.files.len() == m.files.len()
                                && m.files.iter().all(|f| {
                                    a.files
                                        .iter()
                                        .filter(|u| {
                                            (u.subdir, u.filename.as_str())
                                                == (f.subdir, f.file_name.as_str())
                                        })
                                        .count()
                                        == 1
                                })
                        })
                    })
            }
            GroupStatus::Validating | GroupStatus::Committed | GroupStatus::Visible => {
                once && self.members.len() == members.len()
                    && self
                        .members
                        .iter()
                        .all(|a| a.files.is_empty() && declared(a).is_some())
            }
            _ => false,
        }
    }

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

/// An optional field: `Some(None)` when it is absent, `None` when it is present and not what
/// `read` takes, so that a malformed optional field fails the body and is never read as absent.
fn optional<'a, T>(
    value: &'a Value,
    key: &str,
    read: fn(&'a Value) -> Option<T>,
) -> Option<Option<T>> {
    match value.get(key) {
        None => Some(None),
        Some(field) => read(field).map(Some),
    }
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
        optional(value, "uploaded", Value::as_bool),
    ) {
        (Some(subdir), Some(filename), Some(sha256), Some(size), Some(uploaded)) => {
            Some(FileFact {
                subdir,
                filename: filename.to_owned(),
                sha256: sha256.to_owned(),
                size,
                uploaded,
            })
        }
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
        (Some(id), Some(status), Some(members))
            if status != GroupStatus::Open
                || members
                    .iter()
                    .all(|m| m.files.iter().all(|f| f.uploaded.is_some())) =>
        {
            Some(Group {
                id: id.to_owned(),
                status,
                members,
            })
        }
        _ => None,
    }
}

fn target(value: &Value) -> Option<Target> {
    let url = text(value, "put_url").and_then(|u| Url::parse(u).ok());
    let headers: Option<Vec<(String, String)>> = match value.get("put_headers") {
        None => Some(Vec::new()),
        Some(map) => map.as_object().and_then(|map| {
            map.iter()
                .map(|(k, v)| {
                    v.as_str()
                        .filter(|v| upload_header(k, v))
                        .map(|v| (k.clone(), v.to_owned()))
                })
                .collect()
        }),
    };
    let distinct = headers.as_ref().is_some_and(|h| {
        h.iter()
            .enumerate()
            .all(|(i, (k, _))| h[..i].iter().all(|(j, _)| !j.eq_ignore_ascii_case(k)))
    });
    match (url, headers) {
        (Some(url), Some(headers)) if distinct => Some(Target { url, headers }),
        _ => None,
    }
}

/// Whether `name: value` may go with a storage `PUT`: the name is an HTTP token and is not a
/// credential header, and the value is visible ASCII, spaces and tabs.
fn upload_header(name: &str, value: &str) -> bool {
    const TOKEN_MARKS: &[u8] = b"!#$%&'*+-.^_`|~";
    let token = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || TOKEN_MARKS.contains(&b));
    let credential = ["authorization", "proxy-authorization"]
        .iter()
        .any(|c| name.eq_ignore_ascii_case(c));
    let plain = value
        .bytes()
        .all(|b| b.is_ascii_graphic() || b == b' ' || b == b'\t');
    token && !credential && plain
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
                .map(|id| {
                    id.as_str()
                        .filter(|id| valid_group_id(id))
                        .map(str::to_owned)
                })
                .collect()
        }),
    }
}

/// `reserving`: absent, or `{done, total}` with two counts.
fn reserving(value: &Value) -> Option<bool> {
    let counts = |field: &Value| {
        let count = |key| field.get(key).and_then(Value::as_u64);
        (field.is_object() && count("done").is_some() && count("total").is_some()).then_some(())
    };
    optional(value, "reserving", counts).map(|r| r.is_some())
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
        reserving(&value),
    ) {
        (Some(id), Some(status), Some(members), Some(superseded), Some(reserving)) => {
            Some(Created {
                id: id.to_owned(),
                status,
                reserving,
                superseded,
                members,
            })
        }
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
        optional(value, "url", Value::as_str),
    ) {
        (Some(subdir), Some(filename), Some(sha256), Some(size), Some(url)) => Some(RecordFile {
            subdir,
            filename: filename.to_owned(),
            sha256: sha256.to_owned(),
            size,
            url: url.map(str::to_owned),
        }),
        _ => None,
    }
}

/// The body of `GET /packages/{name}/{version}`. A record that shows two files for one
/// subdirectory is a broken response, since one version holds at most one file per subdirectory,
/// and so is a published or yanked version with a file that has no `url`.
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
                .all(|(i, f)| files[..i].iter().all(|g| g.subdir != f.subdir))
                && (state == VersionState::Tombstoned || files.iter().all(|f| f.url.is_some())) =>
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
