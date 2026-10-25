// SPDX-License-Identifier: Apache-2.0
/**
 * The leak sweep through the real library: an object that carries the cell's stamp and has no
 * ledger record is reported with exit class LEAKS (8), on both ledgers; nothing is changed or
 * deleted; an apply reports leaks and is still done.
 */

import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { Session, api } from "../src/index.ts";
import { openSession, pair } from "./support.ts";

/** Runs `body` on a session over each ledger, with the provider that holds one leaked object. */
async function onBothLedgers(body: (name: string, session: Session) => Promise<void>): Promise<void> {
  {
    await using memory = await openSession("fake-leaky");
    await body("memory", memory);
  }
  const directory = mkdtempSync(join(tmpdir(), "ucdk-ts-leaks-"));
  try {
    await using file = await openSession("fake-leaky", {
      ledgerKind: "file",
      ledgerLocation: join(directory, "ledger"),
    });
    await body("file", file);
  } finally {
    rmSync(directory, { recursive: true });
  }
}

// Proves: the verb reaches the library, the response decodes with the leaks arm, and the exit
// class is 8, on the memory and the file ledger. Catches a verb bound to the wrong symbol, or a
// class mapped to done.
test("a stamped object with no record is a leak with exit 8", () =>
  onBothLedgers(async (name, session) => {
    const outcome = await session.leaks();
    assert.equal(outcome.whichResult(), "leaks", name);
    assert.equal(outcome.exit, api.ExitClass.LEAKS, name);
    assert.equal(Number(outcome.exit), 8, name);
    const found = outcome.leaks?.leaks ?? [];
    assert.equal(found.length, 1, name);
    assert.deepEqual([found[0]?.kind, found[0]?.name, found[0]?.owner], ["service", "ghost-run", "ghost"]);
    assert.notEqual(found[0]?.physicalId, "");
    assert.notEqual(found[0]?.node, "");
  }));

// Proves: the same sweep over the ordinary fake is exit 0 with an empty list, before and after an
// apply. Catches a sweep that reports recorded or absent objects.
test("a world with no leak is done", async () => {
  await using session = await openSession("fake");
  const empty = await session.leaks();
  assert.equal(empty.exit, api.ExitClass.DONE);
  assert.deepEqual(empty.leaks?.leaks, []);
  const planned = await session.plan(pair());
  await session.apply(pair(), planned.planned?.hash as string);
  assert.equal((await session.leaks()).exit, api.ExitClass.DONE);
});

// Proves: sweeping twice gives the same answer, and the plan of the same list is the same before
// and after (the leaked object is still there, nothing was recorded or deleted). Catches a sweep
// that deletes or adopts.
test("the sweep changes nothing", () =>
  onBothLedgers(async (name, session) => {
    const before = (await session.plan(pair())).planned?.hash;
    const first = await session.leaks();
    assert.deepEqual((await session.leaks()).encode(), first.encode(), name);
    assert.equal((await session.plan(pair())).planned?.hash, before, name);
    assert.equal((await session.leaks()).leaks?.leaks.length, 1, name);
  }));

// Proves: an apply of a list beside a leaked object ends done (exit 0) with the leak in its
// report, the same object the sweep names, and the object is still leaked after. Catches an apply
// that fails for leaks, or a report that drops them.
test("an apply reports the leak and is still done", () =>
  onBothLedgers(async (name, session) => {
    const leaked = (await session.leaks()).leaks?.leaks[0] as api.Leak;
    const planned = await session.plan(pair());
    const applied = await session.apply(pair(), planned.planned?.hash as string);
    assert.equal(applied.whichResult(), "done", name);
    assert.equal(applied.exit, api.ExitClass.DONE, name);
    assert.equal(applied.done?.created.length, 3, name);
    assert.deepEqual(applied.done?.leaks, [leaked], name);
    assert.equal(applied.done?.leakSweepFailed, undefined, name);
    assert.deepEqual((await session.leaks()).leaks?.leaks, [leaked], name);
  }));
