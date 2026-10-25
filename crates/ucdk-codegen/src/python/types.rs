// SPDX-License-Identifier: Apache-2.0
//! The field types the generator emits code for, and the Python each one becomes.
//!
//! The subset is the one the ucdk protos use: strings, booleans, 32 and 64 bit integers (not
//! `sint`, `fixed` or floating point), enums and messages. Anything else is refused by name when
//! the field is read, so a new proto feature is a decision in this file and not a field the SDK
//! quietly drops.

use prost_reflect::{EnumDescriptor, Kind, MessageDescriptor};

use crate::Error;
use crate::names::{enum_member, local_name, module_of};

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

    /// The Python type, written as seen from `current`, the package of the module it is in.
    pub fn annotation(&self, current: &str) -> String {
        match self {
            Self::Str => "str".to_owned(),
            Self::Bool => "bool".to_owned(),
            Self::Int32 | Self::Int64 | Self::Uint32 | Self::Uint64 => "int".to_owned(),
            Self::Enum(e) => type_ref(e.package_name(), e.full_name(), current),
            Self::Message(m) => type_ref(m.package_name(), m.full_name(), current),
        }
    }

    /// The value of a field of this type that was not written; a message is a new empty one.
    pub fn default_value(&self, current: &str) -> String {
        match self {
            Self::Str => "\"\"".to_owned(),
            Self::Bool => "False".to_owned(),
            Self::Int32 | Self::Int64 | Self::Uint32 | Self::Uint64 => "0".to_owned(),
            Self::Enum(e) => member_ref(e, e.default_value().name(), current),
            Self::Message(_) => format!("{}()", self.annotation(current)),
        }
    }

    /// The wire type of a field of this type.
    pub fn wire(&self) -> &'static str {
        match self {
            Self::Str | Self::Message(_) => "_wire.LEN",
            _ => "_wire.VARINT",
        }
    }

    /// The statement that appends field `number` holding `expr` to the `out` buffer.
    pub fn put(&self, out: &str, number: u32, expr: &str) -> String {
        let function = match self {
            Self::Str => "put_string",
            Self::Bool => "put_bool",
            Self::Message(_) => "put_message",
            _ => "put_varint_field",
        };
        format!("_wire.{function}({out}, {number}, {expr})")
    }

    /// The expression that reads `raw`, the value of a field, as this type.
    pub fn get(&self, current: &str, raw: &str) -> String {
        match self {
            Self::Str => format!("_wire.get_string({raw})"),
            Self::Bool => format!("_wire.get_bool({raw})"),
            Self::Int32 => format!("_wire.get_int32({raw})"),
            Self::Int64 => format!("_wire.get_int64({raw})"),
            Self::Uint32 => format!("_wire.get_uint32({raw})"),
            Self::Uint64 => format!("_wire.get_uint64({raw})"),
            Self::Enum(_) => format!("_wire.get_enum({}, {raw})", self.annotation(current)),
            Self::Message(_) => format!("{}.decode({raw})", self.annotation(current)),
        }
    }
}

/// A type named `full_name` of `package`, as written in a module of package `current`: bare in
/// its own module, qualified by the module it is imported from otherwise.
pub fn type_ref(package: &str, full_name: &str, current: &str) -> String {
    let name = local_name(full_name, package);
    if package == current {
        name
    } else {
        format!("{}.{name}", module_of(package))
    }
}

/// The Python name of the member of `e` called `value` in the proto.
pub fn member_name(e: &EnumDescriptor, value: &str) -> String {
    enum_member(e.name(), value)
}

/// `Enum.MEMBER`, as written in a module of package `current`.
pub fn member_ref(e: &EnumDescriptor, value: &str, current: &str) -> String {
    format!(
        "{}.{}",
        type_ref(e.package_name(), e.full_name(), current),
        member_name(e, value)
    )
}
