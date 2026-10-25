// SPDX-License-Identifier: Apache-2.0
//! The engine's values to response messages.

use ucdk_engine::{EngineError, Failure, ObjectStatus, Outcome, Refusal, Report, Standing, State};
use ucdk_plan::{
    Action, Conflict, ConflictKind, Finding, FindingKind, Lowered, Plan, Retained, Step,
};

use ucdk_provider::Status;

use crate::exit::exit_class;
use crate::v1;

impl From<&Outcome> for v1::Outcome {
    fn from(outcome: &Outcome) -> Self {
        let result = match outcome {
            Outcome::Planned(plan) => v1::outcome::Result::Planned(plan.into()),
            Outcome::Done(report) => v1::outcome::Result::Done(report.into()),
            Outcome::Status(objects) => v1::outcome::Result::Status(v1::StatusResponse {
                objects: objects.iter().map(v1::ObjectStatus::from).collect(),
            }),
            Outcome::Refused(why) => v1::outcome::Result::Refused(why.into()),
            Outcome::Failed(failure) => v1::outcome::Result::Failed(failure.into()),
        };
        Self {
            result: Some(result),
            exit: exit_class(outcome) as i32,
        }
    }
}

/// The response to a resource list that is refused before it is planned.
pub(crate) fn findings_outcome(findings: &[Finding]) -> v1::Outcome {
    let refusal = v1::Refusal {
        reason: Some(v1::refusal::Reason::Findings(v1::Findings {
            findings: findings.iter().map(v1::Finding::from).collect(),
        })),
        message: findings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "),
    };
    v1::Outcome {
        result: Some(v1::outcome::Result::Refused(refusal)),
        exit: v1::ExitClass::Refused as i32,
    }
}

impl From<&Plan> for v1::Plan {
    fn from(plan: &Plan) -> Self {
        Self {
            steps: plan.steps().iter().map(v1::Step::from).collect(),
            hash: plan.hash().to_string(),
        }
    }
}

impl From<&Step> for v1::Step {
    fn from(step: &Step) -> Self {
        let (action, physical_id, retained) = match &step.action {
            Action::Create => (v1::StepAction::Create, String::new(), None),
            Action::Update => (v1::StepAction::Update, String::new(), None),
            Action::Noop => (v1::StepAction::Noop, String::new(), None),
            Action::Delete { physical } => {
                (v1::StepAction::Delete, physical.as_str().to_owned(), None)
            }
            Action::Reap => (v1::StepAction::Reap, String::new(), None),
            Action::Converging => (v1::StepAction::Converging, String::new(), None),
            Action::KnownAfterApply => (v1::StepAction::KnownAfterApply, String::new(), None),
            Action::Retain(why) => (v1::StepAction::Retain, String::new(), Some(why.into())),
        };
        Self {
            path: step.path.to_string(),
            kind: step.kind.as_str().to_owned(),
            action: action as i32,
            physical_id,
            retained,
        }
    }
}

impl From<&Retained> for v1::Retained {
    fn from(why: &Retained) -> Self {
        let (reason, detail) = match why {
            Retained::Keep => (v1::RetainReason::Keep, String::new()),
            Retained::Undeletable(reason) => (v1::RetainReason::Undeletable, reason.clone()),
            Retained::OwnerNotNamed => (v1::RetainReason::OwnerNotNamed, String::new()),
        };
        Self {
            reason: reason as i32,
            detail,
        }
    }
}

impl From<&Report> for v1::Report {
    fn from(report: &Report) -> Self {
        let paths = |list: &[ucdk_provider::NodePath]| -> Vec<String> {
            list.iter().map(ToString::to_string).collect()
        };
        Self {
            created: paths(&report.created),
            updated: paths(&report.updated),
            resumed: paths(&report.resumed),
            deleted: paths(&report.deleted),
            reaped: paths(&report.reaped),
            retained: report
                .retained
                .iter()
                .map(|(path, why)| v1::RetainedObject {
                    path: path.to_string(),
                    retained: Some(why.into()),
                })
                .collect(),
            converging: paths(&report.converging),
            failed: paths(&report.failed),
            blocked_by_failed: report
                .blocked_by_failed
                .iter()
                .map(|(path, producer)| v1::Blocked {
                    path: path.to_string(),
                    producer: producer.to_string(),
                })
                .collect(),
            rolled_back: paths(&report.rolled_back),
            rollback_failed: report
                .rollback_failed
                .iter()
                .map(|(path, error)| v1::RollbackFailure {
                    path: path.to_string(),
                    message: error.to_string(),
                })
                .collect(),
        }
    }
}

impl From<&Refusal> for v1::Refusal {
    fn from(why: &Refusal) -> Self {
        let reason = match why {
            Refusal::Conflicts(list) => v1::refusal::Reason::Conflicts(v1::Conflicts {
                conflicts: list.iter().map(v1::Conflict::from).collect(),
            }),
            Refusal::EmptyScope => v1::refusal::Reason::EmptyScope(v1::EmptyScope {}),
            Refusal::PlanHashMismatch { approved, plan } => {
                v1::refusal::Reason::PlanHashMismatch(v1::PlanHashMismatch {
                    approved: approved.to_string(),
                    plan: plan.to_string(),
                })
            }
            Refusal::PlanChanged { approved, now } => {
                v1::refusal::Reason::PlanChanged(v1::PlanChanged {
                    approved: approved.to_string(),
                    now: now.to_string(),
                })
            }
            Refusal::UnhostedKinds(list) => v1::refusal::Reason::UnhostedKinds(v1::UnhostedKinds {
                kinds: list
                    .iter()
                    .map(|(path, kind)| v1::UnhostedKind {
                        path: path.to_string(),
                        kind: kind.as_str().to_owned(),
                    })
                    .collect(),
            }),
            Refusal::UnsupportedStampMode(paths) => {
                v1::refusal::Reason::UnsupportedStampMode(v1::UnsupportedStampMode {
                    paths: paths.iter().map(ToString::to_string).collect(),
                })
            }
        };
        Self {
            reason: Some(reason),
            message: why.to_string(),
        }
    }
}

impl From<&Conflict> for v1::Conflict {
    fn from(conflict: &Conflict) -> Self {
        let (kind, marker) = match &conflict.why {
            ConflictKind::Foreign => (v1::ConflictKind::Foreign, String::new()),
            ConflictKind::StampWithoutRecord => {
                (v1::ConflictKind::StampWithoutRecord, String::new())
            }
            ConflictKind::PhysicalIdMismatch => {
                (v1::ConflictKind::PhysicalIdMismatch, String::new())
            }
            ConflictKind::StampStripped => (v1::ConflictKind::StampStripped, String::new()),
            ConflictKind::OtherOwner => (v1::ConflictKind::OtherOwner, String::new()),
            ConflictKind::OtherTool(marker) => (v1::ConflictKind::OtherTool, marker.clone()),
            ConflictKind::VanishedRetained => (v1::ConflictKind::VanishedRetained, String::new()),
            ConflictKind::RecordedCycle => (v1::ConflictKind::RecordedCycle, String::new()),
        };
        Self {
            path: conflict.path.to_string(),
            kind: kind as i32,
            marker,
            message: conflict.to_string(),
        }
    }
}

impl From<&Finding> for v1::Finding {
    fn from(finding: &Finding) -> Self {
        let kind = match &finding.kind {
            FindingKind::BadId => v1::FindingKind::BadId,
            FindingKind::DuplicateId => v1::FindingKind::DuplicateId,
            FindingKind::MissingBody => v1::FindingKind::MissingBody,
            FindingKind::MissingImage => v1::FindingKind::MissingImage,
            FindingKind::Unresolved { .. } => v1::FindingKind::Unresolved,
            FindingKind::UnsupportedPlatform { .. } => v1::FindingKind::UnsupportedPlatform,
            FindingKind::EmptyValue => v1::FindingKind::EmptyValue,
            FindingKind::EnvSecretOverlap => v1::FindingKind::EnvSecretOverlap,
            FindingKind::UnboundRef { .. } => v1::FindingKind::UnboundRef,
            FindingKind::NoTarget => v1::FindingKind::NoTarget,
            FindingKind::UnsupportedAccess => v1::FindingKind::UnsupportedAccess,
            FindingKind::AccessToOutput => v1::FindingKind::AccessToOutput,
            FindingKind::MissingOutput => v1::FindingKind::MissingOutput,
            FindingKind::NamedOutput => v1::FindingKind::NamedOutput,
            FindingKind::UnexposedOutput { .. } => v1::FindingKind::UnexposedOutput,
            FindingKind::NotPortable { .. } => v1::FindingKind::NotPortable,
            FindingKind::Cycle { .. } => v1::FindingKind::Cycle,
        };
        Self {
            resource: finding.resource.clone(),
            field: finding.field.clone(),
            kind: kind as i32,
            message: finding.to_string(),
        }
    }
}

impl From<&Failure> for v1::Failure {
    fn from(failure: &Failure) -> Self {
        Self {
            report: Some((&failure.report).into()),
            at: failure.at.as_ref().map(ToString::to_string),
            error: Some((&failure.error).into()),
        }
    }
}

impl From<&EngineError> for v1::Error {
    fn from(error: &EngineError) -> Self {
        let code = match error {
            EngineError::Provider(_) => v1::ErrorCode::Provider,
            EngineError::Ledger(_) => v1::ErrorCode::Ledger,
            EngineError::LedgerBusy => v1::ErrorCode::LedgerBusy,
            EngineError::NotOurs(_) => v1::ErrorCode::NotOurs,
            EngineError::StillPresent(_) => v1::ErrorCode::StillPresent,
            EngineError::Unwitnessed(_) => v1::ErrorCode::Unwitnessed,
            EngineError::OutputUnknown { .. } => v1::ErrorCode::OutputUnknown,
        };
        Self {
            code: code as i32,
            message: error.to_string(),
        }
    }
}

impl From<&ObjectStatus> for v1::ObjectStatus {
    fn from(object: &ObjectStatus) -> Self {
        let record = object.record.map(|state| match state {
            State::Intent => v1::RecordState::Intent,
            State::Confirmed => v1::RecordState::Confirmed,
            State::Reaping => v1::RecordState::Reaping,
        });
        let live = object.live.map(|status| match status {
            Status::Ready => v1::LiveStatus::Ready,
            Status::Converging => v1::LiveStatus::Converging,
            Status::Failed => v1::LiveStatus::Failed,
        });
        let standing = match object.standing {
            Standing::Agrees => v1::Standing::Agrees,
            Standing::Pending => v1::Standing::Pending,
            Standing::Missing => v1::Standing::Missing,
            Standing::NotApplied => v1::Standing::NotApplied,
            Standing::Unclaimed => v1::Standing::Unclaimed,
        };
        Self {
            path: object.path.to_string(),
            kind: object.kind.as_str().to_owned(),
            record: record.map(|r| r as i32),
            live: live.map(|l| l as i32),
            standing: standing as i32,
            outputs: object.outputs.clone(),
        }
    }
}

/// The response to loading a resource list: the lowered nodes in dependency order, or the
/// findings that refuse it.
pub fn catalog_response(lowered: Result<Lowered, Vec<Finding>>) -> v1::CatalogResponse {
    match lowered {
        Ok(lowered) => v1::CatalogResponse {
            nodes: lowered
                .nodes()
                .iter()
                .map(|n| v1::CatalogNode {
                    path: n.node.path().to_string(),
                    kind: n.node.kind().as_str().to_owned(),
                    wanted: n.wanted,
                    deps: n.deps.iter().map(ToString::to_string).collect(),
                })
                .collect(),
            findings: Vec::new(),
            exit: v1::ExitClass::Done as i32,
        },
        Err(findings) => v1::CatalogResponse {
            nodes: Vec::new(),
            findings: findings.iter().map(v1::Finding::from).collect(),
            exit: v1::ExitClass::Refused as i32,
        },
    }
}
