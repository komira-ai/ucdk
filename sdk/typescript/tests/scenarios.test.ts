// SPDX-License-Identifier: Apache-2.0
/**
 * `ledgerShow` through the SDK, over the real library and the ledger goldens of
 * `sdk/ledger-golden`: the document the library shows, the lease by the session's clock, and the
 * ledger errors, as data.
 */

import assert from "node:assert/strict";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { api } from "../src/index.ts";
import {
  AFTER_LAPSE_MS,
  EXPIRES_MS,
  LAPSING_MS,
  WHILE_HELD_MS,
  openSession,
  withGolden,
} from "./support.ts";

function inTemporaryDirectory<T>(body: (directory: string) => Promise<T>): Promise<T> {
  const directory = mkdtempSync(join(tmpdir(), "ucdk-scenario-"));
  return body(directory).finally(() => rmSync(directory, { recursive: true, force: true }));
}

test("a ledger show over the records golden gives its token, fence and records", () =>
  withGolden("records.ucdk", async (golden) => {
    await using session = await openSession({ ledgerKind: "file", ledgerLocation: golden.path });
    const shown = await session.ledgerShow();
    assert.equal(shown.whichResult(), "ledgerShow");
    assert.equal(shown.exit, api.ExitClass.DONE);
    const document = shown.ledgerShow as api.LedgerShowResponse;
    assert.deepEqual([document.token, document.fence], ["v5", 0n]);
    assert.equal(document.lease, undefined);
    assert.deepEqual(
      document.records.map((r) => r.path),
      ["app/db", "app/old", "app/pending", "app/run", "shared/found"],
    );
    const run = document.records[3] as api.LedgerRecord;
    assert.deepEqual([run.kind, run.name, run.physical], ["service", "app-run", "run-1"]);
    assert.deepEqual(run.deps, ["app/db"]);
    assert.deepEqual(run.outputs.map((o) => [o.key, o.value]), [["URL", "run-url"]]);
    assert.deepEqual(
      document.records.map((r) => [r.origin, r.state]),
      [
        [api.RecordOrigin.CREATED, api.RecordState.CONFIRMED],
        [api.RecordOrigin.CREATED, api.RecordState.REAPING],
        [api.RecordOrigin.CREATED, api.RecordState.INTENT],
        [api.RecordOrigin.CREATED, api.RecordState.CONFIRMED],
        [api.RecordOrigin.ADOPTED, api.RecordState.CONFIRMED],
      ],
    );
    const found = document.records[4] as api.LedgerRecord;
    assert.deepEqual(found.outputs.map((o) => [o.key, o.value]), [
      ["EMPTY", ""],
      ["NOTE", "two words"],
    ]);
  }));

test("a ledger show of a memory ledger is the empty document", async () => {
  await using session = await openSession();
  const document = (await session.ledgerShow()).ledgerShow as api.LedgerShowResponse;
  assert.deepEqual([document.token, document.fence, document.records], ["v0", 0n, []]);
  assert.equal(document.lease, undefined);
});

test("a ledger show takes no lease and changes no file", () =>
  withGolden("leased.ucdk", async (golden) => {
    const before = golden.listing();
    await using session = await openSession({
      ledgerKind: "file",
      ledgerLocation: golden.path,
      testClockMs: WHILE_HELD_MS,
    });
    for (let i = 0; i < 2; i += 1) {
      const document = (await session.ledgerShow()).ledgerShow as api.LedgerShowResponse;
      assert.deepEqual([document.fence, document.lease?.holder], [1n, "ci-7"]);
    }
    assert.deepEqual(golden.listing(), before);
  }));

/** The lease a session shows of `leased.ucdk` with its clock at `nowMs`, named `holder`. */
function leaseSeen(nowMs: bigint, holder = ""): Promise<api.LeaseView> {
  return withGolden("leased.ucdk", async (golden) => {
    await using session = await openSession({
      ledgerKind: "file",
      ledgerLocation: golden.path,
      testClockMs: nowMs,
      holder,
    });
    return (await session.ledgerShow()).ledgerShow?.lease as api.LeaseView;
  });
}

test("another holders lease is live, then lapsing, then lapsed by the session clock", async () => {
  for (const [nowMs, state] of [
    [WHILE_HELD_MS, api.LeaseState.LIVE],
    [EXPIRES_MS - 1n, api.LeaseState.LIVE],
    [EXPIRES_MS, api.LeaseState.LAPSING],
    [LAPSING_MS, api.LeaseState.LAPSING],
    [AFTER_LAPSE_MS, api.LeaseState.LAPSED],
  ] as const) {
    const lease = await leaseSeen(nowMs, "ci-8");
    assert.equal(lease.state, state, String(nowMs));
    assert.deepEqual([lease.holder, lease.expiresMs], ["ci-7", EXPIRES_MS]);
    assert.equal(lease.yours, false);
  }
});

test("a session named as the holder sees the lease as its own", async () => {
  for (const nowMs of [WHILE_HELD_MS, LAPSING_MS, AFTER_LAPSE_MS]) {
    assert.equal((await leaseSeen(nowMs, "ci-7")).yours, true, String(nowMs));
    assert.equal((await leaseSeen(nowMs, "ci-8")).yours, false, String(nowMs));
    assert.equal((await leaseSeen(nowMs)).yours, false, String(nowMs));
  }
});

// ---- a ledger that cannot be read is data --------------------------------------------------

test("ledger show of a file that is not a ledger is the ledger error", () =>
  inTemporaryDirectory(async (directory) => {
    const location = join(directory, "ledger.ucdk");
    writeFileSync(location, "this is not a ledger\n");
    await using session = await openSession({ ledgerKind: "file", ledgerLocation: location });
    const shown = await session.ledgerShow();
    assert.equal(readFileSync(location, "utf8"), "this is not a ledger\n");
    assert.equal(shown.whichResult(), "error");
    assert.equal(shown.exit, api.ExitClass.FAILED_NO_EFFECT);
    assert.equal(shown.error?.code, api.ErrorCode.LEDGER);
    assert.ok(shown.error?.message);
  }));

test("a missing file is the ledger error and no file is made", () =>
  inTemporaryDirectory(async (directory) => {
    await using session = await openSession({
      ledgerKind: "file",
      ledgerLocation: join(directory, "ledger.ucdk"),
    });
    const shown = await session.ledgerShow();
    assert.equal(shown.error?.code, api.ErrorCode.LEDGER);
    assert.match(shown.error?.message ?? "", /does not exist/);
    assert.deepEqual(readdirSync(directory), []);
  }));
