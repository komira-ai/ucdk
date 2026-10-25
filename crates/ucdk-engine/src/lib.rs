// SPDX-License-Identifier: Apache-2.0
//! The reconciler engine: it carries a [`ucdk_plan::Plan`] out against a [`ucdk_provider::Provider`] and a
//! [`ucdk_ledger::Ledger`], one trait object each.
//!
//! The whole surface is one pair of plain enums, [`Request`] and [`Outcome`], handled by
//! [`Engine::handle`]. They are not a wire format: a later shared library wraps exactly this
//! seam and nothing else.
//!
//! The engine is synchronous and stops at the first failure.
//!
//! # What it guarantees
//!
//! - **Write-ahead.** Before every create, update and delete the ledger records the intent,
//!   so a crash at any point leaves a record that the next run resolves by looking.
//! - **Re-observe.** The plan's observation is never trusted for a mutation: each node is read
//!   again right before it is changed.
//! - **Stamp in the create call.** A create carries the node's stamp; an object that turns
//!   up in place of a create is taken as ours only if it carries that stamp.
//! - **Ours means stamp and record.** A plan refuses the whole run on any object that has one
//!   without the other (a foreign object, a stamp with no record, a record with no stamp), and
//!   the engine checks both again before a delete.
//! - **Closed-world delete.** A node the file no longer wants is deleted, unless its kind holds
//!   data (kept), the provider says it cannot be deleted, or the file no longer names its owner.
//!   The record of an object is dropped only after a read that shows the object gone; a read
//!   that fails leaves the record where it was.
//! - **Status.** A failed object is updated toward its node even when its fields match. One
//!   that is still converging gets no write, and its record is not confirmed until a read
//!   shows it ready; the run still succeeds and the report says which objects are waiting.
//!   An object that still reads failed after the run's create or repair is reported as failed.
//! - **Read-only status.** `Request::Status` reads the ledger and observes each recorded and
//!   each wanted object through the provider, and makes no ledger write and no provider call
//!   that mutates; each entry says whether record and object agree.
//! - **One cell.** Every stamp carries the engine's cell, and an object stamped for another
//!   cell is not ours.
//! - **Outputs.** After a confirmed create or update the record holds the object's non-secret
//!   outputs; a secret output is never written to the ledger. A node that reads an output is
//!   applied with the producer's recorded value, and a run that changes the producer resolves
//!   the node (a step planned as known after apply) only once the producer is confirmed. A
//!   producer that this run reads as still coming up leaves the node waiting for a later run,
//!   whatever its record says; one that is still failed after the run's update leaves the node
//!   blocked, and the report names the producer.
//! - **Rollback, opt in.** After a failed apply, only the objects that run created are deleted,
//!   newest first; nothing it updated is touched.
//!
//! # Crash safety
//!
//! A create whose answer is lost (a timeout, a crash after the call) leaves an intent record
//! and, maybe, an object. The next run finds the object by its name, sees our stamp and takes
//! it as the intent's object; with no object it creates one. Either way there is one object.
//!
//! # Not done here
//!
//! Lease time and fencing (two runs on one ledger are made safe by the compare-and-swap writes
//! and the create refusal, not by exclusion), durable ledgers, adoption of objects the tool did
//! not create.

mod delete;
mod engine;
mod error;
mod seam;
mod status;
mod write;

pub use engine::Engine;
pub use error::EngineError;
pub use seam::{Failure, Mode, ObjectStatus, Outcome, Refusal, Report, Request, Standing};
pub use ucdk_ledger::State;
