// SPDX-License-Identifier: Apache-2.0
/**
 * plan, apply, plan again, destroy and status through the SDK, over the real library and the
 * fake provider, held to the data the command line prints for the same scenario.
 *
 * `crates/ucdk-cli/tests/scenario.rs` runs the scenario through the CLI and compares each step
 * with the files in `crates/ucdk-cli/tests/data`; these tests read the same files, so the SDK
 * and the command line must agree on the plan hash, the steps, the report and the status.
 */

import assert from "node:assert/strict";
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { api, proto, resources } from "../src/index.ts";
import { Internal } from "../src/errors.ts";
import {
  DATA,
  IMAGE,
  LEDGER_GOLDEN,
  objectsOf,
  openSession,
  pair,
  readPlan,
  readReport,
  readStatus,
  stepsOf,
} from "./support.ts";

test("plan, apply, plan again, status, destroy give the command lines outcomes", async () => {
  const wanted = pair();
  await using session = await openSession();

  const before = await session.status(wanted);
  assert.deepEqual(objectsOf(before), readStatus("status_before_fake.txt"));
  assert.equal(before.exit, api.ExitClass.DONE);

  const planned = await session.plan(wanted);
  const expected = readPlan("plan_apply_fake.txt");
  assert.equal(planned.whichResult(), "planned");
  assert.equal(planned.exit, api.ExitClass.DONE);
  assert.ok(planned.planned);
  assert.equal(planned.planned.hash, expected.hash);
  assert.deepEqual(stepsOf(planned.planned), expected.steps);

  const applied = await session.apply(wanted, planned.planned.hash);
  assert.equal(applied.whichResult(), "done");
  assert.equal(applied.exit, api.ExitClass.DONE);
  assert.deepEqual(applied.done?.created, readReport("apply_fake.txt").created);
  assert.deepEqual(
    [applied.done?.updated, applied.done?.deleted, applied.done?.failed],
    [[], [], []],
  );

  assert.deepEqual(objectsOf(await session.status(wanted)), readStatus("status_fake.txt"));

  const again = (await session.plan(wanted)).planned;
  const afterwards = readPlan("plan_again_fake.txt");
  assert.ok(again);
  assert.equal(again.hash, afterwards.hash);
  assert.deepEqual(stepsOf(again), afterwards.steps);

  const doomed = (await session.plan(wanted, api.Mode.DESTROY)).planned;
  const destroy = readPlan("plan_destroy_fake.txt");
  assert.ok(doomed);
  assert.equal(doomed.hash, destroy.hash);
  assert.deepEqual(stepsOf(doomed), destroy.steps);

  const destroyed = await session.destroy(wanted, doomed.hash);
  assert.equal(destroyed.whichResult(), "done");
  assert.deepEqual(destroyed.done?.deleted, readReport("destroy_fake.txt").deleted);

  const gone = (await session.plan(wanted, api.Mode.DESTROY)).planned;
  const none = readPlan("plan_destroyed_fake.txt");
  assert.ok(gone);
  assert.deepEqual([gone.hash, stepsOf(gone)], [none.hash, none.steps]);
});

test("the catalog wants the nodes the plan creates in the same order", async () => {
  await using session = await openSession();
  const catalog = await session.loadCatalog(pair());
  const { steps } = readPlan("plan_apply_fake.txt");
  assert.equal(catalog.exit, api.ExitClass.DONE);
  const wanted = catalog.nodes.filter((n) => n.wanted).map((n) => [n.path, n.kind]);
  assert.deepEqual(wanted, steps.map(([, path, kind]) => [path, kind]));
  // A role the list turns off stays, so that its leftovers are found.
  assert.deepEqual(
    catalog.nodes.filter((n) => !n.wanted).map((n) => n.path),
    ["example-db/public"],
  );
  const deps = new Map(catalog.nodes.map((n) => [n.path, n.deps]));
  assert.deepEqual(deps.get("example-db/run"), []);
  assert.ok(deps.get("example-svc/run")?.includes("example-db/run"));
});

test("a list the catalog refuses comes back as findings", async () => {
  const dangling = resources.resourceList(
    new resources.Service("web", IMAGE, { uses: [new proto.Ref({ resource: "ghost" })] }),
  );
  await using session = await openSession();
  const catalog = await session.loadCatalog(dangling);
  const planned = await session.plan(dangling);
  assert.equal(catalog.exit, api.ExitClass.REFUSED);
  assert.deepEqual(catalog.nodes, []);
  assert.deepEqual(catalog.findings.map((f) => f.resource), ["web"]);
  assert.equal(planned.exit, api.ExitClass.REFUSED);
  assert.equal(planned.whichResult(), "refused");
  assert.deepEqual(
    planned.refused?.findings?.findings.map((f) => f.kind),
    catalog.findings.map((f) => f.kind),
  );
});

// ---- what the engine refuses is data -------------------------------------------------------

test("a kind the provider does not host is refused and nothing is created", async () => {
  const recorded = readFileSync(join(DATA, "plan_apply_fake_limited_refused.txt"), "utf8");
  assert.match(recorded, /refused: kind job is not hosted by fake-limited \(example-batch\/run\)/);
  const wanted = resources.resourceList(
    new resources.Service("example-svc", IMAGE, { port: 8080 }),
    new resources.Job("example-batch", IMAGE),
  );
  await using session = await openSession("fake-limited");
  const outcome = await session.plan(wanted);
  assert.equal(outcome.exit, api.ExitClass.REFUSED);
  assert.equal(outcome.whichResult(), "refused");
  assert.deepEqual(outcome.refused?.unhostedKinds?.kinds, [
    new api.UnhostedKind({ path: "example-batch/run", kind: "job" }),
  ]);
  const applied = await session.apply(wanted, "a".repeat(64));
  assert.equal(applied.exit, api.ExitClass.REFUSED);
  assert.equal(applied.refused?.whichReason(), "unhostedKinds");
  // A status is refused the same way; the same list without the job is hosted, and nothing of
  // it was created.
  assert.equal((await session.status(wanted)).exit, api.ExitClass.REFUSED);
  const services = resources.resourceList(new resources.Service("example-svc", IMAGE));
  assert.equal((await session.plan(services)).whichResult(), "planned");
  assert.deepEqual(
    (await session.status(services)).status?.objects.map((o) => o.record),
    [undefined],
  );
});

test("an apply under a hash that is not the plan is refused and changes nothing", async () => {
  const wanted = pair();
  await using session = await openSession();
  const stale = await session.apply(wanted, "a".repeat(64));
  const real = (await session.plan(wanted)).planned?.hash;
  assert.equal(stale.exit, api.ExitClass.REFUSED);
  assert.equal(stale.refused?.whichReason(), "planHashMismatch");
  assert.deepEqual(
    stale.refused?.planHashMismatch,
    new api.PlanHashMismatch({ approved: "a".repeat(64), plan: real }),
  );
  const objects = (await session.status(wanted)).status?.objects ?? [];
  assert.ok(objects.length > 0);
  assert.ok(objects.every((o) => o.record === undefined && o.live === undefined));
});

test("a hash that is not a hash is an invalid request, not an error", async () => {
  await using session = await openSession();
  const outcome = await session.apply(pair(), "not-a-hash");
  assert.equal(outcome.exit, api.ExitClass.USAGE);
  assert.equal(outcome.whichResult(), "invalid");
  assert.equal(outcome.invalid?.code, api.ErrorCode.INVALID_REQUEST);
});

test("a destroy of nothing is refused as an empty scope", async () => {
  await using session = await openSession();
  const outcome = await session.plan(resources.resourceList(), api.Mode.DESTROY);
  const destroyed = await session.destroy(resources.resourceList(), "b".repeat(64));
  for (const refused of [outcome, destroyed]) {
    assert.equal(refused.exit, api.ExitClass.REFUSED);
    assert.equal(refused.refused?.whichReason(), "emptyScope");
  }
});

// ---- a ledger in a file --------------------------------------------------------------------

function inTemporaryDirectory(body: (directory: string) => Promise<void>): Promise<void> {
  const directory = mkdtempSync(join(tmpdir(), "ucdk-ts-"));
  return body(directory).finally(() => rmSync(directory, { recursive: true }));
}

test("the ledger outlives the session and a second session reads it", () =>
  inTemporaryDirectory(async (directory) => {
    const wanted = pair();
    const location = join(directory, "ledger");
    {
      await using first = await openSession("fake", { ledgerKind: "file", ledgerLocation: location });
      const plan = await first.plan(wanted);
      assert.equal((await first.apply(wanted, plan.planned?.hash as string)).whichResult(), "done");
    }
    // The fake provider's objects are in memory and went with the first session; the ledger is a
    // file, so the second session finds the records and no object.
    await using second = await openSession("fake", { ledgerKind: "file", ledgerLocation: location });
    assert.deepEqual(
      objectsOf(await second.status(wanted)),
      readStatus("status_fake.txt").map(([path, kind]) => [
        path,
        kind,
        api.RecordState.CONFIRMED,
        undefined,
        api.Standing.MISSING,
      ]),
    );
  }));

test("a second session opens beside the first and sees what it writes", () =>
  inTemporaryDirectory(async (directory) => {
    const wanted = pair();
    const location = join(directory, "ledger");
    await using first = await openSession("fake", { ledgerKind: "file", ledgerLocation: location });
    await using second = await openSession("fake", { ledgerKind: "file", ledgerLocation: location });
    const before = objectsOf(await second.status(wanted));
    assert.ok(before.length > 0);
    for (const [, , , , standing] of before) assert.equal(standing, api.Standing.NOT_APPLIED);
    const plan = await first.plan(wanted);
    assert.equal((await first.apply(wanted, plan.planned?.hash as string)).whichResult(), "done");
    const seen = objectsOf(await second.status(wanted));
    assert.ok(seen.length > 0);
    for (const [, , record, , standing] of seen) {
      assert.equal(record, api.RecordState.CONFIRMED);
      assert.equal(standing, api.Standing.MISSING);
    }
  }));

test("a file that is not a ledger cannot be opened", () =>
  inTemporaryDirectory(async (directory) => {
    const location = join(directory, "ledger");
    writeFileSync(location, "not a ledger\n");
    await assert.rejects(
      openSession("fake", { ledgerKind: "file", ledgerLocation: location }),
      (error: unknown) => {
        assert.ok(error instanceof Internal);
        assert.equal(error.status, 101);
        assert.ok(error.error?.message);
        return true;
      },
    );
  }));

test("a ledger in a directory that is not there cannot be opened", () =>
  inTemporaryDirectory(async (directory) => {
    await assert.rejects(
      openSession("fake", { ledgerKind: "file", ledgerLocation: join(directory, "no", "ledger") }),
      Internal,
    );
  }));

// ---- the ledger's lease --------------------------------------------------------------------

test("an apply on a ledger another holder has leased is refused as data", () =>
  inTemporaryDirectory(async (directory) => {
    const wanted = pair();
    const location = join(directory, "ledger");
    copyFileSync(join(LEDGER_GOLDEN, "leased.ucdk"), location);
    await using session = await openSession("fake", { ledgerKind: "file", ledgerLocation: location });
    // Reading is not writing: the plan and the status are answered.
    const plan = await session.plan(wanted);
    assert.equal(plan.whichResult(), "planned");
    assert.equal((await session.status(wanted)).exit, api.ExitClass.DONE);
    const applied = await session.apply(wanted, plan.planned?.hash as string);
    assert.equal(applied.exit, api.ExitClass.REFUSED);
    assert.equal(applied.whichResult(), "refused");
    assert.equal(applied.refused?.whichReason(), "leaseHeld");
    assert.equal(applied.refused?.leaseHeld?.holder, "ci-7");
    assert.ok((applied.refused?.leaseHeld?.remainingMs ?? 0n) > 0n);
    assert.match(applied.refused?.message ?? "", /leased to ci-7/);
    // Nothing was created or recorded.
    const status = await session.status(wanted);
    assert.ok(status.status?.objects.every((o) => o.record === undefined && o.live === undefined));
  }));

test("an apply on a ledger whose holder lapsed takes over and goes through", () =>
  inTemporaryDirectory(async (directory) => {
    const wanted = pair();
    const location = join(directory, "ledger");
    copyFileSync(join(LEDGER_GOLDEN, "lapsed.ucdk"), location);
    await using session = await openSession("fake", { ledgerKind: "file", ledgerLocation: location });
    const plan = await session.plan(wanted);
    const applied = await session.apply(wanted, plan.planned?.hash as string);
    assert.equal(applied.exit, api.ExitClass.DONE);
    assert.equal(applied.whichResult(), "done");
    assert.deepEqual(applied.done?.created, readReport("apply_fake.txt").created);
  }));
