// SPDX-License-Identifier: Apache-2.0
//! The value of one lowered field, and the extra nodes of a type, from the plans the descriptor
//! gave. The catalog's versioned defaults are the `fallback` options in `resource.proto`: they
//! are written out where the author wrote nothing, so that writing a default is not a change
//! and a console edit of a modelled field is drift.

use prost_reflect::{DynamicMessage, Value as Pb};
use ucdk_proto::v1::SecretRef;
use ucdk_provider::{DesiredField, FieldValue, Kind};

use super::descriptor::{FieldPlan, KindPlan, Piece, Shape, scalar, scalar_text};
use super::read::Role;

/// The role name of a service's second node, as `resource.proto` declares it.
pub const ROLE_PUBLIC: &str = "public";
/// The role name of a job's second node, as `resource.proto` declares it.
pub const ROLE_SCHEDULE: &str = "schedule";

/// A secret as `[store/]name[@version]`.
pub fn secret(secret: &SecretRef) -> String {
    let store = secret
        .store
        .as_ref()
        .map_or_else(String::new, |s| format!("{s}/"));
    let version = secret
        .version
        .as_ref()
        .map_or_else(String::new, |v| format!("@{v}"));
    format!("{store}{}{version}", secret.name)
}

/// The field of a scalar or templated plan: the fallback if the field was not written and there
/// is one, else the value (a template filled in from the message).
pub fn single(plan: &FieldPlan, written: bool, value: &Pb) -> DesiredField {
    let lowered = match (&plan.fallback, written, &plan.shape) {
        (Some(fallback), false, _) => fallback.clone(),
        (_, _, Shape::Render(pieces)) => FieldValue::Text(
            value
                .as_message()
                .map_or_else(String::new, |m| fill(pieces, m)),
        ),
        _ => scalar(value),
    };
    DesiredField::new(&plan.stem, lowered)
}

fn fill(pieces: &[Piece], message: &DynamicMessage) -> String {
    pieces
        .iter()
        .map(|piece| match piece {
            Piece::Text(text) => text.clone(),
            Piece::Field(field) => scalar_text(&message.get_field(field)),
        })
        .collect()
}

/// The extra nodes of a type: one per role, wanted when the role's field is written.
pub fn roles(kind: &KindPlan, body: &DynamicMessage) -> Vec<Role> {
    kind.roles
        .iter()
        .map(|plan| {
            let wanted = body.has_field(&plan.arm);
            let arm = body.get_field(&plan.arm);
            let fields = arm
                .as_message()
                .filter(|_| wanted)
                .map(|m| {
                    plan.fields
                        .iter()
                        .map(|f| DesiredField::new(f.name(), scalar(&m.get_field(f))))
                        .collect()
                })
                .unwrap_or_default();
            Role {
                role: plan.role.clone(),
                kind: Kind::new(&plan.role),
                fields,
                wanted,
            }
        })
        .collect()
}
