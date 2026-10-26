// SPDX-License-Identifier: Apache-2.0
/**
 * What the SDK tests share: the library, the scenario data of the command line, and the pair of
 * services every scenario starts from.
 */

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { Library, Session, api, proto, resources } from "../src/index.ts";

export const REPO = fileURLToPath(new URL("../../../", import.meta.url));
export const DATA = join(REPO, "crates/ucdk-cli/tests/data");
export const LEDGER_GOLDEN = join(REPO, "sdk/ledger-golden");
// The clock a session reads, in milliseconds, against the lease in `leased.ucdk`: `ci-7` took it
// at 1_000_000 for an hour. The tests never read the time.
export const WHILE_HELD_MS = 2_000_000n;
export const AFTER_LAPSE_MS = 10_000_000n;
export const LIBRARY = join(
  REPO,
  "target/debug",
  process.platform === "darwin" ? "libucdk_capi.dylib" : "libucdk_capi.so",
);

export const IMAGE = proto.Image.digest("sha256:aa");

/** The library `cargo xtask sdk-test` builds. */
export function libraryPath(): string {
  if (!existsSync(LIBRARY)) {
    throw new Error(`${LIBRARY} is not built; run \`cargo xtask sdk-test\`, which builds it`);
  }
  return LIBRARY;
}

export function library(): Library {
  return new Library(libraryPath());
}

export async function openSession(
  provider = "fake",
  config: api.SessionConfigInit = {},
): Promise<Session> {
  return Session.open(new api.SessionConfig({ ledgerKind: "memory", ...config, provider }), {
    libraryPath: libraryPath(),
  });
}

/** `example-svc` (public) calls `example-db`: the pair of the command line's scenario. */
export function pair(): proto.ResourceList {
  const db = new resources.Service("example-db", IMAGE, { port: 8080 });
  const svc = new resources.Service("example-svc", IMAGE, {
    port: 8080,
    exposure: new resources.Service.Public(),
    uses: [db],
  });
  return resources.resourceList(svc, db);
}

/** The text of a file of the command line's recorded output. */
function data(name: string): string[] {
  return readFileSync(join(DATA, name), "utf8").split("\n");
}

// ---- the command line's recorded output ---------------------------------------------------------

const ACTIONS: Record<string, api.StepAction> = {
  create: api.StepAction.CREATE,
  update: api.StepAction.UPDATE,
  ok: api.StepAction.NOOP,
  delete: api.StepAction.DELETE,
  wait: api.StepAction.CONVERGING,
  later: api.StepAction.KNOWN_AFTER_APPLY,
  reap: api.StepAction.REAP,
};

export type Step = [api.StepAction, string, string];

/** The hash and the steps of a plan the command line printed. */
export function readPlan(name: string): { hash: string; steps: Step[] } {
  const lines = data(name);
  const line = lines.find((l) => l.startsWith("plan-hash: "));
  if (line === undefined) {
    throw new Error(`${name} has no plan-hash line`);
  }
  const steps: Step[] = [];
  for (const l of lines.filter((l) => l.startsWith("  "))) {
    const [word, path, kind] = l.trim().split(/\s+/) as [string, string, string];
    steps.push([ACTIONS[word] as api.StepAction, path, kind.replace(/^\[|\]$/g, "")]);
  }
  return { hash: line.slice("plan-hash: ".length), steps };
}

/** `{verb: [paths]}` of a run the command line printed. */
export function readReport(name: string): Record<string, string[]> {
  const report: Record<string, string[]> = {};
  for (const l of data(name).filter((l) => l.startsWith("  "))) {
    const [verb, path] = l.trim().split(/\s+/) as [string, string];
    (report[verb] ??= []).push(path);
  }
  return report;
}

export type Object_ = [
  string,
  string,
  api.RecordState | undefined,
  api.LiveStatus | undefined,
  api.Standing,
];

const RECORDS: Record<string, api.RecordState | undefined> = {
  none: undefined,
  confirmed: api.RecordState.CONFIRMED,
};
const LIVES: Record<string, api.LiveStatus | undefined> = {
  absent: undefined,
  ready: api.LiveStatus.READY,
};
const STANDINGS: Record<string, api.Standing> = {
  agrees: api.Standing.AGREES,
  "not applied": api.Standing.NOT_APPLIED,
};

/** The objects of the status the command line printed. */
export function readStatus(name: string): Object_[] {
  const out: Object_[] = [];
  for (const l of data(name)) {
    const match = /^ {2}(\S+) \[(\S+)\] record: (\w+), live: (\w+), (.+)$/.exec(l);
    if (match !== null) {
      const [, path, kind, record, live, standing] = match;
      out.push([
        path as string,
        kind as string,
        RECORDS[record as string],
        LIVES[live as string],
        STANDINGS[standing as string] as api.Standing,
      ]);
    }
  }
  return out;
}

export function stepsOf(plan: api.Plan): Step[] {
  return plan.steps.map((s) => [s.action, s.path, s.kind]);
}

export function objectsOf(outcome: api.Outcome): Object_[] {
  const status = outcome.status;
  if (status === undefined) {
    throw new Error("the outcome has no status");
  }
  return status.objects.map((o) => [o.path, o.kind, o.record, o.live, o.standing]);
}

/** The bytes of a message as a `Buffer`, which `assert` compares with the bytes of a file. */
export function bytesOf(message: { encode(): Uint8Array }): Buffer {
  return Buffer.from(message.encode());
}
