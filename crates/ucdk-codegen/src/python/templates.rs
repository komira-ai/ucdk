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

use prost_reflect::{DescriptorPool, EnumDescriptor};

use super::message::param;
use super::types::member_ref;
use crate::Error;
use crate::kinds::{self, Catalog, Kind, Naming, is_values, member_of, value_of};
use crate::names::{docstring, ident, local_name};

const BASE: &str = include_str!("template_base.py");

/// Names a template keeps for itself; a field of a kind may not be called one.
const NAMING: Naming = Naming {
    name: ident,
    own: &["id", "uses", "ref", "body", "to_resource", "KIND"],
};

/// The Python module of the templates, or `None` when the protos declare no `Resource`.
pub fn generate(pool: &DescriptorPool, modules: &[String]) -> Result<Option<String>, Error> {
    let Some(catalog) = kinds::find(pool)? else {
        return Ok(None);
    };
    let access = member_ref(&catalog.access, catalog.granted().name(), "");
    let mut out: Vec<String> = super::HEADER.lines().map(str::to_owned).collect();
    out.extend(DOC.lines().map(str::to_owned));
    out.extend(modules.iter().map(|m| format!("from . import {m}")));
    out.extend(BASE.replace("@ACCESS@", &access).lines().map(str::to_owned));
    for (name, message) in catalog.arms() {
        out.extend(template(
            &catalog,
            &kinds::read(&catalog, &name, &message, &NAMING)?,
        ));
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

fn enum_expr(e: &EnumDescriptor, number: i32) -> String {
    member_ref(e, value_of(e, number).name(), "")
}

fn tuple(items: &[String]) -> String {
    match items.len() {
        0 => "()".to_owned(),
        1 => format!("({},)", items[0]),
        _ => format!("({})", items.join(", ")),
    }
}

fn template(catalog: &Catalog, kind: &Kind) -> Vec<String> {
    let class = kind.message.name();
    let mut out = vec![
        String::new(),
        String::new(),
        format!("class {class}(_Template):"),
        "    \"\"\"".to_owned(),
    ];
    out.extend(docstring(
        &format!("The `{}` resource.\n\n{}", kind.name, kind.doc),
        4,
    ));
    out.push("    \"\"\"".to_owned());
    out.push(String::new());
    out.push(format!("    KIND = \"{}\"", kind.name));
    out.push(format!(
        "    PORTABILITY = {}",
        enum_expr(&catalog.marking, kind.marking)
    ));
    let members: Vec<String> = kind
        .exposed
        .iter()
        .map(|n| enum_expr(&catalog.output, *n))
        .collect();
    out.push(format!("    EXPOSES = {}", tuple(&members)));

    // Each oneof argument takes either arm: the name each is known by inside the class, and its
    // Python type.
    let mut arm_lists: Vec<(&str, Vec<(&str, String)>, Vec<String>)> = Vec::new();
    for oneof in &kind.oneofs {
        let arms: Vec<(&str, String)> = oneof
            .arms
            .iter()
            .map(|(arm, _)| (arm.attr.as_str(), arm.scalar.annotation("")))
            .collect();
        for ((_, message), (_, ty)) in oneof.arms.iter().zip(&arms) {
            out.push(format!("    {} = {ty}", message.name()));
        }
        let types = arms.iter().map(|(_, ty)| ty.clone()).collect();
        arm_lists.push((&oneof.name, arms, types));
    }

    out.push(String::new());
    out.push("    def __init__(".to_owned());
    out.push("        self,".to_owned());
    out.push("        id: str,".to_owned());
    for f in &kind.required {
        out.push(format!("        {}: {},", f.attr, f.scalar.annotation("")));
    }
    out.push("        *,".to_owned());
    out.push(
        "        uses: Iterable[Union[ucdk_resource_v1.Uses, ucdk_resource_v1.Ref, _Template]] = (),"
            .to_owned(),
    );
    for f in &kind.optional {
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
    let wire = local_name(kind.message.full_name(), kind.message.package_name());
    out.push(format!("        body = ucdk_resource_v1.{wire}("));
    for f in kind.required.iter().chain(&kind.optional) {
        let value = if is_values(f) {
            format!("_values({})", f.attr)
        } else {
            f.attr.clone()
        };
        out.push(format!("            {}={value},", f.attr));
    }
    out.push("        )".to_owned());
    for (oneof, arms, _) in &arm_lists {
        let pairs: Vec<String> = arms
            .iter()
            .map(|(attr, ty)| format!("(\"{attr}\", {ty})"))
            .collect();
        out.push(format!(
            "        _wire.set_oneof(body, {oneof}, ({},))",
            pairs.join(", ")
        ));
    }
    out.push("        self.body = body".to_owned());
    for (property, number) in &kind.properties {
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
    out
}
