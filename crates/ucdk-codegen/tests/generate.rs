// SPDX-License-Identifier: Apache-2.0
//! Output for a fixture proto that has every supported field shape, held as golden files, the
//! output for the repository's protos, and what the generator refuses.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use common::{SHAPES, generate_shapes, pool_of, pool_with, refusal, root, shapes_pool, text_of};
use ucdk_codegen::{Error, python};

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/shapes")
}

fn names(dir: &Path) -> BTreeSet<String> {
    fn walk(dir: &Path, prefix: &str, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).expect("a directory").flatten() {
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.path().is_dir() {
                walk(&entry.path(), &format!("{name}/"), out);
            } else {
                out.insert(name);
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(dir, "", &mut out);
    out
}

/// Rewrites the goldens. The Python test `test_shapes.py` loads them, so a change is read twice:
/// as a diff, and as code that must still pass.
#[test]
#[ignore = "writes the golden files; run on purpose"]
fn write_goldens() {
    let dir = golden_dir();
    for file in generate_shapes() {
        let path = dir.join(&file.path);
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("create");
        std::fs::write(path, file.text).expect("write a golden");
    }
}

#[test]
fn the_fixture_generates_the_golden_files() {
    let files = generate_shapes();
    for file in &files {
        let on_disk = std::fs::read_to_string(golden_dir().join(&file.path))
            .unwrap_or_else(|e| panic!("{} is missing: {e}", file.path));
        assert_eq!(on_disk, file.text, "{} differs from the golden", file.path);
    }
    let generated: BTreeSet<String> = files.iter().map(|f| f.path.clone()).collect();
    assert_eq!(names(&golden_dir()), generated, "a golden is left over");
    // No `Resource` in the fixture, so no templates.
    assert!(!generated.iter().any(|p| p.ends_with("resources.py")));
}

#[test]
fn the_repository_protos_generate_a_module_per_package_and_the_templates() {
    let files = ucdk_codegen::python_for(&root()).expect("the protos generate");
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "ucdk/generated/__init__.py",
            "ucdk/generated/_wire.py",
            "ucdk/generated/google_protobuf.py",
            "ucdk/generated/ucdk_api_v1.py",
            "ucdk/generated/ucdk_resource_v1.py",
            "ucdk/generated/resources.py",
        ]
    );
    assert!(files.iter().all(|f| f.text.contains("Do not edit.")));
    let resources = text_of(&files, "resources.py");
    assert!(resources.contains("class Service(_Template):"));
    assert!(resources.contains("class Job(_Template):"));
    assert!(resources.contains("PORTABILITY = ucdk_resource_v1.Portability.PORTABLE"));
    assert!(
        resources.contains("EXPOSES = (ucdk_resource_v1.Output.URL, ucdk_resource_v1.Output.HOST)")
    );
    assert!(resources.contains("EXPOSES = ()"));
    assert!(resources.contains("def url(self) -> ucdk_resource_v1.Ref:"));
    assert!(resources.contains("from . import google_protobuf"));
    let api = text_of(&files, "ucdk_api_v1.py");
    assert!(api.contains("from . import ucdk_resource_v1"));
    assert!(api.contains("class Outcome(_wire.Message):"));
    // The enum values lose the name of their enum.
    assert!(api.contains("    FAILED_NO_EFFECT = 4"));
    // The nested messages of the resource protos have flat names.
    assert!(
        text_of(&files, "ucdk_resource_v1.py").contains("class Service_Public(_wire.Message):")
    );
}

#[test]
fn the_wire_primitives_are_the_same_text_for_every_proto() {
    let one = text_of(&generate_shapes(), "_wire.py").to_owned();
    let other = ucdk_codegen::python_for(&root()).expect("generates");
    assert_eq!(text_of(&other, "_wire.py"), one);
}

#[test]
fn without_comments_there_are_no_docstrings() {
    let pool = pool_with(&[], SHAPES, false);
    let files = python::generate(&pool, &[SHAPES]).expect("generates");
    let text = text_of(&files, "example_shapes_v1.py");
    assert!(!text.contains("Every kind of field."));
    assert!(!text.contains("Attributes:"));
    assert!(text.contains("class Scalars(_wire.Message):"));
}

#[test]
fn an_enum_value_keeps_its_name_when_the_prefix_leaves_no_name() {
    let text = generate_shapes()
        .into_iter()
        .find(|f| f.path.ends_with("example_shapes_v1.py"))
        .expect("the module")
        .text;
    assert!(
        text.contains("    UNSPECIFIED = 0\n    FAST = 1\n    # Carries no prefix.\n    SLOW = 2")
    );
    assert!(text.contains("    MODE_3D = 3"));
    // A digit inside the enum's name is part of its prefix.
    assert!(text.contains("class Http2Mode(enum.IntEnum):"));
    assert!(text.contains("    ON = 1"));
}

#[test]
fn a_python_keyword_or_a_method_name_gets_a_trailing_underscore() {
    let text = generate_shapes()
        .into_iter()
        .find(|f| f.path.ends_with("example_shapes_v1.py"))
        .expect("the module")
        .text;
    assert!(text.contains("from_: str = \"\","));
    assert!(text.contains("encode_: str = \"\","));
}

// ---- what is refused -----------------------------------------------------------------------

fn refused(source: &str, subject: &str, why: &str) {
    match refusal(source) {
        Error::Refused { subject: s, why: w } => {
            assert!(s.contains(subject), "{s}");
            assert!(w.contains(why), "{w}");
        }
        other => panic!("not a refusal: {other:?}"),
    }
}

#[test]
fn a_type_the_generator_has_no_code_for_is_refused_by_field() {
    refused("message M { float f = 1; }", "t.M.f", "not supported");
    refused("message M { bytes b = 1; }", "t.M.b", "not supported");
    refused("message M { sint32 s = 1; }", "t.M.s", "not supported");
    refused(
        "message M { oneof o { fixed32 f = 1; } }",
        "t.M.f",
        "not supported",
    );
}

#[test]
fn a_map_needs_string_keys_and_a_supported_value() {
    refused(
        "message M { map<int32, string> m = 1; }",
        "t.M.m",
        "key must be a string",
    );
    refused(
        "message M { map<string, float> m = 1; }",
        "t.M.m",
        "not supported",
    );
}

#[test]
fn a_repeated_number_would_be_packed_and_is_refused() {
    refused("message M { repeated int32 r = 1; }", "t.M.r", "packed");
    refused("message M { repeated bool r = 1; }", "t.M.r", "packed");
}

#[test]
fn two_types_that_flatten_to_one_python_name_are_refused() {
    refused(
        "message A { message B {} }\nmessage A_B {}",
        "t.A_B",
        "same Python name",
    );
}

#[test]
fn a_root_file_that_is_not_in_the_pool_is_missing() {
    let pool = shapes_pool();
    assert_eq!(
        python::generate(&pool, &["nope.proto"]),
        Err(Error::Missing("nope.proto".to_owned()))
    );
}

#[test]
fn the_errors_say_what_they_are() {
    assert_eq!(
        Error::Missing("x.proto".to_owned()).to_string(),
        "`x.proto` is not in the descriptors"
    );
    assert_eq!(
        Error::Refused {
            subject: "t.M.f".to_owned(),
            why: "no".to_owned()
        }
        .to_string(),
        "t.M.f: no"
    );
    let compile = ucdk_codegen::compile(Path::new("/no/such/root")).expect_err("no protos");
    assert!(matches!(compile, Error::Compile(_)), "{compile:?}");
    assert!(
        compile
            .to_string()
            .starts_with("the protos do not compile: ")
    );
    let std_error: &dyn std::error::Error = &compile;
    assert!(std_error.source().is_none());
}

#[test]
fn a_missing_proto_file_is_a_compile_error() {
    let roots = [root().join(ucdk_codegen::PROTO_ROOTS[0])];
    let err = ucdk_codegen::compile_files(&roots, &["no/such.proto"]).expect_err("no file");
    assert!(matches!(err, Error::Compile(_)), "{err:?}");
}

#[test]
fn a_pool_of_an_inline_proto_generates() {
    let pool = pool_of("syntax = \"proto3\";\npackage t;\nmessage M { string s = 1; }");
    let files = python::generate(&pool, &["test.proto"]).expect("generates");
    assert!(text_of(&files, "t.py").contains("class M(_wire.Message):"));
    assert!(text_of(&files, "__init__.py").contains("The generated part"));
}
