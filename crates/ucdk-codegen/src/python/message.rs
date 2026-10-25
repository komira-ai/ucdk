// SPDX-License-Identifier: Apache-2.0
//! One message as a Python class, and one enum as an `IntEnum`.
//!
//! A message class holds its fields as plain attributes, built by `__init__`, and encodes and
//! decodes itself with the functions of `_wire.py`. The bytes it writes are the ones the Rust
//! side (prost) writes, field for field:
//!
//! - a field is written in field-number order, except that a whole oneof sits where its lowest
//!   arm number is (prost sorts its fields by their lowest tag, a oneof counting as one field);
//! - a singular scalar is left out when it is the default, an `optional` one and a message when
//!   it is `None`, and an arm of a oneof whenever it is set, default or not;
//! - a map is written in key order, and an entry leaves out a key or a value that is the default.

use prost_reflect::{EnumDescriptor, FieldDescriptor, Kind, MessageDescriptor};

use super::types::{Scalar, member_name};
use crate::names::{docstring, ident, local_name};
use crate::{Error, docs};

/// How a field is held.
#[derive(Clone)]
pub enum Shape {
    /// A scalar with no presence: the default means "not written".
    Plain,
    /// An `optional` scalar, or a message: `None` means "not written".
    Optional,
    /// An arm of a oneof: `None` means "not this arm".
    Arm,
    /// A `repeated` string or message.
    Repeated,
    /// A `map` with string keys and values of this type.
    Map(Scalar),
}

#[derive(Clone)]
pub struct Field {
    /// The attribute name.
    pub attr: String,
    pub number: u32,
    pub scalar: Scalar,
    pub shape: Shape,
    pub doc: Option<String>,
}

pub enum Unit {
    Field(Field),
    /// A oneof, with its arms in declaration order.
    Oneof {
        name: String,
        arms: Vec<Field>,
    },
}

impl Unit {
    /// Where the unit is written: the lowest field number it holds.
    fn order(&self) -> u32 {
        match self {
            Self::Field(f) => f.number,
            Self::Oneof { arms, .. } => arms.iter().map(|a| a.number).min().unwrap_or(0),
        }
    }
}

fn field_doc(field: &FieldDescriptor) -> Option<String> {
    docs::leading(
        field.parent_message().parent_file_descriptor_proto(),
        field.path(),
    )
}

fn read_field(field: &FieldDescriptor, arm: bool) -> Result<Field, Error> {
    let subject = field.full_name();
    let kind = field.kind();
    let (scalar, shape) = if let Kind::Message(entry) = &kind
        && entry.is_map_entry()
    {
        let key = Scalar::of(&entry.map_entry_key_field().kind(), subject)?;
        if !matches!(key, Scalar::Str) {
            return Err(Error::refused(subject, "a map key must be a string"));
        }
        let value = Scalar::of(&entry.map_entry_value_field().kind(), subject)?;
        (value.clone(), Shape::Map(value))
    } else {
        let shape = if arm {
            Shape::Arm
        } else if field.is_list() {
            Shape::Repeated
        } else if field.supports_presence() {
            Shape::Optional
        } else {
            Shape::Plain
        };
        (Scalar::of(&kind, subject)?, shape)
    };
    if matches!(shape, Shape::Repeated) && !matches!(scalar, Scalar::Str | Scalar::Message(_)) {
        return Err(Error::refused(
            subject,
            "a repeated field must hold strings or messages (packed scalars are not implemented)",
        ));
    }
    Ok(Field {
        attr: ident(field.name()),
        number: field.number(),
        scalar,
        shape,
        doc: field_doc(field),
    })
}

/// The fields of `message` as units, in the order they are written.
pub fn units(message: &MessageDescriptor) -> Result<Vec<Unit>, Error> {
    let mut out: Vec<Unit> = Vec::new();
    for field in message.fields() {
        match field.containing_oneof().filter(|o| !o.is_synthetic()) {
            None => out.push(Unit::Field(read_field(&field, false)?)),
            Some(oneof) => {
                let arm = read_field(&field, true)?;
                let existing = out.iter_mut().find_map(|u| match u {
                    Unit::Oneof { name, arms } if name == oneof.name() => Some(arms),
                    _ => None,
                });
                match existing {
                    Some(arms) => arms.push(arm),
                    None => out.push(Unit::Oneof {
                        name: oneof.name().to_owned(),
                        arms: vec![arm],
                    }),
                }
            }
        }
    }
    out.sort_by_key(Unit::order);
    Ok(out)
}

fn fields_of(units: &[Unit]) -> Vec<&Field> {
    let mut all: Vec<&Field> = Vec::new();
    for unit in units {
        match unit {
            Unit::Field(f) => all.push(f),
            Unit::Oneof { arms, .. } => all.extend(arms),
        }
    }
    all.sort_by_key(|f| f.number);
    all
}

/// `name: type = default`, a parameter that takes the field.
pub fn param(f: &Field, current: &str) -> String {
    let ann = f.scalar.annotation(current);
    match &f.shape {
        Shape::Plain => format!("{}: {ann} = {}", f.attr, f.scalar.default_value(current)),
        Shape::Optional | Shape::Arm => format!("{}: Optional[{ann}] = None", f.attr),
        Shape::Repeated => format!("{}: Iterable[{ann}] = ()", f.attr),
        Shape::Map(_) => format!("{}: Optional[Mapping[str, {ann}]] = None", f.attr),
    }
}

fn init(units: &[Unit], current: &str) -> Vec<String> {
    let fields = fields_of(units);
    let mut out = vec!["    def __init__(".to_owned(), "        self,".to_owned()];
    out.extend(
        fields
            .iter()
            .map(|f| format!("        {},", param(f, current))),
    );
    out.push("    ) -> None:".to_owned());
    for f in &fields {
        let a = &f.attr;
        out.push(match f.shape {
            Shape::Plain | Shape::Optional | Shape::Arm => format!("        self.{a} = {a}"),
            Shape::Repeated => format!("        self.{a} = list({a})"),
            Shape::Map(_) => format!("        self.{a} = {{}} if {a} is None else dict({a})"),
        });
    }
    if fields.is_empty() {
        out.push("        pass".to_owned());
    }
    out
}

fn put_field(f: &Field, out: &str, expr: &str) -> String {
    f.scalar.put(out, f.number, expr)
}

fn encode_field(f: &Field) -> Vec<String> {
    let a = &f.attr;
    match &f.shape {
        Shape::Plain => vec![
            format!("if self.{a}:"),
            format!("    {}", put_field(f, "out", &format!("self.{a}"))),
        ],
        Shape::Optional | Shape::Arm => vec![
            format!("if self.{a} is not None:"),
            format!("    {}", put_field(f, "out", &format!("self.{a}"))),
        ],
        Shape::Repeated => vec![
            format!("for item in self.{a}:"),
            format!("    {}", put_field(f, "out", "item")),
        ],
        Shape::Map(value) => {
            let mut lines = vec![
                format!("for key in sorted(self.{a}):"),
                format!("    item = self.{a}[key]"),
                "    entry = bytearray()".to_owned(),
                "    if key:".to_owned(),
                format!("        {}", Scalar::Str.put("entry", 1, "key")),
            ];
            if matches!(value, Scalar::Message(_)) {
                lines.push("    payload = item.encode()".to_owned());
                lines.push("    if payload:".to_owned());
                lines.push("        _wire.put_bytes(entry, 2, payload)".to_owned());
            } else {
                lines.push("    if item:".to_owned());
                lines.push(format!("        {}", value.put("entry", 2, "item")));
            }
            lines.push(format!(
                "    _wire.put_bytes(out, {}, bytes(entry))",
                f.number
            ));
            lines
        }
    }
}

fn encode_unit(unit: &Unit) -> Vec<String> {
    match unit {
        Unit::Field(f) => encode_field(f),
        Unit::Oneof { name, arms } => {
            let mut lines = vec![format!("arm = self.which_{name}()")];
            for (i, arm) in arms.iter().enumerate() {
                let keyword = if i == 0 { "if" } else { "elif" };
                lines.push(format!("{keyword} arm == \"{}\":", arm.attr));
                lines.push(format!(
                    "    {}",
                    put_field(arm, "out", &format!("self.{}", arm.attr))
                ));
            }
            lines
        }
    }
}

/// The statements that read one field into `self`; `oneof` names the oneof an arm belongs to.
fn merge_field(f: &Field, oneof: Option<&str>, current: &str) -> Vec<String> {
    let a = &f.attr;
    // A map entry is a length-delimited message, whatever its value is.
    let wire = match f.shape {
        Shape::Map(_) => "_wire.LEN",
        _ => f.scalar.wire(),
    };
    let mut lines = vec![format!("_wire.expect(wire_type, {wire})")];
    match &f.shape {
        Shape::Plain | Shape::Optional | Shape::Arm => {
            let message = matches!(f.scalar, Scalar::Message(_));
            if let Some(oneof) = oneof {
                if message {
                    // A second occurrence of the arm that is set merges into it.
                    lines.push(format!("current = self.{a}"));
                }
                lines.push(format!("self._clear_{oneof}()"));
            }
            if message {
                let ty = f.scalar.annotation(current);
                let start = if oneof.is_some() {
                    "current"
                } else {
                    &format!("self.{a}")
                };
                lines.push(format!("if {start} is None:"));
                lines.push(format!("    {start} = {ty}()"));
                lines.push(format!("self.{a} = {start}"));
                lines.push(format!("self.{a}._merge(value)"));
            } else {
                lines.push(format!("self.{a} = {}", f.scalar.get(current, "value")));
            }
        }
        Shape::Repeated => lines.push(format!(
            "self.{a}.append({})",
            f.scalar.get(current, "value")
        )),
        Shape::Map(value) => {
            let item = value.default_value(current);
            lines.push(format!("key, item = \"\", {item}"));
            lines.push("for number_, wire_type_, value_ in _wire.read_fields(value):".to_owned());
            lines.push("    if number_ == 1:".to_owned());
            lines.push("        _wire.expect(wire_type_, _wire.LEN)".to_owned());
            lines.push(format!(
                "        key = {}",
                Scalar::Str.get(current, "value_")
            ));
            lines.push("    elif number_ == 2:".to_owned());
            lines.push(format!(
                "        _wire.expect(wire_type_, {})",
                value.wire()
            ));
            lines.push(format!("        item = {}", value.get(current, "value_")));
            lines.push(format!("self.{a}[key] = item"));
        }
    }
    lines
}

fn indent(lines: Vec<String>, by: usize) -> impl Iterator<Item = String> {
    let pad = " ".repeat(by);
    lines.into_iter().map(move |l| format!("{pad}{l}"))
}

/// The arm message's fields when a constructor can take them as parameters: the message holds
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

/// One constructor per arm of a oneof: `Image.digest("sha256:...")`. It takes the arm's value
/// (or, for a message of plain scalars, that message's fields) and passes any other field of
/// the message on as a keyword argument.
fn constructors(arms: &[Field], current: &str, class: &str) -> Vec<String> {
    let mut out = Vec::new();
    for arm in arms {
        let (params, value): (Vec<String>, String) = match flat_fields(arm) {
            Some(fields) => {
                let ty = arm.scalar.annotation(current);
                let args: Vec<String> = fields.iter().map(|f| format!("{0}={0}", f.attr)).collect();
                (
                    fields
                        .iter()
                        .map(|f| format!("{}: {}", f.attr, f.scalar.annotation(current)))
                        .collect(),
                    format!("{ty}({})", args.join(", ")),
                )
            }
            None => (
                vec![format!("{}: {}", arm.attr, arm.scalar.annotation(current))],
                arm.attr.clone(),
            ),
        };
        let mut signature = vec!["cls".to_owned()];
        signature.extend(params);
        signature.push("**fields: Any".to_owned());
        out.push(String::new());
        out.push("    @classmethod".to_owned());
        out.push(format!(
            "    def {}({}) -> \"{class}\":",
            arm.attr,
            signature.join(", ")
        ));
        out.push(format!(
            "        return cls({}={value}, **fields)",
            arm.attr
        ));
    }
    out
}

/// A tuple of strings, one to a line.
fn quoted_tuple(names: &[String], indent: usize) -> Vec<String> {
    let pad = " ".repeat(indent);
    let mut out = vec!["(".to_owned()];
    out.extend(names.iter().map(|n| format!("{pad}    \"{n}\",")));
    out.push(format!("{pad})"));
    out
}

/// The docstring of a message: its comment, then the comment of each field.
fn class_doc(message: &MessageDescriptor, fields: &[&Field]) -> Vec<String> {
    let mut text =
        docs::leading(message.parent_file_descriptor_proto(), message.path()).unwrap_or_default();
    let documented: Vec<&&Field> = fields.iter().filter(|f| f.doc.is_some()).collect();
    if !documented.is_empty() {
        text.push_str("\n\nAttributes:");
        for f in documented {
            let body = f
                .doc
                .as_deref()
                .unwrap_or_default()
                .replace('\n', "\n        ");
            text.push_str(&format!("\n    {}: {body}", f.attr));
        }
    }
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = vec!["    \"\"\"".to_owned()];
    let body = docstring(text.trim(), 4);
    out.extend(body);
    out.push("    \"\"\"".to_owned());
    out.push(String::new());
    out
}

/// The class of `message`, as seen from the module of package `current`.
pub fn class(message: &MessageDescriptor, current: &str) -> Result<Vec<String>, Error> {
    let units = units(message)?;
    let fields = fields_of(&units);
    let name = local_name(message.full_name(), message.package_name());
    let attrs: Vec<String> = fields.iter().map(|f| f.attr.clone()).collect();
    let mut out = vec![
        String::new(),
        String::new(),
        format!("class {name}(_wire.Message):"),
    ];
    out.extend(class_doc(message, &fields));
    out.push(format!(
        "    _FIELDS = {}",
        quoted_tuple(&attrs, 4).join("\n")
    ));
    out.push(String::new());
    out.extend(init(&units, current));
    for unit in &units {
        if let Unit::Oneof { name: oneof, arms } = unit {
            let arm_names: Vec<String> = arms.iter().map(|a| a.attr.clone()).collect();
            let tuple = quoted_tuple(&arm_names, 8).join("\n");
            out.push(String::new());
            out.push(format!("    def which_{oneof}(self) -> Optional[str]:"));
            out.push(format!(
                "        \"\"\"The name of the arm of `{oneof}` that is set, or None.\"\"\""
            ));
            out.push(format!("        return _wire.which(self, {tuple})"));
            out.push(String::new());
            out.push(format!("    def _clear_{oneof}(self) -> None:"));
            out.extend(
                arms.iter()
                    .map(|a| format!("        self.{} = None", a.attr)),
            );
            out.extend(constructors(arms, current, &name));
        }
    }
    out.push(String::new());
    out.push("    def _encode_into(self, out: bytearray) -> None:".to_owned());
    if units.is_empty() {
        out.push("        return".to_owned());
    }
    for unit in &units {
        out.extend(indent(encode_unit(unit), 8));
    }
    out.push(String::new());
    out.push("    def _merge(self, data: bytes) -> None:".to_owned());
    out.push("        for number, wire_type, value in _wire.read_fields(data):".to_owned());
    if fields.is_empty() {
        out.push("            pass".to_owned());
    }
    for (i, f) in fields.iter().enumerate() {
        let keyword = if i == 0 { "if" } else { "elif" };
        out.push(format!("            {keyword} number == {}:", f.number));
        out.extend(indent(merge_field(f, oneof_of(&units, f), current), 16));
    }
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

/// The class of `e`.
pub fn enum_class(e: &EnumDescriptor) -> Vec<String> {
    let name = local_name(e.full_name(), e.package_name());
    let file = e.parent_file_descriptor_proto();
    let mut out = vec![
        String::new(),
        String::new(),
        format!("class {name}(enum.IntEnum):"),
    ];
    if let Some(doc) = docs::leading(file, e.path()) {
        out.push("    \"\"\"".to_owned());
        out.extend(docstring(&doc, 4));
        out.push("    \"\"\"".to_owned());
        out.push(String::new());
    }
    for value in e.values() {
        if let Some(doc) = docs::leading(file, value.path()) {
            out.extend(
                doc.lines()
                    .map(|l| format!("    # {l}").trim_end().to_owned()),
            );
        }
        out.push(format!(
            "    {} = {}",
            member_name(e, value.name()),
            value.number()
        ));
    }
    out
}
