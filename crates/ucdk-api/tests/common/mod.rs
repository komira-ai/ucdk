// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]
//! Helpers shared by the API tests: a resource list, and a driver that runs the engine over the
//! fake provider and an in-memory ledger.

use ucdk_engine::{Engine, Mode, Outcome, Request};
use ucdk_ledger::MemoryLedger;
use ucdk_plan::{Lowered, Plan, lower};
use ucdk_proto::v1::resource::Body;
use ucdk_proto::v1::{Image, Resource, ResourceList, Service, image};
use ucdk_provider::{NodePath, kit};
use ucdk_provider_fake::Fake;

pub fn service_of(id: &str, port: u32) -> Resource {
    Resource {
        id: id.to_owned(),
        body: Some(Body::Service(Service {
            image: Some(Image {
                source: Some(image::Source::Digest("sha256:aa".to_owned())),
                platform: String::new(),
            }),
            port,
            ..Service::default()
        })),
        uses: Vec::new(),
    }
}

pub fn list_of(resources: Vec<Resource>) -> ResourceList {
    ResourceList {
        resource: resources,
    }
}

pub fn lowered(list: &ResourceList) -> Lowered {
    lower(list).expect("the list lowers")
}

pub fn path(text: &str) -> NodePath {
    NodePath::new(text).expect("a valid path")
}

/// A fake provider and a ledger, which a run reads and writes.
pub struct World {
    pub fake: Fake,
    pub ledger: MemoryLedger,
}

impl World {
    pub fn new() -> Self {
        Self {
            fake: Fake::standard(),
            ledger: MemoryLedger::new(),
        }
    }

    pub fn handle(&mut self, request: Request) -> Outcome {
        Engine::new(&mut self.fake, &self.ledger, kit::cell()).handle(request)
    }

    pub fn plan(&mut self, desired: &Lowered, mode: Mode) -> Plan {
        match self.handle(Request::Plan {
            desired: desired.clone(),
            mode,
        }) {
            Outcome::Planned(plan) => plan,
            other => panic!("not planned: {other:?}"),
        }
    }
}
