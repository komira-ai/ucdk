// SPDX-License-Identifier: Apache-2.0
//! What every target reads from the descriptors: the messages and enums of the proto packages,
//! the type of each field, and the order and shape in which a message is written.
//!
//! The subset is the one the ucdk protos use: strings, booleans, 32 and 64 bit integers (not
//! `sint`, `fixed` or floating point), enums and messages. Anything else is refused by name when
//! the field is read, so a new proto feature is a decision here and not a field an SDK quietly
//! drops. The names a target gives the fields are the target's (`units` takes the function).
//!
//! The bytes a target writes are the ones the Rust side (prost) writes, field for field:
//!
//! - a field is written in field-number order, except that a whole oneof sits where its lowest
//!   arm number is (prost sorts its fields by their lowest tag, a oneof counting as one field);
//! - a singular scalar is left out when it is the default, an `optional` one and a message when
//!   it is absent, and an arm of a oneof whenever it is set, default or not;
//! - a map is written in key order, and an entry leaves out a key or a value that is the default.

use std::collections::{BTreeMap, BTreeSet};

use prost_reflect::{DescriptorPool, EnumDescriptor, FieldDescriptor, Kind, MessageDescriptor};

use crate::names::module_of;
use crate::{Error, docs};

/// A type a field, a oneof arm or a map value can have.
#[derive(Clone)]
pub enum Scalar {
    Str,
    Bool,
    Int32,
    Int64,
    Uint32,
    Uint64,
    Enum(EnumDescriptor),
    Message(MessageDescriptor),
}

impl Scalar {
    /// The type of a field of kind `kind`; `subject` names the field in the refusal.
    pub fn of(kind: &Kind, subject: &str) -> Result<Self, Error> {
        Ok(match kind {
            Kind::String => Self::Str,
            Kind::Bool => Self::Bool,
            Kind::Int32 => Self::Int32,
            Kind::Int64 => Self::Int64,
            Kind::Uint32 => Self::Uint32,
            Kind::Uint64 => Self::Uint64,
            Kind::Enum(e) => Self::Enum(e.clone()),
            Kind::Message(m) => Self::Message(m.clone()),
            other => {
                return Err(Error::refused(
                    subject,
                    format!("the type {other:?} is not supported by the generator"),
                ));
            }
        })
    }
}

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
    /// The name in the target language, as the target's naming function made it.
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

fn read_field(
    field: &FieldDescriptor,
    arm: bool,
    name: fn(&str) -> String,
) -> Result<Field, Error> {
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
        attr: name(field.name()),
        number: field.number(),
        scalar,
        shape,
        doc: field_doc(field),
    })
}

/// The fields of `message` as units, in the order they are written.
pub fn units(message: &MessageDescriptor, name: fn(&str) -> String) -> Result<Vec<Unit>, Error> {
    let mut out: Vec<Unit> = Vec::new();
    for field in message.fields() {
        match field.containing_oneof().filter(|o| !o.is_synthetic()) {
            None => out.push(Unit::Field(read_field(&field, false, name)?)),
            Some(oneof) => {
                let arm = read_field(&field, true, name)?;
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

pub fn fields_of(units: &[Unit]) -> Vec<&Field> {
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

/// The messages and enums of one proto package.
#[derive(Default)]
pub struct Module {
    pub messages: Vec<MessageDescriptor>,
    pub enums: Vec<EnumDescriptor>,
}

#[derive(Default)]
struct Collected {
    seen: BTreeSet<String>,
    modules: BTreeMap<String, Module>,
    /// Every message in the order it was found, to follow its fields.
    queue: Vec<MessageDescriptor>,
}

impl Collected {
    fn enum_(&mut self, e: &EnumDescriptor) {
        if self.seen.insert(e.full_name().to_owned()) {
            let module = self.modules.entry(e.package_name().to_owned()).or_default();
            module.enums.push(e.clone());
        }
    }

    /// Adds a message, its nested messages and its nested enums; a map entry is not a message of
    /// its own.
    fn message(&mut self, m: &MessageDescriptor) {
        if m.is_map_entry() || !self.seen.insert(m.full_name().to_owned()) {
            return;
        }
        let module = self.modules.entry(m.package_name().to_owned()).or_default();
        module.messages.push(m.clone());
        self.queue.push(m.clone());
        for e in m.child_enums() {
            self.enum_(&e);
        }
        for child in m.child_messages() {
            self.message(&child);
        }
    }

    /// Adds what a field refers to.
    fn reference(&mut self, kind: &Kind) {
        match kind {
            Kind::Message(m) if m.is_map_entry() => {
                self.reference(&m.map_entry_value_field().kind());
            }
            Kind::Message(m) => self.message(m),
            Kind::Enum(e) => self.enum_(e),
            _ => {}
        }
    }
}

/// The messages of `files`, then whatever else they refer to, grouped by package.
pub fn collect(pool: &DescriptorPool, files: &[&str]) -> Result<BTreeMap<String, Module>, Error> {
    let mut found = Collected::default();
    for name in files {
        let Some(file) = pool.get_file_by_name(name) else {
            return Err(Error::Missing((*name).to_owned()));
        };
        for e in file.enums() {
            found.enum_(&e);
        }
        for m in file.messages() {
            found.message(&m);
        }
    }
    let mut at = 0;
    while at < found.queue.len() {
        let message = found.queue[at].clone();
        at += 1;
        for field in message.fields() {
            found.reference(&field.kind());
        }
    }
    Ok(found.modules)
}

/// The packages whose types the messages of `module` refer to.
pub fn imports(package: &str, module: &Module) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for field in module.messages.iter().flat_map(MessageDescriptor::fields) {
        let kind = field.kind();
        let kind = match &kind {
            Kind::Message(m) if m.is_map_entry() => m.map_entry_value_field().kind(),
            _ => kind,
        };
        let other = match &kind {
            Kind::Message(m) => m.package_name(),
            Kind::Enum(e) => e.package_name(),
            _ => package,
        };
        if other != package {
            out.insert(module_of(other));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use protox::Compiler;

    use super::*;

    const USES: &str = "example/uses/v1/uses.proto";

    fn uses_pool() -> DescriptorPool {
        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/proto");
        let mut compiler = Compiler::new([fixtures]).expect("a compiler");
        compiler.include_imports(true);
        compiler.open_file(USES).expect("the fixture compiles");
        compiler.descriptor_pool()
    }

    #[test]
    fn an_import_adds_the_other_module() {
        let modules = collect(&uses_pool(), &[USES]).expect("collects");
        let packages: Vec<&str> = modules.keys().map(String::as_str).collect();
        assert_eq!(packages, ["example.other.v1", "example.uses.v1"]);
        // The package that is used holds the message and the enum; it imports nothing.
        let other = &modules["example.other.v1"];
        assert_eq!(other.messages.len(), 1);
        assert_eq!(other.enums.len(), 1);
        assert!(imports("example.other.v1", other).is_empty());
        // The package that uses it imports it once, however many fields refer to it.
        let uses = &modules["example.uses.v1"];
        assert_eq!(
            imports("example.uses.v1", uses),
            BTreeSet::from(["example_other_v1".to_owned()])
        );
    }
}
