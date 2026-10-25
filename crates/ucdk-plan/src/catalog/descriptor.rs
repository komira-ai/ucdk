// SPDX-License-Identifier: Apache-2.0
//! The catalog read from its descriptors: which resource types there are and how each lowers.
//!
//! A resource list is a message with one repeated field of resources; a resource has an `id`, a
//! list of `uses` and a `body` oneof whose arms are the types. Everything about a type comes
//! from its message and the options on it (`portability`, `exposes`, and the field options
//! `node_name`, `render`, `fallback`, `role`, `unlowered`; see `resource.proto`). Adding a type
//! is adding an arm and a message with those options; nothing here or in `read` names a type.

use std::error::Error;
use std::fmt;
use std::sync::OnceLock;

use prost_reflect::{
    DescriptorPool, DynamicMessage, ExtensionDescriptor, FieldDescriptor, Kind as PbKind,
    MessageDescriptor, OneofDescriptor, Value as Pb,
};
use ucdk_proto::DESCRIPTOR_SET;
use ucdk_proto::v1::{Output, Portability};
use ucdk_provider::FieldValue;

/// The package whose shared shapes (`Image`, `Value`, `SecretRef`, `Uses`) the catalog knows.
const PACKAGE: &str = "ucdk.resource.v1";
const LIST: &str = "ucdk.resource.v1.ResourceList";

/// Why a message is not a usable catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// The message does not have the shape of a resource list.
    NotAList { message: String, why: &'static str },
    /// The descriptor pool does not define one of the catalog's options.
    MissingOption { name: &'static str },
    /// A type has no `portability` option, or `PORTABILITY_UNSET`.
    Unmarked { kind: String },
    /// A field of a type cannot be lowered.
    BadField {
        kind: String,
        field: String,
        why: &'static str,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAList { message, why } => {
                write!(f, "`{message}` is not a resource list: {why}")
            }
            Self::MissingOption { name } => write!(f, "the option `{name}` is not defined"),
            Self::Unmarked { kind } => write!(f, "type `{kind}` has no portability marking"),
            Self::BadField { kind, field, why } => {
                write!(f, "type `{kind}`, field `{field}`: {why}")
            }
        }
    }
}

impl Error for CatalogError {}

/// How one field of a type lowers.
pub enum Shape {
    /// An `Image`: checked, one text field.
    Image,
    /// `repeated string`: one text field per element.
    List,
    /// `map<string, Value>`: one text field (or output reference) per key.
    Env,
    /// `map<string, SecretRef>`: one text field per key.
    Secrets,
    /// A `string` or integer: one field.
    Scalar,
    /// A message: one text field built from a template.
    Render(Vec<Piece>),
}

/// A part of a `render` template.
pub enum Piece {
    Text(String),
    Field(FieldDescriptor),
}

pub struct FieldPlan {
    pub field: FieldDescriptor,
    /// The node field name, or its prefix.
    pub stem: String,
    pub shape: Shape,
    /// The value when the field is not written.
    pub fallback: Option<FieldValue>,
}

/// A second node of a type: wanted when `arm` is written.
pub struct RolePlan {
    pub role: String,
    pub arm: FieldDescriptor,
    /// The scalar fields of the arm's message; they become the node's fields.
    pub fields: Vec<FieldDescriptor>,
}

pub struct KindPlan {
    pub name: String,
    pub arm: FieldDescriptor,
    pub portability: Portability,
    pub exposes: Vec<Output>,
    pub fields: Vec<FieldPlan>,
    pub roles: Vec<RolePlan>,
}

/// The options the catalog reads, found in the pool.
struct Options {
    portability: ExtensionDescriptor,
    exposes: ExtensionDescriptor,
    node_name: ExtensionDescriptor,
    render: ExtensionDescriptor,
    fallback: ExtensionDescriptor,
    role: ExtensionDescriptor,
    unlowered: ExtensionDescriptor,
}

impl Options {
    fn find(pool: &DescriptorPool) -> Result<Self, CatalogError> {
        let get = |name: &'static str| {
            pool.get_extension_by_name(&format!("{PACKAGE}.{name}"))
                .ok_or(CatalogError::MissingOption { name })
        };
        Ok(Self {
            portability: get("portability")?,
            exposes: get("exposes")?,
            node_name: get("node_name")?,
            render: get("render")?,
            fallback: get("fallback")?,
            role: get("role")?,
            unlowered: get("unlowered")?,
        })
    }
}

/// The resource types of one resource list message, and how each lowers.
pub struct Catalog {
    pub(super) list: prost_reflect::MessageDescriptor,
    pub(super) resources: FieldDescriptor,
    pub(super) id: FieldDescriptor,
    pub(super) uses: FieldDescriptor,
    pub(super) kinds: Vec<KindPlan>,
}

fn not_a_list(list: &MessageDescriptor, why: &'static str) -> CatalogError {
    CatalogError::NotAList {
        message: list.full_name().to_owned(),
        why,
    }
}

fn is_scalar(field: &FieldDescriptor) -> bool {
    !field.is_list()
        && matches!(
            field.kind(),
            PbKind::String | PbKind::Uint32 | PbKind::Int32 | PbKind::Int64
        )
}

fn message_named(kind: &PbKind, name: &str) -> bool {
    matches!(kind, PbKind::Message(m) if m.full_name() == format!("{PACKAGE}.{name}"))
}

fn text_option(options: &DynamicMessage, ext: &ExtensionDescriptor) -> Option<String> {
    options
        .has_extension(ext)
        .then(|| options.get_extension(ext).as_str().map(str::to_owned))
        .flatten()
}

fn int_of(value: &Pb) -> i64 {
    value
        .as_u32()
        .map(i64::from)
        .or(value.as_i32().map(i64::from))
        .or(value.as_i64())
        .unwrap_or_default()
}

/// A scalar as a node field value: text for a string, else an integer.
pub fn scalar(value: &Pb) -> FieldValue {
    value.as_str().map_or_else(
        || FieldValue::Int(int_of(value)),
        |text| FieldValue::Text(text.to_owned()),
    )
}

/// A scalar as the text a template puts in its place.
pub fn scalar_text(value: &Pb) -> String {
    value
        .as_str()
        .map_or_else(|| int_of(value).to_string(), str::to_owned)
}

impl Catalog {
    /// Reads the catalog of a resource list message from its descriptor.
    pub fn new(list: MessageDescriptor) -> Result<Self, CatalogError> {
        let options = Options::find(list.parent_pool())?;
        let repeated: Vec<(FieldDescriptor, MessageDescriptor)> = list
            .fields()
            .filter_map(|f| match f.kind() {
                PbKind::Message(m) if f.is_list() => Some((f, m)),
                _ => None,
            })
            .collect();
        let [(resources, resource)] = repeated.as_slice() else {
            return Err(not_a_list(
                &list,
                "it needs exactly one repeated message field",
            ));
        };
        let resources = resources.clone();
        let id = resource
            .get_field_by_name("id")
            .filter(|f| matches!(f.kind(), PbKind::String) && !f.is_list())
            .ok_or_else(|| not_a_list(&list, "a resource needs a string `id`"))?;
        let uses = resource
            .get_field_by_name("uses")
            .filter(|f| f.is_list() && message_named(&f.kind(), "Uses"))
            .ok_or_else(|| not_a_list(&list, "a resource needs `repeated Uses uses`"))?;
        let body: OneofDescriptor = resource
            .oneofs()
            .find(|o| o.name() == "body")
            .ok_or_else(|| not_a_list(&list, "a resource needs a `body` oneof"))?;
        let kinds = body
            .fields()
            .map(|arm| plan_kind(arm, &options))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            list,
            resources,
            id,
            uses,
            kinds,
        })
    }

    /// The catalog of `ucdk.resource.v1`, read once.
    pub fn builtin() -> &'static Self {
        static BUILTIN: OnceLock<Catalog> = OnceLock::new();
        BUILTIN.get_or_init(|| {
            let pool = DescriptorPool::decode(DESCRIPTOR_SET)
                .expect("the descriptor set is built from resource.proto");
            let list = pool
                .get_message_by_name(LIST)
                .expect("resource.proto declares the list");
            Self::new(list).expect("the built-in catalog is well formed")
        })
    }

    /// An empty instance of the resource list message, for a caller that builds a list by
    /// reflection.
    pub fn new_list(&self) -> DynamicMessage {
        DynamicMessage::new(self.list.clone())
    }

    /// The portability the descriptor declares for the type named `name`; `None` if there is no
    /// such type.
    pub fn portability(&self, name: &str) -> Option<Portability> {
        self.kinds
            .iter()
            .find(|k| k.name == name)
            .map(|k| k.portability)
    }

    /// The names of the types, in declaration order.
    pub fn kind_names(&self) -> Vec<&str> {
        self.kinds.iter().map(|k| k.name.as_str()).collect()
    }
}

fn plan_kind(arm: FieldDescriptor, options: &Options) -> Result<KindPlan, CatalogError> {
    let name = arm.name().to_owned();
    let PbKind::Message(message) = arm.kind() else {
        return Err(CatalogError::BadField {
            kind: name,
            field: arm.name().to_owned(),
            why: "a body arm must be a message",
        });
    };
    let marked = message.options();
    let portability = Portability::try_from(
        marked
            .get_extension(&options.portability)
            .as_enum_number()
            .unwrap_or(0),
    )
    .ok()
    .filter(|p| *p != Portability::Unset)
    .ok_or_else(|| CatalogError::Unmarked { kind: name.clone() })?;
    let exposes = marked
        .get_extension(&options.exposes)
        .as_list()
        .unwrap_or_default()
        .iter()
        .flat_map(|v| v.as_enum_number().and_then(|n| Output::try_from(n).ok()))
        .collect();
    let mut fields = Vec::new();
    let mut roles = Vec::new();
    for field in message.fields() {
        let bad = |why| CatalogError::BadField {
            kind: name.clone(),
            field: field.name().to_owned(),
            why,
        };
        let own = field.options();
        if own.has_extension(&options.unlowered) {
            continue;
        }
        if let Some(role) = text_option(&own, &options.role) {
            let PbKind::Message(inner) = field.kind() else {
                return Err(bad("a role must be on a message field"));
            };
            let scalars: Vec<FieldDescriptor> = inner.fields().collect();
            if !scalars.iter().all(is_scalar) {
                return Err(bad("a role's message may hold only scalar fields"));
            }
            roles.push(RolePlan {
                role,
                arm: field.clone(),
                fields: scalars,
            });
            continue;
        }
        fields.push(plan_field(&field, &own, options).map_err(bad)?);
    }
    Ok(KindPlan {
        name,
        arm,
        portability,
        exposes,
        fields,
        roles,
    })
}

fn plan_field(
    field: &FieldDescriptor,
    own: &DynamicMessage,
    options: &Options,
) -> Result<FieldPlan, &'static str> {
    let stem = text_option(own, &options.node_name).unwrap_or_else(|| field.name().to_owned());
    let template = text_option(own, &options.render);
    let raw = text_option(own, &options.fallback);
    let shape = if field.is_map() {
        let entry = field.kind().as_message().cloned();
        let keyed_by_string = entry
            .as_ref()
            .is_some_and(|e| matches!(e.map_entry_key_field().kind(), PbKind::String));
        let holds = |name| {
            entry
                .as_ref()
                .is_some_and(|e| message_named(&e.map_entry_value_field().kind(), name))
        };
        if !keyed_by_string {
            return Err("a map is keyed by string");
        }
        if holds("Value") {
            Shape::Env
        } else if holds("SecretRef") {
            Shape::Secrets
        } else {
            return Err("a map holds `Value` or `SecretRef`");
        }
    } else if field.is_list() {
        if !matches!(field.kind(), PbKind::String) {
            return Err("a list holds strings");
        }
        Shape::List
    } else if message_named(&field.kind(), "Image") {
        Shape::Image
    } else if let PbKind::Message(inner) = field.kind() {
        let text = template.ok_or("a message needs a `render` template")?;
        Shape::Render(parse_template(&text, &inner).ok_or("the `render` template is malformed")?)
    } else if is_scalar(field) {
        Shape::Scalar
    } else {
        return Err("the type of the field cannot be lowered");
    };
    let fallback = match (&shape, raw) {
        (_, None) => None,
        (Shape::Render(_), Some(text)) => Some(FieldValue::Text(text)),
        (Shape::Scalar, Some(text)) if matches!(field.kind(), PbKind::String) => {
            Some(FieldValue::Text(text))
        }
        (Shape::Scalar, Some(text)) => Some(FieldValue::Int(
            text.parse()
                .map_err(|_| "the `fallback` is not an integer")?,
        )),
        (_, Some(_)) => return Err("a `fallback` needs a field with one value"),
    };
    Ok(FieldPlan {
        field: field.clone(),
        stem,
        shape,
        fallback,
    })
}

/// `{name}` placeholders over the scalar fields of `message`; `None` if one is unknown or the
/// braces do not pair.
fn parse_template(text: &str, message: &MessageDescriptor) -> Option<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let (literal, tail) = rest.split_at(open);
        let close = tail.find('}')?;
        let field = message
            .get_field_by_name(&tail[1..close])
            .filter(is_scalar)?;
        pieces.push(Piece::Text(literal.to_owned()));
        pieces.push(Piece::Field(field));
        rest = &tail[close + 1..];
    }
    pieces.push(Piece::Text(rest.to_owned()));
    Some(pieces)
}

/// The sorted entries of a map field whose keys are strings.
pub fn entries(map: &Pb) -> Vec<(&str, &Pb)> {
    let mut found: Vec<(&str, &Pb)> = map
        .as_map()
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| k.as_str().map(|k| (k, v)))
        .collect();
    found.sort_by_key(|entry| entry.0);
    found
}
