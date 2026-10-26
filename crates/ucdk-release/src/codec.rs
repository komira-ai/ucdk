// SPDX-License-Identifier: Apache-2.0
//! The record codec: how each release record is written as the fields a ledger record has (a
//! tag, a readable name, an optional locator and a string map), and read back.
//!
//! The payload is a string map whose keys come from a closed list per kind, and the list has no
//! key for a credential, so the ledger never holds a secret. A reader refuses a key that is not
//! on the list, a missing required key and a value that is not in the form the writer writes.
//! A record's path is `<tag>/k<key>`, the key hashed from the readable name, which is kept in
//! the record because versions hold characters a path segment does not allow.

use std::collections::BTreeMap;
use std::fmt;

use ucdk_core::{Hash256, Locator, ModelError, RecordKey, RecordPath, Tag};

/// The tag and payload keys of each kind of record.
pub const KINDS: [(&str, &[&str]); 7] = [
    (
        "run",
        &[
            "machine",
            "revision",
            "version",
            "build",
            "commit",
            "timestamp",
        ],
    ),
    (
        "stage",
        &["attempt", "platform", "set", "generation", "confirmed"],
    ),
    ("step", &["input", "set", "generation"]),
    (
        "publication",
        &[
            "digest",
            "build",
            "commit",
            "exceptional",
            "run",
            "stage",
            "attempt",
        ],
    ),
    ("validation", &["k", "set"]),
    ("created", &["vr"]),
    (
        "seal",
        &[
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
    ),
];

/// A record as a ledger holds it: the fields the codec fills.
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

/// The run's pinned inputs, written once by the first BUILD stage.
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
    /// The latest attempt of this stage.
    pub attempt: u32,
    pub platform: String,
    /// The sealed set the stage built or consumes, and the store generation that holds it.
    pub set: Option<Hash256>,
    pub generation: Option<String>,
    /// Every step is confirmed.
    pub confirmed: bool,
}

/// Where one step of a stage stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub stage: String,
    pub step: String,
    /// The input digest: step spec, set hash, platform and revision.
    pub input: Hash256,
    pub set: Option<Hash256>,
    /// For a BUILD step, the generation its output was written into.
    pub generation: Option<String>,
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
}

/// One validation of a stage and the validation run id stamped on what it creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    pub stage: String,
    pub validation: String,
    pub validation_run_id: String,
    /// The start counter.
    pub k: u32,
    /// The set the validation read.
    pub set: Option<Hash256>,
}

/// An object a validation announced it would create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub kind: String,
    pub id: String,
    pub validation_run_id: String,
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
    Step(Step),
    Publication(Publication),
    Validation(Validation),
    Created(Created),
    Seal(Seal),
}

/// Why a record was not read.
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
    /// A value is not in the form the writer writes.
    Bad {
        kind: &'static str,
        key: String,
        value: String,
    },
    /// The locator is present where the kind has none, or absent where it has one.
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
            CodecError::Locator { kind } => write!(f, "a {kind} record has the wrong locator"),
        }
    }
}

/// The payload keys of a kind.
pub fn keys_of(kind: &str) -> Option<&'static [&'static str]> {
    KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, keys)| *keys)
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

    fn flag(self, key: &str, on: bool) -> Self {
        if on { self.put(key, "1") } else { self }
    }
}

fn encoded(kind: &str, name: String, physical: Option<&str>, out: Out) -> Encoded {
    Encoded {
        kind: Tag::new(kind),
        name,
        physical: physical.map(Locator::new),
        outputs: out.0,
    }
}

impl Record {
    /// The record as a ledger holds it.
    pub fn encode(&self) -> Encoded {
        match self {
            Record::Run(r) => encoded(
                "run",
                r.run_id.clone(),
                None,
                Out::default()
                    .put("machine", &r.machine)
                    .put("revision", &r.revision)
                    .put("version", &r.version)
                    .put("build", r.build)
                    .put("commit", &r.commit)
                    .put("timestamp", r.timestamp),
            ),
            Record::Stage(s) => encoded(
                "stage",
                s.stage.clone(),
                None,
                Out::default()
                    .put("attempt", s.attempt)
                    .put("platform", &s.platform)
                    .put_some("set", s.set.as_ref())
                    .put_some("generation", s.generation.as_ref())
                    .flag("confirmed", s.confirmed),
            ),
            Record::Step(s) => encoded(
                "step",
                format!("{}/{}", s.stage, s.step),
                None,
                Out::default()
                    .put("input", &s.input)
                    .put_some("set", s.set.as_ref())
                    .put_some("generation", s.generation.as_ref()),
            ),
            Record::Publication(p) => encoded(
                "publication",
                p.coordinate.clone(),
                None,
                Out::default()
                    .put("digest", &p.digest)
                    .put("build", p.build)
                    .put("commit", &p.commit)
                    .flag("exceptional", p.exceptional)
                    .put("run", &p.run)
                    .put("stage", &p.stage)
                    .put("attempt", p.attempt),
            ),
            Record::Validation(v) => encoded(
                "validation",
                format!("{}/{}", v.stage, v.validation),
                Some(&v.validation_run_id),
                Out::default().put("k", v.k).put_some("set", v.set.as_ref()),
            ),
            Record::Created(c) => encoded(
                "created",
                format!("{}/{}", c.kind, c.id),
                Some(&c.id),
                Out::default().put("vr", &c.validation_run_id),
            ),
            Record::Seal(s) => encoded(
                "seal",
                format!("{}/{}/{}", s.stage, s.platform, s.revision),
                None,
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
                    .put("attempt", s.attempt),
            ),
        }
    }

    /// Reads a record back. The payload keys must all be on the list of the kind.
    pub fn decode(e: &Encoded) -> Result<Self, CodecError> {
        let kind = KINDS
            .iter()
            .map(|(k, _)| *k)
            .find(|k| *k == e.kind.as_str())
            .ok_or_else(|| CodecError::UnknownKind(e.kind.to_string()))?;
        let allowed = keys_of(kind).unwrap_or_default();
        if let Some(key) = e.outputs.keys().find(|k| !allowed.contains(&k.as_str())) {
            return Err(CodecError::UnknownKey {
                kind,
                key: key.clone(),
            });
        }
        if e.physical.is_some() != matches!(kind, "validation" | "created") {
            return Err(CodecError::Locator { kind });
        }
        let r = Reader {
            kind,
            map: &e.outputs,
        };
        let split = |name: &str, parts: usize| -> Result<Vec<String>, CodecError> {
            let found: Vec<String> = name.splitn(parts, '/').map(str::to_owned).collect();
            if found.len() == parts && found.iter().all(|p| !p.is_empty()) {
                Ok(found)
            } else {
                Err(CodecError::Bad {
                    kind,
                    key: "name".to_owned(),
                    value: name.to_owned(),
                })
            }
        };
        let physical = e
            .physical
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default();
        Ok(match kind {
            "run" => Record::Run(Run {
                run_id: e.name.clone(),
                machine: r.hash("machine")?,
                revision: r.text("revision")?,
                version: r.text("version")?,
                build: r.number("build")?,
                commit: r.text("commit")?,
                timestamp: r.number("timestamp")?,
            }),
            "stage" => Record::Stage(Stage {
                stage: e.name.clone(),
                attempt: r.attempt()?,
                platform: r.text("platform")?,
                set: r.opt_hash("set")?,
                generation: r.map.get("generation").cloned(),
                confirmed: r.flag("confirmed")?,
            }),
            "step" => {
                let names = split(&e.name, 2)?;
                Record::Step(Step {
                    stage: names[0].clone(),
                    step: names[1].clone(),
                    input: r.hash("input")?,
                    set: r.opt_hash("set")?,
                    generation: r.map.get("generation").cloned(),
                })
            }
            "publication" => Record::Publication(Publication {
                coordinate: e.name.clone(),
                digest: r.hash("digest")?,
                build: r.number("build")?,
                commit: r.text("commit")?,
                exceptional: r.flag("exceptional")?,
                run: r.text("run")?,
                stage: r.text("stage")?,
                attempt: r.attempt()?,
            }),
            "validation" => {
                let names = split(&e.name, 2)?;
                Record::Validation(Validation {
                    stage: names[0].clone(),
                    validation: names[1].clone(),
                    validation_run_id: physical,
                    k: r.small("k")?,
                    set: r.opt_hash("set")?,
                })
            }
            "created" => {
                let names = split(&e.name, 2)?;
                if names[1] != physical {
                    return Err(r.bad("name", &e.name));
                }
                Record::Created(Created {
                    kind: names[0].clone(),
                    id: physical,
                    validation_run_id: r.text("vr")?,
                })
            }
            _ => {
                let names = split(&e.name, 3)?;
                Record::Seal(Seal {
                    stage: names[0].clone(),
                    platform: names[1].clone(),
                    revision: names[2].clone(),
                    set: r.hash("set")?,
                    generation: r.text("generation")?,
                    inputs: r.hash("inputs")?,
                    environment: r.text("environment")?,
                    machine: r.text("machine")?,
                    version: r.text("version")?,
                    build: r.number("build")?,
                    commit: r.text("commit")?,
                    timestamp: r.number("timestamp")?,
                    run: r.text("run")?,
                    attempt: r.attempt()?,
                })
            }
        })
    }
}

struct Reader<'a> {
    kind: &'static str,
    map: &'a BTreeMap<String, String>,
}

impl Reader<'_> {
    fn bad(&self, key: &str, value: &str) -> CodecError {
        CodecError::Bad {
            kind: self.kind,
            key: key.to_owned(),
            value: value.to_owned(),
        }
    }

    fn text(&self, key: &'static str) -> Result<String, CodecError> {
        self.map.get(key).cloned().ok_or(CodecError::Missing {
            kind: self.kind,
            key,
        })
    }

    /// A number in the form the writer writes: decimal, with no sign and no leading zero.
    fn number(&self, key: &'static str) -> Result<u64, CodecError> {
        let text = self.text(key)?;
        text.parse::<u64>()
            .ok()
            .filter(|n| n.to_string() == text)
            .ok_or_else(|| self.bad(key, &text))
    }

    fn small(&self, key: &'static str) -> Result<u32, CodecError> {
        let n = self.number(key)?;
        u32::try_from(n).map_err(|_| self.bad(key, &n.to_string()))
    }

    /// An attempt number: attempts count from 1, so a 0 is not one the writer writes.
    fn attempt(&self) -> Result<u32, CodecError> {
        match self.small("attempt")? {
            0 => Err(self.bad("attempt", "0")),
            n => Ok(n),
        }
    }

    fn hash(&self, key: &'static str) -> Result<Hash256, CodecError> {
        let text = self.text(key)?;
        Hash256::from_hex(&text).ok_or_else(|| self.bad(key, &text))
    }

    fn opt_hash(&self, key: &'static str) -> Result<Option<Hash256>, CodecError> {
        match self.map.get(key) {
            None => Ok(None),
            Some(text) => Hash256::from_hex(text)
                .map(Some)
                .ok_or_else(|| self.bad(key, text)),
        }
    }

    /// `1` or absent.
    fn flag(&self, key: &'static str) -> Result<bool, CodecError> {
        match self.map.get(key).map(String::as_str) {
            None => Ok(false),
            Some("1") => Ok(true),
            Some(other) => Err(self.bad(key, other)),
        }
    }
}
