// SPDX-License-Identifier: Apache-2.0
/** The typed resource templates: what they ask for, and the wire messages they make. */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import * as gp from "../src/generated/google_protobuf.ts";
import * as r from "../src/generated/resources.ts";
import * as pb from "../src/generated/ucdk_resource_v1.ts";

const IMAGE = pb.Image.digest("sha256:aa");

test("a service needs an id and an image", () => {
  assert.throws(
    // @ts-expect-error the image is required; a JavaScript caller gets the error at run time
    () => new r.Service("web"),
    { name: "TypeError", message: "a Service needs `image`" },
  );
  assert.throws(
    // @ts-expect-error the same for a job
    () => new r.Job("nightly"),
    { name: "TypeError", message: "a Job needs `image`" },
  );
});

test("the resource it makes", () => {
  const service = new r.Service("web", IMAGE, {
    port: 8080,
    args: ["--x"],
    size: new pb.Size({ cpuMillis: 500, memoryMb: 256 }),
    scale: new pb.Scale({ min: 0, max: 3 }),
    healthPath: "/up",
    requestTimeout: new gp.Duration({ seconds: 5n }),
    maxConcurrency: 10,
    secretEnv: { S: new pb.SecretRef({ name: "sec" }) },
    exposure: new r.Service.Public(),
  });
  assert.deepEqual(
    service.toResource(),
    new pb.Resource({
      id: "web",
      service: new pb.Service({
        image: IMAGE,
        port: 8080,
        args: ["--x"],
        size: new pb.Size({ cpuMillis: 500, memoryMb: 256 }),
        scale: new pb.Scale({ min: 0, max: 3 }),
        healthPath: "/up",
        requestTimeout: new gp.Duration({ seconds: 5n }),
        maxConcurrency: 10,
        secretEnv: { S: new pb.SecretRef({ name: "sec" }) },
        public: new pb.Service_Public(),
      }),
    }),
  );
});

test("a job and its trigger", () => {
  const job = new r.Job("nightly", IMAGE, {
    maxRetries: 0,
    trigger: new r.Job.Schedule({ cron: "0 * * * *" }),
  });
  assert.deepEqual(
    job.toResource(),
    new pb.Resource({
      id: "nightly",
      job: new pb.Job({
        image: IMAGE,
        maxRetries: 0,
        schedule: new pb.Job_Schedule({ cron: "0 * * * *" }),
      }),
    }),
  );
  const onDemand = new r.Job("run", IMAGE, { trigger: new r.Job.OnDemand() });
  assert.equal(onDemand.body.whichTrigger(), "onDemand");
  assert.equal(new r.Job("plain", IMAGE).body.whichTrigger(), undefined);
});

test("a oneof takes only its own arms", () => {
  assert.throws(
    // @ts-expect-error a job's trigger is not a service's exposure
    () => new r.Service("web", IMAGE, { exposure: new r.Job.OnDemand() }),
    { name: "TypeError", message: "exposure must be one of Public, Internal" },
  );
  assert.throws(
    // @ts-expect-error nor is a string
    () => new r.Service("web", IMAGE, { exposure: "public" }),
    TypeError,
  );
});

test("what a kind is", () => {
  assert.equal(r.Service.KIND, "service");
  assert.equal(r.Job.KIND, "job");
  assert.equal(r.Service.PORTABILITY, pb.Portability.PORTABLE);
  assert.equal(r.Job.PORTABILITY, pb.Portability.PORTABLE);
  assert.deepEqual(r.Service.EXPOSES, [pb.Output.URL, pb.Output.HOST]);
  assert.deepEqual(r.Job.EXPOSES, []);
});

test("an output is a typed reference", () => {
  const web = new r.Service("web", IMAGE);
  assert.deepEqual(web.url, new pb.Ref({ resource: "web", standard: pb.Output.URL }));
  assert.deepEqual(web.host, new pb.Ref({ resource: "web", standard: pb.Output.HOST }));
  assert.deepEqual(web.ref, new pb.Ref({ resource: "web" }));
  assert.equal("url" in new r.Job("nightly", IMAGE), false);
});

test("an environment value may be a literal, a reference or a value", () => {
  const db = new r.Service("db", IMAGE);
  const web = new r.Service("web", IMAGE, {
    env: { A: "x", B: db.url, C: pb.Value.param("p") },
  });
  assert.deepEqual(
    web.body.env,
    new Map([
      ["A", new pb.Value({ literal: "x" })],
      ["B", new pb.Value({ ref: new pb.Ref({ resource: "db", standard: pb.Output.URL }) })],
      ["C", new pb.Value({ param: "p" })],
    ]),
  );
  assert.deepEqual(new r.Service("plain", IMAGE).body.env, new Map());
  // A Map is taken as it is.
  const mapped = new r.Service("m", IMAGE, { env: new Map([["K", "v"]]) });
  assert.deepEqual(mapped.body.env, new Map([["K", new pb.Value({ literal: "v" })]]));
});

test("uses may be a template, a reference or a uses", () => {
  const db = new r.Service("db", IMAGE);
  const explicit = new pb.Uses({ target: new pb.Ref({ resource: "cache" }), access: pb.Access.CALL });
  const web = new r.Service("web", IMAGE, {
    uses: [db, new pb.Ref({ resource: "queue" }), explicit],
  });
  assert.deepEqual(web.toResource().uses, [
    new pb.Uses({ target: new pb.Ref({ resource: "db" }), access: pb.Access.CALL }),
    new pb.Uses({ target: new pb.Ref({ resource: "queue" }), access: pb.Access.CALL }),
    explicit,
  ]);
});

test("a resource list takes templates and resources", () => {
  const built = new pb.Resource({ id: "x", job: new pb.Job({ image: IMAGE }) });
  const listed = r.resourceList(new r.Service("web", IMAGE), built);
  assert.deepEqual(
    listed.resource.map((x) => x.id),
    ["web", "x"],
  );
  assert.equal(listed.resource[1], built);
  assert.deepEqual(pb.ResourceList.decode(listed.encode()), listed);
});

test("the template says what the kind is", () => {
  // The comments of the proto are in the generated source, where an editor shows them.
  const url = new URL("../src/generated/resources.ts", import.meta.url);
  const source = readFileSync(url, "utf8");
  assert.match(source, /The `service` resource\./);
  assert.match(source, /A container that answers requests/);
});
