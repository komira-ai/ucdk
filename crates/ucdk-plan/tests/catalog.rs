// SPDX-License-Identifier: Apache-2.0
//! The catalog: what a resource list is refused for, and what it lowers to.

use std::collections::BTreeMap;

use prost_types::Duration;
use ucdk_plan::catalog::{ROLE_PUBLIC, ROLE_RUN, ROLE_SCHEDULE};
use ucdk_plan::{
    Finding, FindingKind, Lowered, ValueRef, lower, validate, validate_with_portability,
};
use ucdk_proto::v1::r#ref::Output as Arm;
use ucdk_proto::v1::resource::Body;
use ucdk_proto::v1::{
    Access, Image, Job, Output, Portability, Ref, Resource, ResourceList, Scale, SecretRef,
    Service, Size, StepOutput, Uses, Value, image, job, service, value,
};
use ucdk_provider::FieldValue;

// ---- builders --------------------------------------------------------------------------------

fn digest_image(platform: &str) -> Option<Image> {
    Some(Image {
        source: Some(image::Source::Digest("sha256:aa".to_owned())),
        platform: platform.to_owned(),
    })
}

fn service_with(id: &str, svc: Service) -> Resource {
    Resource {
        id: id.to_owned(),
        body: Some(Body::Service(svc)),
        ..Resource::default()
    }
}

fn job_with(id: &str, j: Job) -> Resource {
    Resource {
        id: id.to_owned(),
        body: Some(Body::Job(j)),
        ..Resource::default()
    }
}

fn service_of(id: &str) -> Resource {
    service_with(
        id,
        Service {
            image: digest_image(""),
            ..Service::default()
        },
    )
}

fn job_of(id: &str) -> Resource {
    job_with(
        id,
        Job {
            image: digest_image(""),
            ..Job::default()
        },
    )
}

fn list(resource: Vec<Resource>) -> ResourceList {
    ResourceList { resource }
}

fn literal(text: &str) -> Value {
    Value {
        v: Some(value::V::Literal(text.to_owned())),
    }
}

fn reference(target: &str, output: Option<Arm>) -> Ref {
    Ref {
        resource: target.to_owned(),
        output,
    }
}

fn output_of(target: &str, output: Output) -> Value {
    Value {
        v: Some(value::V::Ref(reference(
            target,
            Some(Arm::Standard(output as i32)),
        ))),
    }
}

fn uses(target: &str) -> Uses {
    Uses {
        target: Some(reference(target, None)),
        access: Access::Call as i32,
    }
}

/// A service with one environment value.
fn service_env(id: &str, key: &str, v: Value) -> Resource {
    service_with(
        id,
        Service {
            image: digest_image(""),
            env: BTreeMap::from([(key.to_owned(), v)]),
            ..Service::default()
        },
    )
}

fn findings(resources: Vec<Resource>) -> Vec<Finding> {
    validate(&list(resources))
}

fn kinds(resources: Vec<Resource>) -> Vec<FindingKind> {
    findings(resources).into_iter().map(|f| f.kind).collect()
}

fn lowered(resources: Vec<Resource>) -> Lowered {
    lower(&list(resources)).expect("the list lowers")
}

fn paths(l: &Lowered) -> Vec<&str> {
    l.nodes().iter().map(|n| n.node.path().as_str()).collect()
}

fn fields(l: &Lowered, path: &str) -> Vec<(String, FieldValue)> {
    let node = l
        .nodes()
        .iter()
        .find(|n| n.node.path().as_str() == path)
        .expect("the node is there");
    node.node
        .fields()
        .iter()
        .map(|f| (f.name.clone(), f.value.clone()))
        .collect()
}

fn text(name: &str, value: &str) -> (String, FieldValue) {
    (name.to_owned(), FieldValue::Text(value.to_owned()))
}

fn int(name: &str, value: i64) -> (String, FieldValue) {
    (name.to_owned(), FieldValue::Int(value))
}

// ---- Lowering: roles -------------------------------------------------------------------------

#[test]
fn lower_service_emits_fixed_role_set() {
    let l = lowered(vec![service_of("web")]);
    assert_eq!(paths(&l), ["web/run", "web/public"]);
    let kinds: Vec<&str> = l.nodes().iter().map(|n| n.node.kind().as_str()).collect();
    assert_eq!(kinds, ["service", "public"]);
    let names: Vec<&str> = l.nodes().iter().map(|n| n.node.name()).collect();
    assert_eq!(names, ["web-run", "web-public"]);
    assert_eq!(ROLE_RUN, "run");
    assert_eq!(ROLE_PUBLIC, "public");
}

#[test]
fn lower_job_emits_fixed_role_set() {
    let l = lowered(vec![job_of("nightly")]);
    assert_eq!(paths(&l), ["nightly/run", "nightly/schedule"]);
    let kinds: Vec<&str> = l.nodes().iter().map(|n| n.node.kind().as_str()).collect();
    assert_eq!(kinds, ["job", "schedule"]);
    assert_eq!(ROLE_SCHEDULE, "schedule");
}

#[test]
fn lower_emits_the_same_roles_whatever_the_file_turns_on() {
    let public = service_with(
        "a",
        Service {
            image: digest_image(""),
            exposure: Some(service::Exposure::Public(service::Public {})),
            ..Service::default()
        },
    );
    let scheduled = job_with(
        "b",
        Job {
            image: digest_image(""),
            trigger: Some(job::Trigger::Schedule(job::Schedule::default())),
            ..Job::default()
        },
    );
    assert_eq!(
        paths(&lowered(vec![public, scheduled])),
        ["a/run", "a/public", "b/run", "b/schedule"]
    );
}

#[test]
fn turned_off_role_lowers_to_unwanted() {
    let wanted = |resources| -> Vec<bool> {
        lowered(resources)
            .nodes()
            .iter()
            .map(|n| n.wanted)
            .collect()
    };
    let exposure = |e| {
        service_with(
            "web",
            Service {
                image: digest_image(""),
                exposure: e,
                ..Service::default()
            },
        )
    };
    let trigger = |t| {
        job_with(
            "j",
            Job {
                image: digest_image(""),
                trigger: t,
                ..Job::default()
            },
        )
    };
    assert_eq!(wanted(vec![exposure(None)]), [true, false]);
    let internal = service::Exposure::Internal(service::Internal {});
    assert_eq!(wanted(vec![exposure(Some(internal))]), [true, false]);
    let public = service::Exposure::Public(service::Public {});
    assert_eq!(wanted(vec![exposure(Some(public))]), [true, true]);
    assert_eq!(wanted(vec![trigger(None)]), [true, false]);
    let on_demand = job::Trigger::OnDemand(job::OnDemand {});
    assert_eq!(wanted(vec![trigger(Some(on_demand))]), [true, false]);
    let schedule = job::Trigger::Schedule(job::Schedule {
        cron: "0 3 * * *".to_owned(),
        timezone: "UTC".to_owned(),
    });
    let l = lowered(vec![trigger(Some(schedule))]);
    assert_eq!(
        l.nodes().iter().map(|n| n.wanted).collect::<Vec<_>>(),
        [true, true]
    );
    assert_eq!(
        fields(&l, "j/schedule"),
        [text("cron", "0 3 * * *"), text("timezone", "UTC")]
    );
}

// ---- Lowering: fields ------------------------------------------------------------------------

#[test]
fn lowering_writes_the_catalog_defaults_where_the_author_wrote_none() {
    let l = lowered(vec![service_of("web"), job_of("j")]);
    assert_eq!(
        fields(&l, "web/run"),
        [
            text("health_path", ""),
            text("image", "sha256:aa@linux/amd64"),
            int("max_concurrency", 0),
            int("port", 8080),
            text("request_timeout", "60s0n"),
            text("scale", "0..10"),
            text("size", "1000m/512MB"),
        ]
    );
    assert!(fields(&l, "web/public").is_empty());
    assert_eq!(
        fields(&l, "j/run"),
        [
            text("image", "sha256:aa@linux/amd64"),
            int("max_retries", 0),
            text("size", "1000m/512MB"),
            text("timeout", "600s0n"),
        ]
    );
    assert!(fields(&l, "j/schedule").is_empty());
}

#[test]
fn lowering_carries_what_the_author_wrote() {
    let duration = |seconds, nanos| Duration { seconds, nanos };
    let svc = Service {
        image: digest_image("linux/amd64"),
        port: 9000,
        args: vec!["--a".to_owned(), "--b".to_owned()],
        env: BTreeMap::from([
            ("LEVEL".to_owned(), literal("debug")),
            ("DB".to_owned(), output_of("db", Output::Url)),
            ("DB_HOST".to_owned(), output_of("db", Output::Host)),
        ]),
        size: Some(Size {
            cpu_millis: 250,
            memory_mb: 128,
        }),
        scale: Some(Scale {
            min: Some(1),
            max: 3,
        }),
        health_path: "/healthz".to_owned(),
        request_timeout: Some(duration(5, 7)),
        max_concurrency: 40,
        secret_env: BTreeMap::from([
            (
                "PLAIN".to_owned(),
                SecretRef {
                    name: "plain".to_owned(),
                    ..SecretRef::default()
                },
            ),
            (
                "FULL".to_owned(),
                SecretRef {
                    name: "full".to_owned(),
                    store: Some("vault".to_owned()),
                    version: Some("3".to_owned()),
                },
            ),
        ]),
        ..Service::default()
    };
    let l = lowered(vec![service_with("web", svc), service_of("db")]);
    assert_eq!(
        fields(&l, "web/run"),
        [
            text("arg.0", "--a"),
            text("arg.1", "--b"),
            text("env.LEVEL", "debug"),
            text("health_path", "/healthz"),
            text("image", "sha256:aa@linux/amd64"),
            int("max_concurrency", 40),
            int("port", 9000),
            text("request_timeout", "5s7n"),
            text("scale", "1..3"),
            text("secret_env.FULL", "vault/full@3"),
            text("secret_env.PLAIN", "plain"),
            text("size", "250m/128MB"),
        ]
    );
}

#[test]
fn value_reference_lowers_to_a_ref_and_not_to_a_field() {
    let l = lowered(vec![
        service_env("web", "DB", output_of("db", Output::Url)),
        service_of("db"),
    ]);
    let web = l
        .nodes()
        .iter()
        .find(|n| n.node.path().as_str() == "web/run")
        .expect("the node is there");
    assert_eq!(
        web.refs,
        [ValueRef {
            field: "env.DB".to_owned(),
            resource: "db".to_owned(),
            output: Output::Url,
        }]
    );
    assert!(web.node.fields().iter().all(|f| f.name != "env.DB"));
}

#[test]
fn literal_that_reads_like_a_reference_is_only_a_literal() {
    let l = lowered(vec![
        service_env("web", "DB", literal("ref:db.URL")),
        service_of("db"),
    ]);
    let web = l
        .nodes()
        .iter()
        .find(|n| n.node.path().as_str() == "web/run")
        .expect("the node is there");
    assert!(web.refs.is_empty());
    assert_eq!(fields(&l, "web/run")[0], text("env.DB", "ref:db.URL"));
}

#[test]
fn bound_node_has_the_referenced_value_as_a_field() {
    let l = lowered(vec![
        service_env("web", "DB", output_of("db", Output::Url)),
        service_of("db"),
    ]);
    let web = l
        .nodes()
        .iter()
        .find(|n| n.node.path().as_str() == "web/run")
        .expect("the node is there");
    let known = web
        .bound(|r, o| (r == "db" && o == "URL").then_some("db-url-1"))
        .expect("every output is known");
    assert!(
        known
            .fields()
            .iter()
            .any(|f| f.name == "env.DB" && f.value == FieldValue::Text("db-url-1".to_owned()))
    );
    let pending = web.bound(|_, _| None).expect_err("no output is known");
    assert_eq!(pending, [&web.refs[0]]);
}

#[test]
fn lowering_carries_a_jobs_own_values() {
    let j = Job {
        image: digest_image(""),
        size: Some(Size {
            cpu_millis: 500,
            memory_mb: 64,
        }),
        max_retries: Some(2),
        timeout: Some(Duration {
            seconds: 30,
            nanos: 0,
        }),
        ..Job::default()
    };
    let l = lowered(vec![job_with("j", j)]);
    assert_eq!(
        fields(&l, "j/run"),
        [
            text("image", "sha256:aa@linux/amd64"),
            int("max_retries", 2),
            text("size", "500m/64MB"),
            text("timeout", "30s0n"),
        ]
    );
}

#[test]
fn a_scale_with_no_written_minimum_starts_at_zero() {
    let svc = Service {
        image: digest_image(""),
        scale: Some(Scale { min: None, max: 4 }),
        ..Service::default()
    };
    let l = lowered(vec![service_with("web", svc)]);
    assert!(fields(&l, "web/run").contains(&text("scale", "0..4")));
}

// ---- Lowering: order -------------------------------------------------------------------------

fn edges(l: &Lowered) -> Vec<(String, Vec<String>)> {
    l.nodes()
        .iter()
        .map(|n| {
            let deps = n.deps.iter().map(|d| d.as_str().to_owned()).collect();
            (n.node.path().as_str().to_owned(), deps)
        })
        .collect()
}

#[test]
fn lowering_orders_nodes_by_what_they_need() {
    // `app` calls `api` and reads `db`'s URL; `api` reads `db`'s host. The list is given
    // dependents first.
    let mut app = service_env("app", "DB", output_of("db", Output::Url));
    app.uses.push(uses("api"));
    let api = service_env("api", "DB_HOST", output_of("db", Output::Host));
    let l = lowered(vec![app, api, service_of("db")]);
    assert_eq!(
        edges(&l),
        [
            ("db/run".to_owned(), vec![]),
            ("db/public".to_owned(), vec!["db/run".to_owned()]),
            ("api/run".to_owned(), vec!["db/run".to_owned()]),
            ("api/public".to_owned(), vec!["api/run".to_owned()]),
            (
                "app/run".to_owned(),
                vec!["api/run".to_owned(), "db/run".to_owned()]
            ),
            ("app/public".to_owned(), vec!["app/run".to_owned()]),
        ]
    );
}

#[test]
fn lowering_does_not_depend_on_the_order_of_the_list() {
    let a = service_env("a", "X", output_of("b", Output::Url));
    let b = service_of("b");
    let c = job_of("c");
    let forward = lowered(vec![a.clone(), b.clone(), c.clone()]);
    assert_eq!(lowered(vec![c.clone(), b.clone(), a.clone()]), forward);
    assert_eq!(lowered(vec![b, c, a]), forward);
}

#[test]
fn an_empty_list_lowers_to_no_nodes() {
    assert!(lowered(vec![]).nodes().is_empty());
}

// ---- Refusals: collected, not first-only -----------------------------------------------------

fn finding(resource: &str, field: &str, kind: FindingKind) -> Finding {
    Finding {
        resource: resource.to_owned(),
        field: field.to_owned(),
        kind,
    }
}

#[test]
fn a_good_list_has_no_findings() {
    let mut app = service_env("app", "DB", output_of("db", Output::Url));
    app.uses.push(uses("db"));
    assert_eq!(findings(vec![app, service_of("db"), job_of("j")]), []);
}

#[test]
fn unbound_ref_refused_collecting_all_findings() {
    let mut a = service_env("a", "X", output_of("ghost", Output::Url));
    a.uses.push(uses("phantom"));
    let b = service_env("b", "Y", output_of("ghost", Output::Host));
    let mut broken = service_of("c");
    broken.body = None;
    let all = findings(vec![
        a,
        b,
        broken,
        service_of("Bad_Id"),
        service_of("d"),
        service_of("d"),
    ]);
    assert_eq!(
        all,
        [
            finding("c", "", FindingKind::MissingBody),
            finding("Bad_Id", "", FindingKind::BadId),
            finding("d", "", FindingKind::DuplicateId),
            finding(
                "a",
                "service.env.X",
                FindingKind::UnboundRef {
                    target: "ghost".to_owned()
                }
            ),
            finding(
                "a",
                "uses[0]",
                FindingKind::UnboundRef {
                    target: "phantom".to_owned()
                }
            ),
            finding(
                "b",
                "service.env.Y",
                FindingKind::UnboundRef {
                    target: "ghost".to_owned()
                }
            ),
        ]
    );
    // Lowering refuses with the same findings and produces no node.
    let refused = lower(&list(vec![service_env(
        "a",
        "X",
        output_of("ghost", Output::Url),
    )]));
    assert_eq!(refused.unwrap_err().len(), 1);
}

#[test]
fn a_reference_to_a_resource_that_is_not_a_service_or_job_is_still_bound() {
    let mut empty = service_of("c");
    empty.body = None;
    let user = service_env("a", "X", output_of("c", Output::Url));
    let all = kinds(vec![user, empty]);
    assert_eq!(
        all,
        [
            FindingKind::MissingBody,
            FindingKind::UnexposedOutput {
                target: "c".to_owned(),
                output: "URL"
            },
        ]
    );
}

#[test]
fn a_resource_id_must_be_one_path_segment() {
    for bad in ["", "A", "1a", "a/b", "a b", &"a".repeat(33)] {
        assert_eq!(
            kinds(vec![service_of(bad)]),
            [FindingKind::BadId],
            "{bad:?}"
        );
    }
    assert_eq!(
        kinds(vec![service_of(&"a".repeat(32)), service_of("a-1")]),
        []
    );
}

#[test]
fn cycle_is_refused_at_load() {
    // a and b reference each other; c depends on a; d stands alone.
    let a = service_env("a", "X", output_of("b", Output::Url));
    let mut b = service_of("b");
    b.uses.push(uses("a"));
    let c = service_env("c", "X", output_of("a", Output::Url));
    let resources = vec![service_of("d"), c, b, a];
    let members = vec!["a".to_owned(), "b".to_owned(), "c".to_owned()];
    let expected = [finding("a", "", FindingKind::Cycle { members })];
    assert_eq!(findings(resources.clone()), expected);
    // Lowering refuses the same list with the same findings, and yields no node.
    assert_eq!(lower(&list(resources)).unwrap_err(), expected);
}

#[test]
fn a_resource_that_uses_itself_is_a_cycle() {
    let mut a = service_of("a");
    a.uses.push(uses("a"));
    assert_eq!(
        kinds(vec![a]),
        [FindingKind::Cycle {
            members: vec!["a".to_owned()]
        }]
    );
}

#[test]
fn a_cycle_is_reported_together_with_the_other_findings() {
    let mut a = service_of("a");
    a.uses.push(uses("a"));
    a.uses.push(uses("ghost"));
    let found = kinds(vec![a]);
    assert_eq!(found.len(), 2);
    assert!(matches!(found[0], FindingKind::UnboundRef { .. }));
    assert!(matches!(found[1], FindingKind::Cycle { .. }));
}

// ---- Refusals: what each field is checked for ------------------------------------------------

#[test]
fn an_image_needs_a_digest_on_the_supported_platform() {
    let with = |image| {
        service_with(
            "web",
            Service {
                image,
                ..Service::default()
            },
        )
    };
    let source = |s| {
        Some(Image {
            source: s,
            platform: String::new(),
        })
    };
    assert_eq!(kinds(vec![with(None)]), [FindingKind::MissingImage]);
    assert_eq!(kinds(vec![with(source(None))]), [FindingKind::MissingImage]);
    let empty = image::Source::Digest(String::new());
    assert_eq!(
        kinds(vec![with(source(Some(empty)))]),
        [FindingKind::MissingImage]
    );
    let step = image::Source::Output(StepOutput {
        step: "build".to_owned(),
        name: "img".to_owned(),
    });
    assert_eq!(
        kinds(vec![with(source(Some(step)))]),
        [FindingKind::Unresolved {
            what: "build step output"
        }]
    );
    assert_eq!(
        kinds(vec![with(digest_image("linux/arm64"))]),
        [FindingKind::UnsupportedPlatform {
            platform: "linux/arm64".to_owned()
        }]
    );
    assert_eq!(kinds(vec![with(digest_image("linux/amd64"))]), []);
}

#[test]
fn a_job_is_checked_like_a_service() {
    let j = job_with("j", Job::default());
    let found = findings(vec![j]);
    assert_eq!(
        found,
        [finding("j", "job.image", FindingKind::MissingImage)]
    );
}

#[test]
fn a_value_needs_an_arm_and_a_resolved_one() {
    let param = Value {
        v: Some(value::V::Param("region".to_owned())),
    };
    assert_eq!(
        kinds(vec![service_env("a", "K", param)]),
        [FindingKind::Unresolved { what: "parameter" }]
    );
    assert_eq!(
        kinds(vec![service_env("a", "K", Value { v: None })]),
        [FindingKind::EmptyValue]
    );
}

#[test]
fn a_key_in_env_and_secret_env_is_refused() {
    let svc = Service {
        image: digest_image(""),
        env: BTreeMap::from([("K".to_owned(), literal("v"))]),
        secret_env: BTreeMap::from([("K".to_owned(), SecretRef::default())]),
        ..Service::default()
    };
    assert_eq!(
        findings(vec![service_with("a", svc)]),
        [finding("a", "service.env.K", FindingKind::EnvSecretOverlap)]
    );
}

#[test]
fn a_uses_needs_call_access_and_a_resource_target() {
    let target = |output| Uses {
        target: Some(reference("b", output)),
        access: Access::Call as i32,
    };
    let one = |u: Uses| {
        let mut a = service_of("a");
        a.uses.push(u);
        kinds(vec![a, service_of("b")])
    };
    assert_eq!(one(target(None)), []);
    assert_eq!(
        one(Uses {
            target: None,
            access: Access::Call as i32
        }),
        [FindingKind::NoTarget]
    );
    assert_eq!(
        one(Uses {
            target: Some(reference("b", None)),
            access: Access::Unset as i32
        }),
        [FindingKind::UnsupportedAccess]
    );
    assert_eq!(
        one(target(Some(Arm::Standard(Output::Url as i32)))),
        [FindingKind::AccessToOutput]
    );
}

#[test]
fn a_value_reference_names_an_output_its_target_exposes() {
    let one = |target: &str, output: Option<Arm>| {
        let v = Value {
            v: Some(value::V::Ref(reference(target, output))),
        };
        kinds(vec![
            service_env("a", "K", v),
            service_of("svc"),
            job_of("job"),
        ])
    };
    let standard = |o: Output| Some(Arm::Standard(o as i32));
    assert_eq!(one("svc", standard(Output::Url)), []);
    assert_eq!(one("svc", standard(Output::Host)), []);
    assert_eq!(one("svc", None), [FindingKind::MissingOutput]);
    assert_eq!(
        one("svc", standard(Output::Unset)),
        [FindingKind::MissingOutput]
    );
    assert_eq!(
        one("svc", Some(Arm::Standard(99))),
        [FindingKind::MissingOutput]
    );
    assert_eq!(
        one("svc", Some(Arm::Named("x".to_owned()))),
        [FindingKind::NamedOutput]
    );
    assert_eq!(
        one("job", standard(Output::Url)),
        [FindingKind::UnexposedOutput {
            target: "job".to_owned(),
            output: "URL"
        }]
    );
}

#[test]
fn non_portable_cross_node_ref_refused() {
    // A catalog that marks `job` as bound to one cloud: another resource may not use it or
    // read its values, and the finding names the field that does.
    let bound_job = |name: &str| {
        if name == "job" {
            Portability::CloudBound
        } else {
            Portability::Portable
        }
    };
    let resources = vec![
        service_with(
            "web",
            Service {
                image: digest_image(""),
                ..Service::default()
            },
        ),
        job_of("batch"),
    ];
    let mut with_use = resources.clone();
    with_use[0].uses.push(uses("batch"));
    let found = validate_with_portability(&list(with_use), bound_job);
    assert_eq!(
        found,
        [finding(
            "web",
            "uses[0]",
            FindingKind::NotPortable {
                target: "batch".to_owned()
            }
        )]
    );
    // A value reference to it is refused the same way, once, and no second finding follows.
    let mut with_ref = resources.clone();
    with_ref[0] = service_env("web", "K", output_of("batch", Output::Url));
    let found = validate_with_portability(&list(with_ref), bound_job);
    assert_eq!(
        found,
        [finding(
            "web",
            "service.env.K",
            FindingKind::NotPortable {
                target: "batch".to_owned()
            }
        )]
    );
    // The same lists are fine for a portable type, and for the catalog as it is.
    let mut portable = resources;
    portable[0].uses.push(uses("batch"));
    assert_eq!(validate(&list(portable.clone())), []);
    assert_eq!(
        validate_with_portability(&list(portable), |_| Portability::Portable),
        []
    );
}

// ---- Findings read as text -------------------------------------------------------------------

#[test]
fn each_finding_reads_as_a_sentence_naming_its_resource() {
    let all = [
        FindingKind::BadId,
        FindingKind::DuplicateId,
        FindingKind::MissingBody,
        FindingKind::MissingImage,
        FindingKind::Unresolved { what: "parameter" },
        FindingKind::UnsupportedPlatform {
            platform: "p".to_owned(),
        },
        FindingKind::EmptyValue,
        FindingKind::EnvSecretOverlap,
        FindingKind::UnboundRef {
            target: "t".to_owned(),
        },
        FindingKind::NoTarget,
        FindingKind::UnsupportedAccess,
        FindingKind::AccessToOutput,
        FindingKind::MissingOutput,
        FindingKind::NamedOutput,
        FindingKind::UnexposedOutput {
            target: "t".to_owned(),
            output: "URL",
        },
        FindingKind::NotPortable {
            target: "t".to_owned(),
        },
        FindingKind::Cycle {
            members: vec!["a".to_owned(), "b".to_owned()],
        },
    ];
    let mut seen = std::collections::BTreeSet::new();
    for kind in all {
        let whole = finding("res", "", kind.clone()).to_string();
        assert!(whole.starts_with("resource `res`: "), "{whole}");
        let with_field = finding("res", "uses[0]", kind).to_string();
        assert!(
            with_field.starts_with("resource `res`, uses[0]: "),
            "{with_field}"
        );
        assert!(seen.insert(whole), "two findings read the same");
    }
}

#[test]
fn unwanted_view_keeps_every_node_and_wants_none() {
    let l = lowered(vec![service_of("a"), service_of("b")]);
    assert!(l.nodes().iter().any(|n| n.wanted));
    let view = l.unwanted();
    assert_eq!(paths(&view), paths(&l));
    assert!(view.nodes().iter().all(|n| !n.wanted));
    assert!(
        l.nodes().iter().any(|n| n.wanted),
        "the original is unchanged"
    );
}
