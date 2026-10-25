// SPDX-License-Identifier: Apache-2.0
//! The Python each field type becomes (the types themselves are read in `model`).

use prost_reflect::EnumDescriptor;

use crate::model::Scalar;
use crate::names::{enum_member, local_name, module_of};

impl Scalar {
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
