// SPDX-License-Identifier: Apache-2.0
//! Typed templates for the resource kinds, and typed references to their outputs.
//!
//! The kinds are the arms of the `body` oneof of `Resource`; for each, one class that takes the
//! fields of the kind's message as arguments (the `Image`, which nothing can run without, as a
//! required one), each oneof of the kind as one argument that takes either arm, and a property
//! per output the kind `exposes`. The `portability` marking and the exposed outputs are read
//! from the message options, so a new kind in `resource.proto` is a new class here with no
//! generator change. The shapes the classes know by name (`Image`, `Value`, `Uses`, `Ref`) are
//! the ones the planner knows too.

use prost_reflect::{
    DescriptorPool, EnumDescriptor, ExtensionDescriptor, MessageDescriptor, OneofDescriptor, Value,
};

use super::message::{Field, Shape, Unit, param, units};
use super::types::{Scalar, member_name, member_ref};
use crate::names::{docstring, local_name};
use crate::{Error, docs};

const RESOURCE: &str = "ucdk.resource.v1.Resource";
const IMAGE: &str = "ucdk.resource.v1.Image";
const VALUE: &str = "ucdk.resource.v1.Value";
const NEEDED: &str = "ucdk.resource.v1.Resource with a oneof `body`, the options `portability` and \
    `exposes`, and the enums Portability, Output and Access";
const BASE: &str = include_str!("template_base.py");

/// Names a template keeps for itself; a field of a kind may not be called one.
const OWN: [&str; 6] = ["id", "uses", "ref", "body", "to_resource", "KIND"];

/// What the templates are built from, found in the descriptors.
struct Catalog {
    body: OneofDescriptor,
    portability: ExtensionDescriptor,
    exposes: ExtensionDescriptor,
    marking: EnumDescriptor,
    output: EnumDescriptor,
    access: EnumDescriptor,
}

fn find(pool: &DescriptorPool, resource: &MessageDescriptor) -> Result<Catalog, Error> {
    let found = (
        resource.oneofs().find(|o| o.name() == "body"),
        pool.get_extension_by_name("ucdk.resource.v1.portability"),
        pool.get_extension_by_name("ucdk.resource.v1.exposes"),
        pool.get_enum_by_name("ucdk.resource.v1.Portability"),
        pool.get_enum_by_name("ucdk.resource.v1.Output"),
        pool.get_enum_by_name("ucdk.resource.v1.Access"),
    );
    let (Some(body), Some(portability), Some(exposes), Some(marking), Some(output), Some(access)) =
        found
    else {
        return Err(Error::Missing(NEEDED.to_owned()));
    };
    Ok(Catalog {
        body,
        portability,
        exposes,
        marking,
        output,
        access,
    })
}

/// The Python module of the templates, or `None` when the protos declare no `Resource`.
pub fn generate(pool: &DescriptorPool, modules: &[String]) -> Result<Option<String>, Error> {
    let Some(resource) = pool.get_message_by_name(RESOURCE) else {
        return Ok(None);
    };
    let catalog = find(pool, &resource)?;
    // What a bare `Ref` in `uses` is granted: the first access that is not "unset".
    let first = catalog.access.default_value();
    let granted = catalog
        .access
        .values()
        .find(|v| v.number() != 0)
        .unwrap_or(first);
    let access = member_ref(&catalog.access, granted.name(), "");
    let mut out: Vec<String> = super::HEADER.lines().map(str::to_owned).collect();
    out.extend(DOC.lines().map(str::to_owned));
    out.extend(modules.iter().map(|m| format!("from . import {m}")));
    out.extend(BASE.replace("@ACCESS@", &access).lines().map(str::to_owned));
    let kinds: Vec<(String, MessageDescriptor)> = catalog
        .body
        .fields()
        .filter_map(|arm| {
            arm.kind()
                .as_message()
                .map(|m| (arm.name().to_owned(), m.clone()))
        })
        .collect();
    for (kind, message) in &kinds {
        out.extend(template(&catalog, kind, message)?);
    }
    Ok(Some(out.join("\n") + "\n"))
}

const DOC: &str = "\"\"\"Typed resource templates and typed references to their outputs.

A template is built from its fields, so a missing image or a misspelt field is a Python error
and not a refusal at plan time. `to_resource()` gives the wire message.
\"\"\"

from __future__ import annotations

from typing import Any, Dict, Iterable, Mapping, Optional, Union

from . import _wire
";

fn is_image(f: &Field) -> bool {
    matches!(&f.scalar, Scalar::Message(m) if m.full_name() == IMAGE)
        && matches!(f.shape, Shape::Optional)
}

fn is_values(f: &Field) -> bool {
    matches!((&f.shape, &f.scalar), (Shape::Map(_), Scalar::Message(m)) if m.full_name() == VALUE)
}

/// The name of a member of `e`, or an empty string for a number it does not have.
fn member_of(e: &EnumDescriptor, number: i32) -> String {
    let value = e.get_value(number).unwrap_or(e.default_value());
    member_name(e, value.name())
}

fn enum_expr(e: &EnumDescriptor, number: i32) -> String {
    let value = e.get_value(number).unwrap_or(e.default_value());
    member_ref(e, value.name(), "")
}

fn tuple(items: &[String]) -> String {
    match items.len() {
        0 => "()".to_owned(),
        1 => format!("({},)", items[0]),
        _ => format!("({})", items.join(", ")),
    }
}

/// The messages a oneof argument takes, one per arm, each different: the name each is known by
/// inside the template class, and its Python type.
fn arm_types(arms: &[Field]) -> Result<Vec<(String, String)>, Error> {
    let mut seen: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for arm in arms {
        let Scalar::Message(m) = &arm.scalar else {
            return Err(Error::refused(
                &arm.attr,
                "an arm of a oneof of a resource kind must be a message",
            ));
        };
        if seen.iter().any(|s| s == m.full_name()) {
            return Err(Error::refused(
                &arm.attr,
                "two arms of a oneof of a resource kind hold the same message",
            ));
        }
        seen.push(m.full_name().to_owned());
        out.push((m.name().to_owned(), arm.scalar.annotation("")));
    }
    Ok(out)
}

fn template(
    catalog: &Catalog,
    kind: &str,
    message: &MessageDescriptor,
) -> Result<Vec<String>, Error> {
    let class = message.name();
    let units = units(message)?;
    let options = message.options();
    let marking = options
        .get_extension(&catalog.portability)
        .as_enum_number()
        .unwrap_or(0);
    let exposed: Vec<i32> = options
        .get_extension(&catalog.exposes)
        .as_list()
        .unwrap_or_default()
        .iter()
        .filter_map(Value::as_enum_number)
        .collect();

    let mut required: Vec<&Field> = Vec::new();
    let mut optional: Vec<&Field> = Vec::new();
    let mut oneofs: Vec<(&str, &Vec<Field>)> = Vec::new();
    for unit in &units {
        match unit {
            Unit::Field(f) if is_image(f) => required.push(f),
            Unit::Field(f) => optional.push(f),
            Unit::Oneof { name, arms } => oneofs.push((name, arms)),
        }
    }
    let properties: Vec<(String, i32)> = exposed
        .iter()
        .map(|n| (member_of(&catalog.output, *n).to_lowercase(), *n))
        .collect();
    let mut names: Vec<&str> = required
        .iter()
        .chain(&optional)
        .map(|f| f.attr.as_str())
        .collect();
    names.extend(oneofs.iter().map(|(n, _)| *n));
    names.extend(properties.iter().map(|(n, _)| n.as_str()));
    if let Some(taken) = OWN.iter().find(|own| names.contains(own)) {
        return Err(Error::refused(
            message.full_name(),
            format!("a field, oneof or output named `{taken}` collides with the template's own"),
        ));
    }

    let doc =
        docs::leading(message.parent_file_descriptor_proto(), message.path()).unwrap_or_default();
    let mut out = vec![
        String::new(),
        String::new(),
        format!("class {class}(_Template):"),
        "    \"\"\"".to_owned(),
    ];
    out.extend(docstring(&format!("The `{kind}` resource.\n\n{doc}"), 4));
    out.push("    \"\"\"".to_owned());
    out.push(String::new());
    out.push(format!("    KIND = \"{kind}\""));
    out.push(format!(
        "    PORTABILITY = {}",
        enum_expr(&catalog.marking, marking)
    ));
    let members: Vec<String> = exposed
        .iter()
        .map(|n| enum_expr(&catalog.output, *n))
        .collect();
    out.push(format!("    EXPOSES = {}", tuple(&members)));

    let mut arm_lists: Vec<(&str, &Vec<Field>, Vec<String>)> = Vec::new();
    for (oneof, arms) in &oneofs {
        let types = arm_types(arms)?;
        out.extend(types.iter().map(|(name, ty)| format!("    {name} = {ty}")));
        arm_lists.push((oneof, arms, types.into_iter().map(|(_, ty)| ty).collect()));
    }

    out.push(String::new());
    out.push("    def __init__(".to_owned());
    out.push("        self,".to_owned());
    out.push("        id: str,".to_owned());
    for f in &required {
        out.push(format!("        {}: {},", f.attr, f.scalar.annotation("")));
    }
    out.push("        *,".to_owned());
    out.push(
        "        uses: Iterable[Union[ucdk_resource_v1.Uses, ucdk_resource_v1.Ref, _Template]] = (),"
            .to_owned(),
    );
    for f in &optional {
        if is_values(f) {
            out.push(format!(
                "        {}: Optional[Mapping[str, Union[ucdk_resource_v1.Value, str, ucdk_resource_v1.Ref]]] = None,",
                f.attr
            ));
        } else {
            out.push(format!("        {},", param(f, "")));
        }
    }
    for (oneof, _, types) in &arm_lists {
        out.push(format!(
            "        {oneof}: Union[{}, None] = None,",
            types.join(", ")
        ));
    }
    out.push("    ) -> None:".to_owned());
    out.push("        super().__init__(id, uses)".to_owned());
    let wire = local_name(message.full_name(), message.package_name());
    out.push(format!("        body = ucdk_resource_v1.{wire}("));
    for f in required.iter().chain(&optional) {
        let value = if is_values(f) {
            format!("_values({})", f.attr)
        } else {
            f.attr.clone()
        };
        out.push(format!("            {}={value},", f.attr));
    }
    out.push("        )".to_owned());
    for (oneof, arms, types) in &arm_lists {
        let pairs: Vec<String> = arms
            .iter()
            .zip(types)
            .map(|(a, ty)| format!("(\"{}\", {ty})", a.attr))
            .collect();
        out.push(format!(
            "        _wire.set_oneof(body, {oneof}, ({},))",
            pairs.join(", ")
        ));
    }
    out.push("        self.body = body".to_owned());
    for (property, number) in &properties {
        out.push(String::new());
        out.push("    @property".to_owned());
        out.push(format!("    def {property}(self) -> ucdk_resource_v1.Ref:"));
        out.push(format!(
            "        \"\"\"A reference to the `{}` this resource exposes once applied.\"\"\"",
            member_of(&catalog.output, *number)
        ));
        out.push(format!(
            "        return ucdk_resource_v1.Ref(resource=self.id, standard={})",
            enum_expr(&catalog.output, *number)
        ));
    }
    Ok(out)
}
