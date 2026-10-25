// SPDX-License-Identifier: Apache-2.0
//! The resource kinds, read from the descriptors: what each target's typed templates are built
//! from.
//!
//! The kinds are the arms of the `body` oneof of `Resource`; for each, the fields of the kind's
//! message (the `Image`, which nothing can run without, is required), each oneof of the kind,
//! the `portability` marking and the outputs the kind `exposes`. A new kind in `resource.proto`
//! is a new template in every target with no generator change. The shapes the templates know by
//! name (`Image`, `Value`, `Uses`, `Ref`) are the ones the planner knows too.

use prost_reflect::{
    DescriptorPool, EnumDescriptor, EnumValueDescriptor, ExtensionDescriptor, MessageDescriptor,
    OneofDescriptor, Value,
};

use crate::model::{Field, Scalar, Shape, Unit, units};
use crate::names::enum_member;
use crate::{Error, docs};

const RESOURCE: &str = "ucdk.resource.v1.Resource";
const IMAGE: &str = "ucdk.resource.v1.Image";
const VALUE: &str = "ucdk.resource.v1.Value";
const NEEDED: &str = "ucdk.resource.v1.Resource with a oneof `body`, the options `portability` and \
    `exposes`, and the enums Portability, Output and Access";

/// What the templates are built from, found in the descriptors.
pub struct Catalog {
    body: OneofDescriptor,
    portability: ExtensionDescriptor,
    exposes: ExtensionDescriptor,
    pub marking: EnumDescriptor,
    pub output: EnumDescriptor,
    pub access: EnumDescriptor,
}

/// The catalog of the protos in `pool`, or `None` when they declare no `Resource`.
pub fn find(pool: &DescriptorPool) -> Result<Option<Catalog>, Error> {
    let Some(resource) = pool.get_message_by_name(RESOURCE) else {
        return Ok(None);
    };
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
    Ok(Some(Catalog {
        body,
        portability,
        exposes,
        marking,
        output,
        access,
    }))
}

impl Catalog {
    /// What a bare `Ref` in `uses` is granted: the first access that is not "unset".
    pub fn granted(&self) -> EnumValueDescriptor {
        self.access
            .values()
            .find(|v| v.number() != 0)
            .unwrap_or_else(|| self.access.default_value())
    }

    /// The kinds: the name and message of each arm of `body` that is a message.
    pub fn arms(&self) -> Vec<(String, MessageDescriptor)> {
        self.body
            .fields()
            .filter_map(|arm| {
                arm.kind()
                    .as_message()
                    .map(|m| (arm.name().to_owned(), m.clone()))
            })
            .collect()
    }
}

/// The value of `e` numbered `number`, or its default for a number it does not have.
pub fn value_of(e: &EnumDescriptor, number: i32) -> EnumValueDescriptor {
    e.get_value(number).unwrap_or_else(|| e.default_value())
}

/// The name of a member of `e` in a target that drops the enum's own name from its values.
pub fn member_of(e: &EnumDescriptor, number: i32) -> String {
    enum_member(e.name(), value_of(e, number).name())
}

/// One oneof of a kind: its (target-named) name and its arms, each with the message it holds.
pub struct KindOneof {
    pub name: String,
    pub arms: Vec<(Field, MessageDescriptor)>,
}

/// One resource kind.
pub struct Kind {
    /// The arm of `Resource.body`: `service`.
    pub name: String,
    pub message: MessageDescriptor,
    /// The number of the `Portability` marking.
    pub marking: i32,
    /// The numbers of the outputs the kind exposes.
    pub exposed: Vec<i32>,
    /// A property per exposed output: its target-named name and the output's number.
    pub properties: Vec<(String, i32)>,
    /// The fields a template takes by position.
    pub required: Vec<Field>,
    /// The fields it takes as options.
    pub optional: Vec<Field>,
    pub oneofs: Vec<KindOneof>,
    /// The comment of the message.
    pub doc: String,
}

fn is_image(f: &Field) -> bool {
    matches!(&f.scalar, Scalar::Message(m) if m.full_name() == IMAGE)
        && matches!(f.shape, Shape::Optional)
}

/// Whether `f` is a map of `Value`s, which a template takes as literals and references too.
pub fn is_values(f: &Field) -> bool {
    matches!((&f.shape, &f.scalar), (Shape::Map(_), Scalar::Message(m)) if m.full_name() == VALUE)
}

/// The messages a oneof of a kind takes, one per arm, each different.
fn arm_messages(arms: &[Field]) -> Result<Vec<(Field, MessageDescriptor)>, Error> {
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
        out.push((arm.clone(), m.clone()));
    }
    Ok(out)
}

/// How a target names things and what it keeps for itself.
pub struct Naming {
    /// The name of a field, a oneof or an output property.
    pub name: fn(&str) -> String,
    /// Names a template keeps for itself; a field of a kind may not be called one.
    pub own: &'static [&'static str],
}

/// The kind `name` of `catalog`, held in `message`.
pub fn read(
    catalog: &Catalog,
    name: &str,
    message: &MessageDescriptor,
    naming: &Naming,
) -> Result<Kind, Error> {
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

    let mut required: Vec<Field> = Vec::new();
    let mut optional: Vec<Field> = Vec::new();
    let mut oneofs: Vec<(String, Vec<Field>)> = Vec::new();
    for unit in units(message, naming.name)? {
        match unit {
            Unit::Field(f) if is_image(&f) => required.push(f),
            Unit::Field(f) => optional.push(f),
            Unit::Oneof { name, arms } => oneofs.push(((naming.name)(&name), arms)),
        }
    }
    let properties: Vec<(String, i32)> = exposed
        .iter()
        .map(|n| {
            let member = member_of(&catalog.output, *n).to_lowercase();
            ((naming.name)(&member), *n)
        })
        .collect();
    let mut taken: Vec<&str> = required
        .iter()
        .chain(&optional)
        .map(|f| f.attr.as_str())
        .collect();
    taken.extend(oneofs.iter().map(|(n, _)| n.as_str()));
    taken.extend(properties.iter().map(|(n, _)| n.as_str()));
    if let Some(own) = naming.own.iter().find(|own| taken.contains(own)) {
        return Err(Error::refused(
            message.full_name(),
            format!("a field, oneof or output named `{own}` collides with the template's own"),
        ));
    }
    let mut checked = Vec::new();
    for (oneof, arms) in oneofs {
        checked.push(KindOneof {
            name: oneof,
            arms: arm_messages(&arms)?,
        });
    }
    let doc =
        docs::leading(message.parent_file_descriptor_proto(), message.path()).unwrap_or_default();
    Ok(Kind {
        name: name.to_owned(),
        message: message.clone(),
        marking,
        exposed,
        properties,
        required,
        optional,
        oneofs: checked,
        doc,
    })
}
