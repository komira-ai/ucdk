// SPDX-License-Identifier: Apache-2.0
/**
 * The wire contract with the Rust core.
 *
 * `sdk/wire-golden/<case>.bin` is what the Rust messages encode to (written by
 * `crates/ucdk-api/tests/sdk_golden.rs`). Each case here builds the same value with the generated
 * classes and must (1) encode to exactly those bytes and (2) be what the bytes decode to.
 * Building the value by hand, and not by decoding the golden, is what makes (2) a check of the
 * field numbers: a decoder that read field 2 into the wrong property would agree with an encoder
 * that wrote it back the same way, and only the hand-built value tells them apart.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import * as gp from "../src/generated/google_protobuf.ts";
import * as api from "../src/generated/ucdk_api_v1.ts";
import * as pb from "../src/generated/ucdk_resource_v1.ts";
import * as wire from "../src/generated/wire.ts";
import { REPO, bytesOf } from "./support.ts";

const GOLDEN = join(REPO, "sdk/wire-golden");

type Case = wire.Message & { encode(): Uint8Array };

function stepImage(): pb.Image {
  return pb.Image.output("build", "img", { platform: "linux/amd64" });
}

function digestImage(): pb.Image {
  return pb.Image.digest("sha256:aa", { platform: "linux/amd64" });
}

function secret(name: string): pb.SecretRef {
  return new pb.SecretRef({ name, store: "vault", version: "7" });
}

function fullService(more: pb.ServiceInit): pb.Service {
  return new pb.Service({
    image: stepImage(),
    port: 8080,
    args: ["--a", "--b"],
    env: {
      K: pb.Value.literal("v"),
      P: pb.Value.param("p"),
      R: pb.Value.ref(pb.Ref.standard(pb.Output.URL, { resource: "db" })),
      N: pb.Value.ref(pb.Ref.named("cert", { resource: "db" })),
    },
    size: new pb.Size({ cpuMillis: 500, memoryMb: 256 }),
    healthPath: "/healthz",
    requestTimeout: new gp.Duration({ seconds: 30n, nanos: 5 }),
    maxConcurrency: 80,
    secretEnv: { S: secret("sec") },
    ...more,
  });
}

function fullJob(maxRetries: number | undefined, trigger: pb.JobInit): pb.Job {
  return new pb.Job({
    image: digestImage(),
    args: ["go"],
    size: new pb.Size({ cpuMillis: 1000, memoryMb: 512 }),
    maxRetries,
    timeout: new gp.Duration({ seconds: -600n, nanos: -1 }),
    env: { E: pb.Value.literal("e") },
    secretEnv: { T: secret("tok") },
    ...trigger,
  });
}

function resourceList(): pb.ResourceList {
  return new pb.ResourceList({
    resource: [
      new pb.Resource({
        id: "web",
        uses: [new pb.Uses({ target: new pb.Ref({ resource: "db" }), access: pb.Access.CALL })],
        service: fullService({
          public: new pb.Service_Public(),
          scale: new pb.Scale({ min: 0, max: 10 }),
        }),
      }),
      new pb.Resource({
        id: "nightly",
        job: fullJob(3, { schedule: new pb.Job_Schedule({ cron: "0 * * * *", timezone: "UTC" }) }),
      }),
    ],
  });
}

function retained(): api.Retained {
  return new api.Retained({
    reason: api.RetainReason.UNDELETABLE,
    detail: "held by the provider",
  });
}

function report(): api.Report {
  return new api.Report({
    created: ["a/run", "b/run"],
    updated: ["c/run"],
    resumed: ["d/run"],
    deleted: ["e/run"],
    reaped: ["f/run"],
    retained: [new api.RetainedObject({ path: "g/run", retained: retained() })],
    converging: ["h/run"],
    failed: ["i/run"],
    blockedByFailed: [new api.Blocked({ path: "j/run", producer: "i/run" })],
    rolledBack: ["k/run"],
    rollbackFailed: [new api.RollbackFailure({ path: "l/run", message: "gone" })],
    leaks: [leak()],
    leakSweepFailed: error(),
  });
}

function leak(): api.Leak {
  return new api.Leak({
    kind: "service",
    physicalId: "svc-0001",
    name: "ghost-run",
    owner: "ghost",
    node: "abcdefghijklmnopqrstuvwxyz",
  });
}

function error(): api.Error {
  return new api.Error({ code: api.ErrorCode.PROVIDER, message: "the provider said no" });
}

function finding(): api.Finding {
  return new api.Finding({
    resource: "web",
    field: "service.env.DB_URL",
    kind: api.FindingKind.UNBOUND_REF,
    message: "db is not in the list",
  });
}

function refused(reason: api.RefusalInit): api.Outcome {
  return new api.Outcome({
    refused: new api.Refusal({ message: "refused", ...reason }),
    exit: api.ExitClass.REFUSED,
  });
}

function cases(): Record<string, Case> {
  const plan = new api.Plan({
    steps: [
      new api.Step({ path: "web/run", kind: "service", action: api.StepAction.CREATE }),
      new api.Step({
        path: "old/run",
        kind: "service",
        action: api.StepAction.RETAIN,
        physicalId: "fake-1",
        retained: retained(),
      }),
    ],
    hash: "ab".repeat(32),
  });
  return {
    resource_list: resourceList(),
    service_internal: fullService({
      internal: new pb.Service_Internal(),
      scale: new pb.Scale({ max: 10 }),
    }),
    job_on_demand: fullJob(undefined, { onDemand: new pb.Job_OnDemand() }),
    map_defaults: new pb.Service({
      env: { "": new pb.Value() },
      secretEnv: { "": new pb.SecretRef() },
    }),
    session_config: new api.SessionConfig({
      provider: "fake",
      providerOptions: { region: "r1", a: "b" },
      ledgerKind: "file",
      ledgerLocation: "/var/ucdk/ledger",
      cell: "machine/cell/uid",
      credentials: api.CredentialSource.PROFILE,
      profile: "work",
    }),
    plan_request: new api.PlanRequest({ resources: resourceList(), mode: api.Mode.DESTROY }),
    apply_request: new api.ApplyRequest({
      resources: resourceList(),
      planHash: "cd".repeat(32),
      rollback: true,
    }),
    destroy_request: new api.DestroyRequest({
      resources: resourceList(),
      planHash: "cd".repeat(32),
    }),
    status_request: new api.StatusRequest({ resources: resourceList() }),
    leaks_request: new api.LeaksRequest({}),
    outcome_planned: new api.Outcome({ planned: plan, exit: api.ExitClass.DONE }),
    outcome_done: new api.Outcome({ done: report(), exit: api.ExitClass.DONE }),
    outcome_refused_findings: refused({ findings: new api.Findings({ findings: [finding()] }) }),
    outcome_refused_conflicts: refused({
      conflicts: new api.Conflicts({
        conflicts: [
          new api.Conflict({
            path: "web/run",
            kind: api.ConflictKind.OTHER_TOOL,
            marker: "other-tool",
            message: "owned by another tool",
          }),
        ],
      }),
    }),
    outcome_refused_empty_scope: refused({ emptyScope: new api.EmptyScope() }),
    outcome_refused_plan_hash_mismatch: refused({
      planHashMismatch: new api.PlanHashMismatch({
        approved: "aa".repeat(32),
        plan: "bb".repeat(32),
      }),
    }),
    outcome_refused_plan_changed: refused({
      planChanged: new api.PlanChanged({ approved: "aa".repeat(32), now: "bb".repeat(32) }),
    }),
    outcome_refused_unsupported_stamp_mode: refused({
      unsupportedStampMode: new api.UnsupportedStampMode({ paths: ["web/run"] }),
    }),
    outcome_refused_lease_held: refused({
      leaseHeld: new api.LeaseHeld({ holder: "ci-7", remainingMs: 95000n }),
    }),
    outcome_refused_unhosted_kinds: refused({
      unhostedKinds: new api.UnhostedKinds({
        kinds: [new api.UnhostedKind({ path: "batch/run", kind: "job" })],
      }),
    }),
    outcome_failed: new api.Outcome({
      failed: new api.Failure({ report: report(), at: "web/run", error: error() }),
      exit: api.ExitClass.FAILED_CHANGED,
    }),
    outcome_invalid: new api.Outcome({ invalid: error(), exit: api.ExitClass.USAGE }),
    outcome_status: new api.Outcome({
      status: new api.StatusResponse({
        objects: [
          new api.ObjectStatus({
            path: "web/run",
            kind: "service",
            record: api.RecordState.CONFIRMED,
            live: api.LiveStatus.READY,
            standing: api.Standing.AGREES,
            outputs: { URL: "https://web.example.com" },
          }),
        ],
      }),
      exit: api.ExitClass.DONE,
    }),
    outcome_leaks: new api.Outcome({
      leaks: new api.LeaksResponse({ leaks: [leak()] }),
      exit: api.ExitClass.LEAKS,
    }),
    catalog_response: new api.CatalogResponse({
      nodes: [
        new api.CatalogNode({ path: "web/run", kind: "service", wanted: true, deps: ["db/run"] }),
      ],
      findings: [finding()],
      exit: api.ExitClass.REFUSED,
    }),
    error: error(),
  };
}

/** `{case: message name}` as the Rust side wrote it. */
function manifest(): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of readFileSync(join(GOLDEN, "manifest.txt"), "utf8").split("\n")) {
    if (line !== "") {
      const [file, message] = line.split(" ") as [string, string];
      out[file.slice(0, -".bin".length)] = message;
    }
  }
  return out;
}

type Decoder = { decode(data: Uint8Array): Case };

function classOf(messageName: string): Decoder {
  const at = messageName.lastIndexOf(".");
  const modules: Record<string, Record<string, unknown>> = {
    "ucdk.api.v1": api,
    "ucdk.resource.v1": pb,
  };
  return (modules[messageName.slice(0, at)] as Record<string, Decoder>)[
    messageName.slice(at + 1)
  ] as Decoder;
}

function golden(name: string): Buffer {
  return readFileSync(join(GOLDEN, `${name}.bin`));
}

test("every golden has a case and every case a golden", () => {
  assert.deepEqual(Object.keys(manifest()).sort(), Object.keys(cases()).sort());
});

test("each case is the message its manifest line names", () => {
  for (const [name, message] of Object.entries(manifest())) {
    const built = (cases() as Record<string, Case>)[name];
    assert.equal(built?.constructor, classOf(message), name);
  }
});

test("each case encodes to the bytes rust writes", () => {
  for (const [name, value] of Object.entries(cases())) {
    assert.deepEqual(bytesOf(value), golden(name), name);
  }
});

test("each golden decodes to the value the case builds", () => {
  for (const [name, value] of Object.entries(cases())) {
    const decoded = classOf(manifest()[name] as string).decode(golden(name));
    assert.deepEqual(decoded, value, name);
  }
});

test("a decoded golden encodes back to the same bytes", () => {
  for (const name of Object.keys(cases())) {
    const data = golden(name);
    const decoded = classOf(manifest()[name] as string).decode(data);
    assert.deepEqual(bytesOf(decoded), data, name);
  }
});
