// SPDX-License-Identifier: Apache-2.0
//! A plain-text dump of the store and the call log, for golden tests.

use std::fmt::Write as _;

use ucdk_provider::{FieldValue, ProviderError, Status};

use crate::fake::Fake;

fn value(v: &FieldValue) -> String {
    match v {
        FieldValue::Text(t) => format!("{t:?}"),
        FieldValue::Int(i) => i.to_string(),
        FieldValue::Bool(b) => b.to_string(),
    }
}

fn status(s: Status) -> &'static str {
    match s {
        Status::Ready => "ready",
        Status::Converging => "converging",
        Status::Failed => "failed",
    }
}

fn map(m: &std::collections::BTreeMap<String, String>) -> String {
    m.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

impl Fake {
    /// Every object, then every call, one per line, in a fixed order.
    pub fn dump(&self) -> String {
        let mut out = String::from("objects:\n");
        for (id, o) in &self.objects {
            let fields: Vec<String> = o
                .fields
                .iter()
                .map(|f| format!("{}={}", f.name, value(&f.value)))
                .collect();
            let _ = writeln!(
                out,
                "  {id} {} name={} status={} labels=[{}] fields=[{}] unmodelled=[{}]",
                o.kind.as_str(),
                o.name,
                status(o.status),
                map(&o.labels),
                fields.join(","),
                map(&o.unmodelled)
            );
        }
        out.push_str("calls:\n");
        for (n, c) in self.calls.iter().enumerate() {
            let result = match &c.outcome {
                Ok(()) => "ok".to_owned(),
                Err(ProviderError::Refused(why)) => format!("refused {why:?}"),
                Err(e) => e.to_string(),
            };
            let physical = c.physical.as_ref().map_or("-", |p| p.as_str());
            let _ = writeln!(
                out,
                "  {} {} {} {} {result}",
                n + 1,
                c.op.name(),
                c.kind.as_ref().map_or("-", |k| k.as_str()),
                physical
            );
        }
        out
    }
}
