// SPDX-License-Identifier: Apache-2.0
//! The cross-SDK wire contract: one golden file per case, the bytes this crate's messages encode
//! to. Every SDK builds the same value, and its generated encoder must write the same bytes; it
//! must decode the bytes back to the same value (`sdk/python/tests/test_wire_golden.py` is the
//! Python half). The files are in `sdk/wire-golden`, with `manifest.txt` naming the message each
//! one holds (`<file> <message>`), so a golden cannot be read as the wrong message.
//!
//! The cases are chosen so that, together, every message and every field of `ucdk.resource.v1`
//! and `ucdk.api.v1` is written at least once (`ucdk-codegen/tests/golden_coverage.rs` holds
//! them to it), including the shapes whose bytes depend on the writer: a oneof written where its
//! lowest arm sits, an `optional` zero, and a map entry whose key or value is the default.
//!
//! To change a golden on purpose, edit the case and run
//! `cargo test -p ucdk-api --test sdk_golden -- --ignored write_goldens`; the diff of the files is
//! what review reads.

use std::path::PathBuf;

use prost::Message as _;
use prost_types::Duration;
use ucdk_api::v1::{
    ApplyRequest, Blocked, CatalogNode, CatalogResponse, Conflict, ConflictKind, Conflicts,
    CredentialSource, DestroyRequest, EmptyScope, Error, ErrorCode, ExitClass, Failure, Finding,
    FindingKind, Findings, Leak, LeaksRequest, LeaksResponse, LeaseHeld, LiveStatus, Mode,
    ObjectStatus, Outcome, Plan, PlanChanged, PlanHashMismatch, PlanRequest, RecordState, Refusal,
    Report, RetainReason, Retained, RetainedObject, RollbackFailure, SessionConfig, Standing,
    StatusRequest, StatusResponse, Step, StepAction, UnhostedKind, UnhostedKinds,
    UnsupportedStampMode, outcome, refusal,
};
use ucdk_proto::v1::{
    Access, Image, Job, Output, Ref, Resource, ResourceList, Scale, SecretRef, Service, Size,
    StepOutput, Uses, Value, image, job, r#ref, resource, service, value,
};

struct Case {
    file: &'static str,
    message: &'static str,
    bytes: Vec<u8>,
}

fn case(file: &'static str, message: &'static str, value: &impl prost::Message) -> Case {
    Case {
        file,
        message,
        bytes: value.encode_to_vec(),
    }
}

fn text(s: &str) -> String {
    s.to_owned()
}

fn literal(s: &str) -> Value {
    Value {
        v: Some(value::V::Literal(text(s))),
    }
}

fn digest_image() -> Image {
    Image {
        source: Some(image::Source::Digest(text("sha256:aa"))),
        platform: text("linux/amd64"),
    }
}

fn step_image() -> Image {
    Image {
        source: Some(image::Source::Output(StepOutput {
            step: text("build"),
            name: text("img"),
        })),
        platform: text("linux/amd64"),
    }
}

fn secret(name: &str) -> SecretRef {
    SecretRef {
        name: text(name),
        store: Some(text("vault")),
        version: Some(text("7")),
    }
}

fn full_service(exposure: service::Exposure, scale_min: Option<u32>) -> Service {
    Service {
        image: Some(step_image()),
        port: 8080,
        args: vec![text("--a"), text("--b")],
        env: [
            (text("K"), literal("v")),
            (
                text("P"),
                Value {
                    v: Some(value::V::Param(text("p"))),
                },
            ),
            (
                text("R"),
                Value {
                    v: Some(value::V::Ref(Ref {
                        resource: text("db"),
                        output: Some(r#ref::Output::Standard(Output::Url as i32)),
                    })),
                },
            ),
            (
                text("N"),
                Value {
                    v: Some(value::V::Ref(Ref {
                        resource: text("db"),
                        output: Some(r#ref::Output::Named(text("cert"))),
                    })),
                },
            ),
        ]
        .into_iter()
        .collect(),
        size: Some(Size {
            cpu_millis: 500,
            memory_mb: 256,
        }),
        scale: Some(Scale {
            min: scale_min,
            max: 10,
        }),
        health_path: text("/healthz"),
        request_timeout: Some(Duration {
            seconds: 30,
            nanos: 5,
        }),
        max_concurrency: 80,
        exposure: Some(exposure),
        secret_env: [(text("S"), secret("sec"))].into_iter().collect(),
    }
}

fn full_job(trigger: job::Trigger, max_retries: Option<u32>) -> Job {
    Job {
        image: Some(digest_image()),
        args: vec![text("go")],
        size: Some(Size {
            cpu_millis: 1000,
            memory_mb: 512,
        }),
        max_retries,
        timeout: Some(Duration {
            seconds: -600,
            nanos: -1,
        }),
        env: [(text("E"), literal("e"))].into_iter().collect(),
        secret_env: [(text("T"), secret("tok"))].into_iter().collect(),
        trigger: Some(trigger),
    }
}

fn uses(target: &str) -> Uses {
    Uses {
        target: Some(Ref {
            resource: text(target),
            output: None,
        }),
        access: Access::Call as i32,
    }
}

fn retained() -> Retained {
    Retained {
        reason: RetainReason::Undeletable as i32,
        detail: text("held by the provider"),
    }
}

fn report() -> Report {
    Report {
        created: vec![text("a/run"), text("b/run")],
        updated: vec![text("c/run")],
        resumed: vec![text("d/run")],
        deleted: vec![text("e/run")],
        reaped: vec![text("f/run")],
        retained: vec![RetainedObject {
            path: text("g/run"),
            retained: Some(retained()),
        }],
        converging: vec![text("h/run")],
        failed: vec![text("i/run")],
        blocked_by_failed: vec![Blocked {
            path: text("j/run"),
            producer: text("i/run"),
        }],
        rolled_back: vec![text("k/run")],
        rollback_failed: vec![RollbackFailure {
            path: text("l/run"),
            message: text("gone"),
        }],
        leaks: vec![leak()],
        leak_sweep_failed: Some(error()),
    }
}

fn leak() -> Leak {
    Leak {
        kind: text("service"),
        physical_id: text("svc-0001"),
        name: text("ghost-run"),
        owner: text("ghost"),
        node: text("abcdefghijklmnopqrstuvwxyz"),
    }
}

fn error() -> Error {
    Error {
        code: ErrorCode::Provider as i32,
        message: text("the provider said no"),
    }
}

fn finding() -> Finding {
    Finding {
        resource: text("web"),
        field: text("service.env.DB_URL"),
        kind: FindingKind::UnboundRef as i32,
        message: text("db is not in the list"),
    }
}

fn refused(reason: refusal::Reason) -> Outcome {
    Outcome {
        result: Some(outcome::Result::Refused(Refusal {
            reason: Some(reason),
            message: text("refused"),
        })),
        exit: ExitClass::Refused as i32,
    }
}

fn cases() -> Vec<Case> {
    let resources = ResourceList {
        resource: vec![
            Resource {
                id: text("web"),
                uses: vec![uses("db")],
                body: Some(resource::Body::Service(full_service(
                    service::Exposure::Public(service::Public {}),
                    Some(0),
                ))),
            },
            Resource {
                id: text("nightly"),
                uses: Vec::new(),
                body: Some(resource::Body::Job(full_job(
                    job::Trigger::Schedule(job::Schedule {
                        cron: text("0 * * * *"),
                        timezone: text("UTC"),
                    }),
                    Some(3),
                ))),
            },
        ],
    };
    let plan = Plan {
        steps: vec![
            Step {
                path: text("web/run"),
                kind: text("service"),
                action: StepAction::Create as i32,
                physical_id: text(""),
                retained: None,
            },
            Step {
                path: text("old/run"),
                kind: text("service"),
                action: StepAction::Retain as i32,
                physical_id: text("fake-1"),
                retained: Some(retained()),
            },
        ],
        hash: "ab".repeat(32),
    };
    vec![
        case("resource_list", "ucdk.resource.v1.ResourceList", &resources),
        case(
            "service_internal",
            "ucdk.resource.v1.Service",
            &full_service(service::Exposure::Internal(service::Internal {}), None),
        ),
        case(
            "job_on_demand",
            "ucdk.resource.v1.Job",
            &full_job(job::Trigger::OnDemand(job::OnDemand {}), None),
        ),
        // A map entry whose key and value are the default is written as an empty entry; a
        // service with no fields at all is no bytes.
        case(
            "map_defaults",
            "ucdk.resource.v1.Service",
            &Service {
                env: [(text(""), Value { v: None })].into_iter().collect(),
                secret_env: [(text(""), SecretRef::default())].into_iter().collect(),
                ..Service::default()
            },
        ),
        case(
            "session_config",
            "ucdk.api.v1.SessionConfig",
            &SessionConfig {
                provider: text("fake"),
                provider_options: [(text("region"), text("r1")), (text("a"), text("b"))]
                    .into_iter()
                    .collect(),
                ledger_kind: text("file"),
                ledger_location: text("/var/ucdk/ledger"),
                cell: text("machine/cell/uid"),
                credentials: CredentialSource::Profile as i32,
                profile: text("work"),
                holder: text("ci-7"),
            },
        ),
        case(
            "plan_request",
            "ucdk.api.v1.PlanRequest",
            &PlanRequest {
                resources: Some(resources.clone()),
                mode: Mode::Destroy as i32,
            },
        ),
        case(
            "apply_request",
            "ucdk.api.v1.ApplyRequest",
            &ApplyRequest {
                resources: Some(resources.clone()),
                plan_hash: "cd".repeat(32),
                rollback: true,
            },
        ),
        case(
            "destroy_request",
            "ucdk.api.v1.DestroyRequest",
            &DestroyRequest {
                resources: Some(resources.clone()),
                plan_hash: "cd".repeat(32),
            },
        ),
        case(
            "status_request",
            "ucdk.api.v1.StatusRequest",
            &StatusRequest {
                resources: Some(resources),
            },
        ),
        case(
            "leaks_request",
            "ucdk.api.v1.LeaksRequest",
            &LeaksRequest {},
        ),
        case(
            "outcome_planned",
            "ucdk.api.v1.Outcome",
            &Outcome {
                result: Some(outcome::Result::Planned(plan)),
                exit: ExitClass::Done as i32,
            },
        ),
        case(
            "outcome_done",
            "ucdk.api.v1.Outcome",
            &Outcome {
                result: Some(outcome::Result::Done(report())),
                exit: ExitClass::Done as i32,
            },
        ),
        case(
            "outcome_refused_findings",
            "ucdk.api.v1.Outcome",
            &refused(refusal::Reason::Findings(Findings {
                findings: vec![finding()],
            })),
        ),
        case(
            "outcome_refused_conflicts",
            "ucdk.api.v1.Outcome",
            &refused(refusal::Reason::Conflicts(Conflicts {
                conflicts: vec![Conflict {
                    path: text("web/run"),
                    kind: ConflictKind::OtherTool as i32,
                    marker: text("other-tool"),
                    message: text("owned by another tool"),
                }],
            })),
        ),
        case(
            "outcome_refused_empty_scope",
            "ucdk.api.v1.Outcome",
            &refused(refusal::Reason::EmptyScope(EmptyScope {})),
        ),
        case(
            "outcome_refused_plan_hash_mismatch",
            "ucdk.api.v1.Outcome",
            &refused(refusal::Reason::PlanHashMismatch(PlanHashMismatch {
                approved: "aa".repeat(32),
                plan: "bb".repeat(32),
            })),
        ),
        case(
            "outcome_refused_plan_changed",
            "ucdk.api.v1.Outcome",
            &refused(refusal::Reason::PlanChanged(PlanChanged {
                approved: "aa".repeat(32),
                now: "bb".repeat(32),
            })),
        ),
        case(
            "outcome_refused_unsupported_stamp_mode",
            "ucdk.api.v1.Outcome",
            &refused(refusal::Reason::UnsupportedStampMode(
                UnsupportedStampMode {
                    paths: vec![text("web/run")],
                },
            )),
        ),
        // The arms numbered above the message field: written first, as prost writes them.
        case(
            "outcome_refused_unhosted_kinds",
            "ucdk.api.v1.Outcome",
            &refused(refusal::Reason::UnhostedKinds(UnhostedKinds {
                kinds: vec![UnhostedKind {
                    path: text("batch/run"),
                    kind: text("job"),
                }],
            })),
        ),
        case(
            "outcome_refused_lease_held",
            "ucdk.api.v1.Outcome",
            &refused(refusal::Reason::LeaseHeld(LeaseHeld {
                holder: text("ci-7"),
                remaining_ms: 95_000,
            })),
        ),
        case(
            "outcome_failed",
            "ucdk.api.v1.Outcome",
            &Outcome {
                result: Some(outcome::Result::Failed(Failure {
                    report: Some(report()),
                    at: Some(text("web/run")),
                    error: Some(error()),
                })),
                exit: ExitClass::FailedChanged as i32,
            },
        ),
        // Arm 6 sits above the exit field (5) in the message but is written before it.
        case(
            "outcome_invalid",
            "ucdk.api.v1.Outcome",
            &Outcome {
                result: Some(outcome::Result::Invalid(error())),
                exit: ExitClass::Usage as i32,
            },
        ),
        case(
            "outcome_status",
            "ucdk.api.v1.Outcome",
            &Outcome {
                result: Some(outcome::Result::Status(StatusResponse {
                    objects: vec![ObjectStatus {
                        path: text("web/run"),
                        kind: text("service"),
                        record: Some(RecordState::Confirmed as i32),
                        live: Some(LiveStatus::Ready as i32),
                        standing: Standing::Agrees as i32,
                        outputs: [(text("URL"), text("https://web.example.com"))]
                            .into_iter()
                            .collect(),
                    }],
                })),
                exit: ExitClass::Done as i32,
            },
        ),
        case(
            "outcome_leaks",
            "ucdk.api.v1.Outcome",
            &Outcome {
                result: Some(outcome::Result::Leaks(LeaksResponse {
                    leaks: vec![leak()],
                })),
                exit: ExitClass::Leaks as i32,
            },
        ),
        case(
            "catalog_response",
            "ucdk.api.v1.CatalogResponse",
            &CatalogResponse {
                nodes: vec![CatalogNode {
                    path: text("web/run"),
                    kind: text("service"),
                    wanted: true,
                    deps: vec![text("db/run")],
                }],
                findings: vec![finding()],
                exit: ExitClass::Refused as i32,
            },
        ),
        case("error", "ucdk.api.v1.Error", &error()),
    ]
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdk/wire-golden")
}

fn manifest(cases: &[Case]) -> String {
    cases
        .iter()
        .map(|c| format!("{}.bin {}\n", c.file, c.message))
        .collect()
}

#[test]
fn the_golden_files_are_the_bytes_the_messages_encode_to() {
    let dir = golden_dir();
    let cases = cases();
    for c in &cases {
        let on_disk = std::fs::read(dir.join(format!("{}.bin", c.file)))
            .unwrap_or_else(|e| panic!("{}.bin is missing: {e}", c.file));
        assert_eq!(
            on_disk, c.bytes,
            "{}.bin differs from what {} encodes to now",
            c.file, c.message
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("manifest.txt")).expect("manifest.txt"),
        manifest(&cases)
    );
    // No file is left over from a case that was removed.
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("the golden directory")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let mut expected: Vec<String> = cases.iter().map(|c| format!("{}.bin", c.file)).collect();
    expected.push("manifest.txt".to_owned());
    expected.sort();
    assert_eq!(files, expected);
}

#[test]
fn the_goldens_decode_back_to_messages_that_encode_to_the_same_bytes() {
    // The Rust side of the round trip: what the other SDKs are held to.
    let resource = ResourceList::decode(cases()[0].bytes.as_slice()).expect("decodes");
    assert_eq!(resource.encode_to_vec(), cases()[0].bytes);
    let outcome = Outcome::decode(cases()[9].bytes.as_slice()).expect("decodes");
    assert_eq!(outcome.encode_to_vec(), cases()[9].bytes);
}

#[test]
#[ignore = "writes the golden files; run on purpose"]
fn write_goldens() {
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).expect("create the golden directory");
    let cases = cases();
    for old in std::fs::read_dir(&dir).expect("the golden directory") {
        std::fs::remove_file(old.expect("entry").path()).expect("remove an old golden");
    }
    for c in &cases {
        std::fs::write(dir.join(format!("{}.bin", c.file)), &c.bytes).expect("write a golden");
    }
    std::fs::write(dir.join("manifest.txt"), manifest(&cases)).expect("write the manifest");
}
