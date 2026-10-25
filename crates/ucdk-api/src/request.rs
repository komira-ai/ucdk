// SPDX-License-Identifier: Apache-2.0
//! Request messages to the engine's values.

use ucdk_engine::{Mode as EngineMode, Request};
use ucdk_plan::{Finding, Lowered, Plan, PlanHash, lower};
use ucdk_proto::v1::ResourceList;

use crate::v1::{
    ApplyRequest, DestroyRequest, Error, ErrorCode, ExitClass, LeaksRequest, Mode, Outcome,
    PlanRequest, StatusRequest, outcome,
};

/// Why a request message cannot become an engine request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
    /// The resource list is refused: every finding is listed.
    Findings(Vec<Finding>),
    /// The message is well-formed protobuf but not a request: the text says what is wrong.
    Invalid(String),
}

impl RequestError {
    /// The response a host returns for it: findings are a refusal (nothing was changed), and
    /// anything else is a request that was not understood.
    pub fn into_outcome(self) -> Outcome {
        match self {
            Self::Findings(findings) => crate::response::findings_outcome(&findings),
            Self::Invalid(message) => Outcome {
                result: Some(outcome::Result::Invalid(Error {
                    code: ErrorCode::InvalidRequest as i32,
                    message,
                })),
                exit: ExitClass::Usage as i32,
            },
        }
    }
}

fn desired(resources: Option<&ResourceList>) -> Result<Lowered, RequestError> {
    lower(&resources.cloned().unwrap_or_default()).map_err(RequestError::Findings)
}

fn hash(text: &str) -> Result<PlanHash, RequestError> {
    PlanHash::from_hex(text).ok_or_else(|| {
        RequestError::Invalid("plan_hash is not 64 lower-case hexadecimal digits".to_owned())
    })
}

impl PlanRequest {
    /// The engine request that plans the list for the mode.
    pub fn to_request(&self) -> Result<Request, RequestError> {
        let mode = match Mode::try_from(self.mode) {
            Ok(Mode::Apply) => EngineMode::Apply,
            Ok(Mode::Destroy) => EngineMode::Destroy,
            Ok(Mode::Unspecified) | Err(_) => {
                return Err(RequestError::Invalid(
                    "mode is neither apply nor destroy".to_owned(),
                ));
            }
        };
        Ok(Request::Plan {
            desired: desired(self.resources.as_ref())?,
            mode,
        })
    }
}

impl StatusRequest {
    /// The engine request that reads the world; it changes nothing.
    pub fn to_request(&self) -> Result<Request, RequestError> {
        Ok(Request::Status {
            desired: desired(self.resources.as_ref())?,
        })
    }
}

impl LeaksRequest {
    /// The engine request that sweeps the cell for leaks; it changes nothing.
    pub fn to_request(&self) -> Request {
        Request::Leaks
    }
}

/// A run the approver pinned a hash for. The engine needs the plan too, and only the planner
/// makes one, so the host plans first and hands the plan to [`Approved::apply`] or
/// [`Approved::destroy`].
#[derive(Debug, Clone)]
pub struct Approved {
    desired: Lowered,
    plan_hash: PlanHash,
    rollback: bool,
}

impl Approved {
    /// The pinned hash.
    pub fn plan_hash(&self) -> &PlanHash {
        &self.plan_hash
    }

    /// The engine request that applies `plan`.
    pub fn apply(self, plan: Plan) -> Request {
        Request::Apply {
            desired: self.desired,
            plan,
            plan_hash: self.plan_hash,
            rollback: self.rollback,
        }
    }

    /// The engine request that destroys by `plan`.
    pub fn destroy(self, plan: Plan) -> Request {
        Request::Destroy {
            desired: self.desired,
            plan,
            plan_hash: self.plan_hash,
        }
    }
}

impl ApplyRequest {
    /// The list and the pinned hash of an apply.
    pub fn approved(&self) -> Result<Approved, RequestError> {
        Ok(Approved {
            desired: desired(self.resources.as_ref())?,
            plan_hash: hash(&self.plan_hash)?,
            rollback: self.rollback,
        })
    }
}

impl DestroyRequest {
    /// The list and the pinned hash of a destroy.
    pub fn approved(&self) -> Result<Approved, RequestError> {
        Ok(Approved {
            desired: desired(self.resources.as_ref())?,
            plan_hash: hash(&self.plan_hash)?,
            rollback: false,
        })
    }
}
