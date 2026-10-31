// SPDX-License-Identifier: Apache-2.0
//! A validation: a check that ucdk itself evaluates, from a set it reads or a channel it reads
//! back. It runs no command and creates nothing. Its rows are derived from what it reads before
//! anything is evaluated, and it passes only when at least one row was asserted and every row
//! passed.
//!
//! * `SET_CLOSURE` reads the sealed set, one row per member, and is not read again for a set it
//!   already passed.
//! * `CHANNEL_READBACK` reads each coordinate back from the channel the way a stranger would,
//!   and is never skipped: the registry is the truth and may have changed.
//!
//! The record of a validation is written as an intent before it is evaluated and confirmed after
//! it passed. A validation that failed leaves the intent, and is evaluated again by the next
//! request.

use sha2::{Digest, Sha256};
use ucdk_core::Hash256;
use ucdk_ledger::State;
use ucdk_release::closure::{closure_rows, set_closure};
use ucdk_release::codec;
use ucdk_release::machine::{publishes, smokes};
use ucdk_release::publish::Presence;
use ucdk_release::rows::ExpectedRow;
use ucdk_release::selection::Holder;
use ucdk_release_proto::v1::{
    Coordinate, Manifest, Reason, ReleaseSet, Row, RowState, Stage, Validation, ValidationKind,
    Verdict, Visibility,
};
use ucdk_release_seams::Grant;

use crate::engine::{Engine, accounted};
use crate::leases::Leases;
use crate::outcome::{Finished, Flow, Report, row, validation_report, what_failed};

/// What every row of a read-back asserts of its coordinate.
pub const READBACK_ASSERTION: &str = "the channel serves the set's digest";

/// A coordinate to read back: where, which, and the digest the set says it has.
struct Wanted {
    channel: String,
    coordinate: Coordinate,
    digest: String,
    public: bool,
    row: ExpectedRow,
}

/// What one read of a coordinate said.
enum Look {
    /// The row is decided.
    Decided(RowState, String),
    /// Not yet: poll again. `unreadable` says the read itself failed.
    Again { detail: String, unreadable: bool },
}

impl Engine<'_> {
    pub(crate) fn validation(
        &mut self,
        leases: &Leases<'_>,
        stage: &Stage,
        holder: &Holder,
        decl: &Validation,
    ) -> Flow<Finished> {
        let records = self.records()?;
        let (set, hash) = self.load_set(&records, stage)?;
        let closure = decl.kind() == ValidationKind::SetClosure;
        if closure
            && let Some((found, State::Confirmed)) = records.validation(&stage.name, &decl.name)
            && found.set.as_ref() == Some(&hash)
        {
            let noop = validation_report(&decl.name, Verdict::Noop, Vec::new());
            return Ok(Finished {
                verdict: Verdict::Noop,
                reason: Reason::Unspecified,
                message: String::new(),
                report: Some(Box::new(Report::Validation(noop))),
            });
        }
        let wanted = if closure {
            Vec::new()
        } else {
            self.coordinates(stage, holder, &set)?
        };
        let record = |set: Option<Hash256>| {
            codec::Record::Validation(codec::Validation {
                stage: stage.name.clone(),
                validation: decl.name.clone(),
                set,
            })
        };
        self.put(leases, &record(None), State::Intent)?;
        let (expected, rows) = if closure {
            (closure_rows(&set), set_closure(&set))
        } else {
            self.read_back(&wanted, decl.wait_polls)
        };
        let verdict = accounted(&expected, &rows)?;
        if verdict == Verdict::Succeeded {
            leases.settle()?;
            self.put(leases, &record(closure.then_some(hash)), State::Confirmed)?;
        }
        let (reason, message) = match verdict {
            Verdict::Succeeded => (Reason::Unspecified, String::new()),
            Verdict::Indeterminate => (Reason::Channel, what_failed(&rows)),
            _ => (Reason::Validation, what_failed(&rows)),
        };
        Ok(Finished {
            verdict,
            reason,
            message,
            report: Some(Box::new(Report::Validation(validation_report(
                &decl.name, verdict, rows,
            )))),
        })
    }

    /// The coordinates a read-back validation reads, in order: those of its Smoke action when it
    /// is attached to one, and otherwise those of every Publish action of the stage; and for
    /// each the digest the set says it has.
    fn coordinates(&self, stage: &Stage, holder: &Holder, set: &ReleaseSet) -> Flow<Vec<Wanted>> {
        let legs: Vec<(&str, &[String])> = match holder {
            Holder::Action(name) => smokes(stage)
                .filter(|(action, _)| action.name == *name)
                .map(|(_, smoke)| (smoke.channel.as_str(), smoke.artifacts.as_slice()))
                .collect(),
            Holder::Stage => publishes(stage)
                .map(|(_, publish)| (publish.channel.as_str(), publish.artifacts.as_slice()))
                .collect(),
        };
        let mut wanted = Vec::new();
        for (channel, artifacts) in legs {
            for name in artifacts {
                let Some(manifest) = member(set, name) else {
                    return Err(Finished::stopped(
                        Verdict::Refused,
                        Reason::SetMember,
                        format!("the set has no member {name}, which {channel} should serve"),
                    ));
                };
                let public = self.machine.channel(channel).is_some_and(|c| {
                    c.repositories.iter().any(|r| {
                        (r.r#type, r.visibility) == (manifest.r#type, Visibility::Public as i32)
                    })
                });
                let coordinate = Coordinate {
                    channel: channel.to_owned(),
                    r#type: manifest.r#type,
                    name: manifest.name.clone(),
                    version: manifest.version.clone(),
                    platform: manifest.platform.clone(),
                };
                let subject = format!(
                    "{channel}/{} {} {} ({})",
                    manifest.name,
                    manifest.version,
                    manifest.platform,
                    manifest.r#type().as_str_name()
                );
                wanted.push(Wanted {
                    channel: channel.to_owned(),
                    coordinate,
                    digest: manifest.sha256.clone(),
                    public,
                    row: ExpectedRow::new(subject, READBACK_ASSERTION),
                });
            }
        }
        Ok(wanted)
    }

    /// Reads every wanted coordinate back, and the rows it derived for them.
    fn read_back(&mut self, wanted: &[Wanted], wait_polls: u32) -> (Vec<ExpectedRow>, Vec<Row>) {
        let expected = wanted.iter().map(|w| w.row.clone()).collect();
        let rows = wanted
            .iter()
            .map(|w| {
                let (state, detail) = self.poll(w, wait_polls);
                row(&w.row.name(), state, detail)
            })
            .collect();
        (expected, rows)
    }

    /// Reads one coordinate: once, and again after one pause tick for each of `wait_polls` while
    /// the channel does not serve it yet.
    fn poll(&mut self, wanted: &Wanted, wait_polls: u32) -> (RowState, String) {
        let Some(publisher) = self.seams.channels.get_mut(&wanted.channel) else {
            let detail = format!("no publisher was given for the channel {}", wanted.channel);
            return (RowState::RowUnreadable, detail);
        };
        if !wanted.public {
            let detail = format!(
                "the channel {} is not a public repository, and a read of one needs a read grant, \
                 which this engine does not exchange",
                wanted.channel
            );
            return (RowState::RowUnreadable, detail);
        }
        let mut last = (RowState::RowFailed, String::new());
        for attempt in 0..=wait_polls {
            if attempt > 0 {
                self.seams.pause.pause(1);
            }
            match look(&mut **publisher, wanted) {
                Look::Decided(state, detail) => return (state, detail),
                Look::Again { detail, unreadable } => {
                    let state = if unreadable {
                        RowState::RowUnreadable
                    } else {
                        RowState::RowFailed
                    };
                    last = (state, detail);
                }
            }
        }
        last
    }
}

/// One read of a coordinate as a stranger: what the registry says it holds, and when it says it
/// serves the set's digest, the bytes themselves, hashed here. The digest the registry reports
/// is never what says a coordinate was published.
fn look(publisher: &mut dyn ucdk_release_seams::Publisher, wanted: &Wanted) -> Look {
    let stranger = Grant::none();
    match publisher.observe(&stranger, &wanted.coordinate, &[]) {
        Err(e) => Look::Again {
            detail: e.to_string(),
            unreadable: true,
        },
        Ok(Presence::Absent | Presence::Staged(_)) => Look::Again {
            detail: "the channel does not serve it".to_owned(),
            unreadable: false,
        },
        Ok(Presence::Burned) => {
            Look::Decided(RowState::RowFailed, "the coordinate is burned".to_owned())
        }
        Ok(Presence::Yanked(_)) => {
            Look::Decided(RowState::RowFailed, "the coordinate is yanked".to_owned())
        }
        Ok(Presence::Present(held)) if held.digest != wanted.digest => Look::Decided(
            RowState::RowFailed,
            format!(
                "the channel holds digest {}, and the set's is {}",
                held.digest, wanted.digest
            ),
        ),
        Ok(Presence::Present(_)) => match publisher.fetch(&stranger, &wanted.coordinate) {
            Err(e) => Look::Again {
                detail: e.to_string(),
                unreadable: true,
            },
            Ok(None) => Look::Again {
                detail: "the channel reports the coordinate and serves no bytes".to_owned(),
                unreadable: false,
            },
            Ok(Some(bytes)) => {
                let served = Hash256::from_sha256(&Sha256::digest(&bytes).into()).to_string();
                if served == wanted.digest {
                    Look::Decided(RowState::RowPassed, format!("serves {served}"))
                } else {
                    Look::Decided(
                        RowState::RowFailed,
                        format!(
                            "the bytes the channel serves hash to {served}, and the set's digest \
                             is {}",
                            wanted.digest
                        ),
                    )
                }
            }
        },
    }
}

/// The manifest of the member `name`.
fn member<'s>(set: &'s ReleaseSet, name: &str) -> Option<&'s Manifest> {
    set.members
        .iter()
        .filter_map(|m| m.manifest.as_ref())
        .find(|m| m.name == name)
}
