// SPDX-License-Identifier: Apache-2.0
//! The calls on a mopm registry that a publisher makes through its [`Transport`]: the group read,
//! the version read, `stage` and `abort`. A [`Registry`] holds the transport, the configuration
//! and the clock, and decides every answer from what mopm says.
//!
//! The group a set is staged in is addressed by its id, which is a function of the actor and the
//! declared set ([`group_id`]), so a repeat of the same set resumes the same group and a different
//! set is another group. Each call takes the set as [`Member`]s, which [`members`] checks before
//! any request is built, and a [`Caller`], the actor and the bearer token a grant carries.
//!
//! [`members`]: crate::members

use serde_json::{Value, json};
use ucdk_core::{Clock, millis};
use ucdk_release::codec::GrantFault;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{SeamError, StageAnswer};

use crate::answers::{
    Created, Group, GroupStatus, Target, VersionRecord, VersionState, parse_created, parse_group,
    parse_record,
};
use crate::config::{Call, Config};
use crate::errors::{ConflictAnswer, ConflictReason, Mapped, Op, error_code, map_reply};
use crate::errors::{map_schema_violation, map_transport};
use crate::group_id::group_id;
use crate::mapping::{Member, MemberFile};
use crate::platform::Subdir;
use crate::requests::get;
use crate::transport::{Header, Method, Request, Response, Transport};
use crate::url::Url;

mod observe;

pub use observe::MAX_LIST_PAGES;

/// A presigned upload URL lives at most 300 seconds. A `PUT` starts only on an answer younger than
/// half of that, and otherwise the identical create is repeated first for fresh URLs.
const URL_REFRESH_MS: u64 = 150_000;

/// The statuses of a storage `PUT` that are retried.
const RETRY_STATUSES: [u16; 4] = [500, 502, 503, 504];

/// Who is calling: the actor an input of the group id, and the bearer token. The mopm grant
/// source builds them into the secret of a grant, and the publisher reads them out of it.
pub struct Caller<'a> {
    /// `u:<GitHub id>` for an account, `ci:gh-repo:<repository id>` for a configured CI identity.
    pub actor: &'a str,
    pub token: &'a [u8],
}

/// The bytes of one declared file, found by its package, version and subdirectory.
#[derive(Debug, Clone, Copy)]
pub struct SetFile<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub subdir: Subdir,
    pub bytes: &'a [u8],
}

/// How a step of `stage` ends other than by going on.
enum Early {
    /// The answer of the whole call.
    Answer(StageAnswer),
    Fail(SeamError),
}

impl From<SeamError> for Early {
    fn from(error: SeamError) -> Self {
        Early::Fail(error)
    }
}

/// Why a `PUT` of file bytes did not go through.
enum Put {
    /// Storage refused the URL: it is expired.
    Expired,
    Failed(SeamError),
}

/// What the steps of one `stage` call share.
struct Stage<'a> {
    caller: &'a Caller<'a>,
    members: &'a [Member],
    id: String,
    deadline: u64,
}

/// The latest create answer and when it came.
struct Upload {
    created: Created,
    at: u64,
}

fn refuse() -> SeamError {
    SeamError::Refused(Reason::Channel)
}

/// An answer that is an error of a kind the verb did not expect.
fn unexpected(mapped: Mapped) -> SeamError {
    match mapped {
        Mapped::Failed(error) => error,
        _ => SeamError::Transient("mopm answered in a way this call does not expect".to_owned()),
    }
}

fn create_body(members: &[Member]) -> Vec<u8> {
    let members: Vec<Value> = members
        .iter()
        .map(|m| {
            let files: Vec<Value> = m
                .files
                .iter()
                .map(|f| {
                    json!({
                        "subdir": f.subdir.as_str(),
                        "filename": f.file_name,
                        "sha256": f.sha256,
                        "size": f.size,
                    })
                })
                .collect();
            json!({"name": m.name, "version": m.version, "files": files})
        })
        .collect();
    json!({"members": members, "on_overlap": "refuse"})
        .to_string()
        .into_bytes()
}

/// The registry as a publisher reaches it.
pub struct Registry<T: Transport> {
    transport: T,
    config: Config,
    clock: Box<dyn Clock>,
    /// A create under `on_overlap: refuse` listed a superseded group, which contradicts the
    /// contract: this registry does not honour the option, and no later stage is sent.
    overlap_ignored: bool,
}

impl<T: Transport> Registry<T> {
    pub fn new(transport: T, config: Config, clock: Box<dyn Clock>) -> Self {
        Self {
            transport,
            config,
            clock,
            overlap_ignored: false,
        }
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// The time `call` may run until, from now.
    fn deadline(&self, call: Call) -> u64 {
        self.clock
            .now_millis()
            .saturating_add(millis(self.config.budget(call)))
    }

    /// Ends a stage that has used its budget, between requests: the group stays open and the next
    /// stage resumes it.
    fn within(&self, deadline: u64) -> Result<(), SeamError> {
        if self.clock.now_millis() >= deadline {
            return Err(SeamError::Transient(
                "the call ran out of its time budget; the group stays open and the next stage resumes it"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    /// A request that carries the caller's token as a bearer. A token that cannot be a header
    /// value is a grant the registry would refuse, so it is refused here with nothing sent.
    fn authorized<'b>(
        caller: &Caller<'_>,
        method: Method,
        url: Url,
        body: &'b [u8],
    ) -> Result<Request<'b>, SeamError> {
        let bearer =
            Header::bearer(caller.token).ok_or(SeamError::GrantRefused(GrantFault::Denied))?;
        let mut headers = vec![Header::plain("accept", "application/json"), bearer];
        if !body.is_empty() {
            headers.push(Header::plain("content-type", "application/json"));
        }
        Ok(Request {
            method,
            url,
            headers,
            body,
        })
    }

    /// `GET /groups/{id}`: `None` only for a 404 `group_not_found`. Any other failure of the read
    /// is an error and never "no group".
    pub fn read_group(
        &mut self,
        caller: &Caller<'_>,
        id: &str,
    ) -> Result<Option<Group>, SeamError> {
        let url = self.config.registry.join(&["groups", id]);
        let request = Self::authorized(caller, Method::Get, url, &[])?;
        let response = self
            .transport
            .send(&request)
            .map_err(|e| map_transport(Op::Read, &e))?;
        if response.status == 404
            && error_code(&response.body).as_deref() == Some("group_not_found")
        {
            return Ok(None);
        }
        if let Some(mapped) = map_reply(Op::Read, &response, self.config.retry_after_ceiling) {
            return Err(unexpected(mapped));
        }
        let group = parse_group(&response.body)
            .filter(|g| g.id == id)
            .ok_or_else(|| map_schema_violation(Op::Read, "a group read"))?;
        Ok(Some(group))
    }

    /// `GET /packages/{name}/{version}`, public: `None` for a 404 `package_not_found` or
    /// `version_not_found`. The record names the package and version it was asked for, byte for
    /// byte, or it is a broken response.
    pub fn read_record(
        &mut self,
        name: &str,
        version: &str,
    ) -> Result<Option<VersionRecord>, SeamError> {
        let request = get(self.config.registry.join(&["packages", name, version]));
        let response = self
            .transport
            .send(&request)
            .map_err(|e| map_transport(Op::Read, &e))?;
        match map_reply(Op::Read, &response, self.config.retry_after_ceiling) {
            None => parse_record(&response.body)
                .filter(|r| (r.name.as_str(), r.version.as_str()) == (name, version))
                .map(Some)
                .ok_or_else(|| map_schema_violation(Op::Read, "a version record")),
            Some(Mapped::Missing) => Ok(None),
            Some(other) => Err(unexpected(other)),
        }
    }

    /// The record of every member's version, in the order of `members`.
    fn records(&mut self, members: &[Member]) -> Result<Vec<Option<VersionRecord>>, SeamError> {
        members
            .iter()
            .map(|m| self.read_record(&m.name, &m.version))
            .collect()
    }

    /// Stages the set `members` in its group and uploads every file the registry does not hold.
    ///
    /// `Duplicate` when nothing had to be uploaded: the group is already open with every declared
    /// file stored, is past open, or the numbers are held by a group that stands. `Staged` when
    /// bytes were added. The same set sent again resumes the same group; a different set that
    /// shares a member is refused by the registry (`on_overlap: refuse`) and ends nothing.
    pub fn stage(
        &mut self,
        caller: &Caller<'_>,
        members: &[Member],
        files: &[SetFile<'_>],
    ) -> Result<StageAnswer, SeamError> {
        match self.stage_set(caller, members, files) {
            Ok(answer) | Err(Early::Answer(answer)) => Ok(answer),
            Err(Early::Fail(error)) => Err(error),
        }
    }

    fn stage_set(
        &mut self,
        caller: &Caller<'_>,
        members: &[Member],
        files: &[SetFile<'_>],
    ) -> Result<StageAnswer, Early> {
        if self.overlap_ignored {
            return Err(refuse().into());
        }
        let plan = members
            .iter()
            .flat_map(|m| m.files.iter().map(move |f| (m, f)))
            .map(|(m, f)| bytes_of(files, m, f).map(|bytes| (m, f, bytes)))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| {
                SeamError::Permanent(
                    "a declared file has no bytes, or bytes of another size".to_owned(),
                )
            })?;
        let stage = Stage {
            caller,
            members,
            id: group_id(caller.actor, members),
            deadline: self.deadline(Call::Stage),
        };
        self.within(stage.deadline)?;
        let existing = self.read_group(caller, &stage.id)?;
        if existing.is_some_and(|g| g.holds_every_file(members)) {
            return Ok(StageAnswer::Duplicate);
        }
        let mut upload = Upload {
            created: self.create(&stage)?,
            at: self.clock.now_millis(),
        };
        let mut sent = false;
        for (member, file, bytes) in plan {
            sent |= self.upload_one(&stage, &mut upload, (member, file), bytes)?;
        }
        Ok(if sent {
            StageAnswer::Staged
        } else {
            StageAnswer::Duplicate
        })
    }

    /// `POST /groups` with the declared set, repeated unchanged while the registry says it is still
    /// reserving (at most `reserving_repeats` times more).
    fn create(&mut self, stage: &Stage<'_>) -> Result<Created, Early> {
        let body = create_body(stage.members);
        for _ in 0..=self.config.reserving_repeats {
            self.within(stage.deadline)?;
            let url = self.config.registry.join(&["groups"]);
            let request = Self::authorized(stage.caller, Method::Post, url, &body)?;
            let response = self
                .transport
                .send(&request)
                .map_err(|e| map_transport(Op::Create, &e))?;
            let created = self.created(stage, &response)?;
            if !created.reserving {
                return Ok(created);
            }
        }
        Err(Early::Fail(SeamError::Transient(
            "the registry is still reserving the set; the next stage resumes it".to_owned(),
        )))
    }

    /// What one create answer means: the group it names, or the end of the call.
    fn created(&mut self, stage: &Stage<'_>, response: &Response) -> Result<Created, Early> {
        if let Some(mapped) = map_reply(Op::Create, response, self.config.retry_after_ceiling) {
            return Err(match mapped {
                Mapped::Duplicate => self.after_version_exists(stage.members),
                Mapped::Conflict(reason) => self.after_conflict(stage.members, reason),
                other => Early::Fail(unexpected(other)),
            });
        }
        let created = parse_created(&response.body)
            .ok_or_else(|| map_schema_violation(Op::Create, "a group create"))?;
        if created.id != stage.id {
            return Err(self.drifted(stage, &created.id));
        }
        if !created.superseded.is_empty() {
            self.overlap_ignored = true;
            return Err(refuse().into());
        }
        Ok(created)
    }

    /// 409 `version_exists`: the numbers are held, yanked or burned. A tombstoned record is a held
    /// number (the engine's observe reads it as burned). A published or yanked version that lacks
    /// a requested coordinate is sealed with another file set, which `Refused` says; every other
    /// case is `Duplicate`.
    fn after_version_exists(&mut self, members: &[Member]) -> Early {
        let records = match self.records(members) {
            Ok(records) => records,
            Err(error) => return Early::Fail(error),
        };
        if records.iter().any(Option::is_none) {
            return Early::Fail(SeamError::Transient(
                "mopm says the version exists but its record cannot be read".to_owned(),
            ));
        }
        let sealed_otherwise = members.iter().zip(&records).any(|(m, record)| {
            record.as_ref().is_some_and(|r| {
                r.state != VersionState::Tombstoned && !m.files.iter().all(|f| holds(r, f))
            })
        });
        if sealed_otherwise {
            return Early::Fail(refuse());
        }
        Early::Answer(StageAnswer::Duplicate)
    }

    /// 409 `group_conflict`: the class of its reason, and for `number_taken` a read of the records.
    fn after_conflict(&mut self, members: &[Member], reason: ConflictReason) -> Early {
        match reason.answer() {
            ConflictAnswer::Duplicate => Early::Answer(StageAnswer::Duplicate),
            ConflictAnswer::Fail(error) => Early::Fail(error),
            ConflictAnswer::DuplicateIfHeld => match self.records(members) {
                Err(error) => Early::Fail(error),
                Ok(records) => {
                    let held = members.iter().zip(&records).all(|(m, record)| {
                        record.as_ref().is_some_and(|r| {
                            r.state != VersionState::Tombstoned
                                && m.files.iter().all(|f| holds(r, f))
                        })
                    });
                    if held {
                        Early::Answer(StageAnswer::Duplicate)
                    } else {
                        Early::Fail(SeamError::Transient(
                            "another publish took a number and the version is not held".to_owned(),
                        ))
                    }
                }
            },
        }
    }

    /// The create answer names a group other than the one computed from the actor and the set, so
    /// the derivation or the actor string drifted. The group it names is aborted, best effort:
    /// `Permanent` when that worked, `Unknown` when it did not.
    fn drifted(&mut self, stage: &Stage<'_>, named: &str) -> Early {
        let url = self.config.registry.join(&["groups", named]);
        let aborted = Self::authorized(stage.caller, Method::Delete, url, &[])
            .ok()
            .is_some_and(|request| {
                self.transport
                    .send(&request)
                    .is_ok_and(|response| Response::is_success(&response))
            });
        Early::Fail(if aborted {
            SeamError::Permanent(
                "the group id derivation drifted: the registry named another group than the one computed; the group it named was aborted"
                    .to_owned(),
            )
        } else {
            SeamError::Unknown(
                "the group id derivation drifted and the group the registry named could not be aborted"
                    .to_owned(),
            )
        })
    }

    /// Repeats the identical create for fresh upload URLs.
    fn refresh(&mut self, stage: &Stage<'_>, upload: &mut Upload) -> Result<(), Early> {
        upload.created = self.create(stage)?;
        upload.at = self.clock.now_millis();
        Ok(())
    }

    /// Uploads one file if the registry lists it as not stored; whether bytes were sent.
    fn upload_one(
        &mut self,
        stage: &Stage<'_>,
        upload: &mut Upload,
        (member, file): (&Member, &MemberFile),
        bytes: &[u8],
    ) -> Result<bool, Early> {
        if self.clock.now_millis().saturating_sub(upload.at) > URL_REFRESH_MS {
            self.refresh(stage, upload)?;
        }
        let mut refreshed = false;
        loop {
            let Some(target) = upload.created.pending(member, file) else {
                return Ok(false);
            };
            match self.put(target, bytes, stage.deadline) {
                Ok(()) => return Ok(true),
                Err(Put::Expired) if !refreshed => {
                    refreshed = true;
                    self.refresh(stage, upload)?;
                }
                Err(Put::Expired) => {
                    return Err(Early::Fail(SeamError::Transient(
                        "an upload URL expired again".to_owned(),
                    )));
                }
                Err(Put::Failed(error)) => return Err(Early::Fail(error)),
            }
        }
    }

    /// `PUT` of the bytes to the URL the create answer gave, with its headers and no token. A
    /// connect failure, a timeout and a 5xx are retried `retries` times.
    fn put(&mut self, target: &Target, bytes: &[u8], deadline: u64) -> Result<(), Put> {
        if !self.config.allows_file_origin(&target.url) {
            return Err(Put::Failed(SeamError::Transient(
                "an upload URL names an origin that is not configured".to_owned(),
            )));
        }
        let mut last = String::new();
        for _ in 0..=self.config.retries {
            self.within(deadline).map_err(Put::Failed)?;
            let request = Request {
                method: Method::Put,
                url: target.url.clone(),
                headers: target
                    .headers
                    .iter()
                    .map(|(name, value)| Header::plain(name, value))
                    .collect(),
                body: bytes,
            };
            match self.transport.send(&request) {
                Ok(r) if r.is_success() => return Ok(()),
                Ok(r) if r.status == 403 => return Err(Put::Expired),
                Ok(r) if RETRY_STATUSES.contains(&r.status) => {
                    last = format!("storage answered {}", r.status);
                }
                Ok(r) => {
                    return Err(Put::Failed(SeamError::Transient(format!(
                        "storage answered {} to an upload",
                        r.status
                    ))));
                }
                Err(e) => last = e.to_string(),
            }
        }
        Err(Put::Failed(SeamError::Transient(format!(
            "an upload failed after {} tries: {last}",
            self.config.retries.saturating_add(1)
        ))))
    }

    /// Gives back the staged state of the set `members`: the group its id names. The set is exact,
    /// so another set is another group, a 404 and nothing given back. An empty set gives back
    /// nothing and sends nothing.
    ///
    /// `Ok` when the group is aborted, is gone, ended some other way (nothing to give back), or
    /// is committed (an abort touches nothing of it). `InFlight` while the group is validating: it
    /// cannot be aborted and proceeds to publish. `Unknown` when a 404 is not confirmed by the
    /// group read, or the answer was lost.
    pub fn abort(&mut self, caller: &Caller<'_>, members: &[Member]) -> Result<(), SeamError> {
        if members.is_empty() {
            return Ok(());
        }
        let id = group_id(caller.actor, members);
        let deadline = self.deadline(Call::Abort);
        let url = self.config.registry.join(&["groups", &id]);
        let request = Self::authorized(caller, Method::Delete, url, &[])?;
        let response = self
            .transport
            .send(&request)
            .map_err(|e| map_transport(Op::Abort, &e))?;
        let code = error_code(&response.body);
        let follow_up = matches!(
            (response.status, code.as_deref()),
            (404, Some("group_not_found")) | (409, Some("group_not_open"))
        );
        if !follow_up {
            return match map_reply(Op::Abort, &response, self.config.retry_after_ceiling) {
                None => Ok(()),
                Some(mapped) => Err(unexpected(mapped)),
            };
        }
        if self.clock.now_millis() >= deadline {
            return Err(SeamError::Unknown(
                "the abort ran out of its time budget before the group could be read".to_owned(),
            ));
        }
        let group = self.read_group(caller, &id).map_err(|e| {
            SeamError::Unknown(format!(
                "the group could not be read after the abort: {e:?}"
            ))
        })?;
        match (response.status == 404, group.map(|g| g.status)) {
            (true, None) | (false, None) => Ok(()),
            (true, Some(_)) => Err(SeamError::Unknown(
                "mopm answered 404 for a group that a read then shows".to_owned(),
            )),
            (false, Some(GroupStatus::Validating)) => Err(SeamError::InFlight(
                "the group is validating and proceeds to publish; observe settles it".to_owned(),
            )),
            (false, Some(GroupStatus::Open)) => Err(SeamError::Unknown(
                "the group is still open after the registry said it is not".to_owned(),
            )),
            (false, Some(_)) => Ok(()),
        }
    }
}

/// The bytes of `file` of `member`, if `files` has them and they are of the declared size.
fn bytes_of<'a>(files: &[SetFile<'a>], member: &Member, file: &MemberFile) -> Option<&'a [u8]> {
    files
        .iter()
        .find(|f| {
            (f.name, f.version, f.subdir)
                == (member.name.as_str(), member.version.as_str(), file.subdir)
        })
        .map(|f| f.bytes)
        .filter(|bytes| u64::try_from(bytes.len()) == Ok(file.size))
}

/// Whether `record` holds a file for the subdirectory of `file`.
fn holds(record: &VersionRecord, file: &MemberFile) -> bool {
    record.files.iter().any(|f| f.subdir == file.subdir)
}
