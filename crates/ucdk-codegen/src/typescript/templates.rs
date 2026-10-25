// SPDX-License-Identifier: Apache-2.0
//! Typed templates for the resource kinds, and typed references to their outputs.
//!
//! The kinds are read as the Python target reads them (`kinds`): one class per kind that takes
//! its id and the fields it cannot run without, then an object of options, each oneof of the
//! kind as one option that takes either arm, and a getter per output the kind `exposes`.

use prost_reflect::{DescriptorPool, EnumDescriptor};

use super::message::init_type;
use super::names::{doc, member, pair, param, pascal};
use super::types::{member_ref, ty};
use crate::Error;
use crate::kinds::{self, Catalog, Kind, Naming, is_values, member_of, value_of};
use crate::names::local_name;

const BASE: &str = include_str!("template_base.ts");

/// Names a template keeps for itself; a field of a kind may not be called one.
const NAMING: Naming = Naming {
    name: member,
    own: &["id", "uses", "ref", "body", "toResource", "KIND"],
};

const DOC: &str = "/**
 * Typed resource templates and typed references to their outputs.
 *
 * A template is built from its fields, so a missing image or a misspelt option is a type error
 * and not a refusal at plan time. `toResource()` gives the wire message.
 */
";

/// The module of the templates, or `None` when the protos declare no `Resource`. `modules` are
/// the modules of the other packages; those the templates refer to are imported.
pub fn generate(pool: &DescriptorPool, modules: &[String]) -> Result<Option<String>, Error> {
    let Some(catalog) = kinds::find(pool)? else {
        return Ok(None);
    };
    let access = member_ref(&catalog.access, catalog.granted().name(), "");
    let mut body: Vec<String> = BASE
        .replace("@ACCESS@", &access)
        .lines()
        .map(str::to_owned)
        .collect();
    for (name, message) in catalog.arms() {
        let kind = kinds::read(&catalog, &name, &message, &NAMING)?;
        body.extend(template(&catalog, &kind));
    }
    let text = body.join("\n");
    let mut out: Vec<String> = super::HEADER.lines().map(str::to_owned).collect();
    out.extend(DOC.lines().map(str::to_owned));
    out.push(String::new());
    out.push("import * as wire from \"./wire.ts\";".to_owned());
    out.extend(
        modules
            .iter()
            .filter(|m| text.contains(&format!("{m}.")))
            .map(|m| format!("import * as {m} from \"./{m}.ts\";")),
    );
    out.push(String::new());
    out.push(text);
    Ok(Some(out.join("\n") + "\n"))
}

fn enum_expr(e: &EnumDescriptor, number: i32) -> String {
    member_ref(e, value_of(e, number).name(), "")
}

fn template(catalog: &Catalog, kind: &Kind) -> Vec<String> {
    let class = kind.message.name();
    let wire_class = format!(
        "{}.{}",
        crate::names::module_of(kind.message.package_name()),
        local_name(kind.message.full_name(), kind.message.package_name())
    );
    let options = format!("{class}Options");
    let mut out = vec![String::new()];

    out.push(format!("export interface {options} {{"));
    out.push("  uses?: Iterable<UseInit>;".to_owned());
    for f in &kind.optional {
        if let Some(text) = f.doc.as_deref() {
            out.extend(doc(text, 2));
        }
        if is_values(f) {
            out.push(format!("  {}?: wire.MapInit<ValueInit>;", f.attr));
        } else {
            out.push(format!("  {}?: {};", f.attr, init_type(f, "")));
        }
    }
    for oneof in &kind.oneofs {
        let types: Vec<String> = oneof
            .arms
            .iter()
            .map(|(arm, _)| ty(&arm.scalar, ""))
            .collect();
        out.push(format!("  {}?: {};", oneof.name, types.join(" | ")));
    }
    out.push("}".to_owned());
    out.push(String::new());

    out.extend(doc(
        &format!("The `{}` resource.\n\n{}", kind.name, kind.doc),
        0,
    ));
    out.push(format!("export class {class} extends Template {{"));
    out.push(format!("  static readonly KIND = \"{}\";", kind.name));
    out.push(format!(
        "  static readonly PORTABILITY = {};",
        enum_expr(&catalog.marking, kind.marking)
    ));
    let members: Vec<String> = kind
        .exposed
        .iter()
        .map(|n| enum_expr(&catalog.output, *n))
        .collect();
    out.push(format!(
        "  static readonly EXPOSES = [{}] as const;",
        members.join(", ")
    ));
    for oneof in &kind.oneofs {
        for (arm, message) in &oneof.arms {
            out.push(format!(
                "  static readonly {} = {};",
                message.name(),
                ty(&arm.scalar, "")
            ));
        }
    }
    out.push(format!("  readonly body: {wire_class};"));
    out.push(String::new());

    let mut params = vec!["id: string".to_owned()];
    params.extend(
        kind.required
            .iter()
            .map(|f| format!("{}: {}", param(&f.attr), ty(&f.scalar, ""))),
    );
    params.push(format!("options: {options} = {{}}"));
    out.push(format!("  constructor({}) {{", params.join(", ")));
    out.push("    super(id, options.uses ?? []);".to_owned());
    for f in &kind.required {
        out.push(format!("    if ({} === undefined) {{", param(&f.attr)));
        out.push(format!(
            "      throw new TypeError(\"a {class} needs `{}`\");",
            f.attr
        ));
        out.push("    }".to_owned());
    }
    out.push(format!("    const body = new {wire_class}({{"));
    for f in &kind.required {
        out.push(format!("      {},", pair(&f.attr, &param(&f.attr))));
    }
    for f in &kind.optional {
        let value = if is_values(f) {
            format!("values(options.{})", f.attr)
        } else {
            format!("options.{}", f.attr)
        };
        out.push(format!("      {}: {value},", f.attr));
    }
    // Each arm of a oneof is given the option when it is of that arm's type.
    for oneof in &kind.oneofs {
        let given = format!("options.{}", oneof.name);
        for (arm, _) in &oneof.arms {
            out.push(format!(
                "      {}: {given} instanceof {} ? {given} : undefined,",
                arm.attr,
                ty(&arm.scalar, "")
            ));
        }
    }
    out.push("    });".to_owned());
    for oneof in &kind.oneofs {
        let given = format!("options.{}", oneof.name);
        let names: Vec<String> = oneof
            .arms
            .iter()
            .map(|(_, message)| message.name().to_owned())
            .collect();
        out.push(format!(
            "    if ({given} !== undefined && body.which{}() === undefined) {{",
            pascal(&oneof.name)
        ));
        out.push(format!(
            "      throw new TypeError(\"{} must be one of {}\");",
            oneof.name,
            names.join(", ")
        ));
        out.push("    }".to_owned());
    }
    out.push("    this.body = body;".to_owned());
    out.push("  }".to_owned());

    for (property, number) in &kind.properties {
        out.push(String::new());
        out.push(format!(
            "  /** A reference to the `{}` this resource exposes once applied. */",
            member_of(&catalog.output, *number)
        ));
        out.push(format!("  get {property}(): ucdk_resource_v1.Ref {{"));
        out.push(format!(
            "    return new ucdk_resource_v1.Ref({{ resource: this.id, standard: {} }});",
            enum_expr(&catalog.output, *number)
        ));
        out.push("  }".to_owned());
    }

    out.push(String::new());
    out.push("  toResource(): ucdk_resource_v1.Resource {".to_owned());
    out.push("    return new ucdk_resource_v1.Resource({".to_owned());
    out.push("      id: this.id,".to_owned());
    out.push("      uses: this.usesAsWire(),".to_owned());
    out.push(format!("      {}: this.body,", member(&kind.name)));
    out.push("    });".to_owned());
    out.push("  }".to_owned());
    out.push("}".to_owned());
    out
}
