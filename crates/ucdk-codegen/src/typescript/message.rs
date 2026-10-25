// SPDX-License-Identifier: Apache-2.0
//! One message as a TypeScript class, and one enum as a constant object.
//!
//! A message class holds its fields as plain properties, built by a constructor that takes an
//! object of them (all optional: a field left out is its default), and encodes and decodes
//! itself with the functions of `wire.ts`, writing the bytes the Rust side writes (the rules are
//! in `model`). A `oneof` is one property per arm; `whichChoice()` names the arm that is set.
//! Only syntax Node can run by stripping the types is written: no `enum`, no parameter
//! properties.

use std::collections::BTreeSet;

use prost_reflect::{EnumDescriptor, MessageDescriptor};

use super::names::{STATIC_TAKEN, doc, member, pair, param, pascal};
use super::types::{default_value, get, is_set, member_name, put, ty};
use crate::model::{Field, Scalar, Shape, Unit, fields_of, units as read_units};
use crate::names::local_name;
use crate::{Error, docs};

/// The fields of `message` as units, in the order they are written.
pub fn units(message: &MessageDescriptor) -> Result<Vec<Unit>, Error> {
    read_units(message, member)
}

/// The type of an optional property of the constructor's object: what a user may give.
pub fn init_type(f: &Field, current: &str) -> String {
    let t = ty(&f.scalar, current);
    match &f.shape {
        Shape::Plain | Shape::Optional | Shape::Arm => t,
        Shape::Repeated => format!("Iterable<{t}>"),
        Shape::Map(_) => format!("wire.MapInit<{t}>"),
    }
}

/// The type of the property that holds `f`.
fn property_type(f: &Field, current: &str) -> String {
    let t = ty(&f.scalar, current);
    match &f.shape {
        Shape::Plain => t,
        Shape::Optional | Shape::Arm => format!("{t} | undefined"),
        Shape::Repeated => format!("{t}[]"),
        Shape::Map(_) => format!("Map<string, {t}>"),
    }
}

fn indent(lines: Vec<String>, by: usize) -> impl Iterator<Item = String> {
    let pad = " ".repeat(by);
    lines.into_iter().map(move |l| format!("{pad}{l}"))
}

fn quoted_list(names: &[String]) -> String {
    let items: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    items.join(", ")
}

fn encode_field(f: &Field) -> Vec<String> {
    let a = &f.attr;
    let this = format!("this.{a}");
    match &f.shape {
        Shape::Plain => vec![
            format!("if ({}) {{", is_set(&f.scalar, &this)),
            format!("  {}", put(&f.scalar, "out", f.number, &this)),
            "}".to_owned(),
        ],
        Shape::Optional | Shape::Arm => vec![
            format!("if ({this} !== undefined) {{"),
            format!("  {}", put(&f.scalar, "out", f.number, &this)),
            "}".to_owned(),
        ],
        Shape::Repeated => vec![
            format!("for (const item of {this}) {{"),
            format!("  {}", put(&f.scalar, "out", f.number, "item")),
            "}".to_owned(),
        ],
        Shape::Map(value) => {
            let mut lines = vec![
                format!("for (const [key, item] of wire.sortedEntries({this})) {{"),
                "  const entry = new wire.Out();".to_owned(),
                "  if (key !== \"\") {".to_owned(),
                format!("    {}", put(&Scalar::Str, "entry", 1, "key")),
                "  }".to_owned(),
            ];
            if matches!(value, Scalar::Message(_)) {
                lines.push("  const payload = item.encode();".to_owned());
                lines.push("  if (payload.length > 0) {".to_owned());
                lines.push("    wire.putBytes(entry, 2, payload);".to_owned());
            } else {
                lines.push(format!("  if ({}) {{", is_set(value, "item")));
                lines.push(format!("    {}", put(value, "entry", 2, "item")));
            }
            lines.push("  }".to_owned());
            lines.push(format!(
                "  wire.putBytes(out, {}, entry.bytes());",
                f.number
            ));
            lines.push("}".to_owned());
            lines
        }
    }
}

fn encode_unit(unit: &Unit) -> Vec<String> {
    match unit {
        Unit::Field(f) => encode_field(f),
        Unit::Oneof { name, arms } => {
            // Asking which arm is set is also the check that only one is.
            let mut lines = vec![format!("this.which{}();", pascal(name))];
            for (i, arm) in arms.iter().enumerate() {
                let keyword = if i == 0 { "if" } else { "} else if" };
                lines.push(format!("{keyword} (this.{} !== undefined) {{", arm.attr));
                lines.push(format!(
                    "  {}",
                    put(
                        &arm.scalar,
                        "out",
                        arm.number,
                        &format!("this.{}", arm.attr)
                    )
                ));
            }
            lines.push("}".to_owned());
            lines
        }
    }
}

/// The statements that read one field into `this`; `oneof` names the oneof an arm belongs to.
fn merge_field(f: &Field, oneof: Option<&str>, current: &str) -> Vec<String> {
    let a = &f.attr;
    let mut lines = Vec::new();
    match &f.shape {
        Shape::Plain | Shape::Optional | Shape::Arm => {
            if matches!(f.scalar, Scalar::Message(_)) {
                // A second occurrence of a message merges into the one that is set.
                lines.push(format!("const current = this.{a};"));
                if let Some(oneof) = oneof {
                    lines.push(format!("this.clear{}();", pascal(oneof)));
                }
                lines.push(format!(
                    "const message = current ?? {};",
                    default_value(&f.scalar, current)
                ));
                lines.push("message.mergeFrom(wire.bytesOf(value));".to_owned());
                lines.push(format!("this.{a} = message;"));
            } else {
                if let Some(oneof) = oneof {
                    lines.push(format!("this.clear{}();", pascal(oneof)));
                }
                lines.push(format!("this.{a} = {};", get(&f.scalar, current, "value")));
            }
        }
        Shape::Repeated => lines.push(format!(
            "this.{a}.push({});",
            get(&f.scalar, current, "value")
        )),
        Shape::Map(value) => {
            lines.push("let key = \"\";".to_owned());
            lines.push(format!(
                "let item: {} = {};",
                ty(value, current),
                default_value(value, current)
            ));
            lines.push("for (const part of wire.readFields(wire.bytesOf(value))) {".to_owned());
            lines.push("  if (part.number === 1) {".to_owned());
            lines.push(format!(
                "    key = {};",
                get(&Scalar::Str, current, "part.value")
            ));
            lines.push("  } else if (part.number === 2) {".to_owned());
            lines.push(format!("    item = {};", get(value, current, "part.value")));
            lines.push("  }".to_owned());
            lines.push("}".to_owned());
            lines.push(format!("this.{a}.set(key, item);"));
        }
    }
    lines
}

/// The arm message's fields when a factory can take them as parameters: the message holds
/// plain scalars and nothing else.
fn flat_fields(arm: &Field) -> Option<Vec<Field>> {
    let Scalar::Message(m) = &arm.scalar else {
        return None;
    };
    // A message that cannot be read is refused when its own class is written, not here.
    units(m).ok().and_then(|units| {
        units
            .into_iter()
            .map(|unit| match unit {
                Unit::Field(f) if matches!(f.shape, Shape::Plain) => Some(f),
                _ => None,
            })
            .collect()
    })
}

/// One factory per arm of a oneof: `Image.digest("sha256:...")`. It takes the arm's value (or,
/// for a message of plain scalars, that message's fields) and the other fields of the message
/// as an object.
fn factories(arms: &[Field], current: &str, class: &str) -> Vec<String> {
    let mut out = Vec::new();
    for arm in arms {
        let (params, value): (Vec<String>, String) = match flat_fields(arm) {
            Some(fields) => {
                let t = ty(&arm.scalar, current);
                let args: Vec<String> = fields
                    .iter()
                    .map(|f| pair(&f.attr, &param(&f.attr)))
                    .collect();
                (
                    fields
                        .iter()
                        .map(|f| format!("{}: {}", param(&f.attr), ty(&f.scalar, current)))
                        .collect(),
                    if args.is_empty() {
                        format!("new {t}()")
                    } else {
                        format!("new {t}({{ {} }})", args.join(", "))
                    },
                )
            }
            None => (
                vec![format!(
                    "{}: {}",
                    param(&arm.attr),
                    ty(&arm.scalar, current)
                )],
                param(&arm.attr),
            ),
        };
        let mut signature = params;
        signature.push(format!("fields: {class}Init = {{}}"));
        out.push(String::new());
        out.push(format!(
            "  static {}({}): {class} {{",
            arm.attr,
            signature.join(", ")
        ));
        out.push(format!(
            "    return new {class}({{ ...fields, {} }});",
            pair(&arm.attr, &value)
        ));
        out.push("  }".to_owned());
    }
    out
}

/// The names the members of a class take, which must all be different.
fn check_names(message: &MessageDescriptor, units: &[Unit]) -> Result<(), Error> {
    let mut names = BTreeSet::new();
    let mut take = |name: String| {
        if names.insert(name.clone()) {
            Ok(())
        } else {
            Err(Error::refused(
                message.full_name(),
                format!("two members are called `{name}` in TypeScript"),
            ))
        }
    };
    for unit in units {
        match unit {
            Unit::Field(f) => take(f.attr.clone())?,
            Unit::Oneof { name, arms } => {
                take(format!("which{}", pascal(name)))?;
                take(format!("clear{}", pascal(name)))?;
                for arm in arms {
                    take(arm.attr.clone())?;
                    if STATIC_TAKEN.contains(&arm.attr.as_str()) {
                        return Err(Error::refused(
                            message.full_name(),
                            format!(
                                "the arm `{}` would be a static method the class constructor already has",
                                arm.attr
                            ),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

/// The comment of a field, as the lines of a doc comment at `pad`.
fn field_doc(f: &Field, pad: usize) -> Vec<String> {
    f.doc.as_deref().map(|d| doc(d, pad)).unwrap_or_default()
}

/// The constructor's object type and the class of `message`, as seen from the module of package
/// `current`.
pub fn class(message: &MessageDescriptor, current: &str) -> Result<Vec<String>, Error> {
    let units = units(message)?;
    check_names(message, &units)?;
    let fields = fields_of(&units);
    let name = local_name(message.full_name(), message.package_name());

    let mut out = vec![String::new()];
    out.push(format!("export interface {name}Init {{"));
    for f in &fields {
        out.extend(field_doc(f, 2));
        out.push(format!("  {}?: {};", f.attr, init_type(f, current)));
    }
    out.push("}".to_owned());
    out.push(String::new());
    if let Some(text) = docs::leading(message.parent_file_descriptor_proto(), message.path()) {
        out.extend(doc(&text, 0));
    }
    out.push(format!("export class {name} extends wire.Message {{"));
    for f in &fields {
        out.extend(field_doc(f, 2));
        out.push(format!("  {}: {};", f.attr, property_type(f, current)));
    }
    out.push(String::new());
    let init = if fields.is_empty() { "_init" } else { "init" };
    out.push(format!("  constructor({init}: {name}Init = {{}}) {{"));
    out.push("    super();".to_owned());
    for f in &fields {
        let a = &f.attr;
        out.push(match &f.shape {
            Shape::Plain => format!(
                "    this.{a} = init.{a} ?? {};",
                default_value(&f.scalar, current)
            ),
            Shape::Optional | Shape::Arm => format!("    this.{a} = init.{a};"),
            Shape::Repeated => format!("    this.{a} = Array.from(init.{a} ?? []);"),
            Shape::Map(_) => format!("    this.{a} = wire.toMap(init.{a});"),
        });
    }
    out.push("  }".to_owned());

    for unit in &units {
        if let Unit::Oneof { name: oneof, arms } = unit {
            let arm_names: Vec<String> = arms.iter().map(|a| a.attr.clone()).collect();
            let union: Vec<String> = arm_names.iter().map(|n| format!("\"{n}\"")).collect();
            out.push(String::new());
            out.push(format!(
                "  /** The name of the arm of `{oneof}` that is set, or undefined. */"
            ));
            out.push(format!(
                "  which{}(): {} | undefined {{",
                pascal(oneof),
                union.join(" | ")
            ));
            out.push(format!(
                "    return wire.which(this, [{}]);",
                quoted_list(&arm_names)
            ));
            out.push("  }".to_owned());
            out.push(String::new());
            out.push(format!("  private clear{}(): void {{", pascal(oneof)));
            out.extend(
                arms.iter()
                    .map(|a| format!("    this.{} = undefined;", a.attr)),
            );
            out.push("  }".to_owned());
            out.extend(factories(arms, current, &name));
        }
    }

    out.push(String::new());
    out.push("  protected encodeInto(out: wire.Out): void {".to_owned());
    if units.is_empty() {
        out.push("    void out;".to_owned());
    }
    for unit in &units {
        out.extend(indent(encode_unit(unit), 4));
    }
    out.push("  }".to_owned());
    out.push(String::new());
    out.push(
        "  /** Reads the fields of `data` into this message; unknown fields are dropped. */"
            .to_owned(),
    );
    out.push("  mergeFrom(data: Uint8Array): void {".to_owned());
    if fields.is_empty() {
        out.push("    wire.validate(data);".to_owned());
    } else {
        out.push("    for (const { number, value } of wire.readFields(data)) {".to_owned());
        out.push("      switch (number) {".to_owned());
        for f in &fields {
            out.push(format!("        case {}: {{", f.number));
            out.extend(indent(merge_field(f, oneof_of(&units, f), current), 10));
            out.push("          break;".to_owned());
            out.push("        }".to_owned());
        }
        out.push("        default:".to_owned());
        out.push("          break;".to_owned());
        out.push("      }".to_owned());
        out.push("    }".to_owned());
    }
    out.push("  }".to_owned());
    out.push(String::new());
    out.push("  /** The message the bytes hold; unknown fields are dropped. */".to_owned());
    out.push(format!("  static decode(data: Uint8Array): {name} {{"));
    out.push(format!("    const message = new {name}();"));
    out.push("    message.mergeFrom(data);".to_owned());
    out.push("    return message;".to_owned());
    out.push("  }".to_owned());
    out.push("}".to_owned());
    Ok(out)
}

/// The name of the oneof that has `arm`, if any.
fn oneof_of<'a>(units: &'a [Unit], arm: &Field) -> Option<&'a str> {
    units.iter().find_map(|u| match u {
        Unit::Oneof { name, arms } if arms.iter().any(|a| a.number == arm.number) => {
            Some(name.as_str())
        }
        _ => None,
    })
}

/// The constant object and the type of `e`.
pub fn enum_def(e: &EnumDescriptor) -> Vec<String> {
    let name = local_name(e.full_name(), e.package_name());
    let file = e.parent_file_descriptor_proto();
    let mut out = vec![String::new()];
    if let Some(text) = docs::leading(file, e.path()) {
        out.extend(doc(&text, 0));
    }
    out.push(format!("export const {name} = {{"));
    for value in e.values() {
        if let Some(text) = docs::leading(file, value.path()) {
            out.extend(doc(&text, 2));
        }
        out.push(format!(
            "  {}: {},",
            member_name(e, value.name()),
            value.number()
        ));
    }
    out.push("} as const;".to_owned());
    out.push(format!(
        "export type {name} = (typeof {name})[keyof typeof {name}];"
    ));
    out
}
