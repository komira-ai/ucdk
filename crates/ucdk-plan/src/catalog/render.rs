// SPDX-License-Identifier: Apache-2.0
//! The fields each type lowers to. The catalog's versioned defaults are written out where the
//! author wrote nothing, so that writing a default is not a change and a console edit of a
//! modelled field is drift.

use ucdk_proto::v1::{Job, Service, Size, job, service};
use ucdk_provider::{DesiredField, FieldValue, Kind};

use super::read::Extra;

/// The service port when the author wrote none.
const DEFAULT_PORT: i64 = 8080;
const DEFAULT_SIZE: &str = "1000m/512MB";
const DEFAULT_SCALE: &str = "0..10";
const DEFAULT_REQUEST_TIMEOUT: &str = "60s0n";
const DEFAULT_JOB_TIMEOUT: &str = "600s0n";

pub const ROLE_PUBLIC: &str = "public";
pub const ROLE_SCHEDULE: &str = "schedule";

fn text(name: &str, value: impl Into<String>) -> DesiredField {
    DesiredField::new(name, FieldValue::Text(value.into()))
}

fn int(name: &str, value: i64) -> DesiredField {
    DesiredField::new(name, FieldValue::Int(value))
}

pub fn size_field(size: Option<&Size>) -> DesiredField {
    let rendered = size.map_or_else(
        || DEFAULT_SIZE.to_owned(),
        |s| format!("{}m/{}MB", s.cpu_millis, s.memory_mb),
    );
    text("size", rendered)
}

/// A duration as `<seconds>s<nanos>n`, or the default when the author wrote none.
fn duration(wrote: Option<(i64, i32)>, default: &str) -> String {
    wrote.map_or_else(|| default.to_owned(), |(s, n)| format!("{s}s{n}n"))
}

/// The fields of a service's run node, and its public node.
pub fn service(svc: &Service) -> (Vec<DesiredField>, Extra) {
    let scale = svc.scale.as_ref().map_or_else(
        || DEFAULT_SCALE.to_owned(),
        |s| format!("{}..{}", s.min.unwrap_or(0), s.max),
    );
    let timeout = svc.request_timeout.as_ref().map(|d| (d.seconds, d.nanos));
    let fields = vec![
        int(
            "port",
            if svc.port == 0 {
                DEFAULT_PORT
            } else {
                i64::from(svc.port)
            },
        ),
        text("scale", scale),
        text("health_path", svc.health_path.clone()),
        text(
            "request_timeout",
            duration(timeout, DEFAULT_REQUEST_TIMEOUT),
        ),
        int("max_concurrency", i64::from(svc.max_concurrency)),
    ];
    let public = Extra {
        role: ROLE_PUBLIC,
        kind: Kind::new(ROLE_PUBLIC),
        fields: Vec::new(),
        wanted: matches!(svc.exposure, Some(service::Exposure::Public(_))),
    };
    (fields, public)
}

/// The fields of a job's run node, and its schedule node.
pub fn job(job: &Job) -> (Vec<DesiredField>, Extra) {
    let timeout = job.timeout.as_ref().map(|d| (d.seconds, d.nanos));
    let fields = vec![
        int("max_retries", i64::from(job.max_retries.unwrap_or(0))),
        text("timeout", duration(timeout, DEFAULT_JOB_TIMEOUT)),
    ];
    let (schedule_fields, wanted) = match &job.trigger {
        Some(job::Trigger::Schedule(s)) => (
            vec![
                text("cron", s.cron.clone()),
                text("timezone", s.timezone.clone()),
            ],
            true,
        ),
        _ => (Vec::new(), false),
    };
    let schedule = Extra {
        role: ROLE_SCHEDULE,
        kind: Kind::new(ROLE_SCHEDULE),
        fields: schedule_fields,
        wanted,
    };
    (fields, schedule)
}
