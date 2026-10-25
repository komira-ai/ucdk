// SPDX-License-Identifier: Apache-2.0
//! Reading a resource list: every check, every finding, and the typed data lowering needs.

use std::collections::{BTreeMap, BTreeSet};

use prost_reflect::{DynamicMessage, Value as Pb};
use ucdk_proto::v1::r#ref::Output as Arm;
use ucdk_proto::v1::{
    Access, Image, Output, Portability, Ref, SecretRef, Uses, Value, image, value,
};
use ucdk_provider::{DesiredField, FieldValue, Kind, OwnerLabel};

use super::descriptor::{Catalog, FieldPlan, KindPlan, Shape, entries, scalar};
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
    pub roles: Vec<Role>,
}

/// A further role of a type (`public` for a service, `schedule` for a job). Its node is always
/// emitted; `wanted` says whether the file turns it on.
pub struct Role {
    pub role: String,
    pub kind: Kind,
    pub fields: Vec<DesiredField>,
    pub wanted: bool,
}

/// A shared-shape message of the catalog's own package, from its dynamic form. The dynamic and
/// the generated message come from one schema (the catalog checked the names), so the round
/// trip through the wire cannot fail.
fn typed<T: prost::Message + Default>(message: &DynamicMessage) -> T {
    message
        .transcode_to()
        .expect("a shared shape of the catalog has the schema of its generated type")
}

/// What the checks of one resource know and collect.
struct Reader<'a> {
    id: &'a str,
    /// Every resource of the list by id, with the outputs its type exposes.
    exposes: &'a BTreeMap<String, &'a [Output]>,
    /// The resources whose type is marked `CLOUD_BOUND`.
    cloud_bound: &'a BTreeSet<String>,
    findings: Vec<Finding>,
    deps: BTreeSet<String>,
    refs: Vec<ValueRef>,
}

/// A value as written: text now, or another resource's output later.
enum Read {
    Text(String),
    Ref { resource: String, output: Output },
}

impl<'a> Reader<'a> {
    fn report(&mut self, field: &str, kind: FindingKind) {
        self.findings.push(Finding::new(self.id, field, kind));
    }

    /// Resolves a reference to a resource of the list, recording the edge. An unbound target
    /// is a finding, and so is a target whose type is not portable: another resource's
    /// node cannot be bound to something only one cloud hosts.
    fn bind(&mut self, field: &str, target: &Ref) -> Option<&'a [Output]> {
        let found = self.exposes.get(target.resource.as_str()).copied();
        if found.is_some() && self.cloud_bound.contains(target.resource.as_str()) {
            let kind = FindingKind::NotPortable {
                target: target.resource.clone(),
            };
            self.report(field, kind);
            return None;
        }
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
                    output,
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

    fn image(&mut self, field: &str, stem: &str, image: Option<&Image>) -> Option<DesiredField> {
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
                stem,
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

    /// The fields of the run node, one plan per field of the type's message, in the order the
    /// message declares them.
    fn fields(&mut self, kind: &KindPlan, body: &DynamicMessage) -> Vec<DesiredField> {
        let mut fields = Vec::new();
        for plan in &kind.fields {
            let name = format!("{}.{}", kind.name, plan.field.name());
            let written = body.has_field(&plan.field);
            let value = body.get_field(&plan.field);
            match &plan.shape {
                Shape::Image => {
                    let image = value.as_message().filter(|_| written).map(typed::<Image>);
                    fields.extend(self.image(&name, &plan.stem, image.as_ref()));
                }
                Shape::List => fields.extend(
                    value
                        .as_list()
                        .unwrap_or_default()
                        .iter()
                        .enumerate()
                        .map(|(i, v)| DesiredField::new(format!("{}.{i}", plan.stem), scalar(v))),
                ),
                Shape::Env => self.env(kind, body, plan, &name, &value, &mut fields),
                Shape::Secrets => {
                    for (key, secret) in entries(&value) {
                        let secret = secret
                            .as_message()
                            .map(typed::<SecretRef>)
                            .unwrap_or_default();
                        fields.push(DesiredField::new(
                            format!("{}.{key}", plan.stem),
                            FieldValue::Text(render::secret(&secret)),
                        ));
                    }
                }
                Shape::Scalar | Shape::Render(_) => {
                    fields.push(render::single(plan, written, &value));
                }
            }
        }
        fields
    }

    /// A `map<string, Value>` field: literals become fields, output references become refs.
    /// A key that is also in a `map<string, SecretRef>` field of the message is refused.
    fn env(
        &mut self,
        kind: &KindPlan,
        body: &DynamicMessage,
        plan: &FieldPlan,
        name: &str,
        map: &Pb,
        fields: &mut Vec<DesiredField>,
    ) {
        for (key, wire) in entries(map) {
            let wire = wire.as_message().map(typed::<Value>).unwrap_or_default();
            let node_field = format!("{}.{key}", plan.stem);
            match self.value(&format!("{name}.{key}"), &wire) {
                Some(Read::Text(t)) => {
                    fields.push(DesiredField::new(node_field, FieldValue::Text(t)));
                }
                Some(Read::Ref { resource, output }) => self.refs.push(ValueRef {
                    field: node_field,
                    resource,
                    output,
                }),
                None => {}
            }
            let also_secret = kind
                .fields
                .iter()
                .filter(|other| matches!(other.shape, Shape::Secrets))
                .any(|other| {
                    entries(&body.get_field(&other.field))
                        .iter()
                        .any(|e| e.0 == key)
                });
            if also_secret {
                self.report(&format!("{name}.{key}"), FindingKind::EnvSecretOverlap);
            }
        }
    }
}

/// Reads the list: every finding if anything is wrong, else the resources in dependency
/// order. `portability` says how portable each type is, by its name.
pub fn read(
    catalog: &Catalog,
    list: &DynamicMessage,
    portability: impl Fn(&str) -> Portability,
) -> Result<Vec<Spec>, Vec<Finding>> {
    let mut findings = Vec::new();
    let mut cloud_bound: BTreeSet<String> = BTreeSet::new();
    let mut exposes: BTreeMap<String, &[Output]> = BTreeMap::new();
    let mut typed_resources = Vec::new();
    let resources = list.get_field(&catalog.resources);
    for resource in resources
        .as_list()
        .unwrap_or_default()
        .iter()
        .filter_map(Pb::as_message)
    {
        let id = resource
            .get_field(&catalog.id)
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let found = catalog.kinds.iter().find_map(|k| {
            let body = resource.get_field(&k.arm);
            body.as_message()
                .filter(|_| resource.has_field(&k.arm))
                .map(|b| (k, b.clone()))
        });
        if OwnerLabel::new(&id).is_err() {
            findings.push(Finding::new(&id, "", FindingKind::BadId));
        } else if exposes.contains_key(&id) {
            findings.push(Finding::new(&id, "", FindingKind::DuplicateId));
        } else if let Some((kind, body)) = found {
            exposes.insert(id.clone(), &kind.exposes);
            if portability(&kind.name) == Portability::CloudBound {
                cloud_bound.insert(id.clone());
            }
            typed_resources.push((id, resource, kind, body));
        } else {
            findings.push(Finding::new(&id, "", FindingKind::MissingBody));
            exposes.insert(id, &[]);
        }
    }

    let mut specs = Vec::new();
    for (id, resource, kind, body) in typed_resources {
        let mut reader = Reader {
            id: &id,
            exposes: &exposes,
            cloud_bound: &cloud_bound,
            findings: Vec::new(),
            deps: BTreeSet::new(),
            refs: Vec::new(),
        };
        let run_fields = reader.fields(kind, &body);
        let uses = resource.get_field(&catalog.uses);
        for (index, wire) in uses
            .as_list()
            .unwrap_or_default()
            .iter()
            .filter_map(Pb::as_message)
            .enumerate()
        {
            reader.uses(index, &typed::<Uses>(wire));
        }
        let roles = render::roles(kind, &body);
        let Reader {
            findings: found,
            refs,
            deps,
            ..
        } = reader;
        findings.extend(found);
        specs.push(Spec {
            id,
            kind: Kind::new(&kind.name),
            run_fields,
            run_refs: refs,
            run_deps: deps,
            roles,
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
