// SPDX-License-Identifier: Apache-2.0
//! The typed templates are read from the descriptors: a kind added to `resource.proto` is a new
//! class and nothing else changes, and a kind the templates cannot express is refused.

mod common;

use common::{pool_with, root, text_of};
use prost_reflect::DescriptorPool;
use ucdk_codegen::{Error, File, python, typescript};

const RESOURCE: &str = "ucdk/resource/v1/resource.proto";

fn resource_proto() -> String {
    let path = root().join(ucdk_codegen::PROTO_ROOTS[0]).join(RESOURCE);
    std::fs::read_to_string(path).expect("resource.proto")
}

/// The resource protos with `edit` applied to their text.
fn edited(edit: impl Fn(String) -> String) -> DescriptorPool {
    let text = edit(resource_proto());
    pool_with(&[(RESOURCE, &text)], RESOURCE, true)
}

/// The resource protos with one more kind: an arm of the `body` oneof and its message.
fn with_kind(arm: &str, message: &str) -> DescriptorPool {
    edited(|text| {
        let added = text.replace(
            "    Job job = 11;\n",
            &format!("    Job job = 11;\n    {arm}\n"),
        );
        format!("{added}\n{message}\n")
    })
}

fn generate(pool: &DescriptorPool) -> Result<Vec<File>, Error> {
    python::generate(pool, &[RESOURCE])
}

fn refused(pool: &DescriptorPool, subject: &str, why: &str) {
    match generate(pool) {
        Err(Error::Refused { subject: s, why: w }) => {
            assert!(s.contains(subject), "{s}");
            assert!(w.contains(why), "{w}");
        }
        other => panic!("not refused: {:?}", other.map(|f| f.len())),
    }
}

const QUEUE: &str = "message Queue {
  option (portability) = CLOUD_BOUND;
  option (exposes) = HOST;
  Image image = 1;
  uint32 depth = 2;
}";

#[test]
fn a_new_kind_is_a_new_class_and_changes_no_other_generated_file_but_the_resource_module() {
    let before = generate(&edited(|t| t)).expect("generates");
    let after = generate(&with_kind("Queue queue = 12;", QUEUE)).expect("generates");

    let changed: Vec<&str> = after
        .iter()
        .filter(|f| !before.contains(f))
        .map(|f| f.path.rsplit('/').next().expect("a name"))
        .collect();
    assert_eq!(changed, ["ucdk_resource_v1.py", "resources.py"]);
    assert_eq!(after.len(), before.len());

    let templates = text_of(&after, "resources.py");
    assert!(templates.contains("class Queue(_Template):"));
    assert!(templates.contains("    KIND = \"queue\""));
    assert!(templates.contains("    PORTABILITY = ucdk_resource_v1.Portability.CLOUD_BOUND"));
    assert!(templates.contains("    EXPOSES = (ucdk_resource_v1.Output.HOST,)"));
    assert!(templates.contains("depth: int = 0,"));
    assert!(templates.contains("    def host(self) -> ucdk_resource_v1.Ref:"));
    // Its image is required, like the other kinds'.
    assert!(templates.contains("        image: ucdk_resource_v1.Image,\n        *,"));
    // The kinds that were there are written as before.
    let old = text_of(&before, "resources.py");
    let class_service = old.find("class Service(_Template):").expect("Service");
    let class_job = old.find("class Job(_Template):").expect("Job");
    assert!(templates.contains(&old[class_service..class_job]));
}

#[test]
fn an_arm_of_the_body_that_is_not_a_message_is_not_a_kind() {
    let files = generate(&with_kind("string note = 12;", "")).expect("generates");
    let templates = text_of(&files, "resources.py");
    assert!(!templates.contains("note"));
    assert!(templates.contains("class Service(_Template):"));
}

#[test]
fn a_field_named_like_a_part_of_the_template_is_refused() {
    let odd = "message Odd { option (portability) = PORTABLE; string id = 1; }";
    refused(
        &with_kind("Odd odd = 12;", odd),
        "ucdk.resource.v1.Odd",
        "named `id`",
    );
    let odd = "message Odd { option (portability) = PORTABLE; Size uses = 1; }";
    refused(
        &with_kind("Odd odd = 12;", odd),
        "ucdk.resource.v1.Odd",
        "named `uses`",
    );
}

#[test]
fn an_output_named_like_a_part_of_the_template_is_refused() {
    // `ref` is the template's own way to name the resource itself.
    let pool = edited(|t| {
        let t = t.replace("  HOST = 2;\n", "  HOST = 2;\n  REF = 3;\n");
        let t = t.replace(
            "option (exposes) = HOST;\n",
            "option (exposes) = HOST;\n  option (exposes) = REF;\n",
        );
        format!("{t}\n")
    });
    refused(&pool, "ucdk.resource.v1.Service", "named `ref`");
}

#[test]
fn a_oneof_of_a_kind_must_hold_distinct_messages() {
    let odd =
        "message Odd { option (portability) = PORTABLE; oneof o { string a = 1; Size b = 2; } }";
    refused(&with_kind("Odd odd = 12;", odd), "a", "must be a message");
    let odd =
        "message Odd { option (portability) = PORTABLE; oneof o { Size a = 1; Size b = 2; } }";
    refused(&with_kind("Odd odd = 12;", odd), "b", "same message");
}

#[test]
fn a_kind_with_a_field_the_generator_cannot_write_is_refused() {
    let odd = "message Odd { option (portability) = PORTABLE; float f = 1; }";
    refused(
        &with_kind("Odd odd = 12;", odd),
        "ucdk.resource.v1.Odd.f",
        "not supported",
    );
}

#[test]
fn templates_need_the_pieces_they_are_read_from() {
    for (name, rename) in [
        ("an enum", ("Access", "Grant")),
        ("the body", ("oneof body", "oneof main")),
        ("an option", ("exposes", "reveals")),
    ] {
        let pool = edited(|t| t.replace(rename.0, rename.1));
        match generate(&pool) {
            Err(Error::Missing(what)) => assert!(what.contains("`body`"), "{name}: {what}"),
            other => panic!("{name}: {:?}", other.map(|f| f.len())),
        }
    }
}

// ---- the TypeScript templates read the same kinds ------------------------------------------

fn generate_ts(pool: &DescriptorPool) -> Result<Vec<File>, Error> {
    typescript::generate(pool, &[RESOURCE])
}

#[test]
fn a_new_kind_is_a_new_typescript_class_and_changes_no_other_generated_file_but_two() {
    let before = generate_ts(&edited(|t| t)).expect("generates");
    let after = generate_ts(&with_kind("Queue queue = 12;", QUEUE)).expect("generates");

    let changed: Vec<&str> = after
        .iter()
        .filter(|f| !before.contains(f))
        .map(|f| f.path.rsplit('/').next().expect("a name"))
        .collect();
    assert_eq!(changed, ["ucdk_resource_v1.ts", "resources.ts"]);
    assert_eq!(after.len(), before.len());

    let templates = text_of(&after, "resources.ts");
    assert!(templates.contains("export class Queue extends Template {"));
    assert!(templates.contains("  static readonly KIND = \"queue\";"));
    assert!(
        templates
            .contains("  static readonly PORTABILITY = ucdk_resource_v1.Portability.CLOUD_BOUND;")
    );
    assert!(
        templates.contains("  static readonly EXPOSES = [ucdk_resource_v1.Output.HOST] as const;")
    );
    assert!(templates.contains("  depth?: number;"));
    assert!(templates.contains("  get host(): ucdk_resource_v1.Ref {"));
    // Its image is required, like the other kinds'.
    assert!(templates.contains(
        "constructor(id: string, image: ucdk_resource_v1.Image, options: QueueOptions = {}) {"
    ));
    assert!(templates.contains("      queue: this.body,"));
    // The kinds that were there are written as before.
    let old = text_of(&before, "resources.ts");
    let class_service = old
        .find("export interface ServiceOptions")
        .expect("Service");
    let class_job = old.find("export interface JobOptions").expect("Job");
    assert!(templates.contains(&old[class_service..class_job]));
}

#[test]
fn a_typescript_template_refuses_what_the_python_one_does_and_its_own_names() {
    let odd = "message Odd { option (portability) = PORTABLE; string id = 1; }";
    let pool = with_kind("Odd odd = 12;", odd);
    match generate_ts(&pool) {
        Err(Error::Refused { why, .. }) => assert!(why.contains("named `id`"), "{why}"),
        other => panic!("not refused: {:?}", other.map(|f| f.len())),
    }
    // `to_resource` is Python's name for the method, and TypeScript's is `toResource`: the
    // proto name is the same field in both.
    let odd = "message Odd { option (portability) = PORTABLE; string to_resource = 1; }";
    let pool = with_kind("Odd odd = 12;", odd);
    refused(&pool, "ucdk.resource.v1.Odd", "named `to_resource`");
    match generate_ts(&pool) {
        Err(Error::Refused { why, .. }) => assert!(why.contains("named `toResource`"), "{why}"),
        other => panic!("not refused: {:?}", other.map(|f| f.len())),
    }
}

#[test]
fn a_oneof_of_a_typescript_kind_takes_each_arm_and_refuses_the_rest() {
    let text = text_of(
        &generate_ts(&edited(|t| t)).expect("generates"),
        "resources.ts",
    )
    .to_owned();
    assert!(text.contains(
        "      public: options.exposure instanceof ucdk_resource_v1.Service_Public ? options.exposure : undefined,"
    ));
    assert!(
        text.contains(
            "if (options.exposure !== undefined && body.whichExposure() === undefined) {"
        )
    );
    assert!(text.contains("throw new TypeError(\"exposure must be one of Public, Internal\");"));
    assert!(text.contains("throw new TypeError(\"a Service needs `image`\");"));
    let odd =
        "message Odd { option (portability) = PORTABLE; oneof o { string a = 1; Size b = 2; } }";
    match generate_ts(&with_kind("Odd odd = 12;", odd)) {
        Err(Error::Refused { why, .. }) => assert!(why.contains("must be a message"), "{why}"),
        other => panic!("not refused: {:?}", other.map(|f| f.len())),
    }
}

#[test]
fn typescript_templates_need_the_pieces_they_are_read_from() {
    let pool = edited(|t| t.replace("Access", "Grant"));
    assert!(matches!(generate_ts(&pool), Err(Error::Missing(_))));
}
