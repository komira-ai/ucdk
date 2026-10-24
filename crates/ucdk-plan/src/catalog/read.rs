// SPDX-License-Identifier: Apache-2.0
//! Reading a resource list: every check, every finding, and the typed data lowering needs.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_proto::v1::r#ref::Output as Arm;
use ucdk_proto::v1::resource::Body as Wire;
use ucdk_proto::v1::{
    Access, Image, Job, Output, Ref, Resource, ResourceList, SecretRef, Service, Size, Uses, Value,
    image, value,
};
use ucdk_provider::{DesiredField, FieldValue, Kind, OwnerLabel};

use super::finding::{Finding, FindingKind};
use super::lower::ValueRef;
use super::render;
use crate::graph;

/// The only platform a v1 image may name; the empty platform means it.
const PLATFORM: &str = "linux/amd64";

/// A resource that passed every check, with what lowering needs.
pub struct Spec {
    pub id: String,
    pub kind: Kind,
    pub run_fields: Vec<DesiredField>,
    /// The fields of the run node whose value is another resource's output.
    pub run_refs: Vec<ValueRef>,
    /// The resources whose run node this one's run node needs.
    pub run_deps: BTreeSet<String>,
    pub extra: Extra,
}

/// The second role of a type: `public` for a service, `schedule` for a job. Its node is always
/// emitted; `wanted` says whether the file turns it on.
pub struct Extra {
    pub role: &'static str,
    pub kind: Kind,
    pub fields: Vec<DesiredField>,
    pub wanted: bool,
}

#[derive(Clone, Copy)]
enum Body<'a> {
    Service(&'a Service),
    Job(&'a Job),
}

impl<'a> Body<'a> {
    fn of(resource: &'a Resource) -> Option<Self> {
        match &resource.body {
            Some(Wire::Service(s)) => Some(Self::Service(s)),
            Some(Wire::Job(j)) => Some(Self::Job(j)),
            None => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Service(_) => "service",
            Self::Job(_) => "job",
        }
    }

    /// The outputs the type exposes to a reference.
    fn exposes(self) -> &'static [Output] {
        match self {
            Self::Service(_) => &[Output::Url, Output::Host],
            Self::Job(_) => &[],
        }
    }

    fn image(self) -> Option<&'a Image> {
        match self {
            Self::Service(s) => s.image.as_ref(),
            Self::Job(j) => j.image.as_ref(),
        }
    }

    fn args(self) -> &'a [String] {
        match self {
            Self::Service(s) => &s.args,
            Self::Job(j) => &j.args,
        }
    }

    fn env(self) -> &'a BTreeMap<String, Value> {
        match self {
            Self::Service(s) => &s.env,
            Self::Job(j) => &j.env,
        }
    }

    fn secret_env(self) -> &'a BTreeMap<String, SecretRef> {
        match self {
            Self::Service(s) => &s.secret_env,
            Self::Job(j) => &j.secret_env,
        }
    }

    fn size(self) -> Option<&'a Size> {
        match self {
            Self::Service(s) => s.size.as_ref(),
            Self::Job(j) => j.size.as_ref(),
        }
    }
}

/// What the checks of one resource know and collect.
struct Reader<'a> {
    id: &'a str,
    /// Every resource of the list by id, with the outputs its type exposes.
    exposes: &'a BTreeMap<&'a str, &'static [Output]>,
    findings: Vec<Finding>,
    deps: BTreeSet<String>,
    refs: Vec<ValueRef>,
}

/// A value as written: text now, or another resource's output later.
enum Read {
    Text(String),
    Ref {
        resource: String,
        output: &'static str,
    },
}

impl Reader<'_> {
    fn report(&mut self, field: &str, kind: FindingKind) {
        self.findings.push(Finding::new(self.id, field, kind));
    }

    /// Resolves a reference to a resource of the list, recording the edge. An unbound target
    /// is a finding.
    fn bind(&mut self, field: &str, target: &Ref) -> Option<&'static [Output]> {
        let found = self.exposes.get(target.resource.as_str()).copied();
        if found.is_some() {
            self.deps.insert(target.resource.clone());
        } else {
            let kind = FindingKind::UnboundRef {
                target: target.resource.clone(),
            };
            self.report(field, kind);
        }
        found
    }

    /// A value reference: the output it names, known once the target is applied.
    fn value_ref(&mut self, field: &str, target: &Ref) -> Option<Read> {
        let exposed = self.bind(field, target)?;
        let standard = match &target.output {
            Some(Arm::Standard(number)) => Output::try_from(*number).ok(),
            Some(Arm::Named(_)) => {
                self.report(field, FindingKind::NamedOutput);
                return None;
            }
            None => None,
        };
        match standard {
            Some(output @ (Output::Url | Output::Host)) if exposed.contains(&output) => {
                Some(Read::Ref {
                    resource: target.resource.clone(),
                    output: output.as_str_name(),
                })
            }
            Some(output @ (Output::Url | Output::Host)) => {
                let kind = FindingKind::UnexposedOutput {
                    target: target.resource.clone(),
                    output: output.as_str_name(),
                };
                self.report(field, kind);
                None
            }
            _ => {
                self.report(field, FindingKind::MissingOutput);
                None
            }
        }
    }

    fn value(&mut self, field: &str, wire: &Value) -> Option<Read> {
        match &wire.v {
            Some(value::V::Literal(text)) => Some(Read::Text(text.clone())),
            Some(value::V::Ref(target)) => self.value_ref(field, target),
            Some(value::V::Param(_)) => {
                self.report(field, FindingKind::Unresolved { what: "parameter" });
                None
            }
            None => {
                self.report(field, FindingKind::EmptyValue);
                None
            }
        }
    }

    fn image(&mut self, field: &str, image: Option<&Image>) -> Option<DesiredField> {
        let Some(image) = image else {
            self.report(field, FindingKind::MissingImage);
            return None;
        };
        let platform = if image.platform.is_empty() {
            PLATFORM
        } else {
            image.platform.as_str()
        };
        if platform != PLATFORM {
            let kind = FindingKind::UnsupportedPlatform {
                platform: platform.to_owned(),
            };
            self.report(field, kind);
        }
        match &image.source {
            Some(image::Source::Digest(digest)) if !digest.is_empty() => Some(DesiredField::new(
                "image",
                FieldValue::Text(format!("{digest}@{platform}")),
            )),
            Some(image::Source::Output(_)) => {
                self.report(
                    field,
                    FindingKind::Unresolved {
                        what: "build step output",
                    },
                );
                None
            }
            _ => {
                self.report(field, FindingKind::MissingImage);
                None
            }
        }
    }

    fn uses(&mut self, index: usize, uses: &Uses) {
        let field = format!("uses[{index}]");
        if uses.access != Access::Call as i32 {
            self.report(&field, FindingKind::UnsupportedAccess);
        }
        match &uses.target {
            None => self.report(&field, FindingKind::NoTarget),
            Some(target) => {
                if self.bind(&field, target).is_some() && target.output.is_some() {
                    self.report(&field, FindingKind::AccessToOutput);
                }
            }
        }
    }

    /// The fields every workload shares, in the order they are checked.
    fn common(&mut self, body: Body<'_>) -> Vec<DesiredField> {
        let name = body.name();
        let mut fields = Vec::new();
        fields.extend(self.image(&format!("{name}.image"), body.image()));
        fields.extend(
            body.args().iter().enumerate().map(|(i, arg)| {
                DesiredField::new(format!("arg.{i}"), FieldValue::Text(arg.clone()))
            }),
        );
        for (key, wire) in body.env() {
            match self.value(&format!("{name}.env.{key}"), wire) {
                Some(Read::Text(t)) => {
                    fields.push(DesiredField::new(format!("env.{key}"), FieldValue::Text(t)));
                }
                Some(Read::Ref { resource, output }) => self.refs.push(ValueRef {
                    field: format!("env.{key}"),
                    resource,
                    output,
                }),
                None => {}
            }
            if body.secret_env().contains_key(key) {
                self.report(&format!("{name}.env.{key}"), FindingKind::EnvSecretOverlap);
            }
        }
        for (key, secret) in body.secret_env() {
            let store = secret
                .store
                .as_ref()
                .map_or_else(String::new, |s| format!("{s}/"));
            let version = secret
                .version
                .as_ref()
                .map_or_else(String::new, |v| format!("@{v}"));
            fields.push(DesiredField::new(
                format!("secret_env.{key}"),
                FieldValue::Text(format!("{store}{}{version}", secret.name)),
            ));
        }
        fields.push(render::size_field(body.size()));
        fields
    }
}

/// Reads the list: every finding if anything is wrong, else the resources in dependency
/// order.
pub fn read(list: &ResourceList) -> Result<Vec<Spec>, Vec<Finding>> {
    let mut findings = Vec::new();
    let mut exposes: BTreeMap<&str, &'static [Output]> = BTreeMap::new();
    let mut typed = Vec::new();
    for resource in &list.resource {
        let id = resource.id.as_str();
        if OwnerLabel::new(id).is_err() {
            findings.push(Finding::new(id, "", FindingKind::BadId));
        } else if exposes.contains_key(id) {
            findings.push(Finding::new(id, "", FindingKind::DuplicateId));
        } else if let Some(body) = Body::of(resource) {
            exposes.insert(id, body.exposes());
            typed.push((resource, body));
        } else {
            findings.push(Finding::new(id, "", FindingKind::MissingBody));
            exposes.insert(id, &[]);
        }
    }

    let mut specs = Vec::new();
    for (resource, body) in typed {
        let mut reader = Reader {
            id: &resource.id,
            exposes: &exposes,
            findings: Vec::new(),
            deps: BTreeSet::new(),
            refs: Vec::new(),
        };
        let mut run_fields = reader.common(body);
        for (index, uses) in resource.uses.iter().enumerate() {
            reader.uses(index, uses);
        }
        let (own, extra) = match body {
            Body::Service(service) => render::service(service),
            Body::Job(job) => render::job(job),
        };
        run_fields.extend(own);
        findings.append(&mut reader.findings);
        specs.push(Spec {
            id: resource.id.clone(),
            kind: Kind::new(body.name()),
            run_fields,
            run_refs: reader.refs,
            run_deps: reader.deps,
            extra,
        });
    }

    let edges: BTreeMap<String, BTreeSet<String>> = specs
        .iter()
        .map(|spec| (spec.id.clone(), spec.run_deps.clone()))
        .collect();
    let order = graph::order(&edges).unwrap_or_else(|members| {
        let first = members.first().cloned().unwrap_or_default();
        findings.push(Finding::new(&first, "", FindingKind::Cycle { members }));
        Vec::new()
    });
    if !findings.is_empty() {
        return Err(findings);
    }
    let mut by_id: BTreeMap<String, Spec> = specs
        .into_iter()
        .map(|spec| (spec.id.clone(), spec))
        .collect();
    Ok(order.iter().flat_map(|id| by_id.remove(id)).collect())
}
