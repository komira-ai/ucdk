// SPDX-License-Identifier: Apache-2.0
//! Lowering is driven by the descriptors: a resource type declared only in a proto plans, hashes
//! and exposes outputs with no lowering code, and a type the descriptors cannot lower is refused
//! when the catalog is read.

use prost_reflect::ReflectMessage as _;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, Value};
use protox::Compiler;
use protox::file::{
    ChainFileResolver, File, FileResolver, GoogleFileResolver, IncludeFileResolver,
};
use ucdk_plan::catalog::{Catalog, CatalogError};
use ucdk_plan::{FindingKind, LedgerToken, Observed, ValueRef, plan};
use ucdk_proto::v1::{Image, Output, Portability, Service, Value as Wire, image, value};
use ucdk_provider::{FieldValue, Kind, NodePath, kit};

const TEST_PROTOS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/proto");
const UCDK_PROTOS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../ucdk-proto/proto");

/// Source text held in memory, found by name.
struct Memory(Vec<(String, String)>);

impl FileResolver for Memory {
    fn open_file(&self, name: &str) -> Result<File, protox::Error> {
        self.0.iter().find(|(n, _)| n == name).map_or_else(
            || Err(protox::Error::file_not_found(name)),
            |(_, source)| File::from_source(name, source),
        )
    }
}

fn pool(memory: Vec<(String, String)>, open: &str) -> DescriptorPool {
    let mut chain = ChainFileResolver::new();
    chain.add(Memory(memory));
    chain.add(IncludeFileResolver::new(TEST_PROTOS.into()));
    chain.add(IncludeFileResolver::new(UCDK_PROTOS.into()));
    chain.add(GoogleFileResolver::new());
    let mut compiler = Compiler::with_file_resolver(chain);
    compiler.open_file(open).expect("the proto compiles");
    compiler.descriptor_pool()
}

fn queue_pool() -> DescriptorPool {
    pool(Vec::new(), "queue.proto")
}

fn queue_catalog() -> Catalog {
    let list = queue_pool()
        .get_message_by_name("example.queue.v1.ResourceList")
        .expect("the list");
    Catalog::new(list).expect("the queue catalog is well formed")
}

fn desc(pool: &DescriptorPool, name: &str) -> MessageDescriptor {
    pool.get_message_by_name(name)
        .expect("a message of the pool")
}

/// A generated message, as the dynamic message of the same name in `pool`.
fn dynamic(pool: &DescriptorPool, name: &str, message: &impl prost::Message) -> Value {
    let decoded = DynamicMessage::decode(desc(pool, name), message.encode_to_vec().as_slice())
        .expect("the same schema decodes");
    Value::Message(decoded)
}

fn set(message: &mut DynamicMessage, name: &str, value: Value) {
    message.set_field_by_name(name, value);
}

/// A map field with one entry.
fn map_of(key: &str, value: Value) -> Value {
    Value::Map(
        [(prost_reflect::MapKey::String(key.to_owned()), value)]
            .into_iter()
            .collect(),
    )
}

fn text(t: &str) -> Value {
    Value::String(t.to_owned())
}

fn digest() -> Image {
    Image {
        source: Some(image::Source::Digest("sha256:aa".to_owned())),
        platform: String::new(),
    }
}

/// A resource of the queue catalog with `body` set under the field `arm`.
fn resource(pool: &DescriptorPool, id: &str, arm: &str, body: Value) -> Value {
    let mut r = DynamicMessage::new(desc(pool, "example.queue.v1.Resource"));
    set(&mut r, "id", text(id));
    set(&mut r, arm, body);
    Value::Message(r)
}

fn queue_with(pool: &DescriptorPool, f: impl FnOnce(&mut DynamicMessage)) -> Value {
    let mut q = DynamicMessage::new(desc(pool, "example.queue.v1.Queue"));
    f(&mut q);
    Value::Message(q)
}

fn list_of(catalog: &Catalog, resources: Vec<Value>) -> DynamicMessage {
    let mut l = catalog.new_list();
    set(&mut l, "resource", Value::List(resources));
    l
}

/// A service `web` that reads the queue's `output` as its `LINK` setting.
fn web_reading(pool: &DescriptorPool, output: Output) -> Value {
    let link = Wire {
        v: Some(value::V::Ref(ucdk_proto::v1::Ref {
            resource: "jobs".to_owned(),
            output: Some(ucdk_proto::v1::r#ref::Output::Standard(output as i32)),
        })),
    };
    let mut svc = Service {
        image: Some(digest()),
        ..Service::default()
    };
    svc.env.insert("LINK".to_owned(), link);
    resource(
        pool,
        "web",
        "service",
        dynamic(pool, "ucdk.resource.v1.Service", &svc),
    )
}

fn jobs_queue(pool: &DescriptorPool) -> Value {
    let mut tags = Value::List(vec![text("a"), text("b")]);
    let _ = &mut tags;
    resource(
        pool,
        "jobs",
        "queue",
        queue_with(pool, |q| {
            set(q, "topic", text("orders"));
            set(q, "tags", tags);
            let mode = Wire {
                v: Some(value::V::Literal("strict".to_owned())),
            };
            let mode = dynamic(pool, "ucdk.resource.v1.Value", &mode);
            set(q, "settings", map_of("MODE", mode));
            let mut fifo = DynamicMessage::new(desc(pool, "example.queue.v1.Queue.Fifo"));
            set(&mut fifo, "group", text("g1"));
            set(&mut fifo, "dedup_window_seconds", Value::I64(300));
            set(q, "fifo", Value::Message(fifo));
        }),
    )
}

fn fields(node: &ucdk_plan::PlanNode) -> Vec<(String, FieldValue)> {
    node.node
        .fields()
        .iter()
        .map(|f| (f.name.clone(), f.value.clone()))
        .collect()
}

fn t(name: &str, v: &str) -> (String, FieldValue) {
    (name.to_owned(), FieldValue::Text(v.to_owned()))
}

fn i(name: &str, v: i64) -> (String, FieldValue) {
    (name.to_owned(), FieldValue::Int(v))
}

// ---- a type declared only in a proto ------------------------------------------------------

/// Proves: a type that exists only as a message in a test proto lowers: its fields, its
/// repeated and map fields, its template fields, its role node (wanted, with the arm's fields)
/// and its kind come from the descriptors, and the node order follows the references.
#[test]
fn a_type_declared_only_in_a_proto_lowers_by_its_descriptor() {
    let catalog = queue_catalog();
    let pool = queue_pool();
    let l = list_of(
        &catalog,
        vec![web_reading(&pool, Output::Host), jobs_queue(&pool)],
    );
    assert!(catalog.validate(&l).is_empty());
    let lowered = catalog.lower(&l).expect("the list lowers");
    let paths: Vec<&str> = lowered
        .nodes()
        .iter()
        .map(|n| n.node.path().as_str())
        .collect();
    assert_eq!(paths, ["jobs/run", "jobs/fifo", "web/run", "web/public"]);

    let run = &lowered.nodes()[0];
    assert_eq!(run.node.kind(), &Kind::new("queue"));
    assert_eq!(
        fields(run),
        [
            t("dead_letter", "none"),
            t("retention", "345600s"),
            t("settings.MODE", "strict"),
            t("size", "250m/128MB"),
            t("tag.0", "a"),
            t("tag.1", "b"),
            t("topic", "orders"),
            i("visibility_seconds", 30),
        ]
    );
    let fifo = &lowered.nodes()[1];
    assert_eq!(fifo.node.kind(), &Kind::new("fifo"));
    assert!(fifo.wanted);
    assert_eq!(
        fields(fifo),
        [i("dedup_window_seconds", 300), t("group", "g1")]
    );
    assert_eq!(
        fifo.deps.iter().map(NodePath::as_str).collect::<Vec<_>>(),
        ["jobs/run"]
    );
}

/// Proves: the service of the built-in catalog lowers in the new catalog exactly as it does in
/// the built-in one: the shared types carry their own descriptors, so a catalog that adds a
/// type keeps the old ones.
#[test]
fn the_new_catalog_lowers_the_old_types_as_the_built_in_one_does() {
    let catalog = queue_catalog();
    let pool = queue_pool();
    let svc = Service {
        image: Some(digest()),
        port: 9000,
        ..Service::default()
    };
    let here = list_of(
        &catalog,
        vec![resource(
            &pool,
            "web",
            "service",
            dynamic(&pool, "ucdk.resource.v1.Service", &svc),
        )],
    );
    let there = ucdk_plan::lower(&ucdk_proto::v1::ResourceList {
        resource: vec![ucdk_proto::v1::Resource {
            id: "web".to_owned(),
            body: Some(ucdk_proto::v1::resource::Body::Service(svc)),
            ..Default::default()
        }],
    })
    .expect("lowers");
    assert_eq!(catalog.lower(&here).expect("lowers"), there);
}

/// Proves: a written-nothing queue takes the fallbacks the descriptor declares, and its role
/// node is still emitted, unwanted and without fields (closed-world removal).
#[test]
fn a_queue_with_nothing_written_takes_its_declared_fallbacks_and_keeps_its_role_node() {
    let catalog = queue_catalog();
    let pool = queue_pool();
    let l = list_of(
        &catalog,
        vec![resource(&pool, "jobs", "queue", queue_with(&pool, |_| {}))],
    );
    let lowered = catalog.lower(&l).expect("lowers");
    assert_eq!(
        fields(&lowered.nodes()[0]),
        [
            t("dead_letter", "none"),
            t("retention", "345600s"),
            t("size", "250m/128MB"),
            t("topic", ""),
            i("visibility_seconds", 30),
        ]
    );
    let fifo = &lowered.nodes()[1];
    assert_eq!(fifo.node.path().as_str(), "jobs/fifo");
    assert!(!fifo.wanted);
    assert!(fifo.node.fields().is_empty());
}

/// Proves: a written zero is not "not written": an explicit minimum of a message field and a
/// written size are used, and only an unwritten field takes the fallback.
#[test]
fn a_written_message_field_is_rendered_not_defaulted() {
    let catalog = queue_catalog();
    let pool = queue_pool();
    let size = ucdk_proto::v1::Size {
        cpu_millis: 500,
        memory_mb: 64,
    };
    let q = queue_with(&pool, |q| {
        set(q, "size", dynamic(&pool, "ucdk.resource.v1.Size", &size));
        set(q, "visibility_seconds", Value::U32(5));
    });
    let l = list_of(&catalog, vec![resource(&pool, "jobs", "queue", q)]);
    let lowered = catalog.lower(&l).expect("lowers");
    let got = fields(&lowered.nodes()[0]);
    assert!(got.contains(&t("size", "500m/64MB")));
    assert!(got.contains(&i("visibility_seconds", 5)));
}

/// Proves: an output the new type exposes is a reference the consumer can use: the consumer's
/// node carries it as a value reference and `bound` fills it from the producer.
#[test]
fn the_new_type_exposes_its_outputs_to_references() {
    let catalog = queue_catalog();
    let pool = queue_pool();
    let l = list_of(
        &catalog,
        vec![web_reading(&pool, Output::Host), jobs_queue(&pool)],
    );
    let lowered = catalog.lower(&l).expect("lowers");
    let web = &lowered.nodes()[2];
    assert_eq!(
        web.refs,
        [ValueRef {
            field: "env.LINK".to_owned(),
            resource: "jobs".to_owned(),
            output: Output::Host,
        }]
    );
    assert_eq!(
        web.deps.iter().map(NodePath::as_str).collect::<Vec<_>>(),
        ["jobs/run"]
    );
    let bound = web
        .bound(|resource, output| {
            (resource == "jobs" && output == "HOST").then_some("queue-host-1")
        })
        .expect("the producer's value is known");
    assert!(
        bound
            .fields()
            .iter()
            .any(|f| f.name == "env.LINK" && f.value == FieldValue::Text("queue-host-1".to_owned()))
    );
}

/// Proves: an output the new type does not expose is refused, naming the type's resource: the
/// `exposes` option of the message is what decides.
#[test]
fn an_output_the_new_type_does_not_expose_is_refused() {
    let catalog = queue_catalog();
    let pool = queue_pool();
    let l = list_of(
        &catalog,
        vec![web_reading(&pool, Output::Url), jobs_queue(&pool)],
    );
    let found = catalog.validate(&l);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].kind,
        FindingKind::UnexposedOutput {
            target: "jobs".to_owned(),
            output: "URL"
        }
    );
    assert_eq!(found[0].field, "service.env.LINK");
}

/// Proves: the plan of a list with the new type is stable: the same list plans to the same hash
/// every time, the hash is pinned, and any modelled field of the new type changes it.
#[test]
fn a_plan_with_the_new_type_hashes_stably() {
    let catalog = queue_catalog();
    let pool = queue_pool();
    let hash = |q: Value| {
        let l = list_of(
            &catalog,
            vec![
                web_reading(&pool, Output::Host),
                resource(&pool, "jobs", "queue", q),
            ],
        );
        let lowered = catalog.lower(&l).expect("lowers");
        let observed = Observed::new(LedgerToken::new("v1"), kit::cell());
        plan(&lowered, &observed, None)
            .expect("planned")
            .hash()
            .to_string()
    };
    let orders = |topic: &'static str| {
        let pool = queue_pool();
        queue_with(&pool, move |q| set(q, "topic", text(topic)))
    };
    let first = hash(orders("orders"));
    assert_eq!(first, hash(orders("orders")));
    assert_ne!(first, hash(orders("returns")));
    assert_eq!(first.len(), 64);
    assert_eq!(
        first,
        "594fb152f22f02f166a532632d8a2f27ddbb094ff4bc13c76bb4b898ee4e3413"
    );
}

/// Proves: a type marked `CLOUD_BOUND` in its descriptor is refused as a reference target by
/// plain `validate`, with no override from the caller.
#[test]
fn portability_comes_from_the_descriptor() {
    let source = include_str!("proto/queue.proto");
    // The same file with its type marked the other way, found before the real one.
    let bound = source.replace("= PORTABLE;", "= CLOUD_BOUND;");
    let pool = pool(vec![("queue.proto".to_owned(), bound)], "queue.proto");
    let catalog = Catalog::new(desc(&pool, "example.queue.v1.ResourceList")).expect("catalog");
    assert_eq!(catalog.portability("queue"), Some(Portability::CloudBound));
    assert_eq!(catalog.portability("service"), Some(Portability::Portable));
    assert_eq!(catalog.portability("nothing"), None);
    assert_eq!(catalog.kind_names(), ["service", "queue"]);
    let mut web = Service {
        image: Some(digest()),
        ..Service::default()
    };
    web.env.insert(
        "LINK".to_owned(),
        Wire {
            v: Some(value::V::Ref(ucdk_proto::v1::Ref {
                resource: "jobs".to_owned(),
                output: Some(ucdk_proto::v1::r#ref::Output::Standard(Output::Host as i32)),
            })),
        },
    );
    let l = list_of(
        &catalog,
        vec![
            resource(
                &pool,
                "web",
                "service",
                dynamic(&pool, "ucdk.resource.v1.Service", &web),
            ),
            resource(&pool, "jobs", "queue", queue_with(&pool, |_| {})),
        ],
    );
    let found = catalog.validate(&l);
    assert_eq!(
        found.iter().map(|f| f.kind.clone()).collect::<Vec<_>>(),
        [FindingKind::NotPortable {
            target: "jobs".to_owned()
        }]
    );
}

/// Proves: a key in both a value map and a secret map of a new type is refused, and a secret
/// lowers as `store/name@version`: the secret rules are properties of the shapes, not of the
/// two built-in types.
#[test]
fn secret_and_value_maps_of_a_new_type_follow_the_shape_rules() {
    let catalog = queue_catalog();
    let pool = queue_pool();
    let secret = ucdk_proto::v1::SecretRef {
        name: "api".to_owned(),
        store: Some("vault".to_owned()),
        version: Some("3".to_owned()),
    };
    let build = |overlap: bool| {
        let q = queue_with(&pool, |q| {
            let secret = dynamic(&pool, "ucdk.resource.v1.SecretRef", &secret);
            set(q, "secret_settings", map_of("TOKEN", secret));
            if overlap {
                let plain = Wire {
                    v: Some(value::V::Literal("x".to_owned())),
                };
                let plain = dynamic(&pool, "ucdk.resource.v1.Value", &plain);
                set(q, "settings", map_of("TOKEN", plain));
            }
        });
        list_of(&catalog, vec![resource(&pool, "jobs", "queue", q)])
    };
    let ok = catalog.lower(&build(false)).expect("lowers");
    assert!(fields(&ok.nodes()[0]).contains(&t("secret_settings.TOKEN", "vault/api@3")));
    let found = catalog.validate(&build(true));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, FindingKind::EnvSecretOverlap);
    assert_eq!(found[0].field, "queue.settings.TOKEN");
}

/// Proves: the built-in catalog is the one `resource.proto` declares: two types, both
/// `PORTABLE`, read from the descriptors.
#[test]
fn the_built_in_catalog_is_read_from_resource_proto() {
    let builtin = Catalog::builtin();
    assert_eq!(builtin.kind_names(), ["service", "job"]);
    assert_eq!(builtin.portability("service"), Some(Portability::Portable));
    assert_eq!(builtin.portability("job"), Some(Portability::Portable));
}

// ---- what the descriptors cannot lower -------------------------------------------------------

/// The proto of a list whose one type is `types` (messages declared in the file, one of which
/// is `T`), with the standard resource around it.
fn case(types: &str) -> String {
    format!(
        r#"syntax = "proto3";
package case.v1;
import "ucdk/resource/v1/resource.proto";
{types}
message Resource {{
  string id = 1;
  repeated ucdk.resource.v1.Uses uses = 2;
  oneof body {{ T t = 10; }}
}}
message ResourceList {{ repeated Resource resource = 1; }}
"#
    )
}

const MARK: &str = "option (ucdk.resource.v1.portability) = PORTABLE;";

fn read_case(types: &str) -> Result<Catalog, CatalogError> {
    let pool = pool(vec![("case.proto".to_owned(), case(types))], "case.proto");
    Catalog::new(desc(&pool, "case.v1.ResourceList"))
}

fn bad_field(types: &str) -> (String, &'static str) {
    match read_case(types).err().expect("refused") {
        CatalogError::BadField { field, why, .. } => (field, why),
        other => panic!("expected a bad field, got {other:?}"),
    }
}

/// Proves: each way a field can fail to lower is refused when the catalog is read, naming the
/// field and the reason, instead of lowering to something the author did not mean.
#[test]
fn a_field_the_descriptors_cannot_lower_is_refused_when_the_catalog_is_read() {
    let ucdk = "ucdk.resource.v1";
    let cases: Vec<(String, &str, &str)> = vec![
        (
            format!("message T {{ {MARK} bool flag = 1; }}"),
            "flag",
            "the type of the field cannot be lowered",
        ),
        (
            format!("message T {{ {MARK} {ucdk}.Size s = 1; }}"),
            "s",
            "a message needs a `render` template",
        ),
        (
            format!("message T {{ {MARK} {ucdk}.Size s = 1 [({ucdk}.render) = \"{{nope}}\"]; }}"),
            "s",
            "the `render` template is malformed",
        ),
        (
            format!(
                "message T {{ {MARK} {ucdk}.Size s = 1 [({ucdk}.render) = \"{{cpu_millis\"]; }}"
            ),
            "s",
            "the `render` template is malformed",
        ),
        (
            format!("message T {{ {MARK} uint32 n = 1 [({ucdk}.fallback) = \"many\"]; }}"),
            "n",
            "the `fallback` is not an integer",
        ),
        (
            format!("message T {{ {MARK} repeated string l = 1 [({ucdk}.fallback) = \"x\"]; }}"),
            "l",
            "a `fallback` needs a field with one value",
        ),
        (
            format!("message T {{ {MARK} repeated uint32 l = 1; }}"),
            "l",
            "a list holds strings",
        ),
        (
            format!("message T {{ {MARK} map<int32, {ucdk}.Value> m = 1; }}"),
            "m",
            "a map is keyed by string",
        ),
        (
            format!("message T {{ {MARK} map<string, string> m = 1; }}"),
            "m",
            "a map holds `Value` or `SecretRef`",
        ),
        (
            format!("message T {{ {MARK} string s = 1 [({ucdk}.role) = \"r\"]; }}"),
            "s",
            "a role must be on a message field",
        ),
        (
            format!(
                "message R {{ repeated string xs = 1; }} message T {{ {MARK} R r = 1 [({ucdk}.role) = \"r\"]; }}"
            ),
            "r",
            "a role's message may hold only scalar fields",
        ),
    ];
    for (types, field, why) in cases {
        assert_eq!(bad_field(&types), (field.to_owned(), why), "{types}");
    }
}

/// Proves: a type without a portability marking, or with `UNSET`, is refused: the doc's "UNSET
/// is never a legal marking" is enforced at read time.
#[test]
fn a_type_without_a_portability_marking_is_refused() {
    for types in [
        "message T { string s = 1; }".to_owned(),
        "message T { option (ucdk.resource.v1.portability) = PORTABILITY_UNSET; string s = 1; }"
            .to_owned(),
    ] {
        assert_eq!(
            read_case(&types).err(),
            Some(CatalogError::Unmarked {
                kind: "t".to_owned()
            })
        );
    }
}

/// Proves: a message that is not shaped like a resource list is refused with the part that is
/// missing: no repeated message field, two of them, a resource without `id`, without `uses`,
/// without a `body`, and a body arm that is not a message.
#[test]
fn a_message_that_is_not_a_resource_list_is_refused() {
    let why = |source: &str, list: &str| {
        let pool = pool(
            vec![("shape.proto".to_owned(), source.to_owned())],
            "shape.proto",
        );
        match Catalog::new(desc(&pool, list)).err().expect("refused") {
            CatalogError::NotAList { why, .. } => why,
            other => panic!("expected NotAList, got {other:?}"),
        }
    };
    let head = r#"syntax = "proto3"; package shape.v1; import "ucdk/resource/v1/resource.proto";"#;
    let src = |body: &str| format!("{head} {body}");
    assert_eq!(
        why(&src("message R {} message L { R x = 1; }"), "shape.v1.L"),
        "it needs exactly one repeated message field"
    );
    assert_eq!(
        why(
            &src("message R {} message L { repeated R a = 1; repeated R b = 2; }"),
            "shape.v1.L"
        ),
        "it needs exactly one repeated message field"
    );
    for id in ["", "repeated string id = 1;", "uint32 id = 1;"] {
        let resource = format!(
            "message R {{ {id} repeated ucdk.resource.v1.Uses uses = 2; }} message L {{ repeated R r = 1; }}"
        );
        assert_eq!(
            why(&src(&resource), "shape.v1.L"),
            "a resource needs a string `id`"
        );
    }
    for uses in [
        "ucdk.resource.v1.Uses uses = 2;",
        "repeated string uses = 2;",
    ] {
        assert_eq!(
            why(
                &src(&format!(
                    "message R {{ string id = 1; {uses} }} message L {{ repeated R r = 1; }}"
                )),
                "shape.v1.L"
            ),
            "a resource needs `repeated Uses uses`"
        );
    }
    assert_eq!(
        why(
            &src("message R { string id = 1; } message L { repeated R r = 1; }"),
            "shape.v1.L"
        ),
        "a resource needs `repeated Uses uses`"
    );
    assert_eq!(
        why(
            &src(
                "message R { string id = 1; repeated ucdk.resource.v1.Uses uses = 2; } message L { repeated R r = 1; }"
            ),
            "shape.v1.L"
        ),
        "a resource needs a `body` oneof"
    );
    let arm = "message R { string id = 1; repeated ucdk.resource.v1.Uses uses = 2; oneof body { string s = 10; } } message L { repeated R r = 1; }";
    let pool = pool(vec![("shape.proto".to_owned(), src(arm))], "shape.proto");
    assert_eq!(
        Catalog::new(desc(&pool, "shape.v1.L")).err(),
        Some(CatalogError::BadField {
            kind: "s".to_owned(),
            field: "s".to_owned(),
            why: "a body arm must be a message"
        })
    );
}

/// Proves: a pool that does not define the catalog's options (it never imported
/// `resource.proto`) is refused by name rather than lowering with no information.
#[test]
fn a_pool_without_the_catalog_options_is_refused() {
    let source =
        r#"syntax = "proto3"; package bare.v1; message R {} message L { repeated R r = 1; }"#;
    let pool = pool(
        vec![("bare.proto".to_owned(), source.to_owned())],
        "bare.proto",
    );
    assert_eq!(
        Catalog::new(desc(&pool, "bare.v1.L")).err(),
        Some(CatalogError::MissingOption {
            name: "portability"
        })
    );
}

/// Proves: every catalog error reads as a sentence naming what it is about.
#[test]
fn each_catalog_error_reads_as_a_sentence() {
    let errors = [
        CatalogError::NotAList {
            message: "m.L".to_owned(),
            why: "no list",
        },
        CatalogError::MissingOption { name: "role" },
        CatalogError::Unmarked {
            kind: "queue".to_owned(),
        },
        CatalogError::BadField {
            kind: "queue".to_owned(),
            field: "x".to_owned(),
            why: "bad",
        },
    ];
    let shown: Vec<String> = errors.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        [
            "`m.L` is not a resource list: no list",
            "the option `role` is not defined",
            "type `queue` has no portability marking",
            "type `queue`, field `x`: bad",
        ]
    );
    let _: &dyn std::error::Error = &errors[0];
}

/// Proves: a list whose elements are not in the catalog's list message cannot be built by
/// accident: `new_list` is an instance of the catalog's own list message.
#[test]
fn new_list_is_an_instance_of_the_catalogs_list_message() {
    let catalog = queue_catalog();
    assert_eq!(
        catalog.new_list().descriptor().full_name(),
        "example.queue.v1.ResourceList"
    );
    assert!(catalog.validate(&catalog.new_list()).is_empty());
}
