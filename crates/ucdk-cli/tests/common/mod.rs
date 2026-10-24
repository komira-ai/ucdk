// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]
//! Helpers shared by the CLI tests: resource lists written as binary files, a way to run one
//! command over a world, and the plan and apply steps an approver takes.

use std::fs;
use std::path::PathBuf;

use prost::Message as _;
use ucdk_cli::{World, run};
use ucdk_ledger::{Ledger as _, Record};
use ucdk_proto::v1::resource::Body;
use ucdk_proto::v1::{
    Access, Image, Job, Ref, Resource, ResourceList, Service, Uses, image, service,
};
use ucdk_provider::NodePath;
use ucdk_provider_fake::Fake;

// ---- resource lists --------------------------------------------------------------------------

fn image() -> Option<Image> {
    Some(Image {
        source: Some(image::Source::Digest("sha256:aa".to_owned())),
        platform: String::new(),
    })
}

/// A service; `public` turns its public role on and `uses` names the services it calls.
pub fn service_of(id: &str, public: bool, uses: &[&str]) -> Resource {
    Resource {
        id: id.to_owned(),
        body: Some(Body::Service(Service {
            image: image(),
            port: 8080,
            exposure: public.then_some(service::Exposure::Public(service::Public {})),
            ..Service::default()
        })),
        uses: uses
            .iter()
            .map(|target| Uses {
                target: Some(Ref {
                    resource: (*target).to_owned(),
                    output: None,
                }),
                access: Access::Call as i32,
            })
            .collect(),
    }
}

/// `resource`, a service, listening on `port`.
pub fn with_port(mut resource: Resource, port: u32) -> Resource {
    if let Some(Body::Service(service)) = &mut resource.body {
        service.port = port;
    }
    resource
}

pub fn job_of(id: &str) -> Resource {
    Resource {
        id: id.to_owned(),
        body: Some(Body::Job(Job {
            image: image(),
            ..Job::default()
        })),
        uses: Vec::new(),
    }
}

/// `example-svc` (public) calls `example-db`.
pub fn pair() -> Vec<Resource> {
    vec![
        service_of("example-svc", true, &["example-db"]),
        service_of("example-db", false, &[]),
    ]
}

// ---- files -----------------------------------------------------------------------------------

/// A directory of the test's own under the build's scratch directory.
pub fn scratch(test: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("ucdk-cli")
        .join(test);
    fs::create_dir_all(&dir).expect("create the test directory");
    dir
}

/// Writes `resources` as a binary `ResourceList` and returns the path.
pub fn write_list(dir: &std::path::Path, name: &str, resources: Vec<Resource>) -> String {
    let path = dir.join(name);
    let list = ResourceList {
        resource: resources,
    };
    fs::write(&path, list.encode_to_vec()).expect("write the list");
    path.to_string_lossy().into_owned()
}

pub fn write_text(dir: &std::path::Path, name: &str, text: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, text).expect("write the file");
    path.to_string_lossy().into_owned()
}

// ---- running a command -----------------------------------------------------------------------

#[derive(Debug)]
pub struct Ran {
    pub code: u8,
    pub out: String,
    pub err: String,
}

/// Runs one command over `world`, returning only what that command printed.
pub fn cmd(world: &mut World, args: &[&str]) -> Ran {
    let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
    let code = run(&args, world);
    Ran {
        code,
        out: std::mem::take(&mut world.out),
        err: std::mem::take(&mut world.err),
    }
}

/// `plan` of `input`, saved to `<dir>/<name>`; returns the run and the hash it printed.
pub fn plan(
    world: &mut World,
    provider: &str,
    mode: &str,
    input: &str,
    saved: &str,
) -> (Ran, String) {
    let ran = cmd(
        world,
        &[
            "plan",
            &format!("--provider={provider}"),
            &format!("--mode={mode}"),
            &format!("--input={input}"),
            &format!("--out={saved}"),
        ],
    );
    let hash = ran
        .out
        .lines()
        .find_map(|l| l.strip_prefix("plan-hash: "))
        .unwrap_or_default()
        .to_owned();
    (ran, hash)
}

/// `apply` or `destroy` of `input` under the saved plan and the hash pinned for it.
pub fn carry_out(
    world: &mut World,
    verb: &str,
    provider: &str,
    input: &str,
    saved: &str,
    hash: &str,
) -> Ran {
    cmd(
        world,
        &[
            verb,
            &format!("--provider={provider}"),
            &format!("--input={input}"),
            &format!("--plan={saved}"),
            &format!("--expect-plan-hash={hash}"),
        ],
    )
}

/// What an approver does: plan, read the hash, and run the verb under it.
pub fn approve_and_run(
    world: &mut World,
    verb: &str,
    provider: &str,
    input: &str,
    dir: &std::path::Path,
) -> Ran {
    let mode = if verb == "destroy" {
        "destroy"
    } else {
        "apply"
    };
    let saved = dir
        .join(format!("{verb}.plan"))
        .to_string_lossy()
        .into_owned();
    let (planned, hash) = plan(world, provider, mode, input, &saved);
    assert_eq!(planned.code, 0, "the plan is made: {planned:?}");
    carry_out(world, verb, provider, input, &saved, &hash)
}

// ---- looking at the world --------------------------------------------------------------------

/// The mutating calls the fake has logged so far.
pub fn mutations(fake: &Fake) -> usize {
    fake.calls().iter().filter(|c| c.op.mutates()).count()
}

/// The objects of the fake, one per line, without the call log.
pub fn objects(fake: &Fake) -> String {
    let dump = fake.dump();
    let end = dump.find("calls:").expect("the dump has a call section");
    dump[..end].to_owned()
}

pub fn records(world: &World) -> Vec<(NodePath, Record)> {
    world
        .ledger
        .read()
        .expect("read the ledger")
        .records
        .into_iter()
        .collect()
}

pub fn recorded_paths(world: &World) -> Vec<String> {
    records(world)
        .into_iter()
        .map(|(path, _)| path.as_str().to_owned())
        .collect()
}
