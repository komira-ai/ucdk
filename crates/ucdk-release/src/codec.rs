// SPDX-License-Identifier: Apache-2.0
//! The record codec: how each release record is written as the fields a ledger record has (a
//! tag, a readable name and a string map), and read back.
//!
//! The payload is a string map whose keys come from a closed list per kind, and the list has no
//! key for a credential, so the ledger never holds a secret. A reader refuses a key that is not
//! on the list, a missing required key and a value that is not in the form the writer writes,
//! and the writer refuses to write what the reader would refuse. A record's path is
//! `<tag>/k<key>`, the key hashed from the readable name, which is kept in the record because
//! versions hold characters a path segment does not allow.
//!
//! A name that is made of parts (run id, stage, action, platform, revision) and a generation are held
//! to the rule of the parts of a store key (`store::is_plain`), so a part read back from a
//! record can be put into a key without meeting another part.

use std::collections::BTreeMap;
use std::fmt;

use ucdk_core::{Hash256, Locator, ModelError, RecordKey, RecordPath, Tag};

use crate::store::is_plain;
use crate::transitions::Kind;

/// The payload keys of a kind, in the order the writer writes them.
pub fn keys_of(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Run => &[
            "machine",
            "revision",
            "version",
            "build",
            "commit",
            "timestamp",
        ],
        Kind::Stage => &["attempt", "platform", "set", "generation", "confirmed"],
        Kind::Action => &["input", "set", "generation"],
        Kind::Publication => &[
            "digest",
            "build",
            "commit",
            "exceptional",
            "run",
            "stage",
            "attempt",
            "phase",
        ],
        Kind::Validation => &["set"],
        Kind::Grant => &[
            "phase",
            "channel",
            "repository",
            "subject",
            "audience",
            "scope",
            "coordinates",
            "outcome",
            "fault",
            "expires_ms",
        ],
        Kind::Seal => &[
            "set",
            "generation",
            "inputs",
            "environment",
            "machine",
            "version",
            "build",
            "commit",
            "timestamp",
            "run",
            "attempt",
        ],
    }
}

/// A record as a ledger holds it: the fields the codec fills. No kind has a locator, so
/// `physical` is always `None` in what the codec writes and must be `None` in what it reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    pub kind: Tag,
    pub name: String,
    pub physical: Option<Locator>,
    pub outputs: BTreeMap<String, String>,
}

impl Encoded {
    /// The ledger path of the record: its tag, then `k` and the key of its name.
    pub fn path(&self) -> Result<RecordPath, ModelError> {
        RecordPath::new(&format!(
            "{}/{}",
            self.kind,
            RecordKey::of_name(&self.name).segment()
        ))
    }
}

/// The run's pinned inputs, written once by the first stage that holds a Build or Test action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub run_id: String,
    pub machine: Hash256,
    pub revision: String,
    pub version: String,
    pub build: u64,
    pub commit: String,
    pub timestamp: u64,
}

/// Where one stage of the run stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    pub stage: String,
    /// The latest attempt of this stage, from 1.
    pub attempt: u32,
    /// The stage's platform; none for a stage that names none (a Test-only stage, or a
    /// `CHANGE_CHECK` one), in which case the key is absent. Present, it is a part of a store
    /// key, so it is never empty.
    pub platform: Option<String>,
    /// The sealed set the stage built or consumes, and the store generation that holds it.
    pub set: Option<Hash256>,
    pub generation: Option<String>,
    /// Every action and every validation of the stage is confirmed.
    pub confirmed: bool,
}

/// Where one action of a stage stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub stage: String,
    pub action: String,
    /// The action input digest.
    pub input: Hash256,
    pub set: Option<Hash256>,
    /// For a Build action, the generation its output was written into.
    pub generation: Option<String>,
}

/// Which effect of a publication may have landed while its record is an intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Staging,
    Committing,
}

/// One published coordinate. Whether its bytes were found rather than written is the record's
/// origin, not a payload key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    pub coordinate: String,
    pub digest: Hash256,
    pub build: u64,
    pub commit: String,
    pub exceptional: bool,
    /// Which run, stage and attempt wrote it, so a reader knows whose leftover intent it sees.
    pub run: String,
    pub stage: String,
    pub attempt: u32,
    /// For an intent, the phase whose effect may have landed; absent once confirmed.
    pub phase: Option<Phase>,
}

/// One validation of an action or of a stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    pub stage: String,
    pub validation: String,
    /// The set the validation read; absent for a read-back.
    pub set: Option<Hash256>,
}

/// What a grant was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantPhase {
    Stage,
    Commit,
    Read,
}

/// Why a grant exchange was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantFault {
    Denied,
    Expired,
    Used,
    Unreachable,
}

/// How a grant exchange was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantAnswer {
    /// The issuer granted it until `expires_ms`, as the issuer stated.
    Granted {
        expires_ms: u64,
    },
    Refused {
        fault: GrantFault,
    },
}

/// One exchange of a grant, as audit data. It holds what the exchange asked for and how it was
/// answered, and never a token, an identity token or an issuer's token id: the codec has no key
/// for one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub stage: String,
    /// The action's name, or `validation:<name>` for a stage-level validation, which belongs to
    /// no action. A name cannot contain `:`, so the two never collide.
    pub holder: String,
    pub phase: GrantPhase,
    /// The attempt of the stage, from 1.
    pub attempt: u32,
    /// The exchange, counted within the phase of the attempt.
    pub n: u32,
    pub channel: String,
    /// The artifact type.
    pub repository: String,
    pub subject: String,
    pub audience: String,
    pub scope: String,
    /// A digest of the sorted coordinates the grant covers.
    pub coordinates: Hash256,
    /// Absent while the exchange is an intent.
    pub answer: Option<GrantAnswer>,
}

/// A sealed set, kept across runs so a later run can adopt it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seal {
    pub stage: String,
    pub platform: String,
    pub revision: String,
    pub set: Hash256,
    pub generation: String,
    pub inputs: Hash256,
    pub environment: String,
    pub machine: String,
    pub version: String,
    pub build: u64,
    pub commit: String,
    pub timestamp: u64,
    pub run: String,
    pub attempt: u32,
}

/// Every kind of release record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Run(Run),
    Stage(Stage),
    Action(Action),
    Publication(Publication),
    Validation(Validation),
    Grant(Grant),
    Seal(Seal),
}

/// Why a record was not written or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    UnknownKind(String),
    /// The payload holds a key that is not on the list of its kind.
    UnknownKey {
        kind: &'static str,
        key: String,
    },
    Missing {
        kind: &'static str,
        key: &'static str,
    },
    /// A value or a name is not in the form the writer writes.
    Bad {
        kind: &'static str,
        key: String,
        value: String,
    },
    /// A part of a name, or a generation, is not a part of a store key: it is empty, is `.` or
    /// `..`, or holds a `/`, a `\` or a control character.
    Part {
        kind: &'static str,
        part: &'static str,
        value: String,
    },
    /// The record has a locator, and no kind has one.
    Locator {
        kind: &'static str,
    },
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::UnknownKind(kind) => write!(f, "{kind:?} is not a kind of release record"),
            CodecError::UnknownKey { kind, key } => write!(f, "a {kind} record has no key {key:?}"),
            CodecError::Missing { kind, key } => write!(f, "a {kind} record lacks its key {key:?}"),
            CodecError::Bad { kind, key, value } => {
                write!(
                    f,
                    "a {kind} record has {value:?} for {key:?}, which is not in the form it is written in"
                )
            }
            CodecError::Part { kind, part, value } => write!(
                f,
                "the {part} of a {kind} record, {value:?}, is empty, is a dot part, or holds a `/`, a `\\` or a control character"
            ),
            CodecError::Locator { kind } => {
                write!(f, "a {kind} record has a locator, and no kind has one")
            }
        }
    }
}

/// A closed set of words a payload value is one of.
trait Word: Copy + 'static {
    const ALL: &'static [Self];
    fn word(self) -> &'static str;
}

impl Word for Phase {
    const ALL: &'static [Self] = &[Phase::Staging, Phase::Committing];
    fn word(self) -> &'static str {
        match self {
            Phase::Staging => "staging",
            Phase::Committing => "committing",
        }
    }
}

impl Word for GrantPhase {
    const ALL: &'static [Self] = &[GrantPhase::Stage, GrantPhase::Commit, GrantPhase::Read];
    fn word(self) -> &'static str {
        match self {
            GrantPhase::Stage => "stage",
            GrantPhase::Commit => "commit",
            GrantPhase::Read => "read",
        }
    }
}

impl Word for GrantFault {
    const ALL: &'static [Self] = &[
        GrantFault::Denied,
        GrantFault::Expired,
        GrantFault::Used,
        GrantFault::Unreachable,
    ];
    fn word(self) -> &'static str {
        match self {
            GrantFault::Denied => "denied",
            GrantFault::Expired => "expired",
            GrantFault::Used => "used",
            GrantFault::Unreachable => "unreachable",
        }
    }
}

/// The two words of the `outcome` key, which the public type folds into [`GrantAnswer`].
#[derive(Clone, Copy)]
enum Outcome {
    Granted,
    Refused,
}

impl Word for Outcome {
    const ALL: &'static [Self] = &[Outcome::Granted, Outcome::Refused];
    fn word(self) -> &'static str {
        match self {
            Outcome::Granted => "granted",
            Outcome::Refused => "refused",
        }
    }
}

#[derive(Default)]
struct Out(BTreeMap<String, String>);

impl Out {
    fn put(mut self, key: &str, value: impl ToString + Copy) -> Self {
        self.0.insert(key.to_owned(), value.to_string());
        self
    }

    fn put_some(self, key: &str, value: Option<impl ToString + Copy>) -> Self {
        match value {
            Some(v) => self.put(key, v),
            None => self,
        }
    }

    fn word(self, key: &str, value: impl Word) -> Self {
        self.put(key, value.word())
    }

    fn flag(self, key: &str, on: bool) -> Self {
        if on { self.put(key, "1") } else { self }
    }
}

/// Holds one part to the rule of the parts of a store key.
fn plain(kind: Kind, part: &'static str, value: &str) -> Result<(), CodecError> {
    if is_plain(value) {
        Ok(())
    } else {
        Err(CodecError::Part {
            kind: kind.tag(),
            part,
            value: value.to_owned(),
        })
    }
}

/// An attempt counts from 1, so a 0 is not one the writer writes.
fn attempt_of(kind: Kind, n: u32) -> Result<u32, CodecError> {
    if n == 0 {
        Err(CodecError::Bad {
            kind: kind.tag(),
            key: "attempt".to_owned(),
            value: "0".to_owned(),
        })
    } else {
        Ok(n)
    }
}

fn encoded(kind: Kind, name: String, out: Out) -> Encoded {
    Encoded {
        kind: Tag::new(kind.tag()),
        name,
        physical: None,
        outputs: out.0,
    }
}

impl Record {
    /// The kind of the record.
    pub fn kind(&self) -> Kind {
        match self {
            Record::Run(_) => Kind::Run,
            Record::Stage(_) => Kind::Stage,
            Record::Action(_) => Kind::Action,
            Record::Publication(_) => Kind::Publication,
            Record::Validation(_) => Kind::Validation,
            Record::Grant(_) => Kind::Grant,
            Record::Seal(_) => Kind::Seal,
        }
    }

    /// The record as a ledger holds it. It is refused when a part of its name, or a generation,
    /// is not a part of a store key, or when an attempt is 0: the reader would refuse what was
    /// written.
    pub fn encode(&self) -> Result<Encoded, CodecError> {
        let kind = self.kind();
        Ok(match self {
            Record::Run(r) => {
                plain(kind, "run_id", &r.run_id)?;
                plain(kind, "revision", &r.revision)?;
                encoded(
                    kind,
                    r.run_id.clone(),
                    Out::default()
                        .put("machine", &r.machine)
                        .put("revision", &r.revision)
                        .put("version", &r.version)
                        .put("build", r.build)
                        .put("commit", &r.commit)
                        .put("timestamp", r.timestamp),
                )
            }
            Record::Stage(s) => {
                plain(kind, "stage", &s.stage)?;
                if let Some(platform) = &s.platform {
                    plain(kind, "platform", platform)?;
                }
                if let Some(generation) = &s.generation {
                    plain(kind, "generation", generation)?;
                }
                encoded(
                    kind,
                    s.stage.clone(),
                    Out::default()
                        .put("attempt", attempt_of(kind, s.attempt)?)
                        .put_some("platform", s.platform.as_ref())
                        .put_some("set", s.set.as_ref())
                        .put_some("generation", s.generation.as_ref())
                        .flag("confirmed", s.confirmed),
                )
            }
            Record::Action(a) => {
                plain(kind, "stage", &a.stage)?;
                plain(kind, "action", &a.action)?;
                if let Some(generation) = &a.generation {
                    plain(kind, "generation", generation)?;
                }
                encoded(
                    kind,
                    format!("{}/{}", a.stage, a.action),
                    Out::default()
                        .put("input", &a.input)
                        .put_some("set", a.set.as_ref())
                        .put_some("generation", a.generation.as_ref()),
                )
            }
            Record::Publication(p) => {
                plain(kind, "run", &p.run)?;
                plain(kind, "stage", &p.stage)?;
                encoded(
                    kind,
                    p.coordinate.clone(),
                    Out::default()
                        .put("digest", &p.digest)
                        .put("build", p.build)
                        .put("commit", &p.commit)
                        .flag("exceptional", p.exceptional)
                        .put("run", &p.run)
                        .put("stage", &p.stage)
                        .put("attempt", attempt_of(kind, p.attempt)?)
                        .put_some("phase", p.phase.map(Word::word)),
                )
            }
            Record::Validation(v) => {
                plain(kind, "stage", &v.stage)?;
                plain(kind, "validation", &v.validation)?;
                encoded(
                    kind,
                    format!("{}/{}", v.stage, v.validation),
                    Out::default().put_some("set", v.set.as_ref()),
                )
            }
            Record::Grant(g) => {
                plain(kind, "stage", &g.stage)?;
                plain(kind, "holder", &g.holder)?;
                let out = Out::default()
                    .word("phase", g.phase)
                    .put("channel", &g.channel)
                    .put("repository", &g.repository)
                    .put("subject", &g.subject)
                    .put("audience", &g.audience)
                    .put("scope", &g.scope)
                    .put("coordinates", &g.coordinates);
                let out = match g.answer {
                    None => out,
                    Some(GrantAnswer::Granted { expires_ms }) => out
                        .word("outcome", Outcome::Granted)
                        .put("expires_ms", expires_ms),
                    Some(GrantAnswer::Refused { fault }) => {
                        out.word("outcome", Outcome::Refused).word("fault", fault)
                    }
                };
                encoded(
                    kind,
                    format!(
                        "{}/{}/{}/a{}-{}",
                        g.stage,
                        g.holder,
                        g.phase.word(),
                        attempt_of(kind, g.attempt)?,
                        g.n
                    ),
                    out,
                )
            }
            Record::Seal(s) => {
                plain(kind, "stage", &s.stage)?;
                plain(kind, "platform", &s.platform)?;
                plain(kind, "revision", &s.revision)?;
                plain(kind, "generation", &s.generation)?;
                plain(kind, "run", &s.run)?;
                encoded(
                    kind,
                    format!("{}/{}/{}", s.stage, s.platform, s.revision),
                    Out::default()
                        .put("set", &s.set)
                        .put("generation", &s.generation)
                        .put("inputs", &s.inputs)
                        .put("environment", &s.environment)
                        .put("machine", &s.machine)
                        .put("version", &s.version)
                        .put("build", s.build)
                        .put("commit", &s.commit)
                        .put("timestamp", s.timestamp)
                        .put("run", &s.run)
                        .put("attempt", attempt_of(kind, s.attempt)?),
                )
            }
        })
    }

    /// Reads a record back. The payload keys must all be on the list of the kind, and a name
    /// made of parts must hold exactly the parts of its kind, each a part of a store key.
    pub fn decode(e: &Encoded) -> Result<Self, CodecError> {
        let kind = Kind::from_tag(e.kind.as_str())
            .ok_or_else(|| CodecError::UnknownKind(e.kind.to_string()))?;
        let allowed = keys_of(kind);
        if let Some(key) = e.outputs.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(CodecError::UnknownKey {
                kind: kind.tag(),
                key: key.clone(),
            });
        }
        if e.physical.is_some() {
            return Err(CodecError::Locator { kind: kind.tag() });
        }
        let r = Reader {
            kind,
            map: &e.outputs,
        };
        Ok(match kind {
            Kind::Run => {
                plain(kind, "run_id", &e.name)?;
                Record::Run(Run {
                    run_id: e.name.clone(),
                    machine: r.hash("machine")?,
                    revision: r.part("revision", "revision")?,
                    version: r.text("version")?,
                    build: r.number("build")?,
                    commit: r.text("commit")?,
                    timestamp: r.number("timestamp")?,
                })
            }
            Kind::Stage => {
                plain(kind, "stage", &e.name)?;
                Record::Stage(Stage {
                    stage: e.name.clone(),
                    attempt: r.attempt()?,
                    platform: r.opt_part("platform")?,
                    set: r.opt_hash("set")?,
                    generation: r.opt_part("generation")?,
                    confirmed: r.flag("confirmed")?,
                })
            }
            Kind::Action => {
                let names = r.split(&e.name, ["stage", "action"])?;
                Record::Action(Action {
                    stage: names[0].clone(),
                    action: names[1].clone(),
                    input: r.hash("input")?,
                    set: r.opt_hash("set")?,
                    generation: r.opt_part("generation")?,
                })
            }
            Kind::Publication => Record::Publication(Publication {
                coordinate: e.name.clone(),
                digest: r.hash("digest")?,
                build: r.number("build")?,
                commit: r.text("commit")?,
                exceptional: r.flag("exceptional")?,
                run: r.part("run", "run")?,
                stage: r.part("stage", "stage")?,
                attempt: r.attempt()?,
                phase: r.opt_word("phase")?,
            }),
            Kind::Validation => {
                let names = r.split(&e.name, ["stage", "validation"])?;
                Record::Validation(Validation {
                    stage: names[0].clone(),
                    validation: names[1].clone(),
                    set: r.opt_hash("set")?,
                })
            }
            Kind::Grant => {
                let names = r.split(&e.name, ["stage", "holder", "phase", "exchange"])?;
                let phase: GrantPhase = r.word("phase")?;
                if names[2] != phase.word() {
                    return Err(r.bad("name", &e.name));
                }
                let (attempt, n) = r.exchange(&names[3], &e.name)?;
                let answer = match r.opt_word::<Outcome>("outcome")? {
                    None => {
                        r.absent("fault")?;
                        r.absent("expires_ms")?;
                        None
                    }
                    Some(Outcome::Granted) => {
                        r.absent("fault")?;
                        Some(GrantAnswer::Granted {
                            expires_ms: r.number("expires_ms")?,
                        })
                    }
                    Some(Outcome::Refused) => {
                        r.absent("expires_ms")?;
                        Some(GrantAnswer::Refused {
                            fault: r.word("fault")?,
                        })
                    }
                };
                Record::Grant(Grant {
                    stage: names[0].clone(),
                    holder: names[1].clone(),
                    phase,
                    attempt,
                    n,
                    channel: r.text("channel")?,
                    repository: r.text("repository")?,
                    subject: r.text("subject")?,
                    audience: r.text("audience")?,
                    scope: r.text("scope")?,
                    coordinates: r.hash("coordinates")?,
                    answer,
                })
            }
            Kind::Seal => {
                let names = r.split(&e.name, ["stage", "platform", "revision"])?;
                Record::Seal(Seal {
                    stage: names[0].clone(),
                    platform: names[1].clone(),
                    revision: names[2].clone(),
                    set: r.hash("set")?,
                    generation: r.part("generation", "generation")?,
                    inputs: r.hash("inputs")?,
                    environment: r.text("environment")?,
                    machine: r.text("machine")?,
                    version: r.text("version")?,
                    build: r.number("build")?,
                    commit: r.text("commit")?,
                    timestamp: r.number("timestamp")?,
                    run: r.part("run", "run")?,
                    attempt: r.attempt()?,
                })
            }
        })
    }
}

struct Reader<'a> {
    kind: Kind,
    map: &'a BTreeMap<String, String>,
}

impl Reader<'_> {
    fn bad(&self, key: &str, value: &str) -> CodecError {
        CodecError::Bad {
            kind: self.kind.tag(),
            key: key.to_owned(),
            value: value.to_owned(),
        }
    }

    /// Splits a name into the `N` parts of its kind, each held to the rule of a store key part.
    fn split<const N: usize>(
        &self,
        name: &str,
        parts: [&'static str; N],
    ) -> Result<[String; N], CodecError> {
        let found: Vec<&str> = name.split('/').collect();
        if found.len() != N {
            return Err(self.bad("name", name));
        }
        for (part, value) in parts.iter().zip(&found) {
            plain(self.kind, part, value)?;
        }
        Ok(std::array::from_fn(|i| found[i].to_owned()))
    }

    fn text(&self, key: &'static str) -> Result<String, CodecError> {
        self.map.get(key).cloned().ok_or(CodecError::Missing {
            kind: self.kind.tag(),
            key,
        })
    }

    /// A required text that is a part of a store key.
    fn part(&self, key: &'static str, part: &'static str) -> Result<String, CodecError> {
        let text = self.text(key)?;
        plain(self.kind, part, &text)?;
        Ok(text)
    }

    /// An optional text that is a part of a store key.
    fn opt_part(&self, key: &'static str) -> Result<Option<String>, CodecError> {
        self.map.get(key).map_or(Ok(None), |text| {
            plain(self.kind, key, text)?;
            Ok(Some(text.clone()))
        })
    }

    /// A number in the form the writer writes: decimal, with no sign and no leading zero.
    fn number(&self, key: &'static str) -> Result<u64, CodecError> {
        let text = self.text(key)?;
        self.canonical(key, &text)
    }

    fn canonical(&self, key: &str, text: &str) -> Result<u64, CodecError> {
        text.parse::<u64>()
            .ok()
            .filter(|n| n.to_string() == text)
            .ok_or_else(|| self.bad(key, text))
    }

    /// An attempt number of the payload.
    fn attempt(&self) -> Result<u32, CodecError> {
        let n = self.number("attempt")?;
        let n = u32::try_from(n).map_err(|_| self.bad("attempt", &n.to_string()))?;
        attempt_of(self.kind, n)
    }

    /// The `a<attempt>-<n>` part of a grant's name, whose `name` is given for the refusal.
    fn exchange(&self, part: &str, name: &str) -> Result<(u32, u32), CodecError> {
        let counts = part
            .strip_prefix('a')
            .and_then(|rest| rest.split_once('-'))
            .and_then(|(attempt, n)| {
                let attempt = self.canonical("name", attempt).ok()?;
                let n = self.canonical("name", n).ok()?;
                Some((u32::try_from(attempt).ok()?, u32::try_from(n).ok()?))
            });
        let (attempt, n) = counts.ok_or_else(|| self.bad("name", name))?;
        Ok((attempt_of(self.kind, attempt)?, n))
    }

    fn hash(&self, key: &'static str) -> Result<Hash256, CodecError> {
        let text = self.text(key)?;
        Hash256::from_hex(&text).ok_or_else(|| self.bad(key, &text))
    }

    fn opt_hash(&self, key: &'static str) -> Result<Option<Hash256>, CodecError> {
        self.map.get(key).map_or(Ok(None), |text| {
            Hash256::from_hex(text)
                .map(Some)
                .ok_or_else(|| self.bad(key, text))
        })
    }

    /// `1` or absent.
    fn flag(&self, key: &'static str) -> Result<bool, CodecError> {
        match self.map.get(key).map(String::as_str) {
            None => Ok(false),
            Some("1") => Ok(true),
            Some(other) => Err(self.bad(key, other)),
        }
    }

    fn pick<T: Word>(&self, key: &str, text: &str) -> Result<T, CodecError> {
        T::ALL
            .iter()
            .copied()
            .find(|w| w.word() == text)
            .ok_or_else(|| self.bad(key, text))
    }

    /// A required word of a closed set.
    fn word<T: Word>(&self, key: &'static str) -> Result<T, CodecError> {
        self.pick(key, &self.text(key)?)
    }

    /// A word of a closed set, or absent.
    fn opt_word<T: Word>(&self, key: &'static str) -> Result<Option<T>, CodecError> {
        self.map
            .get(key)
            .map_or(Ok(None), |text| self.pick(key, text).map(Some))
    }

    /// A key the record must not carry in this form.
    fn absent(&self, key: &'static str) -> Result<(), CodecError> {
        match self.map.get(key) {
            None => Ok(()),
            Some(value) => Err(self.bad(key, value)),
        }
    }
}
